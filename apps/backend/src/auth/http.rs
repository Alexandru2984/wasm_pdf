use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use time::Duration;

use crate::AppState;

use super::error::AuthError;
use super::model::{
    AuthResponse, BackupCodeLoginRequest, BackupCodesRegenerateRequest, BackupCodesResponse,
    LoginOutcome, LoginRequest, MeResponse, PasskeyListResponse, PasskeyLoginFinishRequest,
    PasskeyRegistrationChallenge, PasskeyRegistrationFinishRequest, PasskeyRegistrationResponse,
    PasskeyRegistrationStartRequest, RegisterRequest, SessionBundle,
};
use super::rate_limit::RateLimitCategory;
use super::service::AuthService;

const SESSION_COOKIE: &str = "pdf_editor_session";
const CSRF_HEADER: &str = "x-csrf-token";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/auth/register", post(register))
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/refresh", post(refresh))
        .route("/api/v1/auth/logout", post(logout))
        .route("/api/v1/auth/me", get(me))
        .route(
            "/api/v1/auth/passkeys/register/start",
            post(start_passkey_registration),
        )
        .route(
            "/api/v1/auth/passkeys/register/finish",
            post(finish_passkey_registration),
        )
        .route(
            "/api/v1/auth/passkeys/login/finish",
            post(finish_passkey_login),
        )
        .route("/api/v1/auth/mfa/backup-code", post(login_with_backup_code))
        .route(
            "/api/v1/auth/mfa/backup-codes/regenerate",
            post(regenerate_backup_codes),
        )
        .route("/api/v1/auth/passkeys", get(list_passkeys))
}

async fn register(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<RegisterRequest>,
) -> Result<impl IntoResponse, AuthError> {
    let auth = auth_service(&state)?;
    auth.enforce_rate_limit(RateLimitCategory::RegisterIp, client_ip(&headers))
        .await?;
    auth.enforce_rate_limit(
        RateLimitCategory::RegisterIdentity,
        &request.email.trim().to_lowercase(),
    )
    .await?;
    let bundle = auth.register(request, user_agent(&headers)).await?;
    let (jar, response) = authenticated_response(jar, auth, bundle);
    Ok((StatusCode::CREATED, jar, Json(response)))
}

async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<LoginRequest>,
) -> Result<Response, AuthError> {
    let auth = auth_service(&state)?;
    auth.enforce_rate_limit(RateLimitCategory::LoginIp, client_ip(&headers))
        .await?;
    auth.enforce_rate_limit(
        RateLimitCategory::LoginIdentity,
        &request.email.trim().to_lowercase(),
    )
    .await?;
    match auth.login(request, user_agent(&headers)).await? {
        LoginOutcome::Authenticated(bundle) => {
            let (jar, response) = authenticated_response(jar, auth, bundle);
            Ok((jar, Json(response)).into_response())
        }
        LoginOutcome::PasskeyRequired(challenge) => {
            Ok((StatusCode::ACCEPTED, Json(challenge)).into_response())
        }
    }
}

async fn refresh(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AuthError> {
    let auth = auth_service(&state)?;
    let session = session_token(&jar)?;
    auth.enforce_rate_limit(RateLimitCategory::RefreshSession, session)
        .await?;
    let csrf = csrf_token(&headers)?;
    let bundle = auth.refresh(session, csrf, user_agent(&headers)).await?;
    let (jar, response) = authenticated_response(jar, auth, bundle);
    Ok((jar, Json(response)))
}

async fn logout(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AuthError> {
    let auth = auth_service(&state)?;
    let session = session_token(&jar)?;
    auth.enforce_rate_limit(RateLimitCategory::LogoutSession, session)
        .await?;
    auth.logout(session, csrf_token(&headers)?, user_agent(&headers))
        .await?;
    let removal = Cookie::build(SESSION_COOKIE)
        .path("/api/v1/auth")
        .http_only(true)
        .same_site(SameSite::Strict)
        .secure(auth.cookie_secure())
        .build();
    Ok((jar.remove(removal), StatusCode::NO_CONTENT))
}

async fn me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<MeResponse>, AuthError> {
    let auth = auth_service(&state)?;
    Ok(Json(auth.me(bearer_token(&headers)?).await?))
}

async fn start_passkey_registration(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<PasskeyRegistrationStartRequest>,
) -> Result<Json<PasskeyRegistrationChallenge>, AuthError> {
    let auth = auth_service(&state)?;
    let access_token = bearer_token(&headers)?;
    auth.enforce_rate_limit(RateLimitCategory::MfaCeremony, access_token)
        .await?;
    Ok(Json(
        auth.start_passkey_registration(access_token, request)
            .await?,
    ))
}

async fn finish_passkey_registration(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<PasskeyRegistrationFinishRequest>,
) -> Result<Json<PasskeyRegistrationResponse>, AuthError> {
    let auth = auth_service(&state)?;
    auth.enforce_rate_limit(
        RateLimitCategory::MfaCeremony,
        &request.ceremony_id.to_string(),
    )
    .await?;
    Ok(Json(
        auth.finish_passkey_registration(bearer_token(&headers)?, request, user_agent(&headers))
            .await?,
    ))
}

async fn finish_passkey_login(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<PasskeyLoginFinishRequest>,
) -> Result<impl IntoResponse, AuthError> {
    let auth = auth_service(&state)?;
    auth.enforce_rate_limit(
        RateLimitCategory::MfaCeremony,
        &request.ceremony_id.to_string(),
    )
    .await?;
    let bundle = auth
        .finish_passkey_login(request, user_agent(&headers))
        .await?;
    let (jar, response) = authenticated_response(jar, auth, bundle);
    Ok((jar, Json(response)))
}

async fn login_with_backup_code(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<BackupCodeLoginRequest>,
) -> Result<impl IntoResponse, AuthError> {
    let auth = auth_service(&state)?;
    auth.enforce_rate_limit(
        RateLimitCategory::MfaCeremony,
        &request.ceremony_id.to_string(),
    )
    .await?;
    let bundle = auth
        .login_with_backup_code(request, user_agent(&headers))
        .await?;
    let (jar, response) = authenticated_response(jar, auth, bundle);
    Ok((jar, Json(response)))
}

async fn list_passkeys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<PasskeyListResponse>, AuthError> {
    let auth = auth_service(&state)?;
    Ok(Json(auth.list_passkeys(bearer_token(&headers)?).await?))
}

async fn regenerate_backup_codes(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<BackupCodesRegenerateRequest>,
) -> Result<Json<BackupCodesResponse>, AuthError> {
    let auth = auth_service(&state)?;
    let access_token = bearer_token(&headers)?;
    auth.enforce_rate_limit(RateLimitCategory::MfaCeremony, access_token)
        .await?;
    Ok(Json(
        auth.regenerate_backup_codes(access_token, request, user_agent(&headers))
            .await?,
    ))
}

fn auth_service(state: &AppState) -> Result<&AuthService, AuthError> {
    state.auth.as_ref().ok_or(AuthError::Unavailable)
}

fn authenticated_response(
    jar: CookieJar,
    auth: &AuthService,
    bundle: SessionBundle,
) -> (CookieJar, AuthResponse) {
    let cookie = Cookie::build((SESSION_COOKIE, bundle.session_token))
        .path("/api/v1/auth")
        .http_only(true)
        .same_site(SameSite::Strict)
        .secure(auth.cookie_secure())
        .max_age(Duration::days(auth.session_days()))
        .build();
    (jar.add(cookie), bundle.response)
}

fn session_token(jar: &CookieJar) -> Result<&str, AuthError> {
    jar.get(SESSION_COOKIE)
        .map(Cookie::value)
        .ok_or(AuthError::Unauthorized)
}

fn csrf_token(headers: &HeaderMap) -> Result<&str, AuthError> {
    headers
        .get(CSRF_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .ok_or(AuthError::InvalidCsrf)
}

fn bearer_token(headers: &HeaderMap) -> Result<&str, AuthError> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| !value.is_empty() && value.len() <= 4_096)
        .ok_or(AuthError::Unauthorized)
}

fn user_agent(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
}

fn client_ip(headers: &HeaderMap) -> &str {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| value.parse::<std::net::IpAddr>().is_ok())
        .unwrap_or("unknown")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_a_valid_first_forwarded_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.7".parse().expect("header"));
        assert_eq!(client_ip(&headers), "203.0.113.7");

        headers.insert(
            "x-forwarded-for",
            "2001:db8::4, 10.0.0.2".parse().expect("header"),
        );
        assert_eq!(client_ip(&headers), "2001:db8::4");

        headers.insert("x-forwarded-for", "forged".parse().expect("header"));
        assert_eq!(client_ip(&headers), "unknown");
    }
}
