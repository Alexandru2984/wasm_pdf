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
    pub email: EmailConfig,
}

#[derive(Clone)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub run_migrations: bool,
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

#[derive(Clone)]
pub struct EmailConfig {
    pub enabled: bool,
    pub public_base_url: Url,
    pub token_secret: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_username: Option<String>,
    pub smtp_password: Option<String>,
    pub smtp_tls: SmtpTls,
    pub from_address: String,
    pub from_name: String,
}

pub struct RuntimeDatabaseRole {
    pub name: String,
    pub password: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmtpTls {
    StartTls,
    None,
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
        let database = DatabaseConfig::from_env()?;
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
        let email = email_config(&webauthn_rp_origin)?;

        if environment == Environment::Production {
            validate_production(
                &database.url,
                &jwt_secret,
                cookie_secure,
                &webauthn_rp_id,
                &webauthn_rp_origin,
                &email,
            )?;
        }

        Ok(Self {
            environment,
            host,
            port,
            log_filter,
            database_url: database.url,
            database_max_connections: database.max_connections,
            run_migrations: database.run_migrations,
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
            email,
        })
    }

    pub const fn socket_address(&self) -> SocketAddr {
        SocketAddr::new(self.host, self.port)
    }
}

impl DatabaseConfig {
    /// Load only `PostgreSQL` settings for isolated migration and provisioning
    /// jobs that must not receive authentication or SMTP secrets.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid, missing, or placeholder database settings.
    pub fn from_env() -> anyhow::Result<Self> {
        let url = database_url()?;
        let max_connections = std::env::var("DATABASE_MAX_CONNECTIONS")
            .unwrap_or_else(|_| "10".to_owned())
            .parse::<u32>()
            .context("DATABASE_MAX_CONNECTIONS must be a positive integer")?;
        if max_connections == 0 || max_connections > 100 {
            bail!("DATABASE_MAX_CONNECTIONS must be between 1 and 100");
        }
        let run_migrations = parse_bool("RUN_MIGRATIONS", true)?;
        if parse_environment()? == Environment::Production && url.contains("change-me") {
            bail!("DATABASE password placeholder is forbidden in production");
        }
        Ok(Self {
            url,
            max_connections,
            run_migrations,
        })
    }
}

impl RuntimeDatabaseRole {
    /// Load the least-privilege runtime role requested by the provisioning
    /// command.
    ///
    /// # Errors
    ///
    /// Returns an error for missing credentials or an unsafe role identifier.
    pub fn from_env() -> anyhow::Result<Self> {
        let name = nonempty_env("DATABASE_RUNTIME_USER", "pdf_editor_runtime")?;
        let mut bytes = name.bytes();
        let valid_first = bytes
            .next()
            .is_some_and(|byte| matches!(byte, b'a'..=b'z' | b'_'));
        if name.len() > 63
            || !valid_first
            || !bytes.all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_'))
        {
            bail!("DATABASE_RUNTIME_USER must be a lowercase PostgreSQL identifier");
        }
        let password = required_secret("DATABASE_RUNTIME_PASSWORD")?;
        if password.len() < 43
            || password.contains("change-me")
            || password.contains("replace-with")
        {
            bail!("DATABASE_RUNTIME_PASSWORD must be a non-placeholder 256-bit secret");
        }
        Ok(Self { name, password })
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
            email: EmailConfig {
                enabled: false,
                public_base_url: Url::parse("http://localhost:8080")
                    .expect("the default public URL is valid"),
                token_secret: String::new(),
                smtp_host: "localhost".to_owned(),
                smtp_port: 1025,
                smtp_username: None,
                smtp_password: None,
                smtp_tls: SmtpTls::None,
                from_address: "no-reply@localhost".to_owned(),
                from_name: "PDF Editor".to_owned(),
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
    email: &EmailConfig,
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
    if !email.enabled {
        bail!("EMAIL_DELIVERY_ENABLED must be true in production");
    }
    if email.smtp_tls != SmtpTls::StartTls {
        bail!("SMTP_TLS must be starttls in production");
    }
    if email.public_base_url.scheme() != "https" {
        bail!("PUBLIC_BASE_URL must use https in production");
    }
    let webauthn_origin =
        Url::parse(webauthn_rp_origin).context("WEBAUTHN_RP_ORIGIN must be an absolute URL")?;
    if email.public_base_url.host_str() != webauthn_origin.host_str() {
        bail!("PUBLIC_BASE_URL and WEBAUTHN_RP_ORIGIN must use the same host");
    }
    if email.token_secret.len() < 43
        || email.token_secret == jwt_secret
        || email.token_secret.contains("replace-with")
    {
        bail!("EMAIL_TOKEN_SECRET must be a distinct non-placeholder 256-bit secret");
    }
    Ok(())
}

fn email_config(default_base_url: &str) -> anyhow::Result<EmailConfig> {
    let enabled = parse_bool("EMAIL_DELIVERY_ENABLED", false)?;
    let public_base_url = Url::parse(
        &std::env::var("PUBLIC_BASE_URL").unwrap_or_else(|_| default_base_url.to_owned()),
    )
    .context("PUBLIC_BASE_URL must be an absolute URL")?;
    if !matches!(public_base_url.scheme(), "http" | "https") || public_base_url.host().is_none() {
        bail!("PUBLIC_BASE_URL must be an absolute http(s) URL");
    }
    if !public_base_url.username().is_empty()
        || public_base_url.password().is_some()
        || public_base_url.query().is_some()
        || public_base_url.fragment().is_some()
    {
        bail!("PUBLIC_BASE_URL must not contain credentials, a query, or a fragment");
    }
    let token_secret = optional_secret("EMAIL_TOKEN_SECRET")?.unwrap_or_default();
    if enabled && token_secret.len() < 32 {
        bail!("EMAIL_TOKEN_SECRET must contain at least 32 bytes when email is enabled");
    }
    let smtp_host = nonempty_env("SMTP_HOST", "localhost")?;
    let smtp_port = std::env::var("SMTP_PORT")
        .unwrap_or_else(|_| "1025".to_owned())
        .parse::<u16>()
        .context("SMTP_PORT must be a valid TCP port")?;
    if smtp_port == 0 {
        bail!("SMTP_PORT must not be zero");
    }
    let smtp_username = optional_nonempty_env("SMTP_USERNAME")?;
    let smtp_password = optional_secret("SMTP_PASSWORD")?;
    if smtp_username.is_some() != smtp_password.is_some() {
        bail!("SMTP_USERNAME and SMTP_PASSWORD must be configured together");
    }
    let smtp_tls = match std::env::var("SMTP_TLS")
        .unwrap_or_else(|_| "none".to_owned())
        .to_ascii_lowercase()
        .as_str()
    {
        "starttls" => SmtpTls::StartTls,
        "none" => SmtpTls::None,
        _ => bail!("SMTP_TLS must be starttls or none"),
    };
    let from_address = nonempty_env("SMTP_FROM_ADDRESS", "no-reply@localhost")?;
    if !email_address::EmailAddress::is_valid(&from_address) {
        bail!("SMTP_FROM_ADDRESS must be a valid email address");
    }
    let from_name = nonempty_env("SMTP_FROM_NAME", "PDF Editor")?;

    Ok(EmailConfig {
        enabled,
        public_base_url,
        token_secret,
        smtp_host,
        smtp_port,
        smtp_username,
        smtp_password,
        smtp_tls,
        from_address,
        from_name,
    })
}

fn optional_nonempty_env(name: &str) -> anyhow::Result<Option<String>> {
    match std::env::var(name) {
        Ok(value) if value.trim().is_empty() => bail!("{name} must not be empty"),
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error).with_context(|| format!("could not read {name}")),
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
                &EmailConfig {
                    enabled: true,
                    public_base_url: Url::parse("https://pdf.example.com").unwrap(),
                    token_secret: "b".repeat(43),
                    smtp_host: "smtp.example.com".to_owned(),
                    smtp_port: 587,
                    smtp_username: Some("app".to_owned()),
                    smtp_password: Some("secret".to_owned()),
                    smtp_tls: SmtpTls::StartTls,
                    from_address: "no-reply@example.com".to_owned(),
                    from_name: "PDF Editor".to_owned(),
                },
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
                &EmailConfig {
                    enabled: false,
                    public_base_url: Url::parse("http://localhost:8080").unwrap(),
                    token_secret: String::new(),
                    smtp_host: "localhost".to_owned(),
                    smtp_port: 1025,
                    smtp_username: None,
                    smtp_password: None,
                    smtp_tls: SmtpTls::None,
                    from_address: "no-reply@localhost".to_owned(),
                    from_name: "PDF Editor".to_owned(),
                },
            )
            .is_err()
        );
    }
}
