//! Requests that wait for the owner. A row past `expires_at` reads as expired before the hourly
//! sweep marks it, and can no longer be decided.

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::{
    agents::{ApprovalKind, ApprovalStatus},
    scheduler::RecipientType,
};

pub struct Approval {
    pub id: Uuid,
    pub connection_id: Uuid,
    pub user_id: Uuid,
    pub kind: ApprovalKind,
    pub recipient_type: RecipientType,
    pub recipient: String,
    pub recipient_name: Option<String>,
    /// The request, sealed and bound to `id`; opaque here
    pub payload: Vec<u8>,
    pub status: ApprovalStatus,
    pub delivery_id: Option<Uuid>,
    pub scheduled_message_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
}

pub struct NewApproval<'a> {
    /// Chosen by the caller, since the payload is sealed to it before the row exists
    pub id: Uuid,
    pub connection_id: Uuid,
    pub kind: ApprovalKind,
    pub recipient_type: RecipientType,
    pub recipient: &'a str,
    pub recipient_name: Option<&'a str>,
    pub payload: &'a [u8],
}

/// What an approval became
#[derive(Clone, Copy, Debug, Default)]
pub struct Became {
    pub delivery_id: Option<Uuid>,
    pub scheduled_message_id: Option<Uuid>,
}

/// The owner comes from the connection. `None` when the connection is gone.
pub async fn create(
    db: impl PgExecutor<'_>,
    new: NewApproval<'_>,
) -> sqlx::Result<Option<Approval>> {
    sqlx::query_as!(
        Approval,
        r#"INSERT INTO agent_approvals
               (id, connection_id, user_id, kind, recipient_type, recipient, recipient_name, payload)
           SELECT $1, id, user_id, $3, $4, $5, $6, $7 FROM agent_connections WHERE id = $2
           RETURNING id, connection_id, user_id, kind AS "kind: ApprovalKind",
               recipient_type AS "recipient_type: RecipientType", recipient, recipient_name,
               payload, status AS "status: ApprovalStatus", delivery_id, scheduled_message_id,
               created_at, expires_at, decided_at"#,
        new.id,
        new.connection_id,
        new.kind.as_str(),
        new.recipient_type.as_str(),
        new.recipient,
        new.recipient_name,
        new.payload,
    )
    .fetch_optional(db)
    .await
}

pub async fn find(
    db: impl PgExecutor<'_>,
    id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Option<Approval>> {
    sqlx::query_as!(
        Approval,
        r#"SELECT id, connection_id, user_id, kind AS "kind: ApprovalKind",
               recipient_type AS "recipient_type: RecipientType", recipient, recipient_name,
               payload,
               CASE WHEN status = 'waiting' AND expires_at <= now() THEN 'expired' ELSE status END
                   AS "status!: ApprovalStatus",
               delivery_id, scheduled_message_id, created_at, expires_at, decided_at
           FROM agent_approvals WHERE id = $1 AND user_id = $2"#,
        id,
        user_id
    )
    .fetch_optional(db)
    .await
}

/// For the agent that asked: another connection's approval isn't found
pub async fn find_for_connection(
    db: impl PgExecutor<'_>,
    id: Uuid,
    connection_id: Uuid,
) -> sqlx::Result<Option<Approval>> {
    sqlx::query_as!(
        Approval,
        r#"SELECT id, connection_id, user_id, kind AS "kind: ApprovalKind",
               recipient_type AS "recipient_type: RecipientType", recipient, recipient_name,
               payload,
               CASE WHEN status = 'waiting' AND expires_at <= now() THEN 'expired' ELSE status END
                   AS "status!: ApprovalStatus",
               delivery_id, scheduled_message_id, created_at, expires_at, decided_at
           FROM agent_approvals WHERE id = $1 AND connection_id = $2"#,
        id,
        connection_id
    )
    .fetch_optional(db)
    .await
}

/// A page of the user's approvals, those still waiting first, then newest first
pub async fn list(
    db: impl PgExecutor<'_>,
    user_id: Uuid,
    limit: i64,
    offset: i64,
) -> sqlx::Result<Vec<Approval>> {
    sqlx::query_as!(
        Approval,
        r#"SELECT id, connection_id, user_id, kind AS "kind: ApprovalKind",
               recipient_type AS "recipient_type: RecipientType", recipient, recipient_name,
               payload,
               CASE WHEN status = 'waiting' AND expires_at <= now() THEN 'expired' ELSE status END
                   AS "status!: ApprovalStatus",
               delivery_id, scheduled_message_id, created_at, expires_at, decided_at
           FROM agent_approvals WHERE user_id = $1
           ORDER BY (status = 'waiting' AND expires_at > now()) DESC, created_at DESC, id
           LIMIT $2 OFFSET $3"#,
        user_id,
        limit,
        offset,
    )
    .fetch_all(db)
    .await
}

/// A page of one connection's approvals, newest first
pub async fn list_for_connection(
    db: impl PgExecutor<'_>,
    connection_id: Uuid,
    limit: i64,
    offset: i64,
) -> sqlx::Result<Vec<Approval>> {
    sqlx::query_as!(
        Approval,
        r#"SELECT id, connection_id, user_id, kind AS "kind: ApprovalKind",
               recipient_type AS "recipient_type: RecipientType", recipient, recipient_name,
               payload,
               CASE WHEN status = 'waiting' AND expires_at <= now() THEN 'expired' ELSE status END
                   AS "status!: ApprovalStatus",
               delivery_id, scheduled_message_id, created_at, expires_at, decided_at
           FROM agent_approvals WHERE connection_id = $1
           ORDER BY created_at DESC, id LIMIT $2 OFFSET $3"#,
        connection_id,
        limit,
        offset,
    )
    .fetch_all(db)
    .await
}

pub async fn count_waiting(db: impl PgExecutor<'_>, user_id: Uuid) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM agent_approvals
           WHERE user_id = $1 AND status = 'waiting' AND expires_at > now()"#,
        user_id
    )
    .fetch_one(db)
    .await
}

/// Approves a waiting approval, `None` when it isn't waiting any more. Run it in the transaction
/// that creates the delivery or schedule and roll back on `None`: the row lock makes a second
/// approval wait for the first and then find nothing waiting.
pub async fn approve(
    db: impl PgExecutor<'_>,
    id: Uuid,
    user_id: Uuid,
    became: Became,
) -> sqlx::Result<Option<Approval>> {
    sqlx::query_as!(
        Approval,
        r#"UPDATE agent_approvals
           SET status = 'approved', decided_at = now(), delivery_id = $3,
               scheduled_message_id = $4
           WHERE id = $1 AND user_id = $2 AND status = 'waiting' AND expires_at > now()
           RETURNING id, connection_id, user_id, kind AS "kind: ApprovalKind",
               recipient_type AS "recipient_type: RecipientType", recipient, recipient_name,
               payload, status AS "status: ApprovalStatus", delivery_id, scheduled_message_id,
               created_at, expires_at, decided_at"#,
        id,
        user_id,
        became.delivery_id,
        became.scheduled_message_id,
    )
    .fetch_optional(db)
    .await
}

/// `None` when it isn't waiting any more
pub async fn reject(
    db: impl PgExecutor<'_>,
    id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Option<Approval>> {
    sqlx::query_as!(
        Approval,
        r#"UPDATE agent_approvals SET status = 'rejected', decided_at = now()
           WHERE id = $1 AND user_id = $2 AND status = 'waiting' AND expires_at > now()
           RETURNING id, connection_id, user_id, kind AS "kind: ApprovalKind",
               recipient_type AS "recipient_type: RecipientType", recipient, recipient_name,
               payload, status AS "status: ApprovalStatus", delivery_id, scheduled_message_id,
               created_at, expires_at, decided_at"#,
        id,
        user_id,
    )
    .fetch_optional(db)
    .await
}

/// Whether the user has an approval other than `id` created in the last hour, so a new one
/// needn't email them again
pub async fn others_in_last_hour(
    db: impl PgExecutor<'_>,
    user_id: Uuid,
    id: Uuid,
) -> sqlx::Result<bool> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM agent_approvals
               WHERE user_id = $1 AND id <> $2 AND created_at > now() - interval '1 hour'
           ) AS "exists!""#,
        user_id,
        id,
    )
    .fetch_one(db)
    .await
}

pub async fn expire_overdue(db: impl PgExecutor<'_>) -> sqlx::Result<u64> {
    let expired = sqlx::query!(
        "UPDATE agent_approvals SET status = 'expired', decided_at = expires_at \
         WHERE status = 'waiting' AND expires_at <= now()"
    )
    .execute(db)
    .await?;
    Ok(expired.rows_affected())
}
