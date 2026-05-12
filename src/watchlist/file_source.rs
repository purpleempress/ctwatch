use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct File {
    domains: Vec<String>,
}

pub fn load(path: &Path) -> Result<Vec<String>> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("read {path:?}"))?;
    let file: File = serde_yaml::from_str(&raw).context("parse YAML")?;
    Ok(file.domains)
}
