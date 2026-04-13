use std::collections::BTreeMap;

use url::form_urlencoded;

use crate::ai_pipeline::GatewayProviderTransportSnapshot;

use super::super::{
    build_antigravity_v1internal_url, build_claude_code_messages_url, build_claude_messages_url,
    build_gemini_content_url, build_kiro_generate_assistant_response_url,
    build_passthrough_path_url, build_vertex_api_key_gemini_content_url,
    resolve_local_vertex_api_key_query_auth, AntigravityRequestUrlAction,
    LocalSameFormatProviderFamily, LocalSameFormatProviderSpec,
};

pub(crate) fn build_same_format_upstream_url(
    parts: &http::request::Parts,
    transport: &GatewayProviderTransportSnapshot,
    mapped_model: &str,
    spec: LocalSameFormatProviderSpec,
    upstream_is_stream: bool,
    kiro_auth: Option<&crate::ai_pipeline::transport::kiro::KiroRequestAuth>,
) -> Option<String> {
    if let Some(kiro_auth) = kiro_auth {
        return build_kiro_generate_assistant_response_url(
            &transport.endpoint.base_url,
            parts.uri.query(),
            Some(kiro_auth.auth_config.effective_api_region()),
        );
    }
    if transport
        .provider
        .provider_type
        .trim()
        .eq_ignore_ascii_case("claude_code")
    {
        return Some(build_claude_code_messages_url(
            &transport.endpoint.base_url,
            parts.uri.query(),
        ));
    }
    if transport
        .provider
        .provider_type
        .trim()
        .eq_ignore_ascii_case("vertex_ai")
    {
        let auth = resolve_local_vertex_api_key_query_auth(transport)?;
        return build_vertex_api_key_gemini_content_url(
            mapped_model,
            upstream_is_stream,
            &auth.value,
            parts.uri.query(),
        );
    }
    if transport
        .provider
        .provider_type
        .trim()
        .eq_ignore_ascii_case("antigravity")
    {
        let query = parts.uri.query().map(|query| {
            form_urlencoded::parse(query.as_bytes())
                .into_owned()
                .collect::<BTreeMap<String, String>>()
        });
        return build_antigravity_v1internal_url(
            &transport.endpoint.base_url,
            if upstream_is_stream {
                AntigravityRequestUrlAction::StreamGenerateContent
            } else {
                AntigravityRequestUrlAction::GenerateContent
            },
            query.as_ref(),
        );
    }

    let custom_path = transport
        .endpoint
        .custom_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if let Some(path) = custom_path {
        let blocked_keys = match spec.family {
            LocalSameFormatProviderFamily::Standard => &[][..],
            LocalSameFormatProviderFamily::Gemini => &["key"][..],
        };
        let url = build_passthrough_path_url(
            &transport.endpoint.base_url,
            path,
            parts.uri.query(),
            blocked_keys,
        )?;
        return Some(maybe_add_gemini_stream_alt_sse(url, spec));
    }

    let url = match spec.family {
        LocalSameFormatProviderFamily::Standard => Some(build_claude_messages_url(
            &transport.endpoint.base_url,
            parts.uri.query(),
        )),
        LocalSameFormatProviderFamily::Gemini => build_gemini_content_url(
            &transport.endpoint.base_url,
            mapped_model,
            spec.require_streaming,
            parts.uri.query(),
        ),
    }?;

    Some(maybe_add_gemini_stream_alt_sse(url, spec))
}

fn maybe_add_gemini_stream_alt_sse(
    upstream_url: String,
    spec: LocalSameFormatProviderSpec,
) -> String {
    if spec.family != LocalSameFormatProviderFamily::Gemini || !spec.require_streaming {
        return upstream_url;
    }

    let has_alt = upstream_url
        .split_once('?')
        .map(|(_, query)| {
            form_urlencoded::parse(query.as_bytes())
                .any(|(key, _)| key.as_ref().eq_ignore_ascii_case("alt"))
        })
        .unwrap_or(false);
    if has_alt {
        return upstream_url;
    }

    if upstream_url.contains('?') {
        format!("{upstream_url}&alt=sse")
    } else {
        format!("{upstream_url}?alt=sse")
    }
}
