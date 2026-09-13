//! `POST /server/api.php` — drop-in replacement for the PHP entry point.
//!
//! Contract (from the original PHP backend):
//! - form-urlencoded body: `action=regex/solve` + `data=<encodeURIComponent(JSON)>`
//! - always HTTP 200, `application/json`
//! - success: `{"success":true,"data":{...},"metadata":{"script-time":"1.23ms"}}`
//! - envelope failure: `{"success":false,"data":{code,data,message},"metadata":{...}}`

use std::sync::Arc;

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};

use crate::solve;
use crate::state::AppState;

const CODE_UNKNOWN: i64 = 1000;
const CODE_NO_ACTION: i64 = 1001;
const CODE_INVALID_JSON: i64 = 1204;

pub async fn handle(State(state): State<Arc<AppState>>, body: String) -> Response {
    let started = std::time::Instant::now();

    let action = form_value(&body, "action").unwrap_or_default();
    if action.is_empty() {
        return envelope_error(CODE_NO_ACTION, "No action was provided", started);
    }

    let payload = if action == "regex/solve" {
        let data_raw = match form_value(&body, "data") {
            Some(d) => d,
            None => return envelope_error(CODE_INVALID_JSON, "No data was provided", started),
        };
        let parsed: Value = match serde_json::from_str(&data_raw) {
            Ok(v) => v,
            Err(_) => return envelope_error(CODE_INVALID_JSON, "JSON was not valid", started),
        };
        let req: solve::SolveRequest = match serde_json::from_value(parsed) {
            Ok(r) => r,
            // Deliberately not echoing serde's message: it quotes request
            // field names back to the client.
            Err(_) => return envelope_error(CODE_INVALID_JSON, "JSON was not valid", started),
        };

        // PCRE2 matching is synchronous FFI: run it on the blocking pool so
        // the async runtime stays responsive. The hard anti-DoS limits are
        // the PCRE2 match/depth limits (see config.rs), not this thread.
        match tokio::task::spawn_blocking(move || solve::solve(&req)).await {
            Ok(data) => envelope_success(data, started),
            Err(e) => envelope_error(CODE_UNKNOWN, &format!("internal error: {e}"), started),
        }
    } else {
        let version = state.pcre2_version.clone();
        envelope_success(
            crate::actions::stubs::stub_response(&action, &version),
            started,
        )
    };

    payload
}

fn form_value(body: &str, key: &str) -> Option<String> {
    // urlencoded bodies from the frontend are built by encodeURIComponent, so
    // `+` never appears; still decode it defensively for other clients.
    for pair in body.split('&') {
        // skip malformed pairs (no `=`) instead of aborting the whole scan
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        if percent_decode(k) == key {
            return Some(percent_decode(v));
        }
    }
    None
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' => {
                let hex = |b: u8| -> Option<u8> {
                    match b {
                        b'0'..=b'9' => Some(b - b'0'),
                        b'a'..=b'f' => Some(b - b'a' + 10),
                        b'A'..=b'F' => Some(b - b'A' + 10),
                        _ => None,
                    }
                };
                if i + 2 < bytes.len() {
                    if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                        out.push(h * 16 + l);
                        i += 3;
                        continue;
                    }
                }
                out.push(b'%');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn metadata(started: std::time::Instant) -> Value {
    json!({ "script-time": format!("{:.2}ms", started.elapsed().as_secs_f64() * 1000.0) })
}

fn envelope_success(data: Value, started: std::time::Instant) -> Response {
    let body = json!({
        "success": true,
        "data": data,
        "metadata": metadata(started),
    });
    json_response(body)
}

fn envelope_error(code: i64, message: &str, started: std::time::Instant) -> Response {
    let body = json!({
        "success": false,
        "data": { "code": code, "data": null, "message": message },
        "metadata": metadata(started),
    });
    json_response(body)
}

fn json_response(body: Value) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}
