use anyhow::Context;
use backend::{AppState, Config, build_router, init_tracing};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_env()?;
    init_tracing(&config.log_filter)?;

    let address = config.socket_address();
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("could not bind backend to {address}"))?;
    let app = build_router(AppState::new());

    tracing::info!(%address, version = env!("CARGO_PKG_VERSION"), "backend_started");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("backend server failed")?;
    tracing::info!("backend_stopped");

    Ok(())
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "shutdown_signal_failed");
    }
}
