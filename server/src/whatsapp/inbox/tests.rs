use std::sync::Arc;

use chrono::{TimeDelta, Utc};
use sqlx::PgPool;
use tokio::sync::mpsc;
use uuid::Uuid;
use whatsapp_rust::{
    buffa::MessageField,
    prelude::{Jid, wa},
    wacore::{
        proto_helpers::MessageBuilderExt,
        types::{events::InboundMessage, message::MessageInfo},
    },
};

use super::{
    Body, Change, Chat, Content, Feed, Kind, Readers, WINDOW_DAYS, apply, content_of, open_body,
    sweep,
};
use crate::{
    agents::{
        Grant, Preset,
        grant::{self, ChatRule, Scope, SendLevel},
        key::AccessKey,
    },
    crypto::Cipher,
    db::{
        agent_connections::{self, Credential, NewConnection},
        inbox_messages, whatsapp_accounts, whatsapp_contacts,
    },
    scheduler::RecipientType,
    whatsapp::contacts::{Names, seal_names},
};

const ADA: &str = "8801700000001";
const BOB: &str = "8801700000002";
const GROUP: &str = "120363000000000000@g.us";

/// A grant that reads only the chats listed
fn reading(chats: &[&str]) -> Grant {
    let mut grant = Preset::AskBeforeSending.grant();
    grant.chats.scope = Scope::Selected;
    for id in chats {
        grant.chats.rules.push(ChatRule {
            chat: grant::Chat {
                recipient_type: if id.ends_with("@g.us") {
                    RecipientType::Group
                } else {
                    RecipientType::Contact
                },
                id: (*id).to_owned(),
                name: None,
            },
            read: true,
            send: SendLevel::Ask,
            daily_limit: None,
            note: None,
        });
    }
    grant
}

fn readers(account_id: Uuid, grants: Vec<Grant>) -> Readers {
    let readers = Readers::default();
    readers.0.write().unwrap().insert(account_id, grants);
    readers
}

fn feed(account_id: Uuid, readers: Readers) -> Feed {
    Feed {
        account_id,
        readers,
        changes: mpsc::unbounded_channel().0,
    }
}

fn inbound(info: MessageInfo, message: wa::Message) -> InboundMessage {
    InboundMessage::builder()
        .message(Arc::new(message))
        .info(Arc::new(info))
        .build()
}

fn direct(from: &str, id: &str, message: wa::Message) -> InboundMessage {
    let mut info = MessageInfo::default();
    info.source.chat = Jid::pn(from);
    info.source.sender = Jid::pn(from);
    info.id = id.into();
    info.push_name = "Ada L.".into();
    info.timestamp = Utc::now();
    inbound(info, message)
}

fn protocol(
    kind: wa::message::protocol_message::Type,
    target: &str,
    edited: Option<wa::Message>,
) -> wa::Message {
    let mut key = wa::MessageKey::default();
    key.id = Some(target.to_owned());
    let mut protocol = wa::message::ProtocolMessage::default();
    protocol.r#type = Some(kind);
    protocol.key = MessageField::some(key);
    if let Some(edited) = edited {
        protocol.edited_message = MessageField::some(edited);
    }
    let mut message = wa::Message::default();
    message.protocol_message = MessageField::some(protocol);
    message
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
    (user_id, account.id)
}

fn arrived(chat: Chat, id: &str, text: &str) -> Change {
    Change::Arrived {
        chat,
        message_id: id.to_owned(),
        from_me: false,
        sent_at: Utc::now(),
        expires_at: None,
        body: Body {
            sender_jid: format!("{ADA}@s.whatsapp.net"),
            sender_number: Some(ADA.to_owned()),
            sender_name: None,
            kind: Kind::Text,
            text: Some(text.to_owned()),
            file_name: None,
            reply_to: None,
            edited: false,
        },
    }
}

async fn kept(pool: &PgPool, user_id: Uuid, number: &str) -> Vec<inbox_messages::Kept> {
    inbox_messages::page(pool, user_id, "contact", number, None, 50)
        .await
        .unwrap()
}

#[test]
fn only_a_readable_chat_is_captured() {
    let account_id = Uuid::new_v4();
    let feed = feed(account_id, readers(account_id, vec![reading(&[ADA])]));

    let captured = feed.captured(&direct(ADA, "A1", wa::Message::text("Hello")));
    let Some(Change::Arrived {
        chat,
        message_id,
        from_me,
        body,
        ..
    }) = captured
    else {
        panic!("{captured:?}");
    };
    assert_eq!(chat, Chat::Number(ADA.to_owned()));
    assert_eq!(message_id, "A1");
    assert!(!from_me);
    assert_eq!(body.text.as_deref(), Some("Hello"));
    assert_eq!(body.sender_number.as_deref(), Some(ADA));
    assert_eq!(body.sender_name.as_deref(), Some("Ada L."));
    assert_eq!(body.sender_jid, format!("{ADA}@s.whatsapp.net"));

    assert_eq!(
        feed.captured(&direct(BOB, "B1", wa::Message::text("Hi"))),
        None
    );
    // Another account's readers don't count
    let other = self::feed(Uuid::new_v4(), feed.readers);
    assert_eq!(
        other.captured(&direct(ADA, "A2", wa::Message::text("Hi"))),
        None
    );
}

#[test]
fn a_group_and_a_chat_addressed_by_lid_are_named_as_a_grant_names_them() {
    let account_id = Uuid::new_v4();
    let feed = feed(
        account_id,
        readers(account_id, vec![reading(&[ADA, GROUP])]),
    );

    let mut info = MessageInfo::default();
    info.source.chat = GROUP.parse().unwrap();
    info.source.sender = Jid::lid("99887766");
    info.source.sender_alt = Some(Jid::pn(BOB));
    info.source.is_group = true;
    info.id = "G1".into();
    let in_group = inbound(info, wa::Message::text("Meeting at 5"));
    let Some(Change::Arrived { chat, body, .. }) = feed.captured(&in_group) else {
        panic!("not captured");
    };
    assert_eq!(chat, Chat::Group(GROUP.to_owned()));
    assert_eq!(body.sender_number.as_deref(), Some(BOB));
    assert_eq!(body.sender_jid, "99887766@lid");

    // The owner's own message from the phone, in a chat WhatsApp addresses by LID
    let mut info = MessageInfo::default();
    info.source.chat = Jid::lid("11223344");
    info.source.sender = Jid::lid("55555555");
    info.source.is_from_me = true;
    info.source.recipient_alt = Some(Jid::pn(ADA));
    info.id = "M1".into();
    info.push_name = "Me".into();
    let mine = inbound(info.clone(), wa::Message::text("On my way"));
    let Some(Change::Arrived {
        chat,
        from_me,
        body,
        ..
    }) = feed.captured(&mine)
    else {
        panic!("not captured");
    };
    assert_eq!(chat, Chat::Number(ADA.to_owned()));
    assert!(from_me);
    assert_eq!((body.sender_number, body.sender_name), (None, None));

    // Without the number, the writer looks it up
    info.source.recipient_alt = None;
    let unnamed = inbound(info, wa::Message::text("On my way"));
    assert!(matches!(
        feed.captured(&unnamed),
        Some(Change::Arrived { chat: Chat::Lid(lid), .. }) if lid == "11223344"
    ));
}

#[test]
fn statuses_and_channels_are_not_chats() {
    let account_id = Uuid::new_v4();
    let mut everything = Preset::AskBeforeSending.grant();
    everything.chats.default.read = true;
    let feed = feed(account_id, readers(account_id, vec![everything]));
    for chat in ["status@broadcast", "120363111111111111@newsletter"] {
        let mut info = MessageInfo::default();
        info.source.chat = chat.parse().unwrap();
        info.source.sender = Jid::pn(ADA);
        info.id = "S1".into();
        let message = inbound(info, wa::Message::text("Hello"));
        assert_eq!(feed.captured(&message), None, "{chat}");
    }
    // Under "all chats" with reading on, any person's chat is captured
    assert!(
        feed.captured(&direct(BOB, "B1", wa::Message::text("Hi")))
            .is_some()
    );
}

#[test]
fn reads_what_a_message_holds() {
    use wa::message::protocol_message::Type;

    let text = |text: &str, reply_to: Option<&str>| Content::Message {
        kind: Kind::Text,
        text: Some(text.to_owned()),
        file_name: None,
        reply_to: reply_to.map(str::to_owned),
    };
    assert_eq!(
        content_of(&wa::Message::text("  Hello  ")),
        text("Hello", None)
    );

    let mut context = wa::ContextInfo::default();
    context.stanza_id = Some("Q1".to_owned());
    assert_eq!(
        content_of(&wa::Message::text_with_context("Yes", context)),
        text("Yes", Some("Q1"))
    );

    let mut image = wa::message::ImageMessage::default();
    image.caption = Some("The receipt".to_owned());
    let mut picture = wa::Message::default();
    picture.image_message = MessageField::some(image.clone());
    assert_eq!(
        content_of(&picture),
        Content::Message {
            kind: Kind::Image,
            text: Some("The receipt".to_owned()),
            file_name: None,
            reply_to: None,
        }
    );

    let mut document = wa::message::DocumentMessage::default();
    document.file_name = Some("invoice.pdf".to_owned());
    let mut file = wa::Message::default();
    file.document_message = MessageField::some(document);
    assert!(matches!(
        content_of(&file),
        Content::Message { kind: Kind::Document, file_name: Some(name), .. } if name == "invoice.pdf"
    ));

    // View-once pictures are never kept
    image.view_once = Some(true);
    let mut once = wa::Message::default();
    once.image_message = MessageField::some(image);
    assert_eq!(content_of(&once), Content::Nothing);

    let mut reaction = wa::Message::default();
    reaction.reaction_message = MessageField::some(wa::message::ReactionMessage::default());
    assert_eq!(content_of(&reaction), Content::Nothing);
    assert_eq!(content_of(&wa::Message::default()), Content::Nothing);

    assert_eq!(
        content_of(&protocol(Type::REVOKE, "A1", None)),
        Content::Revoke {
            message_id: "A1".to_owned()
        }
    );
    let edit = protocol(
        Type::MESSAGE_EDIT,
        "A1",
        Some(wa::Message::text("Hello there")),
    );
    assert_eq!(
        content_of(&edit),
        Content::Edit {
            message_id: "A1".to_owned(),
            text: Some("Hello there".to_owned()),
        }
    );
}

#[test]
fn a_disappearing_message_carries_its_end() {
    let account_id = Uuid::new_v4();
    let feed = feed(account_id, readers(account_id, vec![reading(&[ADA])]));
    let mut message = direct(ADA, "A1", wa::Message::text("Hello"));
    message.ephemeral_expiration = Some(3600);
    let Some(Change::Arrived {
        sent_at,
        expires_at,
        ..
    }) = feed.captured(&message)
    else {
        panic!("not captured");
    };
    assert_eq!(expires_at, Some(sent_at + TimeDelta::hours(1)));
}

#[sqlx::test]
async fn keeps_edits_and_removes_what_arrives(pool: PgPool) {
    let cipher = Cipher::new(&[0; 32]);
    let (user_id, account_id) = account(&pool).await;
    let readers = readers(account_id, vec![reading(&[ADA])]);
    let ada = || Chat::Number(ADA.to_owned());
    let run = async |change| {
        apply(&pool, &cipher, &readers, account_id, change)
            .await
            .unwrap();
    };

    run(arrived(ada(), "A1", "Hello")).await;
    run(arrived(ada(), "A2", "Are you there?")).await;
    // Delivered twice, kept once; another chat's message isn't written at all
    run(arrived(ada(), "A1", "Hello")).await;
    run(arrived(Chat::Number(BOB.to_owned()), "B1", "Hi")).await;
    assert_eq!(kept(&pool, user_id, ADA).await.len(), 2);
    assert!(kept(&pool, user_id, BOB).await.is_empty());

    run(Change::Edited {
        chat: ada(),
        message_id: "A1".to_owned(),
        text: Some("Hello Habib".to_owned()),
    })
    .await;
    let first = inbox_messages::find(&pool, user_id, "contact", ADA, "A1")
        .await
        .unwrap()
        .unwrap();
    let body = open_body(&cipher, &first).unwrap();
    assert_eq!(body.text.as_deref(), Some("Hello Habib"));
    assert!(body.edited);
    // The text is sealed: the stored bytes don't hold it
    assert!(!first.body.windows(5).any(|window| window == b"Hello"));

    run(Change::Removed {
        chat: ada(),
        message_id: "A2".to_owned(),
    })
    .await;
    assert_eq!(kept(&pool, user_id, ADA).await.len(), 1);

    run(Change::ChatRemoved { chat: ada() }).await;
    assert!(kept(&pool, user_id, ADA).await.is_empty());
}

#[sqlx::test]
async fn a_chat_known_only_by_its_lid_is_found_in_the_address_book(pool: PgPool) {
    let cipher = Cipher::new(&[0; 32]);
    let (user_id, account_id) = account(&pool).await;
    let readers = readers(account_id, vec![reading(&[ADA])]);
    let lid = || Chat::Lid("11223344".to_owned());

    apply(
        &pool,
        &cipher,
        &readers,
        account_id,
        arrived(lid(), "A1", "Hello"),
    )
    .await
    .unwrap();
    assert!(kept(&pool, user_id, ADA).await.is_empty());

    let jid = "11223344@lid";
    let names = Names {
        full_name: Some("Ada".to_owned()),
        first_name: None,
        username: None,
    };
    let sealed = seal_names(&cipher, account_id, jid, &names).unwrap();
    whatsapp_contacts::save(&pool, account_id, jid, Some(ADA), Some("11223344"), &sealed)
        .await
        .unwrap();
    apply(
        &pool,
        &cipher,
        &readers,
        account_id,
        arrived(lid(), "A1", "Hello"),
    )
    .await
    .unwrap();
    assert_eq!(kept(&pool, user_id, ADA).await.len(), 1);
}

#[sqlx::test]
async fn the_sweep_drops_old_messages_and_chats_nobody_reads(pool: PgPool) {
    let cipher = Cipher::new(&[0; 32]);
    let (user_id, account_id) = account(&pool).await;
    let both = readers(account_id, vec![reading(&[ADA, BOB])]);

    for (number, id) in [(ADA, "A1"), (ADA, "A2"), (ADA, "A3"), (BOB, "B1")] {
        let chat = Chat::Number(number.to_owned());
        apply(
            &pool,
            &cipher,
            &both,
            account_id,
            arrived(chat, id, "Hello"),
        )
        .await
        .unwrap();
    }
    sqlx::query!(
        "UPDATE inbox_messages SET sent_at = now() - make_interval(days => $1 + 1)
         WHERE message_id = 'A1'",
        WINDOW_DAYS,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query!(
        "UPDATE inbox_messages SET expires_at = now() - interval '1 minute'
         WHERE message_id = 'A2'"
    )
    .execute(&pool)
    .await
    .unwrap();

    // Bob's chat is no longer readable by any connection
    let only_ada = readers(account_id, vec![reading(&[ADA])]);
    sweep(&pool, &only_ada).await.unwrap();
    let left = kept(&pool, user_id, ADA).await;
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].message_id, "A3");
    assert!(kept(&pool, user_id, BOB).await.is_empty());
}

#[sqlx::test]
async fn readers_come_from_the_connections_that_read(pool: PgPool) {
    let (user_id, account_id) = account(&pool).await;
    let create = async |name: &str, grant: &Grant, expired: bool| {
        let key = AccessKey::generate().unwrap();
        agent_connections::create(
            &pool,
            NewConnection {
                user_id,
                name,
                credential: Credential::Key {
                    hash: &key.hash,
                    hint: &key.hint,
                },
                grant,
                expires_at: expired.then(|| Utc::now() - TimeDelta::minutes(1)),
            },
        )
        .await
        .unwrap()
        .unwrap();
    };
    create("Sends only", &Preset::AskBeforeSending.grant(), false).await;
    create("Reads Ada", &reading(&[ADA]), false).await;
    create("Expired", &reading(&[BOB]), true).await;

    let readers = Readers::default();
    readers.reload(&pool).await.unwrap();
    assert!(readers.reads(account_id, RecipientType::Contact, ADA));
    assert!(!readers.reads(account_id, RecipientType::Contact, BOB));
    assert!(!readers.reads(account_id, RecipientType::Group, GROUP));
    assert!(!readers.reads(Uuid::new_v4(), RecipientType::Contact, ADA));
}

#[test]
fn a_reply_quotes_the_kept_message() {
    use crate::whatsapp::supervisor::{Quote, text_message};

    let chat = Jid::pn(ADA);
    let quote = Quote {
        message_id: "A1".to_owned(),
        sender_jid: "99887766@lid".to_owned(),
        text: Some("Can we meet at 5?".to_owned()),
    };
    let reply = text_message(&chat, "Yes, see you then".to_owned(), Some(quote));
    let text = reply.extended_text_message.as_option().unwrap();
    assert_eq!(text.text.as_deref(), Some("Yes, see you then"));
    let context = text.context_info.as_option().unwrap();
    assert_eq!(context.stanza_id.as_deref(), Some("A1"));
    assert_eq!(context.participant.as_deref(), Some("99887766@lid"));
    assert_eq!(context.remote_jid, None);
    let quoted = context.quoted_message.as_option().unwrap();
    assert_eq!(quoted.conversation.as_deref(), Some("Can we meet at 5?"));

    let plain = text_message(&chat, "Hello".to_owned(), None);
    assert_eq!(plain.conversation.as_deref(), Some("Hello"));
    // A sender that can't be addressed leaves an ordinary message rather than a failed send
    let unaddressed = Quote {
        message_id: "A1".to_owned(),
        sender_jid: String::new(),
        text: None,
    };
    let fallback = text_message(&chat, "Hello".to_owned(), Some(unaddressed));
    assert_eq!(fallback.conversation.as_deref(), Some("Hello"));
}

#[test]
fn what_this_device_sends_is_handed_over_as_the_owners_message() {
    let (changes, mut incoming) = mpsc::unbounded_channel();
    let inbox = super::Inbox {
        readers: Readers::default(),
        changes,
    };
    let account_id = Uuid::new_v4();
    inbox.sent(
        account_id,
        &Jid::pn(ADA),
        "S1",
        "8801700000000@s.whatsapp.net".to_owned(),
        "Yes, see you then".to_owned(),
        Some("A1".to_owned()),
    );
    let (account, change) = incoming.try_recv().unwrap();
    assert_eq!(account, account_id);
    let Change::Arrived {
        chat,
        message_id,
        from_me,
        body,
        ..
    } = change
    else {
        panic!("{change:?}");
    };
    assert_eq!(chat, Chat::Number(ADA.to_owned()));
    assert_eq!(message_id, "S1");
    assert!(from_me);
    assert_eq!(body.text.as_deref(), Some("Yes, see you then"));
    assert_eq!(body.reply_to.as_deref(), Some("A1"));

    // A status update isn't a chat
    inbox.sent(
        account_id,
        &"status@broadcast".parse().unwrap(),
        "S2",
        String::new(),
        "Hello".to_owned(),
        None,
    );
    assert!(incoming.try_recv().is_err());
}

#[sqlx::test]
async fn a_sender_without_a_number_gets_one_from_the_chat_or_the_address_book(pool: PgPool) {
    let cipher = Cipher::new(&[0; 32]);
    let (user_id, account_id) = account(&pool).await;
    let readers = readers(account_id, vec![reading(&[ADA, GROUP])]);
    let unnumbered = |chat: Chat, id: &str| {
        let Change::Arrived {
            chat,
            message_id,
            from_me,
            sent_at,
            expires_at,
            mut body,
        } = arrived(chat, id, "Hello")
        else {
            unreachable!();
        };
        body.sender_jid = "99887766@lid".to_owned();
        body.sender_number = None;
        Change::Arrived {
            chat,
            message_id,
            from_me,
            sent_at,
            expires_at,
            body,
        }
    };
    let number_of = async |kind: &str, chat: &str, id: &str| {
        let kept = inbox_messages::find(&pool, user_id, kind, chat, id)
            .await
            .unwrap()
            .unwrap();
        open_body(&cipher, &kept).unwrap().sender_number
    };
    let run = async |change| {
        apply(&pool, &cipher, &readers, account_id, change)
            .await
            .unwrap();
    };

    run(unnumbered(Chat::Number(ADA.to_owned()), "A1")).await;
    assert_eq!(number_of("contact", ADA, "A1").await.as_deref(), Some(ADA));

    // In a group, only the address book can tell whose LID it is
    run(unnumbered(Chat::Group(GROUP.to_owned()), "G1")).await;
    assert_eq!(number_of("group", GROUP, "G1").await, None);
    let jid = "99887766@lid";
    let names = Names {
        full_name: Some("Bob".to_owned()),
        first_name: None,
        username: None,
    };
    let sealed = seal_names(&cipher, account_id, jid, &names).unwrap();
    whatsapp_contacts::save(&pool, account_id, jid, Some(BOB), Some("99887766"), &sealed)
        .await
        .unwrap();
    run(unnumbered(Chat::Group(GROUP.to_owned()), "G2")).await;
    assert_eq!(number_of("group", GROUP, "G2").await.as_deref(), Some(BOB));
}
