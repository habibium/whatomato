use sqlx::PgExecutor;
use uuid::Uuid;

use crate::whatsapp::media::FileKind;

pub struct Stored {
    pub kind: FileKind,
    pub mime_type: String,
    pub file_name: Option<String>,
    /// Sealed; empty once the send it was for is over
    pub data: Option<Vec<u8>>,
}

pub struct NewFile<'a> {
    pub id: Uuid,
    pub user_id: Uuid,
    pub kind: FileKind,
    pub mime_type: &'a str,
    pub file_name: Option<&'a str>,
    pub size: i32,
    pub sealed: &'a [u8],
}

pub async fn save(db: impl PgExecutor<'_>, file: NewFile<'_>) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO outgoing_files (id, user_id, kind, mime_type, file_name, size, data)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        file.id,
        file.user_id,
        file.kind.as_str(),
        file.mime_type,
        file.file_name,
        file.size,
        file.sealed,
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn find(
    db: impl PgExecutor<'_>,
    id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Option<Stored>> {
    sqlx::query_as!(
        Stored,
        r#"SELECT kind AS "kind: FileKind", mime_type, file_name, data
           FROM outgoing_files WHERE id = $1 AND user_id = $2"#,
        id,
        user_id,
    )
    .fetch_optional(db)
    .await
}

/// Drops a file's data once the send it was for is over; what it was stays for the history
pub async fn release(db: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<()> {
    sqlx::query!("UPDATE outgoing_files SET data = NULL WHERE id = $1", id)
        .execute(db)
        .await?;
    Ok(())
}

/// Drops the data of files nothing waits for: a request for approval lasts 24 hours, so a file
/// older than a day with no send still queued is never going out
pub async fn release_stale(db: impl PgExecutor<'_>) -> sqlx::Result<u64> {
    let released = sqlx::query!(
        "UPDATE outgoing_files f SET data = NULL
         WHERE data IS NOT NULL AND created_at < now() - interval '25 hours'
           AND NOT EXISTS (
               SELECT 1 FROM deliveries d
               WHERE d.file_id = f.id AND d.status IN ('pending', 'sending')
           )"
    )
    .execute(db)
    .await?;
    Ok(released.rows_affected())
}
