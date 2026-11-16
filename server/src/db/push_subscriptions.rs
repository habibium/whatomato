use chrono::{DateTime, Utc};
use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

/// Browsers one user may have subscribed at once; past it the oldest goes
pub const MAX_PER_USER: i64 = 10;

pub struct Subscription {
    pub id: Uuid,
    pub endpoint: String,
    pub p256dh: Vec<u8>,
    pub auth: Vec<u8>,
    /// What the browser called itself, such as "Chrome on Linux"
    pub label: String,
    pub created_at: DateTime<Utc>,
}

pub struct NewSubscription<'a> {
    pub user_id: Uuid,
    pub endpoint: &'a str,
    pub p256dh: &'a [u8],
    pub auth: &'a [u8],
    pub label: &'a str,
}

/// Stores the browser's subscription. One it already made, under any account, becomes this
/// user's: a browser notifies for whoever turned notifications on in it last.
pub async fn save(pool: &PgPool, new: NewSubscription<'_>) -> sqlx::Result<Subscription> {
    let mut transaction = pool.begin().await?;
    let subscription = sqlx::query_as!(
        Subscription,
        "INSERT INTO push_subscriptions (user_id, endpoint, p256dh, auth, label)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (endpoint) DO UPDATE
         SET user_id = excluded.user_id, p256dh = excluded.p256dh, auth = excluded.auth,
             label = excluded.label
         RETURNING id, endpoint, p256dh, auth, label, created_at",
        new.user_id,
        new.endpoint,
        new.p256dh,
        new.auth,
        new.label,
    )
    .fetch_one(&mut *transaction)
    .await?;
    sqlx::query!(
        "DELETE FROM push_subscriptions WHERE id IN (
             SELECT id FROM push_subscriptions WHERE user_id = $1
             ORDER BY created_at DESC, id OFFSET $2)",
        new.user_id,
        MAX_PER_USER,
    )
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(subscription)
}

pub async fn list(db: impl PgExecutor<'_>, user_id: Uuid) -> sqlx::Result<Vec<Subscription>> {
    sqlx::query_as!(
        Subscription,
        "SELECT id, endpoint, p256dh, auth, label, created_at
         FROM push_subscriptions WHERE user_id = $1 ORDER BY created_at, id",
        user_id
    )
    .fetch_all(db)
    .await
}

/// Whether the user had this browser subscribed
pub async fn delete(db: impl PgExecutor<'_>, user_id: Uuid, endpoint: &str) -> sqlx::Result<bool> {
    let deleted = sqlx::query!(
        "DELETE FROM push_subscriptions WHERE user_id = $1 AND endpoint = $2",
        user_id,
        endpoint
    )
    .execute(db)
    .await?;
    Ok(deleted.rows_affected() > 0)
}

/// For a subscription its push service says is gone
pub async fn delete_by_id(db: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<()> {
    sqlx::query!("DELETE FROM push_subscriptions WHERE id = $1", id)
        .execute(db)
        .await?;
    Ok(())
}

/// Every subscription is tied to the instance's push key pair, and goes when the pair does
pub async fn delete_all(db: impl PgExecutor<'_>) -> sqlx::Result<u64> {
    let deleted = sqlx::query!("DELETE FROM push_subscriptions")
        .execute(db)
        .await?;
    Ok(deleted.rows_affected())
}
