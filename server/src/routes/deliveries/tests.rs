use axum::http::{Method, StatusCode};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::routes::auth::tests::TestApp;

async fn delivery(pool: &PgPool, user_id: Uuid, body: &str, status: &str, age_minutes: f64) {
    sqlx::query!(
        "INSERT INTO deliveries (user_id, recipient_type, recipient, body, status, created_at) \
         VALUES ($1, 'contact', '15550001', $2, $3, now() - make_interval(mins => 0, secs => $4))",
        user_id,
        body,
        status,
        age_minutes * 60.0,
    )
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test]
async fn the_history_needs_a_session(pool: PgPool) {
    let app = TestApp::new(pool);
    let response = app.call(Method::GET, "/deliveries", None, None).await;
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn the_history_pages_through_newest_first(pool: PgPool) {
    let app = TestApp::new(pool.clone());
    let (user_id, session) = app.signed_in("ada@example.com").await;
    delivery(&pool, user_id, "oldest", "sent", 3.0).await;
    delivery(&pool, user_id, "middle", "failed", 2.0).await;
    delivery(&pool, user_id, "newest", "pending", 1.0).await;

    let page = app
        .call(
            Method::GET,
            "/deliveries?limit=2&offset=1",
            None,
            Some(&session),
        )
        .await;
    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(page.body["total"], 3);
    assert_eq!(page.body["limit"], 2);
    assert_eq!(page.body["offset"], 1);
    assert_eq!(page.body["items"][0]["body"], "middle");
    assert_eq!(page.body["items"][1]["body"], "oldest");
}

#[sqlx::test]
async fn a_send_in_progress_reads_as_pending(pool: PgPool) {
    let app = TestApp::new(pool.clone());
    let (user_id, session) = app.signed_in("ada@example.com").await;
    delivery(&pool, user_id, "going", "sending", 1.0).await;

    let page = app
        .call(Method::GET, "/deliveries", None, Some(&session))
        .await;
    assert_eq!(page.body["items"][0]["status"], "pending");
}

#[sqlx::test]
async fn a_bad_page_is_refused(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;
    for query in ["limit=0", "limit=101", "offset=-1", "limit=many"] {
        let response = app
            .call(
                Method::GET,
                &format!("/deliveries?{query}"),
                None,
                Some(&session),
            )
            .await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST, "{query}");
    }
}

#[sqlx::test]
async fn nobody_sees_another_users_history(pool: PgPool) {
    let app = TestApp::new(pool.clone());
    let (owner, _) = app.signed_in("ada@example.com").await;
    let (_, stranger) = app.signed_in("grace@example.com").await;
    delivery(&pool, owner, "private", "sent", 1.0).await;

    let page = app
        .call(Method::GET, "/deliveries", None, Some(&stranger))
        .await;
    assert_eq!(page.body["total"], 0);
    assert_eq!(page.body["items"], json!([]));
}
