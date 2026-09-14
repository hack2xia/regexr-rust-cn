//! Static asset serving with rust-embed (all assets embedded in the binary).

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use bytes::Bytes;
use rust_embed::RustEmbed;

use crate::state::AppState;

#[derive(RustEmbed)]
#[folder = "static/"]
struct Assets;

/// The `#phpinject` script contents: a logged-out private-deployment init,
/// mirroring what index.php used to inject server-side.
fn injected_init(pcre2_version: &str) -> String {
    format!(
        r#"
		/** Private deployment (regexr-rust) **/
		// first param false indicates a local init (no shared pattern).
		regexr.init(false, {{
			userId: 0,
			authenticated: false,
			username: null,
			author: null,
			type: null
		}}, {{
			"PCREVersion": "{}",
			"PHPVersion": "private-rust-deploy"
		}});
	"#,
        pcre2_version
    )
}

/// The `#phpinject` script contents as an external JS file (`/server/init.js`).
///
/// Served separately (instead of being injected inline into index.html) so
/// the CSP can drop `script-src 'unsafe-inline'`. Mirrors what index.php used
/// to inject server-side: a logged-out private-deployment init.
pub async fn init_js(State(state): State<Arc<AppState>>) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        injected_init(&state.pcre2_version),
    )
        .into_response()
}

/// index.html, cached in `AppState.index` as refcounted `Bytes` (serving
/// clones a handle, never the page).
fn assembled_index(state: &AppState) -> Bytes {
    state
        .index
        .get_or_init(|| {
            let html = Assets::get("index.html").expect("index.html embedded");
            Bytes::from(html.data.into_owned())
        })
        .clone()
}

fn serve_key(state: &AppState, key: &str) -> Response {
    match Assets::get(key) {
        Some(file) => {
            let mime = mime_guess::from_path(key).first_or_octet_stream();
            let cache = if key.starts_with("assets/") {
                "public, max-age=86400"
            } else {
                "no-cache"
            };
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, mime.as_ref()),
                    (header::CACHE_CONTROL, cache),
                ],
                file.data,
            )
                .into_response()
        }
        // Unknown paths fall back to the app (mirrors the old .htaccess
        // rewrite-to-index behavior for deep links).
        None => Html(assembled_index(state)).into_response(),
    }
}

pub async fn index(State(state): State<Arc<AppState>>) -> Response {
    Html(assembled_index(&state)).into_response()
}

pub async fn regexr_js(State(state): State<Arc<AppState>>) -> Response {
    serve_key(&state, "regexr.js")
}

pub async fn regexr_css(State(state): State<Arc<AppState>>) -> Response {
    serve_key(&state, "regexr.css")
}

/// `/assets/*path`: the wildcard captures the path relative to `assets/`.
pub async fn asset(State(state): State<Arc<AppState>>, Path(path): Path<String>) -> Response {
    serve_key(&state, &format!("assets/{path}"))
}

/// Fallback for unknown paths: serve the app (mirrors the old .htaccess
/// rewrite-to-index behavior).
pub async fn fallback(State(state): State<Arc<AppState>>) -> Response {
    Html(assembled_index(&state)).into_response()
}
