# Roadmap către paritate 1:1

Roadmap-ul separă funcționalitățile în verticale verificabile. O componentă nu
este considerată livrată doar pentru că există o interfață sau un modul gol;
trebuie să aibă teste, observabilitate și modelul de securitate corespunzător.

## M0 · Fundație local-first — livrat

- workspace Rust și contracte comune;
- Merge, Split, Rotate, Reorder, Crop, Watermark, Extract Text și Flatten în
  motorul `pdf-engine`;
- Web Worker WASM și UI Yew;
- backend Axum pentru health și telemetrie;
- Prometheus LTS, Loki, Grafana Alloy și Grafana;
- containere de producție și pipeline-uri CI/CD.

## M1 · Paritate motor PDF

Operațiile vor fi adăugate individual în `pdf-engine`, fără dependențe de UI:

1. font embedded cu subset Unicode pentru watermark;
2. OCR pentru documente bazate pe imagini;
3. suport pentru appearance matrices complexe la flatten;
4. redact cu eliminarea reală a conținutului, nu doar suprapunere vizuală;
5. semnare criptografică și validarea lanțului de certificate;
6. conversie PDF/A cu validare de conformitate.

Fiecare operație necesită corpus de documente, teste pentru obiecte indirecte,
pagini moștenite, fonturi, imagini, formulare, documente corupte și limite de
memorie. Funcțiile care cer randare fidelă pot necesita un al doilea engine
compatibil WASM; alegerea se face pe baza acoperirii de licență și a testelor,
nu prin înlocuirea arbitrară a lui `lopdf`.

## M2 · Identitate și control acces

- PostgreSQL pentru utilizatori, credențiale și audit — fundație livrată;
- sesiuni server-side cu cookie `Secure`, `HttpOnly`, `SameSite` și protecție
  CSRF, plus JWT cu durată scurtă — livrat;
- WebAuthn/passkeys cu verificarea origin/RP ID, challenge server-side și
  protecție anti-replay — backend livrat; UI și browser E2E rămân;
- coduri de backup generate criptografic, afișate o singură dată, stocate numai
  sub formă de hash și consumate tranzacțional — backend livrat;
- rate limiting distribuit pe identitate, IP/sesiune și categorie de endpoint
  pentru fluxurile de autentificare — livrat;
- rotație/revocare și audit pentru register/login/refresh/logout — livrate;
- inventar și revocare dispozitive, schimbare parolă cu revocare globală,
  profil, ștergere cont și lifecycle MFA/passkey — backend livrat;
- recuperare cont și testele complete de autorizare negative.

Înainte de implementare se fixează threat model-ul, schema de tenancy și
politicile de retenție. Endpoint-urile de identitate nu vor fi amestecate cu
motorul PDF client-side.

## M3 · Persistență și integrări backend

- adaptor `aws-sdk-s3` pentru R2/S3, cu multipart upload, checksum, criptare,
  URL-uri semnate cu durată scurtă și ștergere verificabilă;
- istoric opt-in și politici explicite de retenție;
- webhooks semnate, idempotency keys, retry cu backoff și protecție anti-replay;
- interfață de provider pentru chat, summarize și rephrase;
- pipeline RAG cu separarea tenant-urilor, filtrarea conținutului și citarea
  surselor;
- timeouts, circuit breakers, bugete și metrici pentru toate apelurile externe.

Conținutul ajunge la backend sau la un provider AI numai după o acțiune
explicită a utilizatorului. Providerul și scopul transferului trebuie să fie
vizibile înainte de confirmare.

## M4 · Runtime server-WASM

Logica de domeniu va fi extrasă din adaptorul Axum în servicii independente de
transport. Ținta nativă rămâne referința de performanță; Spin/WasmEdge primește
adaptoare proprii pentru HTTP, secrets, PostgreSQL, S3 și observabilitate.

Criterii de finalizare:

- aceeași suită de contract tests pe adaptorul nativ și cel WASM;
- fără dependențe native incompatibile ascunse în serviciile de domeniu;
- parity pentru autentificare, autorizare, timeouts și formatul metricilor;
- benchmark-uri publicate pentru cold start, latență și memorie.

## Cerințe transversale

Fiecare milestone trebuie să păstreze:

- loguri structurate fără date sensibile;
- label-uri Prometheus cu cardinalitate limitată;
- request/correlation ID de la edge la integrarea externă;
- dependency audit, SBOM, imagini non-root și secrete în afara imaginilor;
- teste unitare, de integrare, browser și end-to-end;
- migrații reversibile și documentație operațională înainte de rollout.
