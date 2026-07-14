# Architecture

The editor is a local-first Rust workspace with a deliberately narrow server
boundary. PDF contents remain in the browser unless a user explicitly enables
history or permanent storage.

```text
Browser
├── Yew UI (apps/frontend)
└── Dedicated Web Worker (apps/pdf-worker)
    └── PDF engine (crates/pdf-engine, lopdf)

NGINX
├── static frontend and WASM artifacts
└── /api and /metrics proxy
    └── Axum backend (apps/backend)
        ├── auth, webhook and AI integration boundaries
        ├── optional R2/S3 persistence boundary
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
- Native Axum is the primary deployment target. Server-WASM adapters (Spin or
  WasmEdge) can reuse service modules but require target-specific network,
  database and AWS SDK adapters.

## Delivery roadmap

The initial vertical slice implements merge and split in WASM. The same engine
boundary is intended for crop, flatten, rotate, text extraction, watermark,
PDF/A conversion, reorder, redact and signing. Authentication (JWT/session,
WebAuthn/passkeys, backup codes), rate limiting, webhooks, AI/RAG and R2/S3 are
represented as backend module boundaries and are separate production features;
they must not be advertised as complete until their threat models and
integration tests are delivered.

