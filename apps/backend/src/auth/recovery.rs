use sqlx::{query, query_as};
use uuid::Uuid;

use crate::email::AccountTokenPurpose;

use super::crypto::hash_password;
use super::error::AuthError;
use super::model::{PasswordResetConfirmRequest, PasswordResetRequest};
use super::service::{
    AuthService, RequestContext, insert_audit, normalize_email_for_login, validate_password,
};

impl AuthService {
    /// Queue a fresh verification link for the authenticated account.
    pub(super) async fn request_email_verification(
        &self,
        access_token: &str,
        context: RequestContext<'_>,
    ) -> Result<(), AuthError> {
        let (claims, user) = self.authenticated_user(access_token).await?;
        if user.email_verified_at.is_some() {
            return Ok(());
        }
        let email = self.email.as_ref().ok_or(AuthError::Unavailable)?;
        let mut transaction = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AuthError::internal)?;
        email
            .queue_verification(&mut transaction, user.id, &user.email, &user.display_name)
            .await
            .map_err(AuthError::internal)?;
        insert_audit(
            &mut transaction,
            Some(user.id),
            Some(claims.sid),
            "auth.email_verification_requested",
            "success",
            context,
        )
        .await?;
        transaction.commit().await.map_err(AuthError::internal)
    }

    /// Consume a signed, one-time email verification token.
    pub(super) async fn confirm_email_verification(
        &self,
        raw_token: &str,
        context: RequestContext<'_>,
    ) -> Result<(), AuthError> {
        let email = self.email.as_ref().ok_or(AuthError::Unavailable)?;
        let token = email
            .validate_token(raw_token, AccountTokenPurpose::VerifyEmail)
            .await
            .map_err(|error| {
                tracing::warn!(%error, "email_verification_token_rejected");
                AuthError::InvalidAccountToken
            })?;
        let mut transaction = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AuthError::internal)?;
        let consumed_user = query_as::<_, ConsumedToken>(
            r"UPDATE account_tokens SET consumed_at = now()
               WHERE id = $1 AND user_id = $2 AND purpose = 'verify_email'
                 AND consumed_at IS NULL AND expires_at > now()
               RETURNING user_id",
        )
        .bind(token.id)
        .bind(token.user_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(AuthError::internal)?
        .ok_or(AuthError::InvalidAccountToken)?;
        query(
            r"UPDATE users
               SET email_verified_at = COALESCE(email_verified_at, now()), updated_at = now()
               WHERE id = $1",
        )
        .bind(consumed_user.user_id)
        .execute(&mut *transaction)
        .await
        .map_err(AuthError::internal)?;
        insert_audit(
            &mut transaction,
            Some(consumed_user.user_id),
            None,
            "auth.email_verified",
            "success",
            context,
        )
        .await?;
        transaction.commit().await.map_err(AuthError::internal)
    }

    /// Queue password recovery without revealing whether the identity exists.
    pub(super) async fn request_password_reset(
        &self,
        request: PasswordResetRequest,
        context: RequestContext<'_>,
    ) -> Result<(), AuthError> {
        if request.email.len() > 320 || request.email.chars().any(char::is_control) {
            return Ok(());
        }
        let normalized = normalize_email_for_login(&request.email);
        let account = query_as::<_, RecoveryAccount>(
            r"SELECT id, email, display_name
               FROM users WHERE email_normalized = $1 AND status = 'active'",
        )
        .bind(normalized)
        .fetch_optional(self.database.pool())
        .await
        .map_err(AuthError::internal)?;
        let (Some(email), Some(account)) = (self.email.as_ref(), account) else {
            return Ok(());
        };
        let mut transaction = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AuthError::internal)?;
        email
            .queue_password_reset(
                &mut transaction,
                account.id,
                &account.email,
                &account.display_name,
            )
            .await
            .map_err(AuthError::internal)?;
        insert_audit(
            &mut transaction,
            Some(account.id),
            None,
            "auth.password_reset_requested",
            "success",
            context,
        )
        .await?;
        transaction.commit().await.map_err(AuthError::internal)
    }

    /// Consume a password recovery token, rotate the credential version and
    /// revoke all browser sessions.
    pub(super) async fn confirm_password_reset(
        &self,
        request: PasswordResetConfirmRequest,
        context: RequestContext<'_>,
    ) -> Result<(), AuthError> {
        validate_password(&request.new_password)?;
        let email = self.email.as_ref().ok_or(AuthError::Unavailable)?;
        let token = email
            .validate_token(&request.token, AccountTokenPurpose::ResetPassword)
            .await
            .map_err(|error| {
                tracing::warn!(%error, "password_reset_token_rejected");
                AuthError::InvalidAccountToken
            })?;
        let password_hash = hash_password(request.new_password).await?;
        let mut transaction = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AuthError::internal)?;
        let consumed_user = query_as::<_, ConsumedToken>(
            r"UPDATE account_tokens SET consumed_at = now()
               WHERE id = $1 AND user_id = $2 AND purpose = 'reset_password'
                 AND consumed_at IS NULL AND expires_at > now()
               RETURNING user_id",
        )
        .bind(token.id)
        .bind(token.user_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(AuthError::internal)?
        .ok_or(AuthError::InvalidAccountToken)?;
        let updated = query(
            r"UPDATE users
               SET password_hash = $2, token_version = token_version + 1,
                   failed_login_attempts = 0, locked_until = NULL, updated_at = now()
               WHERE id = $1 AND status = 'active'",
        )
        .bind(consumed_user.user_id)
        .bind(password_hash)
        .execute(&mut *transaction)
        .await
        .map_err(AuthError::internal)?;
        if updated.rows_affected() != 1 {
            return Err(AuthError::InvalidAccountToken);
        }
        query("UPDATE sessions SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL")
            .bind(consumed_user.user_id)
            .execute(&mut *transaction)
            .await
            .map_err(AuthError::internal)?;
        query(
            r"DELETE FROM account_tokens
               WHERE user_id = $1 AND purpose = 'reset_password' AND id <> $2",
        )
        .bind(consumed_user.user_id)
        .bind(token.id)
        .execute(&mut *transaction)
        .await
        .map_err(AuthError::internal)?;
        insert_audit(
            &mut transaction,
            Some(consumed_user.user_id),
            None,
            "auth.password_reset_completed",
            "success",
            context,
        )
        .await?;
        transaction.commit().await.map_err(AuthError::internal)
    }
}

#[derive(sqlx::FromRow)]
struct RecoveryAccount {
    id: Uuid,
    email: String,
    display_name: String,
}

#[derive(sqlx::FromRow)]
struct ConsumedToken {
    user_id: Uuid,
}
