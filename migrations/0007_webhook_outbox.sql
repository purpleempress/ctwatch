CREATE TABLE webhook_outbox (
    id            BIGSERIAL   PRIMARY KEY,
    event_id      UUID        NOT NULL UNIQUE,
    body          JSONB       NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    attempts      INTEGER     NOT NULL DEFAULT 0,
    last_attempt  TIMESTAMPTZ,
    last_status   INTEGER,
    delivered_at  TIMESTAMPTZ,
    next_attempt  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX webhook_outbox_due ON webhook_outbox (next_attempt) WHERE delivered_at IS NULL;
