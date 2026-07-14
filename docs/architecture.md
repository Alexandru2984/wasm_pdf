# Architecture

The editor is a local-first Rust workspace with a deliberately narrow server
boundary. PDF contents remain in the browser unless a user explicitly enables
history, an AI operation or permanent storage.

```text
Browser
├── Yew UI (apps/frontend)
└── Dedicated Web Worker (apps/pdf-worker)
    └── PDF engine (crates/pdf-engine, lopdf)

NGINX
├── static frontend and WASM artifacts
└── /api and /metrics proxy
    └── Axum backend (apps/backend)
        ├── current: health, telemetry and metrics
        ├── planned: auth, webhook and AI service adapters
        ├── planned: optional R2/S3 persistence adapter
        └── structured logs + OpenMetrics endpoint

Observability
├── Prometheus scrapes backend metrics
├── Promtail reads container JSON logs and pushes them to Loki
└── Grafana queries Prometheus and Loki
```

## Design constraints

- CPU-heavy PDF work runs only in a dedicated browser worker.
- `pdf-engine` has no browser UI dependencies and is covered by native tests.
- Worker requests are versioned, typed messages; every response carries its
  request identifier so the UI can safely run concurrent jobs.
- The backend never receives transient document bytes. S3-compatible storage is
  reserved for explicit history/permanent-storage flows.
- Telemetry uses closed enums and contains only operation, status and duration;
  filenames, document contents and extracted text are excluded.
- Prometheus labels are derived from matched routes and bounded enums, never
  from arbitrary paths, user IDs or request bodies.
- Input limits are enforced inside `pdf-engine`, so native tests and WASM builds
  share the same behavior.
- Native Axum is the primary deployment target. Server-WASM adapters (Spin or
  WasmEdge) can reuse service modules but require target-specific network,
  database and AWS SDK adapters.

## Delivery roadmap

The delivered vertical slices implement merge, split, rotate and reorder in
WASM. The same engine boundary is intended for crop, flatten, text extraction, watermark,
PDF/A conversion, reorder, redact and signing. Authentication (JWT/session,
WebAuthn/passkeys, backup codes), rate limiting, webhooks, AI/RAG and R2/S3 are
represented as backend module boundaries and are separate production features;
they must not be advertised as complete until their threat models and
integration tests are delivered. The ordered delivery and acceptance criteria
are maintained in [roadmap.md](roadmap.md).
