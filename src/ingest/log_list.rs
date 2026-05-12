use anyhow::Result;
use base64::Engine;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogState {
    Usable,
    Pending,
    Qualified,
    Readonly,
    Retired,
    Rejected,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct CtLog {
    pub log_id: Vec<u8>, // sha256 of public key, 32 bytes
    pub url: String,     // ends with /
    pub operator: String,
    pub state: LogState,
}

#[derive(Deserialize)]
struct RawList {
    operators: Vec<RawOperator>,
}

#[derive(Deserialize)]
struct RawOperator {
    name: String,
    logs: Vec<RawLog>,
}

#[derive(Deserialize)]
struct RawLog {
    log_id: String, // base64 of sha256(public_key)
    url: String,
    #[serde(default)]
    state: serde_json::Value,
}

pub fn parse_log_list(raw: &str) -> Result<Vec<CtLog>> {
    let list: RawList = serde_json::from_str(raw)?;
    let mut out = Vec::new();
    for op in list.operators {
        for log in op.logs {
            let log_id = base64::engine::general_purpose::STANDARD.decode(&log.log_id)?;
            let state = parse_state(&log.state);
            let mut url = log.url.clone();
            if !url.ends_with('/') {
                url.push('/');
            }
            out.push(CtLog {
                log_id,
                url,
                operator: op.name.clone(),
                state,
            });
        }
    }
    Ok(out)
}

fn parse_state(v: &serde_json::Value) -> LogState {
    let Some(obj) = v.as_object() else {
        return LogState::Unknown;
    };
    if obj.contains_key("usable") {
        return LogState::Usable;
    }
    if obj.contains_key("pending") {
        return LogState::Pending;
    }
    if obj.contains_key("qualified") {
        return LogState::Qualified;
    }
    if obj.contains_key("readonly") {
        return LogState::Readonly;
    }
    if obj.contains_key("retired") {
        return LogState::Retired;
    }
    if obj.contains_key("rejected") {
        return LogState::Rejected;
    }
    LogState::Unknown
}

pub async fn fetch_log_list(client: &reqwest::Client, url: &str) -> Result<Vec<CtLog>> {
    let body = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse_log_list(&body)
}

pub fn filter_operators(logs: Vec<CtLog>, allowlist: &str) -> Vec<CtLog> {
    if allowlist.trim() == "*" {
        return logs;
    }
    let allow: Vec<&str> = allowlist.split(',').map(str::trim).collect();
    logs.into_iter()
        .filter(|l| allow.iter().any(|a| a.eq_ignore_ascii_case(&l.operator)))
        .collect()
}
