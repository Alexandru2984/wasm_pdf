use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use sqlx::{query, query_scalar};
use time::{Duration, OffsetDateTime};

use crate::Database;

use super::error::AuthError;

type HmacSha256 = Hmac<Sha256>;

const CLEANUP_INTERVAL: u64 = 256;

#[derive(Clone, Copy, Debug)]
pub enum RateLimitCategory {
    RegisterIp,
    RegisterIdentity,
    LoginIp,
    LoginIdentity,
    RefreshSession,
    LogoutSession,
    MfaCeremony,
    AccountMutation,
    RecoveryIp,
    RecoveryIdentity,
    RecoveryConfirm,
    TelemetryIp,
}

impl RateLimitCategory {
    const fn policy(self) -> (&'static str, i32, i64) {
        match self {
            Self::RegisterIp => ("register_ip", 5, 3_600),
            Self::RegisterIdentity => ("register_identity", 3, 3_600),
            Self::LoginIp => ("login_ip", 30, 900),
            Self::LoginIdentity => ("login_identity", 10, 900),
            Self::RefreshSession => ("refresh_session", 60, 60),
            Self::LogoutSession => ("logout_session", 30, 60),
            Self::MfaCeremony => ("mfa_ceremony", 10, 300),
            Self::AccountMutation => ("account_mutation", 30, 300),
            Self::RecoveryIp => ("recovery_ip", 20, 3_600),
            Self::RecoveryIdentity => ("recovery_identity", 5, 3_600),
            Self::RecoveryConfirm => ("recovery_confirm", 10, 900),
            Self::TelemetryIp => ("telemetry_ip", 240, 60),
        }
    }
}

#[derive(Clone)]
pub struct RateLimiter {
    database: Database,
    key: Arc<[u8]>,
    cleanup_counter: Arc<AtomicU64>,
}

impl RateLimiter {
    pub fn new(database: Database, secret: &[u8]) -> Self {
        Self {
            database,
            key: Arc::from(secret),
            cleanup_counter: Arc::new(AtomicU64::new(0)),
        }
    }

    pub async fn enforce(&self, category: RateLimitCategory, scope: &str) -> Result<(), AuthError> {
        let (category_name, limit, window_seconds) = category.policy();
        let now = OffsetDateTime::now_utc();
        let window_epoch = now.unix_timestamp().div_euclid(window_seconds) * window_seconds;
        let window_start =
            OffsetDateTime::from_unix_timestamp(window_epoch).map_err(AuthError::internal)?;
        let expires_at = window_start + Duration::seconds(window_seconds * 2);
        let scope_hash = self.scope_hash(category_name, scope)?;
        let count = query_scalar::<_, i32>(
            r"INSERT INTO rate_limit_buckets
               (scope_hash, category, window_start, request_count, expires_at)
               VALUES ($1, $2, $3, 1, $4)
               ON CONFLICT (scope_hash, category, window_start)
               DO UPDATE SET request_count = rate_limit_buckets.request_count + 1
               RETURNING request_count",
        )
        .bind(scope_hash)
        .bind(category_name)
        .bind(window_start)
        .bind(expires_at)
        .fetch_one(self.database.pool())
        .await
        .map_err(AuthError::internal)?;

        self.maybe_cleanup().await;
        if count > limit {
            let retry_after = u64::try_from(window_epoch + window_seconds - now.unix_timestamp())
                .unwrap_or(1)
                .max(1);
            tracing::warn!(category = category_name, retry_after, "auth_rate_limited");
            return Err(AuthError::RateLimited { retry_after });
        }
        Ok(())
    }

    fn scope_hash(&self, category: &str, scope: &str) -> Result<Vec<u8>, AuthError> {
        let mut mac = HmacSha256::new_from_slice(&self.key).map_err(AuthError::internal)?;
        mac.update(b"pdf-editor-rate-limit-v1\0");
        mac.update(category.as_bytes());
        mac.update(b"\0");
        mac.update(scope.as_bytes());
        Ok(mac.finalize().into_bytes().to_vec())
    }

    async fn maybe_cleanup(&self) {
        let count = self.cleanup_counter.fetch_add(1, Ordering::Relaxed);
        if !count.is_multiple_of(CLEANUP_INTERVAL) {
            return;
        }
        if let Err(error) = query(
            r"DELETE FROM rate_limit_buckets
               WHERE ctid IN (
                   SELECT ctid FROM rate_limit_buckets
                   WHERE expires_at < now() LIMIT 500
               )",
        )
        .execute(self.database.pool())
        .await
        {
            tracing::warn!(%error, "rate_limit_cleanup_failed");
        }
    }
}
