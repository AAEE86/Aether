use axum::{extract::Request, middleware::Next, response::Response};
use http::header::HeaderName;

/// Cloudflare headers to strip from incoming requests and outgoing responses.
/// Prevents leaking CF metadata to upstream providers or back to clients.
static CF_HEADERS: &[&str] = &[
    "cf-connecting-ip",
    "cf-ipcountry",
    "cf-ray",
    "cf-visitor",
    "cdn-loop",
    "true-client-ip",
    "cf-worker",
    "cf-ew-via",
    "cf-warp-tag-id",
];

pub async fn strip_cf_headers_middleware(mut request: Request, next: Next) -> Response {
    // Strip CF headers from the incoming request
    for name in CF_HEADERS {
        if let Ok(header) = HeaderName::from_bytes(name.as_bytes()) {
            request.headers_mut().remove(&header);
        }
    }

    let mut response = next.run(request).await;

    // Strip CF headers from the outgoing response
    for name in CF_HEADERS {
        if let Ok(header) = HeaderName::from_bytes(name.as_bytes()) {
            response.headers_mut().remove(&header);
        }
    }

    response
}
