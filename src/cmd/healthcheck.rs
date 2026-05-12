use crate::config::Config;
use anyhow::Result;

pub async fn run() -> Result<()> {
    let cfg = Config::load()?;
    let url = format!("http://{}/v1/healthz", cfg.listen_addr);
    let resp = reqwest::get(&url).await?;
    if resp.status().is_success() {
        Ok(())
    } else {
        anyhow::bail!("healthcheck failed: {}", resp.status())
    }
}
