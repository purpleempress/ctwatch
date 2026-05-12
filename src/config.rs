use figment::{
    providers::{Env, Format, Yaml},
    Figment,
};
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub database_url: String,
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,
    #[serde(default = "default_log_list_url")]
    pub log_list_url: String,
    #[serde(default = "default_log_operators")]
    pub log_operators: String,
    #[serde(default = "default_poll_interval_s", with = "humantime_serde_seconds")]
    pub log_poll_interval: Duration,
    #[serde(default = "default_entry_batch")]
    pub log_entry_batch_size: u32,
    #[serde(default = "default_retention_hot")]
    pub retention_days_hot: u32,
    #[serde(default = "default_retention_cold")]
    pub retention_days_cold: u32,
    #[serde(default = "default_watchlist_mode")]
    pub watchlist_mode: String,
    #[serde(default = "default_stream_enabled")]
    pub stream_enabled: bool,
    #[serde(default = "default_stream_max_lag")]
    pub stream_max_lag: usize,
    #[serde(default = "default_stream_max_subs")]
    pub stream_max_subscribers: usize,
    #[serde(default = "default_crtsh")]
    pub crtsh_base_url: String,
    #[serde(default = "default_metrics_enabled")]
    pub metrics_enabled: bool,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    pub webhook_url: Option<String>,
    pub webhook_hmac_secret: Option<String>,
    #[serde(default = "default_webhook_max_attempts")]
    pub webhook_max_attempts: u32,
    pub watchlist_file: Option<String>,
    pub watchlist_url: Option<String>,
    #[serde(
        default = "default_watchlist_refresh",
        with = "humantime_serde_seconds"
    )]
    pub watchlist_refresh: Duration,
}

fn default_listen_addr() -> String {
    "0.0.0.0:8080".into()
}
fn default_log_list_url() -> String {
    "https://www.gstatic.com/ct/log_list/v3/log_list.json".into()
}
fn default_log_operators() -> String {
    "*".into()
}
fn default_poll_interval_s() -> Duration {
    Duration::from_secs(30)
}
fn default_entry_batch() -> u32 {
    1000
}
fn default_retention_hot() -> u32 {
    90
}
fn default_retention_cold() -> u32 {
    400
}
fn default_watchlist_mode() -> String {
    "db".into()
}
fn default_stream_enabled() -> bool {
    true
}
fn default_stream_max_lag() -> usize {
    64
}
fn default_stream_max_subs() -> usize {
    100
}
fn default_crtsh() -> String {
    "https://crt.sh".into()
}
fn default_metrics_enabled() -> bool {
    true
}
fn default_log_level() -> String {
    "info".into()
}
fn default_webhook_max_attempts() -> u32 {
    7
}
fn default_watchlist_refresh() -> Duration {
    Duration::from_secs(60)
}

mod humantime_serde_seconds {
    use serde::{Deserialize, Deserializer};
    use std::time::Duration;
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let s = String::deserialize(d)?;
        let secs: u64 = if let Some(stripped) = s.strip_suffix('s') {
            stripped.parse().map_err(serde::de::Error::custom)?
        } else if let Some(stripped) = s.strip_suffix('m') {
            let m: u64 = stripped.parse().map_err(serde::de::Error::custom)?;
            m * 60
        } else {
            s.parse().map_err(serde::de::Error::custom)?
        };
        Ok(Duration::from_secs(secs))
    }
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        let yaml_path = std::env::var("CTWATCH_CONFIG").ok();
        let mut fig = Figment::new();
        if let Some(p) = yaml_path {
            fig = fig.merge(Yaml::file(p));
        }
        let cfg: Self = fig
            .merge(Env::raw().lowercase(true))
            .extract()
            .map_err(|e| anyhow::anyhow!("config: {e}"))?;
        Ok(cfg)
    }
}
