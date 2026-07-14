#!/usr/bin/env bash

set -Eeuo pipefail

umask 077

PROJECT_DIR=${PROJECT_DIR:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)}
COMPOSE_PROJECT_NAME=${COMPOSE_PROJECT_NAME:-wasm-pdf-editor}
POSTGRES_DB=${POSTGRES_DB:-pdf_editor}
POSTGRES_USER=${POSTGRES_USER:-pdf_editor}
BACKUP_DIR=${BACKUP_DIR:-/var/backups/wasm-pdf-editor/postgres}
BACKUP_RETENTION_DAYS=${BACKUP_RETENTION_DAYS:-14}
BACKUP_AGE_RECIPIENT_FILE=${BACKUP_AGE_RECIPIENT_FILE:-/etc/wasm-pdf-editor/backup.age-recipient}
BACKUP_REQUIRE_OFFSITE=${BACKUP_REQUIRE_OFFSITE:-false}
BACKUP_S3_URI=${BACKUP_S3_URI:-}
BACKUP_S3_ENDPOINT=${BACKUP_S3_ENDPOINT:-}

compose=(
  docker compose
  --project-name "$COMPOSE_PROJECT_NAME"
  --project-directory "$PROJECT_DIR"
  --file "$PROJECT_DIR/docker-compose.yml"
)

fail() {
  printf 'backup-postgres: %s\n' "$*" >&2
  exit 1
}

for command_name in age docker flock sha256sum; do
  command -v "$command_name" >/dev/null 2>&1 || fail "required command not found: $command_name"
done

[[ $BACKUP_RETENTION_DAYS =~ ^[1-9][0-9]*$ ]] || fail "BACKUP_RETENTION_DAYS must be a positive integer"
[[ -r $BACKUP_AGE_RECIPIENT_FILE ]] || fail "age recipient file is not readable: $BACKUP_AGE_RECIPIENT_FILE"

recipient=$(awk '!/^($|#)/ { print; exit }' "$BACKUP_AGE_RECIPIENT_FILE")
[[ $recipient == age1* ]] || fail "recipient file must contain an age X25519 public recipient"

if [[ $BACKUP_REQUIRE_OFFSITE == true && -z $BACKUP_S3_URI ]]; then
  fail "BACKUP_REQUIRE_OFFSITE=true but BACKUP_S3_URI is empty"
fi
if [[ -n $BACKUP_S3_URI ]]; then
  command -v aws >/dev/null 2>&1 || fail "aws CLI is required when BACKUP_S3_URI is configured"
  [[ $BACKUP_S3_URI == s3://* ]] || fail "BACKUP_S3_URI must start with s3://"
fi

install -d -m 0700 "$BACKUP_DIR"
exec 9>"$BACKUP_DIR/.backup.lock"
flock -n 9 || fail "another backup is already running"

container_id=$("${compose[@]}" ps --quiet postgres)
[[ -n $container_id ]] || fail "PostgreSQL container is not running"
[[ $(docker inspect --format '{{.State.Running}}' "$container_id") == true ]] || fail "PostgreSQL container is not running"

timestamp=$(date -u +'%Y%m%dT%H%M%SZ')
backup_name="postgres-${timestamp}.dump.age"
backup_path="$BACKUP_DIR/$backup_name"
temporary_path="$BACKUP_DIR/.${backup_name}.partial"
checksum_path="${backup_path}.sha256"
temporary_checksum_path="${checksum_path}.partial"
[[ ! -e $backup_path && ! -e $checksum_path ]] || fail "backup name collision: $backup_name"

cleanup() {
  rm -f -- "$temporary_path" "$temporary_checksum_path"
}
trap cleanup EXIT

printf 'backup-postgres: creating encrypted backup %s\n' "$backup_name"
"${compose[@]}" exec -T postgres \
  pg_dump \
  --username "$POSTGRES_USER" \
  --dbname "$POSTGRES_DB" \
  --format custom \
  --compress zstd:9 \
  --no-owner \
  --no-acl \
  --lock-wait-timeout 10s \
  | age --encrypt --recipient "$recipient" --output "$temporary_path"

[[ -s $temporary_path ]] || fail "encrypted backup is empty"
chmod 0600 "$temporary_path"
mv -- "$temporary_path" "$backup_path"

checksum=$(sha256sum "$backup_path" | awk '{print $1}')
printf '%s  %s\n' "$checksum" "$backup_name" >"$temporary_checksum_path"
chmod 0600 "$temporary_checksum_path"
mv -- "$temporary_checksum_path" "$checksum_path"

if [[ -n $BACKUP_S3_URI ]]; then
  aws_options=()
  if [[ -n $BACKUP_S3_ENDPOINT ]]; then
    aws_options+=(--endpoint-url "$BACKUP_S3_ENDPOINT")
  fi
  destination=${BACKUP_S3_URI%/}
  aws "${aws_options[@]}" s3 cp --only-show-errors "$backup_path" "$destination/$backup_name"
  aws "${aws_options[@]}" s3 cp --only-show-errors "$checksum_path" "$destination/${backup_name}.sha256"
  printf 'backup-postgres: uploaded encrypted backup to off-site storage\n'
fi

find "$BACKUP_DIR" -maxdepth 1 -type f \
  \( -name 'postgres-*.dump.age' -o -name 'postgres-*.dump.age.sha256' \) \
  -mtime "+$BACKUP_RETENTION_DAYS" -delete

trap - EXIT
printf 'backup-postgres: completed %s (%s bytes)\n' "$backup_name" "$(stat --format '%s' "$backup_path")"
