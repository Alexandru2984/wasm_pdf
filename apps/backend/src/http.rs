use std::time::Instant;

use axum::extract::{MatchedPath, Request, State};
use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tower_http::catch_panic::CatchPanicLayer;
use tracing::Instrument;
use uuid::Uuid;

use crate::AppState;

const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health/live", get(liveness))
        .route("/health/ready", get(readiness))
        .route("/metrics", get(metrics))
        .route(
            "/api/v1/telemetry/pdf-operations",
            post(report_pdf_operation),
        )
        .fallback(not_found)
        .layer(CatchPanicLayer::new())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            observe_request,
        ))
        .with_state(state)
}

async fn liveness() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        uptime_seconds: None,
    })
}

async fn readiness(State(state): State<AppState>) -> impl IntoResponse {
    let database_ready = match &state.database {
        Some(database) => database.is_ready().await,
        None => true,
    };
    let status = if database_ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(HealthResponse {
            status: if database_ready { "ready" } else { "not_ready" },
            version: env!("CARGO_PKG_VERSION"),
            uptime_seconds: Some(state.started_at.elapsed().as_secs()),
        }),
    )
}

async fn metrics(State(state): State<AppState>) -> Response {
    match state.metrics.encode() {
        Ok(body) => (
            [(
                header::CONTENT_TYPE,
                "application/openmetrics-text; version=1.0.0; charset=utf-8",
            )],
            body,
        )
            .into_response(),
        Err(error) => {
            tracing::error!(%error, "metrics_encoding_failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn report_pdf_operation(
    State(state): State<AppState>,
    Json(report): Json<PdfOperationReport>,
) -> StatusCode {
    if !report.duration_ms.is_finite() || !(0.0..=86_400_000.0).contains(&report.duration_ms) {
        return StatusCode::BAD_REQUEST;
    }
    state.metrics.observe_pdf_operation(
        report.operation.as_label(),
        report.status.as_label(),
        report.duration_ms / 1_000.0,
    );
    tracing::info!(
        operation = report.operation.as_label(),
        status = report.status.as_label(),
        duration_ms = report.duration_ms,
        "pdf_operation_reported"
    );
    StatusCode::ACCEPTED
}

async fn not_found() -> StatusCode {
    StatusCode::NOT_FOUND
}

async fn observe_request(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let started_at = Instant::now();
    let method = request.method().to_string();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or("unmatched", MatchedPath::as_str)
        .to_owned();
    let request_id = valid_request_id(&request).unwrap_or_else(|| Uuid::new_v4().to_string());
    if let Ok(header_value) = HeaderValue::from_str(&request_id) {
        request
            .headers_mut()
            .insert(REQUEST_ID_HEADER, header_value);
    }

    let span = tracing::info_span!(
        "http_request",
        request_id = %request_id,
        method = %method,
        route = %route,
    );
    let mut response = next.run(request).instrument(span).await;
    let status = response.status();
    let duration = started_at.elapsed();
    state
        .metrics
        .observe_http(&method, &route, status.as_u16(), duration.as_secs_f64());
    tracing::info!(
        request_id = %request_id,
        method = %method,
        route = %route,
        status = status.as_u16(),
        duration_ms = duration.as_secs_f64() * 1_000.0,
        "http_request_completed"
    );

    if let Ok(header_value) = HeaderValue::from_str(&request_id) {
        response
            .headers_mut()
            .insert(REQUEST_ID_HEADER, header_value);
    }
    response
}

fn valid_request_id(request: &Request) -> Option<String> {
    let value = request.headers().get(&REQUEST_ID_HEADER)?.to_str().ok()?;
    if value.is_empty()
        || value.len() > 128
        || !value.chars().all(|character| character.is_ascii_graphic())
    {
        return None;
    }
    Some(value.to_owned())
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    uptime_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct PdfOperationReport {
    operation: PdfOperation,
    status: OperationStatus,
    duration_ms: f64,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PdfOperation {
    Merge,
    Split,
    Crop,
    Flatten,
    Rotate,
    ExtractText,
    Watermark,
    PdfA,
    Reorder,
    Redact,
    Sign,
}

impl PdfOperation {
    const fn as_label(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Split => "split",
            Self::Crop => "crop",
            Self::Flatten => "flatten",
            Self::Rotate => "rotate",
            Self::ExtractText => "extract_text",
            Self::Watermark => "watermark",
            Self::PdfA => "pdf_a",
            Self::Reorder => "reorder",
            Self::Redact => "redact",
            Self::Sign => "sign",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OperationStatus {
    Success,
    Error,
}

impl OperationStatus {
    const fn as_label(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Error => "error",
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn health_and_metrics_are_exposed() {
        let state = AppState::new();
        let app = router(state);

        let health = app
            .clone()
            .oneshot(
                Request::get("/health/ready")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("health response");
        assert_eq!(health.status(), StatusCode::OK);
        assert!(health.headers().contains_key(&REQUEST_ID_HEADER));

        let metrics = app
            .oneshot(
                Request::get("/metrics")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("metrics response");
        assert_eq!(metrics.status(), StatusCode::OK);
        let body = to_bytes(metrics.into_body(), 1024 * 1024)
            .await
            .expect("metrics body");
        let body = String::from_utf8(body.to_vec()).expect("UTF-8 metrics");
        assert!(body.contains("pdf_editor_http_requests_total"));
        assert!(body.contains("route=\"/health/ready\""));
    }

    #[tokio::test]
    async fn operation_reports_have_bounded_labels() {
        let app = router(AppState::new());
        let valid = Request::post("/api/v1/telemetry/pdf-operations")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                r#"{"operation":"merge","status":"success","duration_ms":12.5}"#,
            ))
            .expect("request");
        let response = app
            .clone()
            .oneshot(valid)
            .await
            .expect("telemetry response");
        assert_eq!(response.status(), StatusCode::ACCEPTED);

        let invalid = Request::post("/api/v1/telemetry/pdf-operations")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                r#"{"operation":"attacker-label","status":"success","duration_ms":12.5}"#,
            ))
            .expect("request");
        let response = app.oneshot(invalid).await.expect("validation response");
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}
