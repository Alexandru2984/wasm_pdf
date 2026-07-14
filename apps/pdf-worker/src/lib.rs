//! JavaScript boundary for the dedicated PDF Web Worker.

mod protocol;

use pdf_engine::{Error as EngineError, crop, merge, reorder, rotate, split};
use protocol::{Operation, PROTOCOL_VERSION, WorkerRequest, WorkerResponse};
use wasm_bindgen::prelude::*;

/// Installs readable panic messages in the browser worker console.
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// Process one versioned worker message.
///
/// JavaScript owns message transfer and invokes this function from a dedicated
/// worker. Keeping the event loop in JavaScript lets it transfer output
/// `ArrayBuffer`s without another copy.
#[wasm_bindgen]
pub fn handle_request(value: JsValue) -> JsValue {
    let started_at = js_sys::Date::now();
    let response = match serde_wasm_bindgen::from_value::<WorkerRequest>(value) {
        Ok(request) => process(request, started_at),
        Err(error) => WorkerResponse::error(
            "unknown".to_owned(),
            Operation::Unknown,
            "invalid_request",
            format!("worker request is invalid: {error}"),
            elapsed_ms(started_at),
        ),
    };

    serde_wasm_bindgen::to_value(&response).unwrap_or_else(|error| {
        JsValue::from_str(&format!("could not serialize worker response: {error}"))
    })
}

fn process(request: WorkerRequest, started_at: f64) -> WorkerResponse {
    let protocol_version = request.protocol_version();
    let request_id = request.request_id().to_owned();
    let operation = request.operation();

    if protocol_version != PROTOCOL_VERSION {
        return WorkerResponse::error(
            request_id,
            operation,
            "unsupported_protocol",
            format!(
                "protocol version {protocol_version} is unsupported; expected {PROTOCOL_VERSION}"
            ),
            elapsed_ms(started_at),
        );
    }

    let result = match request {
        WorkerRequest::Merge { documents, .. } => {
            let documents = documents
                .into_iter()
                .map(serde_bytes::ByteBuf::into_vec)
                .collect::<Vec<_>>();
            merge(&documents).map(|bytes| vec![bytes])
        }
        WorkerRequest::Split {
            document, ranges, ..
        } => split(&document, &ranges),
        WorkerRequest::Rotate {
            document,
            ranges,
            angle_degrees,
            ..
        } => rotate(&document, &ranges, angle_degrees).map(|bytes| vec![bytes]),
        WorkerRequest::Reorder {
            document, order, ..
        } => reorder(&document, &order).map(|bytes| vec![bytes]),
        WorkerRequest::Crop {
            document,
            ranges,
            rectangle,
            ..
        } => crop(&document, &ranges, rectangle).map(|bytes| vec![bytes]),
    };

    match result {
        Ok(files) => WorkerResponse::success(request_id, operation, files, elapsed_ms(started_at)),
        Err(error) => engine_error_response(request_id, operation, &error, started_at),
    }
}

fn engine_error_response(
    request_id: String,
    operation: Operation,
    error: &EngineError,
    started_at: f64,
) -> WorkerResponse {
    WorkerResponse::error(
        request_id,
        operation,
        error.code(),
        error.to_string(),
        elapsed_ms(started_at),
    )
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn elapsed_ms(started_at: f64) -> u64 {
    (js_sys::Date::now() - started_at).max(0.0).round() as u64
}
