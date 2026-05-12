-- TimescaleDB warns:
--   column "cert_hash" should be used for segmenting or ordering
-- because cert_names.PRIMARY KEY is (name, cert_hash, not_before) but the
-- original compress_orderby only listed (not_before, name). Adding cert_hash to
-- the orderby silences the warning. Affects future compressions only; existing
-- compressed chunks keep their old ordering.
ALTER TABLE cert_names SET (
    timescaledb.compress_orderby = 'not_before DESC, name, cert_hash'
);
