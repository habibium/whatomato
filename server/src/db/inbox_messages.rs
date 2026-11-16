use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

pub struct NewMessage<'a> {
    pub account_id: Uuid,
    pub chat_type: &'a str,
    pub chat_id: &'a str,
    pub message_id: &'a str,
    pub from_me: bool,
    pub sent_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub body: &'a [u8],
}

pub struct Kept {
    pub account_id: Uuid,
    pub chat_type: String,
    pub chat_id: String,
    pub message_id: String,
    pub from_me: bool,
    pub sent_at: DateTime<Utc>,
    pub body: Vec<u8>,
}

pub struct ChatSummary {
    pub chat_type: String,
    pub chat_id: String,
    pub last_message_at: DateTime<Utc>,
    pub messages: i64,
}

/// A message WhatsApp delivers twice is kept once
pub async fn save(db: impl PgExecutor<'_>, message: NewMessage<'_>) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO inbox_messages
             (account_id, chat_type, chat_id, message_id, from_me, sent_at, expires_at, body)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT DO NOTHING",
        message.account_id,
        message.chat_type,
        message.chat_id,
        message.message_id,
        message.from_me,
        message.sent_at,
        message.expires_at,
        message.body,
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn find_in_account(
    db: impl PgExecutor<'_>,
    account_id: Uuid,
    chat_type: &str,
    chat_id: &str,
    message_id: &str,
) -> sqlx::Result<Option<Kept>> {
    sqlx::query_as!(
        Kept,
        "SELECT account_id, chat_type, chat_id, message_id, from_me, sent_at, body
         FROM inbox_messages
         WHERE account_id = $1 AND chat_type = $2 AND chat_id = $3 AND message_id = $4",
        account_id,
        chat_type,
        chat_id,
        message_id,
    )
    .fetch_optional(db)
    .await
}

pub async fn replace_body(
    db: impl PgExecutor<'_>,
    account_id: Uuid,
    chat_type: &str,
    chat_id: &str,
    message_id: &str,
    body: &[u8],
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE inbox_messages SET body = $5
         WHERE account_id = $1 AND chat_type = $2 AND chat_id = $3 AND message_id = $4",
        account_id,
        chat_type,
        chat_id,
        message_id,
        body,
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn remove(
    db: impl PgExecutor<'_>,
    account_id: Uuid,
    chat_type: &str,
    chat_id: &str,
    message_id: &str,
) -> sqlx::Result<()> {
    sqlx::query!(
        "DELETE FROM inbox_messages
         WHERE account_id = $1 AND chat_type = $2 AND chat_id = $3 AND message_id = $4",
        account_id,
        chat_type,
        chat_id,
        message_id,
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn remove_chat(
    db: impl PgExecutor<'_>,
    account_id: Uuid,
    chat_type: &str,
    chat_id: &str,
) -> sqlx::Result<()> {
    sqlx::query!(
        "DELETE FROM inbox_messages WHERE account_id = $1 AND chat_type = $2 AND chat_id = $3",
        account_id,
        chat_type,
        chat_id,
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Drops what is older than the window, and what a disappearing chat's timer has ended
pub async fn sweep(db: impl PgExecutor<'_>, days: i32) -> sqlx::Result<u64> {
    let swept = sqlx::query!(
        "DELETE FROM inbox_messages
         WHERE sent_at < now() - make_interval(days => $1) OR expires_at <= now()",
        days,
    )
    .execute(db)
    .await?;
    Ok(swept.rows_affected())
}

/// Every chat with a kept message, across accounts, for dropping those nobody may read any more
pub async fn kept_chats(db: impl PgExecutor<'_>) -> sqlx::Result<Vec<(Uuid, String, String)>> {
    let chats = sqlx::query!("SELECT DISTINCT account_id, chat_type, chat_id FROM inbox_messages")
        .fetch_all(db)
        .await?;
    Ok(chats
        .into_iter()
        .map(|chat| (chat.account_id, chat.chat_type, chat.chat_id))
        .collect())
}

/// The owner's chats with a kept message, the latest first
pub async fn chats(db: impl PgExecutor<'_>, user_id: Uuid) -> sqlx::Result<Vec<ChatSummary>> {
    sqlx::query_as!(
        ChatSummary,
        r#"SELECT m.chat_type, m.chat_id, max(m.sent_at) AS "last_message_at!",
                  count(*) AS "messages!"
           FROM inbox_messages m
           JOIN whatsapp_accounts a ON a.id = m.account_id
           WHERE a.user_id = $1 AND (m.expires_at IS NULL OR m.expires_at > now())
           GROUP BY m.chat_type, m.chat_id
           ORDER BY max(m.sent_at) DESC, m.chat_id"#,
        user_id,
    )
    .fetch_all(db)
    .await
}

/// A chat's messages, the latest first, from the one just older than `before`: a kept message's
/// time and id
pub async fn page(
    db: impl PgExecutor<'_>,
    user_id: Uuid,
    chat_type: &str,
    chat_id: &str,
    before: Option<(DateTime<Utc>, &str)>,
    limit: i64,
) -> sqlx::Result<Vec<Kept>> {
    let (before_at, before_id) = before.unzip();
    sqlx::query_as!(
        Kept,
        "SELECT m.account_id, m.chat_type, m.chat_id, m.message_id, m.from_me, m.sent_at, m.body
         FROM inbox_messages m
         JOIN whatsapp_accounts a ON a.id = m.account_id
         WHERE a.user_id = $1 AND m.chat_type = $2 AND m.chat_id = $3
           AND ($4::timestamptz IS NULL OR (m.sent_at, m.message_id) < ($4, $5))
           AND (m.expires_at IS NULL OR m.expires_at > now())
         ORDER BY m.sent_at DESC, m.message_id DESC
         LIMIT $6",
        user_id,
        chat_type,
        chat_id,
        before_at,
        before_id,
        limit,
    )
    .fetch_all(db)
    .await
}

pub async fn find(
    db: impl PgExecutor<'_>,
    user_id: Uuid,
    chat_type: &str,
    chat_id: &str,
    message_id: &str,
) -> sqlx::Result<Option<Kept>> {
    sqlx::query_as!(
        Kept,
        "SELECT m.account_id, m.chat_type, m.chat_id, m.message_id, m.from_me, m.sent_at, m.body
         FROM inbox_messages m
         JOIN whatsapp_accounts a ON a.id = m.account_id
         WHERE a.user_id = $1 AND m.chat_type = $2 AND m.chat_id = $3 AND m.message_id = $4
           AND (m.expires_at IS NULL OR m.expires_at > now())",
        user_id,
        chat_type,
        chat_id,
        message_id,
    )
    .fetch_optional(db)
    .await
}

/// The owner's latest kept messages across chats, for a search to open and scan
pub async fn latest(db: impl PgExecutor<'_>, user_id: Uuid, limit: i64) -> sqlx::Result<Vec<Kept>> {
    sqlx::query_as!(
        Kept,
        "SELECT m.account_id, m.chat_type, m.chat_id, m.message_id, m.from_me, m.sent_at, m.body
         FROM inbox_messages m
         JOIN whatsapp_accounts a ON a.id = m.account_id
         WHERE a.user_id = $1 AND (m.expires_at IS NULL OR m.expires_at > now())
         ORDER BY m.sent_at DESC, m.message_id DESC
         LIMIT $2",
        user_id,
        limit,
    )
    .fetch_all(db)
    .await
}
