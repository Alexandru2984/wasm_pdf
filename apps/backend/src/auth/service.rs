use email_address::EmailAddress;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use sqlx::postgres::PgDatabaseError;
use sqlx::{Postgres, Transaction, query, query_as};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;
use webauthn_rs::prelude::{Url, Webauthn, WebauthnBuilder};

use crate::{AuthConfig, Database};

use super::crypto::{hash_password, random_token, token_hash, verify_password};
use super::error::AuthError;
use super::model::{
    AccessClaims, AuthResponse, LoginOutcome, LoginRequest, MeResponse, PublicUser,
    RegisterRequest, SessionBundle, SessionUserRecord, UserRecord,
};
use super::rate_limit::{RateLimitCategory, RateLimiter};

const MAX_LOGIN_ATTEMPTS: i32 = 5;
const LOGIN_LOCK_MINUTES: i64 = 15;
const SESSION_IDLE_DAYS: i64 = 7;

#[derive(Clone)]
pub struct AuthService {
    pub(super) database: Database,
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    validation: Validation,
    issuer: String,
    audience: String,
    access_token_seconds: i64,
    session_days: i64,
    cookie_secure: bool,
    dummy_password_hash: String,
    rate_limiter: RateLimiter,
    pub(super) webauthn: Webauthn,
}

impl AuthService {
    /// Construct the service and pre-compute a dummy password hash used to make
    /// unknown-account login attempts follow the same expensive verification path.
    ///
    /// # Errors
    ///
    /// Returns an error if the password hashing worker cannot initialize.
    pub async fn new(database: Database, config: &AuthConfig) -> Result<Self, AuthError> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(&[&config.jwt_issuer]);
        validation.set_audience(&[&config.jwt_audience]);
        validation.leeway = 5;
        let dummy_password_hash = hash_password(random_token()).await?;
        let rate_limiter = RateLimiter::new(database.clone(), config.jwt_secret.as_bytes());
        let origin = Url::parse(&config.webauthn_rp_origin).map_err(AuthError::internal)?;
        let webauthn = WebauthnBuilder::new(&config.webauthn_rp_id, &origin)
            .map_err(AuthError::internal)?
            .rp_name(&config.webauthn_rp_name)
            .build()
            .map_err(AuthError::internal)?;
        Ok(Self {
            database,
            encoding_key: EncodingKey::from_secret(config.jwt_secret.as_bytes()),
            decoding_key: DecodingKey::from_secret(config.jwt_secret.as_bytes()),
            validation,
            issuer: config.jwt_issuer.clone(),
            audience: config.jwt_audience.clone(),
            access_token_seconds: config.access_token_seconds,
            session_days: config.session_days,
            cookie_secure: config.cookie_secure,
            dummy_password_hash,
            rate_limiter,
            webauthn,
        })
    }

    pub const fn cookie_secure(&self) -> bool {
        self.cookie_secure
    }

    pub const fn session_days(&self) -> i64 {
        self.session_days
    }

    pub(crate) async fn enforce_rate_limit(
        &self,
        category: RateLimitCategory,
        scope: &str,
    ) -> Result<(), AuthError> {
        self.rate_limiter.enforce(category, scope).await
    }

    /// Register a new account and create its initial session.
    ///
    /// # Errors
    ///
    /// Returns validation/conflict errors or an internal persistence/crypto error.
    pub async fn register(
        &self,
        request: RegisterRequest,
        user_agent: Option<&str>,
    ) -> Result<SessionBundle, AuthError> {
        let email = validate_email(&request.email)?;
        let display_name = validate_display_name(&request.display_name)?;
        validate_password(&request.password)?;
        let password_hash = hash_password(request.password).await?;
        let user_id = Uuid::new_v4();
        let mut transaction = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AuthError::internal)?;
        let user = query_as::<_, UserRecord>(
            r"INSERT INTO users (id, email, email_normalized, display_name, password_hash)
               VALUES ($1, $2, $3, $4, $5)
               RETURNING id, email, display_name, password_hash, status, mfa_required,
                         token_version, failed_login_attempts, locked_until",
        )
        .bind(user_id)
        .bind(email.trim())
        .bind(&email)
        .bind(display_name)
        .bind(password_hash)
        .fetch_one(&mut *transaction)
        .await
        .map_err(map_registration_error)?;
        let bundle = self
            .create_session(&mut transaction, &user, user_agent)
            .await?;
        insert_audit(
            &mut transaction,
            Some(user.id),
            Some(bundle.session_id),
            "auth.register",
            "success",
            user_agent,
        )
        .await?;
        transaction.commit().await.map_err(AuthError::internal)?;
        Ok(bundle)
    }

    /// Verify credentials and create a new session.
    ///
    /// # Errors
    ///
    /// Returns generic invalid credentials or an internal persistence/crypto error.
    pub async fn login(
        &self,
        request: LoginRequest,
        user_agent: Option<&str>,
    ) -> Result<LoginOutcome, AuthError> {
        let normalized = normalize_email_for_login(&request.email);
        if request.password.len() > 1_024 {
            return Err(AuthError::InvalidCredentials);
        }
        let user = query_as::<_, UserRecord>(
            r"SELECT id, email, display_name, password_hash, status, mfa_required,
                      token_version, failed_login_attempts, locked_until
               FROM users WHERE email_normalized = $1",
        )
        .bind(&normalized)
        .fetch_optional(self.database.pool())
        .await
        .map_err(AuthError::internal)?;
        let hash = user.as_ref().map_or_else(
            || self.dummy_password_hash.clone(),
            |user| user.password_hash.clone(),
        );
        let password_valid = verify_password(request.password, hash).await?;
        let Some(user) = user else {
            return Err(AuthError::InvalidCredentials);
        };
        let now = OffsetDateTime::now_utc();
        if !password_valid
            || user.status != "active"
            || user.locked_until.is_some_and(|until| until > now)
        {
            if !password_valid {
                self.record_failed_login(&user).await?;
            }
            return Err(AuthError::InvalidCredentials);
        }

        if user.mfa_required {
            let challenge = self.start_passkey_authentication(&user).await?;
            self.reset_successful_login(&user).await?;
            return Ok(LoginOutcome::PasskeyRequired(challenge));
        }

        let mut transaction = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AuthError::internal)?;
        query(
            r"UPDATE users
               SET failed_login_attempts = 0, locked_until = NULL,
                   last_login_at = now(), updated_at = now()
               WHERE id = $1",
        )
        .bind(user.id)
        .execute(&mut *transaction)
        .await
        .map_err(AuthError::internal)?;
        let bundle = self
            .create_session(&mut transaction, &user, user_agent)
            .await?;
        insert_audit(
            &mut transaction,
            Some(user.id),
            Some(bundle.session_id),
            "auth.login",
            "success",
            user_agent,
        )
        .await?;
        transaction.commit().await.map_err(AuthError::internal)?;
        Ok(LoginOutcome::Authenticated(bundle))
    }

    /// Rotate a valid session after CSRF verification and issue a fresh access token.
    ///
    /// # Errors
    ///
    /// Returns unauthorized/CSRF errors or an internal persistence/crypto error.
    pub async fn refresh(
        &self,
        session_token: &str,
        csrf_token: &str,
        user_agent: Option<&str>,
    ) -> Result<SessionBundle, AuthError> {
        let session_hash = token_hash(session_token);
        let csrf_hash = token_hash(csrf_token);
        let current = query_as::<_, RefreshRecord>(
            r"SELECT s.id AS session_id, s.csrf_token_hash, u.id AS id,
                      u.email, u.display_name, u.password_hash, u.status,
                      u.mfa_required, u.token_version, u.failed_login_attempts,
                      u.locked_until
               FROM sessions s JOIN users u ON u.id = s.user_id
               WHERE s.token_hash = $1 AND s.revoked_at IS NULL
                 AND s.expires_at > now() AND s.idle_expires_at > now()",
        )
        .bind(session_hash)
        .fetch_optional(self.database.pool())
        .await
        .map_err(AuthError::internal)?
        .ok_or(AuthError::Unauthorized)?;
        if current.csrf_token_hash != csrf_hash {
            return Err(AuthError::InvalidCsrf);
        }
        if current.user.status != "active" {
            return Err(AuthError::Unauthorized);
        }

        let mut transaction = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AuthError::internal)?;
        let rotation =
            query("UPDATE sessions SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL")
                .bind(current.session_id)
                .execute(&mut *transaction)
                .await
                .map_err(AuthError::internal)?;
        if rotation.rows_affected() != 1 {
            return Err(AuthError::Unauthorized);
        }
        let bundle = self
            .create_session(&mut transaction, &current.user, user_agent)
            .await?;
        insert_audit(
            &mut transaction,
            Some(current.user.id),
            Some(bundle.session_id),
            "auth.refresh",
            "success",
            user_agent,
        )
        .await?;
        transaction.commit().await.map_err(AuthError::internal)?;
        Ok(bundle)
    }

    /// Revoke a session after CSRF verification.
    ///
    /// # Errors
    ///
    /// Returns a CSRF error or an internal persistence error.
    pub async fn logout(
        &self,
        session_token: &str,
        csrf_token: &str,
        user_agent: Option<&str>,
    ) -> Result<(), AuthError> {
        let mut transaction = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AuthError::internal)?;
        let revoked = query_as::<_, RevokedSession>(
            r"UPDATE sessions SET revoked_at = now()
               WHERE token_hash = $1 AND csrf_token_hash = $2 AND revoked_at IS NULL
               RETURNING id, user_id",
        )
        .bind(token_hash(session_token))
        .bind(token_hash(csrf_token))
        .fetch_optional(&mut *transaction)
        .await
        .map_err(AuthError::internal)?
        .ok_or(AuthError::InvalidCsrf)?;
        insert_audit(
            &mut transaction,
            Some(revoked.user_id),
            Some(revoked.id),
            "auth.logout",
            "success",
            user_agent,
        )
        .await?;
        transaction.commit().await.map_err(AuthError::internal)?;
        Ok(())
    }

    /// Resolve the current user from a signed access token and active session.
    ///
    /// # Errors
    ///
    /// Returns unauthorized or an internal persistence error.
    pub async fn me(&self, access_token: &str) -> Result<MeResponse, AuthError> {
        let claims = self.decode_access_token(access_token)?;
        let record = query_as::<_, SessionUserRecord>(
            r"SELECT s.id AS session_id, u.id AS user_id, u.email, u.display_name,
                      u.status, u.mfa_required, u.token_version
               FROM sessions s JOIN users u ON u.id = s.user_id
               WHERE s.id = $1 AND u.id = $2 AND s.revoked_at IS NULL
                 AND s.expires_at > now() AND s.idle_expires_at > now()",
        )
        .bind(claims.sid)
        .bind(claims.sub)
        .fetch_optional(self.database.pool())
        .await
        .map_err(AuthError::internal)?
        .ok_or(AuthError::Unauthorized)?;
        if record.status != "active" || record.token_version != claims.ver {
            return Err(AuthError::Unauthorized);
        }
        Ok(MeResponse {
            user: PublicUser::from(&record),
        })
    }

    pub(super) fn decode_access_token(&self, token: &str) -> Result<AccessClaims, AuthError> {
        decode::<AccessClaims>(token, &self.decoding_key, &self.validation)
            .map(|data| data.claims)
            .map_err(|_| AuthError::Unauthorized)
    }

    pub(super) async fn create_session(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        user: &UserRecord,
        user_agent: Option<&str>,
    ) -> Result<SessionBundle, AuthError> {
        let session_id = Uuid::new_v4();
        let session_token = random_token();
        let csrf_token = random_token();
        let now = OffsetDateTime::now_utc();
        let expires_at = now + Duration::days(self.session_days);
        let idle_expires_at = now + Duration::days(SESSION_IDLE_DAYS.min(self.session_days));
        query(
            r"INSERT INTO sessions
               (id, user_id, token_hash, csrf_token_hash, user_agent, expires_at, idle_expires_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(session_id)
        .bind(user.id)
        .bind(token_hash(&session_token))
        .bind(token_hash(&csrf_token))
        .bind(user_agent.map(|value| truncate(value, 512)))
        .bind(expires_at)
        .bind(idle_expires_at)
        .execute(&mut **transaction)
        .await
        .map_err(AuthError::internal)?;
        let claims = AccessClaims {
            sub: user.id,
            sid: session_id,
            ver: user.token_version,
            iss: self.issuer.clone(),
            aud: self.audience.clone(),
            iat: now.unix_timestamp(),
            exp: (now + Duration::seconds(self.access_token_seconds)).unix_timestamp(),
            jti: Uuid::new_v4(),
        };
        let access_token = encode(&Header::new(Algorithm::HS256), &claims, &self.encoding_key)
            .map_err(AuthError::internal)?;
        Ok(SessionBundle {
            response: AuthResponse {
                access_token,
                token_type: "Bearer",
                expires_in: self.access_token_seconds,
                csrf_token,
                user: PublicUser::from(user),
            },
            session_token,
            session_id,
        })
    }

    async fn record_failed_login(&self, user: &UserRecord) -> Result<(), AuthError> {
        query(
            r"UPDATE users
               SET failed_login_attempts =
                       CASE WHEN failed_login_attempts + 1 >= $2
                            THEN 0 ELSE failed_login_attempts + 1 END,
                   locked_until =
                       CASE WHEN failed_login_attempts + 1 >= $2
                            THEN now() + make_interval(mins => $3) ELSE locked_until END,
                   updated_at = now()
               WHERE id = $1",
        )
        .bind(user.id)
        .bind(MAX_LOGIN_ATTEMPTS)
        .bind(i32::try_from(LOGIN_LOCK_MINUTES).unwrap_or(15))
        .execute(self.database.pool())
        .await
        .map_err(AuthError::internal)?;
        Ok(())
    }

    async fn reset_successful_login(&self, user: &UserRecord) -> Result<(), AuthError> {
        query(
            r"UPDATE users
               SET failed_login_attempts = 0, locked_until = NULL,
                   last_login_at = now(), updated_at = now()
               WHERE id = $1",
        )
        .bind(user.id)
        .execute(self.database.pool())
        .await
        .map_err(AuthError::internal)?;
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct RefreshRecord {
    session_id: Uuid,
    csrf_token_hash: Vec<u8>,
    #[sqlx(flatten)]
    user: UserRecord,
}

#[derive(sqlx::FromRow)]
struct RevokedSession {
    id: Uuid,
    user_id: Uuid,
}

pub(super) async fn insert_audit(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Option<Uuid>,
    session_id: Option<Uuid>,
    event_type: &str,
    outcome: &str,
    user_agent: Option<&str>,
) -> Result<(), AuthError> {
    query(
        r"INSERT INTO audit_events
           (id, user_id, session_id, event_type, outcome, user_agent)
           VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(session_id)
    .bind(event_type)
    .bind(outcome)
    .bind(user_agent.map(|value| truncate(value, 512)))
    .execute(&mut **transaction)
    .await
    .map_err(AuthError::internal)?;
    Ok(())
}

fn validate_email(value: &str) -> Result<String, AuthError> {
    let normalized = value.trim().to_lowercase();
    if normalized.len() > 254 || !EmailAddress::is_valid(&normalized) {
        return Err(AuthError::Validation("Email is invalid."));
    }
    Ok(normalized)
}

fn normalize_email_for_login(value: &str) -> String {
    value.trim().to_lowercase()
}

fn validate_display_name(value: &str) -> Result<&str, AuthError> {
    let trimmed = value.trim();
    let characters = trimmed.chars().count();
    if !(2..=80).contains(&characters) || trimmed.chars().any(char::is_control) {
        return Err(AuthError::Validation(
            "Display name must contain between 2 and 80 characters.",
        ));
    }
    Ok(trimmed)
}

fn validate_password(value: &str) -> Result<(), AuthError> {
    let characters = value.chars().count();
    if !(12..=128).contains(&characters) || value.len() > 512 {
        return Err(AuthError::Validation(
            "Password must contain between 12 and 128 characters.",
        ));
    }
    Ok(())
}

fn truncate(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn map_registration_error(error: sqlx::Error) -> AuthError {
    if error
        .as_database_error()
        .and_then(|database| database.try_downcast_ref::<PgDatabaseError>())
        .is_some_and(|database| database.code() == "23505")
    {
        AuthError::EmailTaken
    } else {
        AuthError::internal(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_identity_input() {
        assert_eq!(
            validate_email(" USER@Example.COM ").expect("email"),
            "user@example.com"
        );
        assert!(validate_email("not-an-email").is_err());
        assert!(validate_display_name("A").is_err());
        assert!(validate_password("short").is_err());
        assert!(validate_password("a long enough password").is_ok());
    }

    #[test]
    fn truncation_preserves_utf8_boundaries() {
        assert_eq!(truncate("abcțdef", 4), "abc");
        assert_eq!(truncate("abcdef", 6), "abcdef");
    }
}
