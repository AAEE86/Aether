mod auth;
mod request;
mod url;

pub use auth::{
    is_gemini_cli_provider_transport, resolve_local_gemini_cli_request_auth, GeminiCliRequestAuth,
    GeminiCliRequestAuthSupport, GeminiCliRequestAuthUnsupportedReason, GEMINI_CLI_PROVIDER_TYPE,
};
pub use request::{
    build_gemini_cli_v1internal_request, classify_gemini_cli_v1internal_request_body,
    GeminiCliRequestEnvelopeSupport, GeminiCliRequestEnvelopeUnsupportedReason,
};
pub use url::{
    build_gemini_cli_v1internal_url, GeminiCliRequestUrlAction, GEMINI_CLI_V1INTERNAL_PATH_TEMPLATE,
};
