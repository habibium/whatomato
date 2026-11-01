// No test here gets as far as linking, since that would open a real WhatsApp connection

use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::routes::auth::tests::{PASSWORD, TestApp, TestResponse};

async fn call(
    app: &TestApp,
    method: Method,
    path: &str,
    body: Option<Value>,
    cookie: Option<&str>,
) -> TestResponse {
    let mut request = Request::builder()
        .method(method)
        .uri(format!("/api/whatsapp{path}"))
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
    app.send(request.body(body).unwrap()).await
}

#[sqlx::test]
async fn the_status_needs_a_session_and_starts_unlinked(pool: PgPool) {
    let app = TestApp::new(pool);
    let anonymous = call(&app, Method::GET, "/status", None, None).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);

    let (_, session) = app.signed_in("ada@example.com").await;
    let status = call(&app, Method::GET, "/status", None, Some(&session)).await;
    assert_eq!(status.status, StatusCode::OK);
    assert_eq!(status.body, json!({ "status": "not_linked" }));
}

#[sqlx::test]
async fn commands_for_a_link_conflict_before_there_is_one(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;
    let cases = [
        (
            Method::POST,
            "/link/cancel",
            None,
            "WhatsApp isn't being linked",
        ),
        (Method::POST, "/pause", None, "WhatsApp isn't connected"),
        (
            Method::POST,
            "/resume",
            None,
            "WhatsApp isn't paused or stopped",
        ),
        (Method::GET, "/groups", None, "WhatsApp isn't connected"),
        (
            Method::POST,
            "/test-message",
            None,
            "WhatsApp isn't connected",
        ),
        (
            Method::POST,
            "/unlink",
            Some(json!({ "password": PASSWORD })),
            "WhatsApp isn't linked",
        ),
    ];
    for (method, path, body, message) in cases {
        let response = call(&app, method, path, body, Some(&session)).await;
        assert_eq!(response.status, StatusCode::CONFLICT, "{path}");
        assert_eq!(response.body["error"], message, "{path}");
    }
}

#[sqlx::test]
async fn linking_and_unlinking_check_the_password(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;
    let wrong = json!({ "password": "not the password" });
    for path in ["/link", "/unlink"] {
        let response = call(
            &app,
            Method::POST,
            path,
            Some(wrong.clone()),
            Some(&session),
        )
        .await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST, "{path}");
    }

    let status = call(&app, Method::GET, "/status", None, Some(&session)).await;
    assert_eq!(status.body, json!({ "status": "not_linked" }));
}

#[sqlx::test]
async fn linking_refuses_a_number_that_cannot_be_whole(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;
    let response = call(
        &app,
        Method::POST,
        "/link",
        Some(json!({ "password": PASSWORD, "phone_number": "+1 555" })),
        Some(&session),
    )
    .await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST);
    assert!(
        response.body["details"]["phone_number"].is_array(),
        "{}",
        response.body
    );
}
