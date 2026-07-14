#!/usr/bin/env bash
set -euo pipefail

project="${DATABASE_SECURITY_PROJECT:-wasm-pdf-database-security}"
owner_password='ci-database-owner-password'
runtime_password='ci-runtime-password-0123456789-abcdefghijklmnopqrstuvwxyz'
runtime_user='pdf_editor_runtime_ci'

compose() {
  POSTGRES_PASSWORD="$owner_password" docker compose -p "$project" "$@"
}

cleanup() {
  compose down --volumes --remove-orphans >/dev/null 2>&1 || true
}
trap cleanup EXIT

cleanup
compose up -d --wait postgres
container_id=$(compose ps -q postgres)
database_ip=$(docker inspect \
  --format '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' \
  "$container_id")
test -n "$database_ip"

APP_ENV=test \
DATABASE_HOST="$database_ip" \
DATABASE_USER=pdf_editor \
DATABASE_PASSWORD="$owner_password" \
DATABASE_MAX_CONNECTIONS=2 \
RUN_MIGRATIONS=true \
RUST_LOG=backend=warn \
  cargo run --quiet --locked --package backend -- migrate

APP_ENV=test \
DATABASE_HOST="$database_ip" \
DATABASE_USER=pdf_editor \
DATABASE_PASSWORD="$owner_password" \
DATABASE_MAX_CONNECTIONS=2 \
RUN_MIGRATIONS=false \
DATABASE_RUNTIME_USER="$runtime_user" \
DATABASE_RUNTIME_PASSWORD="$runtime_password" \
RUST_LOG=backend=warn \
  cargo run --quiet --locked --package backend -- provision-database-role

role_flags=$(docker exec -e PGPASSWORD="$owner_password" "$container_id" \
  psql --no-psqlrc --set ON_ERROR_STOP=1 --username pdf_editor \
  --dbname pdf_editor --tuples-only --no-align \
  --command "SELECT rolsuper, rolcreatedb, rolcreaterole, rolreplication, rolbypassrls FROM pg_roles WHERE rolname='$runtime_user'")
if [[ "$role_flags" != 'f|f|f|f|f' ]]; then
  printf 'runtime role has unexpected cluster privileges: %s\n' "$role_flags" >&2
  exit 1
fi

docker exec -e PGPASSWORD="$runtime_password" "$container_id" \
  psql --no-psqlrc --set ON_ERROR_STOP=1 --host 127.0.0.1 \
  --username "$runtime_user" --dbname pdf_editor \
  --command 'SELECT count(*) FROM users' >/dev/null
if docker exec -e PGPASSWORD="$runtime_password" "$container_id" \
  psql --no-psqlrc --set ON_ERROR_STOP=1 --host 127.0.0.1 \
  --username "$runtime_user" --dbname pdf_editor \
  --command 'CREATE TABLE privilege_escape (id integer)' >/dev/null 2>&1; then
  printf 'runtime role unexpectedly created a table\n' >&2
  exit 1
fi

docker exec -i -e PGPASSWORD="$owner_password" "$container_id" \
  psql --no-psqlrc --set ON_ERROR_STOP=1 --username pdf_editor \
  --dbname pdf_editor <<'SQL' >/dev/null
INSERT INTO users (id, email, email_normalized, display_name, password_hash, created_at, updated_at)
VALUES ('10000000-0000-0000-0000-000000000001', 'expired@example.test', 'expired@example.test', 'Expired', 'not-used', now() - interval '10 days', now() - interval '10 days');
INSERT INTO sessions (id, user_id, token_hash, csrf_token_hash, created_at, last_seen_at, expires_at, idle_expires_at)
VALUES ('20000000-0000-0000-0000-000000000001', '10000000-0000-0000-0000-000000000001', decode(repeat('01', 32), 'hex'), decode(repeat('02', 32), 'hex'), now() - interval '10 days', now() - interval '5 days', now() - interval '5 days', now() - interval '5 days');
INSERT INTO webauthn_ceremonies (id, user_id, session_id, kind, state, nickname, created_at, expires_at)
VALUES ('30000000-0000-0000-0000-000000000001', '10000000-0000-0000-0000-000000000001', '20000000-0000-0000-0000-000000000001', 'registration', '{}', 'Expired', now() - interval '2 days', now() - interval '1 day');
INSERT INTO account_tokens (id, user_id, purpose, expires_at, created_at)
VALUES ('40000000-0000-0000-0000-000000000001', '10000000-0000-0000-0000-000000000001', 'reset_password', now() - interval '9 days', now() - interval '10 days');
INSERT INTO rate_limit_buckets (scope_hash, category, window_start, request_count, expires_at)
VALUES (decode(repeat('03', 32), 'hex'), 'test', now() - interval '1 hour', 1, now() - interval '30 minutes');
INSERT INTO audit_events (id, event_type, outcome, created_at)
VALUES ('50000000-0000-0000-0000-000000000001', 'expired', 'success', now() - interval '40 days'),
       ('50000000-0000-0000-0000-000000000002', 'current', 'success', now());
SQL

APP_ENV=test \
DATABASE_HOST="$database_ip" \
DATABASE_USER="$runtime_user" \
DATABASE_PASSWORD="$runtime_password" \
DATABASE_MAX_CONNECTIONS=2 \
RUN_MIGRATIONS=false \
AUDIT_RETENTION_DAYS=30 \
SESSION_RETENTION_DAYS=1 \
MAINTENANCE_INTERVAL_SECONDS=300 \
RUST_LOG=backend=warn \
  cargo run --quiet --locked --package backend -- maintenance

remaining=$(docker exec -e PGPASSWORD="$owner_password" "$container_id" \
  psql --no-psqlrc --set ON_ERROR_STOP=1 --username pdf_editor \
  --dbname pdf_editor --tuples-only --no-align \
  --command "SELECT (SELECT count(*) FROM sessions), (SELECT count(*) FROM webauthn_ceremonies), (SELECT count(*) FROM account_tokens), (SELECT count(*) FROM rate_limit_buckets), (SELECT count(*) FROM audit_events WHERE event_type='expired'), (SELECT count(*) FROM audit_events WHERE event_type='current')")
if [[ "$remaining" != '0|0|0|0|0|1' ]]; then
  printf 'unexpected retention result: %s\n' "$remaining" >&2
  exit 1
fi

printf 'database security integration test passed\n'
