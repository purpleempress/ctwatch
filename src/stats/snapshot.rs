use crate::stats::Counters;
use chrono::Utc;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::time::Duration;

pub async fn build(
    pool: &PgPool,
    counters: &Counters,
    retention_days_hot: u32,
) -> anyhow::Result<Value> {
    let now = Utc::now();
    let since_hot = now - chrono::Duration::days(retention_days_hot as i64);

    let total_certs = crate::store::certs::count_in_window(pool, since_hot).await?;
    let total_names = crate::store::names::count(pool).await?;
    let watchlist_size = crate::store::watchlist::count(pool).await?;
    let outbox_pending = crate::store::outbox::pending_count(pool).await?;
    let outbox_failed = crate::store::outbox::failed_count(pool, 7).await?;
    let active_backfill = crate::store::backfill_jobs::active_count(pool).await?;
    let completed_backfill = crate::store::backfill_jobs::completed_count(pool).await?;
    let cursors = crate::store::cursors::list(pool).await?;

    let rate_1m = counters.rate_certs(Duration::from_secs(60)).await;
    let rate_5m = counters.rate_certs(Duration::from_secs(300)).await;
    let rate_1h = counters.rate_certs(Duration::from_secs(3600)).await;
    let dup_5m = counters.rate_dups(Duration::from_secs(300)).await;
    let stream_5m = counters.rate_stream(Duration::from_secs(300)).await;

    Ok(json!({
        "since_process_start": now - chrono::Duration::seconds(counters.started_at().elapsed().as_secs() as i64),
        "now": now,
        "totals": {
            "certs_in_window": total_certs,
            "unique_names_400d": total_names,
            "watchlist_size": watchlist_size,
            "webhook_outbox_pending": outbox_pending,
            "webhook_outbox_failed": outbox_failed,
        },
        "ingest": {
            "precerts_per_sec_1m": rate_1m,
            "precerts_per_sec_5m": rate_5m,
            "precerts_per_sec_1h": rate_1h,
            "duplicates_dropped_per_sec_5m": dup_5m,
            "writer_queue_depth": counters.queue_depth(),
        },
        "stream": {
            "subscribers": counters.subscribers(),
            "messages_sent_per_sec_5m": stream_5m,
            "subscribers_dropped_total": counters.stream_dropped(),
        },
        "logs": cursors.iter().map(|c| json!({
            "log_id": hex::encode(&c.log_id),
            "operator": c.operator,
            "url": c.log_url,
            "state": c.state,
            "cursor": c.last_tree_size,
        })).collect::<Vec<_>>(),
        "backfill": {
            "active_jobs": active_backfill,
            "completed_jobs_total": completed_backfill,
            "progress_pct": Value::Null,
        }
    }))
}
