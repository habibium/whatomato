use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::{agents::Outcome, scheduler::RecipientType};

pub struct Activity {
    pub id: Uuid,
    /// Empty once the connection is revoked; `connection_name` still says which it was
    pub connection_id: Option<Uuid>,
    pub connection_name: String,
    pub tool: String,
    pub recipient_type: Option<RecipientType>,
    pub recipient: Option<String>,
    pub recipient_name: Option<String>,
    pub outcome: Outcome,
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub struct ActivityChat<'a> {
    pub recipient_type: RecipientType,
    pub recipient: &'a str,
    pub recipient_name: Option<&'a str>,
}

/// Never the message's text
pub struct NewActivity<'a> {
    pub user_id: Uuid,
    pub connection_id: Uuid,
    pub connection_name: &'a str,
    pub tool: &'a str,
    pub chat: Option<ActivityChat<'a>>,
    pub outcome: Outcome,
    pub reason: Option<&'a str>,
}

pub async fn insert(db: impl PgExecutor<'_>, new: NewActivity<'_>) -> sqlx::Result<()> {
    let chat = new.chat.as_ref();
    sqlx::query!(
        "INSERT INTO agent_activity (user_id, connection_id, connection_name, tool, \
         recipient_type, recipient, recipient_name, outcome, reason) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        new.user_id,
        new.connection_id,
        new.connection_name,
        new.tool,
        chat.map(|chat| chat.recipient_type.as_str()),
        chat.map(|chat| chat.recipient),
        chat.and_then(|chat| chat.recipient_name),
        new.outcome.as_str(),
        new.reason,
    )
    .execute(db)
    .await?;
    Ok(())
}

/// A page of the user's activity, newest first, for one connection if it's given
pub async fn page(
    db: impl PgExecutor<'_>,
    user_id: Uuid,
    connection_id: Option<Uuid>,
    limit: i64,
    offset: i64,
) -> sqlx::Result<Vec<Activity>> {
    sqlx::query_as!(
        Activity,
        r#"SELECT id, connection_id, connection_name, tool,
               recipient_type AS "recipient_type: RecipientType", recipient, recipient_name,
               outcome AS "outcome: Outcome", reason, created_at
           FROM agent_activity
           WHERE user_id = $1 AND ($2::uuid IS NULL OR connection_id = $2)
           ORDER BY created_at DESC, id LIMIT $3 OFFSET $4"#,
        user_id,
        connection_id,
        limit,
        offset,
    )
    .fetch_all(db)
    .await
}

pub async fn count(
    db: impl PgExecutor<'_>,
    user_id: Uuid,
    connection_id: Option<Uuid>,
) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM agent_activity
           WHERE user_id = $1 AND ($2::uuid IS NULL OR connection_id = $2)"#,
        user_id,
        connection_id,
    )
    .fetch_one(db)
    .await
}

/// Activity is kept 90 days
pub async fn delete_old(db: impl PgExecutor<'_>) -> sqlx::Result<u64> {
    let deleted =
        sqlx::query!("DELETE FROM agent_activity WHERE created_at < now() - interval '90 days'")
            .execute(db)
            .await?;
    Ok(deleted.rows_affected())
}
