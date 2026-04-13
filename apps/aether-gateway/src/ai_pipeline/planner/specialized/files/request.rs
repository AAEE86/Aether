use std::collections::BTreeMap;

use serde_json::json;

use crate::ai_pipeline::contracts::GEMINI_FILES_UPLOAD_PLAN_KIND;
use crate::ai_pipeline::planner::spec_metadata::local_gemini_files_spec_metadata;
use crate::ai_pipeline::transport::auth::{
    build_passthrough_headers_with_auth, resolve_local_gemini_auth,
};
use crate::ai_pipeline::transport::policy::supports_local_gemini_transport_with_network;
use crate::ai_pipeline::transport::url::build_gemini_files_passthrough_url;
use crate::ai_pipeline::transport::{apply_local_body_rules, apply_local_header_rules};
use crate::ai_pipeline::GatewayProviderTransportSnapshot;
use crate::AppState;

use super::support::{
    mark_skipped_local_gemini_files_candidate, LocalGeminiFilesCandidateAttempt,
    LocalGeminiFilesDecisionInput, GEMINI_FILES_CANDIDATE_API_FORMAT,
};
use super::LocalGeminiFilesSpec;

pub(super) struct LocalGeminiFilesCandidatePayloadParts {
    pub(super) transport: GatewayProviderTransportSnapshot,
    pub(super) auth_header: String,
    pub(super) auth_value: String,
    pub(super) provider_request_headers: BTreeMap<String, String>,
    pub(super) provider_request_body: Option<serde_json::Value>,
    pub(super) provider_request_body_base64: Option<String>,
    pub(super) original_request_body: serde_json::Value,
    pub(super) upstream_url: String,
    pub(super) file_name: String,
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn resolve_local_gemini_files_candidate_payload_parts(
    state: &AppState,
    parts: &http::request::Parts,
    body_json: &serde_json::Value,
    body_base64: Option<&str>,
    body_is_empty: bool,
    trace_id: &str,
    input: &LocalGeminiFilesDecisionInput,
    attempt: &LocalGeminiFilesCandidateAttempt,
    spec: LocalGeminiFilesSpec,
) -> Option<LocalGeminiFilesCandidatePayloadParts> {
    let spec_metadata = local_gemini_files_spec_metadata(spec);
    let candidate = &attempt.eligible.candidate;
    let transport = &attempt.eligible.transport;

    if !supports_local_gemini_transport_with_network(transport, GEMINI_FILES_CANDIDATE_API_FORMAT) {
        mark_skipped_local_gemini_files_candidate(
            state,
            input,
            trace_id,
            candidate,
            attempt.candidate_index,
            &attempt.candidate_id,
            "transport_unsupported",
        )
        .await;
        return None;
    }

    let Some((auth_header, auth_value)) = resolve_local_gemini_auth(transport) else {
        mark_skipped_local_gemini_files_candidate(
            state,
            input,
            trace_id,
            candidate,
            attempt.candidate_index,
            &attempt.candidate_id,
            "transport_auth_unavailable",
        )
        .await;
        return None;
    };

    let custom_path = transport
        .endpoint
        .custom_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let passthrough_path = custom_path.unwrap_or(parts.uri.path());
    let Some(upstream_url) = build_gemini_files_passthrough_url(
        &transport.endpoint.base_url,
        passthrough_path,
        parts.uri.query(),
    ) else {
        mark_skipped_local_gemini_files_candidate(
            state,
            input,
            trace_id,
            candidate,
            attempt.candidate_index,
            &attempt.candidate_id,
            "upstream_url_missing",
        )
        .await;
        return None;
    };

    let mut provider_request_body = if spec_metadata.decision_kind == GEMINI_FILES_UPLOAD_PLAN_KIND
        && !body_is_empty
        && body_base64.is_none()
    {
        Some(body_json.clone())
    } else {
        None
    };
    let provider_request_body_base64 =
        if spec_metadata.decision_kind == GEMINI_FILES_UPLOAD_PLAN_KIND {
            body_base64
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        } else {
            None
        };
    let original_request_body = if let Some(body_bytes_b64) = provider_request_body_base64.clone() {
        json!({"body_bytes_b64": body_bytes_b64})
    } else if !body_is_empty {
        body_json.clone()
    } else {
        serde_json::Value::Null
    };
    if provider_request_body_base64.is_some() && transport.endpoint.body_rules.is_some() {
        mark_skipped_local_gemini_files_candidate(
            state,
            input,
            trace_id,
            candidate,
            attempt.candidate_index,
            &attempt.candidate_id,
            "transport_body_rules_unsupported_for_binary_upload",
        )
        .await;
        return None;
    }
    if let Some(body) = provider_request_body.as_mut() {
        if !apply_local_body_rules(
            body,
            transport.endpoint.body_rules.as_ref(),
            Some(body_json),
        ) {
            mark_skipped_local_gemini_files_candidate(
                state,
                input,
                trace_id,
                candidate,
                attempt.candidate_index,
                &attempt.candidate_id,
                "transport_body_rules_apply_failed",
            )
            .await;
            return None;
        }
    }

    let mut provider_request_headers = build_passthrough_headers_with_auth(
        &parts.headers,
        &auth_header,
        &auth_value,
        &BTreeMap::new(),
    );
    if !apply_local_header_rules(
        &mut provider_request_headers,
        transport.endpoint.header_rules.as_ref(),
        &[&auth_header, "content-type"],
        provider_request_body
            .as_ref()
            .unwrap_or(&original_request_body),
        Some(&original_request_body),
    ) {
        mark_skipped_local_gemini_files_candidate(
            state,
            input,
            trace_id,
            candidate,
            attempt.candidate_index,
            &attempt.candidate_id,
            "transport_header_rules_apply_failed",
        )
        .await;
        return None;
    }

    let file_name = parts
        .uri
        .path()
        .trim_start_matches("/v1beta/")
        .trim()
        .to_string();

    Some(LocalGeminiFilesCandidatePayloadParts {
        transport: transport.clone(),
        auth_header,
        auth_value,
        provider_request_headers,
        provider_request_body,
        provider_request_body_base64,
        original_request_body,
        upstream_url,
        file_name,
    })
}
