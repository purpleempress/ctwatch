CREATE TABLE cert_names (
    name              TEXT        NOT NULL,
    registered_domain TEXT        NOT NULL,
    cert_hash         BYTEA       NOT NULL,
    not_before        TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (name, cert_hash, not_before)
);

SELECT create_hypertable('cert_names', 'not_before', chunk_time_interval => INTERVAL '7 days');

CREATE INDEX cert_names_regdom_seen ON cert_names (registered_domain, not_before DESC);
CREATE INDEX cert_names_name_trgm   ON cert_names USING gin (name gin_trgm_ops);

ALTER TABLE cert_names SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'registered_domain',
    timescaledb.compress_orderby   = 'not_before DESC, name'
);
