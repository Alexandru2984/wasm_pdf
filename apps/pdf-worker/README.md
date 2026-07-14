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

Rotate requests use `operation: "rotate"`, one `document`, optional inclusive
`ranges` (empty means all pages), and `angle_degrees` set to `90`, `180`, or
`270`. The output is one transformed PDF.

Reorder requests use `operation: "reorder"`, one `document`, and an `order`
array containing a complete one-based page permutation. Missing, duplicate, or
out-of-bounds page numbers are rejected instead of dropping content silently.

Crop requests use `operation: "crop"`, one `document`, optional inclusive
`ranges`, and `rectangle: { left, bottom, right, top }` in PDF points. Empty
ranges select all pages; the rectangle must fit every selected page's MediaBox.
