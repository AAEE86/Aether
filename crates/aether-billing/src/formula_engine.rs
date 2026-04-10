use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use crate::precision::quantize_cost;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormulaEvaluationStatus {
    Complete,
    Incomplete,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FormulaEvaluationResult {
    pub status: FormulaEvaluationStatus,
    pub cost: f64,
    pub resolved_dimensions: BTreeMap<String, serde_json::Value>,
    pub resolved_variables: BTreeMap<String, serde_json::Value>,
    pub cost_breakdown: BTreeMap<String, f64>,
    pub tier_index: Option<i64>,
    pub tier_info: Option<serde_json::Value>,
    pub missing_required: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Error)]
pub enum UnsafeExpressionError {
    #[error("unsupported expression syntax: {0}")]
    Unsupported(String),
}

#[derive(Debug, Error)]
pub enum ExpressionEvaluationError {
    #[error("expression evaluation failed: {0}")]
    Failed(String),
}

#[derive(Debug, Error)]
#[error("missing required dimensions: {missing_required:?}")]
pub struct BillingIncompleteError {
    pub missing_required: Vec<String>,
}

pub struct FormulaEngine;

type TierMetadata = Option<(i64, serde_json::Value)>;
type MappingResolution = Result<(serde_json::Value, bool, TierMetadata), ExpressionEvaluationError>;

impl Default for FormulaEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl FormulaEngine {
    pub fn new() -> Self {
        Self
    }

    pub fn evaluate(
        &self,
        expression: &str,
        variables: Option<&BTreeMap<String, serde_json::Value>>,
        dimensions: Option<&BTreeMap<String, serde_json::Value>>,
        dimension_mappings: Option<&BTreeMap<String, serde_json::Value>>,
        strict_mode: bool,
    ) -> Result<FormulaEvaluationResult, ExpressionEvaluationError> {
        let dims = dimensions.cloned().unwrap_or_default();
        let mut resolved = variables.cloned().unwrap_or_default();
        let mut missing_required = Vec::new();
        let mut tier_index = None;
        let mut tier_info = None;
        let mut computed = BTreeMap::new();

        if let Some(mappings) = dimension_mappings {
            for (var_name, mapping) in mappings {
                let source = mapping
                    .get("source")
                    .and_then(|v| v.as_str())
                    .unwrap_or("constant")
                    .to_ascii_lowercase();
                if source == "computed" {
                    computed.insert(var_name.clone(), mapping.clone());
                    continue;
                }
                if source == "constant" && resolved.contains_key(var_name) {
                    continue;
                }

                let (value, is_missing, tier_meta) = resolve_mapping(var_name, mapping, &dims)?;
                if let Some((idx, info)) = tier_meta {
                    if tier_index.is_none() {
                        tier_index = Some(idx);
                        tier_info = Some(info);
                    }
                }
                if is_missing {
                    missing_required.push(var_name.clone());
                    continue;
                }
                resolved.insert(var_name.clone(), value);
            }
        }

        if !computed.is_empty() {
            let mut unresolved = computed;
            for _ in 0..std::cmp::max(4, unresolved.len() + 1) {
                let mut progressed = false;
                let names: Vec<String> = unresolved.keys().cloned().collect();
                for var_name in names {
                    let mapping = match unresolved.get(&var_name).cloned() {
                        Some(value) => value,
                        None => continue,
                    };
                    let (status, value) = try_resolve_computed(&mapping, &dims, &resolved)?;
                    match status {
                        ComputedStatus::Pending => {}
                        ComputedStatus::MissingRequired => {
                            missing_required.push(var_name.clone());
                            unresolved.remove(&var_name);
                        }
                        ComputedStatus::Ok | ComputedStatus::Defaulted => {
                            resolved.insert(var_name.clone(), value);
                            unresolved.remove(&var_name);
                            progressed = true;
                        }
                    }
                }
                if !progressed {
                    break;
                }
            }

            for (var_name, mapping) in unresolved {
                let required = mapping
                    .get("required")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if required {
                    missing_required.push(var_name);
                }
            }
        }

        if !missing_required.is_empty() {
            if strict_mode {
                return Err(ExpressionEvaluationError::Failed(
                    BillingIncompleteError { missing_required }.to_string(),
                ));
            }
            return Ok(FormulaEvaluationResult {
                status: FormulaEvaluationStatus::Incomplete,
                cost: 0.0,
                resolved_dimensions: dims,
                resolved_variables: resolved,
                cost_breakdown: BTreeMap::new(),
                tier_index,
                tier_info,
                missing_required,
                error: None,
            });
        }

        let cost = evaluate_expression(expression, &resolved)
            .map_err(|err| ExpressionEvaluationError::Failed(err.to_string()))?;
        if cost < 0.0 {
            return Ok(FormulaEvaluationResult {
                status: FormulaEvaluationStatus::Incomplete,
                cost: 0.0,
                resolved_dimensions: dims,
                resolved_variables: resolved,
                cost_breakdown: BTreeMap::new(),
                tier_index,
                tier_info,
                missing_required: Vec::new(),
                error: Some("negative_cost".to_string()),
            });
        }

        let mut breakdown = BTreeMap::new();
        for (key, value) in &resolved {
            if key.ends_with("_cost") {
                if let Some(number) = as_f64(value) {
                    breakdown.insert(key.clone(), quantize_cost(number));
                }
            }
        }

        Ok(FormulaEvaluationResult {
            status: FormulaEvaluationStatus::Complete,
            cost: quantize_cost(cost),
            resolved_dimensions: dims,
            resolved_variables: resolved,
            cost_breakdown: breakdown,
            tier_index,
            tier_info,
            missing_required: Vec::new(),
            error: None,
        })
    }
}

pub fn extract_variable_names(expression: &str) -> Result<BTreeSet<String>, UnsafeExpressionError> {
    let tokens = tokenize(expression)?;
    let mut names = BTreeSet::new();
    for index in 0..tokens.len() {
        if let Token::Identifier(name) = &tokens[index] {
            if matches!(tokens.get(index + 1), Some(Token::LeftParen)) {
                continue;
            }
            names.insert(name.clone());
        }
    }
    Ok(names)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComputedStatus {
    Ok,
    Pending,
    MissingRequired,
    Defaulted,
}

fn try_resolve_computed(
    mapping: &serde_json::Value,
    dims: &BTreeMap<String, serde_json::Value>,
    resolved: &BTreeMap<String, serde_json::Value>,
) -> Result<(ComputedStatus, serde_json::Value), ExpressionEvaluationError> {
    let required = mapping
        .get("required")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let default = mapping
        .get("default")
        .cloned()
        .unwrap_or_else(|| serde_json::json!(0));
    let expr = mapping
        .get("expression")
        .or_else(|| mapping.get("transform_expression"))
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if expr.is_empty() {
        return Ok((
            if required {
                ComputedStatus::MissingRequired
            } else {
                ComputedStatus::Defaulted
            },
            default,
        ));
    }

    let needed = extract_variable_names(expr)
        .map_err(|err| ExpressionEvaluationError::Failed(err.to_string()))?;
    let env: BTreeMap<String, serde_json::Value> = dims
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .chain(resolved.iter().map(|(k, v)| (k.clone(), v.clone())))
        .collect();
    if needed.iter().any(|name| !env.contains_key(name)) {
        return Ok((
            if required {
                ComputedStatus::Pending
            } else {
                ComputedStatus::Defaulted
            },
            default,
        ));
    }

    match evaluate_expression(expr, &env) {
        Ok(value) => Ok((ComputedStatus::Ok, serde_json::json!(value))),
        Err(err) if required => Err(ExpressionEvaluationError::Failed(err.to_string())),
        Err(_) => Ok((ComputedStatus::Defaulted, default)),
    }
}

fn resolve_mapping(
    var_name: &str,
    mapping: &serde_json::Value,
    dims: &BTreeMap<String, serde_json::Value>,
) -> MappingResolution {
    let source = mapping
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or("constant")
        .to_ascii_lowercase();

    match source.as_str() {
        "constant" => Ok((
            mapping
                .get("default")
                .cloned()
                .unwrap_or_else(|| serde_json::json!(0)),
            false,
            None,
        )),
        "dimension" => resolve_dimension(var_name, mapping, dims),
        "tiered" => resolve_tiered(var_name, mapping, dims),
        "matrix" => resolve_matrix(var_name, mapping, dims),
        _ => Ok((
            mapping
                .get("default")
                .cloned()
                .unwrap_or_else(|| serde_json::json!(0)),
            false,
            None,
        )),
    }
}

fn resolve_dimension(
    var_name: &str,
    mapping: &serde_json::Value,
    dims: &BTreeMap<String, serde_json::Value>,
) -> MappingResolution {
    let key = mapping
        .get("key")
        .and_then(|v| v.as_str())
        .unwrap_or(var_name);
    let required = mapping
        .get("required")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let allow_zero = mapping
        .get("allow_zero")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let default = mapping
        .get("default")
        .cloned()
        .unwrap_or_else(|| serde_json::json!(0));

    let Some(value) = dims.get(key).cloned() else {
        return Ok((default, required, None));
    };
    if !allow_zero && is_zero_like(&value) {
        return Ok((default, required, None));
    }
    Ok((value, false, None))
}

fn resolve_tiered(
    _var_name: &str,
    mapping: &serde_json::Value,
    dims: &BTreeMap<String, serde_json::Value>,
) -> MappingResolution {
    let tier_key = mapping
        .get("tier_key")
        .and_then(|v| v.as_str())
        .unwrap_or("total_input_context");
    let default = mapping
        .get("default")
        .cloned()
        .unwrap_or_else(|| serde_json::json!(0));
    let Some(tiers) = mapping.get("tiers").and_then(|v| v.as_array()) else {
        return Ok((default, false, None));
    };

    let tier_value = dims.get(tier_key).and_then(as_f64).unwrap_or(0.0);
    let ttl_key = mapping.get("ttl_key").and_then(|v| v.as_str());
    let ttl_value_key = mapping.get("ttl_value_key").and_then(|v| v.as_str());
    let cache_ttl_minutes = ttl_key.and_then(|key| dims.get(key)).and_then(as_f64);

    let mut matched_index = None;
    let mut matched_tier = None;
    for (index, tier) in tiers.iter().enumerate() {
        let up_to = tier.get("up_to").and_then(as_f64);
        if up_to.is_none() || tier_value <= up_to.unwrap_or_default() {
            matched_index = Some(index as i64);
            matched_tier = Some(tier.clone());
            break;
        }
    }
    let Some(tier) = matched_tier.or_else(|| tiers.last().cloned()) else {
        return Ok((default, false, None));
    };

    if let (Some(cache_ttl_minutes), Some(ttl_value_key)) = (cache_ttl_minutes, ttl_value_key) {
        if let Some(ttl_pricing) = tier.get("cache_ttl_pricing").and_then(|v| v.as_array()) {
            if let Some(ttl_entry) = ttl_pricing.iter().find(|entry| {
                entry
                    .get("ttl_minutes")
                    .and_then(as_f64)
                    .map(|value| value == cache_ttl_minutes)
                    .unwrap_or(false)
            }) {
                if let Some(value) = ttl_entry.get(ttl_value_key).filter(|value| !value.is_null()) {
                    return Ok((
                        value.clone(),
                        false,
                        Some((matched_index.unwrap_or(0), tier)),
                    ));
                }
            }
        }
    }

    Ok((
        tier.get("value").cloned().unwrap_or(default),
        false,
        Some((matched_index.unwrap_or(0), tier)),
    ))
}

fn resolve_matrix(
    var_name: &str,
    mapping: &serde_json::Value,
    dims: &BTreeMap<String, serde_json::Value>,
) -> MappingResolution {
    let key = mapping
        .get("key")
        .and_then(|v| v.as_str())
        .unwrap_or(var_name);
    let default = mapping
        .get("default")
        .cloned()
        .unwrap_or_else(|| serde_json::json!(0));
    let Some(value) = dims.get(key) else {
        let required = mapping
            .get("required")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        return Ok((default, required, None));
    };
    let Some(entries) = mapping.get("entries").and_then(|v| v.as_object()) else {
        return Ok((default, false, None));
    };
    let lookup_key = match value {
        serde_json::Value::String(text) => text.clone(),
        _ => value.to_string(),
    };
    Ok((
        entries.get(&lookup_key).cloned().unwrap_or(default),
        false,
        None,
    ))
}

fn is_zero_like(value: &serde_json::Value) -> bool {
    as_f64(value).map(|number| number == 0.0).unwrap_or(false)
}

fn as_f64(value: &serde_json::Value) -> Option<f64> {
    value.as_f64().or_else(|| value.as_i64().map(|v| v as f64))
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Identifier(String),
    Plus,
    Minus,
    Star,
    DoubleStar,
    Slash,
    DoubleSlash,
    Percent,
    LeftParen,
    RightParen,
    Comma,
}

fn tokenize(input: &str) -> Result<Vec<Token>, UnsafeExpressionError> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            ch if ch.is_ascii_whitespace() => index += 1,
            ch if ch.is_ascii_digit() || ch == '.' => {
                let start = index;
                index += 1;
                while index < chars.len() && (chars[index].is_ascii_digit() || chars[index] == '.')
                {
                    index += 1;
                }
                let text: String = chars[start..index].iter().collect();
                let number = text.parse::<f64>().map_err(|_| {
                    UnsafeExpressionError::Unsupported(format!("invalid numeric literal: {text}"))
                })?;
                tokens.push(Token::Number(number));
            }
            ch if ch.is_ascii_alphabetic() || ch == '_' => {
                let start = index;
                index += 1;
                while index < chars.len()
                    && (chars[index].is_ascii_alphanumeric() || chars[index] == '_')
                {
                    index += 1;
                }
                tokens.push(Token::Identifier(chars[start..index].iter().collect()));
            }
            '+' => {
                tokens.push(Token::Plus);
                index += 1;
            }
            '-' => {
                tokens.push(Token::Minus);
                index += 1;
            }
            '*' => {
                if matches!(chars.get(index + 1), Some('*')) {
                    tokens.push(Token::DoubleStar);
                    index += 2;
                } else {
                    tokens.push(Token::Star);
                    index += 1;
                }
            }
            '/' => {
                if matches!(chars.get(index + 1), Some('/')) {
                    tokens.push(Token::DoubleSlash);
                    index += 2;
                } else {
                    tokens.push(Token::Slash);
                    index += 1;
                }
            }
            '%' => {
                tokens.push(Token::Percent);
                index += 1;
            }
            '(' => {
                tokens.push(Token::LeftParen);
                index += 1;
            }
            ')' => {
                tokens.push(Token::RightParen);
                index += 1;
            }
            ',' => {
                tokens.push(Token::Comma);
                index += 1;
            }
            other => {
                return Err(UnsafeExpressionError::Unsupported(format!(
                    "unsupported token: {other}"
                )));
            }
        }
    }
    Ok(tokens)
}

fn evaluate_expression(
    expression: &str,
    variables: &BTreeMap<String, serde_json::Value>,
) -> Result<f64, UnsafeExpressionError> {
    let tokens = tokenize(expression)?;
    let mut parser = Parser {
        tokens: &tokens,
        index: 0,
        variables,
    };
    let value = parser.parse_expression()?;
    if parser.index != tokens.len() {
        return Err(UnsafeExpressionError::Unsupported(
            "unexpected trailing tokens".to_string(),
        ));
    }
    Ok(value)
}

struct Parser<'a> {
    tokens: &'a [Token],
    index: usize,
    variables: &'a BTreeMap<String, serde_json::Value>,
}

impl<'a> Parser<'a> {
    fn parse_expression(&mut self) -> Result<f64, UnsafeExpressionError> {
        let mut left = self.parse_term()?;
        loop {
            match self.peek() {
                Some(Token::Plus) => {
                    self.index += 1;
                    left += self.parse_term()?;
                }
                Some(Token::Minus) => {
                    self.index += 1;
                    left -= self.parse_term()?;
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_term(&mut self) -> Result<f64, UnsafeExpressionError> {
        let mut left = self.parse_power()?;
        loop {
            match self.peek() {
                Some(Token::Star) => {
                    self.index += 1;
                    left *= self.parse_power()?;
                }
                Some(Token::Slash) => {
                    self.index += 1;
                    left /= self.parse_power()?;
                }
                Some(Token::DoubleSlash) => {
                    self.index += 1;
                    left = (left / self.parse_power()?).floor();
                }
                Some(Token::Percent) => {
                    self.index += 1;
                    left %= self.parse_power()?;
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_power(&mut self) -> Result<f64, UnsafeExpressionError> {
        let left = self.parse_unary()?;
        if matches!(self.peek(), Some(Token::DoubleStar)) {
            self.index += 1;
            let right = self.parse_power()?;
            return Ok(left.powf(right));
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<f64, UnsafeExpressionError> {
        match self.peek() {
            Some(Token::Plus) => {
                self.index += 1;
                self.parse_unary()
            }
            Some(Token::Minus) => {
                self.index += 1;
                Ok(-self.parse_unary()?)
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<f64, UnsafeExpressionError> {
        match self.next() {
            Some(Token::Number(value)) => Ok(*value),
            Some(Token::Identifier(name)) => {
                if matches!(self.peek(), Some(Token::LeftParen)) {
                    self.index += 1;
                    let args = self.parse_arguments()?;
                    evaluate_function(name, &args)
                } else {
                    let value = self.variables.get(name).and_then(as_f64).ok_or_else(|| {
                        UnsafeExpressionError::Unsupported(format!("unknown variable: {name}"))
                    })?;
                    Ok(value)
                }
            }
            Some(Token::LeftParen) => {
                let value = self.parse_expression()?;
                match self.next() {
                    Some(Token::RightParen) => Ok(value),
                    _ => Err(UnsafeExpressionError::Unsupported(
                        "missing closing ')'".to_string(),
                    )),
                }
            }
            other => Err(UnsafeExpressionError::Unsupported(format!(
                "unexpected token: {other:?}"
            ))),
        }
    }

    fn parse_arguments(&mut self) -> Result<Vec<f64>, UnsafeExpressionError> {
        let mut args = Vec::new();
        if matches!(self.peek(), Some(Token::RightParen)) {
            self.index += 1;
            return Ok(args);
        }
        loop {
            args.push(self.parse_expression()?);
            match self.next() {
                Some(Token::Comma) => {}
                Some(Token::RightParen) => break,
                other => {
                    return Err(UnsafeExpressionError::Unsupported(format!(
                        "unexpected token in function call: {other:?}"
                    )));
                }
            }
        }
        Ok(args)
    }

    fn peek(&self) -> Option<&'a Token> {
        self.tokens.get(self.index)
    }

    fn next(&mut self) -> Option<&'a Token> {
        let token = self.tokens.get(self.index);
        if token.is_some() {
            self.index += 1;
        }
        token
    }
}

fn evaluate_function(name: &str, args: &[f64]) -> Result<f64, UnsafeExpressionError> {
    match name {
        "min" => args.iter().copied().reduce(f64::min).ok_or_else(|| {
            UnsafeExpressionError::Unsupported("min() requires arguments".to_string())
        }),
        "max" => args.iter().copied().reduce(f64::max).ok_or_else(|| {
            UnsafeExpressionError::Unsupported("max() requires arguments".to_string())
        }),
        "abs" => args.first().copied().map(f64::abs).ok_or_else(|| {
            UnsafeExpressionError::Unsupported("abs() requires one argument".to_string())
        }),
        "round" => {
            let Some(value) = args.first().copied() else {
                return Err(UnsafeExpressionError::Unsupported(
                    "round() requires at least one argument".to_string(),
                ));
            };
            let digits = args.get(1).copied().unwrap_or(0.0) as i32;
            let factor = 10_f64.powi(digits);
            Ok((value * factor).round() / factor)
        }
        "int" => args
            .first()
            .copied()
            .map(|value| value.trunc())
            .ok_or_else(|| {
                UnsafeExpressionError::Unsupported("int() requires one argument".to_string())
            }),
        "float" => args.first().copied().ok_or_else(|| {
            UnsafeExpressionError::Unsupported("float() requires one argument".to_string())
        }),
        _ => Err(UnsafeExpressionError::Unsupported(format!(
            "function not allowed: {name}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::{extract_variable_names, FormulaEngine, FormulaEvaluationStatus};
    use std::collections::BTreeMap;

    #[test]
    fn extracts_variable_names_without_functions() {
        let names = extract_variable_names("min(input_cost, output_cost) + tax")
            .expect("variables should parse");
        assert!(names.contains("input_cost"));
        assert!(names.contains("output_cost"));
        assert!(names.contains("tax"));
        assert!(!names.contains("min"));
    }

    #[test]
    fn evaluates_formula_with_computed_variables() {
        let engine = FormulaEngine::new();
        let dimensions = BTreeMap::from([
            ("input_tokens".to_string(), serde_json::json!(1000)),
            ("output_tokens".to_string(), serde_json::json!(500)),
        ]);
        let variables = BTreeMap::from([
            ("input_price_per_1m".to_string(), serde_json::json!(3.0)),
            ("output_price_per_1m".to_string(), serde_json::json!(15.0)),
        ]);
        let mappings = BTreeMap::from([
            (
                "input_cost".to_string(),
                serde_json::json!({
                    "source": "computed",
                    "expression": "input_tokens * input_price_per_1m / 1000000"
                }),
            ),
            (
                "output_cost".to_string(),
                serde_json::json!({
                    "source": "computed",
                    "expression": "output_tokens * output_price_per_1m / 1000000"
                }),
            ),
        ]);

        let result = engine
            .evaluate(
                "input_cost + output_cost",
                Some(&variables),
                Some(&dimensions),
                Some(&mappings),
                false,
            )
            .expect("formula should evaluate");

        assert_eq!(result.status, FormulaEvaluationStatus::Complete);
        assert_eq!(result.cost, 0.0105);
        assert_eq!(result.cost_breakdown.get("input_cost"), Some(&0.003_f64));
        assert_eq!(result.cost_breakdown.get("output_cost"), Some(&0.0075_f64));
    }

    #[test]
    fn resolves_tiered_mapping() {
        let engine = FormulaEngine::new();
        let dimensions =
            BTreeMap::from([("total_input_context".to_string(), serde_json::json!(2000))]);
        let mappings = BTreeMap::from([(
            "input_price_per_1m".to_string(),
            serde_json::json!({
                "source": "tiered",
                "tier_key": "total_input_context",
                "tiers": [
                    { "up_to": 1000, "value": 1.0 },
                    { "up_to": 4000, "value": 2.0 }
                ],
                "default": 0.0
            }),
        )]);

        let result = engine
            .evaluate(
                "input_price_per_1m",
                None,
                Some(&dimensions),
                Some(&mappings),
                false,
            )
            .expect("tiered mapping should evaluate");

        assert_eq!(result.cost, 2.0);
        assert_eq!(result.tier_index, Some(1));
    }

    #[test]
    fn incomplete_when_required_dimension_missing() {
        let engine = FormulaEngine::new();
        let mappings = BTreeMap::from([(
            "input_tokens".to_string(),
            serde_json::json!({
                "source": "dimension",
                "key": "input_tokens",
                "required": true
            }),
        )]);

        let result = engine
            .evaluate("input_tokens", None, None, Some(&mappings), false)
            .expect("incomplete result should return ok");

        assert_eq!(result.status, FormulaEvaluationStatus::Incomplete);
        assert_eq!(result.missing_required, vec!["input_tokens".to_string()]);
    }

    #[test]
    fn ttl_pricing_requires_exact_match() {
        let engine = FormulaEngine::new();
        let dimensions = BTreeMap::from([
            ("total_input_context".to_string(), serde_json::json!(22_562)),
            ("cache_ttl_minutes".to_string(), serde_json::json!(5)),
        ]);
        let mappings = BTreeMap::from([(
            "cache_creation_price_per_1m".to_string(),
            serde_json::json!({
                "source": "tiered",
                "tier_key": "total_input_context",
                "ttl_key": "cache_ttl_minutes",
                "ttl_value_key": "cache_creation_price_per_1m",
                "tiers": [{
                    "up_to": null,
                    "value": 3.125,
                    "cache_ttl_pricing": [{
                        "ttl_minutes": 60,
                        "cache_creation_price_per_1m": 5.0
                    }]
                }],
                "default": 0.0
            }),
        )]);

        let result = engine
            .evaluate(
                "cache_creation_price_per_1m",
                None,
                Some(&dimensions),
                Some(&mappings),
                false,
            )
            .expect("tiered mapping should evaluate");

        assert_eq!(result.cost, 3.125);
    }

    #[test]
    fn ttl_pricing_null_value_falls_back_to_base_tier_value() {
        let engine = FormulaEngine::new();
        let dimensions = BTreeMap::from([
            ("total_input_context".to_string(), serde_json::json!(22_562)),
            ("cache_ttl_minutes".to_string(), serde_json::json!(60)),
        ]);
        let mappings = BTreeMap::from([(
            "cache_read_price_per_1m".to_string(),
            serde_json::json!({
                "source": "tiered",
                "tier_key": "total_input_context",
                "ttl_key": "cache_ttl_minutes",
                "ttl_value_key": "cache_read_price_per_1m",
                "tiers": [{
                    "up_to": null,
                    "value": 0.25,
                    "cache_ttl_pricing": [{
                        "ttl_minutes": 60,
                        "cache_read_price_per_1m": null
                    }]
                }],
                "default": 0.0
            }),
        )]);

        let result = engine
            .evaluate(
                "cache_read_price_per_1m",
                None,
                Some(&dimensions),
                Some(&mappings),
                false,
            )
            .expect("tiered mapping should evaluate");

        assert_eq!(result.cost, 0.25);
    }
}
