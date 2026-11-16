use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor, PgPool, types::Json};
use uuid::Uuid;

use crate::{agents::Grant, scheduler::RecipientType};

/// Connections one user may have at once
pub const MAX_PER_USER: i64 = 20;

pub struct Connection {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    /// The key's last 4 characters; `None` when it was made by signing in
    pub key_hint: Option<String>,
    /// The URL of the metadata document of the client that signed in
    pub client_id: Option<String>,
    /// What that document called the client, so never to be trusted
    pub client_name: Option<String>,
    /// As stored; only [`Grant::parse`] turns it into access, so a bad row gives none
    pub grant: serde_json::Value,
    pub paused_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub struct NewConnection<'a> {
    pub user_id: Uuid,
    pub name: &'a str,
    pub credential: Credential<'a>,
    pub grant: &'a Grant,
    pub expires_at: Option<DateTime<Utc>>,
}

/// How the agent proves it is this connection
#[derive(Clone, Copy)]
pub enum Credential<'a> {
    Key {
        hash: &'a [u8],
        hint: &'a str,
    },
    /// Tokens, issued to the client that signed in
    SignIn {
        client_id: &'a str,
        client_name: &'a str,
    },
}

/// The connection a key belongs to, and whether it may be used now
pub struct KeyMatch {
    pub connection: Connection,
    pub paused: bool,
    pub expired: bool,
}

/// What [`decide`](crate::agents::decide) needs counted, over the last 24 hours. A send counts
/// once queued, and while it waits for approval; a skipped run never does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Counts {
    pub chat_sends: u32,
    pub connection_sends: u32,
    pub waiting_approvals: u32,
    /// Its scheduled messages, and its requests to create one still waiting
    pub schedules: u32,
}

/// A connection as read under its lock
pub struct Locked {
    pub grant: serde_json::Value,
    pub paused: bool,
    pub expired: bool,
}

/// Locks the connection's row until the transaction ends, so its counts can't change between a
/// decision and the insert it allows. `FOR NO KEY UPDATE` leaves inserts that reference the row
/// free. `None` when there's no such connection.
pub async fn lock(db: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Option<Locked>> {
    sqlx::query_as!(
        Locked,
        r#"SELECT "grant", paused_at IS NOT NULL AS "paused!",
               coalesce(expires_at <= now(), false) AS "expired!"
           FROM agent_connections WHERE id = $1 FOR NO KEY UPDATE"#,
        id
    )
    .fetch_optional(db)
    .await
}

/// [`lock`] without waiting: `None` also while another transaction holds it
pub async fn try_lock(db: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Option<Locked>> {
    sqlx::query_as!(
        Locked,
        r#"SELECT "grant", paused_at IS NOT NULL AS "paused!",
               coalesce(expires_at <= now(), false) AS "expired!"
           FROM agent_connections WHERE id = $1 FOR NO KEY UPDATE SKIP LOCKED"#,
        id
    )
    .fetch_optional(db)
    .await
}

pub async fn exists(db: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<bool> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM agent_connections WHERE id = $1) AS "exists!""#,
        id
    )
    .fetch_one(db)
    .await
}

/// `None` when the user already has [`MAX_PER_USER`] connections
pub async fn create(pool: &PgPool, new: NewConnection<'_>) -> sqlx::Result<Option<Connection>> {
    let mut transaction = pool.begin().await?;
    let connection = create_in(&mut transaction, new).await?;
    transaction.commit().await?;
    Ok(connection)
}

/// [`create`] inside the caller's transaction
pub async fn create_in(
    transaction: &mut PgConnection,
    new: NewConnection<'_>,
) -> sqlx::Result<Option<Connection>> {
    // Holding the user's row makes a second create wait, so it counts this one; a lock taken in the
    // same statement as the count wouldn't, since the count reads the statement's earlier snapshot
    let exists = sqlx::query_scalar!("SELECT id FROM users WHERE id = $1 FOR UPDATE", new.user_id)
        .fetch_optional(&mut *transaction)
        .await?;
    if exists.is_none() {
        return Ok(None);
    }
    let (key_hash, key_hint, client_id, client_name) = match new.credential {
        Credential::Key { hash, hint } => (Some(hash), Some(hint), None, None),
        Credential::SignIn {
            client_id,
            client_name,
        } => (None, None, Some(client_id), Some(client_name)),
    };
    sqlx::query_as!(
        Connection,
        r#"INSERT INTO agent_connections
               (user_id, name, key_hash, key_hint, client_id, client_name, "grant", expires_at)
           SELECT $1, $2, $3, $4, $5, $6, $7, $8
           WHERE (SELECT count(*) FROM agent_connections WHERE user_id = $1) < $9
           RETURNING id, user_id, name, key_hint, client_id, client_name, "grant", paused_at,
               expires_at, last_used_at, created_at, updated_at"#,
        new.user_id,
        new.name,
        key_hash,
        key_hint,
        client_id,
        client_name,
        Json(new.grant) as _,
        new.expires_at,
        MAX_PER_USER,
    )
    .fetch_optional(&mut *transaction)
    .await
}

pub async fn list(db: impl PgExecutor<'_>, user_id: Uuid) -> sqlx::Result<Vec<Connection>> {
    sqlx::query_as!(
        Connection,
        r#"SELECT id, user_id, name, key_hint, client_id, client_name, "grant", paused_at,
               expires_at, last_used_at, created_at, updated_at
           FROM agent_connections WHERE user_id = $1 ORDER BY created_at, id"#,
        user_id
    )
    .fetch_all(db)
    .await
}

pub async fn find(
    db: impl PgExecutor<'_>,
    id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Option<Connection>> {
    sqlx::query_as!(
        Connection,
        r#"SELECT id, user_id, name, key_hint, client_id, client_name, "grant", paused_at,
               expires_at, last_used_at, created_at, updated_at
           FROM agent_connections WHERE id = $1 AND user_id = $2"#,
        id,
        user_id
    )
    .fetch_optional(db)
    .await
}

/// Paused and expired connections are found too, so the caller can tell the agent which
pub async fn find_by_key(
    db: impl PgExecutor<'_>,
    key_hash: &[u8],
) -> sqlx::Result<Option<KeyMatch>> {
    sqlx::query_as!(
        Found,
        r#"SELECT id, user_id, name, key_hint, client_id, client_name, "grant", paused_at,
               expires_at, last_used_at, created_at, updated_at,
               paused_at IS NOT NULL AS "paused!",
               coalesce(expires_at <= now(), false) AS "expired!"
           FROM agent_connections WHERE key_hash = $1"#,
        key_hash
    )
    .fetch_optional(db)
    .await
    .map(|found| found.map(KeyMatch::from))
}

/// [`find_by_key`] for an access token that hasn't run out
pub async fn find_by_token(
    db: impl PgExecutor<'_>,
    access_hash: &[u8],
) -> sqlx::Result<Option<KeyMatch>> {
    sqlx::query_as!(
        Found,
        r#"SELECT c.id, c.user_id, c.name, c.key_hint, c.client_id, c.client_name, c."grant",
               c.paused_at, c.expires_at, c.last_used_at, c.created_at, c.updated_at,
               c.paused_at IS NOT NULL AS "paused!",
               coalesce(c.expires_at <= now(), false) AS "expired!"
           FROM agent_tokens t JOIN agent_connections c ON c.id = t.connection_id
           WHERE t.access_hash = $1 AND t.access_expires_at > now()"#,
        access_hash
    )
    .fetch_optional(db)
    .await
    .map(|found| found.map(KeyMatch::from))
}

struct Found {
    id: Uuid,
    user_id: Uuid,
    name: String,
    key_hint: Option<String>,
    client_id: Option<String>,
    client_name: Option<String>,
    grant: serde_json::Value,
    paused_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
    last_used_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    paused: bool,
    expired: bool,
}

impl From<Found> for KeyMatch {
    fn from(found: Found) -> Self {
        Self {
            connection: Connection {
                id: found.id,
                user_id: found.user_id,
                name: found.name,
                key_hint: found.key_hint,
                client_id: found.client_id,
                client_name: found.client_name,
                grant: found.grant,
                paused_at: found.paused_at,
                expires_at: found.expires_at,
                last_used_at: found.last_used_at,
                created_at: found.created_at,
                updated_at: found.updated_at,
            },
            paused: found.paused,
            expired: found.expired,
        }
    }
}

pub async fn rename(
    db: impl PgExecutor<'_>,
    id: Uuid,
    user_id: Uuid,
    name: &str,
) -> sqlx::Result<Option<Connection>> {
    sqlx::query_as!(
        Connection,
        r#"UPDATE agent_connections SET name = $3, updated_at = now()
           WHERE id = $1 AND user_id = $2
           RETURNING id, user_id, name, key_hint, client_id, client_name, "grant", paused_at,
               expires_at, last_used_at, created_at, updated_at"#,
        id,
        user_id,
        name
    )
    .fetch_optional(db)
    .await
}

pub async fn set_grant(
    db: impl PgExecutor<'_>,
    id: Uuid,
    user_id: Uuid,
    grant: &Grant,
) -> sqlx::Result<Option<Connection>> {
    sqlx::query_as!(
        Connection,
        r#"UPDATE agent_connections SET "grant" = $3, updated_at = now()
           WHERE id = $1 AND user_id = $2
           RETURNING id, user_id, name, key_hint, client_id, client_name, "grant", paused_at,
               expires_at, last_used_at, created_at, updated_at"#,
        id,
        user_id,
        Json(grant) as _,
    )
    .fetch_optional(db)
    .await
}

/// Pausing a paused connection keeps when it was first paused
pub async fn pause(
    db: impl PgExecutor<'_>,
    id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Option<Connection>> {
    sqlx::query_as!(
        Connection,
        r#"UPDATE agent_connections
           SET paused_at = coalesce(paused_at, now()), updated_at = now()
           WHERE id = $1 AND user_id = $2
           RETURNING id, user_id, name, key_hint, client_id, client_name, "grant", paused_at,
               expires_at, last_used_at, created_at, updated_at"#,
        id,
        user_id
    )
    .fetch_optional(db)
    .await
}

pub async fn resume(
    db: impl PgExecutor<'_>,
    id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Option<Connection>> {
    sqlx::query_as!(
        Connection,
        r#"UPDATE agent_connections SET paused_at = NULL, updated_at = now()
           WHERE id = $1 AND user_id = $2
           RETURNING id, user_id, name, key_hint, client_id, client_name, "grant", paused_at,
               expires_at, last_used_at, created_at, updated_at"#,
        id,
        user_id
    )
    .fetch_optional(db)
    .await
}

/// How many were running before
pub async fn pause_all(db: impl PgExecutor<'_>, user_id: Uuid) -> sqlx::Result<u64> {
    let paused = sqlx::query!(
        "UPDATE agent_connections SET paused_at = now(), updated_at = now() \
         WHERE user_id = $1 AND paused_at IS NULL",
        user_id
    )
    .execute(db)
    .await?;
    Ok(paused.rows_affected())
}

/// The old key stops working at once. `None` too for a connection made by signing in, which has
/// no key.
pub async fn rotate_key(
    db: impl PgExecutor<'_>,
    id: Uuid,
    user_id: Uuid,
    key_hash: &[u8],
    key_hint: &str,
) -> sqlx::Result<Option<Connection>> {
    sqlx::query_as!(
        Connection,
        r#"UPDATE agent_connections SET key_hash = $3, key_hint = $4, updated_at = now()
           WHERE id = $1 AND user_id = $2 AND key_hash IS NOT NULL
           RETURNING id, user_id, name, key_hint, client_id, client_name, "grant", paused_at,
               expires_at, last_used_at, created_at, updated_at"#,
        id,
        user_id,
        key_hash,
        key_hint
    )
    .fetch_optional(db)
    .await
}

/// What revoking did to the schedules the connection created or last changed
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Revoked {
    pub schedules_deleted: u64,
    /// Kept without the connection, now the owner's own
    pub schedules_kept: u64,
}

/// Revokes a connection. Its approvals go with it; its deliveries, activity and, unless
/// `delete_schedules`, its schedules stay without it. `None` when there's no such connection.
pub async fn delete(
    pool: &PgPool,
    id: Uuid,
    user_id: Uuid,
    delete_schedules: bool,
) -> sqlx::Result<Option<Revoked>> {
    let mut transaction = pool.begin().await?;
    let exists = sqlx::query_scalar!(
        "SELECT id FROM agent_connections WHERE id = $1 AND user_id = $2 FOR UPDATE",
        id,
        user_id
    )
    .fetch_optional(&mut *transaction)
    .await?;
    if exists.is_none() {
        return Ok(None);
    }
    // Before the connection goes, since its delete clears `connection_id` on what it made
    let revoked = if delete_schedules {
        let deleted = sqlx::query!(
            "DELETE FROM scheduled_messages WHERE connection_id = $1 AND user_id = $2",
            id,
            user_id
        )
        .execute(&mut *transaction)
        .await?;
        Revoked {
            schedules_deleted: deleted.rows_affected(),
            schedules_kept: 0,
        }
    } else {
        let kept = schedules(&mut *transaction, id, user_id).await?;
        Revoked {
            schedules_deleted: 0,
            schedules_kept: kept.unsigned_abs(),
        }
    };
    sqlx::query!(
        "DELETE FROM agent_connections WHERE id = $1 AND user_id = $2",
        id,
        user_id
    )
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(Some(revoked))
}

/// How many of the user's scheduled messages belong to the connection: those it created or last
/// changed
pub async fn schedules(db: impl PgExecutor<'_>, id: Uuid, user_id: Uuid) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM scheduled_messages
           WHERE connection_id = $1 AND user_id = $2"#,
        id,
        user_id
    )
    .fetch_one(db)
    .await
}

/// Scheduled messages that belong to `connection_id` and its requests for a new one still
/// waiting, counted as the cap of 50 counts them
pub async fn held_schedules(db: impl PgExecutor<'_>, connection_id: Uuid) -> sqlx::Result<u32> {
    let count = sqlx::query_scalar!(
        r#"SELECT (SELECT count(*) FROM scheduled_messages WHERE connection_id = $1)
               + (SELECT count(*) FROM agent_approvals
                  WHERE connection_id = $1 AND kind = 'schedule_create'
                      AND status = 'waiting' AND expires_at > now()) AS "count!""#,
        connection_id,
    )
    .fetch_one(db)
    .await?;
    Ok(u32::try_from(count).unwrap_or(u32::MAX))
}

/// Records a call, writing at most once a minute per connection
pub async fn touch(db: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE agent_connections SET last_used_at = now() \
         WHERE id = $1 AND (last_used_at IS NULL OR last_used_at < now() - interval '1 minute')",
        id
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Sends by `connection_id` in the last 24 hours, counted as [`counts`] does, and its approvals
/// waiting: what `whoami` shows of the limits
pub async fn usage(db: impl PgExecutor<'_>, connection_id: Uuid) -> sqlx::Result<(u32, u32)> {
    let row = sqlx::query!(
        r#"SELECT
               (SELECT count(*) FROM deliveries
                WHERE connection_id = $1 AND NOT skipped
                    AND created_at > now() - interval '24 hours')
               + (SELECT count(*) FROM agent_approvals
                  WHERE connection_id = $1 AND kind = 'send' AND status = 'waiting'
                      AND expires_at > now()) AS "sends!",
               (SELECT count(*) FROM agent_approvals
                WHERE connection_id = $1 AND status = 'waiting' AND expires_at > now())
                   AS "waiting!",
               (SELECT count(*) FROM scheduled_messages WHERE connection_id = $1)
                   + (SELECT count(*) FROM agent_approvals
                      WHERE connection_id = $1 AND kind = 'schedule_create'
                          AND status = 'waiting' AND expires_at > now())
                   AS "schedules!""#,
        connection_id,
    )
    .fetch_one(db)
    .await?;
    let small = |count: i64| u32::try_from(count).unwrap_or(u32::MAX);
    Ok((small(row.sends), small(row.waiting)))
}

/// [`usage`] for each of the user's connections, in one query
pub async fn usage_all(
    db: impl PgExecutor<'_>,
    user_id: Uuid,
) -> sqlx::Result<HashMap<Uuid, (u32, u32)>> {
    let rows = sqlx::query!(
        r#"SELECT c.id,
               (SELECT count(*) FROM deliveries d
                WHERE d.connection_id = c.id AND NOT d.skipped
                    AND d.created_at > now() - interval '24 hours')
               + (SELECT count(*) FROM agent_approvals a
                  WHERE a.connection_id = c.id AND a.kind = 'send' AND a.status = 'waiting'
                      AND a.expires_at > now()) AS "sends!",
               (SELECT count(*) FROM agent_approvals a
                WHERE a.connection_id = c.id AND a.status = 'waiting' AND a.expires_at > now())
                   AS "waiting!"
           FROM agent_connections c WHERE c.user_id = $1"#,
        user_id,
    )
    .fetch_all(db)
    .await?;
    let small = |count: i64| u32::try_from(count).unwrap_or(u32::MAX);
    Ok(rows
        .into_iter()
        .map(|row| (row.id, (small(row.sends), small(row.waiting))))
        .collect())
}

/// Sends by `connection_id` in the last 24 hours, to one chat and in all, counting the deliveries
/// it caused and its send approvals still waiting; how many approvals of any kind wait; and how
/// many scheduled messages it has or is waiting to create
pub async fn counts(
    db: impl PgExecutor<'_>,
    connection_id: Uuid,
    recipient_type: RecipientType,
    recipient: &str,
) -> sqlx::Result<Counts> {
    let row = sqlx::query!(
        r#"WITH sends AS (
               SELECT recipient_type, recipient FROM deliveries
               WHERE connection_id = $1 AND NOT skipped
                   AND created_at > now() - interval '24 hours'
               UNION ALL
               SELECT recipient_type, recipient FROM agent_approvals
               WHERE connection_id = $1 AND kind = 'send' AND status = 'waiting'
                   AND expires_at > now()
           )
           SELECT
               (SELECT count(*) FROM sends WHERE recipient_type = $2 AND recipient = $3)
                   AS "chat!",
               (SELECT count(*) FROM sends) AS "connection!",
               (SELECT count(*) FROM agent_approvals
                WHERE connection_id = $1 AND status = 'waiting' AND expires_at > now())
                   AS "waiting!",
               (SELECT count(*) FROM scheduled_messages WHERE connection_id = $1)
                   + (SELECT count(*) FROM agent_approvals
                      WHERE connection_id = $1 AND kind = 'schedule_create'
                          AND status = 'waiting' AND expires_at > now())
                   AS "schedules!""#,
        connection_id,
        recipient_type.as_str(),
        recipient,
    )
    .fetch_one(db)
    .await?;
    let small = |count: i64| u32::try_from(count).unwrap_or(u32::MAX);
    Ok(Counts {
        chat_sends: small(row.chat),
        connection_sends: small(row.connection),
        waiting_approvals: small(row.waiting),
        schedules: small(row.schedules),
    })
}
