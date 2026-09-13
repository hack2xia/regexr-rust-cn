//! Shared application state.

use std::sync::OnceLock;

use bytes::Bytes;

pub struct AppState {
    pub pcre2_version: String,
    /// index.html with the phpinject block filled in, assembled lazily on
    /// first request and cached (as `Bytes` so serving clones a refcount,
    /// not the page).
    pub index: OnceLock<Bytes>,
}

impl AppState {
    pub fn new(pcre2_version: String) -> Self {
        AppState {
            pcre2_version,
            index: OnceLock::new(),
        }
    }
}
