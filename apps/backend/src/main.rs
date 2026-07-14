use anyhow::Context;
use backend::{
    AppState, AuthService, Config, Database, DatabaseConfig, EmailService, MaintenanceConfig,
    MaintenanceService, Metrics, RuntimeDatabaseRole, build_router, init_tracing,
};
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpStream},
    time::Duration,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let command = std::env::args().nth(1);
    if command.as_deref() == Some("healthcheck") {
        return healthcheck();
    }
    if command.as_deref().is_some_and(|command| {
        !matches!(
            command,
            "migrate" | "provision-database-role" | "maintenance"
        )
    }) {
        anyhow::bail!(
            "supported commands: healthcheck, migrate, provision-database-role, maintenance"
        );
    }

    if matches!(
        command.as_deref(),
        Some("migrate" | "provision-database-role" | "maintenance")
    ) {
        let log_filter = std::env::var("RUST_LOG").unwrap_or_else(|_| "backend=info".to_owned());
        if log_filter.trim().is_empty() {
            anyhow::bail!("RUST_LOG must not be empty");
        }
        init_tracing(&log_filter)?;
        let database_config = DatabaseConfig::from_env()?;
        if command.as_deref() == Some("migrate") {
            if !database_config.run_migrations {
                anyhow::bail!("the migrate command requires RUN_MIGRATIONS=true");
            }
        } else if database_config.run_migrations {
            anyhow::bail!("one-shot database commands require RUN_MIGRATIONS=false");
        }
        let database = Database::connect_with(&database_config).await?;

        if command.as_deref() == Some("migrate") {
            tracing::info!("database_migrations_completed");
            return Ok(());
        }

        if command.as_deref() == Some("maintenance") {
            let maintenance =
                MaintenanceService::new(database, MaintenanceConfig::from_env()?, Metrics::new());
            maintenance.run_once().await?;
            return Ok(());
        }
        let role = RuntimeDatabaseRole::from_env()?;
        database.provision_runtime_role(&role).await?;
        tracing::info!(role = %role.name, "database_runtime_role_provisioned");
        return Ok(());
    }

    let config = Config::from_env()?;
    init_tracing(&config.log_filter)?;
    let database = Database::connect(&config).await?;
    let mut state = AppState::with_database(database.clone());
    let maintenance = MaintenanceService::new(
        database.clone(),
        config.maintenance.clone(),
        state.metrics.clone(),
    );
    let email = EmailService::from_config(database.clone(), &config.email, state.metrics.clone())?;
    let email_dispatcher = email.as_ref().map(EmailService::spawn_dispatcher);
    let auth = AuthService::new(database.clone(), &config.auth, email)
        .await
        .map_err(|error| anyhow::anyhow!("could not initialize authentication: {error:?}"))?;
    state.auth = Some(auth);
    let maintenance_worker = maintenance.spawn();

    let address = config.socket_address();
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("could not bind backend to {address}"))?;
    let app = build_router(state);

    tracing::info!(%address, version = env!("CARGO_PKG_VERSION"), "backend_started");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("backend server failed")?;
    tracing::info!("backend_stopped");
    if let Some(dispatcher) = email_dispatcher {
        dispatcher.abort();
    }
    maintenance_worker.abort();

    Ok(())
}

fn healthcheck() -> anyhow::Result<()> {
    let port = std::env::var("APP_PORT")
        .unwrap_or_else(|_| "8080".to_owned())
        .parse()
        .context("APP_PORT must be a valid TCP port")?;
    let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
    let timeout = Duration::from_secs(2);
    let mut stream = TcpStream::connect_timeout(&address.into(), timeout)
        .with_context(|| format!("backend health endpoint is unreachable at {address}"))?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    stream
        .write_all(b"GET /health/ready HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;

    let mut response = [0_u8; 64];
    let bytes_read = stream.read(&mut response)?;
    let status_line = std::str::from_utf8(&response[..bytes_read]).unwrap_or_default();
    if !status_line.starts_with("HTTP/1.1 200") {
        anyhow::bail!("backend readiness check returned a non-success response");
    }
    Ok(())
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "shutdown_signal_failed");
    }
}
