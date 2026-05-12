use anyhow::Result;
use sqlx::PgPool;
use std::time::Duration;

use crate::ingest::{client::CtClient, entry::decode_leaf, log_list::CtLog};
use crate::store::cursors;
use crate::writer::{WriterHandle, WriterJob};

pub struct WorkerCfg {
    pub poll_interval: Duration,
    pub batch_size: u32,
}

pub fn spawn(pool: PgPool, writer: WriterHandle, log: CtLog, cfg: WorkerCfg) {
    let client = CtClient::new();
    tokio::spawn(async move {
        // Ensure cursor row exists.
        if let Err(e) = cursors::upsert(&pool, &log.log_id, &log.url, &log.operator).await {
            tracing::error!(operator = %log.operator, url = %log.url, "cursor upsert: {e:#}");
            return;
        }
        run(pool, client, writer, log, cfg).await
    });
}

async fn run(pool: PgPool, client: CtClient, writer: WriterHandle, log: CtLog, cfg: WorkerCfg) {
    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(300);
    loop {
        match tick(&pool, &client, &writer, &log, &cfg).await {
            Ok(true) => {
                backoff = Duration::from_secs(1);
            } // made progress
            Ok(false) => {} // caught up
            Err(e) => {
                tracing::warn!(operator = %log.operator, url = %log.url, "tick error: {e:#}; backoff={:?}", backoff);
                tokio::time::sleep(backoff).await;
                backoff = std::cmp::min(backoff * 2, max_backoff);
                continue;
            }
        }
        tokio::time::sleep(cfg.poll_interval).await;
    }
}

async fn tick(
    pool: &PgPool,
    client: &CtClient,
    writer: &WriterHandle,
    log: &CtLog,
    cfg: &WorkerCfg,
) -> Result<bool> {
    let cursor = current_cursor(pool, &log.log_id).await?;
    let sth = client.get_sth(&log.url).await?;
    if (sth.tree_size as i64) <= cursor {
        return Ok(false);
    }
    let start = cursor as u64;
    let end_inclusive = std::cmp::min(start + cfg.batch_size as u64 - 1, sth.tree_size - 1);
    let entries = client.get_entries(&log.url, start, end_inclusive).await?;
    if entries.is_empty() {
        return Ok(false);
    }

    let last_index = start + entries.len() as u64;
    for raw in entries {
        let decoded = decode_leaf(&raw.leaf_input, &raw.extra_data)?;
        let job = WriterJob {
            log_id: log.log_id.clone(),
            log_url: log.url.clone(),
            operator: log.operator.clone(),
            entry: decoded,
        };
        writer
            .tx
            .send(job)
            .await
            .map_err(|e| anyhow::anyhow!("writer rx closed: {e}"))?;
    }
    cursors::advance(pool, &log.log_id, last_index as i64).await?;
    Ok(true)
}

async fn current_cursor(pool: &PgPool, log_id: &[u8]) -> Result<i64> {
    let r = sqlx::query!(
        "SELECT last_tree_size FROM ingest_cursors WHERE log_id = $1",
        log_id
    )
    .fetch_one(pool)
    .await?;
    Ok(r.last_tree_size)
}
