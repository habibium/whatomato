// Nothing here sends: the scheduler isn't running, and no account in these tests is linked

use axum::http::{Method, StatusCode};
use chrono::{DateTime, TimeDelta, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::routes::auth::tests::TestApp;

fn tomorrow() -> DateTime<Utc> {
    Utc::now() + TimeDelta::days(1)
}

fn once() -> Value {
    json!({
        "recipient_type": "contact",
        "recipient": "+1 (555) 000-1234",
        "recipient_name": "Ada",
        "body": "  Hello  ",
        "schedule_kind": "once",
        "run_at": tomorrow(),
    })
}

fn recurring() -> Value {
    json!({
        "recipient_type": "contact",
        "recipient": "15550001234",
        "body": "Morning",
        "schedule_kind": "recurring",
        "cron": "0 9 * * *",
        "timezone": "Asia/Dhaka",
    })
}

fn with(mut input: Value, field: &str, value: Value) -> Value {
    input[field] = value;
    input
}

async fn create(app: &TestApp, input: Value, session: &str) -> Value {
    let response = app
        .call(Method::POST, "/messages", Some(input), Some(session))
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
    response.body
}

fn path_of(message: &Value) -> String {
    format!("/messages/{}", message["id"].as_str().unwrap())
}

#[sqlx::test]
async fn messages_need_a_session(pool: PgPool) {
    let app = TestApp::new(pool);
    let response = app.call(Method::GET, "/messages", None, None).await;
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn a_one_time_message_runs_at_its_time(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;

    let message = create(&app, once(), &session).await;

    assert_eq!(message["recipient"], "15550001234");
    assert_eq!(message["body"], "Hello");
    assert_eq!(message["next_run_at"], message["run_at"]);
    assert_eq!(message["cron"], Value::Null);
    assert_eq!(message["timezone"], "UTC");
    assert_eq!(message["enabled"], true);
}

#[sqlx::test]
async fn a_recurring_message_gets_its_next_match(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;

    let message = create(&app, recurring(), &session).await;

    assert_eq!(message["run_at"], Value::Null);
    let next: DateTime<Utc> = message["next_run_at"].as_str().unwrap().parse().unwrap();
    assert!(next > Utc::now());
    assert_eq!(next.format("%H:%M").to_string(), "03:00");
}

#[sqlx::test]
async fn a_group_is_addressed_by_its_id(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;

    let input = with(
        with(once(), "recipient_type", json!("group")),
        "recipient",
        json!("120363000000000000@g.us"),
    );
    let message = create(&app, input, &session).await;

    assert_eq!(message["recipient"], "120363000000000000@g.us");
}

#[sqlx::test]
async fn a_message_created_off_waits_for_no_run(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;

    let message = create(&app, with(once(), "enabled", json!(false)), &session).await;

    assert_eq!(message["next_run_at"], Value::Null);
}

#[sqlx::test]
async fn every_bad_field_is_named(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;
    let past = Utc::now() - TimeDelta::minutes(1);
    let group = with(once(), "recipient_type", json!("group"));
    let cases = [
        (with(once(), "recipient", json!("12345")), "recipient"),
        (with(once(), "recipient", json!("call me")), "recipient"),
        (
            with(
                group.clone(),
                "recipient",
                json!("15550001234@s.whatsapp.net"),
            ),
            "recipient",
        ),
        (with(group, "recipient", json!("nonsense")), "recipient"),
        (
            with(once(), "recipient_name", json!("a".repeat(256))),
            "recipient_name",
        ),
        (with(once(), "body", json!("   ")), "body"),
        (with(once(), "body", json!("x".repeat(4097))), "body"),
        (with(once(), "run_at", json!(past)), "run_at"),
        (with(once(), "run_at", Value::Null), "run_at"),
        (with(recurring(), "cron", Value::Null), "cron"),
        (with(recurring(), "cron", json!("*/5 * * * * *")), "cron"),
        (with(recurring(), "cron", json!("0 0 31 2 *")), "cron"),
        (
            with(recurring(), "timezone", json!("Mars/Base")),
            "timezone",
        ),
    ];
    for (input, field) in cases {
        let response = app
            .call(Method::POST, "/messages", Some(input), Some(&session))
            .await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST, "{field}");
        assert!(
            response.body["details"][field].is_array(),
            "{field}: {}",
            response.body
        );
    }
}

#[sqlx::test]
async fn a_message_can_change_from_once_to_recurring(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;
    let message = create(&app, once(), &session).await;

    let updated = app
        .call(
            Method::PUT,
            &path_of(&message),
            Some(recurring()),
            Some(&session),
        )
        .await;

    assert_eq!(updated.status, StatusCode::OK, "{}", updated.body);
    assert_eq!(updated.body["run_at"], Value::Null);
    assert_eq!(updated.body["cron"], "0 9 * * *");
    assert_ne!(updated.body["next_run_at"], message["next_run_at"]);
}

#[sqlx::test]
async fn switching_off_clears_the_next_run_and_on_rearms_it(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;
    let message = create(&app, recurring(), &session).await;
    let path = path_of(&message);

    let off = app
        .call(
            Method::PATCH,
            &path,
            Some(json!({ "enabled": false })),
            Some(&session),
        )
        .await;
    assert_eq!(off.body["enabled"], false);
    assert_eq!(off.body["next_run_at"], Value::Null);

    let on = app
        .call(
            Method::PATCH,
            &path,
            Some(json!({ "enabled": true })),
            Some(&session),
        )
        .await;
    assert_eq!(on.body["enabled"], true);
    assert_eq!(on.body["next_run_at"], message["next_run_at"]);
}

#[sqlx::test]
async fn a_past_one_time_message_wont_switch_back_on(pool: PgPool) {
    let app = TestApp::new(pool.clone());
    let (_, session) = app.signed_in("ada@example.com").await;
    let message = create(&app, with(once(), "enabled", json!(false)), &session).await;
    let id: uuid::Uuid = message["id"].as_str().unwrap().parse().unwrap();
    sqlx::query!(
        "UPDATE scheduled_messages SET run_at = now() - interval '1 hour' WHERE id = $1",
        id
    )
    .execute(&pool)
    .await
    .unwrap();

    let refused = app
        .call(
            Method::PATCH,
            &path_of(&message),
            Some(json!({ "enabled": true })),
            Some(&session),
        )
        .await;

    assert_eq!(refused.status, StatusCode::CONFLICT);
    let unchanged = app
        .call(Method::GET, &path_of(&message), None, Some(&session))
        .await;
    assert_eq!(unchanged.body["enabled"], false);
}

#[sqlx::test]
async fn only_your_own_template_can_be_named(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, owner) = app.signed_in("ada@example.com").await;
    let (_, stranger) = app.signed_in("grace@example.com").await;
    let template = app
        .call(
            Method::POST,
            "/templates",
            Some(json!({ "name": "Mine", "body": "x" })),
            Some(&owner),
        )
        .await;
    let template_id = template.body["id"].clone();

    let refused = app
        .call(
            Method::POST,
            "/messages",
            Some(with(once(), "template_id", template_id.clone())),
            Some(&stranger),
        )
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert!(refused.body["details"]["template_id"].is_array());

    let message = create(
        &app,
        with(once(), "template_id", template_id.clone()),
        &owner,
    )
    .await;
    assert_eq!(message["template_id"], template_id);

    let path = format!("/templates/{}", template_id.as_str().unwrap());
    app.call(Method::DELETE, &path, None, Some(&owner)).await;
    let kept = app
        .call(Method::GET, &path_of(&message), None, Some(&owner))
        .await;
    assert_eq!(kept.status, StatusCode::OK);
    assert_eq!(kept.body["template_id"], Value::Null);
    assert_eq!(kept.body["body"], "Hello");
}

#[sqlx::test]
async fn another_users_message_doesnt_exist(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, owner) = app.signed_in("ada@example.com").await;
    let (_, stranger) = app.signed_in("grace@example.com").await;
    let message = create(&app, once(), &owner).await;
    let path = path_of(&message);

    let cases = [
        (Method::GET, None),
        (Method::PUT, Some(recurring())),
        (Method::PATCH, Some(json!({ "enabled": false }))),
        (Method::DELETE, None),
    ];
    for (method, body) in cases {
        let response = app.call(method.clone(), &path, body, Some(&stranger)).await;
        assert_eq!(response.status, StatusCode::NOT_FOUND, "{method}");
    }
    let listed = app
        .call(Method::GET, "/messages", None, Some(&stranger))
        .await;
    assert_eq!(listed.body, json!([]));
    let kept = app.call(Method::GET, &path, None, Some(&owner)).await;
    assert_eq!(kept.body["enabled"], true);
}

#[sqlx::test]
async fn deleting_a_message_keeps_its_history(pool: PgPool) {
    let app = TestApp::new(pool.clone());
    let (user_id, session) = app.signed_in("ada@example.com").await;
    let message = create(&app, once(), &session).await;
    let id: uuid::Uuid = message["id"].as_str().unwrap().parse().unwrap();
    sqlx::query!(
        "INSERT INTO deliveries (user_id, scheduled_message_id, recipient_type, recipient, body) \
         VALUES ($1, $2, 'contact', '15550001234', 'Hello')",
        user_id,
        id,
    )
    .execute(&pool)
    .await
    .unwrap();

    let deleted = app
        .call(Method::DELETE, &path_of(&message), None, Some(&session))
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);

    let history = app
        .call(Method::GET, "/deliveries", None, Some(&session))
        .await;
    assert_eq!(history.body["total"], 1);
    assert_eq!(
        history.body["items"][0]["scheduled_message_id"],
        Value::Null
    );
}

#[sqlx::test]
async fn sending_now_needs_a_linked_account(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;

    let refused = app
        .call(
            Method::POST,
            "/messages/send-now",
            Some(json!({
                "recipient_type": "contact",
                "recipient": "15550001234",
                "body": "Now",
            })),
            Some(&session),
        )
        .await;

    assert_eq!(refused.status, StatusCode::CONFLICT);
    assert_eq!(refused.body["error"], "WhatsApp isn't linked");
    let history = app
        .call(Method::GET, "/deliveries", None, Some(&session))
        .await;
    assert_eq!(history.body["total"], 0);
}

#[sqlx::test]
async fn sending_now_checks_the_message_first(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;

    let refused = app
        .call(
            Method::POST,
            "/messages/send-now",
            Some(json!({
                "recipient_type": "contact",
                "recipient": "123",
                "body": "",
            })),
            Some(&session),
        )
        .await;

    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert!(refused.body["details"]["recipient"].is_array());
    assert!(refused.body["details"]["body"].is_array());
}
