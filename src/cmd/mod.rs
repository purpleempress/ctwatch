use anyhow::Result;
use clap::Subcommand;

pub mod backfill;
pub mod healthcheck;
pub mod migrate;
pub mod serve;

#[derive(Subcommand)]
pub enum Command {
    /// Run migrations, then start ingest + HTTP server
    Serve,
    /// Apply pending migrations and exit
    Migrate,
    /// Container probe — exits 0 if the local server is healthy
    Healthcheck,
    /// Backfill CT log entries from a given timestamp to the current STH
    Backfill(backfill::BackfillArgs),
}

pub async fn dispatch(cmd: Command) -> Result<()> {
    match cmd {
        Command::Serve => serve::run().await,
        Command::Migrate => migrate::run().await,
        Command::Healthcheck => healthcheck::run().await,
        Command::Backfill(args) => backfill::run(args).await,
    }
}
