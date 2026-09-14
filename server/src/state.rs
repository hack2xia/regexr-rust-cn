//! Shared application state.

use std::sync::OnceLock;

use bytes::Bytes;

pub struct AppState {
    pub pcre2_version: String,
    /// index.html with the phpinject block filled in, assembled lazily on
    /// first request and cached (as `Bytes` so serving clones a refcount,
    /// not the page).
    pub index: OnceLock<Bytes>,
    /// Solve concurrency: one permit per *in-flight blocking PCRE2 task*.
    /// The permit is acquired before `spawn_blocking` and moved into the
    /// closure, so its lifetime matches the actual FFI work — unlike a tower
    /// `concurrency_limit` layer, whose permit is released when the request
    /// future is dropped (e.g. on timeout) while the blocking task keeps
    /// running. Static assets are never gated by this semaphore.
    pub solve_semaphore: std::sync::Arc<tokio::sync::Semaphore>,
}

impl AppState {
    pub fn new(pcre2_version: String) -> Self {
        AppState {
            pcre2_version,
            index: OnceLock::new(),
            solve_semaphore: std::sync::Arc::new(tokio::sync::Semaphore::new(
                crate::config::MAX_CONCURRENT_SOLVE,
            )),
        }
    }
}
