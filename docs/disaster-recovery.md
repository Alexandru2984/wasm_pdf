# Database backup and disaster recovery

PostgreSQL is the system of record for accounts, sessions, MFA credentials,
audit events and the email outbox. Production backups use PostgreSQL's custom
dump format and are streamed directly into `age`; an unencrypted dump is never
written to host storage. The native `age` recipient uses X25519 for key
agreement, HKDF-SHA-256 for key derivation and ChaCha20-Poly1305 for authenticated
payload encryption.

## Recovery objectives

The supplied daily timer gives an RPO of at most 24 hours. The initial RTO
target is 60 minutes, including provisioning PostgreSQL, downloading a backup,
restoring it and validating application readiness. Lower RPO requires WAL
archiving and point-in-time recovery; that is a separate operational milestone.

Backups on the application VPS are not sufficient. Configure encrypted upload
to a versioned R2/S3 bucket in another failure domain, with object lock or an
immutable retention policy where available. Keep the `age` private identity
offline or on a dedicated recovery host; the application VPS needs only its
public recipient.

## One-time key setup

Install `age`, Docker Compose and, for off-site upload, AWS CLI v2. Generate the
identity on a trusted workstation, not on the VPS:

```bash
umask 077
age-keygen -o backup-identity.age
age-keygen -y backup-identity.age > backup-recipient.txt
```

Store `backup-identity.age` in an offline password manager or hardware-backed
secret store. Copy only `backup-recipient.txt` to
`/etc/wasm-pdf-editor/backup.age-recipient` on the VPS. The backup service runs
as `deploy`, so install the public recipient as that user:

```bash
sudo install -o deploy -g deploy -m 0600 backup-recipient.txt \
  /etc/wasm-pdf-editor/backup.age-recipient
```

Copy `infra/examples/backup.env.example` to
`/etc/wasm-pdf-editor/backup.env` and replace every placeholder:

```dotenv
BACKUP_DIR=/var/backups/wasm-pdf-editor/postgres
BACKUP_RETENTION_DAYS=14
BACKUP_AGE_RECIPIENT_FILE=/etc/wasm-pdf-editor/backup.age-recipient
BACKUP_REQUIRE_OFFSITE=true
BACKUP_S3_URI=s3://pdf-editor-production-backups/postgres
BACKUP_S3_ENDPOINT=https://ACCOUNT_ID.r2.cloudflarestorage.com
AWS_ACCESS_KEY_ID=replace-with-scoped-key-id
AWS_SECRET_ACCESS_KEY=replace-with-scoped-secret
AWS_DEFAULT_REGION=auto
```

The bucket credential must be limited to the backup prefix. Prefer a bucket
lifecycle longer than the local retention and enable versioning/immutability.
Do not commit either environment file.
Keep `/etc/wasm-pdf-editor/backup.env` owned by root with mode `0600`; systemd
loads it before dropping privileges to the deployment user.

Install and enable the systemd timer:

```bash
sudo install -m 0644 infra/systemd/wasm-pdf-backup.service /etc/systemd/system/
sudo install -m 0644 infra/systemd/wasm-pdf-backup.timer /etc/systemd/system/
sudo install -d -o deploy -g deploy -m 0700 /var/backups/wasm-pdf-editor/postgres
sudo systemctl daemon-reload
sudo systemctl enable --now wasm-pdf-backup.timer
sudo systemctl start wasm-pdf-backup.service
sudo systemctl status wasm-pdf-backup.service
```

## Restore verification drill

Every backup has a SHA-256 sidecar. Integrity alone does not prove that a dump
can be restored. At least monthly, copy an encrypted backup and its sidecar to
a controlled recovery machine, start the application PostgreSQL service, and
perform a full disposable restore:

```bash
export BACKUP_AGE_IDENTITY_FILE=$PWD/backup-identity.age
scripts/verify-postgres-backup.sh /secure/path/postgres-YYYYMMDDTHHMMSSZ.dump.age
```

The verifier checks the encrypted checksum, creates a randomly named database,
restores the complete dump in one transaction, verifies successful SQLx
migrations and removes the temporary database. Record the date, backup name,
duration and result outside the application VPS.

## Disaster restore

First verify the selected backup. Stop all backend writers before replacing the
production database. The restore command deliberately requires the exact
database name as a confirmation token:

```bash
docker compose -p wasm-pdf-editor -f docker-compose.yml stop backend
export BACKUP_AGE_IDENTITY_FILE=/secure/path/backup-identity.age
scripts/verify-postgres-backup.sh /secure/path/postgres-YYYYMMDDTHHMMSSZ.dump.age
scripts/restore-postgres.sh \
  /secure/path/postgres-YYYYMMDDTHHMMSSZ.dump.age \
  restore:pdf_editor
docker compose -f docker-compose.yml -f compose.production.yml up -d --remove-orphans
curl --fail https://"$PUBLIC_DOMAIN"/health/ready
```

The production restore drops and recreates `POSTGRES_DB`, so it refuses to run
while the backend container is active. Never rehearse that command against the
production database; use the disposable verifier for routine drills.
