# PDF Editor · Rust/WASM

Editor PDF local-first construit ca workspace Rust. Documentele sunt procesate
în browser, într-un Web Worker WebAssembly, iar backend-ul primește numai
telemetrie operațională. Verticalele livrate implementează Merge, Split, Rotate,
Reorder, Crop, Watermark, Extract Text și Flatten, de la interfață până la
motorul PDF și observabilitate.

> Aceasta este fundația noii arhitecturi, nu încă paritatea funcțională 1:1 cu
> aplicația originală. Starea exactă și următoarele milestone-uri sunt în
> [roadmap](docs/roadmap.md).

## Ce este funcțional

- UI Yew compilat în WASM, cu selectare, validare și descărcare locală;
- Merge, Split, Rotate, Reorder, Crop, Watermark, Extract Text și Flatten
  implementate în Rust cu `lopdf`;
- procesare într-un Web Worker dedicat, fără blocarea thread-ului UI;
- transfer de `ArrayBuffer` între UI și worker, fără upload de conținut PDF;
- protocol worker versionat, cu request ID și erori stabile;
- backend Axum cu health checks, request IDs, loguri JSON și OpenMetrics;
- PostgreSQL 18 cu pool `sqlx`, migrații embedded și readiness dependent de DB;
- conturi cu parole Argon2id, sesiuni server-side rotative, JWT-uri scurte și
  protecție CSRF pentru operațiile bazate pe cookie;
- rate limiting distribuit pentru autentificare, cu bucket-uri PostgreSQL,
  scope-uri HMAC și răspuns standard `429 Retry-After`;
- telemetrie pentru succes, eroare și durata procesării client-side;
- NGINX, Prometheus LTS, Loki, Grafana Alloy și dashboard Grafana preconfigurat;
- imagini multi-stage, procese non-root și containere read-only unde este
  posibil;
- CI pentru format, Clippy, teste native, ținte WASM, configurații și imagini;
- CD către GitHub Container Registry și rollout VPS atomic prin SSH, cu host
  key pinning, backup pre-deploy, health-check și rollback automat;
- backup-uri PostgreSQL criptate în flux cu `age`, upload R2/S3 și verificare
  prin restaurare completă într-o bază temporară.

## Arhitectură

```text
Fișiere locale
    │
    ▼
Yew UI ── transferable buffers ──► Web Worker
    │                                  │
    │ numai metadate                   ▼
    │ operație/status/durată       pdf-engine + lopdf
    ▼                                  │
NGINX ──► Axum                         └──► Blob URL / download local
            ├── PostgreSQL
            ├── JSON logs ──► Alloy ──► Loki
            └── /metrics ────────────────► Prometheus
                                                │
                                                ▼
                                             Grafana
```

Conținutul documentelor tranzitorii nu traversează limita de rețea. Persistența
R2/S3 va fi disponibilă numai pentru un flux explicit de istoric și va necesita
consimțământ, autorizare și politici de retenție. Mai multe detalii sunt în
[deciziile de arhitectură](docs/architecture.md).

## Pornire rapidă

Cerința minimă este Docker cu pluginul Compose.

```bash
cp .env.example .env
docker compose up --build -d
docker compose ps
```

Aplicația este disponibilă la <http://localhost:8080>, iar Grafana la
<http://localhost:3000>. Schimbă `GRAFANA_ADMIN_PASSWORD` în `.env` înaintea
oricărui deployment accesibil din rețea. Schimbă obligatoriu și
`POSTGRES_PASSWORD`; valorile implicite sunt destinate exclusiv dezvoltării.

Verificări rapide:

```bash
curl --fail http://localhost:8080/health/ready
curl --fail http://localhost:8080/health/live
docker compose logs --tail=100 backend frontend
```

Porturile pot fi mutate fără modificarea fișierelor:

```bash
HTTP_PORT=18080 GRAFANA_PORT=13000 docker compose up --build -d
```

Oprire:

```bash
docker compose down
```

Deployment-ul public nu trebuie pornit numai din configurația locală de mai
sus. Override-ul de producție adaugă TLS automat, secrets montate din fișiere,
migrare one-shot și elimină expunerea directă a frontend-ului/Grafana. Pașii
compleți sunt în [ghidul de deployment VPS](docs/deployment.md).
Procedura de backup, testul de restaurare și pașii de disaster recovery sunt în
[runbook-ul dedicat](docs/disaster-recovery.md).

Adaugă `--volumes` numai dacă vrei să ștergi și baza PostgreSQL, plus istoricul
local Prometheus, Loki și Grafana.

## Dezvoltare și verificare

Versiunea Rust și ținta WASM sunt fixate în `rust-toolchain.toml`.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo check -p pdf-worker --target wasm32-unknown-unknown --release --locked
cargo check -p frontend --target wasm32-unknown-unknown --release --locked
```

Motorul PDF este independent de browser; testele native construiesc documente
de probă și verifică numărul, ordinea și textul paginilor după Merge/Split.
Build-ul de producție al frontend-ului instalează versiunile fixate de
`wasm-pack` și Trunk în [Dockerfile](docker/frontend/Dockerfile).

## API backend

| Metodă | Rută | Scop |
| --- | --- | --- |
| `GET` | `/health/live` | liveness fără dependențe externe |
| `GET` | `/health/ready` | readiness și uptime |
| `GET` | `/metrics` | OpenMetrics, accesibil intern Prometheus |
| `POST` | `/api/v1/auth/register` | creare cont și sesiune inițială |
| `POST` | `/api/v1/auth/login` | autentificare și sesiune nouă |
| `POST` | `/api/v1/auth/refresh` | rotație sesiune; necesită cookie și `X-CSRF-Token` |
| `POST` | `/api/v1/auth/logout` | revocare sesiune; necesită cookie și `X-CSRF-Token` |
| `GET` | `/api/v1/auth/me` | identitatea asociată unui JWT Bearer activ |
| `PUT` | `/api/v1/auth/profile` | actualizare nume public |
| `PUT` | `/api/v1/auth/password` | schimbare parolă, revocare globală și sesiune nouă |
| `DELETE` | `/api/v1/auth/account` | ștergere permanentă după reconfirmarea parolei |
| `GET` | `/api/v1/auth/sessions` | dispozitive/sesiuni active, cu IP și expirare |
| `DELETE` | `/api/v1/auth/sessions/{id}` | revocarea unei alte sesiuni |
| `POST` | `/api/v1/auth/sessions/others/revoke` | revocarea tuturor celorlalte sesiuni |
| `POST` | `/api/v1/auth/passkeys/register/start` | challenge de înrolare, cu JWT și reconfirmarea parolei |
| `POST` | `/api/v1/auth/passkeys/register/finish` | verificare attestation și salvare passkey |
| `POST` | `/api/v1/auth/passkeys/login/finish` | verificare assertion și emitere sesiune MFA |
| `GET` | `/api/v1/auth/passkeys` | passkeys și numărul codurilor de backup rămase |
| `DELETE` | `/api/v1/auth/passkeys/{id}` | eliminare passkey după reconfirmarea parolei |
| `POST` | `/api/v1/auth/mfa/disable` | eliminarea tuturor factorilor după reconfirmarea parolei |
| `POST` | `/api/v1/auth/mfa/backup-code` | consumă un cod one-time după etapa de parolă |
| `POST` | `/api/v1/auth/mfa/backup-codes/regenerate` | înlocuiește codurile după reconfirmarea parolei |
| `POST` | `/api/v1/auth/email/verification/request` | retrimite verificarea pentru contul autentificat |
| `POST` | `/api/v1/auth/email/verification/confirm` | consumă linkul one-time de verificare |
| `POST` | `/api/v1/auth/password/reset/request` | răspuns generic și email de recuperare, dacă identitatea există |
| `POST` | `/api/v1/auth/password/reset/confirm` | schimbă parola și revocă toate sesiunile |
| `POST` | `/api/v1/telemetry/pdf-operations` | rezultat și durată, fără bytes PDF |

Exemplu de telemetrie acceptată:

```bash
curl --fail-with-body \
  --header 'Content-Type: application/json' \
  --data '{"operation":"merge","status":"success","duration_ms":125.5}' \
  http://localhost:8080/api/v1/telemetry/pdf-operations
```

Operația și statusul sunt enum-uri închise pentru a preveni cardinalitatea
necontrolată în Prometheus. Durata acceptată este finită și limitată la 24 de
ore.

Parolele au între 12 și 128 de caractere și sunt stocate exclusiv ca hash
Argon2id cu salt aleator. Cookie-ul de sesiune este `HttpOnly`, `SameSite=Strict`
și `Secure` implicit în afara configurației locale. Backend-ul păstrează numai
hash-urile tokenurilor de sesiune și CSRF, rotește sesiunea la refresh și
invalidează imediat JWT-ul asociat sesiunii vechi. JWT-urile expiră implicit în
15 minute și sunt reverificate față de sesiunea activă din PostgreSQL. Contractul
și modelul de amenințări sunt descrise în [documentația de autentificare](docs/authentication.md).
Schimbarea parolei incrementează versiunea globală a tokenurilor, revocă toate
sesiunile existente și emite o singură sesiune nouă. IP-ul validat și user-agent-ul
sunt păstrate pentru inventarul de dispozitive și evenimentele de audit.
După înrolarea unui passkey, login-ul corect cu parolă răspunde `202` cu un
challenge WebAuthn; cookie-ul și JWT-ul sunt emise numai după assertion valid
sau consumarea unui cod de backup.
Verificarea emailului și resetarea parolei sunt livrate printr-un outbox
PostgreSQL durabil și SMTP STARTTLS. Linkurile sunt semnate HMAC-SHA-256 cu o
cheie separată, nu sunt stocate în clar și sunt consumate atomic o singură dată.

## Contractul Web Worker

Protocolul curent este `1`. Merge primește între 1 și 64 de documente; Split și
Rotate primesc exact un document și intervale inclusive, numerotate de la 1.
La Rotate, lista goală selectează toate paginile, iar unghiurile acceptate sunt
90, 180 și 270 de grade în sens orar. Reorder primește o permutare completă:
fiecare pagină trebuie inclusă exact o dată.
Crop folosește coordonate în puncte PDF (`left, bottom, right, top`), validează
aria și impune încadrarea în `MediaBox` pentru fiecare pagină selectată.
Watermark inserează textul în content stream, cu poziție, mărime, rotație și
opacitate. Implementarea curentă folosește Helvetica/WinAnsi și acceptă text
ASCII imprimabil; fonturile Unicode embedded rămân necesare pentru paritate.
Extract Text poate selecta intervale, limitează decompresia la 32 MiB per pagină
și livrează un fișier UTF-8 `text/plain`; PDF-urile scanate necesită OCR separat.
Flatten materializează appearance stream-urile câmpurilor `Widget`, elimină
`AcroForm` și păstrează adnotările non-formular; un widget fără aparență este
refuzat pentru a evita pierderea vizuală tăcută.
Operațiile au o limită cumulată de 256 MiB și refuză PDF-urile criptate.

```javascript
worker.postMessage(
  {
    protocol_version: 1,
    request_id: crypto.randomUUID(),
    operation: "split",
    document: pdfBytes,
    ranges: [{ start: 1, end: 3 }, { start: 8, end: 8 }],
  },
  [pdfBytes.buffer],
);
```

Răspunsurile includ `status`, `request_id`, `operation`, `duration_ms` și fie
`files` cu MIME type, fie `code` plus `message`. Contractul și exemplul Merge sunt descrise
în [documentația worker-ului](apps/pdf-worker/README.md).

## Observabilitate

Backend-ul scrie JSON structurat în stdout, inclusiv `request_id`, rută, status
și latență. NGINX folosește tot loguri JSON. Grafana Alloy descoperă numai
containerele proiectului Compose `wasm-pdf-editor` și le trimite către Loki.
Accesul la Docker trece printr-un proxy care permite numai citirea containerelor,
rețelelor și logurilor; Alloy nu montează socket-ul daemonului.

Prometheus colectează:

- `pdf_editor_http_requests_total`;
- `pdf_editor_http_request_duration_seconds`;
- `pdf_editor_operations_total`;
- `pdf_editor_operation_duration_seconds`.

Folderul Grafana `PDF Editor` și dashboard-ul `PDF Editor · Overview` sunt
provisionate automat cu rate HTTP, succes/erori PDF, percentile de latență și
loguri corelate.

## CI/CD

Workflow-ul `Test` rulează la push și pull request. Toate acțiunile GitHub sunt
fixate la commit SHA. Pipeline-ul blochează advisories Rust cunoscute, licențe
sau surse nepermise, secrete ajunse în istoricul Git și vulnerabilități de
severitate high/critical din imaginile finale; validează și ambele configurații
Compose, inclusiv overlay-ul de producție. După un test reușit pe `main`,
workflow-ul `Deploy` publică imaginile:

```text
ghcr.io/<owner>/<repository>-backend:sha-<commit>
ghcr.io/<owner>/<repository>-frontend:sha-<commit>
```

Tag-ul `latest` este publicat împreună cu SBOM și provenance. Pentru rollout
automat pe o platformă externă se configurează secretul repository
`DEPLOY_WEBHOOK_URL`; acesta primește un JSON cu revizia și ambele imagini. Fără
secret, publicarea imaginilor rămâne rezultatul final al pipeline-ului.

## Structura proiectului

```text
.
├── apps
│   ├── backend                 # Axum, PostgreSQL, migrations, metrics, tracing
│   ├── frontend                # Yew CSR și clientul Web Worker
│   └── pdf-worker              # adaptor wasm-bindgen și protocolul v1
├── crates
│   └── pdf-engine              # transformări PDF independente de runtime
├── docker
│   ├── backend/Dockerfile
│   └── frontend/Dockerfile
├── docs
│   ├── architecture.md
│   ├── deployment.md
│   └── roadmap.md
├── infra
│   ├── grafana                 # provisioning și dashboard
│   ├── loki/loki-config.yml
│   ├── nginx/nginx.conf
│   ├── prometheus/prometheus.yml
│   ├── alloy/config.alloy
│   └── caddy/Caddyfile
├── .github/workflows
│   ├── deploy.yml
│   └── test.yml
├── Cargo.toml                  # workspace și versiuni comune
├── Cargo.lock                  # build-uri reproductibile
├── docker-compose.yml
├── compose.production.yml       # secrets, migrare one-shot și Caddy HTTPS
└── rust-toolchain.toml
```

## Limite de producție ale versiunii curente

- UI-ul raportează telemetrie best-effort; nu raportează nume, bytes sau text
  extras din documente.
- Limita de 256 MiB reduce abuzul, dar memoria efectivă necesară poate depăși
  dimensiunea fișierului în timpul parsării. Limitele browserului rămân valabile.
- PDF-urile criptate trebuie decriptate înainte de procesare.
- TLS este obligatoriu înainte de expunerea publică; autentificarea prin
  parolă/sesiune/JWT, passkeys, backup codes și rate limiting-ul distribuit sunt
  livrate în backend. UI-ul de administrare MFA și recuperarea asistată a
  contului rămân în lucru.
- Migrarea inițială include schema pentru utilizatori, sesiuni, passkeys, coduri
  backup și audit.
- Axum nativ este ținta server principală. Un deployment Spin/WasmEdge cere
  adaptoare specifice pentru HTTP, stocare, baze de date și SDK-ul S3; binarul
  nativ existent nu trebuie prezentat drept componentă server-WASM.

Vezi [roadmap-ul](docs/roadmap.md) pentru operațiile PDF rămase, identitate,
storage, webhooks, AI/RAG și criteriile de finalizare.

## Licență

[MIT](LICENSE)
