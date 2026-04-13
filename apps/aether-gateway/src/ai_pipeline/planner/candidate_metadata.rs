use serde_json::{Map, Value};

use crate::ai_pipeline::planner::candidate_eligibility::EligibleLocalExecutionCandidate;
use crate::ai_pipeline::{ConversionMode, ExecutionStrategy};
use crate::append_execution_contract_fields_to_value;

pub(crate) struct LocalExecutionCandidateMetadataParts<'a> {
    pub(crate) eligible: &'a EligibleLocalExecutionCandidate,
    pub(crate) provider_api_format: &'a str,
    pub(crate) client_api_format: &'a str,
    pub(crate) extra_fields: Map<String, Value>,
}

pub(crate) fn build_local_execution_candidate_metadata(
    parts: LocalExecutionCandidateMetadataParts<'_>,
) -> Value {
    let candidate = &parts.eligible.candidate;
    let mut object = Map::new();
    object.insert(
        "provider_api_format".to_string(),
        Value::String(parts.provider_api_format.to_string()),
    );
    object.insert(
        "client_api_format".to_string(),
        Value::String(parts.client_api_format.to_string()),
    );
    object.insert(
        "global_model_id".to_string(),
        Value::String(candidate.global_model_id.clone()),
    );
    object.insert(
        "global_model_name".to_string(),
        Value::String(candidate.global_model_name.clone()),
    );
    object.insert(
        "model_id".to_string(),
        Value::String(candidate.model_id.clone()),
    );
    object.insert(
        "selected_provider_model_name".to_string(),
        Value::String(candidate.selected_provider_model_name.clone()),
    );
    object.insert(
        "mapping_matched_model".to_string(),
        candidate
            .mapping_matched_model
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
    );
    object.insert(
        "provider_name".to_string(),
        Value::String(candidate.provider_name.clone()),
    );
    object.insert(
        "key_name".to_string(),
        Value::String(candidate.key_name.clone()),
    );
    object.extend(parts.extra_fields);
    Value::Object(object)
}

pub(crate) fn build_local_execution_candidate_contract_metadata(
    parts: LocalExecutionCandidateMetadataParts<'_>,
    execution_strategy: ExecutionStrategy,
    conversion_mode: ConversionMode,
    provider_contract: &str,
) -> Value {
    let client_api_format = parts.client_api_format;
    append_execution_contract_fields_to_value(
        build_local_execution_candidate_metadata(parts),
        execution_strategy,
        conversion_mode,
        client_api_format,
        provider_contract,
    )
}
