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
- telemetrie pentru succes, eroare și durata procesării client-side;
- NGINX, Prometheus, Loki, Promtail și dashboard Grafana preconfigurat;
- imagini multi-stage, procese non-root și containere read-only unde este
  posibil;
- CI pentru format, Clippy, teste native, ținte WASM, configurații și imagini;
- CD către GitHub Container Registry, urmat opțional de un webhook de rollout.

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
            ├── JSON logs ──► Promtail ──► Loki
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
oricărui deployment accesibil din rețea.

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

Adaugă `--volumes` numai dacă vrei să ștergi și istoricul local Prometheus,
Loki și Grafana.

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
și latență. NGINX folosește tot loguri JSON. Promtail descoperă numai
containerele proiectului Compose `wasm-pdf-editor` și le trimite către Loki.

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
fixate la commit SHA. După un test reușit pe `main`, workflow-ul `Deploy`
publică imaginile:

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
│   ├── backend                 # Axum, health, telemetry, metrics, tracing
│   ├── frontend                # Yew CSR și clientul Web Worker
│   └── pdf-worker              # adaptor wasm-bindgen și protocolul v1
├── crates
│   └── pdf-engine              # transformări PDF independente de runtime
├── docker
│   ├── backend/Dockerfile
│   └── frontend/Dockerfile
├── docs
│   ├── architecture.md
│   └── roadmap.md
├── infra
│   ├── grafana                 # provisioning și dashboard
│   ├── loki/loki-config.yml
│   ├── nginx/nginx.conf
│   ├── prometheus/prometheus.yml
│   └── promtail/promtail-config.yml
├── .github/workflows
│   ├── deploy.yml
│   └── test.yml
├── Cargo.toml                  # workspace și versiuni comune
├── Cargo.lock                  # build-uri reproductibile
├── docker-compose.yml
└── rust-toolchain.toml
```

## Limite de producție ale versiunii curente

- UI-ul raportează telemetrie best-effort; nu raportează nume, bytes sau text
  extras din documente.
- Limita de 256 MiB reduce abuzul, dar memoria efectivă necesară poate depăși
  dimensiunea fișierului în timpul parsării. Limitele browserului rămân valabile.
- PDF-urile criptate trebuie decriptate înainte de procesare.
- TLS, autentificarea, autorizarea și rate limiting-ul sunt obligatorii înainte
  de expunerea viitoarelor endpoint-uri cu date de utilizator.
- Axum nativ este ținta server principală. Un deployment Spin/WasmEdge cere
  adaptoare specifice pentru HTTP, stocare, baze de date și SDK-ul S3; binarul
  nativ existent nu trebuie prezentat drept componentă server-WASM.

Vezi [roadmap-ul](docs/roadmap.md) pentru operațiile PDF rămase, identitate,
storage, webhooks, AI/RAG și criteriile de finalizare.

## Licență

[MIT](LICENSE)
