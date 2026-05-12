# ctwatch

A self-hosted CT mirror. Polls every usable Certificate Transparency log, stores SANs in Postgres + TimescaleDB, exposes an HTTP read API. One Rust binary, MIT.

## Features

- 90-day rolling mirror of every precert from usable CT logs
- 400-day unique-name index for subdomain discovery
- HTTP read API plus Prometheus metrics
- Webhook alerts on watchlist matches, HMAC-SHA256 signed, with a durable retry outbox
- `/v1/stream` WebSocket fan-out, optional `?domains=` filter
- `ctwatch backfill` to fill the historical window without waiting 90 days
- crt.sh passthrough for raw DER bodies and per-log observation history

## Quickstart

```bash
git clone https://github.com/purpleempress/ctwatch.git
cd ctwatch
cp deploy/.env.example deploy/.env
sed -i "s|^DB_PASSWORD=|DB_PASSWORD=$(openssl rand -base64 32 | tr -d '/+=')|" deploy/.env
mkdir -p deploy/data/pg deploy/data/config
cd deploy
docker compose build
docker compose up -d
curl http://127.0.0.1:8080/v1/healthz
```

Give it a few minutes; `/v1/stats` will start reporting `precerts_per_sec_5m > 0`. To populate the 90-day window straight away:

```bash
docker compose exec ctwatch ctwatch backfill \
  --since "$(date -u -d '90 days ago' +%Y-%m-%dT%H:%M:%SZ)"
```

## Endpoints

| Endpoint | Purpose |
|---|---|
| `GET /v1/lookup?domain=…` | SAN history (last 400d) |
| `GET /v1/certs?domain=…` | Cert history (last 90d) |
| `GET /v1/cert/{hash}` | One cert by `sha256(precert DER)` |
| `GET /v1/cert/{hash}/raw` | crt.sh passthrough, DER body |
| `GET /v1/cert/{hash}/observations` | crt.sh passthrough, per-log history |
| `GET /v1/stream` (WebSocket) | Live precert fan-out, `?domains=…` filter optional |
| `GET /v1/stats` | Live ingest rate, log lag, totals |
| `GET /v1/healthz` | Readiness probe |
| `GET /v1/metrics` | Prometheus exposition |
| `GET /v1/logs` | Per-log cursor state |
| `GET/POST/DELETE /v1/watchlist[/{domain}]` | Watchlist (db mode) |
| `POST /v1/admin/webhook/{event_id}/redeliver` | Re-enqueue a failed webhook delivery |

## Watchlist sources

`WATCHLIST_MODE` picks where the in-memory watchlist comes from:

- `db` (default): managed via `/v1/watchlist`, reloaded from Postgres every `WATCHLIST_REFRESH`
- `file`: YAML at `WATCHLIST_FILE`
- `url`: JSON `{ domains: [...] }` at `WATCHLIST_URL`
- `disabled`: matcher stays empty, webhooks never fire

Set `WEBHOOK_URL` and `WEBHOOK_HMAC_SECRET` to turn alerts on. Every match writes a row to `webhook_outbox`; the dispatcher POSTs the body with `X-Ctwatch-Signature: sha256=<hex hmac>`. Failures retry with exponential backoff up to `WEBHOOK_MAX_ATTEMPTS` (default 7).

## Architecture

See `ARCHITECTURE.md`.

## Licence

MIT.
