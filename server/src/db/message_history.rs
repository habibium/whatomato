use sqlx::PgExecutor;
use uuid::Uuid;

/// The owner's choice: `None` while every chat isn't kept, `Some(None)` to keep everything,
/// `Some(Some(days))` to keep that many days
pub async fn find(db: impl PgExecutor<'_>, user_id: Uuid) -> sqlx::Result<Option<Option<i32>>> {
    sqlx::query_scalar!(
        "SELECT keep_days FROM message_history WHERE user_id = $1",
        user_id
    )
    .fetch_optional(db)
    .await
}

/// `None` keeps everything
pub async fn save(
    db: impl PgExecutor<'_>,
    user_id: Uuid,
    keep_days: Option<i32>,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO message_history (user_id, keep_days) VALUES ($1, $2)
         ON CONFLICT (user_id) DO UPDATE SET keep_days = EXCLUDED.keep_days, updated_at = now()",
        user_id,
        keep_days,
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn delete(db: impl PgExecutor<'_>, user_id: Uuid) -> sqlx::Result<()> {
    sqlx::query!("DELETE FROM message_history WHERE user_id = $1", user_id)
        .execute(db)
        .await?;
    Ok(())
}

/// Every WhatsApp account whose owner keeps every chat, with how many days
pub async fn by_account(db: impl PgExecutor<'_>) -> sqlx::Result<Vec<(Uuid, Option<i32>)>> {
    let rows = sqlx::query!(
        "SELECT a.id, h.keep_days FROM message_history h
         JOIN whatsapp_accounts a ON a.user_id = h.user_id"
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.id, row.keep_days))
        .collect())
}
