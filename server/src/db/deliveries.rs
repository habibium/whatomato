use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

use super::Chats;
use crate::scheduler::{DeliveryStatus, RecipientType};

pub struct Delivery {
    pub id: Uuid,
    pub scheduled_message_id: Option<Uuid>,
    /// The agent connection that caused it, if one did
    pub connection_id: Option<Uuid>,
    /// That connection's name; empty when the owner sent it or the connection is gone
    pub connection_name: Option<String>,
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
    pub connection_id: Option<Uuid>,
    pub recipient_type: RecipientType,
    pub recipient: &'a str,
    pub recipient_name: Option<&'a str>,
    pub body: &'a str,
    /// The id of the message this one quotes, in the same chat
    pub reply_to: Option<&'a str>,
    /// The numbers it tags, each written as `@number` in `body`
    pub mentions: &'a [String],
}

/// A delivery taken off the queue to send
pub struct Claimed {
    pub id: Uuid,
    pub user_id: Uuid,
    /// The agent connection that caused it, if one did
    pub connection_id: Option<Uuid>,
    pub recipient_type: RecipientType,
    pub recipient: String,
    pub body: String,
    pub reply_to: Option<String>,
    pub mentions: Vec<String>,
    /// Counting this one
    pub attempts: i32,
}

pub async fn create(db: impl PgExecutor<'_>, delivery: NewDelivery<'_>) -> sqlx::Result<Uuid> {
    sqlx::query_scalar!(
        "INSERT INTO deliveries \
         (user_id, scheduled_message_id, connection_id, recipient_type, recipient, \
          recipient_name, body, reply_to, mentions) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id",
        delivery.user_id,
        delivery.scheduled_message_id,
        delivery.connection_id,
        delivery.recipient_type.as_str(),
        delivery.recipient,
        delivery.recipient_name,
        delivery.body,
        delivery.reply_to,
        delivery.mentions,
    )
    .fetch_one(db)
    .await
}

/// Records a run that was skipped before any attempt, failed with `error`, unless the schedule's
/// last run was skipped too, by the same connection with the same `error`: a streak of skips shows
/// once. `None` when it wasn't recorded.
pub async fn create_skipped(
    db: impl PgExecutor<'_>,
    delivery: NewDelivery<'_>,
    error: &str,
) -> sqlx::Result<Option<Uuid>> {
    sqlx::query_scalar!(
        r#"INSERT INTO deliveries
               (user_id, scheduled_message_id, connection_id, recipient_type, recipient,
                recipient_name, body, status, error, skipped)
           SELECT $1, $2, $3, $4, $5, $6, $7, 'failed', $8, true
           WHERE NOT EXISTS (
               SELECT 1 FROM (
                   SELECT skipped, connection_id, error FROM deliveries
                   WHERE scheduled_message_id = $2
                   ORDER BY created_at DESC, id DESC LIMIT 1
               ) last
               WHERE last.skipped AND last.connection_id IS NOT DISTINCT FROM $3
                   AND last.error IS NOT DISTINCT FROM $8
           )
           RETURNING id"#,
        delivery.user_id,
        delivery.scheduled_message_id,
        delivery.connection_id,
        delivery.recipient_type.as_str(),
        delivery.recipient,
        delivery.recipient_name,
        delivery.body,
        error,
    )
    .fetch_optional(db)
    .await
}

/// Narrows a listing of the user's deliveries
#[derive(Clone, Copy, Default)]
pub struct Filter<'a> {
    pub recipient: Option<&'a str>,
    /// Only what this agent connection caused, and whatever went to `also_to`
    pub connection_id: Option<Uuid>,
    /// Read only with `connection_id`
    pub also_to: Chats<'a>,
}

/// A page of the user's deliveries that pass `filter`, newest first
pub async fn page(
    db: impl PgExecutor<'_>,
    user_id: Uuid,
    filter: Filter<'_>,
    limit: i64,
    offset: i64,
) -> sqlx::Result<Vec<Delivery>> {
    sqlx::query_as!(
        Delivery,
        r#"SELECT d.id, d.scheduled_message_id, d.connection_id, c.name AS "connection_name?",
               d.recipient_type AS "recipient_type: RecipientType", d.recipient,
               d.recipient_name, d.body, d.status AS "status: DeliveryStatus", d.error,
               d.created_at, d.sent_at
           FROM deliveries d LEFT JOIN agent_connections c ON c.id = d.connection_id
           WHERE d.user_id = $1 AND ($2::text IS NULL OR d.recipient = $2)
               AND ($3::uuid IS NULL OR d.connection_id = $3
                   OR (d.recipient_type, d.recipient)
                       IN (SELECT * FROM unnest($6::text[], $7::text[])))
           ORDER BY d.created_at DESC, d.id LIMIT $4 OFFSET $5"#,
        user_id,
        filter.recipient,
        filter.connection_id,
        limit,
        offset,
        filter.also_to.types,
        filter.also_to.recipients,
    )
    .fetch_all(db)
    .await
}

pub async fn count(
    db: impl PgExecutor<'_>,
    user_id: Uuid,
    filter: Filter<'_>,
) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM deliveries
           WHERE user_id = $1 AND ($2::text IS NULL OR recipient = $2)
               AND ($3::uuid IS NULL OR connection_id = $3
                   OR (recipient_type, recipient)
                       IN (SELECT * FROM unnest($4::text[], $5::text[])))"#,
        user_id,
        filter.recipient,
        filter.connection_id,
        filter.also_to.types,
        filter.also_to.recipients,
    )
    .fetch_one(db)
    .await
}

/// Where some of the user's deliveries stand, for those of `ids` that are theirs
pub async fn states(
    db: impl PgExecutor<'_>,
    user_id: Uuid,
    ids: &[Uuid],
) -> sqlx::Result<Vec<(Uuid, DeliveryStatus, Option<String>)>> {
    let rows = sqlx::query!(
        r#"SELECT id, status AS "status: DeliveryStatus", error FROM deliveries
           WHERE user_id = $1 AND id = ANY($2)"#,
        user_id,
        ids,
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.id, row.status, row.error))
        .collect())
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
           RETURNING id, user_id, connection_id,
               recipient_type AS "recipient_type: RecipientType", recipient, body, reply_to,
               mentions, attempts"#,
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
