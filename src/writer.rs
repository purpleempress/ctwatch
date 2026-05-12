use anyhow::Result;
use chrono::Utc;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::ingest::entry::{DecodedEntry, EntryKind};
use crate::stats::Counters;
use crate::store::{certs, names, outbox};
use crate::stream::{CertEvent, CertEventCert, CertEventLog, CertEventSender};
use crate::watchlist::Matcher;

/// Item handed from an ingest worker to the writer.
#[derive(Debug)]
pub struct WriterJob {
    pub log_id: Vec<u8>,
    pub log_url: String,
    pub operator: String,
    pub entry: DecodedEntry,
}

#[derive(Clone)]
pub struct WriterHandle {
    pub tx: mpsc::Sender<WriterJob>,
}

pub fn spawn(
    pool: PgPool,
    matcher: Matcher,
    counters: Counters,
    stream_tx: CertEventSender,
    capacity: usize,
) -> WriterHandle {
    let (tx, mut rx) = mpsc::channel::<WriterJob>(capacity);
    tokio::spawn(async move {
        let mut batch: Vec<WriterJob> = Vec::with_capacity(100);
        loop {
            // Block for at least one item, then drain up to 100 more without waiting.
            let Some(first) = rx.recv().await else {
                break;
            };
            batch.clear();
            batch.push(first);
            while let Ok(j) = rx.try_recv() {
                batch.push(j);
                if batch.len() >= 100 {
                    break;
                }
            }
            counters.set_queue_depth(rx.len() as i64);

            if let Err(e) = flush_batch(&pool, &matcher, &counters, &stream_tx, &batch).await {
                tracing::error!("writer flush: {e:#}");
            }
        }
    });
    WriterHandle { tx }
}

async fn flush_batch(
    pool: &PgPool,
    matcher: &Matcher,
    counters: &Counters,
    stream_tx: &CertEventSender,
    batch: &[WriterJob],
) -> Result<()> {
    let mut cert_rows: Vec<certs::CertRow> = Vec::with_capacity(batch.len());
    let mut events: Vec<CertEvent> = Vec::with_capacity(batch.len());
    let now = Utc::now();

    for job in batch {
        let (precert_der, _tbs) = match (&job.entry.kind, &job.entry.precert_der) {
            (EntryKind::Precert { tbs, .. }, Some(der)) => (der.clone(), tbs.clone()),
            _ => continue, // final certs dropped per §6.4
        };
        let cert_hash = Sha256::digest(&precert_der).to_vec();

        let Ok(parsed) = parse_cert(&precert_der) else {
            continue;
        };
        let regdoms = parsed.registered_domains.clone();

        cert_rows.push(certs::CertRow {
            cert_hash: cert_hash.clone(),
            issuer_hash: parsed.issuer_hash,
            issuer_cn: parsed.issuer_cn.clone(),
            not_before: parsed.not_before,
            not_after: parsed.not_after,
            sans: parsed.sans.clone(),
            registered_domains: regdoms.clone(),
        });

        events.push(CertEvent {
            event: "precert",
            observed_at: now,
            cert: CertEventCert {
                cert_hash: cert_hash.clone(),
                issuer_cn: parsed.issuer_cn,
                not_before: parsed.not_before,
                not_after: parsed.not_after,
                sans: parsed.sans,
                registered_domains: regdoms.clone(),
            },
            log: CertEventLog {
                operator: job.operator.clone(),
                log_id: job.log_id.clone(),
            },
        });
    }

    // Insert certs (deduped). cert_names insert happens inside insert_batch.
    let inserted = certs::insert_batch(pool, &cert_rows).await?;
    let attempted = cert_rows.len() as u64;
    counters.incr_certs(inserted);
    counters.incr_dups(attempted.saturating_sub(inserted));

    // Upsert names_observed for every SAN (not just newly-inserted certs — same name
    // appearing on a duplicate cert still advances last_seen).
    for row in &cert_rows {
        for name in &row.sans {
            let regdom = crate::parse::registered_domain(name).unwrap_or_else(|| name.clone());
            if let Err(e) = names::upsert(pool, name, &regdom, row.not_before).await {
                tracing::warn!("names upsert {name}: {e}");
            }
        }
    }

    // Watchlist match + outbox enqueue for newly-inserted only.
    // (Conservative: dedup matches don't refire the webhook.)
    for (row, evt) in cert_rows.iter().zip(events.iter()) {
        if inserted == 0 {
            break;
        }
        let matches = matcher.matches(&row.registered_domains).await;
        if !matches.is_empty() {
            counters.incr_watchlist();
            let body = serde_json::json!({
                "event_id": Uuid::new_v4().to_string(),
                "event": "precert.match",
                "matched_at": evt.observed_at,
                "matched_watchlist_entries": matches,
                "cert": evt.cert,
            });
            if let Err(e) = outbox::enqueue(pool, &body).await {
                tracing::warn!("outbox enqueue: {e}");
            }
        }
    }

    // Stream broadcast — non-blocking. Lagging subscribers are dropped by the broadcast itself.
    for evt in events {
        let _ = stream_tx.send(evt);
        counters.incr_stream_sent(1);
    }

    Ok(())
}

struct ParsedCert {
    issuer_hash: Vec<u8>,
    issuer_cn: String,
    not_before: chrono::DateTime<Utc>,
    not_after: chrono::DateTime<Utc>,
    sans: Vec<String>,
    registered_domains: Vec<String>,
}

fn parse_cert(der: &[u8]) -> Result<ParsedCert> {
    use x509_parser::prelude::*;
    let (_, cert) = X509Certificate::from_der(der).map_err(|e| anyhow::anyhow!("x509: {e}"))?;
    let sans = crate::parse::extract_sans_from_der(der)?;
    let registered_domains: std::collections::BTreeSet<String> = sans
        .iter()
        .filter_map(|s| crate::parse::registered_domain(s))
        .collect();
    let issuer_cn = cert
        .issuer()
        .iter_common_name()
        .next()
        .and_then(|cn| cn.as_str().ok())
        .unwrap_or("")
        .to_string();
    let issuer_hash = Sha256::digest(cert.issuer().as_raw()).to_vec();
    let nb = chrono::DateTime::<Utc>::from_timestamp(cert.validity().not_before.timestamp(), 0)
        .ok_or_else(|| anyhow::anyhow!("not_before out of range"))?;
    let na = chrono::DateTime::<Utc>::from_timestamp(cert.validity().not_after.timestamp(), 0)
        .ok_or_else(|| anyhow::anyhow!("not_after out of range"))?;
    Ok(ParsedCert {
        issuer_hash,
        issuer_cn,
        not_before: nb,
        not_after: na,
        sans,
        registered_domains: registered_domains.into_iter().collect(),
    })
}
