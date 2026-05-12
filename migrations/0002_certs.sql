CREATE TABLE certs (
    cert_hash          BYTEA       NOT NULL,
    issuer_hash        BYTEA       NOT NULL,
    issuer_cn          TEXT        NOT NULL,
    not_before         TIMESTAMPTZ NOT NULL,
    not_after          TIMESTAMPTZ NOT NULL,
    sans               TEXT[]      NOT NULL,
    registered_domains TEXT[]      NOT NULL,
    first_seen         TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (cert_hash, not_before)
);

SELECT create_hypertable('certs', 'not_before', chunk_time_interval => INTERVAL '7 days');

CREATE INDEX certs_registered_domains_gin ON certs USING gin (registered_domains);
CREATE INDEX certs_issuer_hash             ON certs (issuer_hash, not_before DESC);

ALTER TABLE certs SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'registered_domains',
    timescaledb.compress_orderby   = 'not_before DESC, cert_hash'
);
