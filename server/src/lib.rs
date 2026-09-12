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

/// Convert tower service errors (load shedding, timeout) into responses.
async fn handle_service_error(err: tower::BoxError) -> (StatusCode, &'static str) {
    if err.is::<tower::load_shed::error::Overloaded>() {
        (StatusCode::SERVICE_UNAVAILABLE, "server busy")
    } else {
        (StatusCode::REQUEST_TIMEOUT, "request timeout")
    }
}

/// Build the full application router (security middleware included).
///
/// Middleware (outermost first): server header -> security headers -> 1MB
/// body limit -> load shed / concurrency limit / timeout. The wall-clock
/// timeout is only a backstop for handler overhead; a running `pcre2_match`
/// FFI call is bounded by the PCRE2 match/depth limits (see config.rs).
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(http::static_assets::index))
        .route("/index.html", get(http::static_assets::index))
        .route("/server/api.php", post(http::api::handle))
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
                .load_shed()
                .concurrency_limit(config::MAX_CONCURRENT_SOLVE)
                .timeout(std::time::Duration::from_secs(config::REQUEST_TIMEOUT_SECS)),
        )
}
