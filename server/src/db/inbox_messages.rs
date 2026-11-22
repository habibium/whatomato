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
    pub terms: &'a [i32],
}

/// Messages of one account as parallel columns, the form Postgres's `unnest` takes
#[derive(Default)]
pub struct NewMessages {
    pub chat_types: Vec<String>,
    pub chat_ids: Vec<String>,
    pub message_ids: Vec<String>,
    pub from_me: Vec<bool>,
    pub sent_at: Vec<DateTime<Utc>>,
    pub expires_at: Vec<Option<DateTime<Utc>>>,
    pub bodies: Vec<Vec<u8>>,
    /// Each message's terms as an array literal, since `unnest` would flatten an array of arrays
    pub terms: Vec<String>,
}

impl NewMessages {
    pub fn push(&mut self, message: &NewMessage<'_>) {
        self.chat_types.push(message.chat_type.to_owned());
        self.chat_ids.push(message.chat_id.to_owned());
        self.message_ids.push(message.message_id.to_owned());
        self.from_me.push(message.from_me);
        self.sent_at.push(message.sent_at);
        self.expires_at.push(message.expires_at);
        self.bodies.push(message.body.to_owned());
        self.terms.push(array_literal(message.terms));
    }
}

fn array_literal(terms: &[i32]) -> String {
    let joined: Vec<String> = terms.iter().map(i32::to_string).collect();
    format!("{{{}}}", joined.join(","))
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
             (account_id, chat_type, chat_id, message_id, from_me, sent_at, expires_at, body,
              terms)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT DO NOTHING",
        message.account_id,
        message.chat_type,
        message.chat_id,
        message.message_id,
        message.from_me,
        message.sent_at,
        message.expires_at,
        message.body,
        message.terms,
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Keeps a batch of one account's messages, skipping those already kept, and says how many were
/// new
pub async fn save_many(
    db: impl PgExecutor<'_>,
    account_id: Uuid,
    messages: &NewMessages,
) -> sqlx::Result<u64> {
    let saved = sqlx::query!(
        "INSERT INTO inbox_messages
             (account_id, chat_type, chat_id, message_id, from_me, sent_at, expires_at, body,
              terms)
         SELECT $1, chat_type, chat_id, message_id, from_me, sent_at, expires_at, body,
                terms::integer[]
         FROM unnest($2::text[], $3::text[], $4::text[], $5::boolean[], $6::timestamptz[],
                     $7::timestamptz[], $8::bytea[], $9::text[])
              AS m (chat_type, chat_id, message_id, from_me, sent_at, expires_at, body, terms)
         ON CONFLICT DO NOTHING",
        account_id,
        &messages.chat_types,
        &messages.chat_ids,
        &messages.message_ids,
        &messages.from_me,
        &messages.sent_at,
        &messages.expires_at as &[Option<DateTime<Utc>>],
        &messages.bodies,
        &messages.terms,
    )
    .execute(db)
    .await?;
    Ok(saved.rows_affected())
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
    message: &Kept,
    body: &[u8],
    terms: &[i32],
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE inbox_messages SET body = $5, terms = $6
         WHERE account_id = $1 AND chat_type = $2 AND chat_id = $3 AND message_id = $4",
        message.account_id,
        message.chat_type,
        message.chat_id,
        message.message_id,
        body,
        terms,
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Messages kept before they had terms, a few at a time
pub async fn without_terms(db: impl PgExecutor<'_>, limit: i64) -> sqlx::Result<Vec<Kept>> {
    sqlx::query_as!(
        Kept,
        "SELECT account_id, chat_type, chat_id, message_id, from_me, sent_at, body
         FROM inbox_messages
         WHERE terms IS NULL
         LIMIT $1",
        limit,
    )
    .fetch_all(db)
    .await
}

pub async fn set_terms(db: impl PgExecutor<'_>, message: &Kept, terms: &[i32]) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE inbox_messages SET terms = $5
         WHERE account_id = $1 AND chat_type = $2 AND chat_id = $3 AND message_id = $4",
        message.account_id,
        message.chat_type,
        message.chat_id,
        message.message_id,
        terms,
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

/// Drops what a disappearing chat's timer has ended, and what is older than its account keeps:
/// `days` for an owner who doesn't keep every chat, else as many days as they chose
pub async fn sweep(db: impl PgExecutor<'_>, days: i32) -> sqlx::Result<u64> {
    let swept = sqlx::query!(
        "DELETE FROM inbox_messages m
         USING whatsapp_accounts a
         LEFT JOIN message_history h ON h.user_id = a.user_id
         WHERE m.account_id = a.id
           AND (m.expires_at <= now()
                OR m.sent_at < now() - make_interval(
                    days => CASE WHEN h.user_id IS NULL THEN $1 ELSE h.keep_days END))",
        days,
    )
    .execute(db)
    .await?;
    Ok(swept.rows_affected())
}

/// Every chat with a kept message in an account whose owner doesn't keep every chat, for dropping
/// those nobody may read any more
pub async fn kept_chats(db: impl PgExecutor<'_>) -> sqlx::Result<Vec<(Uuid, String, String)>> {
    let chats = sqlx::query!(
        "SELECT DISTINCT m.account_id, m.chat_type, m.chat_id
         FROM whatsapp_accounts a
         JOIN inbox_messages m ON m.account_id = a.id
         WHERE NOT EXISTS (SELECT 1 FROM message_history h WHERE h.user_id = a.user_id)"
    )
    .fetch_all(db)
    .await?;
    Ok(chats
        .into_iter()
        .map(|chat| (chat.account_id, chat.chat_type, chat.chat_id))
        .collect())
}

/// The owner's chats with a kept message no older than `since`, the latest first
pub async fn chats(
    db: impl PgExecutor<'_>,
    user_id: Uuid,
    since: Option<DateTime<Utc>>,
) -> sqlx::Result<Vec<ChatSummary>> {
    sqlx::query_as!(
        ChatSummary,
        r#"SELECT m.chat_type, m.chat_id, max(m.sent_at) AS "last_message_at!",
                  count(*) AS "messages!"
           FROM inbox_messages m
           JOIN whatsapp_accounts a ON a.id = m.account_id
           WHERE a.user_id = $1 AND (m.expires_at IS NULL OR m.expires_at > now())
             AND ($2::timestamptz IS NULL OR m.sent_at >= $2)
           GROUP BY m.chat_type, m.chat_id
           ORDER BY max(m.sent_at) DESC, m.chat_id"#,
        user_id,
        since,
    )
    .fetch_all(db)
    .await
}

/// Which chat to look in, and how far back
#[derive(Clone, Copy)]
pub struct Scope<'a> {
    pub user_id: Uuid,
    /// One chat's type and id, or every chat
    pub chat: Option<(&'a str, &'a str)>,
    /// Nothing older is returned
    pub since: Option<DateTime<Utc>>,
}

/// Messages, the latest first, from the one just older than `before`: a kept message's time and id
pub async fn page(
    db: impl PgExecutor<'_>,
    scope: Scope<'_>,
    before: Option<(DateTime<Utc>, &str)>,
    limit: i64,
) -> sqlx::Result<Vec<Kept>> {
    let (chat_type, chat_id) = scope.chat.unzip();
    let (before_at, before_id) = before.unzip();
    sqlx::query_as!(
        Kept,
        "SELECT m.account_id, m.chat_type, m.chat_id, m.message_id, m.from_me, m.sent_at, m.body
         FROM inbox_messages m
         JOIN whatsapp_accounts a ON a.id = m.account_id
         WHERE a.user_id = $1
           AND ($2::text IS NULL OR (m.chat_type = $2 AND m.chat_id = $3))
           AND ($4::timestamptz IS NULL OR m.sent_at >= $4)
           AND ($5::timestamptz IS NULL OR (m.sent_at, m.message_id) < ($5, $6))
           AND (m.expires_at IS NULL OR m.expires_at > now())
         ORDER BY m.sent_at DESC, m.message_id DESC
         LIMIT $7",
        scope.user_id,
        chat_type,
        chat_id,
        scope.since,
        before_at,
        before_id,
        limit,
    )
    .fetch_all(db)
    .await
}

/// As [`page`], only the messages holding every one of `terms`
pub async fn matching(
    db: impl PgExecutor<'_>,
    scope: Scope<'_>,
    terms: &[i32],
    before: Option<(DateTime<Utc>, &str)>,
    limit: i64,
) -> sqlx::Result<Vec<Kept>> {
    let (chat_type, chat_id) = scope.chat.unzip();
    let (before_at, before_id) = before.unzip();
    sqlx::query_as!(
        Kept,
        "SELECT m.account_id, m.chat_type, m.chat_id, m.message_id, m.from_me, m.sent_at, m.body
         FROM inbox_messages m
         JOIN whatsapp_accounts a ON a.id = m.account_id
         WHERE a.user_id = $1 AND m.terms @> $2
           AND ($3::text IS NULL OR (m.chat_type = $3 AND m.chat_id = $4))
           AND ($5::timestamptz IS NULL OR m.sent_at >= $5)
           AND ($6::timestamptz IS NULL OR (m.sent_at, m.message_id) < ($6, $7))
           AND (m.expires_at IS NULL OR m.expires_at > now())
         ORDER BY m.sent_at DESC, m.message_id DESC
         LIMIT $8",
        scope.user_id,
        terms,
        chat_type,
        chat_id,
        scope.since,
        before_at,
        before_id,
        limit,
    )
    .fetch_all(db)
    .await
}

pub async fn find(
    db: impl PgExecutor<'_>,
    scope: Scope<'_>,
    message_id: &str,
) -> sqlx::Result<Option<Kept>> {
    let (chat_type, chat_id) = scope.chat.unzip();
    sqlx::query_as!(
        Kept,
        "SELECT m.account_id, m.chat_type, m.chat_id, m.message_id, m.from_me, m.sent_at, m.body
         FROM inbox_messages m
         JOIN whatsapp_accounts a ON a.id = m.account_id
         WHERE a.user_id = $1 AND m.message_id = $2
           AND ($3::text IS NULL OR (m.chat_type = $3 AND m.chat_id = $4))
           AND ($5::timestamptz IS NULL OR m.sent_at >= $5)
           AND (m.expires_at IS NULL OR m.expires_at > now())
         LIMIT 1",
        scope.user_id,
        message_id,
        chat_type,
        chat_id,
        scope.since,
    )
    .fetch_optional(db)
    .await
}
