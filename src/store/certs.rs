use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::PgPool;

#[derive(Debug, Clone)]
pub struct CertRow {
    pub cert_hash: Vec<u8>,
    pub issuer_hash: Vec<u8>,
    pub issuer_cn: String,
    pub not_before: DateTime<Utc>,
    pub not_after: DateTime<Utc>,
    pub sans: Vec<String>,
    pub registered_domains: Vec<String>,
}

/// Insert a batch of certs with ON CONFLICT DO NOTHING. Returns the number of
/// rows actually inserted (so the caller can compute dedup rates).
pub async fn insert_batch(pool: &PgPool, batch: &[CertRow]) -> Result<u64> {
    if batch.is_empty() {
        return Ok(0);
    }
    let mut tx = pool.begin().await?;
    let mut inserted_certs = 0u64;

    for row in batch {
        let r = sqlx::query!(
            r#"INSERT INTO certs (cert_hash, issuer_hash, issuer_cn, not_before, not_after, sans, registered_domains)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               ON CONFLICT (cert_hash, not_before) DO NOTHING"#,
            row.cert_hash,
            row.issuer_hash,
            row.issuer_cn,
            row.not_before,
            row.not_after,
            &row.sans,
            &row.registered_domains
        )
        .execute(&mut *tx)
        .await?;
        inserted_certs += r.rows_affected();

        if r.rows_affected() > 0 {
            for name in &row.sans {
                let regdom = crate::parse::registered_domain(name).unwrap_or_else(|| name.clone());
                sqlx::query!(
                    r#"INSERT INTO cert_names (name, registered_domain, cert_hash, not_before)
                       VALUES ($1, $2, $3, $4)
                       ON CONFLICT DO NOTHING"#,
                    name,
                    regdom,
                    row.cert_hash,
                    row.not_before
                )
                .execute(&mut *tx)
                .await?;
            }
        }
    }
    tx.commit().await?;
    Ok(inserted_certs)
}

pub async fn count_in_window(pool: &PgPool, since: DateTime<Utc>) -> Result<i64> {
    let r = sqlx::query!(
        "SELECT count(*) as n FROM certs WHERE not_before >= $1",
        since
    )
    .fetch_one(pool)
    .await?;
    Ok(r.n.unwrap_or(0))
}

pub async fn get_one(pool: &PgPool, cert_hash: &[u8]) -> Result<Option<CertRow>> {
    let row = sqlx::query!(
        r#"SELECT cert_hash, issuer_hash, issuer_cn, not_before, not_after, sans, registered_domains
           FROM certs WHERE cert_hash = $1 ORDER BY not_before DESC LIMIT 1"#,
        cert_hash
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| CertRow {
        cert_hash: r.cert_hash,
        issuer_hash: r.issuer_hash,
        issuer_cn: r.issuer_cn,
        not_before: r.not_before,
        not_after: r.not_after,
        sans: r.sans,
        registered_domains: r.registered_domains,
    }))
}

pub async fn list_by_registered_domain(
    pool: &PgPool,
    domain: &str,
    since: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<CertRow>> {
    let rows = sqlx::query!(
        r#"SELECT cert_hash, issuer_hash, issuer_cn, not_before, not_after, sans, registered_domains
           FROM certs
           WHERE registered_domains && ARRAY[$1]::TEXT[]
             AND not_before >= $2
           ORDER BY not_before DESC
           LIMIT $3"#,
        domain,
        since,
        limit
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| CertRow {
            cert_hash: r.cert_hash,
            issuer_hash: r.issuer_hash,
            issuer_cn: r.issuer_cn,
            not_before: r.not_before,
            not_after: r.not_after,
            sans: r.sans,
            registered_domains: r.registered_domains,
        })
        .collect())
}
