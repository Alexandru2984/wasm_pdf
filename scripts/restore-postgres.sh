#!/usr/bin/env bash

set -Eeuo pipefail

umask 077

PROJECT_DIR=${PROJECT_DIR:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)}
COMPOSE_PROJECT_NAME=${COMPOSE_PROJECT_NAME:-wasm-pdf-editor}
POSTGRES_DB=${POSTGRES_DB:-pdf_editor}
POSTGRES_USER=${POSTGRES_USER:-pdf_editor}
BACKUP_AGE_IDENTITY_FILE=${BACKUP_AGE_IDENTITY_FILE:-}

compose=(
  docker compose
  --project-name "$COMPOSE_PROJECT_NAME"
  --project-directory "$PROJECT_DIR"
  --file "$PROJECT_DIR/docker-compose.yml"
)

fail() {
  printf 'restore-postgres: %s\n' "$*" >&2
  exit 1
}

backup_path=${1:-}
confirmation=${2:-}
[[ -n $backup_path ]] || fail "usage: $0 PATH_TO_BACKUP.dump.age restore:POSTGRES_DB"
[[ $confirmation == "restore:$POSTGRES_DB" ]] || fail "confirmation must be exactly: restore:$POSTGRES_DB"
[[ -r $backup_path ]] || fail "backup is not readable: $backup_path"
[[ -r ${backup_path}.sha256 ]] || fail "checksum is not readable: ${backup_path}.sha256"
[[ -n $BACKUP_AGE_IDENTITY_FILE ]] || fail "BACKUP_AGE_IDENTITY_FILE is required"
[[ -r $BACKUP_AGE_IDENTITY_FILE ]] || fail "age identity is not readable: $BACKUP_AGE_IDENTITY_FILE"
[[ $POSTGRES_DB =~ ^[a-zA-Z0-9_]+$ ]] || fail "POSTGRES_DB contains unsupported characters"

for command_name in age docker sha256sum; do
  command -v "$command_name" >/dev/null 2>&1 || fail "required command not found: $command_name"
done

backend_id=$("${compose[@]}" ps --quiet backend)
if [[ -n $backend_id && $(docker inspect --format '{{.State.Running}}' "$backend_id") == true ]]; then
  fail "backend must be stopped before a production database restore"
fi

expected_checksum=$(awk 'NR == 1 { print $1 }' "${backup_path}.sha256")
[[ $expected_checksum =~ ^[0-9a-f]{64}$ ]] || fail "checksum file is malformed"
actual_checksum=$(sha256sum "$backup_path" | awk '{print $1}')
[[ $actual_checksum == "$expected_checksum" ]] || fail "encrypted backup checksum does not match"

printf 'restore-postgres: validating decryption key and archive before destructive action\n'
age --decrypt --identity "$BACKUP_AGE_IDENTITY_FILE" "$backup_path" \
  | "${compose[@]}" exec -T postgres pg_restore --list >/dev/null

printf 'restore-postgres: replacing database %s\n' "$POSTGRES_DB"
"${compose[@]}" exec -T postgres \
  dropdb --username "$POSTGRES_USER" --if-exists --force "$POSTGRES_DB"
"${compose[@]}" exec -T postgres \
  createdb --username "$POSTGRES_USER" --template template0 "$POSTGRES_DB"

age --decrypt --identity "$BACKUP_AGE_IDENTITY_FILE" "$backup_path" \
  | "${compose[@]}" exec -T postgres \
    pg_restore \
    --username "$POSTGRES_USER" \
    --dbname "$POSTGRES_DB" \
    --no-owner \
    --no-acl \
    --exit-on-error \
    --single-transaction

migration_count=$("${compose[@]}" exec -T postgres \
  psql --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" \
  --no-align --tuples-only --set ON_ERROR_STOP=1 \
  --command "SELECT count(*) FROM _sqlx_migrations WHERE success IS TRUE;")
[[ $migration_count =~ ^[1-9][0-9]*$ ]] || fail "restored database has no successful SQLx migrations"

printf 'restore-postgres: restored %s successfully (%s migrations); redeploy the application now\n' \
  "$POSTGRES_DB" "$migration_count"
