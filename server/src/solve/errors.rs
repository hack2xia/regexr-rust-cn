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
    /// Any other PCRE2 runtime error.
    Internal { code: i32, message: String },
}

impl SolveError {
    /// JSON shape consumed by `Text.js _errorText` / `Reference.getError`.
    /// `id` is the frontend documentation key:
    ///   regexparse -> compile error, infinite -> limits (rendered as warning),
    ///   badutf8 -> encoding, error -> generic.
    pub fn to_json(&self) -> Value {
        match self {
            SolveError::RegexParse { message } => json!({
                "message": message,
                "name": "CompileError",
                "id": "regexparse",
            }),
            SolveError::MatchLimit => json!({
                "message": "Backtrack limit exhausted",
                "name": "PREG_BACKTRACK_LIMIT_ERROR",
                "id": "infinite",
                "warning": true,
            }),
            SolveError::DepthLimit => json!({
                "message": "Recursion limit exhausted",
                "name": "PREG_RECURSION_LIMIT_ERROR",
                "id": "infinite",
                "warning": true,
            }),
            SolveError::JitStackLimit => json!({
                "message": "JIT stack limit exhausted",
                "name": "PREG_JIT_STACKLIMIT_ERROR",
                "id": "infinite",
                "warning": true,
            }),
            SolveError::BadUtf8 => json!({
                "message": "Malformed UTF-8 subject",
                "name": "PREG_BAD_UTF8_ERROR",
                "id": "badutf8",
            }),
            SolveError::Internal { message, .. } => json!({
                "message": message,
                "name": "PREG_INTERNAL_ERROR",
                "id": "error",
            }),
        }
    }
}
