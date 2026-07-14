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
  image digests published by the deployment workflow.

## Create host secrets

Secrets live outside the checkout and must never be committed. Create distinct
random values for PostgreSQL, JWT signing and Grafana:

```bash
sudo install -d -m 0700 /etc/wasm-pdf-editor/secrets
openssl rand -base64 48 | sudo tee /etc/wasm-pdf-editor/secrets/postgres_password >/dev/null
openssl rand -base64 48 | sudo tee /etc/wasm-pdf-editor/secrets/jwt_secret >/dev/null
openssl rand -base64 48 | sudo tee /etc/wasm-pdf-editor/secrets/grafana_admin_password >/dev/null
sudo chmod 0600 /etc/wasm-pdf-editor/secrets/*
```

The backend accepts `DATABASE_PASSWORD_FILE`, `JWT_SECRET_FILE` and, when a
complete external connection string is needed, `DATABASE_URL_FILE`. Setting a
value and its `_FILE` variant simultaneously is rejected.

## Render and start

Set deployment values in the shell or in a root-readable environment file:

```bash
export PUBLIC_DOMAIN=pdf.example.com
export BACKEND_IMAGE=ghcr.io/owner/repository-backend:sha-<commit>
export FRONTEND_IMAGE=ghcr.io/owner/repository-frontend:sha-<commit>
export SECRETS_DIR=/etc/wasm-pdf-editor/secrets
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
not development placeholders.

The `migrate` service runs embedded SQLx migrations to completion before the
backend starts. Backend replicas use `RUN_MIGRATIONS=false`, so schema changes
do not race during rollout.

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
mandatory gates before the first public rollout.
