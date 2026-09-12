//! Stub actions for the APIs the frontend calls but a private deployment
//! does not support (no database, no accounts, no community).
//!
//! Returning well-formed empty data keeps the UI fully functional with zero
//! frontend modifications and zero error toasts.

use serde_json::{json, Value};

/// Handle any non-`regex/solve` action. Never fails: always a success envelope.
pub fn stub_response(action: &str, pcre2_version: &str) -> Value {
    match action {
        // Sidebar community search: expects `{results: [...]}`.
        "patterns/search" | "account/patterns" => json!({ "results": [] }),

        // Session bootstrap: logged-out profile.
        "account/verify" => json!({
            "authenticated": false,
            "userId": 0,
            "username": null,
            "author": null,
            "type": null,
        }),

        // Defined in Server.js but never called by the UI; return the version anyway.
        "regex/version" => json!({ "version": pcre2_version }),

        // patterns/load: deep links are unsupported (no storage).
        "patterns/load" => json!({}),

        // Everything else (save/delete/favorite/rate/setAccess/logout/...):
        // empty success object.
        _ => json!({}),
    }
}
