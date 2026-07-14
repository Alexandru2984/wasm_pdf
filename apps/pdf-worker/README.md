# PDF worker

Build the Rust module with `wasm-pack build apps/pdf-worker --target web`. The
container and CI builds place the generated bindings at
`/assets/pdf-worker/`; `js/worker.js` is loaded as an ES module worker.

Requests use protocol version `1`:

```js
worker.postMessage(
  {
    protocol_version: 1,
    request_id: crypto.randomUUID(),
    operation: "merge",
    documents: pdfFiles.map((file) => new Uint8Array(file)),
  },
  pdfFiles,
);
```

Split requests use `operation: "split"`, one `document` byte array and an array
of inclusive, one-based `{ start, end }` ranges. Successful responses contain
transferable `Uint8Array` values under `files[].bytes`.

