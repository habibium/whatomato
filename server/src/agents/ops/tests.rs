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
        grant::{Chat, ChatRule, Contacts, Schedules, Scope, SendLevel, Strangers, Templates},
        key::AccessKey,
    },
    auth::pwned::PwnedPasswords,
    crypto::Cipher,
    db::{
        agent_activity, agent_approvals,
        agent_connections::{self, NewConnection},
        message_templates, scheduled_messages, whatsapp_accounts, whatsapp_contacts,
    },
    email::Mailer,
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
        cipher: cipher.clone(),
        whatsapp: WhatsApp::new(Context::new(pool, cipher, mailer, app_url)),
        agent_access: true,
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
            key_hash: &key.hash,
            key_hint: &key.hint,
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
        id,
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
    refused_with(
        send_message(&state, &view_only, send_to(STRANGER, "Hi")).await,
        "strangers_never",
    );
    assert_eq!(
        last_activity(&state, &view_only).await,
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
    assert!(schedule.created_by_this_connection);
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
        delete_scheduled_message(&state, &own, DeleteScheduleInput { id: theirs.id }).await,
        "not_own_schedule",
    );

    let mut change = update(mine.id);
    change.body = Some("Changed".to_owned());
    let changed = update_scheduled_message(&state, &own, change)
        .await
        .unwrap();
    assert_eq!(changed.outcome, ScheduleOutcome::Updated);
    assert_eq!(changed.schedule.unwrap().body, "Changed");

    delete_scheduled_message(&state, &all, DeleteScheduleInput { id: mine.id })
        .await
        .unwrap();
    assert!(matches!(
        delete_scheduled_message(&state, &all, DeleteScheduleInput { id: mine.id }).await,
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

    let seen = get_approval(&state, &one, GetApprovalInput { id: mine })
        .await
        .unwrap();
    assert_eq!(seen.status, ApprovalStatus::Waiting);
    assert_eq!(seen.kind, ApprovalKind::Send);
    assert!(seen.delivery.is_none());
    assert!(matches!(
        get_approval(&state, &one, GetApprovalInput { id: theirs }).await,
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

    assert!(matches!(
        review::approve(&state, id, owner, None).await,
        Err(ReviewError::CantApprove(_))
    ));

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

    let seen = get_approval(&state, &caller, GetApprovalInput { id })
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
        ["`body` has a problem: {{nope}} isn't a variable."]
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
    let seen = get_approval(&state, &caller, GetApprovalInput { id })
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
    assert_eq!(problems, ["`run_at` must be in the future."]);
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
        ["The scheduled message this would change has been deleted."]
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
            GetApprovalInput { id: Uuid::nil() },
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
            DeleteScheduleInput { id: Uuid::nil() },
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
    std::thread::spawn(move || {
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
                (passed, owner, pool)
            })
    })
    .join()
    .unwrap()
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
            Err(OpError::Invalid(problems)) => {
                let problem = problems.join(" ");
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
        Err(OpError::Invalid(_))
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
        .unwrap()
        .schedule
        .unwrap();
    assert!(changed.created_by_this_connection);
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
    delete_scheduled_message(&state, &agent, DeleteScheduleInput { id: ids[1] })
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
        delete_scheduled_message(&state, &stale, DeleteScheduleInput { id: theirs }).await,
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

    delete_scheduled_message(&state, &agent, DeleteScheduleInput { id: owners })
        .await
        .unwrap();
    let mine = schedules_of(&state, owner, Some(agent_id), 0).await;
    delete_scheduled_message(&state, &agent, DeleteScheduleInput { id: mine[0] })
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
