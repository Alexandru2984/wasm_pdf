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
    Unknown,
}

impl Operation {
    pub const fn as_label(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Split => "split",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PageRange {
    pub start: u32,
    pub end: u32,
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
    ranges: Vec<PageRange>,
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
            ranges,
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
                set(&message, "request_id", &JsValue::from_str(&request_id))?;
                set(&message, "operation", &JsValue::from_str("split"))?;
                let bytes = Uint8Array::from(document.as_slice());
                transfer.push(&bytes.buffer());
                set(&message, "document", &bytes.into())?;
                let values = Array::new();
                for range in ranges {
                    let value = Object::new();
                    set(&value, "start", &range.start.into())?;
                    set(&value, "end", &range.end.into())?;
                    values.push(&value);
                }
                set(&message, "ranges", &values.into())?;
            }
        }

        Ok((message.into(), transfer))
    }
}

fn set(object: &Object, key: &str, value: &JsValue) -> Result<(), String> {
    Reflect::set(object, &JsValue::from_str(key), value)
        .map(|_| ())
        .map_err(|error| format!("Nu am putut construi cererea worker: {error:?}"))
}
