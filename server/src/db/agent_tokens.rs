use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor};
use uuid::Uuid;

pub struct NewTokens<'a> {
    pub connection_id: Uuid,
    pub access_hash: &'a [u8],
    pub refresh_hash: &'a [u8],
    pub access_expires_at: DateTime<Utc>,
    pub refresh_expires_at: DateTime<Utc>,
}

/// A refresh token's row, as read under its lock
pub struct Refresh {
    pub id: Uuid,
    pub connection_id: Uuid,
    /// The client the connection was signed in to with
    pub client_id: Option<String>,
    /// When this token was exchanged for a newer one
    pub replaced_at: Option<DateTime<Utc>>,
    pub expired: bool,
}

pub async fn insert(db: impl PgExecutor<'_>, new: NewTokens<'_>) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO agent_tokens
             (connection_id, access_hash, refresh_hash, access_expires_at, refresh_expires_at)
         VALUES ($1, $2, $3, $4, $5)",
        new.connection_id,
        new.access_hash,
        new.refresh_hash,
        new.access_expires_at,
        new.refresh_expires_at,
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Locks the token's row until the transaction ends, so two refreshes at once take turns
pub async fn find_refresh_locked(
    transaction: &mut PgConnection,
    refresh_hash: &[u8],
) -> sqlx::Result<Option<Refresh>> {
    sqlx::query_as!(
        Refresh,
        r#"SELECT t.id, t.connection_id, c.client_id, t.replaced_at,
               t.refresh_expires_at <= now() AS "expired!"
           FROM agent_tokens t JOIN agent_connections c ON c.id = t.connection_id
           WHERE t.refresh_hash = $1 FOR UPDATE OF t"#,
        refresh_hash
    )
    .fetch_optional(transaction)
    .await
}

/// Marks the refresh token as exchanged. It is kept `days` more, so showing it again is caught
/// as a leak; its access token runs out on its own.
pub async fn replace(db: impl PgExecutor<'_>, id: Uuid, days: i32) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE agent_tokens
         SET replaced_at = now(),
             refresh_expires_at = least(refresh_expires_at, now() + make_interval(days => $2))
         WHERE id = $1",
        id,
        days
    )
    .execute(db)
    .await?;
    Ok(())
}

/// The connection an access or refresh token belongs to, whatever state the token is in
pub async fn connection_of(db: impl PgExecutor<'_>, hash: &[u8]) -> sqlx::Result<Option<Uuid>> {
    sqlx::query_scalar!(
        "SELECT connection_id FROM agent_tokens WHERE access_hash = $1 OR refresh_hash = $1",
        hash
    )
    .fetch_optional(db)
    .await
}

/// Signs the connection out: every token it holds stops working. How many rows went.
pub async fn delete_all(db: impl PgExecutor<'_>, connection_id: Uuid) -> sqlx::Result<u64> {
    let deleted = sqlx::query!(
        "DELETE FROM agent_tokens WHERE connection_id = $1",
        connection_id
    )
    .execute(db)
    .await?;
    Ok(deleted.rows_affected())
}

pub async fn delete_expired(db: impl PgExecutor<'_>) -> sqlx::Result<u64> {
    let deleted = sqlx::query!("DELETE FROM agent_tokens WHERE refresh_expires_at < now()")
        .execute(db)
        .await?;
    Ok(deleted.rows_affected())
}
