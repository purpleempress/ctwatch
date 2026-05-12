use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use std::time::Duration;
use tokio::sync::Semaphore;

use crate::ingest::{client::CtClient, entry::decode_leaf, log_list::CtLog};
use crate::store::backfill_jobs;
use crate::writer::{WriterHandle, WriterJob};

pub struct BackfillCfg {
    pub since: DateTime<Utc>,
    pub rate_limit_qps: u32,
    pub batch_size: u32,
}

/// Binary-search for the entry index of `since` in a given log, then walk forward
/// to the current STH writing through the same WriterHandle as live ingest.
pub async fn run_for_log(
    pool: PgPool,
    writer: WriterHandle,
    log: CtLog,
    cfg: BackfillCfg,
) -> Result<()> {
    let client = CtClient::new();
    let sth = client.get_sth(&log.url).await?;
    let tree_size = sth.tree_size as i64;
    if tree_size == 0 {
        return Ok(());
    }

    let target_ms = cfg.since.timestamp_millis() as u64;
    let start_index = binary_search_index(&client, &log.url, tree_size, target_ms).await?;
    let end_index = tree_size - 1;
    if start_index >= end_index {
        return Ok(());
    }

    let job_id = match backfill_jobs::find_active(&pool, &log.log_id, cfg.since).await? {
        Some(j) => {
            tracing::info!(
                log = %log.operator,
                "resuming backfill job {} at progress {}",
                j.id,
                j.progress_index
            );
            j.id
        }
        None => {
            backfill_jobs::insert(&pool, &log.log_id, cfg.since, start_index, end_index).await?
        }
    };

    let mut cursor = match backfill_jobs::find_active(&pool, &log.log_id, cfg.since).await? {
        Some(j) => j.progress_index,
        None => start_index,
    };

    let request_delay = Duration::from_millis(1000 / cfg.rate_limit_qps.max(1) as u64);
    while cursor < end_index {
        let batch_end = std::cmp::min(cursor + cfg.batch_size as i64 - 1, end_index);
        match client
            .get_entries(&log.url, cursor as u64, batch_end as u64)
            .await
        {
            Ok(entries) => {
                let count = entries.len() as i64;
                for raw in entries {
                    if let Ok(decoded) = decode_leaf(&raw.leaf_input, &raw.extra_data) {
                        let _ = writer
                            .tx
                            .send(WriterJob {
                                log_id: log.log_id.clone(),
                                log_url: log.url.clone(),
                                operator: log.operator.clone(),
                                entry: decoded,
                            })
                            .await;
                    }
                }
                cursor += count.max(1);
                backfill_jobs::update_progress(&pool, job_id, cursor).await?;
            }
            Err(e) => {
                tracing::warn!(log = %log.operator, "backfill batch err: {e:#}; sleeping 5s");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        }
        tokio::time::sleep(request_delay).await;
    }
    backfill_jobs::complete(&pool, job_id).await?;
    Ok(())
}

async fn binary_search_index(
    client: &CtClient,
    url: &str,
    tree_size: i64,
    target_ms: u64,
) -> Result<i64> {
    let (mut lo, mut hi) = (0i64, tree_size - 1);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let entries = client.get_entries(url, mid as u64, mid as u64).await?;
        if entries.is_empty() {
            return Err(anyhow!("empty get-entries at {mid}"));
        }
        let decoded = decode_leaf(&entries[0].leaf_input, &entries[0].extra_data)?;
        if decoded.timestamp_ms >= target_ms {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    Ok(lo)
}

/// Run backfill across multiple logs with a concurrency cap.
pub async fn run_many(
    pool: PgPool,
    writer: WriterHandle,
    logs: Vec<CtLog>,
    since: DateTime<Utc>,
    concurrency: usize,
    rate_limit_qps: u32,
    batch_size: u32,
) -> Result<()> {
    let sem = std::sync::Arc::new(Semaphore::new(concurrency.max(1)));
    let mut tasks = Vec::new();
    for log in logs {
        let pool = pool.clone();
        let writer = writer.clone();
        let sem = sem.clone();
        let cfg = BackfillCfg {
            since,
            rate_limit_qps,
            batch_size,
        };
        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.expect("semaphore");
            if let Err(e) = run_for_log(pool, writer, log.clone(), cfg).await {
                tracing::error!(log = %log.operator, "backfill failed: {e:#}");
            }
        }));
    }
    for t in tasks {
        let _ = t.await;
    }
    Ok(())
}
