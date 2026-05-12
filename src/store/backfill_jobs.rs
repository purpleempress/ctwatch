use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::PgPool;

pub async fn active_count(pool: &PgPool) -> Result<i64> {
    let r = sqlx::query!("SELECT count(*) as n FROM backfill_jobs WHERE status = 'running'")
        .fetch_one(pool)
        .await?;
    Ok(r.n.unwrap_or(0))
}

pub async fn completed_count(pool: &PgPool) -> Result<i64> {
    let r = sqlx::query!("SELECT count(*) as n FROM backfill_jobs WHERE status = 'completed'")
        .fetch_one(pool)
        .await?;
    Ok(r.n.unwrap_or(0))
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: i64,
    pub log_id: Vec<u8>,
    pub target_since: DateTime<Utc>,
    pub target_start_index: i64,
    pub target_end_index: i64,
    pub progress_index: i64,
    pub status: String,
}

pub async fn insert(
    pool: &PgPool,
    log_id: &[u8],
    target_since: DateTime<Utc>,
    target_start_index: i64,
    target_end_index: i64,
) -> Result<i64> {
    let r = sqlx::query!(
        r#"INSERT INTO backfill_jobs
             (log_id, target_since, target_start_index, target_end_index, progress_index)
           VALUES ($1, $2, $3, $4, $3)
           RETURNING id"#,
        log_id,
        target_since,
        target_start_index,
        target_end_index
    )
    .fetch_one(pool)
    .await?;
    Ok(r.id)
}

pub async fn find_active(
    pool: &PgPool,
    log_id: &[u8],
    target_since: DateTime<Utc>,
) -> Result<Option<Job>> {
    let row = sqlx::query!(
        r#"SELECT id, log_id, target_since, target_start_index, target_end_index, progress_index, status
           FROM backfill_jobs
           WHERE log_id = $1 AND target_since = $2 AND status = 'running'
           LIMIT 1"#,
        log_id,
        target_since
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| Job {
        id: r.id,
        log_id: r.log_id,
        target_since: r.target_since,
        target_start_index: r.target_start_index,
        target_end_index: r.target_end_index,
        progress_index: r.progress_index,
        status: r.status,
    }))
}

pub async fn update_progress(pool: &PgPool, id: i64, progress: i64) -> Result<()> {
    sqlx::query!(
        "UPDATE backfill_jobs SET progress_index = $2 WHERE id = $1",
        id,
        progress
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn complete(pool: &PgPool, id: i64) -> Result<()> {
    sqlx::query!(
        "UPDATE backfill_jobs SET status = 'completed', completed_at = now() WHERE id = $1",
        id
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn fail(pool: &PgPool, id: i64, error: &str) -> Result<()> {
    sqlx::query!(
        "UPDATE backfill_jobs SET status = 'failed', completed_at = now(), last_error = $2 WHERE id = $1",
        id,
        error
    )
    .execute(pool)
    .await?;
    Ok(())
}
