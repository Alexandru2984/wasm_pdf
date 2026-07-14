import init, { handle_request } from "/assets/pdf-worker/pdf_worker.js";

const ready = init({ module_or_path: "/assets/pdf-worker/pdf_worker_bg.wasm" });

self.onmessage = async ({ data }) => {
  await ready;
  const response = handle_request(data);
  const transfer = response?.files
    ?.map(({ bytes }) => bytes.buffer)
    .filter(Boolean) ?? [];
  self.postMessage(response, transfer);
};

self.onerror = ({ message, filename, lineno, colno }) => {
  console.error("Uncaught PDF worker error", { message, filename, lineno, colno });
};

