# Security audit · 2026-07-14

## Verdict

**Conditional go for a controlled VPS deployment of the functionality that is
currently implemented.** The local-first editor, account system, email flows,
observability and deployment path have production-oriented controls and passed
the checks below. This is not a claim of complete 1:1 product parity or a
substitute for an independent penetration test.

The public product must not advertise Redact, cryptographic Sign, PDF/A, OCR,
R2/S3 history, webhooks, AI/RAG or server-side WASM as delivered. Their absence
is a product-scope gap, not a hidden security implementation.

Audit target: commit `0a903afa99b8b95842d50ff01f22f6a656eb73d5`, with the
reporting-only documentation commit that follows it. Tests ran on Linux with
Rust 1.96.0, PostgreSQL 18.4 and the pinned container images in the repository.

## Results

| Area | Result | Evidence |
| --- | --- | --- |
| Rust quality | Pass | Format and Clippy with `-D warnings`; 32 native tests passed |
| Dependency policy | Pass with tracked warnings | advisories, bans, licenses and sources passed; no known vulnerability |
| Secret scanning | Pass | Gitleaks scanned all 29 commits and found no leak |
| Container images | Pass | fresh Trivy DB: 0 High/Critical in backend and frontend images |
| Passive web scan | Pass with 3 informational warnings | ZAP: 64 PASS, 0 FAIL, 3 WARN |
| Browser boundary | Pass | Chrome loaded Yew/WASM under hash-based CSP; no `unsafe-inline` |
| HTTP boundary | Pass | 1 MiB body limit returned 413; auth responses are `no-store`; 30 s backend timeout |
| Database isolation | Pass | runtime role performed DML, could not create tables and had no cluster administration flags |
| Retention | Pass | expired rows in five security categories removed; current audit event preserved |
| Account/email lifecycle | Pass | register, SMTP delivery, verification, replay rejection, login and deletion completed end-to-end |
| Recovery | Pass in automated drill | streamed `age` backup and disposable PostgreSQL restore are exercised by CI |

The three ZAP warnings are accepted for this build:

- suspicious comments come from generated Yew/Gloo JavaScript markers and do
  not contain credentials;
- fingerprinted JavaScript, WASM and CSS are intentionally cacheable; the HTML
  entry point is served with revalidation;
- “Modern Web Application” is an informational classification.

Trunk emits a known JavaScript minifier warning (`KeywordDefault`) while still
producing a successful release. The generated JavaScript is served as built;
this affects artifact size, not the browser security policy.

## Identity, cryptography and storage

- Passwords use Argon2id v1.3 with 19,456 KiB memory, two iterations, one lane
  and a random salt. Parameters are pinned by a regression test.
- Session and CSRF values contain 256 random bits. PostgreSQL stores only their
  SHA-256 digests, and CSRF comparison is constant-time.
- JWT access tokens use HS256, a distinct random key of at least 256 bits,
  issuer/audience validation, a 15-minute default lifetime and database-backed
  revocation on every protected request.
- Email verification and recovery links use a separate HMAC-SHA-256 key,
  purpose/expiry binding and atomic one-time consumption.
- Passkey challenges and verified credential state are kept server-side in
  PostgreSQL; backup codes are random one-time values stored only as digests.
- Database backups are streamed directly through `age` X25519 encryption. The
  private identity must be stored outside the VPS.
- PostgreSQL data is not application-field-encrypted. Use encrypted VPS disks,
  protected snapshots and encrypted off-site backups. Production SMTP requires
  STARTTLS. TLS to browsers terminates at Caddy.

PostgreSQL is the system of record. A one-shot owner role applies migrations and
provisions `pdf_editor_runtime`; the long-running backend receives only the
runtime password. That role has table DML and sequence usage, but no superuser,
database creation, role creation, replication, row-security bypass or schema
creation privileges.

## Residual risks and decisions

No known High/Critical finding remains in the tested scope. The following work
or explicit operational decisions remain:

1. An independent penetration test has not been performed. Run one before a
   broad public launch or before storing valuable customer data.
2. Full browser E2E for WebAuthn with a virtual authenticator is still pending.
   Server-side origin, RP, expiry, replay and credential checks are present.
3. Registration creates an initial session before email ownership is verified.
   This is acceptable for the current optional, local-only account experience,
   but future history, billing, sharing, AI or privileged actions must require
   `email_verified=true` where ownership matters.
4. JWT signing uses one symmetric key without a key ID/key ring. Rotation is an
   operational secret replacement that invalidates outstanding access tokens;
   seamless multi-key rotation is not implemented.
5. Daily encrypted logical backups are available, but continuous PostgreSQL
   point-in-time recovery is not. Choose and document an acceptable RPO/RTO;
   add WAL archiving if daily potential data loss is unacceptable.
6. `bincode 1.3.3` and `proc-macro-error 1.0.4` are unmaintained transitive
   dependencies in the Yew/Gloo build graph. RustSec reports no vulnerability;
   upgrades remain tracked through the dependency policy.
7. CSP permits `wasm-unsafe-eval`, which is required to instantiate WASM. It
   does not permit arbitrary inline scripts; the generated bootstrap uses exact
   SHA-256 hashes computed during the image build.

## Public go-live gate

Before DNS is switched to the VPS:

- generate every secret independently, store it as mode `0600`, and keep the
  PostgreSQL owner password away from the long-running backend;
- configure a verified SMTP sender plus SPF, DKIM and DMARC; test bounce and
  abuse handling with the selected provider;
- complete and record an encrypted off-site restore drill, then decide whether
  WAL/PITR is required by the business RPO;
- expose only 80/443, harden SSH, and keep Grafana behind VPN or an SSH tunnel;
- require a green `Test` workflow and deploy immutable SHA image tags through
  the protected GitHub `production` environment;
- configure monitoring ownership and alerts for availability, 5xx, auth rate
  limits, dead email deliveries, backup failures and maintenance errors;
- publish privacy/retention terms and an incident contact;
- run an external penetration test for a broad public launch.
