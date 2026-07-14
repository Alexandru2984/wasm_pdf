use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::{Context, bail};

#[derive(Clone, Debug)]
pub struct Config {
    pub host: IpAddr,
    pub port: u16,
    pub log_filter: String,
    pub database_url: String,
    pub database_max_connections: u32,
    pub run_migrations: bool,
}

impl Config {
    /// Read process configuration from environment variables.
    ///
    /// # Errors
    ///
    /// Returns an error when `APP_HOST` is not an IP address, `APP_PORT` is not
    /// a valid non-zero port, or `RUST_LOG` is empty.
    pub fn from_env() -> anyhow::Result<Self> {
        let host = std::env::var("APP_HOST")
            .unwrap_or_else(|_| "0.0.0.0".to_owned())
            .parse::<IpAddr>()
            .context("APP_HOST must be a valid IP address")?;
        let port = std::env::var("APP_PORT")
            .unwrap_or_else(|_| "8080".to_owned())
            .parse::<u16>()
            .context("APP_PORT must be a valid TCP port")?;
        if port == 0 {
            bail!("APP_PORT must not be zero");
        }
        let log_filter =
            std::env::var("RUST_LOG").unwrap_or_else(|_| "backend=info,tower_http=info".to_owned());
        if log_filter.trim().is_empty() {
            bail!("RUST_LOG must not be empty");
        }
        let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
        if database_url.trim().is_empty() {
            bail!("DATABASE_URL must not be empty");
        }
        let database_max_connections = std::env::var("DATABASE_MAX_CONNECTIONS")
            .unwrap_or_else(|_| "10".to_owned())
            .parse::<u32>()
            .context("DATABASE_MAX_CONNECTIONS must be a positive integer")?;
        if database_max_connections == 0 || database_max_connections > 100 {
            bail!("DATABASE_MAX_CONNECTIONS must be between 1 and 100");
        }
        let run_migrations = parse_bool("RUN_MIGRATIONS", true)?;

        Ok(Self {
            host,
            port,
            log_filter,
            database_url,
            database_max_connections,
            run_migrations,
        })
    }

    pub const fn socket_address(&self) -> SocketAddr {
        SocketAddr::new(self.host, self.port)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            port: 8080,
            log_filter: "backend=info,tower_http=info".to_owned(),
            database_url: "postgres://pdf_editor:pdf_editor@localhost/pdf_editor".to_owned(),
            database_max_connections: 10,
            run_migrations: true,
        }
    }
}

fn parse_bool(name: &str, default: bool) -> anyhow::Result<bool> {
    match std::env::var(name) {
        Ok(value) if value.eq_ignore_ascii_case("true") || value == "1" => Ok(true),
        Ok(value) if value.eq_ignore_ascii_case("false") || value == "0" => Ok(false),
        Ok(_) => bail!("{name} must be true, false, 1, or 0"),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(error).with_context(|| format!("could not read {name}")),
    }
}
