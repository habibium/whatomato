//! The recent messages of the chats an agent connection may read. A message of any other chat is
//! dropped as it arrives and never written.

use std::{
    collections::HashMap,
    sync::{Arc, PoisonError, RwLock},
    time::Duration,
};

use chrono::{DateTime, TimeDelta, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tokio::sync::mpsc;
use uuid::Uuid;
use whatsapp_rust::{
    Server,
    prelude::{Event as LibraryEvent, EventHandler, EventInterest, EventKind, Jid, wa},
    wacore::{
        proto_helpers::MessageExt,
        types::{events::InboundMessage, message::MessageSource},
    },
};

use crate::{
    agents::Grant,
    crypto::Cipher,
    db::{agent_connections, inbox_messages, whatsapp_contacts},
    scheduler::RecipientType,
};

/// How long a message is kept
pub const WINDOW_DAYS: i32 = 7;
/// How soon a changed permission reaches capture
const RELOAD_EVERY: Duration = Duration::from_secs(15);
/// The sweep runs once in this many reloads
const SWEEP_EVERY: u32 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Text,
    Image,
    Video,
    Audio,
    Document,
    Sticker,
    Location,
    Contact,
    Poll,
}

/// The sealed part of a kept message
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Body {
    /// As WhatsApp addressed the sender, which a quoted reply needs
    pub sender_jid: String,
    pub sender_number: Option<String>,
    /// The name the sender gave themselves
    pub sender_name: Option<String>,
    pub kind: Kind,
    /// The text, or a caption
    pub text: Option<String>,
    pub file_name: Option<String>,
    /// The id of the message this one answers
    pub reply_to: Option<String>,
    #[serde(default)]
    pub edited: bool,
}

/// # Errors
/// The body couldn't be serialized or sealed.
pub fn seal_body(
    cipher: &Cipher,
    account_id: Uuid,
    chat: (RecipientType, &str),
    message_id: &str,
    body: &Body,
) -> anyhow::Result<Vec<u8>> {
    cipher.encrypt(
        &serde_json::to_vec(body)?,
        &context(account_id, chat, message_id),
    )
}

/// # Errors
/// The body didn't open: another key, another row, or altered.
pub fn open_body(cipher: &Cipher, message: &inbox_messages::Kept) -> anyhow::Result<Body> {
    let recipient_type = chat_type(&message.chat_type)
        .ok_or_else(|| anyhow::anyhow!("unknown chat type {}", message.chat_type))?;
    let body = cipher.decrypt(
        &message.body,
        &context(
            message.account_id,
            (recipient_type, &message.chat_id),
            &message.message_id,
        ),
    )?;
    Ok(serde_json::from_slice(&body)?)
}

fn context(account_id: Uuid, chat: (RecipientType, &str), message_id: &str) -> Vec<u8> {
    [
        b"inbox:".as_slice(),
        account_id.as_bytes(),
        b":",
        chat.0.as_str().as_bytes(),
        b":",
        chat.1.as_bytes(),
        b":",
        message_id.as_bytes(),
    ]
    .concat()
}

fn chat_type(stored: &str) -> Option<RecipientType> {
    match stored {
        "contact" => Some(RecipientType::Contact),
        "group" => Some(RecipientType::Group),
        _ => None,
    }
}

/// The grants that read at least one chat, by WhatsApp account. Held in memory so the receive
/// path decides without a query.
#[derive(Clone, Default)]
pub struct Readers(Arc<RwLock<HashMap<Uuid, Vec<Grant>>>>);

impl Readers {
    #[must_use]
    pub fn reads(&self, account_id: Uuid, recipient_type: RecipientType, recipient: &str) -> bool {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&account_id)
            .is_some_and(|grants| {
                grants
                    .iter()
                    .any(|grant| grant.reads(recipient_type, recipient))
            })
    }

    fn reads_anything(&self, account_id: Uuid) -> bool {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(&account_id)
    }

    /// Reads every connection's permissions again
    ///
    /// # Errors
    /// The connections couldn't be read; the list stays as it was.
    pub async fn reload(&self, pool: &PgPool) -> sqlx::Result<()> {
        let mut readers: HashMap<Uuid, Vec<Grant>> = HashMap::new();
        for (account_id, grant) in agent_connections::grants_by_account(pool).await? {
            // A grant that doesn't parse gives no access, so it reads nothing
            if let Ok(grant) = Grant::parse(&grant)
                && grant.reads_any()
            {
                readers.entry(account_id).or_default().push(grant);
            }
        }
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = readers;
        Ok(())
    }
}

/// A chat as a message names it
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Chat {
    Group(String),
    Number(String),
    /// A direct chat whose number the message didn't carry; the address book may know it
    Lid(String),
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Change {
    Arrived {
        chat: Chat,
        message_id: String,
        from_me: bool,
        sent_at: DateTime<Utc>,
        expires_at: Option<DateTime<Utc>>,
        body: Body,
    },
    Edited {
        chat: Chat,
        message_id: String,
        text: Option<String>,
    },
    Removed {
        chat: Chat,
        message_id: String,
    },
    ChatRemoved {
        chat: Chat,
    },
}

pub(super) type Changes = mpsc::UnboundedSender<(Uuid, Change)>;

/// What capture shares across accounts
#[derive(Clone)]
pub struct Inbox {
    readers: Readers,
    changes: Changes,
}

impl Inbox {
    /// Starts the writer and the task that keeps the permissions current and the window short
    #[must_use]
    pub fn start(pool: PgPool, cipher: Cipher) -> Self {
        let readers = Readers::default();
        let (changes, incoming) = mpsc::unbounded_channel();
        tokio::spawn(write(pool.clone(), cipher, readers.clone(), incoming));
        tokio::spawn(keep(pool, readers.clone()));
        Self { readers, changes }
    }

    #[must_use]
    pub const fn readers(&self) -> &Readers {
        &self.readers
    }

    /// Keeps a text this device sent, if some connection may read its chat
    pub(super) fn sent(
        &self,
        account_id: Uuid,
        chat: &Jid,
        message_id: &str,
        sender_jid: String,
        text: String,
        reply_to: Option<String>,
    ) {
        let Some(chat) = chat_of_jid(chat) else {
            return;
        };
        let change = Change::Arrived {
            chat,
            message_id: message_id.to_owned(),
            from_me: true,
            sent_at: Utc::now(),
            expires_at: None,
            body: Body {
                sender_jid,
                sender_number: None,
                sender_name: None,
                kind: Kind::Text,
                text: Some(text),
                file_name: None,
                reply_to,
                edited: false,
            },
        };
        // Fails only once the writer has gone, at shutdown
        let _ = self.changes.send((account_id, change));
    }

    pub(super) fn feed(&self, account_id: Uuid) -> Feed {
        Feed {
            account_id,
            readers: self.readers.clone(),
            changes: self.changes.clone(),
        }
    }
}

/// Hands over the messages of readable chats as they arrive. Like the contacts feed it runs inline
/// on the dispatch path, where no event is dropped under load.
pub(super) struct Feed {
    account_id: Uuid,
    readers: Readers,
    changes: Changes,
}

impl Feed {
    fn readable(&self, chat: &Chat) -> bool {
        match chat {
            Chat::Group(id) => self
                .readers
                .reads(self.account_id, RecipientType::Group, id),
            Chat::Number(number) => {
                self.readers
                    .reads(self.account_id, RecipientType::Contact, number)
            }
            // The writer looks the number up, then asks again
            Chat::Lid(_) => self.readers.reads_anything(self.account_id),
        }
    }

    fn captured(&self, message: &InboundMessage) -> Option<Change> {
        let info = &message.info;
        let chat = chat_of(&info.source)?;
        if !self.readable(&chat) {
            return None;
        }
        Some(match content_of(&message.message) {
            Content::Message {
                kind,
                text,
                file_name,
                reply_to,
            } => {
                let source = &info.source;
                let number = |jid: &Jid| jid.server.is_pn_family().then(|| jid.user.to_string());
                let sender_number = number(&source.sender)
                    .or_else(|| source.sender_alt.as_ref().and_then(number))
                    .filter(|_| !source.is_from_me);
                let name = info.push_name.trim();
                Change::Arrived {
                    chat,
                    message_id: info.id.to_string(),
                    from_me: source.is_from_me,
                    sent_at: info.timestamp,
                    expires_at: message
                        .ephemeral_expiration
                        .filter(|seconds| *seconds > 0)
                        .map(|seconds| info.timestamp + TimeDelta::seconds(i64::from(seconds))),
                    body: Body {
                        sender_jid: source.sender.to_non_ad().to_string(),
                        sender_number,
                        sender_name: (!name.is_empty() && !source.is_from_me)
                            .then(|| name.to_owned()),
                        kind,
                        text,
                        file_name,
                        reply_to,
                        edited: false,
                    },
                }
            }
            Content::Edit { message_id, text } => Change::Edited {
                chat,
                message_id,
                text,
            },
            Content::Revoke { message_id } => Change::Removed { chat, message_id },
            Content::Nothing => return None,
        })
    }

    fn removed(&self, jid: &Jid, message_id: Option<&str>) -> Option<Change> {
        let chat = chat_of_jid(jid)?;
        if !self.readers.reads_anything(self.account_id) {
            return None;
        }
        Some(match message_id {
            Some(message_id) => Change::Removed {
                chat,
                message_id: message_id.to_owned(),
            },
            None => Change::ChatRemoved { chat },
        })
    }
}

impl EventHandler for Feed {
    fn handle_event(&self, event: Arc<LibraryEvent>) {
        // A send fails only once the writer has gone, at shutdown
        match &*event {
            LibraryEvent::Messages(batch) => {
                for change in batch.iter().filter_map(|message| self.captured(message)) {
                    let _ = self.changes.send((self.account_id, change));
                }
            }
            LibraryEvent::DeleteChatUpdate(update) => {
                if let Some(change) = self.removed(&update.jid, None) {
                    let _ = self.changes.send((self.account_id, change));
                }
            }
            LibraryEvent::ClearChatUpdate(update) => {
                if let Some(change) = self.removed(&update.jid, None) {
                    let _ = self.changes.send((self.account_id, change));
                }
            }
            LibraryEvent::DeleteMessageForMeUpdate(update) => {
                if let Some(change) = self.removed(&update.chat_jid, Some(&update.message_id)) {
                    let _ = self.changes.send((self.account_id, change));
                }
            }
            _ => {}
        }
    }

    fn interest(&self) -> EventInterest {
        EventInterest::of(&[
            EventKind::Messages,
            EventKind::DeleteChatUpdate,
            EventKind::ClearChatUpdate,
            EventKind::DeleteMessageForMeUpdate,
        ])
    }
}

fn chat_of_jid(jid: &Jid) -> Option<Chat> {
    if jid.server == Server::Group {
        Some(Chat::Group(jid.to_non_ad().to_string()))
    } else if jid.server.is_pn_family() {
        Some(Chat::Number(jid.user.to_string()))
    } else if jid.server.is_lid_family() {
        Some(Chat::Lid(jid.user.to_string()))
    } else {
        // Statuses, channels and broadcast lists aren't chats
        None
    }
}

fn chat_of(source: &MessageSource) -> Option<Chat> {
    match chat_of_jid(&source.chat)? {
        Chat::Lid(lid) => {
            // The other person's number, when the message carried it beside their LID
            let other = if source.is_from_me {
                source.recipient_alt.as_ref()
            } else {
                source.sender_alt.as_ref()
            };
            Some(
                other
                    .filter(|jid| jid.server.is_pn_family())
                    .map_or(Chat::Lid(lid), |jid| Chat::Number(jid.user.to_string())),
            )
        }
        chat => Some(chat),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Content {
    Message {
        kind: Kind,
        text: Option<String>,
        file_name: Option<String>,
        reply_to: Option<String>,
    },
    Edit {
        message_id: String,
        text: Option<String>,
    },
    Revoke {
        message_id: String,
    },
    /// View-once messages, reactions, and what WhatsApp sends for its own bookkeeping
    Nothing,
}

fn content_of(message: &wa::Message) -> Content {
    use wa::message::protocol_message::Type;

    if message.is_view_once() {
        return Content::Nothing;
    }
    let base = message.get_base_message();
    let filled = |text: Option<&str>| {
        text.map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    if let Some(protocol) = base.protocol_message.as_option() {
        let target = protocol
            .key
            .as_option()
            .and_then(|key| key.id.clone())
            .filter(|id| !id.is_empty());
        return match target {
            Some(message_id) if protocol.r#type == Some(Type::REVOKE) => {
                Content::Revoke { message_id }
            }
            Some(message_id) if protocol.r#type == Some(Type::MESSAGE_EDIT) => Content::Edit {
                message_id,
                text: protocol.edited_message.as_option().and_then(|edited| {
                    filled(edited.text_content().or_else(|| edited.get_caption()))
                }),
            },
            _ => Content::Nothing,
        };
    }

    let text = filled(base.text_content().or_else(|| base.get_caption()));
    let mut file_name = None;
    let kind = if base.image_message.as_option().is_some() {
        Kind::Image
    } else if base.video_message.as_option().is_some() {
        Kind::Video
    } else if base.audio_message.as_option().is_some() {
        Kind::Audio
    } else if let Some(document) = base.document_message.as_option() {
        file_name = filled(document.file_name.as_deref());
        Kind::Document
    } else if base.sticker_message.as_option().is_some() {
        Kind::Sticker
    } else if base.location_message.as_option().is_some()
        || base.live_location_message.as_option().is_some()
    {
        Kind::Location
    } else if base.contact_message.as_option().is_some()
        || base.contacts_array_message.as_option().is_some()
    {
        Kind::Contact
    } else if base.poll_creation_message.as_option().is_some()
        || base.poll_creation_message_v3.as_option().is_some()
    {
        Kind::Poll
    } else if text.is_some() {
        Kind::Text
    } else {
        return Content::Nothing;
    };
    let reply_to = base
        .extended_text_message
        .as_option()
        .and_then(|text| text.context_info.as_option())
        .and_then(|context| context.stanza_id.clone())
        .filter(|id| !id.is_empty());
    Content::Message {
        kind,
        text,
        file_name,
        reply_to,
    }
}

async fn write(
    pool: PgPool,
    cipher: Cipher,
    readers: Readers,
    mut incoming: mpsc::UnboundedReceiver<(Uuid, Change)>,
) {
    while let Some((account_id, change)) = incoming.recv().await {
        if let Err(error) = apply(&pool, &cipher, &readers, account_id, change).await {
            tracing::warn!(%account_id, "couldn't keep a WhatsApp message: {error:#}");
        }
    }
}

/// The chat as a grant names it, if some connection may read it
async fn readable(
    pool: &PgPool,
    readers: &Readers,
    account_id: Uuid,
    chat: Chat,
) -> sqlx::Result<Option<(RecipientType, String)>> {
    let (recipient_type, id) = match chat {
        Chat::Group(id) => (RecipientType::Group, id),
        Chat::Number(number) => (RecipientType::Contact, number),
        Chat::Lid(lid) => match whatsapp_contacts::number_of_lid(pool, account_id, &lid).await? {
            Some(number) => (RecipientType::Contact, number),
            None => return Ok(None),
        },
    };
    Ok(readers
        .reads(account_id, recipient_type, &id)
        .then_some((recipient_type, id)))
}

pub(super) async fn apply(
    pool: &PgPool,
    cipher: &Cipher,
    readers: &Readers,
    account_id: Uuid,
    change: Change,
) -> anyhow::Result<()> {
    match change {
        Change::Arrived {
            chat,
            message_id,
            from_me,
            sent_at,
            expires_at,
            mut body,
        } => {
            let Some((recipient_type, id)) = readable(pool, readers, account_id, chat).await?
            else {
                return Ok(());
            };
            if body.sender_number.is_none() && !from_me {
                body.sender_number = match recipient_type {
                    // In a chat with one person, whoever isn't the owner is that person
                    RecipientType::Contact => Some(id.clone()),
                    RecipientType::Group => match body.sender_jid.strip_suffix("@lid") {
                        Some(lid) => {
                            whatsapp_contacts::number_of_lid(pool, account_id, lid).await?
                        }
                        None => None,
                    },
                };
            }
            let sealed = seal_body(
                cipher,
                account_id,
                (recipient_type, &id),
                &message_id,
                &body,
            )?;
            inbox_messages::save(
                pool,
                inbox_messages::NewMessage {
                    account_id,
                    chat_type: recipient_type.as_str(),
                    chat_id: &id,
                    message_id: &message_id,
                    from_me,
                    sent_at,
                    expires_at,
                    body: &sealed,
                },
            )
            .await?;
        }
        Change::Edited {
            chat,
            message_id,
            text,
        } => {
            let Some((recipient_type, id)) = readable(pool, readers, account_id, chat).await?
            else {
                return Ok(());
            };
            let kind = recipient_type.as_str();
            let Some(kept) =
                inbox_messages::find_in_account(pool, account_id, kind, &id, &message_id).await?
            else {
                return Ok(());
            };
            let mut body = open_body(cipher, &kept)?;
            body.text = text;
            body.edited = true;
            let sealed = seal_body(
                cipher,
                account_id,
                (recipient_type, &id),
                &message_id,
                &body,
            )?;
            inbox_messages::replace_body(pool, account_id, kind, &id, &message_id, &sealed).await?;
        }
        // Removals don't ask who may read: what was kept goes either way
        Change::Removed { chat, message_id } => {
            if let Some((recipient_type, id)) = named(pool, account_id, chat).await? {
                inbox_messages::remove(pool, account_id, recipient_type.as_str(), &id, &message_id)
                    .await?;
            }
        }
        Change::ChatRemoved { chat } => {
            if let Some((recipient_type, id)) = named(pool, account_id, chat).await? {
                inbox_messages::remove_chat(pool, account_id, recipient_type.as_str(), &id).await?;
            }
        }
    }
    Ok(())
}

async fn named(
    pool: &PgPool,
    account_id: Uuid,
    chat: Chat,
) -> sqlx::Result<Option<(RecipientType, String)>> {
    Ok(match chat {
        Chat::Group(id) => Some((RecipientType::Group, id)),
        Chat::Number(number) => Some((RecipientType::Contact, number)),
        Chat::Lid(lid) => whatsapp_contacts::number_of_lid(pool, account_id, &lid)
            .await?
            .map(|number| (RecipientType::Contact, number)),
    })
}

async fn keep(pool: PgPool, readers: Readers) {
    let mut interval = tokio::time::interval(RELOAD_EVERY);
    let mut reloads = 0_u32;
    loop {
        interval.tick().await;
        if let Err(error) = readers.reload(&pool).await {
            // Sweeping against a stale list could drop a chat that may be read again
            tracing::warn!(?error, "couldn't read which chats agents may read");
            continue;
        }
        if reloads.is_multiple_of(SWEEP_EVERY)
            && let Err(error) = sweep(&pool, &readers).await
        {
            tracing::warn!(?error, "couldn't sweep kept WhatsApp messages");
        }
        reloads = reloads.wrapping_add(1);
    }
}

/// Drops what has left the window, and every chat no connection may read any more
pub(super) async fn sweep(pool: &PgPool, readers: &Readers) -> sqlx::Result<()> {
    inbox_messages::sweep(pool, WINDOW_DAYS).await?;
    for (account_id, kind, id) in inbox_messages::kept_chats(pool).await? {
        let unread = chat_type(&kind)
            .is_none_or(|recipient_type| !readers.reads(account_id, recipient_type, &id));
        if unread {
            inbox_messages::remove_chat(pool, account_id, &kind, &id).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
