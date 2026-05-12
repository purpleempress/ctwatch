CREATE TABLE watchlist (
    domain        TEXT        PRIMARY KEY,
    added_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    notes         TEXT
);
