use std::io::Write;

use chrono::{DateTime, TimeDelta, Utc};
use flate2::{Compression, write::ZlibEncoder};
use sqlx::PgPool;
use uuid::Uuid;
use whatsapp_rust::{
    HistorySyncAdmission, HistorySyncDecision, HistorySyncMetadata, prelude::wa, waproto,
};

use super::{Admission, context, import_chunk, parse};
use crate::{
    crypto::Cipher,
    db::{history_chunks, inbox_messages, message_history, whatsapp_accounts},
    whatsapp::inbox::{Chat, History, Kind, Readers, open_body, terms},
};

const ADA: &str = "8801700000001";
const BOB: &str = "8801700000002";
const GROUP: &str = "120363000000000000@g.us";
const OWN: &str = "8801700000009";

fn text(body: &str) -> wa::Message {
    let mut message = wa::Message::default();
    message.conversation = Some(body.to_owned());
    message
}

fn protocol(
    kind: wa::message::protocol_message::Type,
    target: &str,
    edited: Option<&str>,
) -> wa::Message {
    let mut key = wa::MessageKey::default();
    key.id = Some(target.to_owned());
    let mut protocol = wa::message::ProtocolMessage::default();
    protocol.key = key.into();
    protocol.r#type = Some(kind);
    if let Some(edited) = edited {
        protocol.edited_message = text(edited).into();
    }
    let mut message = wa::Message::default();
    message.protocol_message = protocol.into();
    message
}

fn entry(
    id: &str,
    from_me: bool,
    participant: Option<&str>,
    sent_at: DateTime<Utc>,
    message: Option<wa::Message>,
) -> wa::HistorySyncMsg {
    let mut key = wa::MessageKey::default();
    key.id = Some(id.to_owned());
    key.from_me = Some(from_me);
    key.participant = participant.map(str::to_owned);
    let mut info = wa::WebMessageInfo::default();
    info.key = key.into();
    info.message_timestamp = u64::try_from(sent_at.timestamp()).ok();
    info.push_name = (!from_me).then(|| "Ada L".to_owned());
    if let Some(message) = message {
        info.message = message.into();
    }
    let mut entry = wa::HistorySyncMsg::default();
    entry.message = info.into();
    entry
}

fn conversation(id: &str, pn: Option<&str>, messages: Vec<wa::HistorySyncMsg>) -> wa::Conversation {
    let mut conversation = wa::Conversation::default();
    conversation.id = id.to_owned();
    conversation.pn_jid = pn.map(str::to_owned);
    conversation.messages = messages;
    conversation
}

fn compressed(conversations: Vec<wa::Conversation>, progress: u32) -> Vec<u8> {
    let mut history = wa::HistorySync::default();
    history.conversations = conversations;
    history.progress = Some(progress);
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(&waproto::codec::history_sync_to_vec(&history))
        .unwrap();
    encoder.finish().unwrap()
}

/// Ada's chat, a chat WhatsApp names by LID, a group and a status update
fn chunk(now: DateTime<Utc>) -> Vec<u8> {
    use wa::message::protocol_message::Type;

    let ago = |days| now - TimeDelta::days(days);
    compressed(
        vec![
            conversation(
                &format!("{ADA}@s.whatsapp.net"),
                None,
                vec![
                    entry("A1", false, None, ago(3), Some(text("Hello Habib"))),
                    entry("A2", true, None, ago(3), Some(text("Hi Ada"))),
                    entry("A3", false, None, ago(2), Some(text("Wrong chat, sorry"))),
                    entry(
                        "A4",
                        false,
                        None,
                        ago(2),
                        Some(protocol(Type::REVOKE, "A3", None)),
                    ),
                    entry(
                        "A5",
                        true,
                        None,
                        ago(1),
                        Some(protocol(
                            Type::MESSAGE_EDIT,
                            "A2",
                            Some("Hi Ada, how are you?"),
                        )),
                    ),
                    // Older than the owner keeps
                    entry("A6", false, None, ago(400), Some(text("Long ago"))),
                    // A system notice carries no message
                    entry("A7", false, None, ago(1), None),
                ],
            ),
            conversation(
                "111@lid",
                Some(&format!("{BOB}@s.whatsapp.net")),
                vec![entry(
                    "B1",
                    false,
                    None,
                    ago(5),
                    Some(text("Gold price crash")),
                )],
            ),
            conversation(
                GROUP,
                None,
                vec![entry(
                    "G1",
                    false,
                    Some("222@lid"),
                    ago(4),
                    Some(text("Meeting at 5")),
                )],
            ),
            conversation(
                "status@broadcast",
                None,
                vec![entry(
                    "S1",
                    false,
                    Some(&format!("{ADA}@s.whatsapp.net")),
                    ago(1),
                    Some(text("My status")),
                )],
            ),
        ],
        40,
    )
}

#[test]
fn reads_each_chats_messages_with_their_edits_and_deletions() {
    let now = Utc::now();
    let since = Some(now - TimeDelta::days(365));
    let parsed = parse(&chunk(now), since, &format!("{OWN}@s.whatsapp.net")).unwrap();

    assert_eq!(parsed.progress, Some(40));
    let ids: Vec<_> = parsed
        .messages
        .iter()
        .map(|message| message.message_id.as_str())
        .collect();
    assert_eq!(ids, ["A1", "A2", "B1", "G1"]);

    let [hello, hi, gold, meeting] = &parsed.messages[..] else {
        unreachable!();
    };
    assert_eq!(hello.chat, Chat::Number(ADA.to_owned()));
    assert!(!hello.from_me);
    assert_eq!(hello.body.sender_number.as_deref(), Some(ADA));
    assert_eq!(hello.body.sender_name.as_deref(), Some("Ada L"));
    assert_eq!(hello.body.kind, Kind::Text);

    assert!(hi.from_me);
    assert_eq!(hi.body.sender_jid, format!("{OWN}@s.whatsapp.net"));
    assert_eq!(hi.body.text.as_deref(), Some("Hi Ada, how are you?"));
    assert!(hi.body.edited);
    assert_eq!(hi.body.sender_name, None);

    // WhatsApp named the chat by LID and gave the number beside it
    assert_eq!(gold.chat, Chat::Number(BOB.to_owned()));
    assert_eq!(meeting.chat, Chat::Group(GROUP.to_owned()));
    assert_eq!(meeting.body.sender_jid, "222@lid");
    assert_eq!(meeting.body.sender_number, None);
}

#[test]
fn keeping_everything_reads_every_age() {
    let now = Utc::now();
    let parsed = parse(&chunk(now), None, "").unwrap();
    assert!(
        parsed
            .messages
            .iter()
            .any(|message| message.message_id == "A6")
    );
}

#[test]
fn a_chunk_that_isnt_history_fails_to_parse() {
    assert!(parse(b"not zlib at all", None, "").is_err());
}

#[test]
fn history_is_admitted_only_for_an_owner_who_keeps_it() {
    let readers = Readers::default();
    let account_id = Uuid::new_v4();
    let admission = Admission {
        account_id,
        readers: readers.clone(),
    };
    let metadata = HistorySyncMetadata::default();

    assert_eq!(
        admission.decide(&metadata),
        HistorySyncDecision::RejectAndAcknowledge
    );
    readers.set_history(account_id, History::Days(30));
    assert_eq!(admission.decide(&metadata), HistorySyncDecision::Accept);
    readers.set_history(account_id, History::Off);
    assert_eq!(
        admission.decide(&metadata),
        HistorySyncDecision::RejectAndAcknowledge
    );
}

async fn account(pool: &PgPool) -> (Uuid, Uuid) {
    let user_id = sqlx::query_scalar!(
        "INSERT INTO users (email, password) VALUES ($1, 'unused') RETURNING id",
        format!("{}@example.com", Uuid::new_v4()),
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let account = whatsapp_accounts::find_or_create(pool, user_id)
        .await
        .unwrap();
    sqlx::query!(
        "UPDATE whatsapp_accounts SET phone_number = $1 WHERE id = $2",
        OWN,
        account.id
    )
    .execute(pool)
    .await
    .unwrap();
    (user_id, account.id)
}

async fn hold(pool: &PgPool, cipher: &Cipher, account_id: Uuid, message_id: &str) {
    let sealed = cipher
        .encrypt(&chunk(Utc::now()), &context(account_id, message_id))
        .unwrap();
    history_chunks::save(pool, account_id, message_id, &sealed)
        .await
        .unwrap();
}

#[sqlx::test]
async fn imports_a_held_chunk_once_and_counts_it(pool: PgPool) {
    let cipher = Cipher::new(&[0; 32]);
    let (user_id, account_id) = account(&pool).await;
    // The group sender's number, as WhatsApp told the library
    sqlx::query!(
        "INSERT INTO whatsapp_lid_mappings
             (account_id, lid, phone_number, learning_source, created_at, updated_at)
         VALUES ($1, '222', $2, 'test', 0, 0)",
        account_id,
        BOB,
    )
    .execute(&pool)
    .await
    .unwrap();
    message_history::save(&pool, user_id, Some(30))
        .await
        .unwrap();
    let readers = Readers::default();
    readers.reload(&pool).await.unwrap();

    hold(&pool, &cipher, account_id, "H1").await;
    let chunk = history_chunks::oldest(&pool).await.unwrap().unwrap();
    import_chunk(&pool, &cipher, &readers, chunk).await.unwrap();

    assert!(history_chunks::oldest(&pool).await.unwrap().is_none());
    let import = whatsapp_accounts::history_import(&pool, user_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(import.progress, Some(40));
    // A6 is older than the 30 days kept
    assert_eq!(import.imported, 4);

    let scope = inbox_messages::Scope {
        user_id,
        chat: Some(("group", GROUP)),
        since: None,
    };
    let group = inbox_messages::page(&pool, scope, None, 10).await.unwrap();
    let body = open_body(&cipher, &group[0]).unwrap();
    assert_eq!(body.sender_number.as_deref(), Some(BOB));

    // Its messages can be searched
    let everywhere = inbox_messages::Scope {
        user_id,
        chat: None,
        since: None,
    };
    let query = terms::of_query(&cipher, account_id, "gold").unwrap();
    let found = inbox_messages::matching(&pool, everywhere, &query, None, 10)
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].chat_id, BOB);

    // The phone sending the same chunk again adds nothing
    hold(&pool, &cipher, account_id, "H2").await;
    let again = history_chunks::oldest(&pool).await.unwrap().unwrap();
    import_chunk(&pool, &cipher, &readers, again).await.unwrap();
    let import = whatsapp_accounts::history_import(&pool, user_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(import.imported, 4);
}

#[sqlx::test]
async fn a_chunk_held_after_history_was_turned_off_is_dropped_unread(pool: PgPool) {
    let cipher = Cipher::new(&[0; 32]);
    let (user_id, account_id) = account(&pool).await;
    let readers = Readers::default();

    hold(&pool, &cipher, account_id, "H1").await;
    let chunk = history_chunks::oldest(&pool).await.unwrap().unwrap();
    import_chunk(&pool, &cipher, &readers, chunk).await.unwrap();

    assert!(history_chunks::oldest(&pool).await.unwrap().is_none());
    let everywhere = inbox_messages::Scope {
        user_id,
        chat: None,
        since: None,
    };
    assert!(
        inbox_messages::page(&pool, everywhere, None, 10)
            .await
            .unwrap()
            .is_empty()
    );
}
