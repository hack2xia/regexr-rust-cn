//! regexr-server library: router assembly shared by the binary and tests.

pub mod actions;
pub mod config;
pub mod http;
pub mod solve;
pub mod state;

use std::sync::Arc;

use axum::http::{header, HeaderValue, StatusCode};
use axum::routing::{get, post};
use axum::Router;
use tower::ServiceBuilder;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::state::AppState;

/// Convert tower service errors (timeout) into responses.
async fn handle_service_error(err: tower::BoxError) -> (StatusCode, &'static str) {
    // The only error reaching this layer now is the wall-clock timeout
    // backstop (solve overload is handled in the API handler via the
    // semaphore, returning 503 there).
    let _ = err;
    (StatusCode::REQUEST_TIMEOUT, "request timeout")
}

/// Build the full application router (security middleware included).
///
/// Middleware (outermost first): server header -> security headers -> 1MB
/// body limit -> timeout backstop. Solve concurrency is enforced *inside*
/// the API handler with a semaphore whose permit lives for the entire
/// blocking PCRE2 task (a tower `concurrency_limit` would release the permit
/// when the request future drops on timeout, while the FFI task keeps
/// running). Static assets are not gated by the solve semaphore.
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(http::static_assets::index))
        .route("/index.html", get(http::static_assets::index))
        .route("/server/api.php", post(http::api::handle))
        .route("/server/init.js", get(http::static_assets::init_js))
        .route("/regexr.js", get(http::static_assets::regexr_js))
        .route("/regexr.css", get(http::static_assets::regexr_css))
        .route("/assets/*path", get(http::static_assets::asset))
        .fallback(get(http::static_assets::fallback))
        .with_state(state)
        .layer(SetResponseHeaderLayer::overriding(
            header::SERVER,
            HeaderValue::from_static("regexr-rust"),
        ))
        .layer(axum::middleware::from_fn(http::security::add_headers))
        .layer(axum::extract::DefaultBodyLimit::max(config::MAX_BODY_BYTES))
        .layer(
            ServiceBuilder::new()
                .layer(axum::error_handling::HandleErrorLayer::new(
                    handle_service_error,
                ))
                .timeout(std::time::Duration::from_secs(config::REQUEST_TIMEOUT_SECS)),
        )
}
