use crate::{config::Config, observability, store};
use anyhow::Result;

pub async fn run() -> Result<()> {
    let cfg = Config::load()?;
    observability::init_tracing(&cfg.log_level)?;
    let pool = store::pool(&cfg.database_url).await?;
    store::migrate(&pool).await?;
    tracing::info!("migrations applied");
    Ok(())
}
