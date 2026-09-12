//! `regex/solve` orchestration: request types, dispatch to the PCRE2 engine,
//! and serialization to the JSON contract the frontend expects.

pub mod engine;
pub mod errors;
pub mod offsets;
pub mod subst;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config;
use engine::CompiledRegex;
use errors::SolveError;
use offsets::{byte_to_utf16, utf16_len};

#[derive(Deserialize, Debug, Default)]
pub struct Tool {
    pub id: Option<String>,
    #[serde(default)]
    pub input: Option<Value>,
}

#[derive(Deserialize, Debug)]
pub struct Test {
    #[serde(default)]
    pub id: Option<Value>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct SolveRequest {
    #[serde(default)]
    pub pattern: String,
    #[serde(default)]
    pub flags: String,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub tool: Option<Tool>,
    #[serde(default)]
    pub tests: Option<Vec<Test>>,
    /// Echoed back untouched (any JSON value).
    #[serde(default)]
    pub id: Option<Value>,
}

/// One match in the frontend contract: `{i, l, groups:[{i,l},...]}` with
/// UTF-16 code-unit offsets. Unmatched groups serialize as `{i:0,l:0}`
/// (matching what the PHP backend sent).
#[derive(Serialize)]
struct JsonMatch {
    i: usize,
    l: usize,
    groups: Vec<JsonGroup>,
}

#[derive(Serialize)]
struct JsonGroup {
    i: usize,
    l: usize,
}

fn json_group(span: Option<(usize, usize)>, s: &str) -> JsonGroup {
    match span {
        Some((bs, be)) => JsonGroup {
            i: byte_to_utf16(s, bs),
            l: utf16_len(&s[bs..be]),
        },
        None => JsonGroup { i: 0, l: 0 },
    }
}

fn json_match(span: &engine::MatchSpan, s: &str) -> JsonMatch {
    JsonMatch {
        i: byte_to_utf16(s, span.start),
        l: utf16_len(&s[span.start..span.end]),
        groups: span.groups.iter().map(|g| json_group(*g, s)).collect(),
    }
}

/// Run one solve request and produce the `data` payload (success envelope).
/// Solve-level errors are reported inside the success envelope as `data.error`,
/// exactly like the PHP backend did.
pub fn solve(req: &SolveRequest) -> Value {
    let started = std::time::Instant::now();
    let mode = req.mode.as_deref().unwrap_or("text");

    let re = match CompiledRegex::compile(&req.pattern, &req.flags) {
        Ok(re) => re,
        Err(e) => {
            // PHP parity: in tests mode a compile error is reported per test.
            if mode == "tests" {
                let matches: Vec<Value> = req
                    .tests
                    .as_deref()
                    .unwrap_or(&[])
                    .iter()
                    .take(config::MAX_TESTS)
                    .map(|t| json!({ "id": t.id.clone(), "error": e.to_json() }))
                    .collect();
                return json!({
                    "id": req.id.clone(),
                    "timestamp": std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0),
                    "time": elapsed_secs(started),
                    "mode": mode,
                    "matches": matches,
                });
            }
            return solve_error_data(req, mode, &e, started);
        }
    };

    let mut data = json!({
        "id": req.id.clone(),
        "timestamp": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        "time": elapsed_secs(started),
        "mode": mode,
    });

    match mode {
        "tests" => {
            let tests = req.tests.as_deref().unwrap_or(&[]);
            let mut matches = Vec::with_capacity(tests.len().min(config::MAX_TESTS));
            for t in tests.iter().take(config::MAX_TESTS) {
                let text = t.text.as_deref().unwrap_or("");
                let mut entry = serde_json::Map::new();
                entry.insert("id".into(), t.id.clone().unwrap_or(Value::Null));
                match re.match_one(text) {
                    Ok(Some(span)) => {
                        entry.insert("i".into(), json!(byte_to_utf16(text, span.start)));
                        entry.insert("l".into(), json!(utf16_len(&text[span.start..span.end])));
                    }
                    Ok(None) => {
                        // no i/l fields at all: frontend treats this as "no match"
                    }
                    Err(e) => {
                        entry.insert("error".into(), e.to_json());
                    }
                }
                matches.push(Value::Object(entry));
            }
            data["matches"] = json!(matches);
        }
        _ => {
            // mode = "text"
            let text = req.text.as_deref().unwrap_or("");
            let result = (|| -> Result<Value, SolveError> {
                let spans = if re.global() {
                    re.match_all(text)?
                } else {
                    re.match_one(text)?.into_iter().collect()
                };
                let matches: Vec<JsonMatch> = spans.iter().map(|sp| json_match(sp, text)).collect();

                let mut data = json!({ "matches": matches });

                if let Some(tool) = &req.tool {
                    let tool_id = tool.id.as_deref().unwrap_or("");
                    let tool_result = match tool_id {
                        "replace" => {
                            let repl = tool.input.as_ref().and_then(|v| v.as_str()).unwrap_or("");
                            re.replace(text, repl)?
                        }
                        "list" => {
                            let repl = tool.input.as_ref().and_then(|v| v.as_str()).unwrap_or("");
                            re.list(text, repl)?
                        }
                        // details/explain never reach the server; unknown ids -> ""
                        _ => String::new(),
                    };
                    data["tool"] = json!({ "id": tool_id, "result": tool_result });
                }
                Ok(data)
            })();

            match result {
                Ok(mut payload) => {
                    // Merge (matches + optional tool) into data.
                    if let Some(src) = payload.as_object_mut() {
                        let obj = data.as_object_mut().unwrap();
                        for (k, v) in std::mem::take(src).into_iter() {
                            obj.insert(k, v);
                        }
                    }
                }
                Err(e) => {
                    if let Some(obj) = data.as_object_mut() {
                        obj.insert("matches".into(), json!([]));
                        obj.insert("error".into(), e.to_json());
                    }
                }
            }
        }
    }

    data["time"] = json!(elapsed_secs(started));
    data
}

fn elapsed_secs(started: std::time::Instant) -> f64 {
    started.elapsed().as_secs_f64()
}

fn solve_error_data(req: &SolveRequest, mode: &str, e: &SolveError, started: std::time::Instant) -> Value {
    json!({
        "id": req.id.clone(),
        "timestamp": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        "time": elapsed_secs(started),
        "mode": mode,
        "matches": [],
        "error": e.to_json(),
    })
}
