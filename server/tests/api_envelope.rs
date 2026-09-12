//! Fixture-based end-to-end tests: replay saved (request -> expected response)
//! pairs through the full axum router. Fixtures under tests/fixtures/ are
//! hand-authored from the PHP contract and can be regenerated with
//! scripts/gen-fixtures.php against the original PHP backend.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use regexr_server::{build_router, state::AppState};
use serde_json::Value;
use tower::ServiceExt;

fn test_app() -> axum::Router {
    build_router(Arc::new(AppState {
        pcre2_version: "10.47 test".into(),
    }))
}

fn form_encode(action: &str, data_json: &str) -> String {
    format!(
        "action={}&data={}",
        action,
        urlencode(data_json.as_bytes())
    )
}

fn urlencode(bytes: &[u8]) -> String {
    let mut out = String::new();
    for b in bytes {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

async fn post_form(app: axum::Router, body: String) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri("/server/api.php")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&bytes).expect("response must be JSON");
    (status, json)
}

/// Recursive subset assertion: every value present in `expected` must match
/// `actual`. Keys absent from `expected` (e.g. variable `time`/`timestamp`,
/// error `message` wording) are ignored.
fn assert_subset(expected: &Value, actual: &Value, path: &str) {
    match (expected, actual) {
        (Value::Object(exp), Value::Object(act)) => {
            for (k, v) in exp {
                let p = format!("{path}.{k}");
                let a = act
                    .get(k)
                    .unwrap_or_else(|| panic!("missing key {p} in {actual}"));
                assert_subset(v, a, &p);
            }
        }
        (Value::Array(exp), Value::Array(act)) => {
            assert_eq!(exp.len(), act.len(), "array length mismatch at {path}");
            for (i, (e, a)) in exp.iter().zip(act.iter()).enumerate() {
                assert_subset(e, a, &format!("{path}[{i}]"));
            }
        }
        (Value::Number(e), Value::Number(a)) => assert_eq!(e, a, "at {path}"),
        (Value::String(e), Value::String(a)) => assert_eq!(e, a, "at {path}"),
        (Value::Bool(e), Value::Bool(a)) => assert_eq!(e, a, "at {path}"),
        (Value::Null, a) => assert!(a.is_null(), "at {path}: expected null, got {a}"),
        (e, a) => panic!("type mismatch at {path}: expected {e}, got {a}"),
    }
}

#[tokio::test]
async fn fixtures_replay() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut count = 0;
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("fixtures dir")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |x| x == "json"))
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let raw = std::fs::read_to_string(entry.path()).unwrap();
        let fixture: Value = serde_json::from_str(&raw).unwrap();
        let name = entry.file_name().to_string_lossy().into_owned();

        let app = test_app();
        let data = serde_json::to_string(&fixture["request"]).unwrap();
        let (status, json) = post_form(app, form_encode("regex/solve", &data)).await;

        assert_eq!(status, StatusCode::OK, "fixture {name}");
        assert_eq!(json["success"], Value::Bool(true), "fixture {name}");
        assert_subset(&fixture["response"], &json["data"], &name);
        count += 1;
    }
    assert!(count >= 10, "expected >=10 fixtures, found {count}");
}

#[tokio::test]
async fn envelope_error_on_invalid_json() {
    let app = test_app();
    let (status, json) = post_form(app.clone(), form_encode("regex/solve", "{not json")).await;
    assert_eq!(status, StatusCode::OK); // PHP contract: always 200
    assert_eq!(json["success"], Value::Bool(false));
    assert_eq!(json["data"]["code"], 1204); // API_INVALID_JSON
}

#[tokio::test]
async fn no_action_error() {
    let app = test_app();
    let (status, json) = post_form(app, "action=".into()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["code"], 1001); // NO_ACTION
}

#[tokio::test]
async fn stub_actions() {
    let app = test_app();
    let (_, json) = post_form(app.clone(), form_encode("patterns/search", "{}")).await;
    assert_eq!(json["data"]["results"], serde_json::json!([]));

    let (_, json) = post_form(app.clone(), form_encode("account/verify", "{}")).await;
    assert_eq!(json["data"]["authenticated"], Value::Bool(false));
}

#[tokio::test]
async fn static_index_has_injected_init_and_security_headers() {
    let app = test_app();
    let resp = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let headers = resp.headers().clone();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&bytes);

    assert!(html.contains(r#"regexr.init(false,"#), "init injected");
    assert!(html.contains("private-rust-deploy"), "versions injected");
    assert!(html.contains("id=\"regexWorker\""), "worker inline kept");
    assert_eq!(
        headers["content-security-policy"],
        "default-src 'self'; script-src 'self' 'unsafe-inline' blob:; worker-src blob:; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; form-action 'self'"
    );
    assert_eq!(headers["x-frame-options"], "DENY");
    assert_eq!(headers["x-content-type-options"], "nosniff");
}

#[tokio::test]
async fn body_limit_enforced() {
    let app = test_app();
    let big = "x".repeat(2 * 1024 * 1024); // 2MB > 1MB limit
    let req = Request::builder()
        .method("POST")
        .uri("/server/api.php")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(big))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
}
