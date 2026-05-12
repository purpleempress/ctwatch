use anyhow::Result;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;

use crate::ingest::{log_list, worker as ingest_worker};
use crate::stats::Counters;
use crate::stream;
use crate::watchlist;
use crate::writer;
use crate::{config::Config, observability, store};

pub async fn run() -> Result<()> {
    let cfg = Config::load()?;
    observability::init_tracing(&cfg.log_level)?;
    let metrics_handle = observability::init_metrics()?;
    tracing::info!("ctwatch starting; listen_addr={}", cfg.listen_addr);

    let pool = store::pool(&cfg.database_url).await?;
    store::migrate(&pool).await?;

    let counters = Counters::new();
    counters.spawn_ticker();

    let (stream_tx, _initial_rx) = stream::channel(cfg.stream_max_lag.max(64));

    let matcher = watchlist::Matcher::new();
    match cfg.watchlist_mode.as_str() {
        "db" => {
            if let Ok(d) = store::watchlist::list(&pool).await {
                matcher.replace(d).await;
            }
            watchlist::spawn_db_reload(pool.clone(), matcher.clone(), cfg.watchlist_refresh);
        }
        "file" => {
            let path = cfg
                .watchlist_file
                .clone()
                .ok_or_else(|| anyhow::anyhow!("WATCHLIST_FILE required when mode=file"))?;
            watchlist::spawn_file_reload(matcher.clone(), path.into(), cfg.watchlist_refresh);
        }
        "url" => {
            let url = cfg
                .watchlist_url
                .clone()
                .ok_or_else(|| anyhow::anyhow!("WATCHLIST_URL required when mode=url"))?;
            watchlist::spawn_url_reload(matcher.clone(), url, cfg.watchlist_refresh);
        }
        "disabled" => { /* matcher stays empty */ }
        other => anyhow::bail!("unknown WATCHLIST_MODE: {other}"),
    }

    let writer = writer::spawn(
        pool.clone(),
        matcher.clone(),
        counters.clone(),
        stream_tx.clone(),
        10_000,
    );

    if let (Some(url), Some(secret)) = (cfg.webhook_url.clone(), cfg.webhook_hmac_secret.clone()) {
        tracing::info!("webhook dispatcher: target {}", url);
        crate::webhook::spawn(
            pool.clone(),
            counters.clone(),
            crate::webhook::WebhookCfg {
                url,
                hmac_secret: secret,
                max_attempts: cfg.webhook_max_attempts,
            },
        );
    } else {
        tracing::info!("webhook dispatcher disabled (WEBHOOK_URL/WEBHOOK_HMAC_SECRET unset)");
    }

    spawn_ingest_workers(pool.clone(), writer.clone(), &cfg).await?;
    spawn_names_pruner(pool.clone(), cfg.retention_days_cold);

    let state = crate::api::AppState {
        pool: pool.clone(),
        counters,
        metrics_handle,
        config: Arc::new(cfg.clone()),
        stream_tx,
    };
    let app = crate::api::router(state);
    let addr: SocketAddr = cfg.listen_addr.parse()?;
    let listener = TcpListener::bind(addr).await?;
    tracing::info!("listening on http://{}", addr);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn spawn_ingest_workers(
    pool: sqlx::PgPool,
    writer: writer::WriterHandle,
    cfg: &Config,
) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("ctwatch/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let logs = log_list::fetch_log_list(&client, &cfg.log_list_url).await?;
    let logs = log_list::filter_operators(logs, &cfg.log_operators);
    let usable: Vec<_> = logs
        .into_iter()
        .filter(|l| matches!(l.state, log_list::LogState::Usable))
        .collect();
    tracing::info!("starting ingest for {} usable logs", usable.len());
    for log in usable {
        ingest_worker::spawn(
            pool.clone(),
            writer.clone(),
            log,
            ingest_worker::WorkerCfg {
                poll_interval: cfg.log_poll_interval,
                batch_size: cfg.log_entry_batch_size,
            },
        );
    }

    // Periodic refresh of log_list every 6h (handles new/decommissioned logs).
    let pool_refresh = pool.clone();
    let writer_refresh = writer.clone();
    let url = cfg.log_list_url.clone();
    let operators = cfg.log_operators.clone();
    let poll = cfg.log_poll_interval;
    let batch = cfg.log_entry_batch_size;
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(6 * 3600));
        tick.tick().await; // skip the immediate tick
        loop {
            tick.tick().await;
            let Ok(client) = reqwest::Client::builder().build() else {
                continue;
            };
            let Ok(logs) = log_list::fetch_log_list(&client, &url).await else {
                continue;
            };
            let logs = log_list::filter_operators(logs, &operators);
            // Spawn workers for newly-seen logs (existing workers self-resume).
            let existing: std::collections::HashSet<Vec<u8>> = store::cursors::list(&pool_refresh)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|c| c.log_id)
                .collect();
            for log in logs
                .into_iter()
                .filter(|l| matches!(l.state, log_list::LogState::Usable))
            {
                if !existing.contains(&log.log_id) {
                    tracing::info!(op = %log.operator, url = %log.url, "spawning worker for new log");
                    ingest_worker::spawn(
                        pool_refresh.clone(),
                        writer_refresh.clone(),
                        log,
                        ingest_worker::WorkerCfg {
                            poll_interval: poll,
                            batch_size: batch,
                        },
                    );
                }
            }
        }
    });
    Ok(())
}

fn spawn_names_pruner(pool: sqlx::PgPool, retention_days: u32) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(24 * 3600));
        tick.tick().await; // immediate tick
        loop {
            tick.tick().await;
            let cutoff = chrono::Utc::now() - chrono::Duration::days(retention_days as i64);
            match store::names::prune_older_than(&pool, cutoff).await {
                Ok(n) => {
                    tracing::info!("pruned {n} names_observed rows older than {retention_days}d")
                }
                Err(e) => tracing::warn!("names prune: {e:#}"),
            }
        }
    });
}
