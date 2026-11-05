use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

pub struct Template {
    pub id: Uuid,
    pub name: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn list(db: impl PgExecutor<'_>, user_id: Uuid) -> sqlx::Result<Vec<Template>> {
    sqlx::query_as!(
        Template,
        "SELECT id, name, body, created_at, updated_at FROM message_templates \
         WHERE user_id = $1 ORDER BY lower(name), id",
        user_id
    )
    .fetch_all(db)
    .await
}

pub async fn find(
    db: impl PgExecutor<'_>,
    id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Option<Template>> {
    sqlx::query_as!(
        Template,
        "SELECT id, name, body, created_at, updated_at FROM message_templates \
         WHERE id = $1 AND user_id = $2",
        id,
        user_id
    )
    .fetch_optional(db)
    .await
}

pub async fn create(
    db: impl PgExecutor<'_>,
    user_id: Uuid,
    name: &str,
    body: &str,
) -> sqlx::Result<Template> {
    sqlx::query_as!(
        Template,
        "INSERT INTO message_templates (user_id, name, body) VALUES ($1, $2, $3) \
         RETURNING id, name, body, created_at, updated_at",
        user_id,
        name,
        body
    )
    .fetch_one(db)
    .await
}

pub async fn update(
    db: impl PgExecutor<'_>,
    id: Uuid,
    user_id: Uuid,
    name: &str,
    body: &str,
) -> sqlx::Result<Option<Template>> {
    sqlx::query_as!(
        Template,
        "UPDATE message_templates SET name = $3, body = $4, updated_at = now() \
         WHERE id = $1 AND user_id = $2 \
         RETURNING id, name, body, created_at, updated_at",
        id,
        user_id,
        name,
        body
    )
    .fetch_optional(db)
    .await
}

/// Whether there was such a template
pub async fn delete(db: impl PgExecutor<'_>, id: Uuid, user_id: Uuid) -> sqlx::Result<bool> {
    let deleted = sqlx::query!(
        "DELETE FROM message_templates WHERE id = $1 AND user_id = $2",
        id,
        user_id
    )
    .execute(db)
    .await?;
    Ok(deleted.rows_affected() > 0)
}
