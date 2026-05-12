use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

pub async fn enqueue(pool: &PgPool, body: &Value) -> Result<Uuid> {
    let event_id = Uuid::new_v4();
    sqlx::query!(
        r#"INSERT INTO webhook_outbox (event_id, body) VALUES ($1, $2)"#,
        event_id,
        body
    )
    .execute(pool)
    .await?;
    Ok(event_id)
}

pub async fn pending_count(pool: &PgPool) -> Result<i64> {
    let r = sqlx::query!("SELECT count(*) as n FROM webhook_outbox WHERE delivered_at IS NULL")
        .fetch_one(pool)
        .await?;
    Ok(r.n.unwrap_or(0))
}

pub async fn failed_count(pool: &PgPool, max_attempts: i32) -> Result<i64> {
    let r = sqlx::query!(
        "SELECT count(*) as n FROM webhook_outbox
         WHERE delivered_at IS NULL AND attempts >= $1",
        max_attempts
    )
    .fetch_one(pool)
    .await?;
    Ok(r.n.unwrap_or(0))
}

#[derive(Debug, Clone)]
pub struct DueRow {
    pub id: i64,
    pub event_id: Uuid,
    pub body: serde_json::Value,
    pub attempts: i32,
}

pub async fn due(pool: &PgPool, limit: i64) -> Result<Vec<DueRow>> {
    let rows = sqlx::query!(
        r#"SELECT id, event_id, body, attempts
           FROM webhook_outbox
           WHERE delivered_at IS NULL
             AND next_attempt <= now()
           ORDER BY next_attempt ASC
           LIMIT $1"#,
        limit
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| DueRow {
            id: r.id,
            event_id: r.event_id,
            body: r.body,
            attempts: r.attempts,
        })
        .collect())
}

pub async fn mark_attempt(
    pool: &PgPool,
    id: i64,
    status: i32,
    next_attempt: DateTime<Utc>,
) -> Result<()> {
    sqlx::query!(
        r#"UPDATE webhook_outbox
           SET attempts = attempts + 1,
               last_attempt = now(),
               last_status = $2,
               next_attempt = $3
           WHERE id = $1"#,
        id,
        status,
        next_attempt
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_delivered(pool: &PgPool, id: i64, status: i32) -> Result<()> {
    sqlx::query!(
        r#"UPDATE webhook_outbox
           SET attempts = attempts + 1,
               last_attempt = now(),
               last_status = $2,
               delivered_at = now()
           WHERE id = $1"#,
        id,
        status
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn force_redeliver(pool: &PgPool, event_id: Uuid) -> Result<bool> {
    let r = sqlx::query!(
        r#"UPDATE webhook_outbox
           SET delivered_at = NULL,
               next_attempt = now()
           WHERE event_id = $1"#,
        event_id
    )
    .execute(pool)
    .await?;
    Ok(r.rows_affected() == 1)
}
