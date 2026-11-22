//! The phone's message history.
//!
//! WhatsApp sends it in chunks once the device links. Each chunk is sealed into Postgres before
//! WhatsApp is told it arrived, then imported into the inbox apart from the receive path, so a
//! restart part way through loses nothing.

use std::{collections::HashMap, sync::Arc, time::Duration};

use chrono::{DateTime, TimeDelta, Utc};
use sqlx::PgPool;
use tokio::sync::Notify;
use uuid::Uuid;
use whatsapp_rust::{
    Client, HistorySyncAdmission, HistorySyncCaptureHook, HistorySyncDecision, HistorySyncMetadata,
    prelude::{Jid, wa},
    wacore::history_sync::{HistorySyncStream, MAX_DECOMPRESSED},
};

use super::{Body, Chat, Content, Readers, chat_of_jid, content_of, named, seal_body, terms};
use crate::{
    crypto::Cipher,
    db::{history_chunks, inbox_messages, whatsapp_accounts},
    scheduler::RecipientType,
};

/// How long the import waits before looking again after a failure, or for a chunk it wasn't woken
/// for
const RETRY: Duration = Duration::from_secs(30);
/// Messages written in one statement
const BATCH: usize = 500;
/// Messages kept before terms existed, indexed in one pass
const INDEX_BATCH: i64 = 200;

/// Holds an account's history chunks until they're imported
pub struct Capture {
    pub(super) account_id: Uuid,
    pub(super) pool: PgPool,
    pub(super) cipher: Cipher,
    pub(super) readers: Readers,
    pub(super) imports: Arc<Notify>,
}

#[async_trait::async_trait]
impl HistorySyncCaptureHook for Capture {
    async fn on_history_sync(
        &self,
        _client: Arc<Client>,
        message_id: &str,
        sync_type: Option<wa::message::HistorySyncType>,
        compressed: &[u8],
    ) -> anyhow::Result<()> {
        use wa::message::HistorySyncType;

        let holds_messages = matches!(
            sync_type,
            Some(
                HistorySyncType::INITIAL_BOOTSTRAP
                    | HistorySyncType::FULL
                    | HistorySyncType::RECENT
                    | HistorySyncType::ON_DEMAND
            )
        );
        if !holds_messages || !self.readers.history(self.account_id).every_chat() {
            return Ok(());
        }
        let sealed = self
            .cipher
            .encrypt(compressed, &context(self.account_id, message_id))?;
        history_chunks::save(&self.pool, self.account_id, message_id, &sealed).await?;
        self.imports.notify_one();
        Ok(())
    }
}

/// Acknowledges history unread for an owner who doesn't keep it, as skipping it always did
pub struct Admission {
    pub(super) account_id: Uuid,
    pub(super) readers: Readers,
}

impl HistorySyncAdmission for Admission {
    fn decide(&self, _metadata: &HistorySyncMetadata<'_>) -> HistorySyncDecision {
        if self.readers.history(self.account_id).every_chat() {
            HistorySyncDecision::Accept
        } else {
            HistorySyncDecision::RejectAndAcknowledge
        }
    }
}

fn context(account_id: Uuid, message_id: &str) -> Vec<u8> {
    [
        b"history:".as_slice(),
        account_id.as_bytes(),
        b":",
        message_id.as_bytes(),
    ]
    .concat()
}

/// Imports held chunks one at a time, oldest first, for as long as the server runs
pub(super) async fn import(pool: PgPool, cipher: Cipher, readers: Readers, imports: Arc<Notify>) {
    loop {
        match history_chunks::oldest(&pool).await {
            Ok(Some(chunk)) => {
                let (account_id, message_id) = (chunk.account_id, chunk.message_id.clone());
                if let Err(error) = import_chunk(&pool, &cipher, &readers, chunk).await {
                    tracing::warn!(%account_id, message_id, "couldn't import WhatsApp history: {error:#}");
                    tokio::time::sleep(RETRY).await;
                }
            }
            Ok(None) => {
                tokio::select! {
                    () = imports.notified() => {}
                    () = tokio::time::sleep(RETRY) => {}
                }
            }
            Err(error) => {
                tracing::warn!(?error, "couldn't look for WhatsApp history to import");
                tokio::time::sleep(RETRY).await;
            }
        }
    }
}

/// A message read from a chunk, before its chat is named as a grant names it
#[derive(Debug)]
pub(super) struct Imported {
    pub chat: Chat,
    pub message_id: String,
    pub from_me: bool,
    pub sent_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub body: Body,
}

#[derive(Debug, Default)]
pub(super) struct Parsed {
    pub messages: Vec<Imported>,
    /// From 0 to 100, how much of the history the phone has sent with this chunk
    pub progress: Option<u32>,
}

async fn import_chunk(
    pool: &PgPool,
    cipher: &Cipher,
    readers: &Readers,
    chunk: history_chunks::Chunk,
) -> anyhow::Result<()> {
    let account_id = chunk.account_id;
    let history = readers.history(account_id);
    let opened = cipher.decrypt(&chunk.sealed, &context(account_id, &chunk.message_id));
    let compressed = match opened {
        Ok(compressed) if history.every_chat() => compressed,
        // Turned off since it arrived, or sealed under another key: nothing to keep either way
        _ => {
            history_chunks::delete(pool, account_id, &chunk.message_id).await?;
            return Ok(());
        }
    };
    let own = whatsapp_accounts::phone_number(pool, account_id)
        .await?
        .map_or_default(|number| format!("{number}@s.whatsapp.net"));
    let since = history.since(Utc::now());
    let parsed = tokio::task::spawn_blocking(move || parse(&compressed, since, &own)).await?;
    let parsed = match parsed {
        Ok(parsed) => parsed,
        Err(error) => {
            // A chunk that doesn't parse never will
            tracing::warn!(%account_id, "dropped WhatsApp history that didn't parse: {error}");
            history_chunks::delete(pool, account_id, &chunk.message_id).await?;
            return Ok(());
        }
    };
    let imported = keep(pool, cipher, account_id, parsed.messages).await?;
    whatsapp_accounts::add_history_import(
        pool,
        account_id,
        parsed
            .progress
            .and_then(|progress| i32::try_from(progress).ok()),
        i64::try_from(imported).unwrap_or(i64::MAX),
    )
    .await?;
    history_chunks::delete(pool, account_id, &chunk.message_id).await?;
    Ok(())
}

/// Seals and writes imported messages, and says how many weren't kept already
async fn keep(
    pool: &PgPool,
    cipher: &Cipher,
    account_id: Uuid,
    messages: Vec<Imported>,
) -> anyhow::Result<u64> {
    let mut names: HashMap<String, Option<String>> = HashMap::new();
    let mut named_messages = Vec::with_capacity(messages.len());
    for mut message in messages {
        let chat = match &message.chat {
            Chat::Lid(lid) => {
                let Some(number) = number_of(pool, account_id, lid, &mut names).await? else {
                    continue;
                };
                (RecipientType::Contact, number)
            }
            Chat::Number(number) => (RecipientType::Contact, number.clone()),
            Chat::Group(id) => (RecipientType::Group, id.clone()),
        };
        if !message.from_me && message.body.sender_number.is_none() {
            message.body.sender_number = match chat.0 {
                RecipientType::Contact => Some(chat.1.clone()),
                RecipientType::Group => match message.body.sender_jid.strip_suffix("@lid") {
                    Some(lid) => number_of(pool, account_id, lid, &mut names).await?,
                    None => None,
                },
            };
        }
        named_messages.push((chat, message));
    }

    let mut imported = 0;
    let mut remaining = named_messages.into_iter().peekable();
    while remaining.peek().is_some() {
        let batch: Vec<_> = remaining.by_ref().take(BATCH).collect();
        let cipher = cipher.clone();
        let sealed =
            tokio::task::spawn_blocking(move || seal_batch(&cipher, account_id, batch)).await??;
        imported += inbox_messages::save_many(pool, account_id, &sealed).await?;
    }
    Ok(imported)
}

/// The number behind a LID, remembered for the rest of the chunk
async fn number_of(
    pool: &PgPool,
    account_id: Uuid,
    lid: &str,
    names: &mut HashMap<String, Option<String>>,
) -> sqlx::Result<Option<String>> {
    if let Some(number) = names.get(lid) {
        return Ok(number.clone());
    }
    let found = named(pool, account_id, Chat::Lid(lid.to_owned()))
        .await?
        .map(|(_, number)| number);
    names.insert(lid.to_owned(), found.clone());
    Ok(found)
}

fn seal_batch(
    cipher: &Cipher,
    account_id: Uuid,
    batch: Vec<((RecipientType, String), Imported)>,
) -> anyhow::Result<inbox_messages::NewMessages> {
    let mut sealed = inbox_messages::NewMessages::default();
    for ((recipient_type, id), message) in batch {
        let body = seal_body(
            cipher,
            account_id,
            (recipient_type, &id),
            &message.message_id,
            &message.body,
        )?;
        let terms = terms::of_body(cipher, account_id, &message.body);
        sealed.push(&inbox_messages::NewMessage {
            account_id,
            chat_type: recipient_type.as_str(),
            chat_id: &id,
            message_id: &message.message_id,
            from_me: message.from_me,
            sent_at: message.sent_at,
            expires_at: message.expires_at,
            body: &body,
            terms: &terms,
        });
    }
    Ok(sealed)
}

/// Reads a chunk's messages no older than `since`. `own` is how the owner is addressed, for
/// quoting their messages. Edits and deletions apply to the messages of the same chunk; one whose
/// message came in another chunk is dropped.
pub(super) fn parse(
    compressed: &[u8],
    since: Option<DateTime<Utc>>,
    own: &str,
) -> Result<Parsed, String> {
    let mut stream = HistorySyncStream::new(compressed, MAX_DECOMPRESSED);
    let mut conversation = wa::Conversation::default();
    let mut messages = Vec::new();
    let now = Utc::now();
    while stream
        .next_conversation_into(&mut conversation)
        .map_err(|error| error.to_string())?
    {
        let Some(chat) = conversation_chat(&conversation) else {
            continue;
        };
        let mut kept: Vec<Imported> = Vec::new();
        let mut edits = Vec::new();
        let mut removed = Vec::new();
        for entry in &conversation.messages {
            let Some(info) = entry.message.as_option() else {
                continue;
            };
            match read_entry(info, &conversation.id, own) {
                Some(Entry::Message(message)) => {
                    let too_old = since.is_some_and(|since| message.sent_at < since);
                    let gone = message
                        .expires_at
                        .is_some_and(|expires_at| expires_at <= now);
                    if !too_old && !gone {
                        kept.push(Imported {
                            chat: chat.clone(),
                            ..*message
                        });
                    }
                }
                Some(Entry::Edit(message_id, text, mentions)) => {
                    edits.push((message_id, text, mentions));
                }
                Some(Entry::Revoke(message_id)) => removed.push(message_id),
                None => {}
            }
        }
        for (message_id, text, mentions) in edits {
            if let Some(message) = kept
                .iter_mut()
                .find(|message| message.message_id == message_id)
            {
                message.body.text = text;
                message.body.mentions = mentions;
                message.body.edited = true;
            }
        }
        messages.extend(
            kept.into_iter()
                .filter(|message| !removed.contains(&message.message_id)),
        );
    }
    let rest = stream.remainder().map_err(|error| error.to_string())?;
    Ok(Parsed {
        messages,
        progress: rest.progress,
    })
}

enum Entry {
    /// Its chat is filled in by the caller
    Message(Box<Imported>),
    Edit(String, Option<String>, Vec<String>),
    Revoke(String),
}

/// One message of the conversation `chat_jid` names, as the inbox keeps it
fn read_entry(info: &wa::WebMessageInfo, chat_jid: &str, own: &str) -> Option<Entry> {
    let key = info.key.as_option()?;
    let message_id = key.id.as_deref().filter(|id| !id.is_empty())?;
    let message = info.message.as_option()?;
    let sent_at = info
        .message_timestamp
        .and_then(|seconds| i64::try_from(seconds).ok())
        .and_then(|seconds| DateTime::from_timestamp(seconds, 0))?;
    let from_me = key.from_me.unwrap_or(false);
    let (kind, text, file_name, reply_to, mentions, media) = match content_of(message) {
        Content::Message {
            kind,
            text,
            file_name,
            reply_to,
            mentions,
            media,
        } => (kind, text, file_name, reply_to, mentions, media),
        Content::Edit {
            message_id,
            text,
            mentions,
        } => return Some(Entry::Edit(message_id, text, mentions)),
        Content::Revoke { message_id } => return Some(Entry::Revoke(message_id)),
        Content::Nothing => return None,
    };
    let sender = if from_me {
        own.to_owned()
    } else {
        key.participant
            .as_deref()
            .or(info.participant.as_deref())
            .filter(|jid| !jid.is_empty())
            .unwrap_or(chat_jid)
            .parse::<Jid>()
            .map_or_default(|jid| jid.to_non_ad().to_string())
    };
    let sender_number = (!from_me)
        .then(|| sender.strip_suffix("@s.whatsapp.net").map(str::to_owned))
        .flatten();
    let name = info.push_name.as_deref().map_or("", str::trim);
    Some(Entry::Message(Box::new(Imported {
        chat: Chat::Number(String::new()),
        message_id: message_id.to_owned(),
        from_me,
        sent_at,
        expires_at: expiry(info, sent_at),
        body: Body {
            sender_jid: sender,
            sender_number,
            sender_name: (!name.is_empty() && !from_me).then(|| name.to_owned()),
            kind,
            text,
            file_name,
            reply_to,
            mentions,
            edited: false,
            media,
        },
    })))
}

/// The chat a conversation is with: its number when WhatsApp gave one beside a LID
fn conversation_chat(conversation: &wa::Conversation) -> Option<Chat> {
    let jid = conversation.id.parse::<Jid>().ok()?;
    match chat_of_jid(&jid)? {
        Chat::Lid(lid) => Some(
            conversation
                .pn_jid
                .as_deref()
                .and_then(|pn| pn.parse::<Jid>().ok())
                .filter(|pn| pn.server.is_pn_family())
                .map_or(Chat::Lid(lid), |pn| Chat::Number(pn.user.to_string())),
        ),
        chat => Some(chat),
    }
}

/// When a disappearing message's timer ends
fn expiry(info: &wa::WebMessageInfo, sent_at: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let seconds = info.ephemeral_duration.filter(|seconds| *seconds > 0)?;
    let started = info
        .ephemeral_start_timestamp
        .and_then(|start| i64::try_from(start).ok())
        .filter(|start| *start > 0)
        .and_then(|start| DateTime::from_timestamp(start, 0))
        .unwrap_or(sent_at);
    Some(started + TimeDelta::seconds(i64::from(seconds)))
}

/// Gives terms to the messages kept before terms existed, then stops
pub(super) async fn index(pool: PgPool, cipher: Cipher) {
    loop {
        let batch = match inbox_messages::without_terms(&pool, INDEX_BATCH).await {
            Ok(batch) if batch.is_empty() => return,
            Ok(batch) => batch,
            Err(error) => {
                tracing::warn!(?error, "couldn't look for kept messages to index");
                tokio::time::sleep(RETRY).await;
                continue;
            }
        };
        for message in batch {
            // One that doesn't open is still given terms, or it would be picked again forever
            let terms = super::open_body(&cipher, &message)
                .map_or_default(|body| terms::of_body(&cipher, message.account_id, &body));
            if let Err(error) = inbox_messages::set_terms(&pool, &message, &terms).await {
                tracing::warn!(?error, "couldn't index a kept message");
                tokio::time::sleep(RETRY).await;
            }
        }
    }
}

#[cfg(test)]
mod tests;
