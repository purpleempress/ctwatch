use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct SignedTreeHead {
    pub tree_size: u64,
    pub timestamp_ms: u64,
    pub sha256_root_hash: Vec<u8>,
    pub tree_head_signature: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct RawEntry {
    pub leaf_input: Vec<u8>,
    pub extra_data: Vec<u8>,
}

#[derive(Deserialize)]
struct RawSth {
    tree_size: u64,
    timestamp: u64,
    sha256_root_hash: String,
    tree_head_signature: String,
}

#[derive(Deserialize)]
struct RawEntriesResp {
    entries: Vec<RawEntryJson>,
}

#[derive(Deserialize)]
struct RawEntryJson {
    leaf_input: String,
    extra_data: String,
}

#[derive(Clone)]
pub struct CtClient {
    inner: reqwest::Client,
}

impl CtClient {
    pub fn new() -> Self {
        let inner = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .pool_idle_timeout(Duration::from_secs(60))
            .user_agent(concat!("ctwatch/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client builds");
        Self { inner }
    }

    pub async fn get_sth(&self, base_url: &str) -> Result<SignedTreeHead> {
        use base64::Engine;
        let url = format!("{base_url}ct/v1/get-sth");
        let raw: RawSth = self
            .inner
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(SignedTreeHead {
            tree_size: raw.tree_size,
            timestamp_ms: raw.timestamp,
            sha256_root_hash: base64::engine::general_purpose::STANDARD
                .decode(&raw.sha256_root_hash)?,
            tree_head_signature: base64::engine::general_purpose::STANDARD
                .decode(&raw.tree_head_signature)?,
        })
    }

    pub async fn get_entries(&self, base_url: &str, start: u64, end: u64) -> Result<Vec<RawEntry>> {
        use base64::Engine;
        if end < start {
            return Err(anyhow!("end < start"));
        }
        let url = format!("{base_url}ct/v1/get-entries?start={start}&end={end}");
        let raw: RawEntriesResp = self
            .inner
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let mut out = Vec::with_capacity(raw.entries.len());
        for e in raw.entries {
            out.push(RawEntry {
                leaf_input: base64::engine::general_purpose::STANDARD.decode(&e.leaf_input)?,
                extra_data: base64::engine::general_purpose::STANDARD.decode(&e.extra_data)?,
            });
        }
        Ok(out)
    }
}

impl Default for CtClient {
    fn default() -> Self {
        Self::new()
    }
}
