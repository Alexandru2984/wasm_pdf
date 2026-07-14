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
    pub auth: AuthConfig,
}

#[derive(Clone, Debug)]
pub struct AuthConfig {
    pub jwt_secret: String,
    pub jwt_issuer: String,
    pub jwt_audience: String,
    pub access_token_seconds: i64,
    pub session_days: i64,
    pub cookie_secure: bool,
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
        let jwt_secret = std::env::var("JWT_SECRET").context("JWT_SECRET must be set")?;
        if jwt_secret.len() < 32 {
            bail!("JWT_SECRET must contain at least 32 bytes");
        }
        let jwt_issuer = nonempty_env("JWT_ISSUER", "wasm-pdf-editor")?;
        let jwt_audience = nonempty_env("JWT_AUDIENCE", "wasm-pdf-editor-web")?;
        let access_token_seconds = parse_i64_range("ACCESS_TOKEN_SECONDS", 900, 60, 3_600)?;
        let session_days = parse_i64_range("SESSION_DAYS", 30, 1, 90)?;
        let cookie_secure = parse_bool("COOKIE_SECURE", true)?;

        Ok(Self {
            host,
            port,
            log_filter,
            database_url,
            database_max_connections,
            run_migrations,
            auth: AuthConfig {
                jwt_secret,
                jwt_issuer,
                jwt_audience,
                access_token_seconds,
                session_days,
                cookie_secure,
            },
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
            auth: AuthConfig {
                jwt_secret: "development-only-secret-at-least-32-bytes".to_owned(),
                jwt_issuer: "wasm-pdf-editor".to_owned(),
                jwt_audience: "wasm-pdf-editor-web".to_owned(),
                access_token_seconds: 900,
                session_days: 30,
                cookie_secure: false,
            },
        }
    }
}

fn nonempty_env(name: &str, default: &str) -> anyhow::Result<String> {
    let value = std::env::var(name).unwrap_or_else(|_| default.to_owned());
    if value.trim().is_empty() {
        bail!("{name} must not be empty");
    }
    Ok(value)
}

fn parse_i64_range(name: &str, default: i64, minimum: i64, maximum: i64) -> anyhow::Result<i64> {
    let value = std::env::var(name)
        .unwrap_or_else(|_| default.to_string())
        .parse::<i64>()
        .with_context(|| format!("{name} must be an integer"))?;
    if !(minimum..=maximum).contains(&value) {
        bail!("{name} must be between {minimum} and {maximum}");
    }
    Ok(value)
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
