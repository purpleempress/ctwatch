use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::PgPool;

pub async fn upsert(
    pool: &PgPool,
    name: &str,
    registered_domain: &str,
    now: DateTime<Utc>,
) -> Result<()> {
    sqlx::query!(
        r#"INSERT INTO names_observed (name, registered_domain, first_seen, last_seen, cert_count)
           VALUES ($1, $2, $3, $3, 1)
           ON CONFLICT (name) DO UPDATE
             SET last_seen = excluded.last_seen,
                 cert_count = names_observed.cert_count + 1"#,
        name,
        registered_domain,
        now
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct NameRow {
    pub name: String,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub cert_count: i32,
}

pub async fn list_by_registered_domain(
    pool: &PgPool,
    domain: &str,
    since: DateTime<Utc>,
    contains: Option<&str>,
    limit: i64,
) -> Result<Vec<NameRow>> {
    let like = contains.map(|c| format!("%{c}%"));
    let rows = sqlx::query!(
        r#"SELECT name, first_seen, last_seen, cert_count
           FROM names_observed
           WHERE registered_domain = $1
             AND last_seen >= $2
             AND ($3::TEXT IS NULL OR name LIKE $3)
           ORDER BY last_seen DESC
           LIMIT $4"#,
        domain,
        since,
        like,
        limit
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| NameRow {
            name: r.name,
            first_seen: r.first_seen,
            last_seen: r.last_seen,
            cert_count: r.cert_count,
        })
        .collect())
}

pub async fn count(pool: &PgPool) -> Result<i64> {
    let r = sqlx::query!("SELECT count(*) as n FROM names_observed")
        .fetch_one(pool)
        .await?;
    Ok(r.n.unwrap_or(0))
}

pub async fn prune_older_than(pool: &PgPool, cutoff: DateTime<Utc>) -> Result<u64> {
    let r = sqlx::query!("DELETE FROM names_observed WHERE last_seen < $1", cutoff)
        .execute(pool)
        .await?;
    Ok(r.rows_affected())
}
