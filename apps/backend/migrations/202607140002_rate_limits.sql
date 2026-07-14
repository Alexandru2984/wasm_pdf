CREATE TABLE rate_limit_buckets (
    scope_hash bytea NOT NULL CHECK (octet_length(scope_hash) = 32),
    category text NOT NULL,
    window_start timestamptz NOT NULL,
    request_count integer NOT NULL CHECK (request_count > 0),
    expires_at timestamptz NOT NULL,
    PRIMARY KEY (scope_hash, category, window_start),
    CHECK (expires_at > window_start)
);

CREATE INDEX rate_limit_buckets_expiry_idx ON rate_limit_buckets (expires_at);
