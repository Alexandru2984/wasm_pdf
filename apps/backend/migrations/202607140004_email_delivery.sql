ALTER TABLE users ADD COLUMN email_verified_at timestamptz;

-- Accounts created before email delivery existed retain their established state.
UPDATE users SET email_verified_at = created_at;

CREATE TABLE account_tokens (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    purpose text NOT NULL CHECK (purpose IN ('verify_email', 'reset_password')),
    expires_at timestamptz NOT NULL,
    consumed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK (expires_at > created_at)
);

CREATE INDEX account_tokens_active_idx
    ON account_tokens (user_id, purpose, expires_at)
    WHERE consumed_at IS NULL;

CREATE TABLE email_outbox (
    id uuid PRIMARY KEY,
    account_token_id uuid NOT NULL REFERENCES account_tokens(id) ON DELETE CASCADE,
    recipient text NOT NULL,
    recipient_name text NOT NULL,
    template text NOT NULL CHECK (template IN ('verify_email', 'reset_password')),
    status text NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'processing', 'sent', 'dead')),
    attempts integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    available_at timestamptz NOT NULL DEFAULT now(),
    claimed_at timestamptz,
    sent_at timestamptz,
    last_error text,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX email_outbox_dispatch_idx
    ON email_outbox (available_at, created_at)
    WHERE status = 'queued';
