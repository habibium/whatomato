// Through the real router. `Push::outbox` stands in for the push services, so nothing leaves.

use axum::http::{Method, StatusCode};
use base64::prelude::{BASE64_URL_SAFE_NO_PAD, Engine as _};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::routes::auth::tests::{TestApp, TestResponse};

const ENDPOINT: &str = "https://fcm.googleapis.com/fcm/send/abc:def";

fn subscription(endpoint: &str) -> Value {
    json!({
        "endpoint": endpoint,
        "p256dh": BASE64_URL_SAFE_NO_PAD.encode([4; 65]),
        "auth": BASE64_URL_SAFE_NO_PAD.encode([7; 16]),
        "label": "Chrome on Linux",
    })
}

async fn subscribe(app: &TestApp, session: &str, body: Value) -> TestResponse {
    app.call(
        Method::POST,
        "/push/subscriptions",
        Some(body),
        Some(session),
    )
    .await
}

async fn subscriptions(app: &TestApp, session: &str) -> Vec<Value> {
    let status = app.call(Method::GET, "/push", None, Some(session)).await;
    assert_eq!(status.status, StatusCode::OK);
    status.body["subscriptions"].as_array().unwrap().clone()
}

#[sqlx::test]
async fn a_browser_turns_notifications_on_and_off(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;

    let status = app.call(Method::GET, "/push", None, Some(&session)).await;
    assert_eq!(status.status, StatusCode::OK);
    let key = status.body["public_key"].as_str().unwrap();
    assert_eq!(BASE64_URL_SAFE_NO_PAD.decode(key).unwrap().len(), 65);
    assert_eq!(status.body["subscriptions"], json!([]));

    let on = subscribe(&app, &session, subscription(ENDPOINT)).await;
    assert_eq!(on.status, StatusCode::OK, "{}", on.body);
    assert_eq!(on.body["endpoint"], ENDPOINT);
    assert_eq!(on.body["label"], "Chrome on Linux");
    // The keys never come back
    assert!(on.body.get("p256dh").is_none() && on.body.get("auth").is_none());
    // A browser sends its subscription again each time the page loads
    let again = subscribe(&app, &session, subscription(ENDPOINT)).await;
    assert_eq!(again.body["id"], on.body["id"]);
    assert_eq!(subscriptions(&app, &session).await.len(), 1);

    for _ in 0..2 {
        let off = app
            .call(
                Method::POST,
                "/push/unsubscribe",
                Some(json!({ "endpoint": ENDPOINT })),
                Some(&session),
            )
            .await;
        assert_eq!(off.status, StatusCode::NO_CONTENT);
    }
    assert_eq!(subscriptions(&app, &session).await.len(), 0);

    for (method, path) in [
        (Method::GET, "/push"),
        (Method::POST, "/push/subscriptions"),
        (Method::POST, "/push/unsubscribe"),
        (Method::POST, "/push/test"),
    ] {
        let signed_out = app
            .call(method, path, Some(subscription(ENDPOINT)), None)
            .await;
        assert_eq!(signed_out.status, StatusCode::UNAUTHORIZED, "{path}");
    }
}

#[sqlx::test]
async fn only_a_known_browsers_subscription_is_taken(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;
    let with = |field: &str, value: Value| {
        let mut body = subscription(ENDPOINT);
        body[field] = value;
        body
    };

    let cases = [
        ("endpoint", json!("https://evil.example/push/abc")),
        ("endpoint", json!("http://fcm.googleapis.com/fcm/send/abc")),
        ("endpoint", json!("https://169.254.169.254/latest")),
        ("endpoint", json!("")),
        ("p256dh", json!(BASE64_URL_SAFE_NO_PAD.encode([4; 64]))),
        ("p256dh", json!("not base64!")),
        ("auth", json!(BASE64_URL_SAFE_NO_PAD.encode([7; 15]))),
        ("label", json!("  ")),
        ("label", json!("a".repeat(61))),
    ];
    for (field, value) in cases {
        let refused = subscribe(&app, &session, with(field, value.clone())).await;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{field}: {value}");
        let named = if field == "auth" { "p256dh" } else { field };
        assert!(
            refused.body["details"][named].is_array(),
            "{field}: {value}"
        );
    }
    assert_eq!(subscriptions(&app, &session).await.len(), 0);

    // Browsers hand the keys over padded or not
    let padded = with(
        "auth",
        json!(format!("{}==", BASE64_URL_SAFE_NO_PAD.encode([7; 16]))),
    );
    assert_eq!(
        subscribe(&app, &session, padded).await.status,
        StatusCode::OK
    );
}

#[sqlx::test]
async fn a_browser_notifies_for_whoever_turned_it_on_last(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, ada) = app.signed_in("ada@example.com").await;
    let (_, grace) = app.signed_in("grace@example.com").await;

    subscribe(&app, &ada, subscription(ENDPOINT)).await;
    assert_eq!(
        subscribe(&app, &grace, subscription(ENDPOINT)).await.status,
        StatusCode::OK
    );
    assert_eq!(subscriptions(&app, &ada).await.len(), 0);
    assert_eq!(subscriptions(&app, &grace).await.len(), 1);

    // Ada can't turn off what is now Grace's
    let off = app
        .call(
            Method::POST,
            "/push/unsubscribe",
            Some(json!({ "endpoint": ENDPOINT })),
            Some(&ada),
        )
        .await;
    assert_eq!(off.status, StatusCode::NO_CONTENT);
    assert_eq!(subscriptions(&app, &grace).await.len(), 1);
}

#[sqlx::test]
async fn an_account_keeps_its_ten_newest_browsers(pool: PgPool) {
    let app = TestApp::new(pool);
    let (ada, _) = app.signed_in("ada@example.com").await;

    // Straight to the database: eleven subscriptions through the API would meet its rate limit
    for n in 0..11 {
        let endpoint = format!("{ENDPOINT}{n}");
        crate::db::push_subscriptions::save(
            &app.pool,
            crate::db::push_subscriptions::NewSubscription {
                user_id: ada,
                endpoint: &endpoint,
                p256dh: &[4; 65],
                auth: &[7; 16],
                label: "Chrome on Linux",
            },
        )
        .await
        .unwrap();
        sqlx::query("UPDATE push_subscriptions SET created_at = now() - make_interval(secs => 100 - $2) WHERE endpoint = $1")
            .bind(&endpoint)
            .bind(f64::from(n))
            .execute(&app.pool)
            .await
            .unwrap();
    }
    let kept = crate::db::push_subscriptions::list(&app.pool, ada)
        .await
        .unwrap();
    assert_eq!(kept.len(), 10);
    assert_eq!(kept[0].endpoint, format!("{ENDPOINT}1"));
    assert_eq!(kept[9].endpoint, format!("{ENDPOINT}10"));
}

#[sqlx::test]
async fn a_test_notification_goes_to_each_of_the_accounts_browsers(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, ada) = app.signed_in("ada@example.com").await;
    let (_, grace) = app.signed_in("grace@example.com").await;
    subscribe(&app, &ada, subscription(ENDPOINT)).await;
    subscribe(
        &app,
        &ada,
        subscription("https://updates.push.services.mozilla.com/wpush/v2/x"),
    )
    .await;
    subscribe(
        &app,
        &grace,
        subscription("https://web.push.apple.com/graces"),
    )
    .await;

    let tested = app.call(Method::POST, "/push/test", None, Some(&ada)).await;
    assert_eq!(tested.status, StatusCode::OK);
    assert_eq!(tested.body["sent"], 2);
    let sent = app.state.push.sent();
    assert_eq!(sent.len(), 2);
    assert!(
        sent.iter()
            .all(|(endpoint, _)| !endpoint.contains("graces"))
    );
    assert_eq!(sent[0].1.title, "Notifications are on");
    assert_eq!(sent[0].1.url, "/settings");
}
