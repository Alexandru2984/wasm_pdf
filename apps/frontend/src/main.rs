mod worker;

use gloo::net::http::Request;
use serde::Serialize;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Event, HtmlInputElement, HtmlSelectElement, InputEvent, Url};
use worker::{Operation, PageRange, WorkerResponse};
use yew::prelude::*;

#[derive(Clone, PartialEq)]
struct DownloadFile {
    name: String,
    url: String,
}

#[function_component(App)]
fn app() -> Html {
    let operation = use_state(|| Operation::Merge);
    let files = use_state(Vec::new);
    let range_text = use_state(|| "1-1".to_owned());
    let busy = use_state(|| false);
    let error = use_state(|| None::<String>);
    let downloads = use_state(Vec::<DownloadFile>::new);

    let on_operation_change = {
        let operation = operation.clone();
        let files = files.clone();
        let downloads = downloads.clone();
        Callback::from(move |event: Event| {
            let input = event.target_unchecked_into::<HtmlSelectElement>();
            operation.set(if input.value() == "split" {
                Operation::Split
            } else {
                Operation::Merge
            });
            files.set(Vec::new());
            revoke_downloads(&downloads);
            downloads.set(Vec::new());
        })
    };

    let on_files_change = {
        let files = files.clone();
        let error = error.clone();
        Callback::from(move |event: Event| {
            let input = event.target_unchecked_into::<HtmlInputElement>();
            let selected = input
                .files()
                .map(|list| {
                    (0..list.length())
                        .filter_map(|index| list.get(index))
                        .collect()
                })
                .unwrap_or_default();
            files.set(selected);
            error.set(None);
        })
    };

    let on_range_change = {
        let range_text = range_text.clone();
        Callback::from(move |event: InputEvent| {
            let input = event.target_unchecked_into::<HtmlInputElement>();
            range_text.set(input.value());
        })
    };

    let on_process = {
        let operation = operation.clone();
        let files = files.clone();
        let range_text = range_text.clone();
        let busy = busy.clone();
        let error = error.clone();
        let downloads = downloads.clone();
        Callback::from(move |_| {
            if *busy {
                return;
            }
            let selected_files = (*files).clone();
            let current_operation = *operation;
            if selected_files.is_empty() {
                error.set(Some("Selectează cel puțin un fișier PDF.".to_owned()));
                return;
            }
            if current_operation == Operation::Split && selected_files.len() != 1 {
                error.set(Some("Split acceptă exact un fișier PDF.".to_owned()));
                return;
            }
            let ranges = if current_operation == Operation::Split {
                match parse_ranges(&range_text) {
                    Ok(ranges) => ranges,
                    Err(message) => {
                        error.set(Some(message));
                        return;
                    }
                }
            } else {
                Vec::new()
            };

            busy.set(true);
            error.set(None);
            revoke_downloads(&downloads);
            downloads.set(Vec::new());

            let busy = busy.clone();
            let error = error.clone();
            let downloads = downloads.clone();
            spawn_local(async move {
                let request =
                    match worker::read_request(current_operation, selected_files, ranges).await {
                        Ok(request) => request,
                        Err(message) => {
                            busy.set(false);
                            error.set(Some(message));
                            return;
                        }
                    };

                let callback_busy = busy.clone();
                let callback_error = error.clone();
                let callback = Callback::from(move |response: Result<WorkerResponse, String>| {
                    callback_busy.set(false);
                    match response {
                        Ok(WorkerResponse::Success {
                            files,
                            duration_ms,
                            operation,
                            ..
                        }) => {
                            let output = files
                                .into_iter()
                                .filter_map(|file| match file.into_download() {
                                    Ok(file) => Some(file),
                                    Err(message) => {
                                        callback_error.set(Some(message));
                                        None
                                    }
                                })
                                .collect::<Vec<_>>();
                            downloads.set(output);
                            report_telemetry(operation, "success", duration_ms);
                        }
                        Ok(WorkerResponse::Error {
                            message,
                            duration_ms,
                            operation,
                            ..
                        }) => {
                            callback_error.set(Some(message));
                            report_telemetry(operation, "error", duration_ms);
                        }
                        Err(message) => callback_error.set(Some(message)),
                    }
                });

                if let Err(message) = worker::dispatch(request, callback) {
                    busy.set(false);
                    error.set(Some(message));
                }
            });
        })
    };

    let is_split = *operation == Operation::Split;
    let file_summary = if files.is_empty() {
        "Niciun fișier selectat".to_owned()
    } else {
        format!("{} fișier(e), procesate doar în browser", files.len())
    };

    html! {
        <main class="shell">
            <header class="hero">
                <p class="eyebrow">{"RUST · WASM · LOCAL-FIRST"}</p>
                <h1>{"PDF Editor"}</h1>
                <p class="lede">
                    {"Unește și separă documente fără upload. Bytes rămân în browser, iar procesarea rulează într-un Web Worker dedicat."}
                </p>
            </header>

            <section class="workspace" aria-labelledby="tool-title">
                <div class="section-heading">
                    <div>
                        <p class="step">{"01 / Operație"}</p>
                        <h2 id="tool-title">{"Alege transformarea"}</h2>
                    </div>
                    <span class="privacy-pill">{"Local processing"}</span>
                </div>

                <label class="field-label" for="operation">{"Operație"}</label>
                <select id="operation" onchange={on_operation_change} disabled={*busy}>
                    <option value="merge" selected={!is_split}>{"Merge — combină PDF-uri"}</option>
                    <option value="split" selected={is_split}>{"Split — extrage intervale"}</option>
                </select>

                <div class="upload-zone">
                    <label for="pdf-files" class="upload-label">
                        <span class="upload-icon" aria-hidden="true">{"↗"}</span>
                        <strong>{if is_split { "Alege un PDF" } else { "Alege PDF-urile" }}</strong>
                        <span>{"Click pentru selectare · max. 256 MiB per operație"}</span>
                    </label>
                    <input
                        id="pdf-files"
                        class="file-input"
                        type="file"
                        accept="application/pdf,.pdf"
                        multiple={!is_split}
                        onchange={on_files_change}
                        disabled={*busy}
                    />
                    <p class="file-summary">{file_summary}</p>
                </div>

                if is_split {
                    <div class="range-field">
                        <label class="field-label" for="ranges">{"Intervale de pagini"}</label>
                        <input
                            id="ranges"
                            type="text"
                            value={(*range_text).clone()}
                            oninput={on_range_change}
                            placeholder="1-3, 5, 8-10"
                            disabled={*busy}
                        />
                        <small>{"Intervalele sunt inclusive și numerotate de la 1."}</small>
                    </div>
                }

                <button class="process-button" onclick={on_process} disabled={*busy || files.is_empty()}>
                    if *busy {
                        <span class="spinner" aria-hidden="true"></span>
                        {"Procesez în worker…"}
                    } else if is_split {
                        {"Separă documentul"}
                    } else {
                        {"Unește documentele"}
                    }
                </button>

                if let Some(message) = &*error {
                    <div class="notice error" role="alert">{message}</div>
                }

                if !downloads.is_empty() {
                    <div class="results" aria-live="polite">
                        <p class="step">{"02 / Rezultat"}</p>
                        <h2>{"Fișiere pregătite"}</h2>
                        <div class="download-list">
                            {downloads.iter().map(|file| html! {
                                <a class="download-link" href={file.url.clone()} download={file.name.clone()}>
                                    <span>{&file.name}</span><span aria-hidden="true">{"↓"}</span>
                                </a>
                            }).collect::<Html>()}
                        </div>
                    </div>
                }
            </section>

            <footer>
                <span>{"PDF contents never cross the network boundary."}</span>
                <span>{"Engine v0.1 · Protocol v1"}</span>
            </footer>
        </main>
    }
}

impl worker::WorkerFile {
    fn into_download(self) -> Result<DownloadFile, String> {
        let bytes = js_sys::Uint8Array::from(self.bytes.as_slice());
        let parts = js_sys::Array::of1(&bytes);
        let blob = web_sys::Blob::new_with_u8_array_sequence(&parts)
            .map_err(|error| format!("Nu am putut crea rezultatul: {error:?}"))?;
        let url = Url::create_object_url_with_blob(&blob)
            .map_err(|error| format!("Nu am putut crea linkul: {error:?}"))?;
        Ok(DownloadFile {
            name: self.name,
            url,
        })
    }
}

fn parse_ranges(value: &str) -> Result<Vec<PageRange>, String> {
    let mut ranges = Vec::new();
    for item in value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let parts = item.split('-').map(str::trim).collect::<Vec<_>>();
        let range = match parts.as_slice() {
            [page] => {
                let page = page
                    .parse::<u32>()
                    .map_err(|_| format!("Pagina „{page}” nu este validă."))?;
                PageRange {
                    start: page,
                    end: page,
                }
            }
            [start, end] => PageRange {
                start: start
                    .parse::<u32>()
                    .map_err(|_| format!("Pagina „{start}” nu este validă."))?,
                end: end
                    .parse::<u32>()
                    .map_err(|_| format!("Pagina „{end}” nu este validă."))?,
            },
            _ => return Err(format!("Intervalul „{item}” nu este valid.")),
        };
        if range.start == 0 || range.start > range.end {
            return Err(format!("Intervalul „{item}” nu este valid."));
        }
        ranges.push(range);
    }
    if ranges.is_empty() {
        return Err("Introdu cel puțin un interval de pagini.".to_owned());
    }
    Ok(ranges)
}

fn revoke_downloads(downloads: &UseStateHandle<Vec<DownloadFile>>) {
    for file in downloads.iter() {
        let _ = Url::revoke_object_url(&file.url);
    }
}

#[derive(Serialize)]
struct TelemetryReport<'a> {
    operation: &'a str,
    status: &'a str,
    duration_ms: f64,
}

fn report_telemetry(operation: Operation, status: &'static str, duration_ms: u64) {
    spawn_local(async move {
        let duration_ms =
            u32::try_from(duration_ms.min(86_400_000)).map_or(86_400_000.0, f64::from);
        let report = TelemetryReport {
            operation: operation.as_label(),
            status,
            duration_ms,
        };
        let Ok(request) = Request::post("/api/v1/telemetry/pdf-operations").json(&report) else {
            return;
        };
        let _ = request.send().await;
    });
}

fn main() {
    yew::Renderer::<App>::new().render();
}
