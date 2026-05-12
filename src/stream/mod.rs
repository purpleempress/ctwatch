use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize)]
pub struct CertEvent {
    pub event: &'static str, // always "precert" in v0.1
    pub observed_at: DateTime<Utc>,
    pub cert: CertEventCert,
    pub log: CertEventLog,
}

#[derive(Debug, Clone, Serialize)]
pub struct CertEventCert {
    #[serde(serialize_with = "hex_ser")]
    pub cert_hash: Vec<u8>,
    pub issuer_cn: String,
    pub not_before: DateTime<Utc>,
    pub not_after: DateTime<Utc>,
    pub sans: Vec<String>,
    pub registered_domains: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CertEventLog {
    pub operator: String,
    #[serde(serialize_with = "hex_ser")]
    pub log_id: Vec<u8>,
}

fn hex_ser<S: serde::Serializer>(b: &[u8], s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&hex::encode(b))
}

pub type CertEventSender = broadcast::Sender<CertEvent>;
pub type CertEventReceiver = broadcast::Receiver<CertEvent>;

pub fn channel(capacity: usize) -> (CertEventSender, CertEventReceiver) {
    broadcast::channel(capacity)
}

pub mod handler;
