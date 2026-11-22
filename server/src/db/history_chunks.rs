use sqlx::PgExecutor;
use uuid::Uuid;

pub struct Chunk {
    pub account_id: Uuid,
    pub message_id: String,
    pub sealed: Vec<u8>,
}

/// The phone may send a chunk again; it's kept once
pub async fn save(
    db: impl PgExecutor<'_>,
    account_id: Uuid,
    message_id: &str,
    chunk: &[u8],
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO history_chunks (account_id, message_id, chunk)
         VALUES ($1, $2, $3)
         ON CONFLICT DO NOTHING",
        account_id,
        message_id,
        chunk,
    )
    .execute(db)
    .await?;
    Ok(())
}

/// The chunk waiting longest, from any account
pub async fn oldest(db: impl PgExecutor<'_>) -> sqlx::Result<Option<Chunk>> {
    sqlx::query_as!(
        Chunk,
        "SELECT account_id, message_id, chunk AS sealed FROM history_chunks
         ORDER BY received_at, message_id
         LIMIT 1"
    )
    .fetch_optional(db)
    .await
}

pub async fn delete(
    db: impl PgExecutor<'_>,
    account_id: Uuid,
    message_id: &str,
) -> sqlx::Result<()> {
    sqlx::query!(
        "DELETE FROM history_chunks WHERE account_id = $1 AND message_id = $2",
        account_id,
        message_id,
    )
    .execute(db)
    .await?;
    Ok(())
}

/// How many chunks an account still has to import
pub async fn waiting(db: impl PgExecutor<'_>, account_id: Uuid) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "waiting!" FROM history_chunks WHERE account_id = $1"#,
        account_id
    )
    .fetch_one(db)
    .await
}
