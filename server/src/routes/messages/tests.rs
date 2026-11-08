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
        "recurrence": { "type": "cron", "expression": "0 9 * * *" },
        "timezone": "Asia/Dhaka",
    })
}

fn sunset(place: &Value) -> Value {
    json!({ "type": "sun", "event": "sunset", "offset_minutes": -10, "days": "* * *", "place": place })
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
    assert_eq!(message["recurrence"], Value::Null);
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
        (with(recurring(), "recurrence", Value::Null), "recurrence"),
        (
            with(
                recurring(),
                "recurrence",
                json!({ "type": "cron", "expression": "*/5 * * * * *" }),
            ),
            "recurrence",
        ),
        (
            with(
                recurring(),
                "recurrence",
                json!({ "type": "cron", "expression": "0 0 31 2 *" }),
            ),
            "recurrence",
        ),
        (
            with(
                with(recurring(), "recurrence", sunset(&Value::Null)),
                "timezone",
                json!("UTC"),
            ),
            "recurrence",
        ),
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
    assert_eq!(updated.body["recurrence"]["expression"], "0 9 * * *");
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

#[sqlx::test]
async fn a_message_names_only_variables_that_exist(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;

    let saved = create(
        &app,
        with(
            once(),
            "body",
            json!("Hi {{first_name|there}}, see you {{day}}"),
        ),
        &session,
    )
    .await;
    assert_eq!(saved["body"], "Hi {{first_name|there}}, see you {{day}}");

    for (body, message) in [
        ("Hi {{firstname}}", "{{firstname}} isn't a variable"),
        ("Hi {{first_name", "a {{ needs a }} to close it"),
    ] {
        let refused = app
            .call(
                Method::POST,
                "/messages",
                Some(with(once(), "body", json!(body))),
                Some(&session),
            )
            .await;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST);
        assert_eq!(refused.body["details"]["body"][0], message);
    }
}

#[sqlx::test]
async fn a_sun_schedule_moves_with_the_sun_and_keeps_a_place_to_a_tenth_of_a_degree(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;
    let input = with(
        recurring(),
        "recurrence",
        sunset(&json!({ "latitude": 23.8103, "longitude": 90.4125 })),
    );

    let message = create(&app, input, &session).await;

    assert_eq!(
        message["recurrence"]["place"],
        json!({ "latitude": 23.8, "longitude": 90.4 })
    );
    let next: DateTime<Utc> = message["next_run_at"].as_str().unwrap().parse().unwrap();
    assert!(next > Utc::now());
    // Ten minutes before sunset in Dhaka, which runs from about 5:10 to 6:50 PM through the year
    let local = next
        .with_timezone(&chrono_tz::Asia::Dhaka)
        .format("%H:%M")
        .to_string();
    assert!(("17:00".."18:41").contains(&local.as_str()), "{local}");
}

fn next_runs_path(recurrence: &Value, timezone: &str) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("recurrence", &recurrence.to_string())
        .append_pair("timezone", timezone)
        .finish();
    format!("/messages/next-runs?{query}")
}

#[sqlx::test]
async fn next_runs_show_a_schedule_before_it_is_saved(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;

    let response = app
        .call(
            Method::GET,
            &next_runs_path(&sunset(&Value::Null), "Asia/Dhaka"),
            None,
            Some(&session),
        )
        .await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert_eq!(response.body["city"], "Dhaka");
    let runs: Vec<DateTime<Utc>> = serde_json::from_value(response.body["runs"].clone()).unwrap();
    assert_eq!(runs.len(), 3);
    assert!(runs[0] > Utc::now());
    for pair in runs.windows(2) {
        let gap = pair[1] - pair[0];
        assert!(
            gap > TimeDelta::hours(23) && gap < TimeDelta::hours(25),
            "{gap}"
        );
    }

    let cron = json!({ "type": "cron", "expression": "0 9 * * *" });
    let response = app
        .call(
            Method::GET,
            &next_runs_path(&cron, "UTC"),
            None,
            Some(&session),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert_eq!(response.body["city"], Value::Null);
}

#[sqlx::test]
async fn next_runs_name_what_is_wrong(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;

    let path = next_runs_path(&sunset(&Value::Null), "UTC");
    let response = app.call(Method::GET, &path, None, Some(&session)).await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        response.body["details"]["recurrence"][0],
        "needs a place, as its time zone has no city to go by"
    );

    let response = app.call(Method::GET, &path, None, None).await;
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);
}

fn preview_path(pairs: &[(&str, &str)]) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    for (name, value) in pairs {
        query.append_pair(name, value);
    }
    format!("/messages/preview?{}", query.finish())
}

#[sqlx::test]
async fn a_preview_fills_in_what_would_go_out(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;
    let path = preview_path(&[
        (
            "body",
            "{{name}}: Maghrib at {{sunset + 10m}}, {{first_name}}",
        ),
        ("timezone", "Asia/Dhaka"),
        ("recipient_type", "group"),
        ("recipient", "120363000000000000@g.us"),
        ("recipient_name", "Family"),
        ("run_at", "2026-11-08T12:00:00Z"),
    ]);

    let response = app.call(Method::GET, &path, None, Some(&session)).await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert_eq!(response.body["text"], "Family: Maghrib at 5:26 PM, ");
    assert_eq!(response.body["problem"], Value::Null);
    assert_eq!(response.body["values"]["sunset"], "5:16 PM");
    assert_eq!(response.body["values"]["sunrise"], "6:08 AM");
    assert_eq!(response.body["values"]["first_name"], Value::Null);
    assert_eq!(response.body["empty"], json!(["first_name"]));
}

#[sqlx::test]
async fn a_preview_reads_a_schedules_next_run_and_the_place_it_names(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;
    let near_dhaka = json!({ "latitude": 23.8, "longitude": 90.4 });
    let path = preview_path(&[
        ("body", "{{time}}|{{sunset}}"),
        ("timezone", "Asia/Dhaka"),
        ("recurrence", &sunset(&near_dhaka).to_string()),
    ]);

    let response = app.call(Method::GET, &path, None, Some(&session)).await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    let text = response.body["text"].as_str().unwrap();
    let (time, sunset) = text.split_once('|').unwrap();
    // Ten minutes before sunset, as the schedule says
    let minutes = |clock: &str| {
        let time = chrono::NaiveTime::parse_from_str(clock, "%-I:%M %p").unwrap();
        time.signed_duration_since(chrono::NaiveTime::MIN)
            .num_minutes()
    };
    assert_eq!(minutes(sunset) - minutes(time), 10, "{text}");
}

#[sqlx::test]
async fn a_preview_names_what_is_wrong_and_still_gives_the_values(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;
    let path = preview_path(&[
        ("body", "{{first_name + 10m}} and {{"),
        ("timezone", "Asia/Dhaka"),
    ]);

    let response = app.call(Method::GET, &path, None, Some(&session)).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert_eq!(
        response.body["problem"],
        "{{first_name}} isn't a time, so it can't move"
    );
    assert_eq!(response.body["text"], "{{first_name + 10m}} and {{");
    assert!(response.body["values"]["sunset"].is_string());

    let too_long = preview_path(&[("body", &"x".repeat(4097)), ("timezone", "UTC")]);
    let response = app.call(Method::GET, &too_long, None, Some(&session)).await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST);

    let response = app.call(Method::GET, &path, None, None).await;
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);
}
