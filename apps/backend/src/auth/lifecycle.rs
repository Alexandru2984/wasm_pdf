use sqlx::{query, query_as, query_scalar};
use uuid::Uuid;

use super::crypto::{hash_password, verify_password};
use super::error::AuthError;
use super::model::{
    ChangePasswordRequest, MeResponse, PasswordConfirmationRequest, PublicUser, SessionBundle,
    SessionListResponse, SessionSummary, UpdateProfileRequest, UserRecord,
};
use super::service::{
    AuthService, RequestContext, insert_audit, validate_display_name, validate_password,
};

impl AuthService {
    /// List every active session belonging to the authenticated user.
    pub(super) async fn list_sessions(
        &self,
        access_token: &str,
    ) -> Result<SessionListResponse, AuthError> {
        let (claims, user) = self.authenticated_user(access_token).await?;
        let sessions = query_as::<_, SessionSummary>(
            r"SELECT id, id = $2 AS current, user_agent,
                      host(ip_address) AS ip_address, created_at, last_seen_at,
                      expires_at, idle_expires_at
               FROM sessions
               WHERE user_id = $1 AND revoked_at IS NULL
                 AND expires_at > now() AND idle_expires_at > now()
               ORDER BY current DESC, last_seen_at DESC",
        )
        .bind(user.id)
        .bind(claims.sid)
        .fetch_all(self.database.pool())
        .await
        .map_err(AuthError::internal)?;
        Ok(SessionListResponse { sessions })
    }

    /// Revoke one other session. The active session must use the logout endpoint.
    pub(super) async fn revoke_session(
        &self,
        access_token: &str,
        session_id: Uuid,
        context: RequestContext<'_>,
    ) -> Result<(), AuthError> {
        let (claims, user) = self.authenticated_user(access_token).await?;
        if session_id == claims.sid {
            return Err(AuthError::Validation(
                "Use the logout endpoint to revoke the current session.",
            ));
        }
        let mut transaction = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AuthError::internal)?;
        let result = query(
            r"UPDATE sessions SET revoked_at = now()
               WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL",
        )
        .bind(session_id)
        .bind(user.id)
        .execute(&mut *transaction)
        .await
        .map_err(AuthError::internal)?;
        if result.rows_affected() != 1 {
            return Err(AuthError::NotFound);
        }
        insert_audit(
            &mut transaction,
            Some(user.id),
            Some(claims.sid),
            "auth.session_revoked",
            "success",
            context,
        )
        .await?;
        transaction.commit().await.map_err(AuthError::internal)?;
        Ok(())
    }

    /// Revoke all sessions except the one represented by the access token.
    pub(super) async fn revoke_other_sessions(
        &self,
        access_token: &str,
        context: RequestContext<'_>,
    ) -> Result<(), AuthError> {
        let (claims, user) = self.authenticated_user(access_token).await?;
        let mut transaction = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AuthError::internal)?;
        query(
            r"UPDATE sessions SET revoked_at = now()
               WHERE user_id = $1 AND id <> $2 AND revoked_at IS NULL",
        )
        .bind(user.id)
        .bind(claims.sid)
        .execute(&mut *transaction)
        .await
        .map_err(AuthError::internal)?;
        insert_audit(
            &mut transaction,
            Some(user.id),
            Some(claims.sid),
            "auth.other_sessions_revoked",
            "success",
            context,
        )
        .await?;
        transaction.commit().await.map_err(AuthError::internal)?;
        Ok(())
    }

    /// Change the password, invalidate every existing token and create a new session.
    pub(super) async fn change_password(
        &self,
        access_token: &str,
        request: ChangePasswordRequest,
        context: RequestContext<'_>,
    ) -> Result<SessionBundle, AuthError> {
        let (_, user) = self.authenticated_user(access_token).await?;
        if !verify_password(request.current_password, user.password_hash.clone()).await? {
            return Err(AuthError::InvalidCredentials);
        }
        validate_password(&request.new_password)?;
        if verify_password(request.new_password.clone(), user.password_hash.clone()).await? {
            return Err(AuthError::Validation(
                "The new password must differ from the current password.",
            ));
        }
        let password_hash = hash_password(request.new_password).await?;
        let mut transaction = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AuthError::internal)?;
        let updated = query_as::<_, UserRecord>(
            r"UPDATE users
               SET password_hash = $2, token_version = token_version + 1,
                   failed_login_attempts = 0, locked_until = NULL, updated_at = now()
               WHERE id = $1 AND token_version = $3
               RETURNING id, email, email_verified_at, display_name, password_hash, status, mfa_required,
                         token_version, failed_login_attempts, locked_until",
        )
        .bind(user.id)
        .bind(password_hash)
        .bind(user.token_version)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(AuthError::internal)?
        .ok_or(AuthError::Unauthorized)?;
        query("UPDATE sessions SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL")
            .bind(user.id)
            .execute(&mut *transaction)
            .await
            .map_err(AuthError::internal)?;
        let bundle = self
            .create_session(&mut transaction, &updated, context)
            .await?;
        insert_audit(
            &mut transaction,
            Some(user.id),
            Some(bundle.session_id),
            "auth.password_changed",
            "success",
            context,
        )
        .await?;
        transaction.commit().await.map_err(AuthError::internal)?;
        Ok(bundle)
    }

    /// Update the public display name of the account.
    pub(super) async fn update_profile(
        &self,
        access_token: &str,
        request: UpdateProfileRequest,
        context: RequestContext<'_>,
    ) -> Result<MeResponse, AuthError> {
        let (claims, user) = self.authenticated_user(access_token).await?;
        let display_name = validate_display_name(&request.display_name)?;
        let mut transaction = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AuthError::internal)?;
        let updated = query_as::<_, UserRecord>(
            r"UPDATE users SET display_name = $2, updated_at = now()
               WHERE id = $1
               RETURNING id, email, email_verified_at, display_name, password_hash, status, mfa_required,
                         token_version, failed_login_attempts, locked_until",
        )
        .bind(user.id)
        .bind(display_name)
        .fetch_one(&mut *transaction)
        .await
        .map_err(AuthError::internal)?;
        insert_audit(
            &mut transaction,
            Some(user.id),
            Some(claims.sid),
            "auth.profile_updated",
            "success",
            context,
        )
        .await?;
        transaction.commit().await.map_err(AuthError::internal)?;
        Ok(MeResponse {
            user: PublicUser::from(&updated),
        })
    }

    /// Permanently delete the account after a password step-up check.
    pub(super) async fn delete_account(
        &self,
        access_token: &str,
        request: PasswordConfirmationRequest,
        context: RequestContext<'_>,
    ) -> Result<(), AuthError> {
        let (claims, user) = self.authenticated_user(access_token).await?;
        if !verify_password(request.password, user.password_hash.clone()).await? {
            return Err(AuthError::InvalidCredentials);
        }
        let mut transaction = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AuthError::internal)?;
        insert_audit(
            &mut transaction,
            Some(user.id),
            Some(claims.sid),
            "auth.account_deleted",
            "success",
            context,
        )
        .await?;
        let deleted = query_scalar::<_, Uuid>("DELETE FROM users WHERE id = $1 RETURNING id")
            .bind(user.id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(AuthError::internal)?;
        if deleted.is_none() {
            return Err(AuthError::Unauthorized);
        }
        transaction.commit().await.map_err(AuthError::internal)?;
        Ok(())
    }
}
