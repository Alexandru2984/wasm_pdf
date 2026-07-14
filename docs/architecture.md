# Architecture

The editor is a local-first Rust workspace with a deliberately narrow server
boundary. PDF contents remain in the browser unless a user explicitly enables
history, an AI operation or permanent storage.

```text
Browser
├── Yew UI (apps/frontend)
└── Dedicated Web Worker (apps/pdf-worker)
    └── PDF engine (crates/pdf-engine, lopdf)

Edge
├── Caddy terminates TLS and proxies /api directly to Axum in production
└── NGINX serves frontend/WASM; it proxies /api only in local development
    └── Axum backend (apps/backend)
        ├── current: health, telemetry, metrics and PostgreSQL pool
        ├── current: password auth, device sessions, JWT, rate limits and audit
        ├── current: passkeys, MFA lifecycle and one-time backup codes
        ├── planned: webhook and AI service adapters
        ├── planned: optional R2/S3 persistence adapter
        └── structured logs + OpenMetrics endpoint

Observability
├── Prometheus scrapes backend metrics
├── Alloy reads container JSON logs through a read-only Docker API proxy
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
- Native readiness checks PostgreSQL with a live query. Migration execution can
  be disabled for deployments that use a separate migration job.

## Delivery roadmap

The delivered vertical slices implement merge, split, rotate, reorder, crop and
standard-font watermarking, bounded text extraction and AcroForm flattening in
WASM. The same engine boundary is intended for PDF/A conversion, redact and signing.
Password authentication, rotating database sessions and short-lived JWTs are
delivered and documented in [authentication.md](authentication.md).
WebAuthn/passkeys, backup codes and their browser management UI are delivered,
as are email verification, password recovery and the durable SMTP outbox.
Virtual-authenticator browser E2E is still tracked separately. Webhooks,
AI/RAG and R2/S3 are
separate production features;
they must not be advertised as complete until their threat models and
integration tests are delivered. The ordered delivery and acceptance criteria
are maintained in [roadmap.md](roadmap.md).
