use chrono::{DateTime, Utc};
use sqlx::{PgExecutor, types::Json};
use uuid::Uuid;

use crate::scheduler::{RecipientType, ScheduleKind, recurrence::Recurrence};

pub struct Message {
    pub id: Uuid,
    pub user_id: Uuid,
    pub recipient_type: RecipientType,
    pub recipient: String,
    pub recipient_name: Option<String>,
    pub body: String,
    pub schedule_kind: ScheduleKind,
    pub run_at: Option<DateTime<Utc>>,
    pub recurrence: Option<Json<Recurrence>>,
    pub timezone: String,
    pub template_id: Option<Uuid>,
    pub enabled: bool,
    pub next_run_at: Option<DateTime<Utc>>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// What a user sets on a message
pub struct Fields<'a> {
    pub recipient_type: RecipientType,
    pub recipient: &'a str,
    pub recipient_name: Option<&'a str>,
    pub body: &'a str,
    pub schedule_kind: ScheduleKind,
    pub run_at: Option<DateTime<Utc>>,
    pub recurrence: Option<Recurrence>,
    pub timezone: &'a str,
    pub template_id: Option<Uuid>,
    pub enabled: bool,
    pub next_run_at: Option<DateTime<Utc>>,
}

pub async fn list(db: impl PgExecutor<'_>, user_id: Uuid) -> sqlx::Result<Vec<Message>> {
    sqlx::query_as!(
        Message,
        r#"SELECT id, user_id, recipient_type AS "recipient_type: RecipientType", recipient,
               recipient_name, body, schedule_kind AS "schedule_kind: ScheduleKind", run_at,
               recurrence AS "recurrence: Json<Recurrence>",
               timezone, template_id, enabled, next_run_at, last_run_at, created_at, updated_at
           FROM scheduled_messages WHERE user_id = $1 ORDER BY created_at, id"#,
        user_id
    )
    .fetch_all(db)
    .await
}

pub async fn find(
    db: impl PgExecutor<'_>,
    id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Option<Message>> {
    sqlx::query_as!(
        Message,
        r#"SELECT id, user_id, recipient_type AS "recipient_type: RecipientType", recipient,
               recipient_name, body, schedule_kind AS "schedule_kind: ScheduleKind", run_at,
               recurrence AS "recurrence: Json<Recurrence>",
               timezone, template_id, enabled, next_run_at, last_run_at, created_at, updated_at
           FROM scheduled_messages WHERE id = $1 AND user_id = $2"#,
        id,
        user_id
    )
    .fetch_optional(db)
    .await
}

pub async fn create(
    db: impl PgExecutor<'_>,
    user_id: Uuid,
    fields: Fields<'_>,
) -> sqlx::Result<Message> {
    sqlx::query_as!(
        Message,
        r#"INSERT INTO scheduled_messages (user_id, recipient_type, recipient, recipient_name, body,
               schedule_kind, run_at, recurrence, timezone, template_id, enabled, next_run_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
           RETURNING id, user_id, recipient_type AS "recipient_type: RecipientType", recipient,
               recipient_name, body, schedule_kind AS "schedule_kind: ScheduleKind", run_at,
               recurrence AS "recurrence: Json<Recurrence>",
               timezone, template_id, enabled, next_run_at, last_run_at, created_at, updated_at"#,
        user_id,
        fields.recipient_type.as_str(),
        fields.recipient,
        fields.recipient_name,
        fields.body,
        fields.schedule_kind.as_str(),
        fields.run_at,
        fields.recurrence.map(Json) as _,
        fields.timezone,
        fields.template_id,
        fields.enabled,
        fields.next_run_at,
    )
    .fetch_one(db)
    .await
}

pub async fn update(
    db: impl PgExecutor<'_>,
    id: Uuid,
    user_id: Uuid,
    fields: Fields<'_>,
) -> sqlx::Result<Option<Message>> {
    sqlx::query_as!(
        Message,
        r#"UPDATE scheduled_messages
           SET recipient_type = $3, recipient = $4, recipient_name = $5, body = $6,
               schedule_kind = $7, run_at = $8, recurrence = $9, timezone = $10, template_id = $11,
               enabled = $12, next_run_at = $13, updated_at = now()
           WHERE id = $1 AND user_id = $2
           RETURNING id, user_id, recipient_type AS "recipient_type: RecipientType", recipient,
               recipient_name, body, schedule_kind AS "schedule_kind: ScheduleKind", run_at,
               recurrence AS "recurrence: Json<Recurrence>",
               timezone, template_id, enabled, next_run_at, last_run_at, created_at, updated_at"#,
        id,
        user_id,
        fields.recipient_type.as_str(),
        fields.recipient,
        fields.recipient_name,
        fields.body,
        fields.schedule_kind.as_str(),
        fields.run_at,
        fields.recurrence.map(Json) as _,
        fields.timezone,
        fields.template_id,
        fields.enabled,
        fields.next_run_at,
    )
    .fetch_optional(db)
    .await
}

pub async fn set_enabled(
    db: impl PgExecutor<'_>,
    id: Uuid,
    user_id: Uuid,
    enabled: bool,
    next_run_at: Option<DateTime<Utc>>,
) -> sqlx::Result<Option<Message>> {
    sqlx::query_as!(
        Message,
        r#"UPDATE scheduled_messages SET enabled = $3, next_run_at = $4, updated_at = now()
           WHERE id = $1 AND user_id = $2
           RETURNING id, user_id, recipient_type AS "recipient_type: RecipientType", recipient,
               recipient_name, body, schedule_kind AS "schedule_kind: ScheduleKind", run_at,
               recurrence AS "recurrence: Json<Recurrence>",
               timezone, template_id, enabled, next_run_at, last_run_at, created_at, updated_at"#,
        id,
        user_id,
        enabled,
        next_run_at,
    )
    .fetch_optional(db)
    .await
}

/// Whether there was such a message
pub async fn delete(db: impl PgExecutor<'_>, id: Uuid, user_id: Uuid) -> sqlx::Result<bool> {
    let deleted = sqlx::query!(
        "DELETE FROM scheduled_messages WHERE id = $1 AND user_id = $2",
        id,
        user_id
    )
    .execute(db)
    .await?;
    Ok(deleted.rows_affected() > 0)
}

/// Messages due by `now`, oldest first, locked until the transaction ends so that one that's being
/// advanced can't go out twice
pub async fn lock_due(
    db: impl PgExecutor<'_>,
    now: DateTime<Utc>,
    limit: i64,
) -> sqlx::Result<Vec<Message>> {
    sqlx::query_as!(
        Message,
        r#"SELECT id, user_id, recipient_type AS "recipient_type: RecipientType", recipient,
               recipient_name, body, schedule_kind AS "schedule_kind: ScheduleKind", run_at,
               recurrence AS "recurrence: Json<Recurrence>",
               timezone, template_id, enabled, next_run_at, last_run_at, created_at, updated_at
           FROM scheduled_messages WHERE enabled AND next_run_at <= $1
           ORDER BY next_run_at LIMIT $2 FOR UPDATE SKIP LOCKED"#,
        now,
        limit,
    )
    .fetch_all(db)
    .await
}

/// Records a run. Without a `next_run_at` the message has no more to do, so it's switched off.
pub async fn advance(
    db: impl PgExecutor<'_>,
    id: Uuid,
    next_run_at: Option<DateTime<Utc>>,
    ran_at: DateTime<Utc>,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE scheduled_messages \
         SET next_run_at = $2, last_run_at = $3, enabled = $2::timestamptz IS NOT NULL, \
             updated_at = now() \
         WHERE id = $1",
        id,
        next_run_at,
        ran_at,
    )
    .execute(db)
    .await?;
    Ok(())
}
