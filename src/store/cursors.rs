use anyhow::Result;
use sqlx::PgPool;

#[derive(Debug, Clone)]
pub struct Cursor {
    pub log_id: Vec<u8>,
    pub log_url: String,
    pub operator: String,
    pub last_tree_size: i64,
    pub state: String,
}

pub async fn upsert(pool: &PgPool, log_id: &[u8], log_url: &str, operator: &str) -> Result<()> {
    sqlx::query!(
        r#"INSERT INTO ingest_cursors (log_id, log_url, operator, last_tree_size, last_updated, state)
           VALUES ($1, $2, $3, 0, now(), 'usable')
           ON CONFLICT (log_id) DO UPDATE
             SET log_url = excluded.log_url,
                 operator = excluded.operator,
                 last_updated = now()"#,
        log_id,
        log_url,
        operator
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn advance(pool: &PgPool, log_id: &[u8], new_tree_size: i64) -> Result<()> {
    sqlx::query!(
        r#"UPDATE ingest_cursors
           SET last_tree_size = $2, last_updated = now()
           WHERE log_id = $1 AND last_tree_size < $2"#,
        log_id,
        new_tree_size
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list(pool: &PgPool) -> Result<Vec<Cursor>> {
    let rows = sqlx::query!(
        r#"SELECT log_id, log_url, operator, last_tree_size, state FROM ingest_cursors"#
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Cursor {
            log_id: r.log_id,
            log_url: r.log_url,
            operator: r.operator,
            last_tree_size: r.last_tree_size,
            state: r.state,
        })
        .collect())
}

pub async fn set_state(pool: &PgPool, log_id: &[u8], state: &str) -> Result<()> {
    sqlx::query!(
        "UPDATE ingest_cursors SET state = $2 WHERE log_id = $1",
        log_id,
        state
    )
    .execute(pool)
    .await?;
    Ok(())
}
