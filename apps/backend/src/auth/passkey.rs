use serde_json::Value;
use sqlx::{Postgres, Transaction, query, query_as, query_scalar};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;
use webauthn_rs::prelude::{CredentialID, Passkey, PasskeyAuthentication, PasskeyRegistration};

use super::crypto::{generate_backup_code, normalize_backup_code, token_hash, verify_password};
use super::error::AuthError;
use super::model::{
    AccessClaims, BackupCodeLoginRequest, BackupCodesRegenerateRequest, BackupCodesResponse,
    PasskeyAuthenticationChallenge, PasskeyListResponse, PasskeyLoginFinishRequest,
    PasskeyRegistrationChallenge, PasskeyRegistrationFinishRequest, PasskeyRegistrationResponse,
    PasskeyRegistrationStartRequest, PasskeySummary, PasswordConfirmationRequest, SessionBundle,
    UserRecord,
};
use super::service::{AuthService, RequestContext, insert_audit};

const CEREMONY_MINUTES: i64 = 5;
const BACKUP_CODE_COUNT: usize = 10;

impl AuthService {
    /// Begin an authenticated passkey registration ceremony.
    ///
    /// # Errors
    ///
    /// Returns an authentication, validation, persistence or `WebAuthn` error.
    pub async fn start_passkey_registration(
        &self,
        access_token: &str,
        request: PasskeyRegistrationStartRequest,
    ) -> Result<PasskeyRegistrationChallenge, AuthError> {
        let (claims, user) = self.authenticated_user(access_token).await?;
        let nickname = validate_nickname(&request.nickname)?;
        if !verify_password(request.password, user.password_hash.clone()).await? {
            return Err(AuthError::InvalidCredentials);
        }
        let existing = query_scalar::<_, Vec<u8>>(
            "SELECT credential_id FROM webauthn_credentials WHERE user_id = $1",
        )
        .bind(user.id)
        .fetch_all(self.database.pool())
        .await
        .map_err(AuthError::internal)?;
        let exclude = (!existing.is_empty()).then(|| {
            existing
                .into_iter()
                .map(CredentialID::from)
                .collect::<Vec<_>>()
        });
        let (public_key, state) = self
            .webauthn
            .start_passkey_registration(user.id, &user.email, &user.display_name, exclude)
            .map_err(AuthError::invalid_webauthn)?;
        let ceremony_id = Uuid::new_v4();
        query(
            r"INSERT INTO webauthn_ceremonies
               (id, user_id, session_id, kind, state, nickname, expires_at)
               VALUES ($1, $2, $3, 'registration', $4, $5, $6)",
        )
        .bind(ceremony_id)
        .bind(user.id)
        .bind(claims.sid)
        .bind(serde_json::to_value(state).map_err(AuthError::internal)?)
        .bind(nickname)
        .bind(OffsetDateTime::now_utc() + Duration::minutes(CEREMONY_MINUTES))
        .execute(self.database.pool())
        .await
        .map_err(AuthError::internal)?;
        Ok(PasskeyRegistrationChallenge {
            ceremony_id,
            public_key,
        })
    }

    /// Consume a registration ceremony and persist the verified passkey.
    ///
    /// # Errors
    ///
    /// Returns an authentication, persistence or `WebAuthn` verification error.
    pub(super) async fn finish_passkey_registration(
        &self,
        access_token: &str,
        request: PasskeyRegistrationFinishRequest,
        context: RequestContext<'_>,
    ) -> Result<PasskeyRegistrationResponse, AuthError> {
        let (claims, user) = self.authenticated_user(access_token).await?;
        let ceremony = query_as::<_, RegistrationCeremony>(
            r"DELETE FROM webauthn_ceremonies
               WHERE id = $1 AND user_id = $2 AND session_id = $3
                 AND kind = 'registration' AND expires_at > now()
               RETURNING state, nickname",
        )
        .bind(request.ceremony_id)
        .bind(user.id)
        .bind(claims.sid)
        .fetch_optional(self.database.pool())
        .await
        .map_err(AuthError::internal)?
        .ok_or(AuthError::InvalidMfa)?;
        let state: PasskeyRegistration =
            serde_json::from_value(ceremony.state).map_err(AuthError::internal)?;
        let passkey = self
            .webauthn
            .finish_passkey_registration(&request.credential, &state)
            .map_err(AuthError::invalid_webauthn)?;
        let credential_id = passkey.cred_id().as_ref().to_vec();
        let credential = serde_json::to_value(&passkey).map_err(AuthError::internal)?;
        let public_key =
            serde_json::to_vec(passkey.get_public_key()).map_err(AuthError::internal)?;
        let id = Uuid::new_v4();
        let mut transaction = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AuthError::internal)?;
        query(
            r"INSERT INTO webauthn_credentials
               (id, user_id, credential_id, public_key, nickname, credential)
               VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(id)
        .bind(user.id)
        .bind(credential_id)
        .bind(public_key)
        .bind(&ceremony.nickname)
        .bind(credential)
        .execute(&mut *transaction)
        .await
        .map_err(map_credential_conflict)?;
        query("UPDATE users SET mfa_required = true, updated_at = now() WHERE id = $1")
            .bind(user.id)
            .execute(&mut *transaction)
            .await
            .map_err(AuthError::internal)?;
        let credential_count =
            query_scalar::<_, i64>("SELECT count(*) FROM webauthn_credentials WHERE user_id = $1")
                .bind(user.id)
                .fetch_one(&mut *transaction)
                .await
                .map_err(AuthError::internal)?;
        let backup_codes = if credential_count == 1 {
            create_backup_codes(&mut transaction, user.id).await?
        } else {
            Vec::new()
        };
        insert_audit(
            &mut transaction,
            Some(user.id),
            Some(claims.sid),
            "auth.passkey_registered",
            "success",
            context,
        )
        .await?;
        transaction.commit().await.map_err(AuthError::internal)?;
        Ok(PasskeyRegistrationResponse {
            credential_id: id,
            nickname: ceremony.nickname,
            backup_codes,
        })
    }

    pub(super) async fn start_passkey_authentication(
        &self,
        user: &UserRecord,
    ) -> Result<PasskeyAuthenticationChallenge, AuthError> {
        let credentials = query_scalar::<_, Value>(
            "SELECT credential FROM webauthn_credentials WHERE user_id = $1 ORDER BY created_at",
        )
        .bind(user.id)
        .fetch_all(self.database.pool())
        .await
        .map_err(AuthError::internal)?;
        let passkeys = credentials
            .into_iter()
            .map(serde_json::from_value::<Passkey>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(AuthError::internal)?;
        if passkeys.is_empty() {
            return Err(AuthError::InvalidCredentials);
        }
        let (public_key, state) = self
            .webauthn
            .start_passkey_authentication(&passkeys)
            .map_err(AuthError::invalid_webauthn)?;
        let ceremony_id = Uuid::new_v4();
        query(
            r"INSERT INTO webauthn_ceremonies
               (id, user_id, kind, state, expires_at)
               VALUES ($1, $2, 'authentication', $3, $4)",
        )
        .bind(ceremony_id)
        .bind(user.id)
        .bind(serde_json::to_value(state).map_err(AuthError::internal)?)
        .bind(OffsetDateTime::now_utc() + Duration::minutes(CEREMONY_MINUTES))
        .execute(self.database.pool())
        .await
        .map_err(AuthError::internal)?;
        Ok(PasskeyAuthenticationChallenge {
            status: "mfa_required",
            ceremony_id,
            public_key,
        })
    }

    /// Verify a passkey assertion and create an authenticated session.
    ///
    /// # Errors
    ///
    /// Returns an MFA, persistence or `WebAuthn` verification error.
    pub(super) async fn finish_passkey_login(
        &self,
        request: PasskeyLoginFinishRequest,
        context: RequestContext<'_>,
    ) -> Result<SessionBundle, AuthError> {
        let ceremony = self
            .consume_authentication_ceremony(request.ceremony_id)
            .await?;
        let state: PasskeyAuthentication =
            serde_json::from_value(ceremony.state).map_err(AuthError::internal)?;
        let result = self
            .webauthn
            .finish_passkey_authentication(&request.credential, &state)
            .map_err(AuthError::invalid_webauthn)?;
        let credential_id = result.cred_id().as_ref().to_vec();
        let mut passkey_record = query_as::<_, StoredPasskey>(
            r"SELECT id, credential FROM webauthn_credentials
               WHERE user_id = $1 AND credential_id = $2",
        )
        .bind(ceremony.user_id)
        .bind(credential_id)
        .fetch_optional(self.database.pool())
        .await
        .map_err(AuthError::internal)?
        .ok_or(AuthError::InvalidMfa)?;
        let mut passkey: Passkey =
            serde_json::from_value(passkey_record.credential).map_err(AuthError::internal)?;
        passkey
            .update_credential(&result)
            .ok_or(AuthError::InvalidMfa)?;
        passkey_record.credential = serde_json::to_value(passkey).map_err(AuthError::internal)?;
        let mut transaction = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AuthError::internal)?;
        query(
            r"UPDATE webauthn_credentials
               SET credential = $2, sign_count = $3, last_used_at = now()
               WHERE id = $1",
        )
        .bind(passkey_record.id)
        .bind(passkey_record.credential)
        .bind(i64::from(result.counter()))
        .execute(&mut *transaction)
        .await
        .map_err(AuthError::internal)?;
        let user = load_user(&mut transaction, ceremony.user_id).await?;
        let bundle = self
            .create_session(&mut transaction, &user, context)
            .await?;
        insert_audit(
            &mut transaction,
            Some(user.id),
            Some(bundle.session_id),
            "auth.passkey_login",
            "success",
            context,
        )
        .await?;
        transaction.commit().await.map_err(AuthError::internal)?;
        Ok(bundle)
    }

    /// Consume a one-time recovery code and create an authenticated session.
    ///
    /// # Errors
    ///
    /// Returns an MFA or persistence error.
    pub(super) async fn login_with_backup_code(
        &self,
        request: BackupCodeLoginRequest,
        context: RequestContext<'_>,
    ) -> Result<SessionBundle, AuthError> {
        let normalized = normalize_backup_code(&request.code).ok_or(AuthError::InvalidMfa)?;
        let mut transaction = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AuthError::internal)?;
        let ceremony = query_as::<_, AuthenticationCeremony>(
            r"SELECT user_id, state FROM webauthn_ceremonies
               WHERE id = $1 AND kind = 'authentication' AND expires_at > now()
               FOR UPDATE",
        )
        .bind(request.ceremony_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(AuthError::internal)?
        .ok_or(AuthError::InvalidMfa)?;
        let consumed_code = query_scalar::<_, Uuid>(
            r"UPDATE backup_codes SET used_at = now()
               WHERE user_id = $1 AND code_hash = $2 AND used_at IS NULL
               RETURNING id",
        )
        .bind(ceremony.user_id)
        .bind(token_hash(&normalized))
        .fetch_optional(&mut *transaction)
        .await
        .map_err(AuthError::internal)?;
        if consumed_code.is_none() {
            return Err(AuthError::InvalidMfa);
        }
        query("DELETE FROM webauthn_ceremonies WHERE id = $1")
            .bind(request.ceremony_id)
            .execute(&mut *transaction)
            .await
            .map_err(AuthError::internal)?;
        let user = load_user(&mut transaction, ceremony.user_id).await?;
        let bundle = self
            .create_session(&mut transaction, &user, context)
            .await?;
        insert_audit(
            &mut transaction,
            Some(user.id),
            Some(bundle.session_id),
            "auth.backup_code_login",
            "success",
            context,
        )
        .await?;
        transaction.commit().await.map_err(AuthError::internal)?;
        Ok(bundle)
    }

    /// List registered passkeys and the remaining recovery-code count.
    ///
    /// # Errors
    ///
    /// Returns an authentication or persistence error.
    pub async fn list_passkeys(
        &self,
        access_token: &str,
    ) -> Result<PasskeyListResponse, AuthError> {
        let (_, user) = self.authenticated_user(access_token).await?;
        let passkeys = query_as::<_, PasskeySummary>(
            r"SELECT id, nickname, created_at, last_used_at
               FROM webauthn_credentials WHERE user_id = $1 ORDER BY created_at",
        )
        .bind(user.id)
        .fetch_all(self.database.pool())
        .await
        .map_err(AuthError::internal)?;
        let unused_backup_codes = query_scalar::<_, i64>(
            "SELECT count(*) FROM backup_codes WHERE user_id = $1 AND used_at IS NULL",
        )
        .bind(user.id)
        .fetch_one(self.database.pool())
        .await
        .map_err(AuthError::internal)?;
        Ok(PasskeyListResponse {
            passkeys,
            unused_backup_codes,
        })
    }

    /// Replace all recovery codes and return the new codes exactly once.
    ///
    /// # Errors
    ///
    /// Returns an authentication, validation or persistence error.
    pub(super) async fn regenerate_backup_codes(
        &self,
        access_token: &str,
        request: BackupCodesRegenerateRequest,
        context: RequestContext<'_>,
    ) -> Result<BackupCodesResponse, AuthError> {
        let (claims, user) = self.authenticated_user(access_token).await?;
        if !verify_password(request.password, user.password_hash.clone()).await? {
            return Err(AuthError::InvalidCredentials);
        }
        let credential_count =
            query_scalar::<_, i64>("SELECT count(*) FROM webauthn_credentials WHERE user_id = $1")
                .bind(user.id)
                .fetch_one(self.database.pool())
                .await
                .map_err(AuthError::internal)?;
        if credential_count == 0 {
            return Err(AuthError::Validation(
                "Register a passkey before generating recovery codes.",
            ));
        }
        let mut transaction = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AuthError::internal)?;
        query("DELETE FROM backup_codes WHERE user_id = $1")
            .bind(user.id)
            .execute(&mut *transaction)
            .await
            .map_err(AuthError::internal)?;
        let backup_codes = create_backup_codes(&mut transaction, user.id).await?;
        insert_audit(
            &mut transaction,
            Some(user.id),
            Some(claims.sid),
            "auth.backup_codes_regenerated",
            "success",
            context,
        )
        .await?;
        transaction.commit().await.map_err(AuthError::internal)?;
        Ok(BackupCodesResponse { backup_codes })
    }

    /// Remove one passkey after a password step-up check.
    pub(super) async fn remove_passkey(
        &self,
        access_token: &str,
        credential_id: Uuid,
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
        let removed = query_scalar::<_, Uuid>(
            "DELETE FROM webauthn_credentials WHERE id = $1 AND user_id = $2 RETURNING id",
        )
        .bind(credential_id)
        .bind(user.id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(AuthError::internal)?;
        if removed.is_none() {
            return Err(AuthError::NotFound);
        }
        let remaining =
            query_scalar::<_, i64>("SELECT count(*) FROM webauthn_credentials WHERE user_id = $1")
                .bind(user.id)
                .fetch_one(&mut *transaction)
                .await
                .map_err(AuthError::internal)?;
        if remaining == 0 {
            query("DELETE FROM backup_codes WHERE user_id = $1")
                .bind(user.id)
                .execute(&mut *transaction)
                .await
                .map_err(AuthError::internal)?;
            query("DELETE FROM webauthn_ceremonies WHERE user_id = $1")
                .bind(user.id)
                .execute(&mut *transaction)
                .await
                .map_err(AuthError::internal)?;
            query("UPDATE users SET mfa_required = false, updated_at = now() WHERE id = $1")
                .bind(user.id)
                .execute(&mut *transaction)
                .await
                .map_err(AuthError::internal)?;
        }
        insert_audit(
            &mut transaction,
            Some(user.id),
            Some(claims.sid),
            "auth.passkey_removed",
            "success",
            context,
        )
        .await?;
        transaction.commit().await.map_err(AuthError::internal)?;
        Ok(())
    }

    /// Disable MFA and remove every passkey and unused recovery code.
    pub(super) async fn disable_mfa(
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
        query("DELETE FROM webauthn_ceremonies WHERE user_id = $1")
            .bind(user.id)
            .execute(&mut *transaction)
            .await
            .map_err(AuthError::internal)?;
        query("DELETE FROM webauthn_credentials WHERE user_id = $1")
            .bind(user.id)
            .execute(&mut *transaction)
            .await
            .map_err(AuthError::internal)?;
        query("DELETE FROM backup_codes WHERE user_id = $1")
            .bind(user.id)
            .execute(&mut *transaction)
            .await
            .map_err(AuthError::internal)?;
        query("UPDATE users SET mfa_required = false, updated_at = now() WHERE id = $1")
            .bind(user.id)
            .execute(&mut *transaction)
            .await
            .map_err(AuthError::internal)?;
        insert_audit(
            &mut transaction,
            Some(user.id),
            Some(claims.sid),
            "auth.mfa_disabled",
            "success",
            context,
        )
        .await?;
        transaction.commit().await.map_err(AuthError::internal)?;
        Ok(())
    }

    pub(super) async fn authenticated_user(
        &self,
        access_token: &str,
    ) -> Result<(AccessClaims, UserRecord), AuthError> {
        let claims = self.decode_access_token(access_token)?;
        let user = query_as::<_, UserRecord>(
            r"SELECT u.id, u.email, u.email_verified_at, u.display_name, u.password_hash, u.status,
                      u.mfa_required, u.token_version, u.failed_login_attempts, u.locked_until
               FROM users u JOIN sessions s ON s.user_id = u.id
               WHERE u.id = $1 AND s.id = $2 AND s.revoked_at IS NULL
                 AND s.expires_at > now() AND s.idle_expires_at > now()",
        )
        .bind(claims.sub)
        .bind(claims.sid)
        .fetch_optional(self.database.pool())
        .await
        .map_err(AuthError::internal)?
        .ok_or(AuthError::Unauthorized)?;
        if user.status != "active" || user.token_version != claims.ver {
            return Err(AuthError::Unauthorized);
        }
        Ok((claims, user))
    }

    async fn consume_authentication_ceremony(
        &self,
        ceremony_id: Uuid,
    ) -> Result<AuthenticationCeremony, AuthError> {
        query_as::<_, AuthenticationCeremony>(
            r"DELETE FROM webauthn_ceremonies
               WHERE id = $1 AND kind = 'authentication' AND expires_at > now()
               RETURNING user_id, state",
        )
        .bind(ceremony_id)
        .fetch_optional(self.database.pool())
        .await
        .map_err(AuthError::internal)?
        .ok_or(AuthError::InvalidMfa)
    }
}

#[derive(sqlx::FromRow)]
struct RegistrationCeremony {
    state: Value,
    nickname: String,
}

#[derive(sqlx::FromRow)]
struct AuthenticationCeremony {
    user_id: Uuid,
    state: Value,
}

#[derive(sqlx::FromRow)]
struct StoredPasskey {
    id: Uuid,
    credential: Value,
}

async fn create_backup_codes(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
) -> Result<Vec<String>, AuthError> {
    let codes = (0..BACKUP_CODE_COUNT)
        .map(|_| generate_backup_code())
        .collect::<Vec<_>>();
    for code in &codes {
        let normalized = normalize_backup_code(code).ok_or(AuthError::InvalidMfa)?;
        query("INSERT INTO backup_codes (id, user_id, code_hash) VALUES ($1, $2, $3)")
            .bind(Uuid::new_v4())
            .bind(user_id)
            .bind(token_hash(&normalized))
            .execute(&mut **transaction)
            .await
            .map_err(AuthError::internal)?;
    }
    Ok(codes)
}

async fn load_user(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
) -> Result<UserRecord, AuthError> {
    query_as::<_, UserRecord>(
        r"SELECT id, email, email_verified_at, display_name, password_hash, status, mfa_required,
                  token_version, failed_login_attempts, locked_until
           FROM users WHERE id = $1 AND status = 'active'",
    )
    .bind(user_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(AuthError::internal)?
    .ok_or(AuthError::InvalidMfa)
}

fn validate_nickname(value: &str) -> Result<&str, AuthError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 80 || value.chars().any(char::is_control) {
        return Err(AuthError::Validation(
            "Passkey nickname must contain between 1 and 80 characters.",
        ));
    }
    Ok(value)
}

fn map_credential_conflict(error: sqlx::Error) -> AuthError {
    if error
        .as_database_error()
        .is_some_and(|database| database.code().as_deref() == Some("23505"))
    {
        AuthError::Validation("This passkey is already registered.")
    } else {
        AuthError::internal(error)
    }
}
