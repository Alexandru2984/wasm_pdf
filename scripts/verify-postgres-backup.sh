#!/usr/bin/env bash

set -Eeuo pipefail

umask 077

PROJECT_DIR=${PROJECT_DIR:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)}
COMPOSE_PROJECT_NAME=${COMPOSE_PROJECT_NAME:-wasm-pdf-editor}
POSTGRES_USER=${POSTGRES_USER:-pdf_editor}
BACKUP_AGE_IDENTITY_FILE=${BACKUP_AGE_IDENTITY_FILE:-}

compose=(
  docker compose
  --project-name "$COMPOSE_PROJECT_NAME"
  --project-directory "$PROJECT_DIR"
  --file "$PROJECT_DIR/docker-compose.yml"
)

fail() {
  printf 'verify-postgres-backup: %s\n' "$*" >&2
  exit 1
}

backup_path=${1:-}
[[ -n $backup_path ]] || fail "usage: $0 PATH_TO_BACKUP.dump.age"
[[ -r $backup_path ]] || fail "backup is not readable: $backup_path"
[[ -r ${backup_path}.sha256 ]] || fail "checksum is not readable: ${backup_path}.sha256"
[[ -n $BACKUP_AGE_IDENTITY_FILE ]] || fail "BACKUP_AGE_IDENTITY_FILE is required"
[[ -r $BACKUP_AGE_IDENTITY_FILE ]] || fail "age identity is not readable: $BACKUP_AGE_IDENTITY_FILE"

for command_name in age docker sha256sum; do
  command -v "$command_name" >/dev/null 2>&1 || fail "required command not found: $command_name"
done

expected_checksum=$(awk 'NR == 1 { print $1 }' "${backup_path}.sha256")
[[ $expected_checksum =~ ^[0-9a-f]{64}$ ]] || fail "checksum file is malformed"
actual_checksum=$(sha256sum "$backup_path" | awk '{print $1}')
[[ $actual_checksum == "$expected_checksum" ]] || fail "encrypted backup checksum does not match"

container_id=$("${compose[@]}" ps --quiet postgres)
[[ -n $container_id ]] || fail "PostgreSQL container is not running"
[[ $(docker inspect --format '{{.State.Running}}' "$container_id") == true ]] || fail "PostgreSQL container is not running"

restore_database="restore_check_$(date -u +'%Y%m%d_%H%M%S')_$$"
[[ $restore_database =~ ^[a-z0-9_]+$ ]] || fail "generated restore database name is invalid"
database_created=false

cleanup() {
  if [[ $database_created == true ]]; then
    "${compose[@]}" exec -T postgres \
      dropdb --username "$POSTGRES_USER" --if-exists --force "$restore_database" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

printf 'verify-postgres-backup: restoring into disposable database %s\n' "$restore_database"
"${compose[@]}" exec -T postgres \
  createdb --username "$POSTGRES_USER" --template template0 "$restore_database"
database_created=true

age --decrypt --identity "$BACKUP_AGE_IDENTITY_FILE" "$backup_path" \
  | "${compose[@]}" exec -T postgres \
    pg_restore \
    --username "$POSTGRES_USER" \
    --dbname "$restore_database" \
    --no-owner \
    --no-acl \
    --exit-on-error \
    --single-transaction

migration_count=$("${compose[@]}" exec -T postgres \
  psql --username "$POSTGRES_USER" --dbname "$restore_database" \
  --no-align --tuples-only --set ON_ERROR_STOP=1 \
  --command "SELECT count(*) FROM _sqlx_migrations WHERE success IS TRUE;")
[[ $migration_count =~ ^[1-9][0-9]*$ ]] || fail "restored database has no successful SQLx migrations"

"${compose[@]}" exec -T postgres \
  dropdb --username "$POSTGRES_USER" --if-exists --force "$restore_database"
database_created=false
trap - EXIT

printf 'verify-postgres-backup: verified checksum and full restore (%s migrations)\n' "$migration_count"
