use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::default_rule::{normalize_task_type, DefaultBillingRuleGenerator};
use crate::precision::quantize_cost;
use crate::pricing::{BillingComputation, BillingModelPricingSnapshot, BillingUsageInput};
use crate::schema::{
    BillingSnapshot, BillingSnapshotStatus, CostResult, BILLING_SNAPSHOT_SCHEMA_VERSION,
};
use crate::{
    normalize_input_tokens_for_billing, ExpressionEvaluationError, FormulaEngine,
    FormulaEvaluationStatus,
};

pub struct BillingService {
    engine: FormulaEngine,
}

impl BillingService {
    pub fn new() -> Self {
        Self {
            engine: FormulaEngine::new(),
        }
    }

    pub fn calculate(
        &self,
        pricing: &BillingModelPricingSnapshot,
        input: &BillingUsageInput,
    ) -> Result<BillingComputation, ExpressionEvaluationError> {
        let Some(rule) =
            DefaultBillingRuleGenerator::generate_for_pricing(pricing, &input.task_type)
        else {
            return Ok(BillingComputation {
                cost_result: CostResult {
                    cost: 0.0,
                    status: BillingSnapshotStatus::NoRule,
                    snapshot: BillingSnapshot {
                        schema_version: BILLING_SNAPSHOT_SCHEMA_VERSION.to_string(),
                        rule_id: None,
                        rule_name: None,
                        scope: None,
                        expression: None,
                        resolved_dimensions: build_dimensions(input),
                        resolved_variables: BTreeMap::new(),
                        cost_breakdown: BTreeMap::new(),
                        total_cost: 0.0,
                        tier_index: None,
                        tier_info: None,
                        missing_required: Vec::new(),
                        status: BillingSnapshotStatus::NoRule,
                        calculated_at: now_marker(),
                        engine_version: "2.0".to_string(),
                    },
                },
                actual_total_cost: 0.0,
                rate_multiplier: pricing
                    .rate_multiplier_for_api_format(input.api_format.as_deref()),
                is_free_tier: pricing.is_free_tier(),
            });
        };

        let dims = build_dimensions(input);
        let result = self.engine.evaluate(
            &rule.expression,
            Some(&rule.variables),
            Some(&dims),
            Some(&rule.dimension_mappings),
            false,
        )?;

        let status = match result.status {
            FormulaEvaluationStatus::Complete => BillingSnapshotStatus::Complete,
            FormulaEvaluationStatus::Incomplete => BillingSnapshotStatus::Incomplete,
        };
        let total_cost = if matches!(status, BillingSnapshotStatus::Complete) {
            result.cost
        } else {
            0.0
        };
        let rate_multiplier = pricing.rate_multiplier_for_api_format(input.api_format.as_deref());
        let is_free_tier = pricing.is_free_tier();
        let actual_total_cost = if is_free_tier {
            0.0
        } else {
            quantize_cost(total_cost * rate_multiplier)
        };

        Ok(BillingComputation {
            cost_result: CostResult {
                cost: total_cost,
                status,
                snapshot: BillingSnapshot {
                    schema_version: BILLING_SNAPSHOT_SCHEMA_VERSION.to_string(),
                    rule_id: Some(rule.id),
                    rule_name: Some(rule.name),
                    scope: Some(rule.scope),
                    expression: Some(rule.expression),
                    resolved_dimensions: result.resolved_dimensions,
                    resolved_variables: result.resolved_variables,
                    cost_breakdown: result.cost_breakdown,
                    total_cost,
                    tier_index: result.tier_index,
                    tier_info: result.tier_info,
                    missing_required: result.missing_required,
                    status,
                    calculated_at: now_marker(),
                    engine_version: "2.0".to_string(),
                },
            },
            actual_total_cost,
            rate_multiplier,
            is_free_tier,
        })
    }
}

impl Default for BillingService {
    fn default() -> Self {
        Self::new()
    }
}

fn build_dimensions(input: &BillingUsageInput) -> BTreeMap<String, Value> {
    let normalized_input_tokens = normalize_input_tokens_for_billing(
        input.api_format.as_deref(),
        input.input_tokens,
        input.cache_read_tokens,
    );
    let classified_cache_creation_tokens = input
        .cache_creation_ephemeral_5m_tokens
        .saturating_add(input.cache_creation_ephemeral_1h_tokens);
    let cache_creation_uncategorized_tokens = input
        .cache_creation_tokens
        .saturating_sub(classified_cache_creation_tokens)
        .max(0);
    let total_input_context = input
        .input_tokens
        .saturating_add(input.cache_creation_tokens)
        .saturating_add(input.cache_read_tokens);

    let mut out = BTreeMap::from([
        ("input_tokens".to_string(), json!(normalized_input_tokens)),
        ("output_tokens".to_string(), json!(input.output_tokens)),
        (
            "cache_creation_tokens".to_string(),
            json!(input.cache_creation_tokens),
        ),
        (
            "cache_creation_ephemeral_5m_tokens".to_string(),
            json!(input.cache_creation_ephemeral_5m_tokens),
        ),
        (
            "cache_creation_ephemeral_1h_tokens".to_string(),
            json!(input.cache_creation_ephemeral_1h_tokens),
        ),
        (
            "cache_creation_uncategorized_tokens".to_string(),
            json!(cache_creation_uncategorized_tokens),
        ),
        (
            "cache_read_tokens".to_string(),
            json!(input.cache_read_tokens),
        ),
        (
            "request_count".to_string(),
            json!(input.request_count.max(0)),
        ),
        (
            "total_input_context".to_string(),
            json!(total_input_context),
        ),
        (
            "effective_task_type".to_string(),
            json!(normalize_task_type(&input.task_type)),
        ),
    ]);

    out.insert(
        "cache_creation_ephemeral_5m_ttl_minutes".to_string(),
        json!(5),
    );
    out.insert(
        "cache_creation_ephemeral_1h_ttl_minutes".to_string(),
        json!(60),
    );

    if let Some(cache_ttl_minutes) = input.cache_ttl_minutes {
        out.insert(
            "cache_ttl_minutes".to_string(),
            json!(cache_ttl_minutes.max(0)),
        );
    }
    out
}

fn now_marker() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::BillingService;
    use crate::{BillingModelPricingSnapshot, BillingSnapshotStatus, BillingUsageInput};

    fn pricing() -> BillingModelPricingSnapshot {
        BillingModelPricingSnapshot {
            provider_id: "provider-1".to_string(),
            provider_billing_type: Some("pay_as_you_go".to_string()),
            provider_api_key_id: Some("key-1".to_string()),
            provider_api_key_rate_multipliers: Some(json!({"openai:chat": 0.5})),
            provider_api_key_cache_ttl_minutes: Some(60),
            global_model_id: "global-model-1".to_string(),
            global_model_name: "gpt-5".to_string(),
            global_model_config: None,
            default_price_per_request: Some(0.02),
            default_tiered_pricing: Some(json!({
                "tiers": [{
                    "up_to": null,
                    "input_price_per_1m": 3.0,
                    "output_price_per_1m": 15.0,
                    "cache_creation_price_per_1m": 3.75,
                    "cache_read_price_per_1m": 0.30
                }]
            })),
            model_id: Some("model-1".to_string()),
            model_provider_model_name: Some("gpt-5-upstream".to_string()),
            model_config: None,
            model_price_per_request: None,
            model_tiered_pricing: None,
        }
    }

    #[test]
    fn calculates_complete_snapshot_for_usage() {
        let result = BillingService::new()
            .calculate(
                &pricing(),
                &BillingUsageInput {
                    task_type: "chat".to_string(),
                    api_format: Some("openai:chat".to_string()),
                    request_count: 1,
                    input_tokens: 1_000,
                    output_tokens: 500,
                    cache_creation_tokens: 0,
                    cache_creation_ephemeral_5m_tokens: 0,
                    cache_creation_ephemeral_1h_tokens: 0,
                    cache_read_tokens: 100,
                    cache_ttl_minutes: Some(60),
                },
            )
            .expect("billing should calculate");

        assert_eq!(result.cost_result.status, BillingSnapshotStatus::Complete);
        assert!(result.cost_result.cost > 0.0);
        assert!(result.actual_total_cost > 0.0);
        assert_eq!(result.rate_multiplier, 0.5);
    }

    #[test]
    fn five_minute_cache_ttl_uses_base_cache_prices() {
        let pricing = BillingModelPricingSnapshot {
            provider_id: "provider-1".to_string(),
            provider_billing_type: Some("pay_as_you_go".to_string()),
            provider_api_key_id: Some("key-1".to_string()),
            provider_api_key_rate_multipliers: None,
            provider_api_key_cache_ttl_minutes: Some(5),
            global_model_id: "global-model-1".to_string(),
            global_model_name: "gpt-5.4".to_string(),
            global_model_config: None,
            default_price_per_request: None,
            default_tiered_pricing: Some(json!({
                "tiers": [{
                    "up_to": null,
                    "input_price_per_1m": 2.5,
                    "output_price_per_1m": 15.0,
                    "cache_creation_price_per_1m": 3.125,
                    "cache_read_price_per_1m": 0.25,
                    "cache_ttl_pricing": [{
                        "ttl_minutes": 60,
                        "cache_creation_price_per_1m": 5.0,
                        "cache_read_price_per_1m": null
                    }]
                }]
            })),
            model_id: None,
            model_provider_model_name: None,
            model_config: None,
            model_price_per_request: None,
            model_tiered_pricing: None,
        };

        let result = BillingService::new()
            .calculate(
                &pricing,
                &BillingUsageInput {
                    task_type: "chat".to_string(),
                    api_format: None,
                    request_count: 1,
                    input_tokens: 1_000,
                    output_tokens: 10,
                    cache_creation_tokens: 0,
                    cache_creation_ephemeral_5m_tokens: 0,
                    cache_creation_ephemeral_1h_tokens: 0,
                    cache_read_tokens: 100,
                    cache_ttl_minutes: Some(5),
                },
            )
            .expect("billing should calculate");

        assert_eq!(
            result
                .cost_result
                .snapshot
                .resolved_variables
                .get("cache_creation_price_per_1m"),
            Some(&json!(3.125))
        );
        assert_eq!(
            result
                .cost_result
                .snapshot
                .resolved_variables
                .get("cache_read_price_per_1m"),
            Some(&json!(0.25))
        );
    }

    #[test]
    fn one_hour_cache_ttl_keeps_base_cache_read_when_ttl_entry_omits_it() {
        let pricing = BillingModelPricingSnapshot {
            provider_id: "provider-1".to_string(),
            provider_billing_type: Some("pay_as_you_go".to_string()),
            provider_api_key_id: Some("key-1".to_string()),
            provider_api_key_rate_multipliers: None,
            provider_api_key_cache_ttl_minutes: Some(60),
            global_model_id: "global-model-1".to_string(),
            global_model_name: "gpt-5.4".to_string(),
            global_model_config: None,
            default_price_per_request: None,
            default_tiered_pricing: Some(json!({
                "tiers": [{
                    "up_to": null,
                    "input_price_per_1m": 2.5,
                    "output_price_per_1m": 15.0,
                    "cache_creation_price_per_1m": 3.125,
                    "cache_read_price_per_1m": 0.25,
                    "cache_ttl_pricing": [{
                        "ttl_minutes": 60,
                        "cache_creation_price_per_1m": 5.0,
                        "cache_read_price_per_1m": null
                    }]
                }]
            })),
            model_id: None,
            model_provider_model_name: None,
            model_config: None,
            model_price_per_request: None,
            model_tiered_pricing: None,
        };

        let result = BillingService::new()
            .calculate(
                &pricing,
                &BillingUsageInput {
                    task_type: "chat".to_string(),
                    api_format: None,
                    request_count: 1,
                    input_tokens: 1_000,
                    output_tokens: 10,
                    cache_creation_tokens: 0,
                    cache_creation_ephemeral_5m_tokens: 0,
                    cache_creation_ephemeral_1h_tokens: 0,
                    cache_read_tokens: 100,
                    cache_ttl_minutes: Some(60),
                },
            )
            .expect("billing should calculate");

        assert_eq!(
            result
                .cost_result
                .snapshot
                .resolved_variables
                .get("cache_creation_price_per_1m"),
            Some(&json!(5.0))
        );
        assert_eq!(
            result
                .cost_result
                .snapshot
                .resolved_variables
                .get("cache_read_price_per_1m"),
            Some(&json!(0.25))
        );
    }
}
