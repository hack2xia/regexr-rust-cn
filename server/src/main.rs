//! regexr-server: single-binary private deployment of RegExr.
//!
//! Serves the embedded static frontend and one API endpoint
//! (`POST /server/api.php`, action=regex/solve) backed by PCRE2.

use regexr_server::{build_router, config, solve, state::AppState};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let pcre2_version = solve::engine::pcre2_version();
    tracing::info!("PCRE2 version: {pcre2_version}");

    let state = std::sync::Arc::new(AppState { pcre2_version });
    let app = build_router(state);

    let addr: std::net::SocketAddr = std::env::var("REGEXR_ADDR")
        .unwrap_or_else(|_| config::DEFAULT_ADDR.to_string())
        .parse()
        .expect("invalid REGEXR_ADDR");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind");

    tracing::info!("listening on http://{addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install SIGINT handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
