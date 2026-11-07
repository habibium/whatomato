use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::scheduler::{DeliveryStatus, RecipientType};

pub struct Delivery {
    pub id: Uuid,
    pub scheduled_message_id: Option<Uuid>,
    pub recipient_type: RecipientType,
    pub recipient: String,
    pub recipient_name: Option<String>,
    pub body: String,
    pub status: DeliveryStatus,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub sent_at: Option<DateTime<Utc>>,
}

pub struct NewDelivery<'a> {
    pub user_id: Uuid,
    pub scheduled_message_id: Option<Uuid>,
    pub recipient_type: RecipientType,
    pub recipient: &'a str,
    pub recipient_name: Option<&'a str>,
    pub body: &'a str,
}

/// A delivery taken off the queue to send
pub struct Claimed {
    pub id: Uuid,
    pub user_id: Uuid,
    pub recipient_type: RecipientType,
    pub recipient: String,
    pub body: String,
    /// Counting this one
    pub attempts: i32,
}

pub async fn create(db: impl PgExecutor<'_>, delivery: NewDelivery<'_>) -> sqlx::Result<Uuid> {
    sqlx::query_scalar!(
        "INSERT INTO deliveries \
         (user_id, scheduled_message_id, recipient_type, recipient, recipient_name, body) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
        delivery.user_id,
        delivery.scheduled_message_id,
        delivery.recipient_type.as_str(),
        delivery.recipient,
        delivery.recipient_name,
        delivery.body,
    )
    .fetch_one(db)
    .await
}

/// A page of the user's deliveries, newest first, to `recipient` alone if it's given
pub async fn page(
    db: impl PgExecutor<'_>,
    user_id: Uuid,
    recipient: Option<&str>,
    limit: i64,
    offset: i64,
) -> sqlx::Result<Vec<Delivery>> {
    sqlx::query_as!(
        Delivery,
        r#"SELECT id, scheduled_message_id, recipient_type AS "recipient_type: RecipientType",
               recipient, recipient_name, body, status AS "status: DeliveryStatus", error,
               created_at, sent_at
           FROM deliveries WHERE user_id = $1 AND ($2::text IS NULL OR recipient = $2)
           ORDER BY created_at DESC, id LIMIT $3 OFFSET $4"#,
        user_id,
        recipient,
        limit,
        offset,
    )
    .fetch_all(db)
    .await
}

pub async fn count(
    db: impl PgExecutor<'_>,
    user_id: Uuid,
    recipient: Option<&str>,
) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM deliveries
           WHERE user_id = $1 AND ($2::text IS NULL OR recipient = $2)"#,
        user_id,
        recipient
    )
    .fetch_one(db)
    .await
}

/// Takes the oldest waiting delivery of every account outside `busy`. Only the holder of the
/// WhatsApp lock claims, so nothing else races for these rows.
pub async fn claim(db: impl PgExecutor<'_>, busy: &[Uuid]) -> sqlx::Result<Vec<Claimed>> {
    sqlx::query_as!(
        Claimed,
        r#"UPDATE deliveries SET status = 'sending', attempts = attempts + 1
           WHERE id IN (
               SELECT DISTINCT ON (user_id) id FROM deliveries
               WHERE status = 'pending' AND next_attempt_at <= now() AND user_id <> ALL($1)
               ORDER BY user_id, next_attempt_at, created_at
           )
           RETURNING id, user_id, recipient_type AS "recipient_type: RecipientType", recipient,
               body, attempts"#,
        busy,
    )
    .fetch_all(db)
    .await
}

pub async fn mark_sent(db: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE deliveries SET status = 'sent', sent_at = now(), error = NULL WHERE id = $1",
        id
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn mark_failed(db: impl PgExecutor<'_>, id: Uuid, error: &str) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE deliveries SET status = 'failed', error = $2 WHERE id = $1",
        id,
        error
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Puts a delivery back on the queue. `error` shows in the history while it waits.
pub async fn retry_at(
    db: impl PgExecutor<'_>,
    id: Uuid,
    at: DateTime<Utc>,
    error: &str,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE deliveries SET status = 'pending', next_attempt_at = $2, error = $3 WHERE id = $1",
        id,
        at,
        error
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Fails what the last server left mid-send, which may have gone out
pub async fn fail_interrupted(db: impl PgExecutor<'_>, error: &str) -> sqlx::Result<u64> {
    let failed = sqlx::query!(
        "UPDATE deliveries SET status = 'failed', error = $1 WHERE status = 'sending'",
        error
    )
    .execute(db)
    .await?;
    Ok(failed.rows_affected())
}
