# ctwatch — design

**Status:** Approved 2026-05-12, ready for implementation plan.

**Repository (planned):** standalone — `github.com/purpleempress/ctwatch`.
**License (planned):** MIT.
**Working name:** `ctwatch`. Rename before first public release if a better name surfaces.

---

## 1. What this is

`ctwatch` is a self-hosted service that maintains a **90-day rolling, SAN-indexed mirror of public Certificate Transparency**, plus a **400-day rolling unique-name index**. It exposes an HTTP API for:

- **Cert history:** `domain → list of certs observed under it in the past 90 days`
- **Subdomain discovery:** `domain → list of unique SANs seen under it in the past 400 days`
- **Watchlist alerts:** webhook fires whenever a new precert names a domain on a configurable watchlist
- **Live stream:** WebSocket fan-out of newly-observed precerts (optionally filtered by registered_domain)
- **Stats:** JSON snapshot of ingest rate, log lag, totals — designed for direct consumption by amber.systems datapoints (poll every ~30s)
- **Backfill:** `ctwatch backfill --since=<RFC3339>` populates the historical window without waiting 90 days for the rolling window to fill

It is designed to run on a single VM with a single Postgres + TimescaleDB instance, with low operational overhead.

## 2. Why this exists

The CT ecosystem has data sources, but the gap is **"self-hosted, queryable, low-friction"**:

| Existing tool | Shape | Gap |
|---|---|---|
| `crt.sh` | Authoritative, full history, HTTP query API | Sectigo-operated, no SLA, can't customize, rate-limited, single point of dependency |
| Cert Spotter | Per-domain monitoring (paid + free tier) | Not a queryable mirror; you can't ask "show me every cert under example.com" without paying |
| `certstream-server` | WebSocket fan-out over live CT entries | Doesn't store anything; ephemeral; Calidog instance is degraded (verified 2026-05-12: connection succeeds but no frames flow) |
| Sectigo's monitor / DigiCert's CT Search | Enterprise-tier subscriptions | Pricing, integration friction, lock-in |
| roll-your-own | A weekend project per researcher | Each one re-discovers the same edge cases; no shared tooling |

`ctwatch` aims to be the simple, opinionated, MIT-licensed default for self-hosted CT mirroring. One Docker image plus a Postgres, you have your own crt.sh-style read API. Adding it to a stack is one binding away.

## 3. Non-goals

- **Not a full cert mirror.** We don't store DER bodies. The cert hash is the handle; refetching the body delegates to `crt.sh` on demand.
- **Not multi-region.** One host, one DB. The data is rebuildable from upstream logs, so HA isn't worth the operational cost. Multi-region is a fork-it move.
- **Not a web UI.** Pure HTTP API. UIs are downstream consumer projects.
- **Not enterprise-grade CT monitor.** We don't validate SCT signatures on every entry or verify Merkle inclusion proofs. We trust the log operators; the BRs and Chrome trust them too.
- **Not auth/authz.** The service expects to live behind a reverse proxy or service-binding that enforces who can call which endpoints. Single-tenant by default.
- **Not rate limiting.** Same — reverse proxy concern.

> The WebSocket endpoint (§8.6) provides a Certstream-shaped live stream, but only as a thin tail of the writer. It is not a Certstream replacement at the protocol level — no full-message fidelity to Calidog's schema, no historical replay. It is a best-effort firehose for live consumers, complementary to the durable webhook path.

## 4. Architecture

```
┌────────────────────────────────────────────────────────────────────┐
│                           ctwatch                                   │
│                                                                     │
│  ┌──────────────────────┐    ┌─────────────────────────────────┐   │
│  │  log_list refresh    │    │  ingest workers (N async tasks) │   │
│  │  (every 6h)          │───▶│  one per "usable" log shard     │   │
│  └──────────────────────┘    │  poll get-sth → get-entries     │   │
│                              │  parse → extract SANs → enqueue │   │
│                              └───────────────┬─────────────────┘   │
│                                              │                      │
│  ┌──────────────────────┐    ┌───────────────▼─────────────────┐   │
│  │  watchlist source    │───▶│  writer (single async task)     │   │
│  │  (db/file/url)       │    │  dedup by cert_hash             │───┐│
│  └──────────────────────┘    │  insert certs + cert_names      │   ││
│                              │  upsert names_observed          │   ││
│                              │  check watchlist → enqueue alert│   ││
│                              │  broadcast to stream subscribers│   ││
│                              └───────────────┬─────────────────┘   ││
│                                              │                      ││
│  ┌──────────────────────┐    ┌───────────────▼─────────────────┐   ││
│  │  HTTP server (axum)  │    │  Postgres 16 + TimescaleDB 2.x  │   ││
│  │  /v1/lookup          │◀───┤  hypertables: certs, cert_names │   ││
│  │  /v1/certs           │    │  plain: names_observed,         │   ││
│  │  /v1/cert/{hash}     │    │         ingest_cursors,         │   ││
│  │  /v1/watchlist/...   │    │         watchlist (if db mode), │   ││
│  │  /v1/stats           │    │         webhook_outbox,         │   ││
│  │  /v1/stream  (WS) ◀──┼────┼─────────────────────────────────│   ││
│  │  /v1/logs            │    │         backfill_jobs           │   ││
│  │  /healthz, /metrics  │    └─────────────────────────────────┘   ││
│  └──────────────────────┘                                          ││
│                                                                    ││
│  ┌──────────────────────────────────────────────────────────────┐ ││
│  │  webhook dispatcher                                          │◀┘│
│  │  HMAC-signed POST to configured URL on watchlist match       │  │
│  │  exponential backoff, persistent retry queue                 │  │
│  └──────────────────────────────────────────────────────────────┘  │
│                                                                     │
│  ┌──────────────────────────────────────────────────────────────┐  │
│  │  backfill worker (ctwatch backfill subcommand)               │  │
│  │  binary-searches each log for entry index ≥ since,           │  │
│  │  then walks forward writing to the same tables               │  │
│  └──────────────────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────────┘
                                                  │
                                                  ▼
                                ┌────────────────────────────────┐
                                │  consumer (your app, amber,    │
                                │  a webhook receiver, a WS      │
                                │  subscriber, etc.)             │
                                └────────────────────────────────┘
```

All within a single Rust binary. State is in Postgres. No background queues outside the process (the webhook retry queue and backfill job state live in Postgres tables for durability).

## 5. Tech stack

| Layer | Choice | Reason |
|---|---|---|
| Language | Rust, MSRV 1.82 | Sum types model CT entry variants cleanly; `Result`-based error handling; long-term maintenance friendliness; single static binary via `musl`. 1.82 matches `axum` 0.7 / `sqlx` 0.8 floors. |
| Runtime | `tokio` | Async fits the many-logs-polling-in-parallel pattern; mature ecosystem. |
| HTTP server | `axum` | tokio-native, low boilerplate, good for the small read API; first-class WebSocket support via `axum::extract::ws`. |
| HTTP client | `reqwest` (rustls) | For CT log polling, crt.sh fallback, webhook delivery. |
| Database | Postgres 16+ with TimescaleDB 2.x | Columnar compression on time-series tables; native retention policies; rich text/array indexes. |
| Postgres driver | `sqlx` (rustls, runtime-tokio) | Compile-time-checked queries via `query!` macros; async; supports migrations. |
| FS hint | xfs or btrfs with `chattr +C` on PG data dir | Filesystem compression conflicts with COW and PG's WAL pattern. TimescaleDB does compression at the data layer. |
| Cert parsing | `x509-parser` | Mature ASN.1 + X.509 parser; sufficient for TBS-of-precert + SANs + poison-extension detection. |
| CT protocol | hand-rolled (~300 LOC) | RFC 6962 client is a thin layer over `reqwest`. No good single canonical crate; rolling it in keeps the dep surface small. |
| Public Suffix List | `publicsuffix` crate | eTLD+1 computation for `registered_domain`. |
| Serialization | `serde` + `serde_json` + `base64` | For both API surface and CT entry decoding. |
| CLI | `clap` (derive macros) | `ctwatch serve`, `ctwatch migrate`, `ctwatch backfill`, `ctwatch healthcheck` subcommands. |
| Observability | `tracing` + `tracing-subscriber` (JSON) + `metrics` + `metrics-exporter-prometheus` | Structured logs to stdout, Prometheus exposition on `/v1/metrics`. |
| Config | `figment` (env + YAML merge, 12-factor) | Env overrides file overrides defaults. |
| Tests | stdlib `#[test]` + `#[tokio::test]` + `#[sqlx::test]` for per-test DB | Real Postgres in integration tests via `testcontainers-rs` if needed. |
| CI / releases | GitHub Actions + `cargo-dist` | Multi-arch binaries (`x86_64-musl`, `aarch64-musl`, `aarch64-darwin`) + multi-arch Docker images to `ghcr.io`. |

No Elixir, no Kafka, no Redis, no Vault. Just Rust + Postgres.

## 6. Ingest topology

**One ingest path, fully self-contained.** No upstream dependency on Certstream.

### 6.1 Log discovery

At startup and every 6 hours thereafter, fetch the Chrome-trusted log list:

```
GET https://www.gstatic.com/ct/log_list/v3/log_list.json
```

Filter to logs in state `usable` and within a configurable operator allowlist (default: all). The list returns `log_id` (sha256 of public key), URL, and operator name.

This **self-updates as logs come online**. We don't hardcode URLs — when 2027 logs go live, ctwatch picks them up automatically; when 2025 logs decommission, it stops polling them. The only operator decisions are which set to follow (default: all `usable` logs across all operators).

Result: typically 15–25 active log shards per epoch in 2026.

### 6.2 Per-log worker

One async task per active log (spawned via `tokio::spawn`):

1. Read `ingest_cursors.last_tree_size` for this log_id from Postgres
2. `GET /ct/v1/get-sth` — if `tree_size` advanced, proceed
3. `GET /ct/v1/get-entries?start=<cursor>&end=<min(cursor+999, tree_size-1)>`
4. Parse each entry's `MerkleTreeLeaf`
5. For each entry, decide:
   - If `LogEntryType == precert_entry` → keep
   - Else (final cert) → **drop** (we only store precerts; see §6.4)
6. Hand parsed precerts to the writer channel
7. After successful write batch, update `ingest_cursors.last_tree_size` to the high watermark
8. Sleep `poll_interval_seconds` (default 30), repeat

If a log returns 5xx or times out, exponential backoff up to 5 minutes between retries. After 1 hour of failure, mark the log as `degraded` in metrics; keep retrying.

### 6.3 Writer

Single async task, `tokio::sync::mpsc` channel (buffer 10k entries), serializes writes to Postgres:

```
for entry := range queue:
    // dedupe — same cert may appear in multiple logs
    INSERT INTO certs (cert_hash, ...) VALUES (...)
        ON CONFLICT (cert_hash, not_before) DO NOTHING
    INSERT INTO cert_names (name, registered_domain, cert_hash, not_before)
        VALUES (...)
        ON CONFLICT DO NOTHING
    INSERT INTO names_observed (name, registered_domain, first_seen, last_seen, cert_count)
        VALUES (...)
        ON CONFLICT (name) DO UPDATE
        SET last_seen = excluded.last_seen, cert_count = names_observed.cert_count + 1
    // watchlist check
    if any registered_domain in watchlist:
        INSERT INTO webhook_outbox (event_id, body, attempts)
            VALUES (...)
    // live stream fan-out (lossy; non-blocking)
    let _ = stream_broadcast.send(CertEvent { ... });
```

Batches of ~100 entries per transaction. The writer is the only path that writes to PG.

The stream broadcast uses `tokio::sync::broadcast` (capacity 1024). Lagging subscribers receive `RecvError::Lagged` and are dropped — the writer never waits.

### 6.4 Why precerts only (no finals)

The CA/Browser Forum Baseline Requirements (§3.2.2.4, §4.2) require domain validation **before** the precert is submitted to CT. A precert is therefore a signed receipt from the CA that says "this name was validated as belonging to whoever submitted the CSR." The final cert is the same SANs minutes (or milliseconds) later. Storing both doubles disk for ~zero additional signal.

Edge case: some CAs deliver SCTs via OCSP or TLS extension and never submit a precert. This is essentially extinct for browser-trusted issuance in 2026 (Let's Encrypt, DigiCert, Sectigo, Google Trust Services, ZeroSSL, GlobalSign all use embedded-SCT precert flows). Estimated coverage loss: <1%. Acceptable.

### 6.5 Backfill

The live ingest path (§6.2) starts from the current STH. To populate the 90-day hot tier without waiting 90 days, `ctwatch backfill` walks each log backward from the current STH to a target timestamp, then writes entries forward through the same writer pipeline.

```
ctwatch backfill --since 2026-02-12T00:00:00Z [--log <log_id>] [--concurrency 4]
```

Algorithm, per log:

1. **Find the entry-index for `--since`.** Binary search over `[0, tree_size)`:
   - Fetch `get-entries?start=mid&end=mid` (a single entry)
   - Parse `MerkleTreeLeaf.TimestampedEntry.timestamp` (millis since epoch — the SCT timestamp, not `not_before`; we use this because it's exposed in the leaf without parsing the cert, ~5x faster)
   - If `leaf_timestamp >= since`: search lower half. Else: upper half.
   - Converges in ~log2(tree_size) ≈ 30–35 GETs per log. Cached per-log.
2. **Write a `backfill_jobs` row** recording `(log_id, target_start_index, target_end_index, progress_index, started_at, status)`. Resumable across crashes.
3. **Drive `get-entries` batches** from `target_start_index` forward at the log's allowed rate (`backfill_rate_limit_per_log_qps`, default 5). Use the same parse/enqueue path as §6.2, writing to the same writer channel.
4. **Update `progress_index`** after each successful batch insert.
5. **Mark `status = completed`** when `progress_index >= target_end_index` (or current STH at start of backfill, whichever is lower).

The live `serve` ingest worker for a given log **continues running** during backfill. Both write to the same `certs` table; `ON CONFLICT DO NOTHING` deduplicates. The live worker advances at the head of the log; the backfill worker walks the historical window. The only contention is on the writer channel; backfill batches are tagged so the writer can prioritize live entries when the channel approaches capacity.

Backfill duration estimate, 2026 rates, fresh install with `--since=90d ago`: ~540M entries / ~20 logs / ~5 QPS each / ~1000 entries per request = ~90 minutes wall time. Real numbers depend on log rate-limit policies; Google's logs sustain >10 QPS while CloudFlare Nimbus throttles harder. The CLI prints per-log progress and an ETA.

Backfill is idempotent and resumable. Re-running with the same `--since` resumes incomplete jobs and is a no-op for completed ones.

## 7. Data model

### 7.1 Schema

```sql
-- Extensions (must be loaded once per database)
CREATE EXTENSION IF NOT EXISTS timescaledb;
CREATE EXTENSION IF NOT EXISTS pg_trgm;
CREATE EXTENSION IF NOT EXISTS btree_gin;

-- One row per unique precert (deduped on cert_hash).
-- Hypertable, partitioned by not_before (7-day chunks).
--
-- cert_hash = sha256(precert_DER) — matches crt.sh's lookup key.
-- The precert DER is byte-identical when the same precert is submitted to multiple
-- logs (only the SCTs differ), so this hash deduplicates cleanly across logs and
-- makes /v1/cert/{hash}/raw a direct passthrough to crt.sh.
CREATE TABLE certs (
    cert_hash          BYTEA       NOT NULL,                -- sha256(precert_DER), 32 bytes
    issuer_hash        BYTEA       NOT NULL,                -- sha256(issuer DN), 32 bytes
    issuer_cn          TEXT        NOT NULL,                -- "Let's Encrypt R10", etc.
    not_before         TIMESTAMPTZ NOT NULL,
    not_after          TIMESTAMPTZ NOT NULL,
    sans               TEXT[]      NOT NULL,                -- lowercase, deduped, sorted
    registered_domains TEXT[]      NOT NULL,                -- eTLD+1 per SAN via PSL, deduped
    first_seen         TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (cert_hash, not_before)
);

SELECT create_hypertable('certs', 'not_before', chunk_time_interval => INTERVAL '7 days');

CREATE INDEX certs_registered_domains_gin ON certs USING gin (registered_domains);
CREATE INDEX certs_issuer_hash             ON certs (issuer_hash, not_before DESC);

-- Flat name→cert mapping for fast subdomain queries.
-- Hypertable, partitioned by not_before.
CREATE TABLE cert_names (
    name              TEXT        NOT NULL,                 -- the SAN, lowercase, no trailing dot
    registered_domain TEXT        NOT NULL,                 -- eTLD+1
    cert_hash         BYTEA       NOT NULL,                 -- no FK (cross-hypertable FKs are awkward)
    not_before        TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (name, cert_hash, not_before)
);

SELECT create_hypertable('cert_names', 'not_before', chunk_time_interval => INTERVAL '7 days');

CREATE INDEX cert_names_regdom_seen ON cert_names (registered_domain, not_before DESC);
CREATE INDEX cert_names_name_trgm   ON cert_names USING gin (name gin_trgm_ops);

-- 400-day cold tier: unique SAN observations only (no per-cert detail).
-- Plain table (upsert pattern; hypertables don't love that).
CREATE TABLE names_observed (
    name              TEXT        PRIMARY KEY,
    registered_domain TEXT        NOT NULL,
    first_seen        TIMESTAMPTZ NOT NULL,
    last_seen         TIMESTAMPTZ NOT NULL,
    cert_count        INT         NOT NULL DEFAULT 1
);

CREATE INDEX names_observed_regdom_lastseen ON names_observed (registered_domain, last_seen DESC);

-- Per-log cursor for the tailer.
CREATE TABLE ingest_cursors (
    log_id         BYTEA       PRIMARY KEY,                 -- sha256(log public key)
    log_url        TEXT        NOT NULL,
    operator       TEXT        NOT NULL,
    last_tree_size BIGINT      NOT NULL DEFAULT 0,
    last_sth_raw   BYTEA,                                   -- raw STH for consistency proofs (optional)
    last_updated   TIMESTAMPTZ NOT NULL,
    state          TEXT        NOT NULL DEFAULT 'usable'    -- usable | degraded | disabled
);

-- Watchlist (when WATCHLIST_MODE=db). Otherwise managed externally.
CREATE TABLE watchlist (
    domain        TEXT        PRIMARY KEY,                  -- registered_domain (eTLD+1)
    added_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    notes         TEXT
);

-- Webhook delivery outbox.
CREATE TABLE webhook_outbox (
    id            BIGSERIAL   PRIMARY KEY,
    event_id      UUID        NOT NULL UNIQUE,
    body          JSONB       NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    attempts      INT         NOT NULL DEFAULT 0,
    last_attempt  TIMESTAMPTZ,
    last_status   INT,
    delivered_at  TIMESTAMPTZ,
    next_attempt  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX webhook_outbox_due ON webhook_outbox (next_attempt) WHERE delivered_at IS NULL;

-- Backfill job tracking. One row per (log_id, target window).
-- Resumable across process restarts.
CREATE TABLE backfill_jobs (
    id                  BIGSERIAL   PRIMARY KEY,
    log_id              BYTEA       NOT NULL,
    target_since        TIMESTAMPTZ NOT NULL,
    target_start_index  BIGINT      NOT NULL,
    target_end_index    BIGINT      NOT NULL,
    progress_index      BIGINT      NOT NULL,
    started_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at        TIMESTAMPTZ,
    status              TEXT        NOT NULL DEFAULT 'running',   -- running | completed | failed
    last_error          TEXT
);

CREATE UNIQUE INDEX backfill_jobs_unique_active
    ON backfill_jobs (log_id, target_since)
    WHERE status = 'running';
```

### 7.2 Retention and compression policies

```sql
-- Compress chunks older than 7 days
ALTER TABLE certs SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'registered_domains',
    timescaledb.compress_orderby   = 'not_before DESC, cert_hash'
);

ALTER TABLE cert_names SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'registered_domain',
    timescaledb.compress_orderby   = 'not_before DESC, name'
);

SELECT add_compression_policy('certs',      INTERVAL '7 days');
SELECT add_compression_policy('cert_names', INTERVAL '7 days');

-- Drop chunks older than RETENTION_DAYS_HOT (default 90)
SELECT add_retention_policy('certs',      INTERVAL '90 days');
SELECT add_retention_policy('cert_names', INTERVAL '90 days');

-- names_observed: manual daily prune
-- (run by a separate async task; not a TS policy because it's not a hypertable)
DELETE FROM names_observed WHERE last_seen < now() - INTERVAL '400 days';
```

### 7.3 Storage estimate

Steady state, 2026 issuance rates (~6M unique precerts/day, ~2.5 SANs/cert average):

| Table | Rows (steady) | Uncompressed | After TS compression |
|---|---|---|---|
| `certs` hot tier (90d) | ~540M | ~110 GB | ~20 GB |
| `cert_names` hot tier (90d) | ~1.35B | ~175 GB | ~30 GB |
| `names_observed` (400d) | ~500M–1B unique | ~150 GB | n/a (plain) |
| Indexes (on uncompressed chunks + plain tables) | — | ~80 GB | ~80 GB |
| Recent 7-day uncompressed chunks (rolling) | — | ~15 GB | — |
| WAL + temp space | — | ~16 GB | — |
| **Total** | | | **~340 GB** |

Forward-compat: cert lifetimes shrink to 200 days (Mar 2026), 100 days (2027), 47 days (2029). Each reduction increases the renewal rate; same domain space, more issuances. Hot-tier storage scales linearly with issuance rate. By 2029, expect roughly 5× the 2026 ingest rate, i.e., ~30M unique precerts/day. Hot tier at 2029 rates: ~500 GB compressed, still single-disk feasible.

## 8. HTTP API

All endpoints return JSON. Errors follow `{ "error": { "type": "...", "message": "..." } }` shape.

### 8.1 Health and metadata

```
GET  /v1/healthz
     200 → { ok: true, oldest_log_lag_seconds, total_certs, total_names, version }
     503 → { ok: false, reasons: [...] }  when ≥half of usable logs are degraded
GET  /v1/metrics                     # Prometheus exposition format
GET  /v1/logs                        # current ingest cursor state per log
```

### 8.2 Subdomain discovery

```
GET  /v1/lookup
     ?domain=example.com               # required; registered_domain or any SAN
     ?since=2026-02-12T00:00:00Z       # optional; default = now - 400d
     ?contains=admin                   # optional; substring filter on name
     ?limit=1000                       # default 1000, max 10000

200 →
{
  domain: "example.com",
  registered_domain: "example.com",
  total: 142,
  names: [
    { name: "www.example.com",    first_seen: "...", last_seen: "...", cert_count: 14 },
    { name: "api.example.com",    first_seen: "...", last_seen: "...", cert_count: 8 },
    { name: "*.example.com",      first_seen: "...", last_seen: "...", cert_count: 22 },
    ...
  ]
}
```

Implementation: queries `names_observed WHERE registered_domain = ? AND last_seen >= ?`, optionally filtered by `name LIKE '%<contains>%'` using the trigram index. For sub-90-day queries with cert-level granularity, the caller should use `/v1/certs` instead.

### 8.3 Cert history

```
GET  /v1/certs
     ?domain=example.com               # required; registered_domain
     ?since=2026-02-12T00:00:00Z       # optional; default = now - 90d
     ?limit=500                        # default 500, max 5000

200 →
{
  domain: "example.com",
  total: 47,
  certs: [
    {
      cert_hash: "abc123...",
      issuer_cn: "Let's Encrypt R10",
      not_before: "...",
      not_after: "...",
      sans: ["www.example.com", "example.com"],
      registered_domains: ["example.com"]
    },
    ...
  ]
}
```

Implementation: queries `certs WHERE registered_domains && ARRAY[?] AND not_before >= ?`. The GIN index on `registered_domains` makes this fast.

### 8.4 Single cert

```
GET  /v1/cert/{cert_hash_hex}
200 → full cert record (same shape as inside /v1/certs)
404 → cert not in our window

GET  /v1/cert/{cert_hash_hex}/raw
     Proxies to crt.sh, returns the DER body. Cache 24h in caller (we don't cache; let the
     reverse proxy / consumer decide). Falls back to 503 if crt.sh is unavailable.

GET  /v1/cert/{cert_hash_hex}/observations
     Proxies to crt.sh, returns per-log observation history (which logs saw this cert, when).
     Caller should cache (24h KV TTL recommended).
```

### 8.5 Watchlist (when WATCHLIST_MODE=db)

```
GET    /v1/watchlist                  # list domains
POST   /v1/watchlist  { domain, notes? }
DELETE /v1/watchlist/{domain}
GET    /v1/watchlist/{domain}/recent  # last 90d of certs matching this watchlist entry
```

When watchlist mode is `file` or `url`, write endpoints return 405. The watchlist is read-only and reloaded periodically from the configured source.

### 8.6 Live stream (WebSocket)

```
GET  /v1/stream?domains=example.com,foo.com   # WebSocket upgrade
     ?domains=                                # optional; comma-separated registered_domains
                                              # missing/empty = firehose (all precerts)
```

**Semantics:**
- Server upgrades to WebSocket. Each newly-observed precert (post-write to `certs`) is sent as a JSON text frame.
- Server-side filter on `registered_domains` if `?domains=` is set: a precert is forwarded iff any of its registered_domains is in the subscription set.
- **Best-effort, lossy.** If the subscriber can't keep up, the server drops them with close code `1013 (try again later)` and reason `"subscriber lagged"` after a lag threshold (default 64 messages behind, tunable via `STREAM_MAX_LAG`). The writer never blocks.
- No durability: drops on disconnect, no replay. Use the webhook path for guaranteed delivery.
- Heartbeat: server sends a `ping` frame every 30s; clients should respond with `pong`. Idle disconnect after 90s without `pong`.

**Frame shape (text frame, JSON):**

```json
{
  "event": "precert",
  "observed_at": "2026-05-12T15:00:00.123Z",
  "cert": {
    "cert_hash": "abc123...",
    "issuer_cn": "Let's Encrypt R10",
    "not_before": "2026-05-12T00:00:00Z",
    "not_after": "2026-08-10T23:59:59Z",
    "sans": ["www.example.com", "example.com"],
    "registered_domains": ["example.com"]
  },
  "log": {
    "operator": "Google",
    "log_id": "..."
  }
}
```

### 8.7 Stats snapshot

```
GET  /v1/stats
200 → JSON snapshot of operational metrics, designed for poll-and-emit-datapoint clients.
      Counters are monotonic since process start; gauges are instantaneous.
```

```json
{
  "since_process_start": "2026-05-10T12:00:00Z",
  "now": "2026-05-12T15:00:00Z",
  "totals": {
    "certs_in_window": 540123456,
    "unique_names_400d": 123456789,
    "watchlist_size": 42,
    "webhook_outbox_pending": 0,
    "webhook_outbox_failed": 0
  },
  "ingest": {
    "precerts_per_sec_1m": 78.2,
    "precerts_per_sec_5m": 81.4,
    "precerts_per_sec_1h": 79.9,
    "duplicates_dropped_per_sec_5m": 245.1,
    "writer_queue_depth": 142
  },
  "stream": {
    "subscribers": 3,
    "messages_sent_per_sec_5m": 78.2,
    "subscribers_dropped_total": 12
  },
  "logs": [
    {
      "log_id": "abc...",
      "operator": "Google",
      "url": "https://ct.googleapis.com/logs/...",
      "state": "usable",
      "tree_size": 12345678,
      "cursor": 12345670,
      "lag_entries": 8,
      "lag_seconds": 12,
      "errors_5m": 0
    }
  ],
  "backfill": {
    "active_jobs": 0,
    "completed_jobs_total": 1,
    "progress_pct": null
  }
}
```

The shape is **stable across versions**: fields may be added but not removed or renamed. Consumers (notably amber.systems datapoints) should poll every 30–60s and emit the gauges they care about.

### 8.8 Admin endpoints

```
POST /v1/admin/webhook/{event_id}/redeliver   # re-enqueue a failed webhook delivery
```

(Future admin endpoints land under `/v1/admin/*`. Reverse proxy should restrict this prefix.)

## 9. Watchlist + webhook alerts

### 9.1 Modes

| Mode | Behavior | Use case |
|---|---|---|
| `db` (default) | Watchlist stored in Postgres; CRUD via HTTP | Single-instance, single-user |
| `file` | YAML/JSON list reloaded on SIGHUP or every N minutes | Config-as-code, GitOps |
| `url` | Periodic GET of a JSON endpoint returning `{ domains: [...] }` | Multi-tenant consumers (amber.systems sync) |

### 9.2 Match semantics

A precert "matches" the watchlist if any element of `cert.registered_domains` exactly equals a watchlist entry. We match on `registered_domain` (eTLD+1), not arbitrary SANs — this is correct because:

- A precert for `foo.example.com` has `registered_domain = "example.com"` and matches a watchlist entry for `"example.com"`.
- This also correctly catches wildcards: a precert for `*.example.com` has `registered_domain = "example.com"`.
- It avoids false matches where a watchlist entry for `mycompany.com` would accidentally match a cert for `attackermycompany.com.attacker.io`.

Watchlist entries SHOULD be registered domains. If a user supplies `foo.example.com`, normalize to `example.com` and warn.

### 9.3 Webhook payload

```
POST <WEBHOOK_URL>
Content-Type: application/json
X-Ctwatch-Event: precert.match
X-Ctwatch-Signature: sha256=<hex hmac of body using WEBHOOK_HMAC_SECRET>
X-Ctwatch-Delivery: <event_id>  # UUID, retry-stable

{
  "event_id": "uuid",
  "event": "precert.match",
  "matched_at": "2026-05-12T15:00:00Z",
  "matched_watchlist_entries": ["example.com"],
  "cert": {
    "cert_hash": "...",
    "issuer_cn": "Let's Encrypt R10",
    "not_before": "...",
    "not_after": "...",
    "sans": [...],
    "registered_domains": [...]
  }
}
```

### 9.4 Delivery semantics

- **At-least-once.** Outbox table is durable; redelivery happens on restart.
- **Retries:** exponential backoff (1s, 4s, 16s, 64s, 256s, 1024s, 4096s) up to 7 attempts (~ 1h 30m total).
- **HMAC:** SHA-256 over the raw body, hex-encoded, in `X-Ctwatch-Signature: sha256=...`.
- **2xx clears the outbox entry.** Anything else triggers retry.
- **After 7 failed attempts:** mark `failed`, surface in `/v1/metrics` (`webhook_failures_total`). Don't drop — operator can re-deliver via admin endpoint.

```
POST /v1/admin/webhook/{event_id}/redeliver
```

### 9.5 Performance bounds

At ~6M precerts/day and a typical small-org watchlist (10–1000 domains), the per-cert hash-lookup in the watchlist takes microseconds. For very large watchlists (>100k domains), use a Bloom filter pre-check before exact lookup — design supports adding this, not needed in v1.

## 10. Configuration

All config is env-var-driven (12-factor). Optional YAML file (`-config /etc/ctwatch.yml`) overrides defaults; env vars override the file.

| Variable | Default | Description |
|---|---|---|
| `DATABASE_URL` | (required) | `postgres://user:pass@host:5432/ctwatch?sslmode=disable` |
| `LISTEN_ADDR` | `0.0.0.0:8080` | HTTP server bind address |
| `LOG_LIST_URL` | `https://www.gstatic.com/ct/log_list/v3/log_list.json` | Override for testing |
| `LOG_OPERATORS` | `*` | Comma-separated allowlist (`Google,Cloudflare,Sectigo,DigiCert,TrustAsia`) or `*` for all |
| `LOG_POLL_INTERVAL` | `30s` | How often to call `get-sth` per log |
| `LOG_ENTRY_BATCH_SIZE` | `1000` | `get-entries` batch size |
| `BACKFILL_RATE_LIMIT_QPS` | `5` | Per-log `get-entries` QPS ceiling for `ctwatch backfill` |
| `BACKFILL_CONCURRENCY` | `4` | Max concurrent backfill workers (across all logs) |
| `RETENTION_DAYS_HOT` | `90` | Drop `certs` and `cert_names` chunks older than this |
| `RETENTION_DAYS_COLD` | `400` | Drop `names_observed` rows older than this |
| `WATCHLIST_MODE` | `db` | `db` \| `file` \| `url` \| `disabled` |
| `WATCHLIST_FILE` | — | Path when mode=file |
| `WATCHLIST_URL` | — | URL when mode=url |
| `WATCHLIST_REFRESH` | `60s` | Reload interval for file/url modes |
| `WEBHOOK_URL` | — | Disabled if unset |
| `WEBHOOK_HMAC_SECRET` | — | Required when WEBHOOK_URL is set |
| `WEBHOOK_MAX_ATTEMPTS` | `7` | |
| `STREAM_ENABLED` | `true` | Enable/disable `/v1/stream` WebSocket endpoint |
| `STREAM_MAX_LAG` | `64` | Drop subscribers more than N messages behind |
| `STREAM_MAX_SUBSCRIBERS` | `100` | Reject new connections after this cap |
| `CRTSH_BASE_URL` | `https://crt.sh` | For `/v1/cert/{hash}/raw` and `/observations` lazy lookups |
| `METRICS_ENABLED` | `true` | `/v1/metrics` Prometheus endpoint |
| `LOG_LEVEL` | `info` | `debug` \| `info` \| `warn` \| `error` |

## 11. Deployment

> **Canonical deployment is a self-contained Docker Compose stack.** The repo ships everything required to bring ctwatch up on a fresh host with a single `docker compose up -d` after cloning. The compose stack includes its own Postgres + TimescaleDB instance — there is no external database dependency. Bare-metal installation is supported as §11.2 but is not the default path.

### 11.1 Canonical deployment — self-contained Docker Compose stack

The repo's `deploy/docker-compose.yml` is the supported deployment artifact. It defines:

- A dedicated `timescale/timescaledb:2.x-pg16` container with data on a named volume
- A `ctwatch` container built locally from the repo's `Dockerfile` (no external image registry needed for v0.x)
- Health checks so the ctwatch container waits for Postgres readiness
- A bind-mounted data directory at `./data/pg` for the Postgres volume so the host can see (and back up) the data
- A bind-mounted config directory at `./data/config` for optional watchlist YAML
- Loopback-only port publishing by default (the service is meant to be reached via a reverse proxy or tunnel, not directly from the internet)

#### File: `deploy/docker-compose.yml`

```yaml
name: ctwatch

services:
  db:
    image: timescale/timescaledb:2.17.2-pg16
    restart: unless-stopped
    environment:
      POSTGRES_DB: ctwatch
      POSTGRES_USER: ctwatch
      POSTGRES_PASSWORD: ${DB_PASSWORD:?DB_PASSWORD must be set in .env}
    volumes:
      - ./data/pg:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD", "pg_isready", "-U", "ctwatch", "-d", "ctwatch"]
      interval: 5s
      timeout: 5s
      retries: 30
    # No port exposure — only reached over the compose-internal network.

  ctwatch:
    build:
      context: ../                                  # repo root
      dockerfile: deploy/Dockerfile
    image: ctwatch:local                            # the image name docker-compose tags after build
    restart: unless-stopped
    depends_on:
      db:
        condition: service_healthy
    environment:
      DATABASE_URL: "postgres://ctwatch:${DB_PASSWORD}@db:5432/ctwatch"
      LISTEN_ADDR: "0.0.0.0:8080"
      LOG_OPERATORS: ${LOG_OPERATORS:-*}
      RETENTION_DAYS_HOT: ${RETENTION_DAYS_HOT:-90}
      RETENTION_DAYS_COLD: ${RETENTION_DAYS_COLD:-400}
      WATCHLIST_MODE: ${WATCHLIST_MODE:-db}
      WATCHLIST_FILE: ${WATCHLIST_FILE:-}
      WATCHLIST_URL: ${WATCHLIST_URL:-}
      WEBHOOK_URL: ${WEBHOOK_URL:-}
      WEBHOOK_HMAC_SECRET: ${WEBHOOK_HMAC_SECRET:-}
      STREAM_ENABLED: ${STREAM_ENABLED:-true}
      LOG_LEVEL: ${LOG_LEVEL:-info}
    volumes:
      - ./data/config:/etc/ctwatch:ro               # watchlist.yml lives here when WATCHLIST_MODE=file
    ports:
      - "127.0.0.1:8080:8080"                       # loopback-only; expose via reverse proxy/tunnel
    healthcheck:
      test: ["CMD", "/usr/local/bin/ctwatch", "healthcheck"]
      interval: 30s
      timeout: 5s
      retries: 3
      start_period: 60s
```

#### File: `deploy/Dockerfile`

```dockerfile
# syntax=docker/dockerfile:1.7

FROM rust:1.82-slim AS builder
WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# Cache dependencies separately from source.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
RUN mkdir src && echo "fn main(){}" > src/main.rs && \
    cargo build --release --locked && \
    rm -rf src target/release/deps/ctwatch*

# Build the real binary.
COPY src ./src
COPY migrations ./migrations
RUN cargo build --release --locked

# Runtime stage — minimal Debian, just what's needed for TLS.
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates tini \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --no-create-home --shell /usr/sbin/nologin ctwatch

COPY --from=builder /build/target/release/ctwatch /usr/local/bin/ctwatch
COPY --from=builder /build/migrations /usr/local/share/ctwatch/migrations

USER ctwatch
EXPOSE 8080
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/ctwatch"]
CMD ["serve"]
```

The `ctwatch serve` subcommand runs migrations on startup automatically (idempotent), then begins ingest + HTTP server. No separate migrate step is needed in the compose flow.

#### File: `deploy/.env.example`

```bash
# Required — generate with: openssl rand -base64 32
DB_PASSWORD=

# Optional — when set, ctwatch posts watchlist matches to this URL with HMAC-SHA256 signing.
WEBHOOK_URL=
WEBHOOK_HMAC_SECRET=

# Optional — restrict to specific CT log operators. "*" (default) tails all usable.
# Comma-separated: Google,Cloudflare,Sectigo,DigiCert,TrustAsia
LOG_OPERATORS=*

# Optional — retention windows (days). Defaults below match the spec.
RETENTION_DAYS_HOT=90
RETENTION_DAYS_COLD=400

# Watchlist source. "db" stores entries in Postgres (CRUD via API); "file" reads
# /etc/ctwatch/watchlist.yml; "url" pulls from a remote JSON endpoint.
WATCHLIST_MODE=db
WATCHLIST_FILE=
WATCHLIST_URL=

# Live stream WebSocket endpoint.
STREAM_ENABLED=true

LOG_LEVEL=info
```

#### First-run quickstart on a fresh host

This is the exact sequence for bringing ctwatch up from nothing. Designed to be runnable by an agent on the target host without further context.

```bash
# 1. Clone the repo.
git clone https://github.com/purpleempress/ctwatch.git
cd ctwatch

# 2. Generate a DB password and create the env file.
cp deploy/.env.example deploy/.env
sed -i "s|^DB_PASSWORD=|DB_PASSWORD=$(openssl rand -base64 32 | tr -d '/+=')|" deploy/.env

# 3. Pre-create the data directories so they exist with the right ownership.
mkdir -p deploy/data/pg deploy/data/config

# 4. If the host's filesystem is btrfs (check with `findmnt -T deploy/data/pg`),
#    disable COW on the Postgres data directory BEFORE Postgres writes anything.
#    On xfs/ext4 this step is a no-op.
if findmnt -T deploy/data/pg -no FSTYPE | grep -q '^btrfs$'; then
  chattr +C deploy/data/pg
fi

# 5. Build the ctwatch image and start the stack.
cd deploy
docker compose build
docker compose up -d

# 6. Watch the logs until you see "ingest worker started" lines for several logs.
docker compose logs -f ctwatch

# 7. Verify (from the host):
curl -sS http://127.0.0.1:8080/v1/healthz
curl -sS http://127.0.0.1:8080/v1/logs              # per-log ingest cursors
curl -sS http://127.0.0.1:8080/v1/stats             # operational snapshot

# 8. Optional: backfill the last 90 days (takes ~90 minutes; runs alongside live ingest).
docker compose exec ctwatch /usr/local/bin/ctwatch backfill \
  --since "$(date -u -d '90 days ago' +%Y-%m-%dT%H:%M:%SZ)"
```

After 5–10 minutes of running, `/v1/healthz` should report `ok: true` with `total_certs > 0`. Without backfill, the full 90-day window fills in over 90 days. Run `ctwatch backfill` to populate the historical window immediately.

#### Host requirements

- **Linux x86_64 or aarch64**, kernel ≥ 5.10
- **Docker Engine ≥ 24.0** with `docker compose` v2 plugin
- **Disk:** ~50 GB free at first run; ~400 GB free for steady-state operation. The Postgres volume lives at `deploy/data/pg/` — bind-mount this onto a partition with sufficient space.
- **Memory:** 8 GB minimum for the stack alone (`ctwatch` ~500 MB, Postgres+TimescaleDB ~2–8 GB depending on `shared_buffers` tuning). The default Postgres config in the TimescaleDB image is sized for ~8 GB.
- **Network egress:** ~36 GB/day to CT log endpoints over HTTPS (TCP/443) at steady state. Backfill bursts ~10–20× higher during the ~90 minutes it runs. The service makes no inbound connections; it polls outbound.

#### Optional: reverse-proxy / tunnel exposure

The service binds to loopback by default. To expose it on a private network, point a reverse proxy (nginx, Caddy, cloudflared, Tailscale serve) at `http://127.0.0.1:8080`. **Do not bind the container to a public interface without an authenticating proxy in front** — ctwatch has no built-in auth.

For the amber.systems integration specifically (Appendix A), the existing `cloudflared` config on chrysalis just needs a new ingress rule:

```yaml
# excerpt from cloudflared config
ingress:
  - hostname: ctwatch.<internal-zone>
    service: http://127.0.0.1:8080
```

WebSocket support: cloudflared, nginx, and Caddy all proxy `Upgrade: websocket` transparently. Verify the reverse-proxy config allows long-lived connections (idle timeout > heartbeat interval, default 30s).

#### Stopping, upgrading, resetting

```bash
# stop
docker compose down

# upgrade (after `git pull`)
docker compose build
docker compose up -d

# full reset (DESTROYS DATA — only use during development)
docker compose down -v
rm -rf deploy/data
```

#### Backups

The compose stack does not include a backup service. For production, schedule on the host:

```bash
# Daily — only the small operationally-important tables (cursors, watchlist, outbox, backfill_jobs).
# The bulk cert/name data is rebuildable from CT logs.
docker compose exec -T db pg_dump -U ctwatch \
  -t ingest_cursors -t watchlist -t webhook_outbox -t backfill_jobs \
  ctwatch \
  | gzip > /mnt/backup/ctwatch-$(date +%F).sql.gz
```

### 11.2 Bare-metal alternative (no Docker)

For hosts that can't run Docker:

```bash
curl -L https://github.com/purpleempress/ctwatch/releases/latest/download/ctwatch-x86_64-unknown-linux-musl.tar.gz | tar xz
sudo install ctwatch /usr/local/bin/
# Postgres + TimescaleDB installed and operational separately
sudo -u postgres createdb ctwatch
sudo -u postgres psql ctwatch -c 'CREATE EXTENSION timescaledb; CREATE EXTENSION pg_trgm; CREATE EXTENSION btree_gin;'
ctwatch migrate
sudo cp deploy/systemd/ctwatch.service /etc/systemd/system/
sudo systemctl enable --now ctwatch
```

The `musl`-linked binary has no glibc dependency, so it works on any modern Linux without extra runtime install. **The compose stack is the supported path; this exists for completeness.**

### 11.3 Systemd unit (used by §11.2)

```ini
[Unit]
Description=ctwatch CT log mirror
After=network-online.target postgresql.service
Wants=network-online.target

[Service]
Type=notify
User=ctwatch
EnvironmentFile=/etc/ctwatch/env
ExecStartPre=/usr/local/bin/ctwatch migrate
ExecStart=/usr/local/bin/ctwatch serve
Restart=always
RestartSec=5s
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
```

## 12. Observability

### 12.1 Metrics (Prometheus)

- `ctwatch_certs_ingested_total{operator}` counter
- `ctwatch_certs_deduped_total` counter (dropped duplicates)
- `ctwatch_names_observed_total` counter
- `ctwatch_log_tree_size{log_id, operator}` gauge
- `ctwatch_log_cursor{log_id, operator}` gauge
- `ctwatch_log_lag_entries{log_id}` gauge (tree_size - cursor)
- `ctwatch_log_lag_seconds{log_id}` gauge (time since last STH advance)
- `ctwatch_log_request_errors_total{log_id, kind}` counter
- `ctwatch_writer_queue_depth` gauge
- `ctwatch_watchlist_matches_total` counter
- `ctwatch_webhook_attempts_total{result}` counter
- `ctwatch_webhook_outbox_depth` gauge
- `ctwatch_webhook_outbox_failed` gauge
- `ctwatch_stream_subscribers` gauge
- `ctwatch_stream_messages_sent_total` counter
- `ctwatch_stream_subscribers_dropped_total{reason}` counter (lag, cap, disconnect)
- `ctwatch_backfill_active_jobs` gauge
- `ctwatch_backfill_progress_entries_total{log_id}` counter
- `ctwatch_db_active_connections` gauge

### 12.2 Structured logs

JSON to stdout. Key fields: `ts`, `level`, `msg`, plus event-specific fields (`log_id`, `cert_hash`, `event_id`, etc.). Designed for journald + Loki/Grafana or similar.

### 12.3 Health checks

`/v1/healthz` returns 200 if:
- DB is reachable
- At least half of `usable` logs have advanced their cursor in the last 5 minutes
- Writer queue depth < 90% of capacity

Otherwise 503 with `reasons[]`.

## 13. Project layout

```
ctwatch/
├── README.md                   # quick start
├── LICENSE                     # MIT
├── ARCHITECTURE.md             # this design doc, lightly edited
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml         # pins MSRV (1.82)
├── docs/
│   ├── API.md                  # endpoint reference
│   ├── CONFIG.md               # env vars reference
│   ├── DEPLOY.md               # docker, systemd, k8s examples
│   ├── INTEGRATIONS.md         # how to consume ctwatch from various stacks
│   └── CONTRIBUTING.md
├── src/
│   ├── main.rs                 # clap entry: serve, migrate, backfill, healthcheck
│   ├── lib.rs                  # public surface for integration tests
│   ├── config.rs               # figment env + yaml loader
│   ├── cmd/
│   │   ├── mod.rs
│   │   ├── serve.rs            # the `serve` subcommand wiring
│   │   ├── migrate.rs          # `ctwatch migrate`
│   │   ├── backfill.rs         # `ctwatch backfill --since=...`
│   │   └── healthcheck.rs      # `ctwatch healthcheck` (container probe)
│   ├── ingest/
│   │   ├── mod.rs
│   │   ├── log_list.rs         # fetch gstatic log_list.json, filter usable
│   │   ├── worker.rs           # per-log poll loop (live tail)
│   │   ├── backfill_worker.rs  # binary-search + walk-forward per log
│   │   ├── client.rs           # RFC 6962 client (get-sth, get-entries)
│   │   └── entry.rs            # MerkleTreeLeaf decode + precert TBS extraction
│   ├── parse/
│   │   ├── mod.rs
│   │   ├── san.rs              # normalize, dedupe, sort SAN list
│   │   └── psl.rs              # eTLD+1 via publicsuffix crate
│   ├── writer.rs               # single-writer task: dedup + insert + watchlist check + broadcast
│   ├── store/
│   │   ├── mod.rs              # PgPool setup
│   │   ├── certs.rs            # INSERT certs + cert_names
│   │   ├── names.rs            # UPSERT names_observed
│   │   ├── cursors.rs          # ingest_cursors I/O
│   │   ├── watchlist.rs        # watchlist table I/O (db mode only)
│   │   ├── outbox.rs           # webhook outbox I/O
│   │   └── backfill_jobs.rs    # backfill_jobs I/O
│   ├── stream/
│   │   ├── mod.rs              # broadcast channel + subscriber bookkeeping
│   │   └── handler.rs          # axum WebSocket handler for /v1/stream
│   ├── stats/
│   │   ├── mod.rs              # in-process counters + ring buffers
│   │   └── snapshot.rs         # JSON snapshot builder for /v1/stats
│   ├── api/
│   │   ├── mod.rs              # axum Router builder
│   │   ├── lookup.rs           # /v1/lookup
│   │   ├── certs.rs            # /v1/certs, /v1/cert/:hash[/raw|/observations]
│   │   ├── watchlist.rs        # /v1/watchlist/*
│   │   ├── stream.rs           # /v1/stream (delegates to stream::handler)
│   │   ├── stats.rs            # /v1/stats
│   │   ├── health.rs           # /v1/healthz, /v1/metrics, /v1/logs
│   │   └── error.rs            # uniform error envelope
│   ├── watchlist/
│   │   ├── mod.rs              # WatchlistSource trait
│   │   ├── db_source.rs
│   │   ├── file_source.rs      # SIGHUP-reloadable
│   │   └── url_source.rs       # periodic GET
│   ├── webhook.rs              # HMAC + retry dispatcher
│   └── observability.rs        # tracing-subscriber + metrics setup
├── migrations/                 # sqlx-compatible numbered SQL
│   ├── 0001_extensions.sql
│   ├── 0002_certs_hypertable.sql
│   ├── 0003_cert_names_hypertable.sql
│   ├── 0004_names_observed.sql
│   ├── 0005_ingest_cursors.sql
│   ├── 0006_watchlist.sql
│   ├── 0007_webhook_outbox.sql
│   ├── 0008_backfill_jobs.sql
│   └── 0009_retention_policies.sql
├── tests/
│   ├── fixtures/               # frozen CT entry samples
│   ├── ingest_test.rs          # parsing + dedup against fixtures
│   ├── store_test.rs           # #[sqlx::test] DB-backed tests
│   ├── stream_test.rs          # WebSocket fan-out + lag drop behavior
│   ├── stats_test.rs           # /v1/stats shape stability
│   ├── backfill_test.rs        # binary-search correctness on mock log
│   └── e2e_test.rs             # spawn binary, hit small real log, validate
├── benches/                    # criterion benches for parse hot path
├── deploy/
│   ├── docker-compose.yml
│   ├── Dockerfile              # multi-stage: rust:1.82-slim → debian:bookworm-slim
│   ├── systemd/ctwatch.service
│   └── examples/
│       └── nginx-reverse-proxy.conf
└── .github/
    └── workflows/
        ├── ci.yml              # cargo fmt --check, cargo clippy --deny warnings, cargo test
        └── release.yml         # cargo-dist on tag → binaries + ghcr image
```

## 14. Testing strategy

| Layer | Approach |
|---|---|
| Unit | `#[test]` and `#[tokio::test]` for: MerkleTreeLeaf decode, TBS precert parsing, SAN normalization, PSL eTLD+1, dedup logic, watchlist matching, HMAC signing, stats ring-buffer rate calc. All deterministic, fixtures in `tests/fixtures/`, no external deps. |
| Integration | `#[sqlx::test]` macro provisions a fresh Postgres+TimescaleDB database per test (against a CI-provided PG instance, or `testcontainers-rs` for local). Tests writer + store + retention policy execution + backfill_jobs lifecycle. |
| Stream | `tests/stream_test.rs` spins up the axum server with an injected broadcast channel, opens N WebSocket clients, verifies fan-out, then withholds reads on one client and verifies it gets dropped with close code 1009 without affecting others. |
| Backfill | `tests/backfill_test.rs` uses a mock CT log (in-process axum) with a deterministic entry timeline; verifies the binary search converges to the right entry index for known `--since` values; verifies resumability across simulated crash mid-job. |
| E2E | `tests/e2e_test.rs` spawns the binary with a tmpdir config, points it at a small real CT log shard, validates cert/name counts in the DB and response shapes from the API, asserts a webhook fires against a local axum receiver, asserts a WebSocket subscriber receives a frame, asserts `/v1/stats` reports non-zero ingest rate. |
| Bench | `criterion` benches for the hot parsing path (`MerkleTreeLeaf` decode + SAN extraction). Tracks regressions across releases. |
| Load | Replay a day of cert metadata at 10× speed against a staging instance. Verify writer keeps up and queue doesn't back up. Manual, not in CI. |

CI gates: `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-features`. E2E runs on `main` post-merge. Releases are gated on all green.

## 15. Release engineering

- **Versioning:** SemVer. v0.x while pre-stable; v1.0 when API and schema are committed-to.
- **Releases:** GitHub Releases triggered by tag `vX.Y.Z`, automated via `cargo-dist`. Each release publishes:
  - `ctwatch-x86_64-unknown-linux-musl.tar.gz` (statically linked, no glibc dep)
  - `ctwatch-aarch64-unknown-linux-musl.tar.gz`
  - `ctwatch-aarch64-apple-darwin.tar.gz`
  - Docker image `ghcr.io/purpleempress/ctwatch:vX.Y.Z` and `:latest` (multi-arch via `docker buildx`)
- **Subcommands shipped:** `serve`, `migrate`, `backfill`, `healthcheck`. All documented in `--help` output and `docs/CONFIG.md`.
- **MSRV policy:** pinned in `rust-toolchain.toml` (1.82). Bumps go in a minor release with a changelog note.
- **Migrations:** every schema change is a numbered SQL file under `migrations/`. `ctwatch migrate` applies pending using `sqlx::migrate!`. No downgrade migrations — we never need to roll back schema in a system where data is rebuildable.
- **Changelog:** `CHANGELOG.md` at repo root, Keep-a-Changelog format.
- **Security policy:** `SECURITY.md` with disclosure email. No private-vuln embargo expected in v0.x given the threat model (read-only public CT data), but document the process.
- **Reproducible builds:** pin `Cargo.lock`, `cargo build --locked` in CI. Optional: `cargo-deny` for license + advisory enforcement on every PR.

## 16. Risks and open questions

| Risk | Mitigation |
|---|---|
| Single point of failure (one VM, one DB) | Data is rebuildable from CT logs via `ctwatch backfill`. Backups are nightly `pg_dump` of `ingest_cursors`, `watchlist`, `webhook_outbox`, and `backfill_jobs` only (the rest re-ingests). Document recovery procedure. |
| Postgres disk fills | `/v1/metrics` exposes `ctwatch_db_disk_used_percent`; ops sets alerts. Retention policies are the primary defense. |
| Log decommissioning mid-stream | Daily log_list refresh marks dead logs as `disabled`; ingest workers stop polling. Existing data remains queryable. |
| Postgres lock contention from writer + retention + backfill | Retention jobs run in low-traffic windows (3am UTC by default). Writer uses small transactions (~100 rows). Backfill batches use the same path and same transaction sizes as live ingest. |
| crt.sh outage breaks `/cert/{hash}/raw` | Endpoint returns 503; the rest of the API is unaffected. Document this dependency. |
| Disk growth as cert lifetimes shrink | TS compression keeps pace through 2029 (~5× growth headroom). Re-evaluate at that point. |
| Watchlist match latency at large list size | Bloom filter pre-check, deferred to v2 when needed. |
| WebSocket subscriber abuse (firehose without filter) | `STREAM_MAX_SUBSCRIBERS` cap (default 100). Reverse proxy can also rate-limit `/v1/stream` upgrades. |
| Backfill rate-limit bans from log operators | Per-log QPS ceiling (`BACKFILL_RATE_LIMIT_QPS`, default 5). Exponential backoff on 429/5xx. Document each operator's documented limits. |
| Adversarial CT log (malicious operator) | Out of scope. We trust the operator set Chrome trusts. Mitigation is to set `LOG_OPERATORS` to a smaller allowlist. |

## 17. What's not in v1

- **SCT validation.** We trust the logs; we don't verify SCTs.
- **Merkle inclusion proof verification.** Same.
- **Bloom filter watchlist pre-check.** Add when watchlist size justifies it.
- **Web UI.** Separate downstream project.
- **Multi-tenant watchlists.** Single-tenant only; tenancy belongs in the consumer.
- **Continuous aggregates** (e.g., "certs per day per CA"). Add when analytical features appear. `/v1/stats` covers the operational live view.
- **GraphQL.** Maybe never.
- **SSE alternative to WebSocket.** WebSocket covers the use case; SSE could be added if a consumer needs it but isn't on the v1 list.

## 18. Acceptance criteria (v0.1)

The first release ships when:

1. **Self-contained quickstart works end-to-end:** from a fresh clone of the repo on a Linux host with Docker installed, the exact sequence in §11.1 (clone → set `DB_PASSWORD` → optional `chattr +C` on btrfs → `docker compose build` → `docker compose up -d`) brings the stack to healthy state within 5 minutes, with no additional manual steps required and no external service dependencies.
2. Ingests from all `usable` 2026 log shards in the Chrome log list.
3. After 1 hour: `total_certs > 0`, all log cursors advancing, writer queue depth bounded.
4. `GET /v1/lookup?domain=<some-major-domain>` returns SANs within seconds.
5. `GET /v1/certs?domain=<some-major-domain>` returns cert metadata.
6. Watchlist match fires a webhook with valid HMAC signature within 60s of cert observation.
7. WebSocket `/v1/stream` accepts connections, fans out new precerts as JSON frames, applies `?domains=` filter correctly, drops lagging subscribers without blocking the writer.
8. `GET /v1/stats` returns the documented JSON shape with live ingest rates and per-log lag.
9. `ctwatch backfill --since=<date>` binary-searches each log to the target timestamp, walks forward to current STH, writes through the same writer pipeline, is resumable across crashes, and runs concurrently with live ingest without conflict.
10. Compression policy activates on 7d+ chunks (verifiable via `timescaledb_information.chunks`).
11. Retention policy drops chunks older than 90d (verifiable on a sped-up test).
12. README + ARCHITECTURE + API + CONFIG docs are complete.
13. CI passes unit + integration + stream + backfill tests.
14. License header on all source files.
15. SemVer-tagged release with binaries and docker image published.

---

## Appendix A — amber.systems integration

This is how amber's Worker stack consumes `ctwatch`. **Nothing in this section affects the upstream OSS design;** it's downstream wiring.

### A.1 Deployment

`ctwatch` is deployed on `chrysalis` (the existing VM that hosts `iptoasn`) using the **canonical self-contained Docker Compose stack documented in §11.1**. No amber-specific deployment tooling exists — amber consumes a vanilla ctwatch deployment over the cloudflared tunnel like any other HTTP service.

Concretely:

- Clone `ctwatch` into `/srv/ctwatch` on chrysalis
- Follow §11.1's first-run quickstart verbatim (set `DB_PASSWORD`, build, `docker compose up -d`)
- Run `ctwatch backfill --since="$(date -u -d '90 days ago' +%Y-%m-%dT%H:%M:%SZ)"` once to populate the historical window
- The stack publishes `127.0.0.1:8080` on chrysalis
- Add one ingress rule to the existing `cloudflared` config:
  ```yaml
  - hostname: ctwatch.<internal-zone>
    service: http://127.0.0.1:8080
  ```
- The Worker reaches `http://ctwatch/...` via the `CTWATCH_VPC` service binding (§A.2)

Amber writes no ctwatch-specific configuration; everything is upstream defaults plus `WATCHLIST_MODE=url` and `WEBHOOK_URL`/`WEBHOOK_HMAC_SECRET` set to the Worker's internal endpoints (§A.4).

### A.2 Worker binding

Add to `wrangler.jsonc`:

```jsonc
"services": [
  { "binding": "IPTOASN_VPC", "service": "iptoasn-router" },
  { "binding": "CTWATCH_VPC", "service": "ctwatch-router" }
]
```

### A.3 New public services

Two additions to `src/services/`:

1. **`net.tls`** (extending the existing service registration that's currently a stub) — calls `CTWATCH_VPC.fetch("http://ctwatch/v1/certs?domain=...")` and reshapes the response to amber's brand-stripped envelope. Falls back to `crt.sh` if `ctwatch` returns 5xx.

2. **`security.subdomains-ct`** (new) — calls `CTWATCH_VPC.fetch("http://ctwatch/v1/lookup?domain=...")`. Distinct from the existing `security.subdomains` (DNS bruteforce) on the services page — register as a new entry in `ALL_SERVICES` in `ui/src/routes/app/services/Index.tsx`. The two together give wider coverage than either alone.

Both go through `handleServiceRoute`, get the standard auth+grants+brand-strip treatment, and emit `usage_events` rows.

### A.4 Watchlist sync + webhook receiver (for Watch feature, separate spec)

When the Watch feature ships, amber's Worker is the authoritative source for which domains users have asked to watch. To get those into `ctwatch`:

- `ctwatch` configured with `WATCHLIST_MODE=url`, `WATCHLIST_URL=https://api.asy.st/v1/internal/ctwatch/watchlist`, `WATCHLIST_REFRESH=60s`
- A new internal Worker route `GET /v1/internal/ctwatch/watchlist` (not in `serviceFor`; bound to a specific bearer credential from `ctwatch`'s end) returns `{ domains: [string] }` aggregated from the watch table in D1
- `ctwatch` configured with `WEBHOOK_URL=https://api.asy.st/v1/internal/ctwatch/event`, `WEBHOOK_HMAC_SECRET=<shared>` — Worker validates HMAC, looks up which user(s) watch the matched domain, fans out user-facing notifications

Both internal endpoints are gated by a shared HMAC secret; the credential lives in wrangler secrets on the Worker side and in `ctwatch`'s env on the VM side. No public exposure.

### A.5 Datapoints integration via `/v1/stats`

amber.systems' datapoints feature polls `/v1/stats` on a 30s schedule and emits selected gauges as time-series:

- `ingest.precerts_per_sec_5m` — CT issuance rate as observed by our mirror
- `ingest.duplicates_dropped_per_sec_5m` — cross-log dedup rate
- `totals.certs_in_window` — size of the 90d hot tier
- `totals.unique_names_400d` — size of the 400d cold tier
- `stream.subscribers` — live consumers connected (useful for capacity planning)
- `webhooks.outbox_depth` and `outbox_failed` — operational health of the alert path
- Per-log `lag_entries` and `lag_seconds` for the top-3 highest-volume logs

The poll source lives in the same Worker as A.3/A.4; the cron trigger and storage shape are spec'd in the datapoints feature itself, not here.

### A.6 Watch feature is its own spec

The amber-side Watch product (alert delivery to users, UI, history view) is out of scope for this spec. We add only the internal endpoints `ctwatch` needs to connect to it. Spec'ing the Watch product is a follow-on.

---

## Appendix B — naming

Working name `ctwatch` is descriptive but generic. Candidates if a better name emerges before publication:

| Name | Reasoning |
|---|---|
| **ctwatch** | descriptive, short, on the nose; possibly already used by minor projects |
| **argus** | Greek hundred-eyed watcher; alludes to transparency monitoring; short and memorable; possibly already in use |
| **treetap** | "tap" the Merkle tree; gives a clear mental image; unique |
| **cthound** | "hound for CT logs"; playful; possibly available |
| **certmast** | "mast" as in lookout; nautical metaphor; almost certainly available |
| **bellman** | town crier; alerts on new certs; literary |

To be decided before first public release.

---

**End of design.**
