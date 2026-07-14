use js_sys::{Array, Object, Reflect, Uint8Array};
use serde::Deserialize;
use wasm_bindgen::{JsValue, closure::Closure};
use wasm_bindgen_futures::JsFuture;
use web_sys::{File, MessageEvent, Worker, WorkerOptions, WorkerType};
use yew::Callback;

const PROTOCOL_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Merge,
    Split,
    Rotate,
    Reorder,
    Crop,
    Watermark,
    Unknown,
}

impl Operation {
    pub const fn as_label(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Split => "split",
            Self::Rotate => "rotate",
            Self::Reorder => "reorder",
            Self::Crop => "crop",
            Self::Watermark => "watermark",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PageRange {
    pub start: u32,
    pub end: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct PdfRect {
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
    pub top: f32,
}

#[derive(Clone, Debug)]
pub struct WatermarkOptions {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub font_size: f32,
    pub rotation_degrees: f32,
    pub opacity: f32,
}

pub enum WorkerRequest {
    Merge {
        request_id: String,
        documents: Vec<Vec<u8>>,
    },
    Split {
        request_id: String,
        document: Vec<u8>,
        ranges: Vec<PageRange>,
    },
    Rotate {
        request_id: String,
        document: Vec<u8>,
        ranges: Vec<PageRange>,
        angle_degrees: i16,
    },
    Reorder {
        request_id: String,
        document: Vec<u8>,
        order: Vec<u32>,
    },
    Crop {
        request_id: String,
        document: Vec<u8>,
        ranges: Vec<PageRange>,
        rectangle: PdfRect,
    },
    Watermark {
        request_id: String,
        document: Vec<u8>,
        ranges: Vec<PageRange>,
        options: WatermarkOptions,
    },
}

pub struct OperationOptions {
    pub ranges: Vec<PageRange>,
    pub angle_degrees: i16,
    pub order: Vec<u32>,
    pub rectangle: Option<PdfRect>,
    pub watermark: Option<WatermarkOptions>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum WorkerResponse {
    Success {
        operation: Operation,
        files: Vec<WorkerFile>,
        duration_ms: u64,
    },
    Error {
        operation: Operation,
        message: String,
        duration_ms: u64,
    },
}

#[derive(Debug, Deserialize)]
pub struct WorkerFile {
    pub name: String,
    #[serde(with = "serde_bytes")]
    pub bytes: Vec<u8>,
}

pub async fn read_request(
    operation: Operation,
    files: Vec<File>,
    options: OperationOptions,
) -> Result<WorkerRequest, String> {
    let mut documents = Vec::with_capacity(files.len());
    for file in files {
        let buffer = JsFuture::from(file.array_buffer())
            .await
            .map_err(|error| format!("Nu am putut citi {}: {error:?}", file.name()))?;
        documents.push(Uint8Array::new(&buffer).to_vec());
    }
    let request_id = uuid::Uuid::new_v4().to_string();
    match operation {
        Operation::Merge => Ok(WorkerRequest::Merge {
            request_id,
            documents,
        }),
        Operation::Split => Ok(WorkerRequest::Split {
            request_id,
            document: documents.into_iter().next().ok_or("Fișierul lipsește.")?,
            ranges: options.ranges,
        }),
        Operation::Rotate => Ok(WorkerRequest::Rotate {
            request_id,
            document: documents.into_iter().next().ok_or("Fișierul lipsește.")?,
            ranges: options.ranges,
            angle_degrees: options.angle_degrees,
        }),
        Operation::Reorder => Ok(WorkerRequest::Reorder {
            request_id,
            document: documents.into_iter().next().ok_or("Fișierul lipsește.")?,
            order: options.order,
        }),
        Operation::Crop => Ok(WorkerRequest::Crop {
            request_id,
            document: documents.into_iter().next().ok_or("Fișierul lipsește.")?,
            ranges: options.ranges,
            rectangle: options
                .rectangle
                .ok_or("Dreptunghiul de decupare lipsește.")?,
        }),
        Operation::Watermark => Ok(WorkerRequest::Watermark {
            request_id,
            document: documents.into_iter().next().ok_or("Fișierul lipsește.")?,
            ranges: options.ranges,
            options: options
                .watermark
                .ok_or("Configurația watermark lipsește.")?,
        }),
        Operation::Unknown => Err("Operația nu este suportată.".to_owned()),
    }
}

pub fn dispatch(
    request: WorkerRequest,
    callback: Callback<Result<WorkerResponse, String>>,
) -> Result<(), String> {
    let options = WorkerOptions::new();
    options.set_type(WorkerType::Module);
    let worker = Worker::new_with_options("/assets/pdf-worker/worker.js", &options)
        .map_err(|error| format!("Nu am putut porni Web Worker-ul: {error:?}"))?;

    let success_worker = worker.clone();
    let success_callback = callback.clone();
    let on_message = Closure::<dyn FnMut(MessageEvent)>::once(move |event: MessageEvent| {
        let response = serde_wasm_bindgen::from_value::<WorkerResponse>(event.data())
            .map_err(|error| format!("Răspuns invalid de la worker: {error}"));
        success_worker.terminate();
        success_callback.emit(response);
    })
    .into_js_value();
    Reflect::set(
        worker.as_ref(),
        &JsValue::from_str("onmessage"),
        &on_message,
    )
    .map_err(|error| format!("Nu am putut conecta worker-ul: {error:?}"))?;

    let error_worker = worker.clone();
    let on_error = Closure::<dyn FnMut(JsValue)>::once(move |event: JsValue| {
        error_worker.terminate();
        callback.emit(Err(format!("Worker-ul PDF a eșuat: {event:?}")));
    })
    .into_js_value();
    Reflect::set(worker.as_ref(), &JsValue::from_str("onerror"), &on_error)
        .map_err(|error| format!("Nu am putut configura worker-ul: {error:?}"))?;

    let (message, transfer) = request.into_js_message()?;
    worker
        .post_message_with_transfer(&message, &transfer)
        .map_err(|error| format!("Nu am putut trimite documentul la worker: {error:?}"))
}

impl WorkerRequest {
    fn into_js_message(self) -> Result<(JsValue, Array), String> {
        let message = Object::new();
        let transfer = Array::new();
        set(&message, "protocol_version", &PROTOCOL_VERSION.into())?;

        match self {
            Self::Merge {
                request_id,
                documents,
            } => {
                set(&message, "request_id", &JsValue::from_str(&request_id))?;
                set(&message, "operation", &JsValue::from_str("merge"))?;
                let values = Array::new();
                for document in documents {
                    let bytes = Uint8Array::from(document.as_slice());
                    transfer.push(&bytes.buffer());
                    values.push(&bytes);
                }
                set(&message, "documents", &values.into())?;
            }
            Self::Split {
                request_id,
                document,
                ranges,
            } => {
                set_document_request(&message, &transfer, &request_id, "split", &document)?;
                set(&message, "ranges", &ranges_to_js(ranges)?.into())?;
            }
            Self::Rotate {
                request_id,
                document,
                ranges,
                angle_degrees,
            } => {
                set_document_request(&message, &transfer, &request_id, "rotate", &document)?;
                set(&message, "ranges", &ranges_to_js(ranges)?.into())?;
                set(&message, "angle_degrees", &angle_degrees.into())?;
            }
            Self::Reorder {
                request_id,
                document,
                order,
            } => {
                set_document_request(&message, &transfer, &request_id, "reorder", &document)?;
                let values = order.into_iter().map(JsValue::from).collect::<Array>();
                set(&message, "order", &values.into())?;
            }
            Self::Crop {
                request_id,
                document,
                ranges,
                rectangle,
            } => {
                set_document_request(&message, &transfer, &request_id, "crop", &document)?;
                set(&message, "ranges", &ranges_to_js(ranges)?.into())?;
                let value = Object::new();
                set(&value, "left", &rectangle.left.into())?;
                set(&value, "bottom", &rectangle.bottom.into())?;
                set(&value, "right", &rectangle.right.into())?;
                set(&value, "top", &rectangle.top.into())?;
                set(&message, "rectangle", &value.into())?;
            }
            Self::Watermark {
                request_id,
                document,
                ranges,
                options,
            } => {
                set_document_request(&message, &transfer, &request_id, "watermark", &document)?;
                set(&message, "ranges", &ranges_to_js(ranges)?.into())?;
                let value = Object::new();
                set(&value, "text", &JsValue::from_str(&options.text))?;
                set(&value, "x", &options.x.into())?;
                set(&value, "y", &options.y.into())?;
                set(&value, "font_size", &options.font_size.into())?;
                set(&value, "rotation_degrees", &options.rotation_degrees.into())?;
                set(&value, "opacity", &options.opacity.into())?;
                set(&message, "options", &value.into())?;
            }
        }

        Ok((message.into(), transfer))
    }
}

fn set_document_request(
    message: &Object,
    transfer: &Array,
    request_id: &str,
    operation: &str,
    document: &[u8],
) -> Result<(), String> {
    set(message, "request_id", &JsValue::from_str(request_id))?;
    set(message, "operation", &JsValue::from_str(operation))?;
    let bytes = Uint8Array::from(document);
    transfer.push(&bytes.buffer());
    set(message, "document", &bytes.into())
}

fn ranges_to_js(ranges: Vec<PageRange>) -> Result<Array, String> {
    let values = Array::new();
    for range in ranges {
        let value = Object::new();
        set(&value, "start", &range.start.into())?;
        set(&value, "end", &range.end.into())?;
        values.push(&value);
    }
    Ok(values)
}

fn set(object: &Object, key: &str, value: &JsValue) -> Result<(), String> {
    Reflect::set(object, &JsValue::from_str(key), value)
        .map(|_| ())
        .map_err(|error| format!("Nu am putut construi cererea worker: {error:?}"))
}
