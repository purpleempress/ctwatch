use chrono::{Duration, Utc};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use sqlx::PgPool;
use std::time::Duration as StdDuration;

use crate::stats::Counters;
use crate::store::outbox;

type HmacSha256 = Hmac<Sha256>;

pub fn spawn(pool: PgPool, counters: Counters, cfg: WebhookCfg) {
    tokio::spawn(async move { run(pool, counters, cfg).await });
}

#[derive(Clone)]
pub struct WebhookCfg {
    pub url: String,
    pub hmac_secret: String,
    pub max_attempts: u32,
}

async fn run(pool: PgPool, counters: Counters, cfg: WebhookCfg) {
    let http = reqwest::Client::builder()
        .timeout(StdDuration::from_secs(15))
        .build()
        .expect("reqwest builds");

    let mut tick = tokio::time::interval(StdDuration::from_secs(2));
    loop {
        tick.tick().await;
        match outbox::due(&pool, 50).await {
            Ok(due) => {
                for row in due {
                    deliver_one(&pool, &http, &cfg, &counters, row).await;
                }
            }
            Err(e) => tracing::warn!("outbox::due: {e:#}"),
        }
    }
}

async fn deliver_one(
    pool: &PgPool,
    http: &reqwest::Client,
    cfg: &WebhookCfg,
    counters: &Counters,
    row: outbox::DueRow,
) {
    let body = serde_json::to_vec(&row.body).unwrap_or_default();
    let sig = sign(&cfg.hmac_secret, &body);
    let req = http
        .post(&cfg.url)
        .header("Content-Type", "application/json")
        .header("X-Ctwatch-Event", "precert.match")
        .header("X-Ctwatch-Signature", format!("sha256={sig}"))
        .header("X-Ctwatch-Delivery", row.event_id.to_string())
        .body(body);

    match req.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16() as i32;
            if resp.status().is_success() {
                let _ = outbox::mark_delivered(pool, row.id, status).await;
                counters.incr_webhook_delivered();
            } else {
                schedule_retry(pool, counters, cfg, row.id, row.attempts, status).await;
            }
        }
        Err(e) => {
            tracing::warn!(event_id = %row.event_id, "webhook POST: {e:#}");
            schedule_retry(pool, counters, cfg, row.id, row.attempts, 0).await;
        }
    }
}

async fn schedule_retry(
    pool: &PgPool,
    counters: &Counters,
    cfg: &WebhookCfg,
    id: i64,
    attempts: i32,
    status: i32,
) {
    let next_attempts = (attempts as u32) + 1;
    if next_attempts >= cfg.max_attempts {
        let _ = outbox::mark_attempt(pool, id, status, Utc::now() + Duration::days(3650)).await;
        counters.incr_webhook_failed();
        return;
    }
    let delay_secs = 4u64.pow(next_attempts.min(7));
    let next = Utc::now() + Duration::seconds(delay_secs as i64);
    let _ = outbox::mark_attempt(pool, id, status, next).await;
    counters.incr_webhook_retry();
}

fn sign(secret: &str, body: &[u8]) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC-SHA256 accepts any key length");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sign_deterministic() {
        let s = sign("secret", b"hello");
        let s2 = sign("secret", b"hello");
        assert_eq!(s, s2);
        assert_eq!(s.len(), 64); // 32 bytes hex
    }
}
