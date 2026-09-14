//! Security response headers applied to every response.

use axum::extract::Request;
use axum::http::{header, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;

/// CSP notes:
/// - No `'unsafe-inline'` in `script-src`: the app bootstrap lives in the
///   external `/server/init.js` (dynamic PCRE version), not an inline block.
///   DOM XSS in the expression pane can therefore no longer execute even if
///   it slips past the token rendering.
/// - `worker-src blob:`: the JS flavor creates a Web Worker from a Blob
///   (BrowserSolver.js) — without this the JS engine silently degrades.
///   Dedicated workers are governed by worker-src, so `script-src blob:` is
///   not needed.
/// - `style-src 'unsafe-inline'` stays: CodeMirror uses inline styles.
const CSP: &str = "default-src 'self'; script-src 'self'; worker-src blob:; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; form-action 'self'; object-src 'none'; base-uri 'none'";

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
