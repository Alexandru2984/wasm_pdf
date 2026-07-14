# Autentificare și sesiuni

Backend-ul combină o sesiune opacă, păstrată într-un cookie `HttpOnly`, cu un
JWT Bearer de scurtă durată. Cookie-ul este folosit numai pentru refresh și
logout; endpoint-urile protejate folosesc JWT-ul și verifică de fiecare dată că
sesiunea din claim-ul `sid` este încă activă în PostgreSQL.

## Fluxuri

- `register` validează identitatea, calculează hash-ul Argon2id într-un task
  blocking izolat și creează utilizatorul, sesiunea și auditul într-o singură
  tranzacție;
- `login` execută o verificare Argon2 inclusiv pentru adrese inexistente,
  răspunde generic la credențiale invalide și blochează temporar contul după
  cinci încercări greșite;
- `refresh` cere simultan cookie-ul și tokenul CSRF, revocă atomic sesiunea
  curentă și emite o sesiune, un token CSRF și un JWT noi;
- `logout` verifică CSRF, revocă sesiunea și șterge cookie-ul;
- `me` validează semnătura, issuer-ul, audience-ul și expirarea JWT-ului, apoi
  verifică sesiunea, starea utilizatorului și versiunea globală a tokenurilor.

Răspunsul de autentificare conține JWT-ul, tokenul CSRF și profilul public.
Clientul păstrează JWT-ul și CSRF-ul numai în memorie; cookie-ul de sesiune este
gestionat de browser și nu este accesibil JavaScript-ului.

## Proprietăți de securitate

- parolele sunt acceptate între 12 și 128 de caractere și stocate ca Argon2id
  cu salt aleator;
- tokenurile opace au 256 de biți de entropie, iar baza de date păstrează numai
  digesturi SHA-256;
- refresh-ul este single-use inclusiv sub cereri concurente;
- cookie-ul are `Path=/api/v1/auth`, `HttpOnly` și `SameSite=Strict`; `Secure`
  este implicit activ și poate fi dezactivat numai pentru dezvoltarea HTTP;
- JWT-urile HS256 cer un secret de cel puțin 32 de bytes, au `iss`, `aud`,
  `sub`, `sid`, `jti`, `iat`, `exp` și o versiune revocabilă;
- evenimentele reușite de register, login, refresh și logout sunt persistate în
  `audit_events`, fără parole sau tokenuri;
- user-agent-ul este limitat la 512 bytes, iar erorile externe nu dezvăluie
  existența contului la login.

În producție, `JWT_SECRET` trebuie generat criptografic și injectat dintr-un
secret manager, `COOKIE_SECURE=true`, iar traficul trebuie terminat exclusiv
prin HTTPS. Valorile din Compose sunt numai pentru dezvoltare.

## Configurare

| Variabilă | Implicit | Restricție |
| --- | --- | --- |
| `JWT_SECRET` | fără implicit în proces | minimum 32 bytes |
| `JWT_ISSUER` | `wasm-pdf-editor` | text nevid |
| `JWT_AUDIENCE` | `wasm-pdf-editor-web` | text nevid |
| `ACCESS_TOKEN_SECONDS` | `900` | 60–3600 secunde |
| `SESSION_DAYS` | `30` | 1–90 zile |
| `COOKIE_SECURE` | `true` | `false` numai pentru HTTP local |

## Funcționalități rămase

Passkeys/WebAuthn, codurile de backup, recuperarea contului și rate limiting-ul
distribuit sunt verticale separate. Coloanele și tabelele necesare există în
migrarea inițială, dar niciunul dintre aceste mecanisme nu este prezentat drept
activ înaintea endpoint-urilor și testelor sale end-to-end.
