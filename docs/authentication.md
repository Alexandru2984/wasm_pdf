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
  verifică sesiunea, starea utilizatorului și versiunea globală a tokenurilor;
- managementul sesiunilor listează dispozitivele active și permite revocarea
  unei alte sesiuni sau a tuturor celorlalte sesiuni;
- schimbarea parolei cere parola curentă, incrementează versiunea tokenurilor,
  revocă toate sesiunile și creează atomic o singură sesiune nouă;
- profilul poate fi actualizat, iar ștergerea contului cere din nou parola și
  elimină datele de identitate prin cascadele PostgreSQL.

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
- evenimentele de identitate și administrare sunt persistate în `audit_events`,
  împreună cu IP și user-agent limitat, fără parole sau tokenuri;
- register, login, refresh și logout sunt limitate prin contoare atomice în
  PostgreSQL; scope-urile IP, identitate și sesiune sunt pseudonimizate cu HMAC,
  iar răspunsurile limitate includ `Retry-After`;
- user-agent-ul este limitat la 512 bytes, iar erorile externe nu dezvăluie
  existența contului la login.

## Passkeys și coduri de backup

Înrolarea necesită o sesiune activă și reconfirmarea parolei. Backend-ul emite
un challenge legat de RP ID/origin și păstrează starea completă a ceremoniei
numai în PostgreSQL, timp de cinci minute. Starea nu este semnată și trimisă
clientului. Finalizarea consumă atomic ceremonia înaintea verificării, refuză
credential ID-uri duplicate și păstrează credentialul verificat complet.

La primul passkey sunt generate zece coduri de backup de 100 de biți, afișate o
singură dată. În DB există numai SHA-256-ul formei normalizate. Un cod poate fi
folosit numai după o parolă corectă care a creat ceremonia MFA; consumarea
codului și crearea sesiunii sunt în aceeași tranzacție. Regenerarea cere din nou
parola și invalidează toate codurile anterioare.

Pentru un cont cu MFA, parola nu mai emite sesiune. Login-ul răspunde `202` cu
`status=mfa_required`, `ceremony_id` și opțiunile WebAuthn. Assertion-ul valid
actualizează sign counter-ul/backup state-ul credentialului și abia apoi creează
cookie-ul, JWT-ul și evenimentul de audit.

Un passkey poate fi eliminat numai după reconfirmarea parolei. Eliminarea
ultimului passkey dezactivează obligatoriu MFA și invalidează toate codurile de
backup. Endpoint-ul dedicat de dezactivare MFA șterge într-o singură tranzacție
ceremoniile în curs, credentialele și codurile de recuperare.

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
| `WEBAUTHN_RP_ID` | `localhost` | domeniu stabil, compatibil cu origin |
| `WEBAUTHN_RP_ORIGIN` | `http://localhost:8080` | origin exact; HTTPS în producție |
| `WEBAUTHN_RP_NAME` | `PDF Editor` | nume afișat de authenticator |

## Funcționalități rămase

UI-ul browser pentru autentificare și înrolare/administrare, verificarea emailului
și recuperarea asistată a contului rămân verticale separate. Testele browser vor
folosi un authenticator virtual; testele API verifică între timp origin/RP,
persistența server-side și expirarea challenge-ului.
