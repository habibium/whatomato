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
}

pub async fn all(db: impl PgExecutor<'_>) -> sqlx::Result<Vec<Account>> {
    sqlx::query_as!(
        Account,
        r#"SELECT id, user_id, phone_number, paused_at IS NOT NULL AS "paused!", stopped_reason, stopped_until
           FROM whatsapp_accounts"#
    )
    .fetch_all(db)
    .await
}

/// The user's account row, created if there's none
pub async fn find_or_create(db: impl PgExecutor<'_>, user_id: Uuid) -> sqlx::Result<Account> {
    // A no-op update, because DO NOTHING returns no row on a conflict
    sqlx::query_as!(
        Account,
        r#"INSERT INTO whatsapp_accounts (user_id) VALUES ($1)
           ON CONFLICT (user_id) DO UPDATE SET user_id = EXCLUDED.user_id
           RETURNING id, user_id, phone_number, paused_at IS NOT NULL AS "paused!", stopped_reason,
               stopped_until"#,
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
         SET phone_number = $2, linked_at = now(), paused_at = NULL, stopped_reason = NULL, \
             stopped_until = NULL, updated_at = now() \
         WHERE id = $1",
        account_id,
        phone_number
    )
    .execute(db)
    .await?;

    Ok(())
}

pub async fn pause(db: impl PgExecutor<'_>, account_id: Uuid) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE whatsapp_accounts SET paused_at = now(), updated_at = now() WHERE id = $1",
        account_id
    )
    .execute(db)
    .await?;

    Ok(())
}

/// Clears a pause and a kept stop alike
pub async fn resume(db: impl PgExecutor<'_>, account_id: Uuid) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE whatsapp_accounts \
         SET paused_at = NULL, stopped_reason = NULL, stopped_until = NULL, updated_at = now() \
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
