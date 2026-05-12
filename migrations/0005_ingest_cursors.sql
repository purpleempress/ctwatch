CREATE TABLE ingest_cursors (
    log_id         BYTEA       PRIMARY KEY,
    log_url        TEXT        NOT NULL,
    operator       TEXT        NOT NULL,
    last_tree_size BIGINT      NOT NULL DEFAULT 0,
    last_sth_raw   BYTEA,
    last_updated   TIMESTAMPTZ NOT NULL DEFAULT now(),
    state          TEXT        NOT NULL DEFAULT 'usable'
);
