SELECT add_compression_policy('certs',      INTERVAL '7 days');
SELECT add_compression_policy('cert_names', INTERVAL '7 days');

SELECT add_retention_policy('certs',      INTERVAL '90 days');
SELECT add_retention_policy('cert_names', INTERVAL '90 days');

-- names_observed pruning happens in code (see writer.rs scheduler in Phase 5),
-- not via TimescaleDB policy, because it isn't a hypertable.
