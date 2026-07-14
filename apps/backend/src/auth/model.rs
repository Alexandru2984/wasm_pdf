use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;
use webauthn_rs::prelude::{
    CreationChallengeResponse, PublicKeyCredential, RegisterPublicKeyCredential,
    RequestChallengeResponse,
};

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub display_name: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct PasskeyRegistrationStartRequest {
    pub nickname: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct PasskeyRegistrationFinishRequest {
    pub ceremony_id: Uuid,
    pub credential: RegisterPublicKeyCredential,
}

#[derive(Debug, Deserialize)]
pub struct PasskeyLoginFinishRequest {
    pub ceremony_id: Uuid,
    pub credential: PublicKeyCredential,
}

#[derive(Debug, Deserialize)]
pub struct BackupCodeLoginRequest {
    pub ceremony_id: Uuid,
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct BackupCodesRegenerateRequest {
    pub password: String,
}

#[derive(Clone, Debug, FromRow)]
pub struct UserRecord {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub password_hash: String,
    pub status: String,
    pub mfa_required: bool,
    pub token_version: i32,
    pub failed_login_attempts: i32,
    pub locked_until: Option<OffsetDateTime>,
}

#[derive(Clone, Debug, FromRow)]
pub struct SessionUserRecord {
    pub session_id: Uuid,
    pub user_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub status: String,
    pub mfa_required: bool,
    pub token_version: i32,
}

#[derive(Clone, Debug, Serialize)]
pub struct PublicUser {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub mfa_required: bool,
}

impl From<&UserRecord> for PublicUser {
    fn from(user: &UserRecord) -> Self {
        Self {
            id: user.id,
            email: user.email.clone(),
            display_name: user.display_name.clone(),
            mfa_required: user.mfa_required,
        }
    }
}

impl From<&SessionUserRecord> for PublicUser {
    fn from(user: &SessionUserRecord) -> Self {
        Self {
            id: user.user_id,
            email: user.email.clone(),
            display_name: user.display_name.clone(),
            mfa_required: user.mfa_required,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: i64,
    pub csrf_token: String,
    pub user: PublicUser,
}

pub enum LoginOutcome {
    Authenticated(SessionBundle),
    PasskeyRequired(PasskeyAuthenticationChallenge),
}

#[derive(Debug, Serialize)]
pub struct PasskeyAuthenticationChallenge {
    pub status: &'static str,
    pub ceremony_id: Uuid,
    pub public_key: RequestChallengeResponse,
}

#[derive(Debug, Serialize)]
pub struct PasskeyRegistrationChallenge {
    pub ceremony_id: Uuid,
    pub public_key: CreationChallengeResponse,
}

#[derive(Debug, Serialize)]
pub struct PasskeyRegistrationResponse {
    pub credential_id: Uuid,
    pub nickname: String,
    pub backup_codes: Vec<String>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct PasskeySummary {
    pub id: Uuid,
    pub nickname: String,
    pub created_at: OffsetDateTime,
    pub last_used_at: Option<OffsetDateTime>,
}

#[derive(Debug, Serialize)]
pub struct PasskeyListResponse {
    pub passkeys: Vec<PasskeySummary>,
    pub unused_backup_codes: i64,
}

#[derive(Debug, Serialize)]
pub struct BackupCodesResponse {
    pub backup_codes: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct MeResponse {
    pub user: PublicUser,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AccessClaims {
    pub sub: Uuid,
    pub sid: Uuid,
    pub ver: i32,
    pub iss: String,
    pub aud: String,
    pub iat: i64,
    pub exp: i64,
    pub jti: Uuid,
}

pub struct SessionBundle {
    pub response: AuthResponse,
    pub session_token: String,
    pub session_id: Uuid,
}
