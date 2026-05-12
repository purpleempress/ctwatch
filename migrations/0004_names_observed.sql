CREATE TABLE names_observed (
    name              TEXT        PRIMARY KEY,
    registered_domain TEXT        NOT NULL,
    first_seen        TIMESTAMPTZ NOT NULL,
    last_seen         TIMESTAMPTZ NOT NULL,
    cert_count        INTEGER     NOT NULL DEFAULT 1
);

CREATE INDEX names_observed_regdom_lastseen ON names_observed (registered_domain, last_seen DESC);
