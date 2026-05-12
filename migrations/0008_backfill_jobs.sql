CREATE TABLE backfill_jobs (
    id                  BIGSERIAL   PRIMARY KEY,
    log_id              BYTEA       NOT NULL,
    target_since        TIMESTAMPTZ NOT NULL,
    target_start_index  BIGINT      NOT NULL,
    target_end_index    BIGINT      NOT NULL,
    progress_index      BIGINT      NOT NULL,
    started_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at        TIMESTAMPTZ,
    status              TEXT        NOT NULL DEFAULT 'running',
    last_error          TEXT
);

CREATE UNIQUE INDEX backfill_jobs_unique_active
    ON backfill_jobs (log_id, target_since)
    WHERE status = 'running';
