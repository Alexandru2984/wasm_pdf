use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

#[derive(Debug)]
pub enum AuthError {
    Validation(&'static str),
    InvalidCredentials,
    Unauthorized,
    InvalidCsrf,
    EmailTaken,
    RateLimited { retry_after: u64 },
    Unavailable,
    Internal(String),
}

impl AuthError {
    pub fn internal(error: impl std::fmt::Display) -> Self {
        Self::Internal(error.to_string())
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        if let Self::RateLimited { retry_after } = self {
            let mut response = (
                StatusCode::TOO_MANY_REQUESTS,
                Json(ErrorEnvelope {
                    error: ErrorBody {
                        code: "rate_limited",
                        message: "Too many requests. Try again later.",
                    },
                }),
            )
                .into_response();
            if let Ok(value) = HeaderValue::from_str(&retry_after.to_string()) {
                response.headers_mut().insert(header::RETRY_AFTER, value);
            }
            return response;
        }
        let (status, code, message) = match &self {
            Self::Validation(message) => (StatusCode::BAD_REQUEST, "invalid_request", *message),
            Self::InvalidCredentials => (
                StatusCode::UNAUTHORIZED,
                "invalid_credentials",
                "Email or password is invalid.",
            ),
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Authentication is required.",
            ),
            Self::InvalidCsrf => (
                StatusCode::FORBIDDEN,
                "invalid_csrf",
                "The CSRF token is missing or invalid.",
            ),
            Self::EmailTaken => (
                StatusCode::CONFLICT,
                "email_taken",
                "An account already exists for this email.",
            ),
            Self::RateLimited { .. } => unreachable!("handled before the error mapping"),
            Self::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "auth_unavailable",
                "Authentication is temporarily unavailable.",
            ),
            Self::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "The request could not be completed.",
            ),
        };
        if let Self::Internal(error) = &self {
            tracing::error!(%error, "auth_internal_error");
        }
        (
            status,
            Json(ErrorEnvelope {
                error: ErrorBody { code, message },
            }),
        )
            .into_response()
    }
}

#[derive(Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limit_response_includes_retry_after() {
        let response = AuthError::RateLimited { retry_after: 42 }.into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            response.headers().get(header::RETRY_AFTER),
            Some(&HeaderValue::from_static("42"))
        );
    }
}
