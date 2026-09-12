//! Security response headers applied to every response.

use axum::extract::Request;
use axum::http::{header, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;

/// CSP notes:
/// - `script-src 'unsafe-inline'`: CodeMirror inline styles/`#phpinject` inline script.
/// - `worker-src blob:`: the JS flavor creates a Web Worker from a Blob
///   (BrowserSolver.js) — without this the JS engine silently degrades.
const CSP: &str = "default-src 'self'; script-src 'self' 'unsafe-inline' blob:; worker-src blob:; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; form-action 'self'";

pub async fn add_headers(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let headers = res.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    headers.insert(
        header::HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("interest-cohort=()"),
    );
    res
}
