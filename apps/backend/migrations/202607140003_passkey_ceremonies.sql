ALTER TABLE webauthn_credentials
    ADD COLUMN credential jsonb NOT NULL DEFAULT '{}'::jsonb;

ALTER TABLE webauthn_credentials
    ALTER COLUMN credential DROP DEFAULT;

CREATE TABLE webauthn_ceremonies (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    session_id uuid REFERENCES sessions(id) ON DELETE CASCADE,
    kind text NOT NULL CHECK (kind IN ('registration', 'authentication')),
    state jsonb NOT NULL,
    nickname text,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    CHECK (expires_at > created_at),
    CHECK (
        (kind = 'registration' AND session_id IS NOT NULL AND nickname IS NOT NULL)
        OR (kind = 'authentication' AND session_id IS NULL AND nickname IS NULL)
    )
);

CREATE INDEX webauthn_ceremonies_expiry_idx ON webauthn_ceremonies (expires_at);
CREATE INDEX webauthn_credentials_user_idx ON webauthn_credentials (user_id);
