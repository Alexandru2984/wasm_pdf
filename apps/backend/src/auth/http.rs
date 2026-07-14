use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use time::Duration;

use crate::AppState;

use super::error::AuthError;
use super::model::{
    AccountTokenRequest, AuthResponse, BackupCodeLoginRequest, BackupCodesRegenerateRequest,
    BackupCodesResponse, ChangePasswordRequest, LoginOutcome, LoginRequest, MeResponse,
    PasskeyListResponse, PasskeyLoginFinishRequest, PasskeyRegistrationChallenge,
    PasskeyRegistrationFinishRequest, PasskeyRegistrationResponse, PasskeyRegistrationStartRequest,
    PasswordConfirmationRequest, PasswordResetConfirmRequest, PasswordResetRequest,
    RegisterRequest, SessionBundle, SessionListResponse, UpdateProfileRequest,
};
use super::rate_limit::RateLimitCategory;
use super::service::{AuthService, RequestContext};

const SESSION_COOKIE: &str = "pdf_editor_session";
const CSRF_HEADER: &str = "x-csrf-token";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/auth/register", post(register))
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/refresh", post(refresh))
        .route("/api/v1/auth/logout", post(logout))
        .route("/api/v1/auth/me", get(me))
        .route("/api/v1/auth/profile", put(update_profile))
        .route("/api/v1/auth/password", put(change_password))
        .route("/api/v1/auth/account", delete(delete_account))
        .route("/api/v1/auth/sessions", get(list_sessions))
        .route(
            "/api/v1/auth/sessions/others/revoke",
            post(revoke_other_sessions),
        )
        .route("/api/v1/auth/sessions/{session_id}", delete(revoke_session))
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
        .route(
            "/api/v1/auth/passkeys/{credential_id}",
            delete(remove_passkey),
        )
        .route("/api/v1/auth/mfa/disable", post(disable_mfa))
        .route(
            "/api/v1/auth/email/verification/request",
            post(request_email_verification),
        )
        .route(
            "/api/v1/auth/email/verification/confirm",
            post(confirm_email_verification),
        )
        .route(
            "/api/v1/auth/password/reset/request",
            post(request_password_reset),
        )
        .route(
            "/api/v1/auth/password/reset/confirm",
            post(confirm_password_reset),
        )
}

async fn request_email_verification(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, AuthError> {
    let auth = auth_service(&state)?;
    let access_token = bearer_token(&headers)?;
    auth.enforce_rate_limit(RateLimitCategory::AccountMutation, access_token)
        .await?;
    auth.request_email_verification(access_token, request_context(&headers))
        .await?;
    Ok(StatusCode::ACCEPTED)
}

async fn confirm_email_verification(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<AccountTokenRequest>,
) -> Result<StatusCode, AuthError> {
    let auth = auth_service(&state)?;
    auth.enforce_rate_limit(
        RateLimitCategory::RecoveryConfirm,
        client_ip(&headers).unwrap_or("unknown"),
    )
    .await?;
    auth.enforce_rate_limit(RateLimitCategory::RecoveryConfirm, &request.token)
        .await?;
    auth.confirm_email_verification(&request.token, request_context(&headers))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn request_password_reset(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<PasswordResetRequest>,
) -> Result<StatusCode, AuthError> {
    let auth = auth_service(&state)?;
    auth.enforce_rate_limit(
        RateLimitCategory::RecoveryIp,
        client_ip(&headers).unwrap_or("unknown"),
    )
    .await?;
    auth.enforce_rate_limit(
        RateLimitCategory::RecoveryIdentity,
        &request.email.trim().to_lowercase(),
    )
    .await?;
    auth.request_password_reset(request, request_context(&headers))
        .await?;
    Ok(StatusCode::ACCEPTED)
}

async fn confirm_password_reset(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<PasswordResetConfirmRequest>,
) -> Result<impl IntoResponse, AuthError> {
    let auth = auth_service(&state)?;
    auth.enforce_rate_limit(
        RateLimitCategory::RecoveryConfirm,
        client_ip(&headers).unwrap_or("unknown"),
    )
    .await?;
    auth.enforce_rate_limit(RateLimitCategory::RecoveryConfirm, &request.token)
        .await?;
    auth.confirm_password_reset(request, request_context(&headers))
        .await?;
    Ok((remove_session_cookie(jar, auth), StatusCode::NO_CONTENT))
}

async fn register(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<RegisterRequest>,
) -> Result<impl IntoResponse, AuthError> {
    let auth = auth_service(&state)?;
    auth.enforce_rate_limit(
        RateLimitCategory::RegisterIp,
        client_ip(&headers).unwrap_or("unknown"),
    )
    .await?;
    auth.enforce_rate_limit(
        RateLimitCategory::RegisterIdentity,
        &request.email.trim().to_lowercase(),
    )
    .await?;
    let bundle = auth.register(request, request_context(&headers)).await?;
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
    auth.enforce_rate_limit(
        RateLimitCategory::LoginIp,
        client_ip(&headers).unwrap_or("unknown"),
    )
    .await?;
    auth.enforce_rate_limit(
        RateLimitCategory::LoginIdentity,
        &request.email.trim().to_lowercase(),
    )
    .await?;
    match auth.login(request, request_context(&headers)).await? {
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
    let bundle = auth
        .refresh(session, csrf, request_context(&headers))
        .await?;
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
    auth.logout(session, csrf_token(&headers)?, request_context(&headers))
        .await?;
    Ok((remove_session_cookie(jar, auth), StatusCode::NO_CONTENT))
}

async fn me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<MeResponse>, AuthError> {
    let auth = auth_service(&state)?;
    Ok(Json(auth.me(bearer_token(&headers)?).await?))
}

async fn update_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<UpdateProfileRequest>,
) -> Result<Json<MeResponse>, AuthError> {
    let auth = auth_service(&state)?;
    let access_token = bearer_token(&headers)?;
    auth.enforce_rate_limit(RateLimitCategory::AccountMutation, access_token)
        .await?;
    Ok(Json(
        auth.update_profile(access_token, request, request_context(&headers))
            .await?,
    ))
}

async fn change_password(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<ChangePasswordRequest>,
) -> Result<impl IntoResponse, AuthError> {
    let auth = auth_service(&state)?;
    let access_token = bearer_token(&headers)?;
    auth.enforce_rate_limit(RateLimitCategory::MfaCeremony, access_token)
        .await?;
    let bundle = auth
        .change_password(access_token, request, request_context(&headers))
        .await?;
    let (jar, response) = authenticated_response(jar, auth, bundle);
    Ok((jar, Json(response)))
}

async fn delete_account(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<PasswordConfirmationRequest>,
) -> Result<impl IntoResponse, AuthError> {
    let auth = auth_service(&state)?;
    let access_token = bearer_token(&headers)?;
    auth.enforce_rate_limit(RateLimitCategory::MfaCeremony, access_token)
        .await?;
    auth.delete_account(access_token, request, request_context(&headers))
        .await?;
    Ok((remove_session_cookie(jar, auth), StatusCode::NO_CONTENT))
}

async fn list_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<SessionListResponse>, AuthError> {
    let auth = auth_service(&state)?;
    Ok(Json(auth.list_sessions(bearer_token(&headers)?).await?))
}

async fn revoke_session(
    State(state): State<AppState>,
    Path(session_id): Path<uuid::Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, AuthError> {
    let auth = auth_service(&state)?;
    let access_token = bearer_token(&headers)?;
    auth.enforce_rate_limit(RateLimitCategory::LogoutSession, access_token)
        .await?;
    auth.revoke_session(access_token, session_id, request_context(&headers))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn revoke_other_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, AuthError> {
    let auth = auth_service(&state)?;
    let access_token = bearer_token(&headers)?;
    auth.enforce_rate_limit(RateLimitCategory::LogoutSession, access_token)
        .await?;
    auth.revoke_other_sessions(access_token, request_context(&headers))
        .await?;
    Ok(StatusCode::NO_CONTENT)
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
        auth.finish_passkey_registration(
            bearer_token(&headers)?,
            request,
            request_context(&headers),
        )
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
        .finish_passkey_login(request, request_context(&headers))
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
        .login_with_backup_code(request, request_context(&headers))
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
        auth.regenerate_backup_codes(access_token, request, request_context(&headers))
            .await?,
    ))
}

async fn remove_passkey(
    State(state): State<AppState>,
    Path(credential_id): Path<uuid::Uuid>,
    headers: HeaderMap,
    Json(request): Json<PasswordConfirmationRequest>,
) -> Result<StatusCode, AuthError> {
    let auth = auth_service(&state)?;
    let access_token = bearer_token(&headers)?;
    auth.enforce_rate_limit(RateLimitCategory::MfaCeremony, access_token)
        .await?;
    auth.remove_passkey(
        access_token,
        credential_id,
        request,
        request_context(&headers),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn disable_mfa(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<PasswordConfirmationRequest>,
) -> Result<StatusCode, AuthError> {
    let auth = auth_service(&state)?;
    let access_token = bearer_token(&headers)?;
    auth.enforce_rate_limit(RateLimitCategory::MfaCeremony, access_token)
        .await?;
    auth.disable_mfa(access_token, request, request_context(&headers))
        .await?;
    Ok(StatusCode::NO_CONTENT)
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

fn remove_session_cookie(jar: CookieJar, auth: &AuthService) -> CookieJar {
    let removal = Cookie::build(SESSION_COOKIE)
        .path("/api/v1/auth")
        .http_only(true)
        .same_site(SameSite::Strict)
        .secure(auth.cookie_secure())
        .build();
    jar.remove(removal)
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

fn client_ip(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| value.parse::<std::net::IpAddr>().is_ok())
}

fn request_context(headers: &HeaderMap) -> RequestContext<'_> {
    RequestContext {
        ip_address: client_ip(headers),
        user_agent: user_agent(headers),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Method, Request};
    use tower::ServiceExt;

    #[test]
    fn accepts_only_a_valid_first_forwarded_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.7".parse().expect("header"));
        assert_eq!(client_ip(&headers), Some("203.0.113.7"));

        headers.insert(
            "x-forwarded-for",
            "2001:db8::4, 10.0.0.2".parse().expect("header"),
        );
        assert_eq!(client_ip(&headers), Some("2001:db8::4"));

        headers.insert("x-forwarded-for", "forged".parse().expect("header"));
        assert_eq!(client_ip(&headers), None);
    }

    #[tokio::test]
    async fn account_lifecycle_routes_are_registered() {
        let app = router().with_state(AppState::new());
        let routes = [
            (Method::GET, "/api/v1/auth/sessions"),
            (Method::PUT, "/api/v1/auth/profile"),
            (Method::PUT, "/api/v1/auth/password"),
            (Method::DELETE, "/api/v1/auth/account"),
            (
                Method::DELETE,
                "/api/v1/auth/sessions/11111111-1111-4111-8111-111111111111",
            ),
            (
                Method::DELETE,
                "/api/v1/auth/passkeys/11111111-1111-4111-8111-111111111111",
            ),
            (Method::POST, "/api/v1/auth/mfa/disable"),
            (Method::POST, "/api/v1/auth/email/verification/request"),
            (Method::POST, "/api/v1/auth/email/verification/confirm"),
            (Method::POST, "/api/v1/auth/password/reset/request"),
            (Method::POST, "/api/v1/auth/password/reset/confirm"),
        ];

        for (method, uri) in routes {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_ne!(response.status(), StatusCode::NOT_FOUND, "route {uri}");
        }
    }
}
