use std::fs;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::{Context, bail};
use url::Url;

#[derive(Clone)]
pub struct Config {
    pub environment: Environment,
    pub host: IpAddr,
    pub port: u16,
    pub log_filter: String,
    pub database_url: String,
    pub database_max_connections: u32,
    pub run_migrations: bool,
    pub auth: AuthConfig,
}

#[derive(Clone)]
pub struct AuthConfig {
    pub jwt_secret: String,
    pub jwt_issuer: String,
    pub jwt_audience: String,
    pub access_token_seconds: i64,
    pub session_days: i64,
    pub cookie_secure: bool,
    pub webauthn_rp_id: String,
    pub webauthn_rp_origin: String,
    pub webauthn_rp_name: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Environment {
    Development,
    Test,
    Production,
}

impl Config {
    /// Read and validate process configuration from environment variables or
    /// their corresponding `_FILE` secret mounts.
    ///
    /// # Errors
    ///
    /// Returns an error for missing, conflicting or unsafe configuration.
    pub fn from_env() -> anyhow::Result<Self> {
        let environment = parse_environment()?;
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
        let database_url = database_url()?;
        let database_max_connections = std::env::var("DATABASE_MAX_CONNECTIONS")
            .unwrap_or_else(|_| "10".to_owned())
            .parse::<u32>()
            .context("DATABASE_MAX_CONNECTIONS must be a positive integer")?;
        if database_max_connections == 0 || database_max_connections > 100 {
            bail!("DATABASE_MAX_CONNECTIONS must be between 1 and 100");
        }
        let run_migrations = parse_bool("RUN_MIGRATIONS", true)?;
        let jwt_secret = required_secret("JWT_SECRET")?;
        if jwt_secret.len() < 32 {
            bail!("JWT_SECRET must contain at least 32 bytes");
        }
        let jwt_issuer = nonempty_env("JWT_ISSUER", "wasm-pdf-editor")?;
        let jwt_audience = nonempty_env("JWT_AUDIENCE", "wasm-pdf-editor-web")?;
        let access_token_seconds = parse_i64_range("ACCESS_TOKEN_SECONDS", 900, 60, 3_600)?;
        let session_days = parse_i64_range("SESSION_DAYS", 30, 1, 90)?;
        let cookie_secure = parse_bool("COOKIE_SECURE", true)?;
        let webauthn_rp_id = nonempty_env("WEBAUTHN_RP_ID", "localhost")?;
        let webauthn_rp_origin = nonempty_env("WEBAUTHN_RP_ORIGIN", "http://localhost:8080")?;
        let webauthn_rp_name = nonempty_env("WEBAUTHN_RP_NAME", "PDF Editor")?;

        if environment == Environment::Production {
            validate_production(
                &database_url,
                &jwt_secret,
                cookie_secure,
                &webauthn_rp_id,
                &webauthn_rp_origin,
            )?;
        }

        Ok(Self {
            environment,
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
                webauthn_rp_id,
                webauthn_rp_origin,
                webauthn_rp_name,
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
            environment: Environment::Development,
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
                webauthn_rp_id: "localhost".to_owned(),
                webauthn_rp_origin: "http://localhost:8080".to_owned(),
                webauthn_rp_name: "PDF Editor".to_owned(),
            },
        }
    }
}

fn parse_environment() -> anyhow::Result<Environment> {
    match std::env::var("APP_ENV")
        .unwrap_or_else(|_| "development".to_owned())
        .to_ascii_lowercase()
        .as_str()
    {
        "development" => Ok(Environment::Development),
        "test" => Ok(Environment::Test),
        "production" => Ok(Environment::Production),
        _ => bail!("APP_ENV must be development, test, or production"),
    }
}

fn database_url() -> anyhow::Result<String> {
    if let Some(value) = optional_secret("DATABASE_URL")? {
        return Ok(value);
    }

    let password = required_secret("DATABASE_PASSWORD")?;
    let host = nonempty_env("DATABASE_HOST", "postgres")?;
    let name = nonempty_env("DATABASE_NAME", "pdf_editor")?;
    let user = nonempty_env("DATABASE_USER", "pdf_editor")?;
    let port = std::env::var("DATABASE_PORT")
        .unwrap_or_else(|_| "5432".to_owned())
        .parse::<u16>()
        .context("DATABASE_PORT must be a valid TCP port")?;
    if port == 0 {
        bail!("DATABASE_PORT must not be zero");
    }

    let mut url = Url::parse("postgres://localhost").context("could not construct DATABASE_URL")?;
    url.set_host(Some(&host))
        .map_err(|_| anyhow::anyhow!("DATABASE_HOST is invalid"))?;
    url.set_port(Some(port))
        .map_err(|()| anyhow::anyhow!("DATABASE_PORT is invalid"))?;
    url.set_username(&user)
        .map_err(|()| anyhow::anyhow!("DATABASE_USER is invalid"))?;
    url.set_password(Some(&password))
        .map_err(|()| anyhow::anyhow!("DATABASE_PASSWORD is invalid"))?;
    url.set_path(&name);
    Ok(url.into())
}

fn required_secret(name: &str) -> anyhow::Result<String> {
    optional_secret(name)?.with_context(|| format!("{name} or {name}_FILE must be set"))
}

fn optional_secret(name: &str) -> anyhow::Result<Option<String>> {
    let direct = std::env::var(name).ok();
    let file_name = format!("{name}_FILE");
    let file = std::env::var(&file_name).ok();
    if direct.is_some() && file.is_some() {
        bail!("set only one of {name} or {file_name}");
    }

    let value = match (direct, file) {
        (Some(value), None) => Some(value),
        (None, Some(path)) => Some(
            fs::read_to_string(&path)
                .with_context(|| format!("could not read {file_name} path {path}"))?
                .trim_end_matches(['\r', '\n'])
                .to_owned(),
        ),
        (None, None) => None,
        (Some(_), Some(_)) => unreachable!(),
    };
    if value.as_ref().is_some_and(String::is_empty) {
        bail!("{name} must not be empty");
    }
    Ok(value)
}

fn validate_production(
    database_url: &str,
    jwt_secret: &str,
    cookie_secure: bool,
    webauthn_rp_id: &str,
    webauthn_rp_origin: &str,
) -> anyhow::Result<()> {
    if !cookie_secure {
        bail!("COOKIE_SECURE must be true in production");
    }
    if !webauthn_rp_origin.starts_with("https://") {
        bail!("WEBAUTHN_RP_ORIGIN must use https in production");
    }
    if webauthn_rp_id.eq_ignore_ascii_case("localhost") || webauthn_rp_id.parse::<IpAddr>().is_ok()
    {
        bail!("WEBAUTHN_RP_ID must be a public DNS name in production");
    }
    if jwt_secret.len() < 43
        || jwt_secret.contains("development-only")
        || jwt_secret.contains("replace-with")
    {
        bail!("JWT_SECRET must be a non-placeholder value with at least 256 bits in production");
    }
    if database_url.contains("change-me") {
        bail!("DATABASE password placeholder is forbidden in production");
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_requires_secure_identity_configuration() {
        assert!(
            validate_production(
                "postgres://app:secret@postgres/app",
                &"a".repeat(43),
                true,
                "pdf.example.com",
                "https://pdf.example.com",
            )
            .is_ok()
        );
        assert!(
            validate_production(
                "postgres://app:change-me@postgres/app",
                "development-only-change-this-32-byte-secret",
                false,
                "localhost",
                "http://localhost:8080",
            )
            .is_err()
        );
    }
}
