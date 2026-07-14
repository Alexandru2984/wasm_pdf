use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use time::Duration;

use crate::AppState;

use super::error::AuthError;
use super::model::{AuthResponse, LoginRequest, MeResponse, RegisterRequest, SessionBundle};
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
}

async fn register(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<RegisterRequest>,
) -> Result<impl IntoResponse, AuthError> {
    let auth = auth_service(&state)?;
    let bundle = auth.register(request, user_agent(&headers)).await?;
    let (jar, response) = authenticated_response(jar, auth, bundle);
    Ok((StatusCode::CREATED, jar, Json(response)))
}

async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<LoginRequest>,
) -> Result<impl IntoResponse, AuthError> {
    let auth = auth_service(&state)?;
    let bundle = auth.login(request, user_agent(&headers)).await?;
    let (jar, response) = authenticated_response(jar, auth, bundle);
    Ok((jar, Json(response)))
}

async fn refresh(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AuthError> {
    let auth = auth_service(&state)?;
    let session = session_token(&jar)?;
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
    auth.logout(
        session_token(&jar)?,
        csrf_token(&headers)?,
        user_agent(&headers),
    )
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
