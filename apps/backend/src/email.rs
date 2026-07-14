use std::sync::Arc;
use std::time::Duration as StdDuration;

use anyhow::{Context, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use lettre::message::{Mailbox, MultiPart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use sha2::Sha256;
use sqlx::{Postgres, Transaction, query, query_as, query_scalar};
use time::{Duration, OffsetDateTime};
use tokio::task::JoinHandle;
use url::Url;
use uuid::Uuid;

use crate::{Database, EmailConfig, Metrics, SmtpTls};

type HmacSha256 = Hmac<Sha256>;

const TOKEN_VERSION: &str = "v1";
const VERIFICATION_LIFETIME: Duration = Duration::hours(24);
const PASSWORD_RESET_LIFETIME: Duration = Duration::minutes(30);
const OUTBOX_POLL_INTERVAL: StdDuration = StdDuration::from_secs(2);
const OUTBOX_CLEANUP_INTERVAL: StdDuration = StdDuration::from_secs(60 * 60);
const MAX_DELIVERY_ATTEMPTS: i32 = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AccountTokenPurpose {
    VerifyEmail,
    ResetPassword,
}

impl AccountTokenPurpose {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::VerifyEmail => "verify_email",
            Self::ResetPassword => "reset_password",
        }
    }

    const fn path(self) -> &'static str {
        match self {
            Self::VerifyEmail => "/verify-email",
            Self::ResetPassword => "/reset-password",
        }
    }

    const fn lifetime(self) -> Duration {
        match self {
            Self::VerifyEmail => VERIFICATION_LIFETIME,
            Self::ResetPassword => PASSWORD_RESET_LIFETIME,
        }
    }
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub(crate) struct AccountToken {
    pub id: Uuid,
    pub user_id: Uuid,
    pub purpose: String,
    pub expires_at: OffsetDateTime,
    pub consumed_at: Option<OffsetDateTime>,
}

#[derive(Clone)]
pub struct EmailService {
    database: Database,
    mailer: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
    base_url: Url,
    token_key: Arc<[u8]>,
    metrics: Metrics,
}

impl EmailService {
    /// Build the SMTP delivery service. Disabled delivery returns `None`.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid sender, relay or TLS configuration.
    pub fn from_config(
        database: Database,
        config: &EmailConfig,
        metrics: Metrics,
    ) -> anyhow::Result<Option<Self>> {
        if !config.enabled {
            return Ok(None);
        }

        let from_address = config
            .from_address
            .parse()
            .context("SMTP_FROM_ADDRESS is not a valid mailbox")?;
        let from = Mailbox::new(Some(config.from_name.clone()), from_address);
        let mut builder = match config.smtp_tls {
            SmtpTls::StartTls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.smtp_host)
                    .context("could not configure SMTP STARTTLS")?
            }
            SmtpTls::None => {
                AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.smtp_host)
            }
        }
        .port(config.smtp_port)
        .timeout(Some(StdDuration::from_secs(10)));
        if let (Some(username), Some(password)) = (&config.smtp_username, &config.smtp_password) {
            builder = builder.credentials(Credentials::new(username.clone(), password.clone()));
        }

        Ok(Some(Self {
            database,
            mailer: builder.build(),
            from,
            base_url: config.public_base_url.clone(),
            token_key: Arc::from(config.token_secret.as_bytes()),
            metrics,
        }))
    }

    pub fn spawn_dispatcher(&self) -> JoinHandle<()> {
        let service = self.clone();
        tokio::spawn(async move {
            service.recover_stale_claims().await;
            let mut last_cleanup = tokio::time::Instant::now() - OUTBOX_CLEANUP_INTERVAL;
            loop {
                if last_cleanup.elapsed() >= OUTBOX_CLEANUP_INTERVAL {
                    service.cleanup_expired_tokens().await;
                    last_cleanup = tokio::time::Instant::now();
                }
                match service.dispatch_one().await {
                    Ok(true) => {}
                    Ok(false) => tokio::time::sleep(OUTBOX_POLL_INTERVAL).await,
                    Err(error) => {
                        tracing::error!(%error, "email_outbox_dispatch_failed");
                        tokio::time::sleep(OUTBOX_POLL_INTERVAL).await;
                    }
                }
            }
        })
    }

    pub(crate) async fn queue_verification(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        user_id: Uuid,
        recipient: &str,
        recipient_name: &str,
    ) -> anyhow::Result<()> {
        self.queue_token(
            transaction,
            user_id,
            recipient,
            recipient_name,
            AccountTokenPurpose::VerifyEmail,
        )
        .await
    }

    pub(crate) async fn queue_password_reset(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        user_id: Uuid,
        recipient: &str,
        recipient_name: &str,
    ) -> anyhow::Result<()> {
        self.queue_token(
            transaction,
            user_id,
            recipient,
            recipient_name,
            AccountTokenPurpose::ResetPassword,
        )
        .await
    }

    async fn queue_token(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        user_id: Uuid,
        recipient: &str,
        recipient_name: &str,
        purpose: AccountTokenPurpose,
    ) -> anyhow::Result<()> {
        query_scalar::<_, Uuid>("SELECT id FROM users WHERE id = $1 FOR UPDATE")
            .bind(user_id)
            .fetch_one(&mut **transaction)
            .await?;
        query(
            r"DELETE FROM account_tokens
               WHERE user_id = $1 AND purpose = $2 AND consumed_at IS NULL",
        )
        .bind(user_id)
        .bind(purpose.as_str())
        .execute(&mut **transaction)
        .await?;

        let token_id = Uuid::new_v4();
        let expires_at = OffsetDateTime::now_utc() + purpose.lifetime();
        query(
            r"INSERT INTO account_tokens (id, user_id, purpose, expires_at)
               VALUES ($1, $2, $3, $4)",
        )
        .bind(token_id)
        .bind(user_id)
        .bind(purpose.as_str())
        .bind(expires_at)
        .execute(&mut **transaction)
        .await?;
        query(
            r"INSERT INTO email_outbox
               (id, account_token_id, recipient, recipient_name, template)
               VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(Uuid::new_v4())
        .bind(token_id)
        .bind(recipient)
        .bind(recipient_name)
        .bind(purpose.as_str())
        .execute(&mut **transaction)
        .await?;
        Ok(())
    }

    pub(crate) async fn validate_token(
        &self,
        raw_token: &str,
        expected_purpose: AccountTokenPurpose,
    ) -> anyhow::Result<AccountToken> {
        if raw_token.len() > 128 || !raw_token.is_ascii() {
            bail!("invalid account token length");
        }
        let mut parts = raw_token.split('.');
        let version = parts.next();
        let encoded_id = parts.next();
        let encoded_mac = parts.next();
        if version != Some(TOKEN_VERSION)
            || encoded_id.is_none()
            || encoded_mac.is_none()
            || parts.next().is_some()
        {
            bail!("invalid account token format");
        }
        let id_bytes = URL_SAFE_NO_PAD
            .decode(encoded_id.unwrap_or_default())
            .context("invalid account token id")?;
        let token_id = Uuid::from_slice(&id_bytes).context("invalid account token id")?;
        let supplied_mac = URL_SAFE_NO_PAD
            .decode(encoded_mac.unwrap_or_default())
            .context("invalid account token signature")?;
        let record = query_as::<_, AccountToken>(
            r"SELECT id, user_id, purpose, expires_at, consumed_at
               FROM account_tokens WHERE id = $1 AND purpose = $2",
        )
        .bind(token_id)
        .bind(expected_purpose.as_str())
        .fetch_optional(self.database.pool())
        .await?
        .context("account token does not exist")?;
        self.verify_signature(&record, &supplied_mac)?;
        if record.consumed_at.is_some() || record.expires_at <= OffsetDateTime::now_utc() {
            bail!("account token is expired or consumed");
        }
        Ok(record)
    }

    fn signed_token(&self, token: &AccountToken) -> anyhow::Result<String> {
        build_signed_token(&self.token_key, token)
    }

    fn verify_signature(&self, token: &AccountToken, supplied: &[u8]) -> anyhow::Result<()> {
        verify_account_signature(&self.token_key, token, supplied)
    }

    async fn recover_stale_claims(&self) {
        if let Err(error) = query(
            r"UPDATE email_outbox
               SET status = 'queued', claimed_at = NULL, available_at = now()
               WHERE status = 'processing' AND claimed_at < now() - interval '15 minutes'",
        )
        .execute(self.database.pool())
        .await
        {
            tracing::warn!(%error, "email_outbox_recovery_failed");
        }
    }

    async fn cleanup_expired_tokens(&self) {
        if let Err(error) = query(
            r"DELETE FROM account_tokens
               WHERE expires_at < now() - interval '7 days'
                  OR consumed_at < now() - interval '30 days'",
        )
        .execute(self.database.pool())
        .await
        {
            tracing::warn!(%error, "account_token_cleanup_failed");
        }
    }

    async fn dispatch_one(&self) -> anyhow::Result<bool> {
        let Some(item) = query_as::<_, OutboxItem>(
            r"WITH candidate AS (
                   SELECT id FROM email_outbox
                   WHERE status = 'queued' AND available_at <= now()
                   ORDER BY available_at, created_at
                   FOR UPDATE SKIP LOCKED LIMIT 1
               )
               UPDATE email_outbox AS outbox
               SET status = 'processing', attempts = attempts + 1, claimed_at = now()
               FROM candidate, account_tokens AS token
               WHERE outbox.id = candidate.id AND token.id = outbox.account_token_id
               RETURNING outbox.id, outbox.recipient, outbox.recipient_name,
                         outbox.template, outbox.attempts, token.id AS token_id,
                         token.user_id, token.purpose, token.expires_at, token.consumed_at",
        )
        .fetch_optional(self.database.pool())
        .await?
        else {
            return Ok(false);
        };

        let result = self.deliver(&item).await;
        match result {
            Ok(()) => {
                query(
                    r"UPDATE email_outbox
                       SET status = 'sent', sent_at = now(), claimed_at = NULL, last_error = NULL
                       WHERE id = $1 AND status = 'processing'",
                )
                .bind(item.id)
                .execute(self.database.pool())
                .await?;
                self.metrics.observe_email_delivery("sent");
                tracing::info!(outbox_id = %item.id, template = %item.template, "email_delivered");
            }
            Err(error) => {
                let dead = item.attempts >= MAX_DELIVERY_ATTEMPTS;
                let delay_seconds = (1_i64 << item.attempts.min(10)) * 5;
                let retry_at = OffsetDateTime::now_utc() + Duration::seconds(delay_seconds);
                let error_message = error.to_string();
                let error = truncate_utf8(&error_message, 500);
                query(
                    r"UPDATE email_outbox
                       SET status = $2, available_at = $3, claimed_at = NULL, last_error = $4
                       WHERE id = $1 AND status = 'processing'",
                )
                .bind(item.id)
                .bind(if dead { "dead" } else { "queued" })
                .bind(retry_at)
                .bind(error)
                .execute(self.database.pool())
                .await?;
                self.metrics
                    .observe_email_delivery(if dead { "dead" } else { "retry" });
                tracing::warn!(
                    outbox_id = %item.id,
                    template = %item.template,
                    attempts = item.attempts,
                    dead,
                    "email_delivery_failed"
                );
            }
        }
        Ok(true)
    }

    async fn deliver(&self, item: &OutboxItem) -> anyhow::Result<()> {
        let purpose = match item.template.as_str() {
            "verify_email" => AccountTokenPurpose::VerifyEmail,
            "reset_password" => AccountTokenPurpose::ResetPassword,
            _ => bail!("unsupported email template"),
        };
        let account_token = item.account_token();
        if account_token.purpose != purpose.as_str()
            || account_token.consumed_at.is_some()
            || account_token.expires_at <= OffsetDateTime::now_utc()
        {
            bail!("email account token is no longer deliverable");
        }
        let token = self.signed_token(&account_token)?;
        let mut link = self
            .base_url
            .join(purpose.path())
            .context("could not construct account action URL")?;
        link.query_pairs_mut().append_pair("token", &token);
        let (subject, action, introduction) = match purpose {
            AccountTokenPurpose::VerifyEmail => (
                "Verifică adresa de email",
                "Verifică emailul",
                "Confirmă adresa pentru contul tău PDF Editor.",
            ),
            AccountTokenPurpose::ResetPassword => (
                "Resetează parola",
                "Alege o parolă nouă",
                "Am primit o cerere de resetare a parolei. Ignoră mesajul dacă nu ai inițiat-o.",
            ),
        };
        let text = format!(
            "Salut, {}!\n\n{}\n\n{}\n\nLinkul expiră automat și poate fi folosit o singură dată.",
            item.recipient_name, introduction, link
        );
        let html = format!(
            "<!doctype html><html><body><p>Salut, {}!</p><p>{}</p><p><a href=\"{}\">{}</a></p><p>Linkul expiră automat și poate fi folosit o singură dată.</p></body></html>",
            escape_html(&item.recipient_name),
            escape_html(introduction),
            escape_html(link.as_str()),
            escape_html(action),
        );
        let recipient = Mailbox::new(
            Some(item.recipient_name.clone()),
            item.recipient
                .parse()
                .context("outbox recipient is invalid")?,
        );
        let message = Message::builder()
            .from(self.from.clone())
            .to(recipient)
            .subject(subject)
            .multipart(MultiPart::alternative_plain_html(text, html))?;
        self.mailer
            .send(message)
            .await
            .context("SMTP delivery failed")?;
        Ok(())
    }
}

fn build_signed_token(key: &[u8], token: &AccountToken) -> anyhow::Result<String> {
    let signature = account_signature(key, token)?;
    Ok(format!(
        "{TOKEN_VERSION}.{}.{}",
        URL_SAFE_NO_PAD.encode(token.id.as_bytes()),
        URL_SAFE_NO_PAD.encode(signature)
    ))
}

fn account_signature(key: &[u8], token: &AccountToken) -> anyhow::Result<Vec<u8>> {
    let mut mac = HmacSha256::new_from_slice(key)?;
    mac.update(b"pdf-editor-account-token-v1\0");
    mac.update(token.id.as_bytes());
    mac.update(token.user_id.as_bytes());
    mac.update(token.purpose.as_bytes());
    mac.update(&token.expires_at.unix_timestamp().to_be_bytes());
    Ok(mac.finalize().into_bytes().to_vec())
}

fn verify_account_signature(
    key: &[u8],
    token: &AccountToken,
    supplied: &[u8],
) -> anyhow::Result<()> {
    let mut mac = HmacSha256::new_from_slice(key)?;
    mac.update(b"pdf-editor-account-token-v1\0");
    mac.update(token.id.as_bytes());
    mac.update(token.user_id.as_bytes());
    mac.update(token.purpose.as_bytes());
    mac.update(&token.expires_at.unix_timestamp().to_be_bytes());
    mac.verify_slice(supplied)
        .map_err(|_| anyhow::anyhow!("invalid account token signature"))
}

#[derive(Debug, sqlx::FromRow)]
struct OutboxItem {
    id: Uuid,
    recipient: String,
    recipient_name: String,
    template: String,
    attempts: i32,
    token_id: Uuid,
    user_id: Uuid,
    purpose: String,
    expires_at: OffsetDateTime,
    consumed_at: Option<OffsetDateTime>,
}

impl OutboxItem {
    fn account_token(&self) -> AccountToken {
        AccountToken {
            id: self.token_id,
            user_id: self.user_id,
            purpose: self.purpose.clone(),
            expires_at: self.expires_at,
            consumed_at: self.consumed_at,
        }
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut boundary = max_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &value[..boundary]
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_escaping_covers_attributes_and_text() {
        assert_eq!(
            escape_html("<Alex & 'friends' \"team\">").as_str(),
            "&lt;Alex &amp; &#39;friends&#39; &quot;team&quot;&gt;"
        );
    }

    #[test]
    fn utf8_truncation_preserves_character_boundaries() {
        assert_eq!(truncate_utf8("abc🦀def", 5), "abc");
        assert_eq!(truncate_utf8("short", 20), "short");
    }

    #[test]
    fn account_token_signature_binds_every_security_field() {
        let key = b"test-email-token-key-with-at-least-32-bytes";
        let token = AccountToken {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            purpose: "verify_email".to_owned(),
            expires_at: OffsetDateTime::from_unix_timestamp(1_900_000_000).expect("timestamp"),
            consumed_at: None,
        };
        let raw = build_signed_token(key, &token).expect("signed token");
        let supplied = URL_SAFE_NO_PAD
            .decode(raw.rsplit('.').next().expect("signature"))
            .expect("base64 signature");
        verify_account_signature(key, &token, &supplied).expect("valid signature");

        let mut changed = token;
        changed.purpose = "reset_password".to_owned();
        assert!(verify_account_signature(key, &changed, &supplied).is_err());
    }
}
