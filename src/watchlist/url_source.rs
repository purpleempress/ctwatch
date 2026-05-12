use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Resp {
    domains: Vec<String>,
}

pub async fn fetch(client: &reqwest::Client, url: &str) -> Result<Vec<String>> {
    let resp: Resp = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .context("parse watchlist URL response")?;
    Ok(resp.domains)
}
