CREATE INDEX sessions_expiry_cleanup_idx
    ON sessions (LEAST(expires_at, idle_expires_at));

CREATE INDEX sessions_revoked_cleanup_idx
    ON sessions (revoked_at)
    WHERE revoked_at IS NOT NULL;

CREATE INDEX account_tokens_expiry_cleanup_idx
    ON account_tokens (expires_at);

CREATE INDEX account_tokens_consumed_cleanup_idx
    ON account_tokens (consumed_at)
    WHERE consumed_at IS NOT NULL;

CREATE INDEX audit_events_cleanup_idx ON audit_events (created_at);
