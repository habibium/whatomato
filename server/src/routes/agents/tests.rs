// Approvals are made through the operations, as an agent would; WhatsApp shows as connected
// through `WhatsApp::pretend`, so nothing reaches it

use axum::http::{Method, StatusCode};
use chrono::{TimeDelta, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    agents::{
        Preset,
        key::{self, AccessKey, PREFIX},
        ops::{
            Caller,
            sends::{SendInput, send_message},
        },
    },
    db::{
        agent_connections::{self, Credential, MAX_PER_USER, NewConnection},
        scheduled_messages::{self, Fields},
    },
    routes::auth::tests::{PASSWORD, TestApp, TestResponse},
    scheduler::{RecipientType, ScheduleKind},
    whatsapp::status::Status,
};

const ADA: &str = "8801700000001";

fn preset(preset: Preset) -> Value {
    serde_json::to_value(preset.grant()).unwrap()
}

async fn call(
    app: &TestApp,
    method: Method,
    path: &str,
    body: Option<Value>,
    session: &str,
) -> TestResponse {
    app.call(method, &format!("/agents{path}"), body, Some(session))
        .await
}

async fn create(app: &TestApp, session: &str, name: &str) -> TestResponse {
    call(
        app,
        Method::POST,
        "",
        Some(json!({
            "name": name,
            "grant": preset(Preset::AskBeforeSending),
            "password": PASSWORD,
        })),
        session,
    )
    .await
}

/// A connection made straight in the database, sparing the password's rate limit
async fn connection(app: &TestApp, owner: Uuid, name: &str) -> Uuid {
    let key = AccessKey::generate().unwrap();
    agent_connections::create(
        &app.pool,
        NewConnection {
            user_id: owner,
            name,
            credential: Credential::Key {
                hash: &key.hash,
                hint: &key.hint,
            },
            grant: &Preset::AskBeforeSending.grant(),
            expires_at: None,
        },
    )
    .await
    .unwrap()
    .unwrap()
    .id
}

fn connected(app: &TestApp, owner: Uuid) {
    app.state.whatsapp.pretend(
        owner,
        Status::Connected {
            phone_number: "8801700000000".into(),
        },
    );
}

/// Has the connection ask to send `body` to Ada, and gives the approval's id
async fn ask(app: &TestApp, owner: Uuid, connection_id: Uuid, body: &str) -> Uuid {
    let connection = agent_connections::find(&app.pool, connection_id, owner)
        .await
        .unwrap()
        .unwrap();
    let caller = Caller::from_connection(&app.state.app_url, connection).unwrap();
    let input = SendInput {
        recipient_type: RecipientType::Contact,
        recipient: ADA.to_owned(),
        body: body.to_owned(),
        timezone: None,
        reply_to: None,
        mentions: Vec::new(),
        attachment: None,
    };
    send_message(&app.state, &caller, input)
        .await
        .unwrap()
        .approval_id
        .unwrap()
}

async fn schedule(app: &TestApp, owner: Uuid, connection_id: Option<Uuid>) -> Uuid {
    let at = Utc::now() + TimeDelta::days(1);
    scheduled_messages::create(
        &app.pool,
        owner,
        connection_id,
        Fields {
            recipient_type: RecipientType::Contact,
            recipient: ADA,
            recipient_name: None,
            body: "Good morning",
            schedule_kind: ScheduleKind::Once,
            run_at: Some(at),
            recurrence: None,
            timezone: "UTC",
            template_id: None,
            enabled: true,
            next_run_at: Some(at),
        },
    )
    .await
    .unwrap()
    .id
}

fn subjects(app: &TestApp, to: &str) -> Vec<&'static str> {
    app.emails_to(to)
        .into_iter()
        .map(|(subject, _)| subject)
        .collect()
}

#[sqlx::test]
async fn creating_answers_with_the_key_once_and_emails_the_owner(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;

    let created = call(
        &app,
        Method::POST,
        "",
        Some(json!({
            "name": "  Claude on my laptop ",
            "grant": preset(Preset::ViewOnly),
            "expires_in_days": 30,
            "password": PASSWORD,
        })),
        &session,
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let key = created.body["key"].as_str().unwrap().to_owned();
    assert!(key.starts_with(PREFIX));
    let connection = &created.body["connection"];
    assert_eq!(connection["name"], "Claude on my laptop");
    assert_eq!(connection["key_hint"], key[key.len() - 4..]);
    assert_eq!(connection["status"], "active");
    assert_eq!(connection["preset"], "view_only");
    assert_eq!(connection["grant"], preset(Preset::ViewOnly));
    assert_eq!(connection["grant_problem"], Value::Null);
    assert!(connection["summary"].as_array().unwrap().len() > 3);
    assert!(connection["expires_at"].is_string());
    assert_eq!(connection["usage"]["daily_ceiling"], 200);
    let id = connection["id"].as_str().unwrap().to_owned();

    let alert = app.emails_to("ada@example.com");
    assert_eq!(alert.len(), 1);
    assert_eq!(alert[0].0, "An AI agent was connected to your account");
    assert!(alert[0].1.contains("\u{201c}Claude on my laptop\u{201d}"));
    assert!(alert[0].1.contains("pause all agents"));
    assert!(!alert[0].1.contains(&key));

    // Never again: not in the list, nor in one connection
    let listed = call(&app, Method::GET, "", None, &session).await;
    assert_eq!(listed.status, StatusCode::OK);
    assert_eq!(listed.body["enabled"], true);
    assert_eq!(listed.body["approvals_waiting"], 0);
    assert_eq!(listed.body["connections"][0]["id"], id.as_str());
    let one = call(&app, Method::GET, &format!("/{id}"), None, &session).await;
    assert_eq!(one.status, StatusCode::OK);
    assert_eq!(one.body["name"], "Claude on my laptop");
    assert_eq!(one.body["schedules"], 0);
    for response in [&listed, &one] {
        assert!(!response.body.to_string().contains(&key));
    }
}

#[sqlx::test]
async fn creating_refuses_bad_input_a_bad_grant_and_a_wrong_password(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;

    let mut bad = Preset::AskBeforeSending.grant();
    bad.chats.default.daily_limit = Some(0);
    let invalid = call(
        &app,
        Method::POST,
        "",
        Some(json!({ "name": "Claude", "grant": bad, "password": PASSWORD })),
        &session,
    )
    .await;
    assert_eq!(invalid.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        invalid.body["details"]["grant"],
        json!([
            "The default rule has a daily limit of 0. To stop sending there, set it to not send \
             instead."
        ])
    );

    let out_of_range = call(
        &app,
        Method::POST,
        "",
        Some(json!({
            "name": "",
            "grant": preset(Preset::ViewOnly),
            "expires_in_days": 366,
            "password": PASSWORD,
        })),
        &session,
    )
    .await;
    assert_eq!(out_of_range.status, StatusCode::BAD_REQUEST);
    assert!(out_of_range.body["details"]["name"].is_array());
    assert!(out_of_range.body["details"]["expires_in_days"].is_array());

    let unreadable = call(
        &app,
        Method::POST,
        "",
        Some(json!({ "name": "Claude", "grant": { "version": 1 }, "password": PASSWORD })),
        &session,
    )
    .await;
    assert_eq!(unreadable.status, StatusCode::BAD_REQUEST);

    let wrong = call(
        &app,
        Method::POST,
        "",
        Some(json!({
            "name": "Claude",
            "grant": preset(Preset::ViewOnly),
            "password": "not the password",
        })),
        &session,
    )
    .await;
    assert_eq!(wrong.status, StatusCode::BAD_REQUEST);
    assert_eq!(wrong.body["details"]["password"], json!(["is incorrect"]));

    let missing = call(
        &app,
        Method::POST,
        "",
        Some(json!({ "name": "Claude", "grant": preset(Preset::ViewOnly) })),
        &session,
    )
    .await;
    assert_eq!(missing.status, StatusCode::BAD_REQUEST);

    let listed = call(&app, Method::GET, "", None, &session).await;
    assert_eq!(listed.body["connections"], json!([]));
    assert_eq!(app.emails_to("ada@example.com"), []);
}

#[sqlx::test]
async fn an_account_holds_at_most_20_connections(pool: PgPool) {
    let app = TestApp::new(pool);
    let (owner, session) = app.signed_in("ada@example.com").await;
    for n in 0..MAX_PER_USER {
        connection(&app, owner, &format!("Agent {n}")).await;
    }
    let refused = create(&app, &session, "One too many").await;
    assert_eq!(refused.status, StatusCode::CONFLICT);
    assert!(
        refused.body["error"]
            .as_str()
            .unwrap()
            .contains("Revoke one")
    );
    assert_eq!(app.emails_to("ada@example.com"), []);
}

#[sqlx::test]
async fn presets_come_in_order_with_their_grants(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;
    let presets = call(&app, Method::GET, "/presets", None, &session).await;
    assert_eq!(presets.status, StatusCode::OK);
    let ids: Vec<_> = presets
        .body
        .as_array()
        .unwrap()
        .iter()
        .map(|preset| preset["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            "ask_before_sending",
            "view_only",
            "send_without_asking",
            "full_access"
        ]
    );
    assert_eq!(presets.body[3]["grant"], preset(Preset::FullAccess));
    assert_eq!(presets.body[0]["name"], "Ask before sending");
    assert!(presets.body[0]["description"].is_string());
}

#[sqlx::test]
async fn renaming_needs_no_password_and_a_new_grant_needs_it(pool: PgPool) {
    let app = TestApp::new(pool);
    let (owner, session) = app.signed_in("ada@example.com").await;
    let id = connection(&app, owner, "Claude").await;
    let path = format!("/{id}");

    let renamed = call(
        &app,
        Method::PATCH,
        &path,
        Some(json!({ "name": "Codex" })),
        &session,
    )
    .await;
    assert_eq!(renamed.status, StatusCode::OK, "{}", renamed.body);
    assert_eq!(renamed.body["name"], "Codex");
    assert_eq!(app.emails_to("ada@example.com"), []);

    let full = preset(Preset::FullAccess);
    let without = call(
        &app,
        Method::PATCH,
        &path,
        Some(json!({ "grant": full })),
        &session,
    )
    .await;
    assert_eq!(without.status, StatusCode::BAD_REQUEST);
    assert!(without.body["details"]["password"].is_array());
    let wrong = call(
        &app,
        Method::PATCH,
        &path,
        Some(json!({ "grant": full, "password": "nope" })),
        &session,
    )
    .await;
    assert_eq!(wrong.status, StatusCode::BAD_REQUEST);
    assert_eq!(wrong.body["details"]["password"], json!(["is incorrect"]));
    let nothing = call(&app, Method::PATCH, &path, Some(json!({})), &session).await;
    assert_eq!(nothing.status, StatusCode::BAD_REQUEST);

    let mut bad = Preset::FullAccess.grant();
    bad.version = 2;
    let invalid = call(
        &app,
        Method::PATCH,
        &path,
        Some(json!({ "grant": bad, "password": PASSWORD })),
        &session,
    )
    .await;
    assert_eq!(invalid.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(invalid.body["details"]["grant"][0].is_string());
    assert_eq!(app.emails_to("ada@example.com"), []);

    let changed = call(
        &app,
        Method::PATCH,
        &path,
        Some(json!({ "name": "Codex CLI", "grant": full, "password": PASSWORD })),
        &session,
    )
    .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
    assert_eq!(changed.body["preset"], "full_access");
    assert_eq!(changed.body["name"], "Codex CLI");
    let emails = app.emails_to("ada@example.com");
    assert_eq!(emails.len(), 1);
    assert_eq!(emails[0].0, "An AI agent's permissions were changed");
    assert!(emails[0].1.contains("\u{201c}Codex CLI\u{201d}"));

    // The same permissions again change nothing, so nobody is emailed
    let same = call(
        &app,
        Method::PATCH,
        &path,
        Some(json!({ "grant": full, "password": PASSWORD })),
        &session,
    )
    .await;
    assert_eq!(same.status, StatusCode::OK);
    assert_eq!(app.emails_to("ada@example.com").len(), 1);
}

#[sqlx::test]
async fn pausing_resuming_and_pausing_all(pool: PgPool) {
    let app = TestApp::new(pool);
    let (owner, session) = app.signed_in("ada@example.com").await;
    let first = connection(&app, owner, "Claude").await;
    let second = connection(&app, owner, "Codex").await;

    let paused = call(
        &app,
        Method::POST,
        &format!("/{first}/pause"),
        None,
        &session,
    )
    .await;
    assert_eq!(paused.status, StatusCode::OK);
    assert_eq!(paused.body["status"], "paused");
    assert!(paused.body["paused_at"].is_string());
    let resumed = call(
        &app,
        Method::POST,
        &format!("/{first}/resume"),
        None,
        &session,
    )
    .await;
    assert_eq!(resumed.body["status"], "active");
    assert_eq!(resumed.body["paused_at"], Value::Null);

    call(
        &app,
        Method::POST,
        &format!("/{second}/pause"),
        None,
        &session,
    )
    .await;
    let all = call(&app, Method::POST, "/pause-all", None, &session).await;
    assert_eq!(all.status, StatusCode::OK);
    assert_eq!(all.body, json!({ "paused": 1 }));
    let listed = call(&app, Method::GET, "", None, &session).await;
    for connection in listed.body["connections"].as_array().unwrap() {
        assert_eq!(connection["status"], "paused");
    }
}

#[sqlx::test]
async fn rotating_the_key_needs_the_password_and_answers_with_the_new_key(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;
    let created = create(&app, &session, "Claude").await;
    let old = created.body["key"].as_str().unwrap().to_owned();
    let id = created.body["connection"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let path = format!("/{id}/rotate-key");

    let wrong = call(
        &app,
        Method::POST,
        &path,
        Some(json!({ "password": "nope" })),
        &session,
    )
    .await;
    assert_eq!(wrong.status, StatusCode::BAD_REQUEST);

    let rotated = call(
        &app,
        Method::POST,
        &path,
        Some(json!({ "password": PASSWORD })),
        &session,
    )
    .await;
    assert_eq!(rotated.status, StatusCode::OK, "{}", rotated.body);
    let new = rotated.body["key"].as_str().unwrap();
    assert!(new.starts_with(PREFIX));
    assert_ne!(new, old);
    assert_eq!(rotated.body["connection"]["key_hint"], new[new.len() - 4..]);
    assert!(
        agent_connections::find_by_key(&app.pool, &key::hash(&old))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        agent_connections::find_by_key(&app.pool, &key::hash(new))
            .await
            .unwrap()
            .is_some()
    );

    assert_eq!(
        subjects(&app, "ada@example.com"),
        [
            "An AI agent was connected to your account",
            "An AI agent's key was replaced"
        ]
    );
    let listed = call(&app, Method::GET, "", None, &session).await;
    assert!(!listed.body.to_string().contains(new));
}

#[sqlx::test]
async fn revoking_keeps_or_deletes_its_schedules(pool: PgPool) {
    let app = TestApp::new(pool);
    let (owner, session) = app.signed_in("ada@example.com").await;
    let kept = connection(&app, owner, "Claude").await;
    let gone = connection(&app, owner, "Codex").await;
    let mine = schedule(&app, owner, None).await;
    let its = schedule(&app, owner, Some(kept)).await;
    for _ in 0..2 {
        schedule(&app, owner, Some(gone)).await;
    }

    let detail = call(&app, Method::GET, &format!("/{gone}"), None, &session).await;
    assert_eq!(detail.body["schedules"], 2);

    let revoked = call(&app, Method::DELETE, &format!("/{kept}"), None, &session).await;
    assert_eq!(revoked.status, StatusCode::OK);
    assert_eq!(
        revoked.body,
        json!({ "schedules_deleted": 0, "schedules_kept": 1 })
    );
    let deleted = call(
        &app,
        Method::DELETE,
        &format!("/{gone}?delete_schedules=true"),
        None,
        &session,
    )
    .await;
    assert_eq!(
        deleted.body,
        json!({ "schedules_deleted": 2, "schedules_kept": 0 })
    );
    let again = call(&app, Method::DELETE, &format!("/{gone}"), None, &session).await;
    assert_eq!(again.status, StatusCode::NOT_FOUND);

    let messages = app
        .call(Method::GET, "/messages", None, Some(&session))
        .await;
    let ids: Vec<_> = messages
        .body
        .as_array()
        .unwrap()
        .iter()
        .map(|message| message["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids, [mine.to_string(), its.to_string()]);
    assert_eq!(messages.body[1]["connection_id"], Value::Null);
    assert_eq!(messages.body[1]["connection_name"], Value::Null);
}

#[sqlx::test]
async fn scheduled_messages_show_which_agent_made_them(pool: PgPool) {
    let app = TestApp::new(pool);
    let (owner, session) = app.signed_in("ada@example.com").await;
    let claude = connection(&app, owner, "Claude").await;
    let mine = schedule(&app, owner, None).await;
    let its = schedule(&app, owner, Some(claude)).await;

    let listed = app
        .call(Method::GET, "/messages", None, Some(&session))
        .await;
    assert_eq!(listed.body[0]["connection_id"], Value::Null);
    assert_eq!(listed.body[0]["connection_name"], Value::Null);
    assert_eq!(listed.body[1]["connection_id"], claude.to_string());
    assert_eq!(listed.body[1]["connection_name"], "Claude");
    let one = app
        .call(
            Method::GET,
            &format!("/messages/{its}"),
            None,
            Some(&session),
        )
        .await;
    assert_eq!(one.body["connection_name"], "Claude");
    let toggled = app
        .call(
            Method::PATCH,
            &format!("/messages/{its}"),
            Some(json!({ "enabled": false })),
            Some(&session),
        )
        .await;
    assert_eq!(toggled.status, StatusCode::OK, "{}", toggled.body);
    assert_eq!(toggled.body["connection_name"], "Claude");
    let own = app
        .call(
            Method::GET,
            &format!("/messages/{mine}"),
            None,
            Some(&session),
        )
        .await;
    assert_eq!(own.body["connection_name"], Value::Null);
}

#[sqlx::test]
async fn approving_queues_the_message_with_the_agents_or_the_owners_text(pool: PgPool) {
    let app = TestApp::new(pool);
    let (owner, session) = app.signed_in("ada@example.com").await;
    let claude = connection(&app, owner, "Claude").await;
    let as_written = ask(&app, owner, claude, "Hello\u{200b} there").await;
    let edited = ask(&app, owner, claude, "Helo").await;

    let listed = call(&app, Method::GET, "/approvals", None, &session).await;
    assert_eq!(listed.status, StatusCode::OK);
    assert_eq!(listed.body["total"], 2);
    let item = &listed.body["items"][1];
    assert_eq!(item["id"], as_written.to_string());
    assert_eq!(item["connection_id"], claude.to_string());
    assert_eq!(item["connection_name"], "Claude");
    assert_eq!(item["status"], "waiting");
    assert_eq!(item["can_approve"], true);
    assert_eq!(item["request"]["body"], "Hello\u{200b} there");
    assert_eq!(item["request"]["recipient"], ADA);
    assert_eq!(item["request"]["schedule"], Value::Null);
    let overview = call(&app, Method::GET, "", None, &session).await;
    assert_eq!(overview.body["approvals_waiting"], 2);
    assert_eq!(
        overview.body["connections"][0]["usage"]["approvals_waiting"],
        2
    );

    // WhatsApp isn't linked yet, so it can't go ahead and stays waiting
    let not_yet = call(
        &app,
        Method::POST,
        &format!("/approvals/{as_written}/approve"),
        Some(json!({})),
        &session,
    )
    .await;
    assert_eq!(not_yet.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(not_yet.body["details"]["request"][0].is_string());

    connected(&app, owner);
    let approved = call(
        &app,
        Method::POST,
        &format!("/approvals/{as_written}/approve"),
        Some(json!({})),
        &session,
    )
    .await;
    assert_eq!(approved.status, StatusCode::OK, "{}", approved.body);
    assert_eq!(approved.body["status"], "approved");
    assert_eq!(approved.body["delivery_status"], "pending");
    assert_eq!(approved.body["can_reject"], false);
    let delivery_id = approved.body["delivery_id"].as_str().unwrap().to_owned();

    let twice = call(
        &app,
        Method::POST,
        &format!("/approvals/{as_written}/approve"),
        Some(json!({})),
        &session,
    )
    .await;
    assert_eq!(twice.status, StatusCode::CONFLICT);
    assert_eq!(twice.body["details"]["status"], json!(["approved"]));

    let too_long = call(
        &app,
        Method::POST,
        &format!("/approvals/{edited}/approve"),
        Some(json!({ "body": "x".repeat(4097) })),
        &session,
    )
    .await;
    assert_eq!(too_long.status, StatusCode::UNPROCESSABLE_ENTITY);
    let fixed = call(
        &app,
        Method::POST,
        &format!("/approvals/{edited}/approve"),
        Some(json!({ "body": "Hello" })),
        &session,
    )
    .await;
    assert_eq!(fixed.status, StatusCode::OK, "{}", fixed.body);

    // Both show who made them
    let deliveries = app
        .call(Method::GET, "/deliveries", None, Some(&session))
        .await;
    let bodies: Vec<_> = deliveries.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            assert_eq!(item["connection_id"], claude.to_string());
            assert_eq!(item["connection_name"], "Claude");
            item["body"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(bodies, ["Hello", "Hello\u{200b} there"]);
    assert_eq!(deliveries.body["items"][1]["id"], delivery_id.as_str());
}

#[sqlx::test]
async fn rejecting_and_expiry_end_an_approval(pool: PgPool) {
    let app = TestApp::new(pool);
    let (owner, session) = app.signed_in("ada@example.com").await;
    connected(&app, owner);
    let claude = connection(&app, owner, "Claude").await;
    let rejected = ask(&app, owner, claude, "No").await;
    let overdue = ask(&app, owner, claude, "Late").await;

    let answer = call(
        &app,
        Method::POST,
        &format!("/approvals/{rejected}/reject"),
        None,
        &session,
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(answer.body["status"], "rejected");
    assert!(answer.body["decided_at"].is_string());
    let approve_after = call(
        &app,
        Method::POST,
        &format!("/approvals/{rejected}/approve"),
        Some(json!({})),
        &session,
    )
    .await;
    assert_eq!(approve_after.status, StatusCode::CONFLICT);
    assert_eq!(approve_after.body["details"]["status"], json!(["rejected"]));

    sqlx::query!(
        "UPDATE agent_approvals SET created_at = now() - interval '25 hours', \
         expires_at = now() - interval '1 hour' WHERE id = $1",
        overdue
    )
    .execute(&app.pool)
    .await
    .unwrap();
    for action in ["approve", "reject"] {
        let late = call(
            &app,
            Method::POST,
            &format!("/approvals/{overdue}/{action}"),
            Some(json!({})),
            &session,
        )
        .await;
        assert_eq!(late.status, StatusCode::CONFLICT, "{action}");
        assert_eq!(late.body["details"]["status"], json!(["expired"]));
        assert!(late.body["error"].as_str().unwrap().contains("expired"));
    }
    let listed = call(&app, Method::GET, "/approvals", None, &session).await;
    let expired = listed.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == overdue.to_string())
        .unwrap()
        .clone();
    assert_eq!(expired["status"], "expired");
    assert_eq!(expired["can_reject"], false);
}

#[sqlx::test]
async fn an_approval_that_cannot_be_read_can_only_be_rejected(pool: PgPool) {
    let app = TestApp::new(pool);
    let (owner, session) = app.signed_in("ada@example.com").await;
    connected(&app, owner);
    let claude = connection(&app, owner, "Claude").await;
    let garbled = ask(&app, owner, claude, "Hello").await;
    sqlx::query!(
        "UPDATE agent_approvals SET payload = '\\x00'::bytea WHERE id = $1",
        garbled
    )
    .execute(&app.pool)
    .await
    .unwrap();

    let listed = call(&app, Method::GET, "/approvals", None, &session).await;
    assert_eq!(listed.status, StatusCode::OK);
    let item = &listed.body["items"][0];
    assert_eq!(item["request"], Value::Null);
    assert!(item["problem"].is_string());
    assert_eq!(item["recipient"], ADA);
    assert_eq!(item["can_approve"], false);
    assert_eq!(item["can_reject"], true);

    let approve = call(
        &app,
        Method::POST,
        &format!("/approvals/{garbled}/approve"),
        Some(json!({})),
        &session,
    )
    .await;
    assert_eq!(approve.status, StatusCode::UNPROCESSABLE_ENTITY);
    let reject = call(
        &app,
        Method::POST,
        &format!("/approvals/{garbled}/reject"),
        None,
        &session,
    )
    .await;
    assert_eq!(reject.status, StatusCode::OK);
}

#[sqlx::test]
async fn another_users_connections_approvals_and_activity_are_not_found(pool: PgPool) {
    let app = TestApp::new(pool);
    let (owner, owner_session) = app.signed_in("ada@example.com").await;
    let (_, session) = app.signed_in("grace@example.com").await;
    let claude = connection(&app, owner, "Claude").await;
    let approval = ask(&app, owner, claude, "Secret plans").await;

    let cases = [
        (Method::GET, format!("/{claude}"), None),
        (
            Method::PATCH,
            format!("/{claude}"),
            Some(json!({ "name": "Mine" })),
        ),
        (Method::POST, format!("/{claude}/pause"), None),
        (Method::POST, format!("/{claude}/resume"), None),
        (
            Method::POST,
            format!("/{claude}/rotate-key"),
            Some(json!({ "password": PASSWORD })),
        ),
        (
            Method::DELETE,
            format!("/{claude}?delete_schedules=true"),
            None,
        ),
        (
            Method::POST,
            format!("/approvals/{approval}/approve"),
            Some(json!({})),
        ),
        (Method::POST, format!("/approvals/{approval}/reject"), None),
        (Method::GET, format!("/approvals/{approval}"), None),
        (
            Method::GET,
            format!("/activity?connection_id={claude}"),
            None,
        ),
    ];
    for (method, path, body) in cases {
        let response = call(&app, method, &path, body, &session).await;
        assert_eq!(response.status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(response.body["error"], "Not Found", "{path}");
    }

    for path in ["", "/approvals", "/activity"] {
        let theirs = call(&app, Method::GET, path, None, &session).await;
        assert_eq!(theirs.status, StatusCode::OK, "{path}");
        let text = theirs.body.to_string();
        assert!(!text.contains("Secret plans"), "{path}");
        assert!(!text.contains("Claude"), "{path}");
    }
    let empty = call(&app, Method::GET, "/approvals", None, &session).await;
    assert_eq!(empty.body["total"], 0);

    // Untouched for its owner
    let mine = call(&app, Method::GET, "/approvals", None, &owner_session).await;
    assert_eq!(mine.body["items"][0]["status"], "waiting");
    assert_eq!(mine.body["items"][0]["request"]["body"], "Secret plans");
    let connection = call(
        &app,
        Method::GET,
        &format!("/{claude}"),
        None,
        &owner_session,
    )
    .await;
    assert_eq!(connection.body["name"], "Claude");
    assert_eq!(connection.body["status"], "active");
}

#[sqlx::test]
async fn activity_lists_every_call_newest_first(pool: PgPool) {
    let app = TestApp::new(pool);
    let (owner, session) = app.signed_in("ada@example.com").await;
    let claude = connection(&app, owner, "Claude").await;
    let codex = connection(&app, owner, "Codex").await;
    ask(&app, owner, claude, "One").await;
    ask(&app, owner, codex, "Two").await;

    let all = call(&app, Method::GET, "/activity", None, &session).await;
    assert_eq!(all.status, StatusCode::OK);
    assert_eq!(all.body["total"], 2);
    let newest = &all.body["items"][0];
    assert_eq!(newest["connection_name"], "Codex");
    assert_eq!(newest["tool"], "send_message");
    assert_eq!(newest["outcome"], "asked");
    assert_eq!(newest["recipient"], ADA);
    assert!(!all.body.to_string().contains("\"One\""));

    let one = call(
        &app,
        Method::GET,
        &format!("/activity?connection_id={claude}&limit=1"),
        None,
        &session,
    )
    .await;
    assert_eq!(one.body["total"], 1);
    assert_eq!(one.body["limit"], 1);
    assert_eq!(one.body["items"][0]["connection_name"], "Claude");

    let bad = call(&app, Method::GET, "/activity?limit=0", None, &session).await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
}

#[sqlx::test]
async fn a_waiting_approval_emails_the_owner_once_an_hour(pool: PgPool) {
    let app = TestApp::new(pool);
    let (owner, _) = app.signed_in("ada@example.com").await;
    let claude = connection(&app, owner, "Claude <b>").await;
    let first = ask(&app, owner, claude, "Hello").await;
    ask(&app, owner, claude, "Hello again").await;

    let emails = app.emails_to("ada@example.com");
    assert_eq!(emails.len(), 1);
    let (subject, text) = &emails[0];
    assert_eq!(*subject, "A message is waiting for your approval");
    assert!(text.contains("\u{201c}Claude <b>\u{201d}"));
    assert!(text.contains(&format!("/agents?approval={first}")));
    assert!(!text.contains("Hello"));

    // An hour on, the next one emails again
    sqlx::query!(
        "UPDATE agent_approvals SET created_at = created_at - interval '2 hours' \
         WHERE user_id = $1",
        owner
    )
    .execute(&app.pool)
    .await
    .unwrap();
    ask(&app, owner, claude, "Later").await;
    assert_eq!(app.emails_to("ada@example.com").len(), 2);
}

#[sqlx::test]
async fn with_agent_access_off_only_the_overview_answers(pool: PgPool) {
    let app = TestApp::without_agent_access(pool);
    let (owner, session) = app.signed_in("ada@example.com").await;
    let claude = connection(&app, owner, "Claude").await;
    let id = Uuid::new_v4();

    let overview = call(&app, Method::GET, "", None, &session).await;
    assert_eq!(overview.status, StatusCode::OK);
    assert_eq!(
        overview.body,
        json!({ "enabled": false, "connections": [], "approvals_waiting": 0 })
    );

    let cases = [
        (Method::POST, String::new(), Some(json!({}))),
        (Method::GET, "/presets".to_owned(), None),
        (Method::GET, format!("/{claude}"), None),
        (
            Method::PATCH,
            format!("/{claude}"),
            Some(json!({ "name": "x" })),
        ),
        (Method::POST, format!("/{claude}/pause"), None),
        (Method::POST, format!("/{claude}/resume"), None),
        (Method::POST, "/pause-all".to_owned(), None),
        (
            Method::POST,
            format!("/{claude}/rotate-key"),
            Some(json!({ "password": PASSWORD })),
        ),
        (Method::DELETE, format!("/{claude}"), None),
        (Method::GET, "/approvals".to_owned(), None),
        (Method::GET, format!("/approvals/{id}"), None),
        (
            Method::POST,
            format!("/approvals/{id}/approve"),
            Some(json!({})),
        ),
        (Method::POST, format!("/approvals/{id}/reject"), None),
        (Method::GET, "/activity".to_owned(), None),
    ];
    for (method, path, body) in cases {
        let response = call(&app, method, &path, body, &session).await;
        assert_eq!(response.status, StatusCode::NOT_FOUND, "{path}");
    }
    assert!(
        agent_connections::find(&app.pool, claude, owner)
            .await
            .unwrap()
            .is_some_and(|connection| connection.paused_at.is_none())
    );
}

#[sqlx::test]
async fn every_route_needs_a_session(pool: PgPool) {
    let app = TestApp::new(pool);
    for (method, path) in [
        (Method::GET, "/agents"),
        (Method::GET, "/agents/presets"),
        (Method::GET, "/agents/approvals"),
        (Method::GET, "/agents/activity"),
    ] {
        let response = app.call(method, path, None, None).await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED, "{path}");
    }
}

#[sqlx::test]
async fn a_connection_whose_grant_cannot_be_read_is_shown_unusable(pool: PgPool) {
    let app = TestApp::new(pool);
    let (owner, session) = app.signed_in("ada@example.com").await;
    let broken = connection(&app, owner, "Broken").await;
    connection(&app, owner, "Fine").await;
    sqlx::query!(
        r#"UPDATE agent_connections SET "grant" = '{"version": 1}' WHERE id = $1"#,
        broken
    )
    .execute(&app.pool)
    .await
    .unwrap();

    let listed = call(&app, Method::GET, "", None, &session).await;
    assert_eq!(listed.status, StatusCode::OK);
    let first = &listed.body["connections"][0];
    assert_eq!(first["status"], "unusable");
    assert_eq!(first["grant"], Value::Null);
    assert_eq!(first["preset"], Value::Null);
    assert!(first["grant_problem"].is_string());
    assert_eq!(first["summary"], json!([]));
    assert_eq!(listed.body["connections"][1]["status"], "active");
}

#[sqlx::test]
async fn an_approval_shows_its_connections_status_and_stays_approvable(pool: PgPool) {
    let app = TestApp::new(pool);
    let (owner, session) = app.signed_in("ada@example.com").await;
    connected(&app, owner);
    let claude = connection(&app, owner, "Claude").await;
    let approval = ask(&app, owner, claude, "See you at nine").await;
    let status = |response: &TestResponse| response.body["connection_status"].clone();

    let listed = call(&app, Method::GET, "/approvals", None, &session).await;
    assert_eq!(listed.body["items"][0]["connection_status"], "active");
    let one = call(
        &app,
        Method::GET,
        &format!("/approvals/{approval}"),
        None,
        &session,
    )
    .await;
    assert_eq!(one.status, StatusCode::OK);
    assert_eq!(one.body["id"], approval.to_string());
    assert_eq!(one.body["request"]["body"], "See you at nine");
    assert_eq!(status(&one), "active");

    let path = format!("/approvals/{approval}");
    call(
        &app,
        Method::POST,
        &format!("/{claude}/pause"),
        None,
        &session,
    )
    .await;
    assert_eq!(
        status(&call(&app, Method::GET, &path, None, &session).await),
        "paused"
    );
    sqlx::query!(
        "UPDATE agent_connections SET expires_at = now() - interval '1 minute' WHERE id = $1",
        claude
    )
    .execute(&app.pool)
    .await
    .unwrap();
    assert_eq!(
        status(&call(&app, Method::GET, &path, None, &session).await),
        "expired"
    );
    sqlx::query!(
        r#"UPDATE agent_connections SET "grant" = '{"version": 99}' WHERE id = $1"#,
        claude
    )
    .execute(&app.pool)
    .await
    .unwrap();
    assert_eq!(
        status(&call(&app, Method::GET, &path, None, &session).await),
        "unusable"
    );

    // The owner's yes is final, whatever became of the connection
    let approved = call(
        &app,
        Method::POST,
        &format!("{path}/approve"),
        Some(json!({})),
        &session,
    )
    .await;
    assert_eq!(approved.status, StatusCode::OK, "{}", approved.body);
    assert_eq!(approved.body["status"], "approved");
    let answered = call(&app, Method::GET, &path, None, &session).await;
    assert_eq!(answered.body["status"], "approved");
    assert!(answered.body["delivery_id"].is_string());

    let missing = call(
        &app,
        Method::GET,
        &format!("/approvals/{}", Uuid::new_v4()),
        None,
        &session,
    )
    .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn a_waiting_request_reaches_the_owners_pages_and_browsers_without_its_text(pool: PgPool) {
    use futures_util::StreamExt as _;

    let app = TestApp::new(pool);
    let (owner, session) = app.signed_in("ada@example.com").await;
    let (other, _) = app.signed_in("grace@example.com").await;
    connected(&app, owner);
    let connection_id = connection(&app, owner, "Assistant").await;
    for (user_id, endpoint) in [
        (owner, "https://fcm.googleapis.com/fcm/send/adas"),
        (other, "https://fcm.googleapis.com/fcm/send/graces"),
    ] {
        crate::db::push_subscriptions::save(
            &app.pool,
            crate::db::push_subscriptions::NewSubscription {
                user_id,
                endpoint,
                p256dh: &[4; 65],
                auth: &[7; 16],
                label: "Chrome on Linux",
            },
        )
        .await
        .unwrap();
    }
    let mut page = Box::pin(app.state.events.approvals_of(owner));
    let mut others_page = Box::pin(app.state.events.approvals_of(other));

    let approval_id = ask(&app, owner, connection_id, "The secret plan").await;

    let wait = std::time::Duration::from_secs(5);
    assert_eq!(
        tokio::time::timeout(wait, page.next()).await.unwrap(),
        Some(())
    );
    // The notification is sent off the request's path
    let sent = tokio::time::timeout(wait, async {
        loop {
            let sent = app.state.push.sent();
            if !sent.is_empty() {
                break sent;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(sent.len(), 1);
    let (endpoint, notification) = &sent[0];
    assert_eq!(endpoint, "https://fcm.googleapis.com/fcm/send/adas");
    assert_eq!(notification.title, "Assistant wants to send a message");
    assert_eq!(notification.url, format!("/agents?approval={approval_id}"));
    assert_eq!(
        notification.body,
        format!("To +{ADA}. It waits for your yes or no.")
    );
    let whole = serde_json::to_string(notification).unwrap();
    assert!(!whole.contains("secret plan"), "{whole}");

    // Deciding it tells the owner's other pages, and never anyone else's
    let rejected = call(
        &app,
        Method::POST,
        &format!("/approvals/{approval_id}/reject"),
        None,
        &session,
    )
    .await;
    assert_eq!(rejected.status, StatusCode::OK, "{}", rejected.body);
    assert_eq!(
        tokio::time::timeout(wait, page.next()).await.unwrap(),
        Some(())
    );
    let quiet = std::time::Duration::from_millis(50);
    assert!(
        tokio::time::timeout(quiet, others_page.next())
            .await
            .is_err()
    );
}

async fn hold_file(app: &TestApp, owner: Uuid, mime_type: &str, name: &str) -> Uuid {
    use crate::{
        db::outgoing_files::{self, NewFile},
        whatsapp::media::{FileKind, seal_file},
    };

    let id = Uuid::new_v4();
    let sealed = seal_file(&app.state.cipher, id, b"file bytes").unwrap();
    outgoing_files::save(
        &app.pool,
        NewFile {
            id,
            user_id: owner,
            kind: FileKind::of(mime_type),
            mime_type,
            file_name: Some(name),
            size: 10,
            sealed: &sealed,
        },
    )
    .await
    .unwrap();
    id
}

#[sqlx::test]
async fn a_file_waiting_to_go_out_shows_only_to_its_owner_and_never_as_a_page(pool: PgPool) {
    use axum::http::header;

    let app = TestApp::new(pool);
    let (owner, session) = app.signed_in("ada@example.com").await;
    let picture = hold_file(&app, owner, "image/png", "receipt.png").await;
    let page = hold_file(&app, owner, "text/html", "trap.html").await;

    let shown = call(
        &app,
        Method::GET,
        &format!("/files/{picture}"),
        None,
        &session,
    )
    .await;
    assert_eq!(shown.status, StatusCode::OK);
    assert_eq!(shown.headers[header::CONTENT_TYPE], "image/png");
    assert_eq!(shown.headers[header::CONTENT_DISPOSITION], "inline");
    assert_eq!(shown.headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");

    let downloaded = call(&app, Method::GET, &format!("/files/{page}"), None, &session).await;
    assert_eq!(
        downloaded.headers[header::CONTENT_TYPE],
        "application/octet-stream"
    );
    assert_eq!(
        downloaded.headers[header::CONTENT_DISPOSITION],
        "attachment; filename=\"trap.html\""
    );

    let (_, other) = app.signed_in("grace@example.com").await;
    let hidden = call(
        &app,
        Method::GET,
        &format!("/files/{picture}"),
        None,
        &other,
    )
    .await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND);

    crate::db::outgoing_files::release(&app.pool, picture)
        .await
        .unwrap();
    let sent = call(
        &app,
        Method::GET,
        &format!("/files/{picture}"),
        None,
        &session,
    )
    .await;
    assert_eq!(sent.status, StatusCode::NOT_FOUND);
}
