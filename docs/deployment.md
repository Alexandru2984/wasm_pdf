# Production deployment on a VPS

Production uses the base Compose model plus `compose.production.yml`. The
override removes the direct frontend port, binds Grafana only to loopback,
mounts credentials as Docker secrets, runs database migrations once and places
Caddy in front of the application for automatic HTTPS.

## Prerequisites

- a Linux VPS with Docker Engine and Docker Compose 2.24.4 or newer;
- a dedicated unprivileged deployment user with SSH key authentication;
- an A/AAAA DNS record pointing the application hostname at the VPS;
- inbound TCP 80/443 and UDP 443; Grafana and PostgreSQL stay non-public;
- immutable backend and frontend image references, preferably `sha-*` tags or
  image digests published by the deployment workflow;
- `age`, AWS CLI v2 and the encrypted off-site backup policy described in the
  [disaster recovery runbook](disaster-recovery.md).

## Create host secrets

Secrets live outside the checkout and must never be committed. Create distinct
random values for PostgreSQL, JWT signing, account-link signing and Grafana.
Write the SMTP credential supplied by the provider separately:

```bash
sudo install -d -m 0700 /etc/wasm-pdf-editor/secrets
openssl rand -base64 48 | sudo tee /etc/wasm-pdf-editor/secrets/postgres_password >/dev/null
openssl rand -base64 48 | sudo tee /etc/wasm-pdf-editor/secrets/database_runtime_password >/dev/null
openssl rand -base64 48 | sudo tee /etc/wasm-pdf-editor/secrets/jwt_secret >/dev/null
openssl rand -base64 48 | sudo tee /etc/wasm-pdf-editor/secrets/email_token_secret >/dev/null
openssl rand -base64 48 | sudo tee /etc/wasm-pdf-editor/secrets/grafana_admin_password >/dev/null
printf '%s' "$SMTP_PROVIDER_PASSWORD" | sudo tee /etc/wasm-pdf-editor/secrets/smtp_password >/dev/null
sudo chmod 0600 /etc/wasm-pdf-editor/secrets/*
```

The backend accepts `DATABASE_PASSWORD_FILE`, `JWT_SECRET_FILE`,
`EMAIL_TOKEN_SECRET_FILE`, `SMTP_PASSWORD_FILE` and, when a complete external
connection string is needed, `DATABASE_URL_FILE`. Setting a value and its
`_FILE` variant simultaneously is rejected.

`postgres_password` belongs only to the database owner and one-shot migration,
role-provisioning and recovery jobs. The long-running backend connects as
`pdf_editor_runtime` using `database_runtime_password`. The provisioning job
removes superuser, database/role creation, replication and row-security bypass
privileges, revokes public schema creation and grants only application DML.

## Prepare the release account

Use `/opt/wasm-pdf-editor` as the release root and allow the unprivileged
`deploy` account to own it. The account needs Docker access and one narrowly
scoped passwordless command for the mandatory pre-deployment backup:

```bash
sudo install -d -o deploy -g deploy -m 0750 \
  /opt/wasm-pdf-editor /opt/wasm-pdf-editor/releases /opt/wasm-pdf-editor/incoming
sudo usermod -aG docker deploy
echo 'deploy ALL=(root) NOPASSWD: /usr/bin/systemctl start wasm-pdf-backup.service' \
  | sudo tee /etc/sudoers.d/wasm-pdf-editor-backup >/dev/null
sudo chmod 0440 /etc/sudoers.d/wasm-pdf-editor-backup
sudo visudo --check --file=/etc/sudoers.d/wasm-pdf-editor-backup
```

Membership in the Docker group is root-equivalent. Use this account only for
deployment, disable password authentication and restrict its SSH key by source
address when GitHub-hosted runner networking permits it.

Install `infra/examples/deployment.env.example` as
`/etc/wasm-pdf-editor/deployment.env`, replace every placeholder and make it
readable only by `deploy`. It contains deployment settings, while credentials
remain separate Docker secret files.

If the GHCR packages are private, authenticate once as `deploy` with a
fine-grained, read-only package token. Never put this token in
`deployment.env`:

```bash
printf '%s' "$GHCR_READ_ONLY_TOKEN" \
  | docker login ghcr.io --username YOUR_GITHUB_USER --password-stdin
```

## Render and start

Set deployment values in the shell or in a root-readable environment file:

```bash
export PUBLIC_DOMAIN=pdf.example.com
export BACKEND_IMAGE=ghcr.io/owner/repository-backend:sha-<commit>
export FRONTEND_IMAGE=ghcr.io/owner/repository-frontend:sha-<commit>
export SECRETS_DIR=/etc/wasm-pdf-editor/secrets
export SMTP_HOST=smtp.provider.example
export SMTP_PORT=587
export SMTP_USERNAME=provider-account
export SMTP_FROM_ADDRESS=no-reply@example.com
```

Validate the merged model before every rollout:

```bash
docker compose -f docker-compose.yml -f compose.production.yml config --quiet
docker compose -f docker-compose.yml -f compose.production.yml pull
docker compose -f docker-compose.yml -f compose.production.yml up -d --remove-orphans
docker compose -f docker-compose.yml -f compose.production.yml ps
curl --fail https://$PUBLIC_DOMAIN/health/ready
```

Caddy obtains and renews the certificate and redirects HTTP to HTTPS. The
backend refuses to start in `APP_ENV=production` unless cookies are secure,
WebAuthn uses an HTTPS origin and public RP ID, and the JWT/database secrets are
not development placeholders. Email delivery must be active over STARTTLS, and
its signing secret must be distinct from the JWT secret.

Local Compose runs Mailpit on `http://127.0.0.1:8025`; it is removed entirely by
the production override and must never be exposed as a production relay.

The `migrate` service runs embedded SQLx migrations to completion before the
backend starts. Backend replicas use `RUN_MIGRATIONS=false`, so schema changes
do not race during rollout.

## Automated rollout from GitHub

The deployment workflow publishes SBOM/provenance-enabled images tagged with
the full tested commit SHA. Configure these repository secrets:

- `VPS_HOST`, `VPS_PORT` (optional, defaults to 22) and `VPS_USER`;
- `VPS_SSH_PRIVATE_KEY`, a dedicated deployment key;
- `VPS_KNOWN_HOSTS`, captured and verified out-of-band from the VPS host key.

Attach required reviewers to the GitHub `production` environment if releases
need an explicit approval gate. Store the VPS secrets in that environment so
they are exposed only to the rollout job.

Only after the `Test` workflow passes on `main`, CI builds a
checksum-protected release archive,
uses strict SSH host-key checking and runs `scripts/deploy-vps.sh` remotely. The
script serializes deployments with `flock`, validates archive paths and SHA-256,
accepts only image tags matching the tested revision, pulls before switching,
runs the systemd backup on upgrades, changes the `current` symlink atomically,
waits for Compose health and verifies readiness over the public TLS hostname.
If readiness fails, it restores the preceding release and image references.
Database migrations must therefore remain backward compatible with one prior
application release.

If the SSH secrets are absent, the workflow intentionally publishes images but
does not mutate any server.

## Grafana access

Grafana binds only to `127.0.0.1:3000`. Reach it through a VPN or an SSH tunnel:

```bash
ssh -L 3000:127.0.0.1:3000 deploy@your-vps
```

Then open `http://127.0.0.1:3000`. Do not publish port 3000 in the VPS firewall.

## Rollback

Keep the preceding immutable image references. To roll back application code,
restore `BACKEND_IMAGE` and `FRONTEND_IMAGE` to the previous commit and run the
same `up -d` command. A migration that is not backward compatible requires a
documented database restore or forward-fix plan before it can be deployed.

Database backup, point-in-time recovery and restore verification are separate
mandatory gates before the first public rollout. The repository now includes a
daily systemd timer, streamed `age` encryption, optional R2/S3 upload and a full
disposable restore verifier. Complete and record one successful restore drill
before exposing the service publicly.
