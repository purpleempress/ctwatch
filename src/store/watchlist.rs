use anyhow::Result;
use sqlx::PgPool;

pub async fn add(pool: &PgPool, domain: &str, notes: Option<&str>) -> Result<bool> {
    let r = sqlx::query!(
        r#"INSERT INTO watchlist (domain, notes) VALUES ($1, $2)
           ON CONFLICT (domain) DO NOTHING"#,
        domain,
        notes
    )
    .execute(pool)
    .await?;
    Ok(r.rows_affected() == 1)
}

pub async fn remove(pool: &PgPool, domain: &str) -> Result<bool> {
    let r = sqlx::query!("DELETE FROM watchlist WHERE domain = $1", domain)
        .execute(pool)
        .await?;
    Ok(r.rows_affected() == 1)
}

pub async fn list(pool: &PgPool) -> Result<Vec<String>> {
    let rows = sqlx::query!("SELECT domain FROM watchlist ORDER BY domain")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|r| r.domain).collect())
}

pub async fn count(pool: &PgPool) -> Result<i64> {
    let r = sqlx::query!("SELECT count(*) as n FROM watchlist")
        .fetch_one(pool)
        .await?;
    Ok(r.n.unwrap_or(0))
}
