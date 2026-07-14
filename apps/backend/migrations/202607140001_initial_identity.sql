CREATE TABLE users (
    id uuid PRIMARY KEY,
    email text NOT NULL,
    email_normalized text NOT NULL UNIQUE,
    display_name text NOT NULL,
    password_hash text NOT NULL,
    status text NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'locked', 'disabled', 'pending_verification')),
    mfa_required boolean NOT NULL DEFAULT false,
    token_version integer NOT NULL DEFAULT 1 CHECK (token_version > 0),
    failed_login_attempts integer NOT NULL DEFAULT 0 CHECK (failed_login_attempts >= 0),
    locked_until timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    last_login_at timestamptz
);

CREATE TABLE sessions (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash bytea NOT NULL UNIQUE CHECK (octet_length(token_hash) = 32),
    csrf_token_hash bytea NOT NULL CHECK (octet_length(csrf_token_hash) = 32),
    user_agent text,
    ip_address inet,
    created_at timestamptz NOT NULL DEFAULT now(),
    last_seen_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    idle_expires_at timestamptz NOT NULL,
    revoked_at timestamptz,
    CHECK (expires_at > created_at),
    CHECK (idle_expires_at > created_at)
);

CREATE INDEX sessions_user_active_idx
    ON sessions (user_id, expires_at)
    WHERE revoked_at IS NULL;

CREATE TABLE webauthn_credentials (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    credential_id bytea NOT NULL UNIQUE,
    public_key bytea NOT NULL,
    sign_count bigint NOT NULL DEFAULT 0 CHECK (sign_count >= 0),
    transports text[] NOT NULL DEFAULT '{}',
    nickname text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    last_used_at timestamptz
);

CREATE TABLE backup_codes (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash bytea NOT NULL UNIQUE CHECK (octet_length(code_hash) = 32),
    created_at timestamptz NOT NULL DEFAULT now(),
    used_at timestamptz
);

CREATE INDEX backup_codes_user_unused_idx
    ON backup_codes (user_id)
    WHERE used_at IS NULL;

CREATE TABLE audit_events (
    id uuid PRIMARY KEY,
    user_id uuid REFERENCES users(id) ON DELETE SET NULL,
    session_id uuid REFERENCES sessions(id) ON DELETE SET NULL,
    event_type text NOT NULL,
    outcome text NOT NULL CHECK (outcome IN ('success', 'failure')),
    ip_address inet,
    user_agent text,
    metadata jsonb NOT NULL DEFAULT '{}',
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX audit_events_user_created_idx ON audit_events (user_id, created_at DESC);
CREATE INDEX audit_events_type_created_idx ON audit_events (event_type, created_at DESC);
