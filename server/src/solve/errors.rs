//! Solve-level errors and their mapping to the JSON contract the frontend
//! expects in the success envelope: `data.error = {message, name, id, warning?}`.

use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub enum SolveError {
    /// Pattern failed to compile. `message` embeds "at offset N" like PHP did.
    RegexParse { message: String },
    /// PCRE2_ERROR_MATCHLIMIT
    MatchLimit,
    /// PCRE2_ERROR_DEPTHLIMIT
    DepthLimit,
    /// PCRE2_ERROR_JIT_STACKLIMIT
    JitStackLimit,
    /// Subject failed UTF-8 validation for PCRE2 (should not happen with &str).
    BadUtf8,
    /// The result (capture-group cells, replace/list output) exceeded the
    /// configured resource budget. Reported as an explicit error — we never
    /// return truncated data pretending to be complete.
    ResultTooLarge,
    /// Any other PCRE2 runtime error.
    Internal { code: i32, message: String },
}

impl SolveError {
    /// JSON shape, verified against real PHP (gen-fixtures.php对拍): compile
    /// errors surface as `{id:"error", name:"PREG_INTERNAL_ERROR", message}`,
    /// limit errors as warning-shaped `{id:"infinite", name:"PREG_*"}`. The
    /// `name` keys match entries in the frontend reference docs
    /// (`reference_content.js`), which `Reference.getError` resolves.
    pub fn to_json(&self) -> Value {
        match self {
            SolveError::RegexParse { message } => json!({
                "message": message,
                "name": "PREG_INTERNAL_ERROR",
                "id": "error",
            }),
            // Limit errors carry no message: PHP has no warning text for them
            // (preg_last_error() code only), so the frontend falls back to the
            // reference-doc tip for `name`.
            SolveError::MatchLimit => json!({
                "name": "PREG_BACKTRACK_LIMIT_ERROR",
                "id": "infinite",
                "warning": true,
            }),
            SolveError::DepthLimit => json!({
                "name": "PREG_RECURSION_LIMIT_ERROR",
                "id": "infinite",
                "warning": true,
            }),
            SolveError::JitStackLimit => json!({
                "name": "PREG_JIT_STACKLIMIT_ERROR",
                "id": "infinite",
                "warning": true,
            }),
            SolveError::BadUtf8 => json!({
                "name": "PREG_BAD_UTF8_ERROR",
                "id": "badutf8",
            }),
            SolveError::ResultTooLarge => json!({
                "message": "Result exceeded server resource limits",
                "name": "PREG_INTERNAL_ERROR",
                "id": "error",
            }),
            SolveError::Internal { message, .. } => json!({
                "message": message,
                "name": "PREG_INTERNAL_ERROR",
                "id": "error",
            }),
        }
    }
}
