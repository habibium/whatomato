use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

pub struct Account {
    pub id: Uuid,
    pub user_id: Uuid,
    /// Set once the phone has linked the device
    pub phone_number: Option<String>,
    pub paused: bool,
    /// `replaced` or `temporary_ban`, a stop only the user can lift
    pub stopped_reason: Option<String>,
    /// When a temporary ban ends
    pub stopped_until: Option<DateTime<Utc>>,
    /// When a paused account next connects for a moment
    pub keepalive_at: Option<DateTime<Utc>>,
}

pub async fn all(db: impl PgExecutor<'_>) -> sqlx::Result<Vec<Account>> {
    sqlx::query_as!(
        Account,
        r#"SELECT id, user_id, phone_number, paused_at IS NOT NULL AS "paused!", stopped_reason,
               stopped_until, keepalive_at
           FROM whatsapp_accounts"#
    )
    .fetch_all(db)
    .await
}

/// How many accounts a phone has linked, and whether the user's is one of them. A row whose
/// pairing is under way or was abandoned has no phone number yet, so it isn't counted.
pub async fn linked(db: impl PgExecutor<'_>, user_id: Uuid) -> sqlx::Result<(i64, bool)> {
    let row = sqlx::query!(
        r#"SELECT count(*) AS "linked!", coalesce(bool_or(user_id = $1), false) AS "own!"
           FROM whatsapp_accounts WHERE phone_number IS NOT NULL"#,
        user_id
    )
    .fetch_one(db)
    .await?;
    Ok((row.linked, row.own))
}

/// The user's account row, created if there's none
pub async fn find_or_create(db: impl PgExecutor<'_>, user_id: Uuid) -> sqlx::Result<Account> {
    // A no-op update, because DO NOTHING returns no row on a conflict
    sqlx::query_as!(
        Account,
        r#"INSERT INTO whatsapp_accounts (user_id) VALUES ($1)
           ON CONFLICT (user_id) DO UPDATE SET user_id = EXCLUDED.user_id
           RETURNING id, user_id, phone_number, paused_at IS NOT NULL AS "paused!", stopped_reason,
               stopped_until, keepalive_at"#,
        user_id
    )
    .fetch_one(db)
    .await
}

pub async fn save_link(
    db: impl PgExecutor<'_>,
    account_id: Uuid,
    phone_number: &str,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE whatsapp_accounts \
         SET phone_number = $2, linked_at = now(), paused_at = NULL, keepalive_at = NULL, \
             stopped_reason = NULL, stopped_until = NULL, updated_at = now() \
         WHERE id = $1",
        account_id,
        phone_number
    )
    .execute(db)
    .await?;

    Ok(())
}

pub async fn pause(
    db: impl PgExecutor<'_>,
    account_id: Uuid,
    keepalive_at: DateTime<Utc>,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE whatsapp_accounts \
         SET paused_at = now(), keepalive_at = $2, updated_at = now() \
         WHERE id = $1",
        account_id,
        keepalive_at
    )
    .execute(db)
    .await?;

    Ok(())
}

pub async fn save_keepalive(
    db: impl PgExecutor<'_>,
    account_id: Uuid,
    keepalive_at: DateTime<Utc>,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE whatsapp_accounts SET keepalive_at = $2, updated_at = now() WHERE id = $1",
        account_id,
        keepalive_at
    )
    .execute(db)
    .await?;

    Ok(())
}

/// Clears a pause and a kept stop alike
pub async fn resume(db: impl PgExecutor<'_>, account_id: Uuid) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE whatsapp_accounts \
         SET paused_at = NULL, keepalive_at = NULL, stopped_reason = NULL, stopped_until = NULL, \
             updated_at = now() \
         WHERE id = $1",
        account_id
    )
    .execute(db)
    .await?;

    Ok(())
}

pub async fn save_stop(
    db: impl PgExecutor<'_>,
    account_id: Uuid,
    reason: &str,
    until: Option<DateTime<Utc>>,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE whatsapp_accounts SET stopped_reason = $2, stopped_until = $3, updated_at = now() \
         WHERE id = $1",
        account_id,
        reason,
        until
    )
    .execute(db)
    .await?;

    Ok(())
}

/// Deletes the account and, through the cascade, every key it held
pub async fn delete(db: impl PgExecutor<'_>, account_id: Uuid) -> sqlx::Result<()> {
    sqlx::query!("DELETE FROM whatsapp_accounts WHERE id = $1", account_id)
        .execute(db)
        .await?;

    Ok(())
}

/// How far the import of the phone's history has got
pub struct HistoryImport {
    pub account_id: Uuid,
    /// Whether a phone has linked the device
    pub linked: bool,
    /// The furthest progress, from 0 to 100, a chunk claimed
    pub progress: Option<i32>,
    pub imported: i64,
}

pub async fn history_import(
    db: impl PgExecutor<'_>,
    user_id: Uuid,
) -> sqlx::Result<Option<HistoryImport>> {
    sqlx::query_as!(
        HistoryImport,
        r#"SELECT id AS account_id, phone_number IS NOT NULL AS "linked!",
                  history_progress AS progress, history_imported AS imported
           FROM whatsapp_accounts WHERE user_id = $1"#,
        user_id
    )
    .fetch_optional(db)
    .await
}

/// Counts what a history chunk brought in. Chunks don't arrive in order, so the progress only
/// grows.
pub async fn add_history_import(
    db: impl PgExecutor<'_>,
    account_id: Uuid,
    progress: Option<i32>,
    imported: i64,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE whatsapp_accounts
         SET history_progress = GREATEST(history_progress, $2),
             history_imported = history_imported + $3
         WHERE id = $1",
        account_id,
        progress,
        imported,
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn phone_number(
    db: impl PgExecutor<'_>,
    account_id: Uuid,
) -> sqlx::Result<Option<String>> {
    Ok(sqlx::query_scalar!(
        "SELECT phone_number FROM whatsapp_accounts WHERE id = $1",
        account_id
    )
    .fetch_optional(db)
    .await?
    .flatten())
}

pub async fn id_of_user(db: impl PgExecutor<'_>, user_id: Uuid) -> sqlx::Result<Option<Uuid>> {
    sqlx::query_scalar!(
        "SELECT id FROM whatsapp_accounts WHERE user_id = $1",
        user_id
    )
    .fetch_optional(db)
    .await
}
