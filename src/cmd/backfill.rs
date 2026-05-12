use anyhow::Result;
use chrono::{DateTime, Utc};
use clap::Args;

use crate::ingest::{backfill_worker, log_list};
use crate::stats::Counters;
use crate::{config::Config, observability, store};
use crate::{stream, watchlist, writer};

#[derive(Args)]
pub struct BackfillArgs {
    /// RFC 3339 timestamp; backfill begins at this point and walks forward to current STH.
    #[arg(long)]
    pub since: String,
    /// Optional hex log_id to restrict backfill to one log (default: all usable).
    #[arg(long)]
    pub log: Option<String>,
    /// Concurrent backfill workers across logs (default 4).
    #[arg(long, default_value_t = 4)]
    pub concurrency: usize,
    /// Per-log get-entries QPS ceiling (default 5).
    #[arg(long, default_value_t = 5)]
    pub rate_limit_qps: u32,
}

pub async fn run(args: BackfillArgs) -> Result<()> {
    let cfg = Config::load()?;
    observability::init_tracing(&cfg.log_level)?;
    let _metrics = observability::init_metrics()?;
    let since: DateTime<Utc> = args.since.parse()?;

    let pool = store::pool(&cfg.database_url).await?;
    store::migrate(&pool).await?;
    let counters = Counters::new();
    counters.spawn_ticker();
    let (stream_tx, _rx) = stream::channel(cfg.stream_max_lag.max(64));
    let matcher = watchlist::Matcher::new(); // empty; matches won't fire during backfill
    let writer = writer::spawn(pool.clone(), matcher, counters.clone(), stream_tx, 10_000);

    let client = reqwest::Client::builder().build()?;
    let all = log_list::fetch_log_list(&client, &cfg.log_list_url).await?;
    let mut usable: Vec<_> = log_list::filter_operators(all, &cfg.log_operators)
        .into_iter()
        .filter(|l| matches!(l.state, log_list::LogState::Usable))
        .collect();

    if let Some(hex_log) = args.log {
        let want = hex::decode(&hex_log)?;
        usable.retain(|l| l.log_id == want);
    }
    tracing::info!("backfilling {} logs from {since}", usable.len());

    backfill_worker::run_many(
        pool,
        writer,
        usable,
        since,
        args.concurrency,
        args.rate_limit_qps,
        cfg.log_entry_batch_size,
    )
    .await?;
    tracing::info!("backfill complete");
    Ok(())
}
