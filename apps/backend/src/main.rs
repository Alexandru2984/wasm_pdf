use anyhow::Context;
use backend::{AppState, AuthService, Config, Database, build_router, init_tracing};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_env()?;
    init_tracing(&config.log_filter)?;
    let database = Database::connect(&config).await?;
    let auth = AuthService::new(database.clone(), &config.auth)
        .await
        .map_err(|error| anyhow::anyhow!("could not initialize authentication: {error:?}"))?;

    let address = config.socket_address();
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("could not bind backend to {address}"))?;
    let app = build_router(AppState::with_services(database, auth));

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
