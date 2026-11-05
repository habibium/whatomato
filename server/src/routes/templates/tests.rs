use axum::http::{Method, StatusCode};
use serde_json::json;
use sqlx::PgPool;

use crate::routes::auth::tests::TestApp;

#[sqlx::test]
async fn templates_need_a_session(pool: PgPool) {
    let app = TestApp::new(pool);
    let response = app.call(Method::GET, "/templates", None, None).await;
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn a_template_can_be_saved_changed_and_deleted(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;
    let cookie = Some(session.as_str());

    let created = app
        .call(
            Method::POST,
            "/templates",
            Some(json!({ "name": "  Friday recap ", "body": " Hello \n" })),
            cookie,
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    assert_eq!(created.body["name"], "Friday recap");
    assert_eq!(created.body["body"], "Hello");
    let path = format!("/templates/{}", created.body["id"].as_str().unwrap());

    let updated = app
        .call(
            Method::PUT,
            &path,
            Some(json!({ "name": "Recap", "body": "Hi" })),
            cookie,
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK);
    assert_eq!(updated.body["name"], "Recap");

    let fetched = app.call(Method::GET, &path, None, cookie).await;
    assert_eq!(fetched.body["body"], "Hi");

    let deleted = app.call(Method::DELETE, &path, None, cookie).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let gone = app.call(Method::GET, &path, None, cookie).await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn templates_list_by_name_whatever_the_case(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;
    let cookie = Some(session.as_str());
    for name in ["banana", "Apple", "cherry"] {
        app.call(
            Method::POST,
            "/templates",
            Some(json!({ "name": name, "body": "x" })),
            cookie,
        )
        .await;
    }

    let listed = app.call(Method::GET, "/templates", None, cookie).await;
    let names: Vec<&str> = listed
        .body
        .as_array()
        .unwrap()
        .iter()
        .map(|template| template["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Apple", "banana", "cherry"]);
}

#[sqlx::test]
async fn a_template_needs_a_name_and_a_body_of_a_sane_length(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in("ada@example.com").await;
    let cases = [
        (json!({ "name": "  ", "body": "x" }), "name"),
        (json!({ "name": "a".repeat(101), "body": "x" }), "name"),
        (json!({ "name": "a", "body": " \n " }), "body"),
        (json!({ "name": "a", "body": "x".repeat(4097) }), "body"),
    ];
    for (input, field) in cases {
        let response = app
            .call(Method::POST, "/templates", Some(input), Some(&session))
            .await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST, "{field}");
        assert!(response.body["details"][field].is_array(), "{field}");
    }
}

#[sqlx::test]
async fn another_users_template_doesnt_exist(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, owner) = app.signed_in("ada@example.com").await;
    let (_, stranger) = app.signed_in("grace@example.com").await;
    let created = app
        .call(
            Method::POST,
            "/templates",
            Some(json!({ "name": "Mine", "body": "x" })),
            Some(&owner),
        )
        .await;
    let path = format!("/templates/{}", created.body["id"].as_str().unwrap());

    let cases = [
        (Method::GET, None),
        (Method::PUT, Some(json!({ "name": "Theirs", "body": "y" }))),
        (Method::DELETE, None),
    ];
    for (method, body) in cases {
        let response = app.call(method.clone(), &path, body, Some(&stranger)).await;
        assert_eq!(response.status, StatusCode::NOT_FOUND, "{method}");
    }
    let listed = app
        .call(Method::GET, "/templates", None, Some(&stranger))
        .await;
    assert_eq!(listed.body, json!([]));

    let kept = app.call(Method::GET, &path, None, Some(&owner)).await;
    assert_eq!(kept.body["name"], "Mine");
}
