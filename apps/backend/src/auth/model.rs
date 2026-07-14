use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

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
