#!/usr/bin/env bash

set -Eeuo pipefail

umask 077

DEPLOY_ROOT=${DEPLOY_ROOT:-/opt/wasm-pdf-editor}
DEPLOY_ENV_FILE=${DEPLOY_ENV_FILE:-/etc/wasm-pdf-editor/deployment.env}
DEPLOY_REQUIRE_BACKUP=${DEPLOY_REQUIRE_BACKUP:-true}
DEPLOY_HEALTH_ATTEMPTS=${DEPLOY_HEALTH_ATTEMPTS:-30}
DEPLOY_HEALTH_INTERVAL_SECONDS=${DEPLOY_HEALTH_INTERVAL_SECONDS:-5}
DEPLOY_KEEP_RELEASES=${DEPLOY_KEEP_RELEASES:-5}
DEPLOY_DRY_RUN=${DEPLOY_DRY_RUN:-false}

fail() {
  printf 'deploy-vps: %s\n' "$*" >&2
  exit 1
}

archive_path=${1:-}
archive_checksum=${2:-}
revision=${3:-}
backend_image=${4:-}
frontend_image=${5:-}

[[ -n $archive_path && -n $archive_checksum && -n $revision && -n $backend_image && -n $frontend_image ]] || \
  fail "usage: $0 ARCHIVE SHA256 REVISION BACKEND_IMAGE FRONTEND_IMAGE"
[[ -r $archive_path ]] || fail "release archive is not readable: $archive_path"
[[ $archive_checksum =~ ^[0-9a-f]{64}$ ]] || fail "release checksum must be lowercase SHA-256"
[[ $revision =~ ^[0-9a-f]{40}$ ]] || fail "revision must be a full 40-character Git SHA"
[[ $backend_image =~ ^[a-z0-9./_-]+:sha-[0-9a-f]{40}$ ]] || fail "backend image must use an immutable sha-REVISION tag"
[[ $frontend_image =~ ^[a-z0-9./_-]+:sha-[0-9a-f]{40}$ ]] || fail "frontend image must use an immutable sha-REVISION tag"
[[ $backend_image == *":sha-$revision" ]] || fail "backend image tag does not match revision"
[[ $frontend_image == *":sha-$revision" ]] || fail "frontend image tag does not match revision"
for command_name in curl docker flock sha256sum tar; do
  command -v "$command_name" >/dev/null 2>&1 || fail "required command not found: $command_name"
done

actual_checksum=$(sha256sum "$archive_path" | awk '{print $1}')
[[ $actual_checksum == "$archive_checksum" ]] || fail "release archive checksum does not match"
[[ -r $DEPLOY_ENV_FILE ]] || fail "deployment environment is not readable: $DEPLOY_ENV_FILE"
environment_owner=$(stat --format '%u' "$DEPLOY_ENV_FILE")
environment_mode=$(stat --format '%a' "$DEPLOY_ENV_FILE")
[[ $environment_owner == 0 || $environment_owner == "$EUID" ]] || fail "deployment environment has an unexpected owner"
(( (8#$environment_mode & 8#022) == 0 )) || fail "deployment environment must not be group/world writable"

set -a
# shellcheck disable=SC1090
source "$DEPLOY_ENV_FILE"
set +a

: "${PUBLIC_DOMAIN:?PUBLIC_DOMAIN is required in the deployment environment}"
: "${SMTP_HOST:?SMTP_HOST is required in the deployment environment}"
: "${SMTP_USERNAME:?SMTP_USERNAME is required in the deployment environment}"
: "${SMTP_FROM_ADDRESS:?SMTP_FROM_ADDRESS is required in the deployment environment}"
[[ ${#PUBLIC_DOMAIN} -le 253 && $PUBLIC_DOMAIN =~ ^[a-zA-Z0-9][a-zA-Z0-9.-]*[a-zA-Z0-9]$ ]] || \
  fail "PUBLIC_DOMAIN is not a valid DNS hostname"
[[ $DEPLOY_HEALTH_ATTEMPTS =~ ^[1-9][0-9]*$ ]] || fail "DEPLOY_HEALTH_ATTEMPTS must be positive"
[[ $DEPLOY_HEALTH_INTERVAL_SECONDS =~ ^[1-9][0-9]*$ ]] || fail "DEPLOY_HEALTH_INTERVAL_SECONDS must be positive"
[[ $DEPLOY_KEEP_RELEASES =~ ^[2-9]$|^[1-9][0-9]+$ ]] || fail "DEPLOY_KEEP_RELEASES must be at least 2"
[[ $DEPLOY_REQUIRE_BACKUP == true || $DEPLOY_REQUIRE_BACKUP == false ]] || \
  fail "DEPLOY_REQUIRE_BACKUP must be true or false"
[[ $DEPLOY_DRY_RUN == true || $DEPLOY_DRY_RUN == false ]] || fail "DEPLOY_DRY_RUN must be true or false"

BACKEND_IMAGE=$backend_image
FRONTEND_IMAGE=$frontend_image
export BACKEND_IMAGE FRONTEND_IMAGE

install -d -m 0750 "$DEPLOY_ROOT" "$DEPLOY_ROOT/releases" "$DEPLOY_ROOT/incoming"
exec 9>"$DEPLOY_ROOT/.deploy.lock"
flock -n 9 || fail "another deployment is already running"

staging_dir="$DEPLOY_ROOT/incoming/$revision"
release_dir="$DEPLOY_ROOT/releases/$revision"
rm -rf -- "$staging_dir"
install -d -m 0750 "$staging_dir"

cleanup_staging() {
  rm -rf -- "$staging_dir"
}
trap cleanup_staging EXIT

while IFS= read -r entry; do
  [[ -n $entry ]] || continue
  [[ $entry != / && $entry != /* && $entry != .. && $entry != ../* && $entry != */../* && $entry != *'/..' ]] || \
    fail "release archive contains an unsafe path"
done < <(tar --list --gzip --file "$archive_path")

tar --extract --gzip --file "$archive_path" --directory "$staging_dir" \
  --no-same-owner --no-same-permissions

[[ -z $(find "$staging_dir" -type l -print -quit) ]] || fail "release archive must not contain symbolic links"

for required_path in \
  docker-compose.yml \
  compose.production.yml \
  infra/caddy/Caddyfile \
  scripts/backup-postgres.sh; do
  [[ -f $staging_dir/$required_path ]] || fail "release is missing regular file $required_path"
done

printf '%s\n' "$archive_checksum" >"$staging_dir/.release-archive.sha256"
printf 'BACKEND_IMAGE=%q\nFRONTEND_IMAGE=%q\n' "$backend_image" "$frontend_image" \
  >"$staging_dir/.deployment-images"

compose_for() {
  local directory=$1
  shift
  docker compose \
    --project-name wasm-pdf-editor \
    --project-directory "$directory" \
    --file "$directory/docker-compose.yml" \
    --file "$directory/compose.production.yml" \
    "$@"
}

compose_for "$staging_dir" config --quiet

if [[ $DEPLOY_DRY_RUN == true ]]; then
  rm -rf -- "$staging_dir"
  trap - EXIT
  printf 'deploy-vps: dry-run validated release %s\n' "$revision"
  exit 0
fi

compose_for "$staging_dir" pull

previous_release=
if [[ -L $DEPLOY_ROOT/current ]]; then
  previous_release=$(readlink -f -- "$DEPLOY_ROOT/current")
fi

postgres_running=false
if [[ -n $previous_release && -d $previous_release ]]; then
  postgres_id=$(compose_for "$previous_release" ps --quiet postgres)
  if [[ -n $postgres_id && $(docker inspect --format '{{.State.Running}}' "$postgres_id") == true ]]; then
    postgres_running=true
  fi
fi

if [[ $postgres_running == true && $DEPLOY_REQUIRE_BACKUP == true ]]; then
  printf 'deploy-vps: creating mandatory pre-deployment backup\n'
  sudo -n systemctl start wasm-pdf-backup.service
fi

if [[ -e $release_dir ]]; then
  [[ -f $release_dir/.release-archive.sha256 ]] || fail "existing release has no checksum metadata"
  [[ $(<"$release_dir/.release-archive.sha256") == "$archive_checksum" ]] || \
    fail "existing release checksum differs"
  rm -rf -- "$staging_dir"
else
  mv -- "$staging_dir" "$release_dir"
fi
trap - EXIT

ln -sfn -- "$release_dir" "$DEPLOY_ROOT/current.next"
mv -T -- "$DEPLOY_ROOT/current.next" "$DEPLOY_ROOT/current"

rollout_ok=true
if ! compose_for "$release_dir" up -d --wait --wait-timeout 180 --remove-orphans; then
  rollout_ok=false
fi

if [[ $rollout_ok == true ]]; then
  rollout_ok=false
  for ((attempt = 1; attempt <= DEPLOY_HEALTH_ATTEMPTS; attempt++)); do
    if curl --fail --silent --show-error \
      --resolve "$PUBLIC_DOMAIN:443:127.0.0.1" \
      "https://$PUBLIC_DOMAIN/health/ready" >/dev/null; then
      rollout_ok=true
      break
    fi
    sleep "$DEPLOY_HEALTH_INTERVAL_SECONDS"
  done
fi

if [[ $rollout_ok != true ]]; then
  printf 'deploy-vps: rollout failed; restoring previous release\n' >&2
  if [[ -n $previous_release && -d $previous_release && -r $previous_release/.deployment-images ]]; then
    set -a
    # shellcheck disable=SC1090,SC1091
    source "$previous_release/.deployment-images"
    set +a
    export BACKEND_IMAGE FRONTEND_IMAGE
    ln -sfn -- "$previous_release" "$DEPLOY_ROOT/current.next"
    mv -T -- "$DEPLOY_ROOT/current.next" "$DEPLOY_ROOT/current"
    compose_for "$previous_release" up -d --wait --wait-timeout 180 --remove-orphans || true
  fi
  fail "release $revision did not become healthy"
fi

mapfile -t old_releases < <(
  find "$DEPLOY_ROOT/releases" -mindepth 1 -maxdepth 1 -type d -printf '%T@ %p\n' \
    | sort --numeric-sort --reverse \
    | awk -v keep="$DEPLOY_KEEP_RELEASES" 'NR > keep { sub(/^[^ ]+ /, ""); print }'
)
for old_release in "${old_releases[@]}"; do
  [[ $old_release != "$previous_release" && $old_release != "$release_dir" ]] || continue
  rm -rf -- "$old_release"
done

rm -f -- "$archive_path"
printf 'deploy-vps: release %s is healthy\n' "$revision"
