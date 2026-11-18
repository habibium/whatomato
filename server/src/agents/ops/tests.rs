// Nothing here reaches WhatsApp: an account shows as connected through `WhatsApp::pretend`, which
// starts no client, so groups can't be listed and nothing is sent

use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use serde_json::json;
use sqlx::PgPool;
use url::Url;
use uuid::Uuid;

use super::{
    Caller, OpError, Page,
    account::{
        FindChatsInput, ListTemplatesInput, WhatsAppStatus, WhoAmIInput, find_chats,
        list_templates, summary, whoami,
    },
    approvals::{GetApprovalInput, ListApprovalsInput, get_approval, list_approvals},
    review::{self, Request, ReviewError},
    schedules::{
        DeleteScheduleInput, ListSchedulesInput, ScheduleInput, ScheduleOutcome,
        UpdateScheduleInput, delete_scheduled_message, list_scheduled_messages, schedule_message,
        update_scheduled_message,
    },
    sends::{
        DeliveryState, ListDeliveriesInput, SendInput, SendOutcome, list_deliveries, send_message,
    },
};
use crate::{
    agents::{
        ApprovalKind, ApprovalStatus, Grant, Outcome, Preset,
        events::Events,
        grant::{
            Chat, ChatRule, Contacts, Deliveries, Schedules, Scope, SendLevel, Strangers, Templates,
        },
        key::AccessKey,
    },
    auth::pwned::PwnedPasswords,
    crypto::Cipher,
    db::{
        agent_activity, agent_approvals,
        agent_connections::{self, Credential, NewConnection},
        message_templates, scheduled_messages, whatsapp_accounts, whatsapp_contacts,
    },
    email::Mailer,
    oauth::client::Clients,
    push::Push,
    rate_limit::RateLimits,
    scheduler::{RecipientType, ScheduleKind, recurrence::Recurrence},
    state::AppState,
    whatsapp::{
        contacts::{Names, seal_names},
        manager::WhatsApp,
        status::Status,
        supervisor::Context,
    },
};

const ADA: &str = "8801700000001";
const STRANGER: &str = "8801700000009";
const GROUP: &str = "120363000000000000@g.us";
const APP_URL: &str = "http://localhost:5173";

fn state(pool: PgPool) -> AppState {
    let cipher = Cipher::new(&[0; 32]);
    let mailer = Mailer::Outbox(Arc::default());
    let app_url = Arc::new(Url::parse(APP_URL).unwrap());
    AppState {
        pool: pool.clone(),
        mailer: mailer.clone(),
        app_url: Arc::clone(&app_url),
        rate_limits: RateLimits::default(),
        pwned_passwords: PwnedPasswords::Listed(&[]),
        oauth_clients: Clients::listed([]),
        cipher: cipher.clone(),
        whatsapp: WhatsApp::new(Context::new(pool, cipher, mailer, app_url)),
        push: Push::outbox(),
        events: Events::default(),
        agent_access: true,
        agent_reading: true,
        max_linked_accounts: None,
    }
}

async fn user(pool: &PgPool) -> Uuid {
    sqlx::query_scalar!(
        "INSERT INTO users (email, password) VALUES ($1, 'unused') RETURNING id",
        format!("{}@example.com", Uuid::new_v4()),
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn connect(state: &AppState, owner: Uuid, grant: &Grant) -> Caller {
    let key = AccessKey::generate().unwrap();
    let connection = agent_connections::create(
        &state.pool,
        NewConnection {
            user_id: owner,
            name: "Assistant",
            credential: Credential::Key {
                hash: &key.hash,
                hint: &key.hint,
            },
            grant,
            expires_at: None,
        },
    )
    .await
    .unwrap()
    .unwrap();
    Caller::from_connection(&state.app_url, connection).unwrap()
}

fn connected(state: &AppState, owner: Uuid) {
    state.whatsapp.pretend(
        owner,
        Status::Connected {
            phone_number: "8801700000000".into(),
        },
    );
}

async fn save_contact(state: &AppState, owner: Uuid, number: &str, name: &str) {
    let account = whatsapp_accounts::find_or_create(&state.pool, owner)
        .await
        .unwrap();
    let jid = format!("{number}@s.whatsapp.net");
    let names = Names {
        full_name: Some(name.to_owned()),
        first_name: name.split_whitespace().next().map(str::to_owned),
        username: None,
    };
    let sealed = seal_names(&state.cipher, account.id, &jid, &names).unwrap();
    whatsapp_contacts::save(&state.pool, account.id, &jid, Some(number), None, &sealed)
        .await
        .unwrap();
}

fn with_rule(mut grant: Grant, recipient: &str, send: SendLevel, limit: Option<u32>) -> Grant {
    let recipient_type = if recipient.ends_with("@g.us") {
        RecipientType::Group
    } else {
        RecipientType::Contact
    };
    grant.chats.rules.push(ChatRule {
        chat: Chat {
            recipient_type,
            id: recipient.to_owned(),
            name: Some("Ruled".to_owned()),
        },
        read: false,
        send,
        daily_limit: limit,
        note: Some("Reply in Bangla".to_owned()),
    });
    grant
}

fn send_to(recipient: &str, body: &str) -> SendInput {
    SendInput {
        recipient_type: if recipient.ends_with("@g.us") {
            RecipientType::Group
        } else {
            RecipientType::Contact
        },
        recipient: recipient.to_owned(),
        body: body.to_owned(),
        timezone: None,
        reply_to: None,
        mentions: Vec::new(),
    }
}

fn once_at(recipient: &str, at: DateTime<Utc>) -> ScheduleInput {
    ScheduleInput {
        recipient_type: RecipientType::Contact,
        recipient: recipient.to_owned(),
        body: "Good morning".to_owned(),
        schedule_kind: ScheduleKind::Once,
        run_at: Some(at),
        recurrence: None,
        timezone: None,
        template_id: None,
        enabled: None,
    }
}

fn tomorrow() -> DateTime<Utc> {
    Utc::now() + TimeDelta::days(1)
}

const fn update(id: Uuid) -> UpdateScheduleInput {
    UpdateScheduleInput {
        scheduled_message_id: id,
        recipient_type: None,
        recipient: None,
        body: None,
        schedule_kind: None,
        run_at: None,
        recurrence: None,
        timezone: None,
        template_id: None,
        enabled: None,
    }
}

/// The tool, outcome, reason and recipient of each of `caller`'s activity rows, oldest first
async fn activity(
    state: &AppState,
    caller: &Caller,
) -> Vec<(String, Outcome, Option<String>, Option<String>)> {
    let mut rows = agent_activity::page(
        &state.pool,
        caller.connection().user_id,
        Some(caller.connection().id),
        100,
        0,
    )
    .await
    .unwrap();
    rows.reverse();
    rows.into_iter()
        .map(|row| (row.tool, row.outcome, row.reason, row.recipient))
        .collect()
}

async fn last_activity(state: &AppState, caller: &Caller) -> (String, Outcome, Option<String>) {
    let (tool, outcome, reason, _) = activity(state, caller).await.pop().unwrap();
    (tool, outcome, reason)
}

#[track_caller]
fn refused_with<T: std::fmt::Debug>(result: Result<T, OpError>, expected: &str) {
    match result {
        Err(error @ OpError::Refused { .. }) => {
            assert_eq!(error.code(), expected);
            assert!(error.to_string().contains("/agents/"), "{error}");
        }
        other => panic!("expected a refusal for {expected}, got {other:?}"),
    }
}

async fn delivery_rows(pool: &PgPool, owner: Uuid) -> Vec<(Option<Uuid>, String, String)> {
    sqlx::query!(
        "SELECT connection_id, recipient, body FROM deliveries WHERE user_id = $1 \
         ORDER BY created_at",
        owner
    )
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .map(|row| (row.connection_id, row.recipient, row.body))
    .collect()
}

async fn approval(state: &AppState, id: Uuid, owner: Uuid) -> agent_approvals::Approval {
    agent_approvals::find(&state.pool, id, owner)
        .await
        .unwrap()
        .unwrap()
}

#[sqlx::test]
async fn a_paused_expired_or_garbled_connection_can_do_nothing(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    let caller = connect(&state, owner, &Preset::FullAccess.grant()).await;
    let id = caller.connection().id;
    let reload = || async {
        agent_connections::find(&state.pool, id, owner)
            .await
            .unwrap()
            .unwrap()
    };

    agent_connections::pause(&state.pool, id, owner)
        .await
        .unwrap();
    refused_with(
        Caller::from_connection(&state.app_url, reload().await).map(|_| ()),
        "paused",
    );
    agent_connections::resume(&state.pool, id, owner)
        .await
        .unwrap();

    sqlx::query!(
        "UPDATE agent_connections SET expires_at = now() - interval '1 minute' WHERE id = $1",
        id
    )
    .execute(&state.pool)
    .await
    .unwrap();
    refused_with(
        Caller::from_connection(&state.app_url, reload().await).map(|_| ()),
        "expired",
    );
    let key = AccessKey::generate().unwrap();
    agent_connections::rotate_key(&state.pool, id, owner, &key.hash, &key.hint)
        .await
        .unwrap();
    let found = agent_connections::find_by_key(&state.pool, &key.hash)
        .await
        .unwrap()
        .unwrap();
    refused_with(
        Caller::from_key_match(&state.app_url, found).map(|_| ()),
        "expired",
    );

    sqlx::query!(
        r#"UPDATE agent_connections SET expires_at = NULL, "grant" = $2 WHERE id = $1"#,
        id,
        json!({ "version": 1, "everything": true }),
    )
    .execute(&state.pool)
    .await
    .unwrap();
    let error = Caller::from_connection(&state.app_url, reload().await)
        .map(|_| ())
        .unwrap_err();
    assert_eq!(error.code(), "grant_unreadable");
    assert_eq!(
        error.to_string(),
        format!(
            "This connection's permissions couldn't be read, so it can't do anything. The owner \
             can fix this by saving its permissions again. The owner manages this connection at \
             {APP_URL}/agents/{id}"
        )
    );
    let found = agent_connections::find_by_key(&state.pool, &key.hash)
        .await
        .unwrap()
        .unwrap();
    refused_with(
        Caller::from_key_match(&state.app_url, found).map(|_| ()),
        "grant_unreadable",
    );
}

#[test]
fn the_summary_names_no_tool_and_counts_chats_in_words() {
    let mut grant = Preset::AskBeforeSending.grant();
    grant = with_rule(grant, ADA, SendLevel::Yes, None);
    let lines = summary(&grant);
    assert!(lines.contains(&"One chat has a rule of its own, which applies there instead.".into()));
    grant = with_rule(grant, STRANGER, SendLevel::No, None);
    assert!(
        summary(&grant)
            .contains(&"2 chats have a rule of their own, which applies there instead.".into())
    );

    grant.chats.scope = Scope::Selected;
    assert!(summary(&grant).contains(
        &"It may use only the 2 chats the owner selected, each under its own rule.".into()
    ));
    grant.chats.rules.truncate(1);
    assert!(
        summary(&grant).contains(
            &"It may use only the one chat the owner selected, under its own rule.".into()
        )
    );
    grant.chats.rules.clear();
    assert!(
        summary(&grant)
            .contains(&"It may use no chats, since the owner hasn't selected any.".into())
    );

    let tools = [
        "find_chats",
        "send_message",
        "whoami",
        "list_",
        "schedule_message",
    ];
    for line in summary(&grant).iter().chain(&lines) {
        assert!(!tools.iter().any(|tool| line.contains(tool)), "{line}");
    }
}

#[sqlx::test]
async fn whoami_says_what_it_may_do_and_how_much_is_used(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    let mut grant = Preset::AskBeforeSending.grant();
    grant.chats.default.note = Some("Sign off as Habib".to_owned());
    let caller = connect(&state, owner, &grant).await;

    let me = whoami(&state, &caller, WhoAmIInput::default())
        .await
        .unwrap();
    assert_eq!(me.connection_name, "Assistant");
    assert_eq!(me.whatsapp.status, WhatsAppStatus::NotLinked);
    assert!(!me.whatsapp.can_send);
    assert_eq!(me.permissions.default_send, SendLevel::Ask);
    assert_eq!(me.default_note.as_deref(), Some("Sign off as Habib"));
    assert_eq!(me.limits.sends_last_24_hours, 0);
    assert_eq!(me.limits.daily_ceiling, 200);
    assert_eq!(me.limits.approvals_waiting_max, 20);
    assert!(
        me.summary
            .iter()
            .any(|line| line.contains("asks the owner to approve"))
    );
    assert!(
        me.summary.iter().all(|line| !line.contains("Habib")),
        "the owner's note stays in its own field"
    );

    send_message(&state, &caller, send_to(ADA, "Hello"))
        .await
        .unwrap();
    connected(&state, owner);
    let me = whoami(&state, &caller, WhoAmIInput::default())
        .await
        .unwrap();
    assert_eq!(me.whatsapp.status, WhatsAppStatus::Connected);
    assert!(me.whatsapp.can_send);
    assert_eq!(me.limits.sends_last_24_hours, 1);
    assert_eq!(me.limits.approvals_waiting, 1);
    assert_eq!(
        last_activity(&state, &caller).await,
        ("whoami".to_owned(), Outcome::Allowed, None)
    );
}

#[sqlx::test]
async fn sending_without_asking_queues_a_delivery_of_this_connection(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    save_contact(&state, owner, ADA, "Ada Lovelace").await;
    let caller = connect(&state, owner, &Preset::SendWithoutAsking.grant()).await;

    let not_linked = send_message(&state, &caller, send_to(ADA, "Hello")).await;
    assert!(matches!(not_linked, Err(OpError::WhatsAppNotReady(_))));
    assert_eq!(
        last_activity(&state, &caller).await,
        (
            "send_message".to_owned(),
            Outcome::Failed,
            Some("whatsapp_not_ready".to_owned())
        )
    );

    connected(&state, owner);
    let sent = send_message(
        &state,
        &caller,
        send_to("+880 1700-000001", "Hi {{first_name}}"),
    )
    .await
    .unwrap();
    assert_eq!(sent.outcome, SendOutcome::Queued);
    assert!(sent.approval_id.is_none());
    assert_eq!(
        delivery_rows(&state.pool, owner).await,
        [(
            Some(caller.connection().id),
            ADA.to_owned(),
            "Hi Ada".to_owned()
        )]
    );
    let (tool, outcome, reason, recipient) = activity(&state, &caller).await.pop().unwrap();
    assert_eq!(
        (tool.as_str(), outcome, reason, recipient.as_deref()),
        ("send_message", Outcome::Allowed, None, Some(ADA))
    );
}

#[sqlx::test]
async fn asking_first_makes_an_approval_and_sends_nothing(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    let caller = connect(&state, owner, &Preset::AskBeforeSending.grant()).await;

    let asked = send_message(&state, &caller, send_to(GROUP, "Meeting moved"))
        .await
        .unwrap();
    assert_eq!(asked.outcome, SendOutcome::WaitingForApproval);
    let id = asked.approval_id.unwrap();
    assert_eq!(
        asked.approval_url.as_deref(),
        Some(format!("{APP_URL}/agents?approval={id}").as_str())
    );
    assert_eq!(delivery_rows(&state.pool, owner).await.len(), 0);

    let approval = approval(&state, id, owner).await;
    assert_eq!(approval.kind, ApprovalKind::Send);
    assert_eq!(approval.status, ApprovalStatus::Waiting);
    assert!(
        !approval
            .payload
            .windows(b"Meeting".len())
            .any(|window| window == b"Meeting"),
        "the text is sealed"
    );
    let request = review::open(&state.cipher, &approval).unwrap();
    assert_eq!(request.chat(), (RecipientType::Group, GROUP, None));
    let Request::Send(send) = request else {
        panic!("a send")
    };
    assert_eq!(send.body, "Meeting moved");
    assert_eq!(
        last_activity(&state, &caller).await,
        ("send_message".to_owned(), Outcome::Asked, None)
    );
}

#[sqlx::test]
async fn refusals_name_their_reason_and_send_nothing(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    connected(&state, owner);
    let view_only = connect(&state, owner, &Preset::ViewOnly.grant()).await;

    refused_with(
        send_message(&state, &view_only, send_to(GROUP, "Hi")).await,
        "sending_off",
    );
    // A rule of no is the reason even for a number that isn't saved
    refused_with(
        send_message(&state, &view_only, send_to(STRANGER, "Hi")).await,
        "sending_off",
    );

    let mut wary = Preset::AskBeforeSending.grant();
    wary.chats.strangers = Strangers::Never;
    let wary = connect(&state, owner, &wary).await;
    refused_with(
        send_message(&state, &wary, send_to(STRANGER, "Hi")).await,
        "strangers_never",
    );
    assert_eq!(
        last_activity(&state, &wary).await,
        (
            "send_message".to_owned(),
            Outcome::Denied,
            Some("strangers_never".to_owned())
        )
    );

    let mut selected = Preset::FullAccess.grant();
    selected.chats.scope = Scope::Selected;
    let selected = connect(
        &state,
        owner,
        &with_rule(selected, ADA, SendLevel::Yes, None),
    )
    .await;
    refused_with(
        send_message(&state, &selected, send_to(GROUP, "Hi")).await,
        "out_of_scope",
    );
    let ruled = send_message(&state, &selected, send_to(ADA, "Hi"))
        .await
        .unwrap();
    assert_eq!(ruled.outcome, SendOutcome::Queued);
    assert_eq!(delivery_rows(&state.pool, owner).await.len(), 1);
}

#[sqlx::test]
async fn strangers_follow_their_setting_unless_they_have_a_rule(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    connected(&state, owner);
    save_contact(&state, owner, ADA, "Ada").await;
    let grant = with_rule(
        Preset::SendWithoutAsking.grant(),
        STRANGER,
        SendLevel::Yes,
        None,
    );
    let caller = connect(&state, owner, &grant).await;

    let saved = send_message(&state, &caller, send_to(ADA, "Hi"))
        .await
        .unwrap();
    assert_eq!(saved.outcome, SendOutcome::Queued);
    let ruled = send_message(&state, &caller, send_to(STRANGER, "Hi"))
        .await
        .unwrap();
    assert_eq!(ruled.outcome, SendOutcome::Queued);
    let unknown = send_message(&state, &caller, send_to("8801700000008", "Hi"))
        .await
        .unwrap();
    assert_eq!(unknown.outcome, SendOutcome::WaitingForApproval);

    let mut allowed = Preset::FullAccess.grant();
    allowed.chats.strangers = Strangers::Allowed;
    let allowed = connect(&state, owner, &allowed).await;
    let free = send_message(&state, &allowed, send_to("8801700000008", "Hi"))
        .await
        .unwrap();
    assert_eq!(free.outcome, SendOutcome::Queued);
}

#[sqlx::test]
async fn a_chats_daily_limit_and_the_ceiling_count_real_sends(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    connected(&state, owner);
    save_contact(&state, owner, ADA, "Ada").await;
    let grant = with_rule(Preset::FullAccess.grant(), ADA, SendLevel::Yes, Some(2));
    let caller = connect(&state, owner, &grant).await;

    for _ in 0..2 {
        send_message(&state, &caller, send_to(ADA, "Hi"))
            .await
            .unwrap();
    }
    refused_with(
        send_message(&state, &caller, send_to(ADA, "Hi")).await,
        "chat_limit",
    );

    sqlx::query!(
        "INSERT INTO deliveries (user_id, connection_id, recipient_type, recipient, body) \
         SELECT $1, $2, 'group', $3, 'old' FROM generate_series(1, 198)",
        owner,
        caller.connection().id,
        GROUP,
    )
    .execute(&state.pool)
    .await
    .unwrap();
    refused_with(
        send_message(&state, &caller, send_to("8801700000005", "Hi")).await,
        "daily_ceiling",
    );

    let other = connect(&state, owner, &Preset::FullAccess.grant()).await;
    let fresh = send_message(&state, &other, send_to(GROUP, "Hi"))
        .await
        .unwrap();
    assert_eq!(
        fresh.outcome,
        SendOutcome::Queued,
        "limits are per connection"
    );
}

#[sqlx::test]
async fn bad_input_is_described_and_recorded_as_failed(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    let caller = connect(&state, owner, &Preset::FullAccess.grant()).await;

    let mut input = send_to("12345", "Hi {{nope}}");
    input.timezone = Some("Mars/Base".to_owned());
    let Err(OpError::Invalid(problems)) = send_message(&state, &caller, input).await else {
        panic!("invalid")
    };
    assert_eq!(
        problems,
        [
            "`body` has a problem: {{nope}} isn't a variable.",
            "`recipient` must be a phone number with its country code.",
            "`timezone` must be an IANA time zone.",
        ]
    );
    assert_eq!(
        last_activity(&state, &caller).await,
        (
            "send_message".to_owned(),
            Outcome::Failed,
            Some("invalid_input".to_owned())
        )
    );
}

#[sqlx::test]
async fn activity_never_holds_a_message(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    connected(&state, owner);
    let caller = connect(&state, owner, &Preset::FullAccess.grant()).await;
    let asker = connect(&state, owner, &Preset::AskBeforeSending.grant()).await;

    send_message(&state, &caller, send_to(ADA, "secret words one"))
        .await
        .unwrap();
    send_message(&state, &asker, send_to(ADA, "secret words two"))
        .await
        .unwrap();
    let mut scheduled = once_at(ADA, tomorrow());
    scheduled.body = "secret words three".to_owned();
    schedule_message(&state, &caller, scheduled).await.unwrap();

    let rows: Vec<String> = sqlx::query_scalar!(
        r#"SELECT to_jsonb(a)::text AS "row!" FROM agent_activity a WHERE user_id = $1"#,
        owner
    )
    .fetch_all(&state.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 3);
    for row in rows {
        assert!(!row.contains("secret"), "{row}");
    }
}

#[sqlx::test]
async fn deliveries_are_own_or_all_as_the_grant_says(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    connected(&state, owner);
    let own = connect(&state, owner, &Preset::SendWithoutAsking.grant()).await;
    let all = connect(&state, owner, &Preset::FullAccess.grant()).await;
    send_message(&state, &all, send_to(GROUP, "From the other"))
        .await
        .unwrap();
    send_message(&state, &own, send_to(GROUP, "Mine"))
        .await
        .unwrap();
    sqlx::query!(
        "INSERT INTO deliveries (user_id, recipient_type, recipient, body, status, error) \
         VALUES ($1, 'contact', $2, 'By hand', 'failed', 'This number isn''t on WhatsApp')",
        owner,
        ADA,
    )
    .execute(&state.pool)
    .await
    .unwrap();

    let mine = list_deliveries(&state, &own, ListDeliveriesInput::default())
        .await
        .unwrap();
    assert_eq!(mine.deliveries.len(), 1);
    assert!(mine.deliveries[0].by_this_connection);
    assert_eq!(mine.deliveries[0].status, DeliveryState::Pending);
    assert!(!mine.more.has_more);

    let everything = list_deliveries(&state, &all, ListDeliveriesInput::default())
        .await
        .unwrap();
    assert_eq!(everything.deliveries.len(), 3);
    let failed = &everything.deliveries[0];
    assert_eq!(failed.status, DeliveryState::Failed);
    assert_eq!(
        failed.error.as_deref(),
        Some("This number isn't on WhatsApp")
    );
    assert!(!failed.by_this_connection);

    let page = |limit, offset| ListDeliveriesInput {
        page: Page { limit, offset },
    };
    let first = list_deliveries(&state, &all, page(Some(2), None))
        .await
        .unwrap();
    assert_eq!(first.deliveries.len(), 2);
    assert!(first.more.has_more);
    assert_eq!(first.more.next_offset, Some(2));
    let rest = list_deliveries(&state, &all, page(Some(2), Some(2)))
        .await
        .unwrap();
    assert_eq!(rest.deliveries.len(), 1);
    assert!(!rest.more.has_more);
    assert!(matches!(
        list_deliveries(&state, &all, page(Some(51), None)).await,
        Err(OpError::Invalid(_))
    ));
}

#[sqlx::test]
async fn find_chats_searches_saved_contacts_with_each_ones_rule(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    save_contact(&state, owner, ADA, "Ada Lovelace").await;
    save_contact(&state, owner, "8801700000002", "Adam Smith").await;
    save_contact(&state, owner, "8801700000003", "Grace Hopper").await;
    let grant = with_rule(
        Preset::AskBeforeSending.grant(),
        ADA,
        SendLevel::Yes,
        Some(5),
    );
    let caller = connect(&state, owner, &grant).await;
    let query = |query: &str| FindChatsInput {
        query: query.to_owned(),
    };

    let found = find_chats(&state, &caller, query(" ADA ")).await.unwrap();
    let names: Vec<_> = found.chats.iter().map(|chat| chat.name.as_str()).collect();
    assert_eq!(names, ["Ada Lovelace", "Adam Smith"]);
    assert!(!found.more);
    assert!(!found.groups_searched, "WhatsApp isn't connected");
    let ada = &found.chats[0];
    assert_eq!(ada.recipient, ADA);
    assert_eq!(ada.rule.send, SendLevel::Yes);
    assert_eq!(ada.rule.daily_limit, Some(5));
    assert_eq!(ada.rule.note.as_deref(), Some("Reply in Bangla"));
    assert_eq!(found.chats[1].rule.send, SendLevel::Ask);

    assert!(matches!(
        find_chats(&state, &caller, query("a")).await,
        Err(OpError::Invalid(_))
    ));

    connected(&state, owner);
    let found = find_chats(&state, &caller, query("ada")).await.unwrap();
    assert!(
        !found.groups_searched,
        "with no client behind it, the groups can't be read"
    );
    assert_eq!(found.chats.len(), 2, "the contacts still answer");

    let mut selected = grant.clone();
    selected.chats.scope = Scope::Selected;
    let selected = connect(&state, owner, &selected).await;
    let found = find_chats(&state, &selected, query("ada")).await.unwrap();
    assert_eq!(found.chats.len(), 1, "only selected chats match");

    let mut blind = Preset::FullAccess.grant();
    blind.contacts = Contacts::None;
    let blind = connect(&state, owner, &blind).await;
    refused_with(
        find_chats(&state, &blind, query("ada")).await,
        "contacts_off",
    );
}

#[sqlx::test]
async fn find_chats_returns_at_most_20_and_says_there_are_more(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    for n in 0..25 {
        save_contact(
            &state,
            owner,
            &format!("88017000001{n:02}"),
            &format!("Member {n:02}"),
        )
        .await;
    }
    let caller = connect(&state, owner, &Preset::FullAccess.grant()).await;
    let found = find_chats(
        &state,
        &caller,
        FindChatsInput {
            query: "member".to_owned(),
        },
    )
    .await
    .unwrap();
    assert_eq!(found.chats.len(), 20);
    assert!(found.more);
}

#[sqlx::test]
async fn scheduling_creates_asks_or_refuses(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    let free = connect(&state, owner, &Preset::FullAccess.grant()).await;
    let asker = connect(&state, owner, &Preset::AskBeforeSending.grant()).await;
    let none = connect(&state, owner, &Preset::ViewOnly.grant()).await;

    let created = schedule_message(&state, &free, once_at("+880 1700 000001", tomorrow()))
        .await
        .unwrap();
    assert_eq!(created.outcome, ScheduleOutcome::Created);
    let schedule = created.schedule.unwrap();
    assert!(schedule.belongs_to_this_connection);
    assert_eq!(schedule.recipient, ADA);
    let stored = scheduled_messages::find(&state.pool, schedule.id, owner)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.connection_id, Some(free.connection().id));

    let sunset = ScheduleInput {
        schedule_kind: ScheduleKind::Recurring,
        run_at: None,
        recurrence: Some(
            serde_json::from_value(json!({
                "type": "sun", "event": "sunset", "offset_minutes": -10, "days": "* * *",
                "place": null
            }))
            .unwrap(),
        ),
        timezone: Some("Asia/Dhaka".to_owned()),
        ..once_at(ADA, tomorrow())
    };
    let asked = schedule_message(&state, &asker, sunset).await.unwrap();
    assert_eq!(asked.outcome, ScheduleOutcome::WaitingForApproval);
    let waiting = approval(&state, asked.approval_id.unwrap(), owner).await;
    assert_eq!(waiting.kind, ApprovalKind::ScheduleCreate);
    let shown = review::Requested::new(review::open(&state.cipher, &waiting).unwrap());
    let schedule = shown.schedule.unwrap();
    assert_eq!(schedule.next_runs.len(), 3);
    assert_eq!(schedule.city, Some("Dhaka"));

    refused_with(
        schedule_message(&state, &none, once_at(ADA, tomorrow())).await,
        "schedules_off",
    );

    let past = schedule_message(
        &state,
        &free,
        once_at(ADA, Utc::now() - TimeDelta::minutes(1)),
    )
    .await;
    let Err(OpError::Invalid(problems)) = past else {
        panic!("invalid")
    };
    assert_eq!(problems, ["`run_at` must be in the future."]);

    let mut cron = once_at(ADA, tomorrow());
    cron.schedule_kind = ScheduleKind::Recurring;
    cron.recurrence = Some(Recurrence::Cron {
        expression: "every day".to_owned(),
    });
    assert!(matches!(
        schedule_message(&state, &free, cron).await,
        Err(OpError::Invalid(_))
    ));

    let mut foreign = once_at(ADA, tomorrow());
    let stranger = user(&state.pool).await;
    foreign.template_id = Some(
        message_templates::create(&state.pool, stranger, "Theirs", "Hi")
            .await
            .unwrap()
            .id,
    );
    let Err(OpError::Invalid(problems)) = schedule_message(&state, &free, foreign).await else {
        panic!("invalid")
    };
    assert_eq!(problems, ["`template_id` isn't one of your templates."]);
}

#[sqlx::test]
async fn schedules_are_own_or_all_as_the_grant_says(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    let mut own_grant = Preset::FullAccess.grant();
    own_grant.schedules = Schedules::Own;
    let own = connect(&state, owner, &own_grant).await;
    let all = connect(&state, owner, &Preset::FullAccess.grant()).await;

    let mine = schedule_message(&state, &own, once_at(ADA, tomorrow()))
        .await
        .unwrap()
        .schedule
        .unwrap();
    let theirs = schedule_message(&state, &all, once_at(ADA, tomorrow()))
        .await
        .unwrap()
        .schedule
        .unwrap();

    let listed = list_scheduled_messages(&state, &own, ListSchedulesInput::default())
        .await
        .unwrap();
    assert_eq!(listed.schedules.len(), 1);
    assert_eq!(listed.schedules[0].id, mine.id);
    let listed = list_scheduled_messages(&state, &all, ListSchedulesInput::default())
        .await
        .unwrap();
    assert_eq!(listed.schedules.len(), 2);

    let mut change = update(theirs.id);
    change.body = Some("Changed".to_owned());
    refused_with(
        update_scheduled_message(&state, &own, change).await,
        "not_own_schedule",
    );
    refused_with(
        delete_scheduled_message(
            &state,
            &own,
            DeleteScheduleInput {
                scheduled_message_id: theirs.id,
            },
        )
        .await,
        "not_own_schedule",
    );

    let mut change = update(mine.id);
    change.body = Some("Changed".to_owned());
    let changed = update_scheduled_message(&state, &own, change)
        .await
        .unwrap();
    assert_eq!(changed.outcome, ScheduleOutcome::Updated);
    assert!(
        !changed.message.contains("now belongs"),
        "{}",
        changed.message
    );
    assert_eq!(changed.schedule.unwrap().body, "Changed");

    delete_scheduled_message(
        &state,
        &all,
        DeleteScheduleInput {
            scheduled_message_id: mine.id,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        delete_scheduled_message(
            &state,
            &all,
            DeleteScheduleInput {
                scheduled_message_id: mine.id
            }
        )
        .await,
        Err(OpError::NotFound(_))
    ));

    let view_only = connect(&state, owner, &Preset::ViewOnly.grant()).await;
    refused_with(
        list_scheduled_messages(&state, &view_only, ListSchedulesInput::default()).await,
        "schedules_off",
    );
}

#[sqlx::test]
async fn an_update_is_decided_for_the_chat_it_will_send_to(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    let grant = with_rule(Preset::FullAccess.grant(), GROUP, SendLevel::No, None);
    let grant = with_rule(grant, STRANGER, SendLevel::Ask, None);
    let caller = connect(&state, owner, &grant).await;
    let schedule = schedule_message(&state, &caller, once_at(ADA, tomorrow()))
        .await
        .unwrap()
        .schedule
        .unwrap();

    let mut to_group = update(schedule.id);
    to_group.recipient_type = Some(RecipientType::Group);
    to_group.recipient = Some(GROUP.to_owned());
    refused_with(
        update_scheduled_message(&state, &caller, to_group).await,
        "sending_off",
    );

    let mut to_asked = update(schedule.id);
    to_asked.recipient = Some(STRANGER.to_owned());
    to_asked.body = Some("New text".to_owned());
    let asked = update_scheduled_message(&state, &caller, to_asked)
        .await
        .unwrap();
    assert_eq!(asked.outcome, ScheduleOutcome::WaitingForApproval);
    let waiting = approval(&state, asked.approval_id.unwrap(), owner).await;
    assert_eq!(waiting.kind, ApprovalKind::ScheduleUpdate);
    assert_eq!(waiting.recipient, STRANGER);
    let Request::ScheduleUpdate {
        scheduled_message_id,
        schedule: whole,
        ..
    } = review::open(&state.cipher, &waiting).unwrap()
    else {
        panic!("an update")
    };
    assert_eq!(scheduled_message_id, schedule.id);
    assert_eq!(whole.body, "New text");
    assert_eq!(whole.run_at, schedule.run_at, "the whole new version");
    let unchanged = scheduled_messages::find(&state.pool, schedule.id, owner)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.recipient, ADA);

    assert!(matches!(
        update_scheduled_message(&state, &caller, update(schedule.id)).await,
        Err(OpError::Invalid(_))
    ));
}

#[sqlx::test]
async fn switching_a_schedule_off_needs_only_the_right_to_manage_it(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    let free = connect(&state, owner, &Preset::FullAccess.grant()).await;
    let schedule = schedule_message(&state, &free, once_at(ADA, tomorrow()))
        .await
        .unwrap()
        .schedule
        .unwrap();

    let mut no_sends = Preset::FullAccess.grant();
    no_sends.chats.default.send = SendLevel::No;
    let no_sends = connect(&state, owner, &no_sends).await;
    let mut off = update(schedule.id);
    off.enabled = Some(false);
    let switched = update_scheduled_message(&state, &no_sends, off)
        .await
        .unwrap()
        .schedule
        .unwrap();
    assert!(!switched.enabled);
    assert_eq!(switched.next_run_at, None);

    let mut on = update(schedule.id);
    on.enabled = Some(true);
    refused_with(
        update_scheduled_message(&state, &no_sends, on).await,
        "sending_off",
    );
}

#[sqlx::test]
async fn templates_follow_the_grant_and_pages_stop_at_the_text_budget(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    for n in 0..15 {
        message_templates::create(
            &state.pool,
            owner,
            &format!("Template {n:02}"),
            &"x".repeat(4000),
        )
        .await
        .unwrap();
    }
    let reader = connect(&state, owner, &Preset::FullAccess.grant()).await;
    let page = list_templates(&state, &reader, ListTemplatesInput::default())
        .await
        .unwrap();
    assert_eq!(page.templates.len(), 9, "40,000 characters at most");
    assert!(page.more.has_more);
    assert_eq!(page.more.next_offset, Some(9));
    let rest = list_templates(
        &state,
        &reader,
        ListTemplatesInput {
            page: Page {
                limit: None,
                offset: Some(9),
            },
        },
    )
    .await
    .unwrap();
    assert_eq!(rest.templates.len(), 6);
    assert!(!rest.more.has_more);

    let mut blind = Preset::FullAccess.grant();
    blind.templates = Templates::None;
    let blind = connect(&state, owner, &blind).await;
    refused_with(
        list_templates(&state, &blind, ListTemplatesInput::default()).await,
        "templates_off",
    );
}

#[sqlx::test]
async fn an_agent_sees_only_its_own_approvals(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    let one = connect(&state, owner, &Preset::AskBeforeSending.grant()).await;
    let two = connect(&state, owner, &Preset::AskBeforeSending.grant()).await;
    let mine = send_message(&state, &one, send_to(ADA, "Hi"))
        .await
        .unwrap()
        .approval_id
        .unwrap();
    let theirs = send_message(&state, &two, send_to(ADA, "Hi"))
        .await
        .unwrap()
        .approval_id
        .unwrap();

    let seen = get_approval(&state, &one, GetApprovalInput { approval_id: mine })
        .await
        .unwrap();
    assert_eq!(seen.status, ApprovalStatus::Waiting);
    assert_eq!(seen.kind, ApprovalKind::Send);
    assert!(seen.delivery.is_none());
    assert!(matches!(
        get_approval(
            &state,
            &one,
            GetApprovalInput {
                approval_id: theirs
            }
        )
        .await,
        Err(OpError::NotFound(_))
    ));
    let listed = list_approvals(&state, &one, ListApprovalsInput::default())
        .await
        .unwrap();
    assert_eq!(listed.approvals.len(), 1);
    assert_eq!(listed.approvals[0].id, mine);
}

/// A waiting send approval from a fresh "ask first" connection
async fn asked_send(state: &AppState, owner: Uuid, body: &str) -> (Caller, Uuid) {
    let caller = connect(state, owner, &Preset::AskBeforeSending.grant()).await;
    let id = send_message(state, &caller, send_to(ADA, body))
        .await
        .unwrap()
        .approval_id
        .unwrap();
    (caller, id)
}

#[sqlx::test]
async fn approving_a_send_queues_it_once_for_the_connection(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    let (caller, id) = asked_send(&state, owner, "Hello").await;

    let Err(ReviewError::CantApprove(problems)) = review::approve(&state, id, owner, None).await
    else {
        panic!("can't approve")
    };
    // Read by the owner, so it says what they can do, not what "the owner" can
    assert_eq!(
        problems,
        ["WhatsApp isn't linked, so this message can't be sent. Link WhatsApp, then approve it."]
    );

    connected(&state, owner);
    let approved = review::approve(&state, id, owner, None).await.unwrap();
    assert_eq!(approved.status, ApprovalStatus::Approved);
    let delivery_id = approved.delivery_id.unwrap();
    assert_eq!(
        delivery_rows(&state.pool, owner).await,
        [(
            Some(caller.connection().id),
            ADA.to_owned(),
            "Hello".to_owned()
        )]
    );

    assert!(matches!(
        review::approve(&state, id, owner, None).await,
        Err(ReviewError::NotWaiting(ApprovalStatus::Approved))
    ));
    assert_eq!(delivery_rows(&state.pool, owner).await.len(), 1);

    let seen = get_approval(&state, &caller, GetApprovalInput { approval_id: id })
        .await
        .unwrap();
    assert_eq!(seen.status, ApprovalStatus::Approved);
    assert_eq!(seen.delivery.unwrap().id, delivery_id);

    let stranger = user(&state.pool).await;
    assert!(matches!(
        review::approve(&state, id, stranger, None).await,
        Err(ReviewError::NotFound)
    ));
}

#[sqlx::test]
async fn two_clicks_at_once_approve_once(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    connected(&state, owner);
    let (_, id) = asked_send(&state, owner, "Hello").await;

    let (first, second) = tokio::join!(
        review::approve(&state, id, owner, None),
        review::approve(&state, id, owner, None)
    );
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    assert_eq!(delivery_rows(&state.pool, owner).await.len(), 1);
}

#[sqlx::test]
async fn the_owner_can_edit_the_text_but_not_into_a_bad_one(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    connected(&state, owner);
    let (_, id) = asked_send(&state, owner, "Hello").await;

    let bad = review::approve(&state, id, owner, Some("Hi {{nope}}".to_owned())).await;
    let Err(ReviewError::CantApprove(problems)) = bad else {
        panic!("can't approve")
    };
    assert_eq!(
        problems,
        [
            "The text has a problem: {{nope}} isn't a variable.",
            "Edit the text and approve it, or reject this request."
        ]
    );
    assert_eq!(
        approval(&state, id, owner).await.status,
        ApprovalStatus::Waiting
    );

    review::approve(&state, id, owner, Some("  Hello, edited  ".to_owned()))
        .await
        .unwrap();
    assert_eq!(
        delivery_rows(&state.pool, owner).await[0].2,
        "Hello, edited"
    );
}

#[sqlx::test]
async fn an_expired_or_rejected_approval_cant_be_approved(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    connected(&state, owner);
    let (_, late) = asked_send(&state, owner, "Hello").await;
    sqlx::query!(
        "UPDATE agent_approvals SET expires_at = now() - interval '1 minute' WHERE id = $1",
        late
    )
    .execute(&state.pool)
    .await
    .unwrap();
    assert!(matches!(
        review::approve(&state, late, owner, None).await,
        Err(ReviewError::NotWaiting(ApprovalStatus::Expired))
    ));
    assert!(matches!(
        review::reject(&state.pool, late, owner).await,
        Err(ReviewError::NotWaiting(ApprovalStatus::Expired))
    ));

    let (caller, id) = asked_send(&state, owner, "Hello").await;
    let rejected = review::reject(&state.pool, id, owner).await.unwrap();
    assert_eq!(rejected.status, ApprovalStatus::Rejected);
    assert!(matches!(
        review::approve(&state, id, owner, None).await,
        Err(ReviewError::NotWaiting(ApprovalStatus::Rejected))
    ));
    assert_eq!(delivery_rows(&state.pool, owner).await.len(), 0);
    let seen = get_approval(&state, &caller, GetApprovalInput { approval_id: id })
        .await
        .unwrap();
    assert_eq!(seen.status, ApprovalStatus::Rejected);
}

#[sqlx::test]
async fn approving_a_schedule_creates_it_for_the_connection(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    let caller = connect(&state, owner, &Preset::AskBeforeSending.grant()).await;
    let id = schedule_message(&state, &caller, once_at(ADA, tomorrow()))
        .await
        .unwrap()
        .approval_id
        .unwrap();

    let approved = review::approve(&state, id, owner, Some("Edited".to_owned()))
        .await
        .unwrap();
    let schedule =
        scheduled_messages::find(&state.pool, approved.scheduled_message_id.unwrap(), owner)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(schedule.connection_id, Some(caller.connection().id));
    assert_eq!(schedule.body, "Edited");
    assert!(schedule.enabled);
}

#[sqlx::test]
async fn a_schedule_whose_time_has_passed_stays_waiting(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    let caller = connect(&state, owner, &Preset::AskBeforeSending.grant()).await;
    let id = schedule_message(&state, &caller, once_at(ADA, tomorrow()))
        .await
        .unwrap()
        .approval_id
        .unwrap();
    // The request's own time can't be moved, so the clock is: its run_at is a day ahead
    let waiting = approval(&state, id, owner).await;
    let Request::ScheduleCreate(mut schedule) = review::open(&state.cipher, &waiting).unwrap()
    else {
        panic!("a schedule")
    };
    schedule.run_at = Some(Utc::now() - TimeDelta::minutes(1));
    let resealed = review::seal(&state.cipher, id, &Request::ScheduleCreate(schedule)).unwrap();
    sqlx::query!(
        "UPDATE agent_approvals SET payload = $2 WHERE id = $1",
        id,
        resealed
    )
    .execute(&state.pool)
    .await
    .unwrap();

    let Err(ReviewError::CantApprove(problems)) = review::approve(&state, id, owner, None).await
    else {
        panic!("can't approve")
    };
    assert_eq!(
        problems,
        [
            "The time it was to be sent has passed.",
            "Reject this request, and have the agent ask again if it's still wanted."
        ]
    );
    assert_eq!(
        approval(&state, id, owner).await.status,
        ApprovalStatus::Waiting
    );
    review::reject(&state.pool, id, owner).await.unwrap();
}

#[sqlx::test]
async fn approving_an_update_changes_the_schedule_and_keeps_its_creator(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    let free = connect(&state, owner, &Preset::FullAccess.grant()).await;
    let schedule = schedule_message(&state, &free, once_at(ADA, tomorrow()))
        .await
        .unwrap()
        .schedule
        .unwrap();
    let mut asking = Preset::AskBeforeSending.grant();
    asking.schedules = Schedules::All;
    let asker = connect(&state, owner, &asking).await;
    let change = |body: &str| {
        let mut change = update(schedule.id);
        change.body = Some(body.to_owned());
        change
    };

    let id = update_scheduled_message(&state, &asker, change("Changed"))
        .await
        .unwrap()
        .approval_id
        .unwrap();
    let approved = review::approve(&state, id, owner, None).await.unwrap();
    assert_eq!(approved.scheduled_message_id, Some(schedule.id));
    let changed = scheduled_messages::find(&state.pool, schedule.id, owner)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(changed.body, "Changed");
    assert_eq!(changed.connection_id, Some(free.connection().id));

    let id = update_scheduled_message(&state, &asker, change("Again"))
        .await
        .unwrap()
        .approval_id
        .unwrap();
    scheduled_messages::delete(&state.pool, schedule.id, owner)
        .await
        .unwrap();
    let Err(ReviewError::CantApprove(problems)) = review::approve(&state, id, owner, None).await
    else {
        panic!("can't approve")
    };
    assert_eq!(
        problems,
        [
            "The scheduled message this would change has been deleted, so there's nothing left to \
             change. Reject this request."
        ]
    );
    assert_eq!(
        approval(&state, id, owner).await.status,
        ApprovalStatus::Waiting
    );
}

#[sqlx::test]
async fn a_sealed_request_opens_only_for_its_own_approval(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    let (_, first) = asked_send(&state, owner, "First").await;
    let (_, second) = asked_send(&state, owner, "Second").await;
    let first = approval(&state, first, owner).await;
    let mut second = approval(&state, second, owner).await;
    second.payload.clone_from(&first.payload);
    assert!(review::open(&state.cipher, &first).is_ok());
    assert!(review::open(&state.cipher, &second).is_err());
}

#[sqlx::test]
async fn only_the_first_approval_in_an_hour_emails_the_owner(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    let (_, first) = asked_send(&state, owner, "First").await;
    let first = approval(&state, first, owner).await;
    assert!(review::should_email(&state.pool, &first).await.unwrap());
    let (_, second) = asked_send(&state, owner, "Second").await;
    let second = approval(&state, second, owner).await;
    assert!(!review::should_email(&state.pool, &second).await.unwrap());

    let other = user(&state.pool).await;
    let (_, theirs) = asked_send(&state, other, "Theirs").await;
    let theirs = approval(&state, theirs, other).await;
    assert!(review::should_email(&state.pool, &theirs).await.unwrap());
}

#[test]
fn every_input_field_carries_a_description() {
    fn check<T: schemars::JsonSchema>() {
        let schema = serde_json::to_value(schemars::schema_for!(T)).unwrap();
        let Some(properties) = schema["properties"].as_object() else {
            return;
        };
        for (name, property) in properties {
            assert!(
                property.get("description").is_some(),
                "{}.{name}: {property}",
                std::any::type_name::<T>()
            );
        }
    }
    check::<WhoAmIInput>();
    check::<FindChatsInput>();
    check::<SendInput>();
    check::<ListDeliveriesInput>();
    check::<GetApprovalInput>();
    check::<ListApprovalsInput>();
    check::<ScheduleInput>();
    check::<ListSchedulesInput>();
    check::<UpdateScheduleInput>();
    check::<DeleteScheduleInput>();
    check::<ListTemplatesInput>();
}

#[test]
fn operations_can_run_on_any_thread() {
    const fn send<T: Send>(_: &T) {}
    fn futures(state: &AppState, caller: &Caller) {
        send(&whoami(state, caller, WhoAmIInput::default()));
        send(&find_chats(
            state,
            caller,
            FindChatsInput {
                query: String::new(),
            },
        ));
        send(&send_message(state, caller, send_to(ADA, "")));
        send(&list_deliveries(
            state,
            caller,
            ListDeliveriesInput::default(),
        ));
        send(&get_approval(
            state,
            caller,
            GetApprovalInput {
                approval_id: Uuid::nil(),
            },
        ));
        send(&list_approvals(
            state,
            caller,
            ListApprovalsInput::default(),
        ));
        send(&schedule_message(state, caller, once_at(ADA, Utc::now())));
        send(&list_scheduled_messages(
            state,
            caller,
            ListSchedulesInput::default(),
        ));
        send(&update_scheduled_message(
            state,
            caller,
            update(Uuid::nil()),
        ));
        send(&delete_scheduled_message(
            state,
            caller,
            DeleteScheduleInput {
                scheduled_message_id: Uuid::nil(),
            },
        ));
        send(&list_templates(
            state,
            caller,
            ListTemplatesInput::default(),
        ));
        send(&review::approve(state, Uuid::nil(), Uuid::nil(), None));
        send(&review::reject(&state.pool, Uuid::nil(), Uuid::nil()));
    }
    let _ = futures;
}

/// Runs `calls` operations at once on a runtime of several threads, each with its own pool
/// connection, and gives how many succeeded and the pool for checking after
fn at_once<F, Fut>(
    pool_options: sqlx::postgres::PgPoolOptions,
    connect_options: sqlx::postgres::PgConnectOptions,
    grant: Grant,
    calls: usize,
    call: F,
) -> (usize, Uuid, PgPool)
where
    F: Fn(Arc<AppState>, Arc<Caller>) -> Fut + Send + 'static,
    Fut: Future<Output = bool> + Send + 'static,
{
    let checks = connect_options.clone();
    let (passed, owner) = std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move {
                let pool = pool_options
                    .max_connections(16)
                    .connect_with(connect_options)
                    .await
                    .unwrap();
                let state = Arc::new(state(pool.clone()));
                let owner = user(&pool).await;
                connected(&state, owner);
                let caller = Arc::new(connect(&state, owner, &grant).await);
                let start = Arc::new(tokio::sync::Barrier::new(calls));
                let mut tasks = tokio::task::JoinSet::new();
                for _ in 0..calls {
                    let (state, caller, start) = (state.clone(), caller.clone(), start.clone());
                    let call = call(state, caller);
                    tasks.spawn(async move {
                        start.wait().await;
                        call.await
                    });
                }
                let passed = tasks.join_all().await.into_iter().filter(|ok| *ok).count();
                pool.close().await;
                (passed, owner)
            })
    })
    .join()
    .unwrap();
    // A connection opened on the runtime above fails once it has shut down, so the caller's checks
    // get their own, opened on the caller's runtime
    let pool = sqlx::postgres::PgPoolOptions::new().connect_lazy_with(checks);
    (passed, owner, pool)
}

#[sqlx::test]
async fn sends_at_the_same_moment_pass_a_daily_limit_only_as_often_as_it_allows(
    pool_options: sqlx::postgres::PgPoolOptions,
    connect_options: sqlx::postgres::PgConnectOptions,
) {
    let grant = with_rule(
        Preset::SendWithoutAsking.grant(),
        ADA,
        SendLevel::Yes,
        Some(3),
    );
    let (passed, owner, pool) = at_once(
        pool_options,
        connect_options,
        grant,
        30,
        |state, caller| async move {
            send_message(&state, &caller, send_to(ADA, "Hello"))
                .await
                .is_ok()
        },
    );

    assert_eq!(passed, 3);
    assert_eq!(delivery_rows(&pool, owner).await.len(), 3);
}

#[sqlx::test]
async fn asks_at_the_same_moment_stop_at_the_waiting_cap(
    pool_options: sqlx::postgres::PgPoolOptions,
    connect_options: sqlx::postgres::PgConnectOptions,
) {
    let grant = Preset::AskBeforeSending.grant();
    let (passed, owner, pool) = at_once(
        pool_options,
        connect_options,
        grant,
        40,
        |state, caller| async move {
            send_message(&state, &caller, send_to(ADA, "Hello"))
                .await
                .is_ok_and(|sent| sent.outcome == SendOutcome::WaitingForApproval)
        },
    );

    assert_eq!(passed, crate::agents::WAITING_MAX as usize);
    let waiting = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM agent_approvals WHERE user_id = $1"#,
        owner
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(waiting, i64::from(crate::agents::WAITING_MAX));
}

#[sqlx::test]
async fn schedules_asked_for_at_the_same_moment_stop_at_the_waiting_cap(
    pool_options: sqlx::postgres::PgPoolOptions,
    connect_options: sqlx::postgres::PgConnectOptions,
) {
    let grant = Preset::AskBeforeSending.grant();
    let (passed, _, _) = at_once(
        pool_options,
        connect_options,
        grant,
        40,
        |state, caller| async move {
            schedule_message(&state, &caller, once_at(ADA, tomorrow()))
                .await
                .is_ok_and(|asked| asked.outcome == ScheduleOutcome::WaitingForApproval)
        },
    );

    assert_eq!(passed, crate::agents::WAITING_MAX as usize);
}

fn every(expression: &str) -> ScheduleInput {
    ScheduleInput {
        schedule_kind: ScheduleKind::Recurring,
        run_at: None,
        recurrence: Some(Recurrence::Cron {
            expression: expression.to_owned(),
        }),
        ..once_at(ADA, tomorrow())
    }
}

#[sqlx::test]
async fn an_agent_may_schedule_at_most_hourly(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    let caller = connect(&state, owner, &Preset::FullAccess.grant()).await;

    for expression in ["* * * * *", "*/30 * * * *", "0-5 9 * * *", "0,59 9 * * 1"] {
        match schedule_message(&state, &caller, every(expression)).await {
            Err(error @ OpError::TooFrequent(_)) => {
                assert_eq!(error.code(), "too_frequent");
                let problem = error.to_string();
                assert!(problem.contains("once an hour"), "{expression}: {problem}");
                assert!(problem.contains("web app"), "{expression}: {problem}");
            }
            other => panic!("{expression}: {other:?}"),
        }
    }
    for expression in ["0 * * * *", "0 9 * * *", "30 8,9 * * *"] {
        let made = schedule_message(&state, &caller, every(expression)).await;
        assert_eq!(
            made.unwrap().outcome,
            ScheduleOutcome::Created,
            "{expression}"
        );
    }
    let sun = ScheduleInput {
        recurrence: Some(
            serde_json::from_value(json!({
                "type": "sun", "event": "sunset", "offset_minutes": 10, "days": "* * *",
                "place": null,
            }))
            .unwrap(),
        ),
        timezone: Some("Asia/Dhaka".to_owned()),
        ..every("0 9 * * *")
    };
    assert_eq!(
        schedule_message(&state, &caller, sun)
            .await
            .unwrap()
            .outcome,
        ScheduleOutcome::Created
    );

    let hourly = schedule_message(&state, &caller, every("0 * * * *"))
        .await
        .unwrap()
        .schedule
        .unwrap();
    let denser = UpdateScheduleInput {
        recurrence: Some(Recurrence::Cron {
            expression: "*/10 * * * *".to_owned(),
        }),
        ..update(hourly.id)
    };
    assert!(matches!(
        update_scheduled_message(&state, &caller, denser).await,
        Err(OpError::TooFrequent(_))
    ));
}

#[sqlx::test]
async fn find_chats_is_limited_to_20_searches_a_minute(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    save_contact(&state, owner, ADA, "Ada Lovelace").await;
    let caller = connect(&state, owner, &Preset::AskBeforeSending.grant()).await;
    let search = || FindChatsInput {
        query: "ada".to_owned(),
    };

    for _ in 0..20 {
        find_chats(&state, &caller, search()).await.unwrap();
    }
    let error = find_chats(&state, &caller, search()).await.unwrap_err();
    assert!(matches!(error, OpError::TooOften(_)), "{error:?}");
    assert!(error.to_string().contains("Wait a minute"), "{error}");
    assert_eq!(
        last_activity(&state, &caller).await,
        (
            "find_chats".to_owned(),
            Outcome::Denied,
            Some("too_often".to_owned())
        )
    );

    // Each connection has its own
    let other = connect(&state, owner, &Preset::AskBeforeSending.grant()).await;
    find_chats(&state, &other, search()).await.unwrap();
}

#[sqlx::test]
async fn a_chats_rule_holds_for_every_spelling_of_its_number(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    connected(&state, owner);
    let grant = with_rule(Preset::FullAccess.grant(), ADA, SendLevel::No, None);
    let caller = connect(&state, owner, &grant).await;

    for spelling in [format!("00{ADA}"), format!("+{ADA}"), format!("+00 {ADA}")] {
        refused_with(
            send_message(&state, &caller, send_to(&spelling, "Hello")).await,
            "sending_off",
        );
    }
    assert!(matches!(
        send_message(&state, &caller, send_to("01700000001", "Hello")).await,
        Err(OpError::Invalid(_))
    ));
}

#[sqlx::test]
async fn a_group_rule_cant_be_dodged_by_a_second_spelling(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    connected(&state, owner);
    let grant = with_rule(Preset::FullAccess.grant(), GROUP, SendLevel::No, None);
    let caller = connect(&state, owner, &grant).await;

    let padded = SendInput {
        recipient: format!(" {GROUP} "),
        ..send_to(GROUP, "Hello")
    };
    refused_with(send_message(&state, &caller, padded).await, "sending_off");
    let device = GROUP.replace("@g.us", ":1@g.us");
    let agent = GROUP.replace("@g.us", ".1@g.us");
    for spelling in [device, agent] {
        assert!(
            matches!(
                send_message(&state, &caller, send_to(&spelling, "Hello")).await,
                Err(OpError::Invalid(_))
            ),
            "{spelling}"
        );
    }
}

/// `count` one-time schedules of `connection`'s, made straight in the database
async fn schedules_of(
    state: &AppState,
    owner: Uuid,
    connection: Option<Uuid>,
    count: i32,
) -> Vec<Uuid> {
    sqlx::query(
        "INSERT INTO scheduled_messages (user_id, connection_id, recipient_type, recipient, body, \
         schedule_kind, run_at, timezone, next_run_at) \
         SELECT $1, $2, 'contact', $3, 'Hello', 'once', now() + interval '1 day', 'UTC', \
             now() + interval '1 day' \
         FROM generate_series(1, $4)",
    )
    .bind(owner)
    .bind(connection)
    .bind(ADA)
    .bind(count)
    .execute(&state.pool)
    .await
    .unwrap();
    sqlx::query_scalar(
        "SELECT id FROM scheduled_messages WHERE user_id = $1 \
         AND connection_id IS NOT DISTINCT FROM $2 ORDER BY created_at, id",
    )
    .bind(owner)
    .bind(connection)
    .fetch_all(&state.pool)
    .await
    .unwrap()
}

async fn belongs_to(state: &AppState, id: Uuid, owner: Uuid) -> Option<Uuid> {
    scheduled_messages::find(&state.pool, id, owner)
        .await
        .unwrap()
        .unwrap()
        .connection_id
}

/// Makes `id` due and runs the scheduler once, giving the delivery it made, if any
async fn fire(state: &AppState, id: Uuid) -> Option<(bool, Option<Uuid>)> {
    sqlx::query(
        "UPDATE scheduled_messages SET enabled = true, next_run_at = now() - interval '1 minute' \
         WHERE id = $1",
    )
    .bind(id)
    .execute(&state.pool)
    .await
    .unwrap();
    crate::scheduler::enqueue_due(&state.pool, &state.cipher, Utc::now(), true)
        .await
        .unwrap();
    sqlx::query_as(
        "SELECT skipped, connection_id FROM deliveries WHERE scheduled_message_id = $1 \
         ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .unwrap()
}

#[sqlx::test]
async fn a_schedule_an_agent_changes_becomes_that_agents(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    let agent = connect(&state, owner, &Preset::FullAccess.grant()).await;
    let other = connect(&state, owner, &Preset::FullAccess.grant()).await;
    let agent_id = agent.connection().id;
    let [owners] = schedules_of(&state, owner, None, 1).await[..] else {
        panic!("one schedule")
    };

    let mut change = update(owners);
    change.body = Some("Changed by the agent".to_owned());
    let changed = update_scheduled_message(&state, &agent, change)
        .await
        .unwrap();
    assert!(
        changed.message.contains("now belongs"),
        "{}",
        changed.message
    );
    let changed = changed.schedule.unwrap();
    assert!(changed.belongs_to_this_connection);
    assert_eq!(belongs_to(&state, owners, owner).await, Some(agent_id));
    assert_eq!(
        agent_connections::schedules(&state.pool, agent_id, owner)
            .await
            .unwrap(),
        1
    );

    // Its runs are now the agent's: checked against its pause, and counted as its sends
    agent_connections::pause(&state.pool, agent_id, owner)
        .await
        .unwrap();
    assert_eq!(fire(&state, owners).await, Some((true, Some(agent_id))));
    agent_connections::resume(&state.pool, agent_id, owner)
        .await
        .unwrap();
    assert_eq!(fire(&state, owners).await, Some((false, Some(agent_id))));
    let counts = agent_connections::counts(&state.pool, agent_id, RecipientType::Contact, ADA)
        .await
        .unwrap();
    assert_eq!(counts.connection_sends, 1);

    // Another connection only switching it on takes it over
    sqlx::query(
        "UPDATE scheduled_messages SET enabled = false, run_at = now() + interval '1 day' \
         WHERE id = $1",
    )
    .bind(owners)
    .execute(&state.pool)
    .await
    .unwrap();
    let mut on = update(owners);
    on.enabled = Some(true);
    update_scheduled_message(&state, &other, on).await.unwrap();
    let other_id = other.connection().id;
    assert_eq!(belongs_to(&state, owners, owner).await, Some(other_id));

    let revoked = agent_connections::delete(&state.pool, other_id, owner, true)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(revoked.schedules_deleted, 1);
    assert!(
        scheduled_messages::find(&state.pool, owners, owner)
            .await
            .unwrap()
            .is_none()
    );
}

#[sqlx::test]
async fn switching_off_or_deleting_takes_nothing_over(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    let agent = connect(&state, owner, &Preset::FullAccess.grant()).await;
    let ids = schedules_of(&state, owner, None, 2).await;

    let mut off = update(ids[0]);
    off.enabled = Some(false);
    update_scheduled_message(&state, &agent, off).await.unwrap();
    assert_eq!(belongs_to(&state, ids[0], owner).await, None);
    delete_scheduled_message(
        &state,
        &agent,
        DeleteScheduleInput {
            scheduled_message_id: ids[1],
        },
    )
    .await
    .unwrap();
}

#[sqlx::test]
async fn a_grant_narrowed_after_the_key_was_looked_up_decides(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    connected(&state, owner);
    save_contact(&state, owner, ADA, "Ada Lovelace").await;
    // Built while it had full access, as when its key was looked up
    let stale = connect(&state, owner, &Preset::FullAccess.grant()).await;
    let id = stale.connection().id;
    let theirs = schedules_of(&state, owner, None, 1).await[0];
    agent_connections::set_grant(&state.pool, id, owner, &Preset::ViewOnly.grant())
        .await
        .unwrap();

    refused_with(
        send_message(&state, &stale, send_to(ADA, "Hi")).await,
        "sending_off",
    );
    refused_with(
        schedule_message(&state, &stale, once_at(ADA, tomorrow())).await,
        "schedules_off",
    );
    let mut change = update(theirs);
    change.body = Some("Changed".to_owned());
    refused_with(
        update_scheduled_message(&state, &stale, change).await,
        "schedules_off",
    );
    let mut off = update(theirs);
    off.enabled = Some(false);
    refused_with(
        update_scheduled_message(&state, &stale, off).await,
        "schedules_off",
    );
    refused_with(
        delete_scheduled_message(
            &state,
            &stale,
            DeleteScheduleInput {
                scheduled_message_id: theirs,
            },
        )
        .await,
        "schedules_off",
    );
    assert_eq!(delivery_rows(&state.pool, owner).await.len(), 0);

    sqlx::query(r#"UPDATE agent_connections SET "grant" = '{"version": 1}' WHERE id = $1"#)
        .bind(id)
        .execute(&state.pool)
        .await
        .unwrap();
    refused_with(
        send_message(&state, &stale, send_to(ADA, "Hi")).await,
        "grant_unreadable",
    );
}

#[sqlx::test]
async fn a_connection_has_at_most_50_schedules(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    let agent = connect(&state, owner, &Preset::FullAccess.grant()).await;
    let agent_id = agent.connection().id;
    schedules_of(&state, owner, Some(agent_id), 49).await;
    let owners = schedules_of(&state, owner, None, 1).await[0];

    let fiftieth = schedule_message(&state, &agent, once_at(ADA, tomorrow()))
        .await
        .unwrap();
    assert_eq!(fiftieth.outcome, ScheduleOutcome::Created);
    let over = schedule_message(&state, &agent, once_at(ADA, tomorrow())).await;
    refused_with(over, "too_many_schedules");
    let (tool, outcome, reason) = last_activity(&state, &agent).await;
    assert_eq!(
        (tool.as_str(), outcome, reason.as_deref()),
        (
            "schedule_message",
            Outcome::Denied,
            Some("too_many_schedules")
        )
    );

    // Taking over the owner's would make 51; changing one of its own makes none
    let mut change = update(owners);
    change.body = Some("Mine now".to_owned());
    refused_with(
        update_scheduled_message(&state, &agent, change).await,
        "too_many_schedules",
    );
    assert_eq!(belongs_to(&state, owners, owner).await, None);
    let mut own = update(fiftieth.schedule.unwrap().id);
    own.body = Some("Still mine".to_owned());
    update_scheduled_message(&state, &agent, own).await.unwrap();

    delete_scheduled_message(
        &state,
        &agent,
        DeleteScheduleInput {
            scheduled_message_id: owners,
        },
    )
    .await
    .unwrap();
    let mine = schedules_of(&state, owner, Some(agent_id), 0).await;
    delete_scheduled_message(
        &state,
        &agent,
        DeleteScheduleInput {
            scheduled_message_id: mine[0],
        },
    )
    .await
    .unwrap();
    schedule_message(&state, &agent, once_at(ADA, tomorrow()))
        .await
        .unwrap();
}

#[sqlx::test]
async fn approvals_count_toward_the_50_and_the_owner_may_go_past(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    let asker = connect(&state, owner, &Preset::AskBeforeSending.grant()).await;
    let asker_id = asker.connection().id;
    schedules_of(&state, owner, Some(asker_id), 48).await;

    let first = schedule_message(&state, &asker, once_at(ADA, tomorrow()))
        .await
        .unwrap();
    let second = schedule_message(&state, &asker, once_at(ADA, tomorrow()))
        .await
        .unwrap();
    assert_eq!(second.outcome, ScheduleOutcome::WaitingForApproval);
    refused_with(
        schedule_message(&state, &asker, once_at(ADA, tomorrow())).await,
        "too_many_schedules",
    );

    // 49 now, one waiting: the owner's yes isn't counted again
    schedules_of(&state, owner, Some(asker_id), 1).await;
    for asked in [first, second] {
        review::approve(&state, asked.approval_id.unwrap(), owner, None)
            .await
            .unwrap();
    }
    assert_eq!(
        agent_connections::schedules(&state.pool, asker_id, owner)
            .await
            .unwrap(),
        51
    );
}

/// An agent that asks before changing any schedule, and the approval for its change to `id`'s text
async fn asked_update(state: &AppState, owner: Uuid, id: Uuid) -> (Caller, Uuid) {
    let mut asking = Preset::AskBeforeSending.grant();
    asking.schedules = Schedules::All;
    let asker = connect(state, owner, &asking).await;
    let mut change = update(id);
    change.body = Some("The agent's text".to_owned());
    let result = update_scheduled_message(state, &asker, change)
        .await
        .unwrap();
    assert_eq!(result.outcome, ScheduleOutcome::WaitingForApproval);
    (asker, result.approval_id.unwrap())
}

#[track_caller]
fn changed_since(result: Result<agent_approvals::Approval, ReviewError>) {
    let Err(ReviewError::CantApprove(problems)) = result else {
        panic!("the approval should have been refused as changed since")
    };
    assert_eq!(
        problems,
        [
            "The scheduled message was changed after the agent asked, and approving would undo \
             that change. Reject this request, and have the agent ask again if it's still wanted."
        ]
    );
}

#[sqlx::test]
async fn approving_a_change_is_refused_once_the_schedule_changed_since(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    let [switched, edited, untouched] = schedules_of(&state, owner, None, 3).await[..] else {
        panic!("three schedules")
    };

    // The owner switched it off after the agent asked
    let (_, id) = asked_update(&state, owner, switched).await;
    scheduled_messages::set_enabled(&state.pool, switched, owner, false, None)
        .await
        .unwrap();
    changed_since(review::approve(&state, id, owner, None).await);
    let still = scheduled_messages::find(&state.pool, switched, owner)
        .await
        .unwrap()
        .unwrap();
    assert!(!still.enabled);
    assert_eq!(still.body, "Hello");
    assert_eq!(
        approval(&state, id, owner).await.status,
        ApprovalStatus::Waiting
    );
    review::reject(&state.pool, id, owner).await.unwrap();

    // The owner edited it after the agent asked
    let (_, id) = asked_update(&state, owner, edited).await;
    sqlx::query(
        "UPDATE scheduled_messages SET body = 'The owner''s text', updated_at = now() \
         WHERE id = $1",
    )
    .bind(edited)
    .execute(&state.pool)
    .await
    .unwrap();
    changed_since(review::approve(&state, id, owner, Some("Edited".to_owned())).await);
    let still = scheduled_messages::find(&state.pool, edited, owner)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(still.body, "The owner's text");
    review::reject(&state.pool, id, owner).await.unwrap();

    // Nothing changed in between
    let (_, id) = asked_update(&state, owner, untouched).await;
    review::approve(&state, id, owner, None).await.unwrap();
    let changed = scheduled_messages::find(&state.pool, untouched, owner)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(changed.body, "The agent's text");
    assert!(changed.enabled);
}

#[sqlx::test]
async fn a_recurring_schedule_running_doesnt_count_as_a_change(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    let free = connect(&state, owner, &Preset::FullAccess.grant()).await;
    let hourly = schedule_message(&state, &free, every("0 * * * *"))
        .await
        .unwrap()
        .schedule
        .unwrap();
    let (_, id) = asked_update(&state, owner, hourly.id).await;

    fire(&state, hourly.id).await.unwrap();
    review::approve(&state, id, owner, None).await.unwrap();
}

#[sqlx::test]
async fn a_change_asked_for_before_the_time_was_kept_opens_but_cant_be_approved(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    let schedule = schedules_of(&state, owner, None, 1).await[0];
    let (_, id) = asked_update(&state, owner, schedule).await;

    // Sealed as version 1 sealed it: no version 2 field
    let waiting = approval(&state, id, owner).await;
    let request = serde_json::to_value(review::open(&state.cipher, &waiting).unwrap()).unwrap();
    let mut old = request.clone();
    old.as_object_mut().unwrap().remove("schedule_updated_at");
    let plain = serde_json::to_vec(&json!({ "version": 1, "request": old })).unwrap();
    let context = [b"agent_approval:".as_slice(), id.as_bytes()].concat();
    let sealed = state.cipher.encrypt(&plain, &context).unwrap();
    sqlx::query("UPDATE agent_approvals SET payload = $2 WHERE id = $1")
        .bind(id)
        .bind(&sealed)
        .execute(&state.pool)
        .await
        .unwrap();

    let waiting = approval(&state, id, owner).await;
    let Request::ScheduleUpdate {
        schedule_updated_at,
        ..
    } = review::open(&state.cipher, &waiting).unwrap()
    else {
        panic!("an update")
    };
    assert_eq!(schedule_updated_at, None);
    let Err(ReviewError::CantApprove(problems)) = review::approve(&state, id, owner, None).await
    else {
        panic!("can't approve")
    };
    assert!(problems[0].contains("older version"), "{problems:?}");
    review::reject(&state.pool, id, owner).await.unwrap();

    // Version 2 must carry the time
    let plain = serde_json::to_vec(&json!({ "version": 2, "request": old })).unwrap();
    let mut forged = waiting;
    forged.payload = state.cipher.encrypt(&plain, &context).unwrap();
    assert!(review::open(&state.cipher, &forged).is_err());
}

#[sqlx::test]
async fn a_change_that_changes_nothing_writes_nothing(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    let agent = connect(&state, owner, &Preset::FullAccess.grant()).await;
    let other = connect(&state, owner, &Preset::FullAccess.grant()).await;
    let mut asking = Preset::AskBeforeSending.grant();
    asking.schedules = Schedules::All;
    let asker = connect(&state, owner, &asking).await;
    let owners = schedules_of(&state, owner, None, 1).await[0];
    let others = schedules_of(&state, owner, Some(other.connection().id), 1).await[0];
    let stored = |id| {
        let pool = state.pool.clone();
        async move {
            scheduled_messages::find(&pool, id, owner)
                .await
                .unwrap()
                .unwrap()
        }
    };
    let held = |caller: &Caller| {
        let pool = state.pool.clone();
        let id = caller.connection().id;
        async move { agent_connections::held_schedules(&pool, id).await.unwrap() }
    };

    for (id, belongs) in [(owners, None), (others, Some(other.connection().id))] {
        let before = stored(id).await;
        // Equal once normalized
        let same = || UpdateScheduleInput {
            enabled: Some(true),
            body: Some(format!("  {}  ", before.body)),
            recipient: Some(format!("+{}", before.recipient)),
            run_at: before.run_at,
            ..update(id)
        };
        for caller in [&agent, &asker] {
            let answer = update_scheduled_message(&state, caller, same())
                .await
                .unwrap();
            assert_eq!(answer.outcome, ScheduleOutcome::Unchanged);
            assert!(answer.approval_id.is_none());
            let shown = answer.schedule.unwrap();
            assert_eq!(shown.id, id);
            assert!(!shown.belongs_to_this_connection);
            assert_eq!(
                last_activity(&state, caller).await,
                (
                    "update_scheduled_message".to_owned(),
                    Outcome::Allowed,
                    None
                )
            );
        }
        assert_eq!(stored(id).await.updated_at, before.updated_at);
        assert_eq!(stored(id).await.connection_id, belongs);

        // Switching off one that's off changes nothing either
        set_off(&state, id, owner).await;
        let off = stored(id).await;
        let mut again = update(id);
        again.enabled = Some(false);
        let answer = update_scheduled_message(&state, &agent, again)
            .await
            .unwrap();
        assert_eq!(answer.outcome, ScheduleOutcome::Unchanged);

        let after = stored(id).await;
        assert_eq!(after.connection_id, belongs);
        assert_eq!(after.updated_at, off.updated_at);
        assert_eq!(after.body, before.body);
    }
    assert_eq!(held(&agent).await, 0);
    assert_eq!(held(&asker).await, 0);
    assert_eq!(held(&other).await, 1);
    let approvals: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_approvals")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(approvals, 0);
}

async fn set_off(state: &AppState, id: Uuid, owner: Uuid) {
    scheduled_messages::set_enabled(&state.pool, id, owner, false, None)
        .await
        .unwrap();
}

#[sqlx::test]
async fn an_owners_edit_made_while_an_agent_waited_for_the_lock_survives(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    let agent = connect(&state, owner, &Preset::FullAccess.grant()).await;
    let agent_id = agent.connection().id;
    let schedule = schedules_of(&state, owner, None, 1).await[0];

    // Holding the connection's lock keeps the agent between its first read and its lock
    let mut held = state.pool.begin().await.unwrap();
    agent_connections::lock(&mut *held, agent_id)
        .await
        .unwrap()
        .unwrap();
    let mut change = update(schedule);
    change.body = Some("The agent's text".to_owned());
    let call = tokio::spawn({
        let state = state.clone();
        async move { update_scheduled_message(&state, &agent, change).await }
    });
    // Until the agent waits on the lock, which the owner's edit then follows
    let waiting = "SELECT count(*) FROM pg_stat_activity \
                   WHERE datname = current_database() AND wait_event_type = 'Lock' \
                       AND query LIKE '%FROM agent_connections%'";
    while sqlx::query_scalar::<_, i64>(waiting)
        .fetch_one(&state.pool)
        .await
        .unwrap()
        == 0
    {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    set_off(&state, schedule, owner).await;
    held.commit().await.unwrap();

    let changed = call.await.unwrap().unwrap().schedule.unwrap();
    assert_eq!(changed.body, "The agent's text");
    assert!(!changed.enabled, "the owner's switch-off survives");
    assert!(changed.belongs_to_this_connection);
}

#[sqlx::test]
async fn whoami_counts_the_schedules_held_against_the_cap(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    let asker = connect(&state, owner, &Preset::AskBeforeSending.grant()).await;
    schedules_of(&state, owner, Some(asker.connection().id), 2).await;
    schedule_message(&state, &asker, once_at(ADA, tomorrow()))
        .await
        .unwrap();

    let me = whoami(&state, &asker, WhoAmIInput {}).await.unwrap();
    assert_eq!(me.limits.schedules, 3);
    assert_eq!(me.limits.schedules_max, 50);
    let summary = me.summary.join(" ");
    assert!(
        summary.contains("at most 50 scheduled messages"),
        "{summary}"
    );
    assert!(summary.contains("once an hour"), "{summary}");
    assert!(summary.contains("scheduled or changed"), "{summary}");
}

#[sqlx::test]
async fn the_selected_chats_bound_what_all_shows(pool: PgPool) {
    let state = state(pool);
    let owner = user(&state.pool).await;
    connected(&state, owner);
    // The owner's own history and schedules, one each to three chats
    for (kind, recipient) in [("contact", ADA), ("contact", STRANGER), ("group", GROUP)] {
        sqlx::query(
            "INSERT INTO deliveries (user_id, recipient_type, recipient, body, status) \
             VALUES ($1, $2, $3, 'By hand', 'sent')",
        )
        .bind(owner)
        .bind(kind)
        .bind(recipient)
        .execute(&state.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO scheduled_messages (user_id, recipient_type, recipient, body, \
             schedule_kind, run_at, timezone, next_run_at) \
             VALUES ($1, $2, $3, 'Hello', 'once', now() + interval '1 day', 'UTC', \
                 now() + interval '1 day')",
        )
        .bind(owner)
        .bind(kind)
        .bind(recipient)
        .execute(&state.pool)
        .await
        .unwrap();
    }

    let mut selected = Preset::FullAccess.grant();
    selected.chats.scope = Scope::Selected;
    let selected = with_rule(selected, ADA, SendLevel::Yes, None);
    let agent = connect(&state, owner, &selected).await;
    send_message(&state, &agent, send_to(ADA, "Mine"))
        .await
        .unwrap();

    // Its own, then the owner's to the chat it may reach; nothing of the other two chats
    let history = list_deliveries(&state, &agent, ListDeliveriesInput::default())
        .await
        .unwrap();
    let seen: Vec<_> = history
        .deliveries
        .iter()
        .map(|delivery| (delivery.recipient.as_str(), delivery.by_this_connection))
        .collect();
    assert_eq!(seen, [(ADA, true), (ADA, false)]);
    let schedules = list_scheduled_messages(&state, &agent, ListSchedulesInput::default())
        .await
        .unwrap();
    let to: Vec<_> = schedules
        .schedules
        .iter()
        .map(|schedule| schedule.recipient.as_str())
        .collect();
    assert_eq!(to, [ADA]);
    assert!(
        crate::agents::ops::account::summary(agent.grant())
            .iter()
            .any(|line| line.contains("any to the chats the owner selected")),
    );

    // A schedule to a chat outside the selection answers as if it weren't there
    let outside: Uuid = sqlx::query_scalar(
        "SELECT id FROM scheduled_messages WHERE user_id = $1 AND recipient = $2",
    )
    .bind(owner)
    .bind(GROUP)
    .fetch_one(&state.pool)
    .await
    .unwrap();
    let mut change = update(outside);
    change.body = Some("Changed".to_owned());
    let mut off = update(outside);
    off.enabled = Some(false);
    for input in [change, off] {
        assert!(matches!(
            update_scheduled_message(&state, &agent, input).await,
            Err(OpError::NotFound(_))
        ));
    }
    assert!(matches!(
        delete_scheduled_message(
            &state,
            &agent,
            DeleteScheduleInput {
                scheduled_message_id: outside
            }
        )
        .await,
        Err(OpError::NotFound(_))
    ));
    let untouched: (String, bool) =
        sqlx::query_as("SELECT body, enabled FROM scheduled_messages WHERE id = $1")
            .bind(outside)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(untouched, ("Hello".to_owned(), true));

    // With every chat in reach, "all" is everything
    let everywhere = connect(&state, owner, &Preset::FullAccess.grant()).await;
    let history = list_deliveries(&state, &everywhere, ListDeliveriesInput::default())
        .await
        .unwrap();
    assert_eq!(history.deliveries.len(), 4);
    let schedules = list_scheduled_messages(&state, &everywhere, ListSchedulesInput::default())
        .await
        .unwrap();
    assert_eq!(schedules.schedules.len(), 3);

    // "Its own" stays its own, whatever chats are selected
    let mut own = selected;
    own.deliveries = Deliveries::Own;
    own.schedules = Schedules::Own;
    let own = connect(&state, owner, &own).await;
    let history = list_deliveries(&state, &own, ListDeliveriesInput::default())
        .await
        .unwrap();
    assert!(history.deliveries.is_empty());
    let schedules = list_scheduled_messages(&state, &own, ListSchedulesInput::default())
        .await
        .unwrap();
    assert!(schedules.schedules.is_empty());
}

mod reading {
    use super::*;
    use crate::{
        agents::ops::inbox::{
            ListChatsInput, ReadMessagesInput, SearchMessagesInput, list_chats, read_messages,
            search_messages,
        },
        db::inbox_messages,
        whatsapp::inbox::{Body, Kind, seal_body},
    };

    const BOB: &str = "8801700000002";

    /// Reads Ada's chat and the group; may use Bob's chat without reading it
    fn reader() -> Grant {
        let mut grant = Preset::AskBeforeSending.grant();
        grant.chats.scope = Scope::Selected;
        for chat in [ADA, GROUP, BOB] {
            grant = with_rule(grant, chat, SendLevel::Ask, None);
        }
        for rule in &mut grant.chats.rules {
            rule.read = rule.chat.id != BOB;
            rule.chat.name = None;
        }
        grant
    }

    async fn keep(
        state: &AppState,
        owner: Uuid,
        chat: &str,
        id: &str,
        minutes_ago: i64,
        text: &str,
    ) {
        let account = whatsapp_accounts::find_or_create(&state.pool, owner)
            .await
            .unwrap();
        let recipient_type = if chat.ends_with("@g.us") {
            RecipientType::Group
        } else {
            RecipientType::Contact
        };
        let from_me = id.starts_with("ME");
        let body = Body {
            sender_jid: format!("{ADA}@s.whatsapp.net"),
            sender_number: (!from_me).then(|| ADA.to_owned()),
            sender_name: (!from_me).then(|| "ada.l".to_owned()),
            kind: Kind::Text,
            text: Some(text.to_owned()),
            file_name: None,
            reply_to: None,
            mentions: Vec::new(),
            edited: false,
        };
        let sealed =
            seal_body(&state.cipher, account.id, (recipient_type, chat), id, &body).unwrap();
        inbox_messages::save(
            &state.pool,
            inbox_messages::NewMessage {
                account_id: account.id,
                chat_type: recipient_type.as_str(),
                chat_id: chat,
                message_id: id,
                from_me,
                sent_at: Utc::now() - TimeDelta::minutes(minutes_ago),
                expires_at: None,
                body: &sealed,
            },
        )
        .await
        .unwrap();
    }

    fn read(recipient: &str, before: Option<&str>, limit: Option<u32>) -> ReadMessagesInput {
        ReadMessagesInput {
            recipient_type: if recipient.ends_with("@g.us") {
                RecipientType::Group
            } else {
                RecipientType::Contact
            },
            recipient: recipient.to_owned(),
            before: before.map(str::to_owned),
            limit,
        }
    }

    fn code<T: std::fmt::Debug>(result: Result<T, OpError>) -> &'static str {
        match result {
            Err(OpError::Refused { code, .. }) => code,
            other => panic!("{other:?}"),
        }
    }

    #[sqlx::test]
    async fn reads_a_chat_oldest_first_and_pages_back(pool: PgPool) {
        let state = state(pool);
        let owner = user(&state.pool).await;
        save_contact(&state, owner, ADA, "Ada Lovelace").await;
        let caller = connect(&state, owner, &reader()).await;
        keep(&state, owner, ADA, "A1", 30, "Hello").await;
        keep(&state, owner, ADA, "ME2", 20, "Hi Ada").await;
        keep(&state, owner, ADA, "A3", 10, "Can we meet at 5?").await;

        let page = read_messages(&state, &caller, read(ADA, None, Some(2)))
            .await
            .unwrap();
        let ids: Vec<_> = page
            .messages
            .iter()
            .map(|message| message.id.as_str())
            .collect();
        assert_eq!(ids, ["ME2", "A3"]);
        assert_eq!(page.chat_name.as_deref(), Some("Ada Lovelace"));
        assert!(page.has_more);
        assert_eq!(page.next_before.as_deref(), Some("ME2"));
        assert!(page.messages[0].from_me && page.messages[0].sender.is_none());
        let sender = page.messages[1].sender.as_ref().unwrap();
        // The saved name wins over the name the sender gave themselves
        assert_eq!(sender.name.as_deref(), Some("Ada Lovelace"));
        assert_eq!(sender.number.as_deref(), Some(ADA));
        assert_eq!(page.messages[1].text.as_deref(), Some("Can we meet at 5?"));

        let older = read_messages(&state, &caller, read(ADA, Some("ME2"), Some(2)))
            .await
            .unwrap();
        assert_eq!(older.messages.len(), 1);
        assert_eq!(older.messages[0].id, "A1");
        assert!(!older.has_more && older.next_before.is_none());

        let gone = read_messages(&state, &caller, read(ADA, Some("NOPE"), None)).await;
        assert!(matches!(gone, Err(OpError::NotFound(_))), "{gone:?}");
    }

    #[sqlx::test]
    async fn reading_stays_inside_the_grant(pool: PgPool) {
        let state = state(pool);
        let owner = user(&state.pool).await;
        let caller = connect(&state, owner, &reader()).await;
        keep(&state, owner, ADA, "A1", 30, "The invoice is ready").await;
        keep(&state, owner, GROUP, "G1", 20, "Invoice meeting moved").await;
        // Kept for another connection that reads Bob's chat
        keep(&state, owner, BOB, "B1", 10, "Private invoice talk").await;

        assert_eq!(
            code(read_messages(&state, &caller, read(BOB, None, None)).await),
            "reading_off"
        );
        assert_eq!(
            code(read_messages(&state, &caller, read(STRANGER, None, None)).await),
            "out_of_scope"
        );

        let chats = list_chats(&state, &caller, ListChatsInput::default())
            .await
            .unwrap();
        let listed: Vec<_> = chats
            .chats
            .iter()
            .map(|chat| chat.recipient.as_str())
            .collect();
        assert_eq!(listed, [GROUP, ADA]);
        assert_eq!(chats.chats[0].send, SendLevel::Ask);

        let everywhere = SearchMessagesInput {
            query: "INVOICE".to_owned(),
            recipient_type: None,
            recipient: None,
        };
        let found = search_messages(&state, &caller, everywhere).await.unwrap();
        let ids: Vec<_> = found
            .matches
            .iter()
            .map(|hit| hit.message.id.as_str())
            .collect();
        assert_eq!(ids, ["G1", "A1"]);
        assert!(found.searched_all && !found.more);

        let in_bob = SearchMessagesInput {
            query: "invoice".to_owned(),
            recipient_type: Some(RecipientType::Contact),
            recipient: Some(BOB.to_owned()),
        };
        assert_eq!(
            code(search_messages(&state, &caller, in_bob).await),
            "reading_off"
        );

        // Another owner's connection sees none of it
        let other = user(&state.pool).await;
        let mut all = Preset::AskBeforeSending.grant();
        all.chats.default.read = true;
        let stranger = connect(&state, other, &all).await;
        let theirs = list_chats(&state, &stranger, ListChatsInput::default())
            .await
            .unwrap();
        assert!(theirs.chats.is_empty());
        let none = read_messages(&state, &stranger, read(ADA, None, None))
            .await
            .unwrap();
        assert!(none.messages.is_empty());
    }

    #[sqlx::test]
    async fn a_connection_that_reads_nothing_and_a_server_with_reading_off_are_refused(
        pool: PgPool,
    ) {
        let mut state = state(pool);
        let owner = user(&state.pool).await;
        let sender = connect(&state, owner, &Preset::AskBeforeSending.grant()).await;
        assert_eq!(
            code(list_chats(&state, &sender, ListChatsInput::default()).await),
            "reading_off"
        );
        assert_eq!(
            code(read_messages(&state, &sender, read(ADA, None, None)).await),
            "reading_off"
        );

        state.agent_reading = false;
        let caller = connect(&state, owner, &reader()).await;
        keep(&state, owner, ADA, "A1", 30, "Hello").await;
        assert_eq!(
            code(read_messages(&state, &caller, read(ADA, None, None)).await),
            "reading_unavailable"
        );
        assert_eq!(
            code(list_chats(&state, &caller, ListChatsInput::default()).await),
            "reading_unavailable"
        );
        let whoami = whoami(&state, &caller, WhoAmIInput {}).await.unwrap();
        assert!(whoami.summary.last().unwrap().contains("turned off"));
    }

    #[sqlx::test]
    async fn a_reply_quotes_a_kept_message_of_a_chat_it_may_read(pool: PgPool) {
        let mut state = state(pool);
        let owner = user(&state.pool).await;
        connected(&state, owner);
        save_contact(&state, owner, ADA, "Ada Lovelace").await;
        save_contact(&state, owner, BOB, "Bob").await;
        let mut grant = reader();
        for rule in &mut grant.chats.rules {
            rule.send = SendLevel::Yes;
        }
        let caller = connect(&state, owner, &grant).await;
        keep(&state, owner, ADA, "A1", 10, "Can we meet at 5?").await;
        keep(&state, owner, BOB, "B1", 10, "Hello").await;
        let reply = |recipient: &str, id: &str| SendInput {
            reply_to: Some(id.to_owned()),
            ..send_to(recipient, "Yes, see you then")
        };

        let sent = send_message(&state, &caller, reply(ADA, "A1"))
            .await
            .unwrap();
        assert_eq!(sent.outcome, SendOutcome::Queued);
        let quoted: Option<String> =
            sqlx::query_scalar("SELECT reply_to FROM deliveries WHERE id = $1")
                .bind(sent.delivery_id.unwrap())
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(quoted.as_deref(), Some("A1"));

        // Only a message of the same chat, and only where it may read
        let elsewhere = send_message(&state, &caller, reply(ADA, "B1")).await;
        assert!(
            matches!(elsewhere, Err(OpError::NotFound(_))),
            "{elsewhere:?}"
        );
        let unread = send_message(&state, &caller, reply(BOB, "B1"))
            .await
            .unwrap_err();
        assert!(matches!(unread, OpError::Invalid(_)), "{unread:?}");
        assert!(unread.to_string().contains("read this chat"), "{unread}");

        // A reply that waits for approval keeps what it quotes
        let asking = connect(&state, owner, &reader()).await;
        let waiting = send_message(&state, &asking, reply(ADA, "A1"))
            .await
            .unwrap();
        assert_eq!(waiting.outcome, SendOutcome::WaitingForApproval);
        let approved = review::approve(&state, waiting.approval_id.unwrap(), owner, None).await;
        let delivery = approved.unwrap().delivery_id.unwrap();
        let quoted: Option<String> =
            sqlx::query_scalar("SELECT reply_to FROM deliveries WHERE id = $1")
                .bind(delivery)
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(quoted.as_deref(), Some("A1"));

        // No chat's setting can turn it back on
        state.agent_reading = false;
        let off = send_message(&state, &caller, reply(ADA, "A1"))
            .await
            .unwrap_err();
        assert!(matches!(off, OpError::Invalid(_)), "{off:?}");
        assert!(
            off.to_string().contains("on this Whatomato server"),
            "{off}"
        );
    }

    #[sqlx::test]
    async fn a_tag_names_who_it_tags_and_whether_that_is_the_owner(pool: PgPool) {
        const OWN: &str = "8801700000005";
        let state = state(pool);
        let owner = user(&state.pool).await;
        save_contact(&state, owner, ADA, "Ada Lovelace").await;
        let caller = connect(&state, owner, &reader()).await;
        let account = whatsapp_accounts::find_or_create(&state.pool, owner)
            .await
            .unwrap();
        sqlx::query("UPDATE whatsapp_accounts SET phone_number = $1 WHERE id = $2")
            .bind(OWN)
            .bind(account.id)
            .execute(&state.pool)
            .await
            .unwrap();
        // What WhatsApp told the library about people outside the address book
        for (lid, number) in [("111", OWN), ("222", BOB)] {
            sqlx::query(
                "INSERT INTO whatsapp_lid_mappings
                     (account_id, lid, phone_number, learning_source, created_at, updated_at)
                 VALUES ($1, $2, $3, 'test', 0, 0)",
            )
            .bind(account.id)
            .bind(lid)
            .bind(number)
            .execute(&state.pool)
            .await
            .unwrap();
        }
        let body = Body {
            sender_jid: "222@lid".to_owned(),
            sender_number: None,
            sender_name: Some("bob".to_owned()),
            kind: Kind::Text,
            text: Some(format!("@111 and @{ADA}, and @333")),
            file_name: None,
            reply_to: None,
            mentions: vec![
                "111@lid".to_owned(),
                format!("{ADA}@s.whatsapp.net"),
                "333@lid".to_owned(),
            ],
            edited: false,
        };
        let chat = (RecipientType::Group, GROUP);
        let sealed = seal_body(&state.cipher, account.id, chat, "G1", &body).unwrap();
        inbox_messages::save(
            &state.pool,
            inbox_messages::NewMessage {
                account_id: account.id,
                chat_type: "group",
                chat_id: GROUP,
                message_id: "G1",
                from_me: false,
                sent_at: Utc::now(),
                expires_at: None,
                body: &sealed,
            },
        )
        .await
        .unwrap();

        let page = read_messages(&state, &caller, read(GROUP, None, None))
            .await
            .unwrap();
        let message = &page.messages[0];
        // Kept without a number, the sender's LID has one now
        let sender = message.sender.as_ref().unwrap();
        assert_eq!(sender.number.as_deref(), Some(BOB));
        let ada = format!("@{ADA}");
        let tags: Vec<_> = message
            .mentions
            .iter()
            .map(|tag| {
                (
                    tag.tag.as_str(),
                    tag.name.as_deref(),
                    tag.number.as_deref(),
                    tag.owner,
                )
            })
            .collect();
        assert_eq!(
            tags,
            [
                ("@111", None, Some(OWN), true),
                (ada.as_str(), Some("Ada Lovelace"), Some(ADA), false),
                ("@333", None, None, false),
            ]
        );
    }

    #[sqlx::test]
    async fn a_group_message_tags_the_numbers_written_in_it(pool: PgPool) {
        let state = state(pool);
        let owner = user(&state.pool).await;
        connected(&state, owner);
        let mut grant = reader();
        for rule in &mut grant.chats.rules {
            rule.send = SendLevel::Yes;
        }
        let caller = connect(&state, owner, &grant).await;
        let tagging = |recipient: &str, body: &str, mentions: &[&str]| SendInput {
            mentions: mentions.iter().map(|&number| number.to_owned()).collect(),
            ..send_to(recipient, body)
        };
        let tags_of = async |delivery: Uuid| -> Vec<String> {
            sqlx::query_scalar("SELECT mentions FROM deliveries WHERE id = $1")
                .bind(delivery)
                .fetch_one(&state.pool)
                .await
                .unwrap()
        };

        let body = format!("@{ADA} and @{BOB}, can you check?");
        let spelled = format!("+{ADA}");
        let sent = send_message(
            &state,
            &caller,
            tagging(GROUP, &body, &[&spelled, BOB, ADA]),
        )
        .await
        .unwrap();
        assert_eq!(sent.outcome, SendOutcome::Queued);
        assert_eq!(tags_of(sent.delivery_id.unwrap()).await, [ADA, BOB]);

        // A tag is written where it goes, and only a group tags anyone
        let unwritten =
            send_message(&state, &caller, tagging(GROUP, "Can you check?", &[ADA])).await;
        assert!(
            matches!(unwritten, Err(OpError::Invalid(_))),
            "{unwritten:?}"
        );
        let longer = format!("@{ADA}0 can you check?");
        let other = send_message(&state, &caller, tagging(GROUP, &longer, &[ADA])).await;
        assert!(matches!(other, Err(OpError::Invalid(_))), "{other:?}");
        let direct = format!("@{ADA} hi");
        let direct = send_message(&state, &caller, tagging(ADA, &direct, &[ADA])).await;
        assert!(matches!(direct, Err(OpError::Invalid(_))), "{direct:?}");

        // One that waits for approval keeps its tags
        let asking = connect(&state, owner, &reader()).await;
        let body = format!("@{BOB} please");
        let waiting = send_message(&state, &asking, tagging(GROUP, &body, &[BOB]))
            .await
            .unwrap();
        assert_eq!(waiting.outcome, SendOutcome::WaitingForApproval);
        let approved = review::approve(&state, waiting.approval_id.unwrap(), owner, None).await;
        assert_eq!(tags_of(approved.unwrap().delivery_id.unwrap()).await, [BOB]);
    }
}
