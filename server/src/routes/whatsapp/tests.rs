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
    assert_eq!(
        status.body,
        json!({ "status": "not_linked", "reason": null, "can_link": true })
    );
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
    for path in ["/link", "/unlink", "/relink"] {
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
    assert_eq!(
        status.body,
        json!({ "status": "not_linked", "reason": null, "can_link": true })
    );
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

#[sqlx::test]
async fn contacts_list_by_name_with_numbers_from_lid_mappings(pool: PgPool) {
    use crate::{
        crypto::Cipher,
        db::{whatsapp_accounts, whatsapp_contacts},
        whatsapp::contacts::{Names, seal_names},
    };

    let app = TestApp::new(pool.clone());
    let (user_id, session) = app.signed_in("ada@example.com").await;
    let empty = call(&app, Method::GET, "/contacts", None, Some(&session)).await;
    assert_eq!(empty.status, StatusCode::OK);
    assert_eq!(empty.body, json!({ "imported": false, "contacts": [] }));

    let cipher = Cipher::new(&[0; 32]);
    let account_id = whatsapp_accounts::find_or_create(&pool, user_id)
        .await
        .unwrap()
        .id;
    let contacts = [
        (
            "15550001@s.whatsapp.net",
            Some("15550001"),
            None,
            Some("zoe"),
            None,
        ),
        (
            "100000000000001@lid",
            None,
            Some("100000000000001"),
            Some("Ada"),
            Some("Ada"),
        ),
        (
            "100000000000002@lid",
            None,
            Some("100000000000002"),
            Some("bob"),
            None,
        ),
        (
            "15550004@s.whatsapp.net",
            Some("15550004"),
            None,
            None,
            None,
        ),
    ];
    for (jid, number, lid, full_name, first_name) in contacts {
        let names = Names {
            full_name: full_name.map(str::to_owned),
            first_name: first_name.map(str::to_owned),
            username: None,
        };
        let sealed = seal_names(&cipher, account_id, jid, &names).unwrap();
        whatsapp_contacts::save(&pool, account_id, jid, number, lid, &sealed)
            .await
            .unwrap();
    }
    sqlx::query!(
        "INSERT INTO whatsapp_lid_mappings \
         (account_id, lid, phone_number, learning_source, created_at, updated_at) \
         VALUES ($1, '100000000000001', '15550002', 'other', 0, 0)",
        account_id
    )
    .execute(&pool)
    .await
    .unwrap();
    whatsapp_contacts::mark_imported(&pool, account_id)
        .await
        .unwrap();

    let listed = call(&app, Method::GET, "/contacts", None, Some(&session)).await;
    assert_eq!(listed.status, StatusCode::OK);
    assert_eq!(
        listed.body,
        json!({
            "imported": true,
            "contacts": [
                {
                    "id": "100000000000001@lid",
                    "name": "Ada",
                    "first_name": "Ada",
                    "phone_number": "15550002",
                    "username": null,
                },
                {
                    "id": "100000000000002@lid",
                    "name": "bob",
                    "first_name": null,
                    "phone_number": null,
                    "username": null,
                },
                {
                    "id": "15550001@s.whatsapp.net",
                    "name": "zoe",
                    "first_name": null,
                    "phone_number": "15550001",
                    "username": null,
                },
            ],
        })
    );
}

#[sqlx::test]
async fn pictures_are_only_for_saved_contacts_and_groups(pool: PgPool) {
    use crate::{
        crypto::Cipher,
        db::{whatsapp_accounts, whatsapp_contacts},
        whatsapp::contacts::{Names, seal_names},
    };

    let app = TestApp::new(pool.clone());
    let (user_id, session) = app.signed_in("ada@example.com").await;
    let account_id = whatsapp_accounts::find_or_create(&pool, user_id)
        .await
        .unwrap()
        .id;
    let names = Names {
        full_name: Some("Grace".into()),
        first_name: None,
        username: None,
    };
    let saved = "15550001@s.whatsapp.net";
    let sealed = seal_names(&Cipher::new(&[0; 32]), account_id, saved, &names).unwrap();
    whatsapp_contacts::save(&pool, account_id, saved, Some("15550001"), None, &sealed)
        .await
        .unwrap();

    // Nothing is linked, so even an allowed picture can't be looked up
    for id in [
        "15550001%40s.whatsapp.net",
        "15550002%40s.whatsapp.net",
        "120363000000000000%40g.us",
        "not-a-jid",
    ] {
        let path = format!("/pictures/{id}");
        let response = call(&app, Method::GET, &path, None, Some(&session)).await;
        assert_eq!(response.status, StatusCode::NOT_FOUND, "{id}");
    }
    let anonymous = call(&app, Method::GET, "/pictures/x", None, None).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    assert!(
        whatsapp_contacts::is_saved(&pool, user_id, saved)
            .await
            .unwrap()
    );
    assert!(
        !whatsapp_contacts::is_saved(&pool, user_id, "15550002@s.whatsapp.net")
            .await
            .unwrap()
    );
}

async fn link_account(pool: &PgPool, user_id: uuid::Uuid, phone_number: Option<&str>) {
    sqlx::query("INSERT INTO whatsapp_accounts (user_id, phone_number, linked_at) VALUES ($1, $2, CASE WHEN $2 IS NULL THEN NULL ELSE now() END)")
        .bind(user_id)
        .bind(phone_number)
        .execute(pool)
        .await
        .unwrap();
}

async fn can_link(app: &TestApp, session: &str) -> Value {
    let status = call(app, Method::GET, "/status", None, Some(session)).await;
    assert_eq!(status.status, StatusCode::OK);
    status.body["can_link"].clone()
}

#[sqlx::test]
async fn under_the_limit_anyone_can_link(pool: PgPool) {
    let app = TestApp::with_max_linked_accounts(pool.clone(), 2);
    let (grace, _) = app.signed_in("grace@example.com").await;
    link_account(&pool, grace, Some("15550001")).await;

    let (_, session) = app.signed_in("ada@example.com").await;
    assert_eq!(can_link(&app, &session).await, json!(true));
}

#[sqlx::test]
async fn at_the_limit_a_new_user_cannot_start_linking(pool: PgPool) {
    let app = TestApp::with_max_linked_accounts(pool.clone(), 1);
    let (grace, _) = app.signed_in("grace@example.com").await;
    link_account(&pool, grace, Some("15550001")).await;

    let (_, session) = app.signed_in("ada@example.com").await;
    assert_eq!(can_link(&app, &session).await, json!(false));
    let response = call(
        &app,
        Method::POST,
        "/link",
        Some(json!({ "password": PASSWORD })),
        Some(&session),
    )
    .await;
    assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response.body["error"],
        "There's no room for another WhatsApp account on this server right now. Try again later."
    );
}

#[sqlx::test]
async fn a_linked_user_keeps_their_place_at_or_over_the_limit(pool: PgPool) {
    let app = TestApp::with_max_linked_accounts(pool.clone(), 1);
    let (grace, grace_session) = app.signed_in("grace@example.com").await;
    let (ada, session) = app.signed_in("ada@example.com").await;
    link_account(&pool, grace, Some("15550001")).await;
    link_account(&pool, ada, Some("15550002")).await;

    assert_eq!(can_link(&app, &session).await, json!(true));
    assert_eq!(can_link(&app, &grace_session).await, json!(true));
}

#[sqlx::test]
async fn an_abandoned_pairing_holds_no_place(pool: PgPool) {
    let app = TestApp::with_max_linked_accounts(pool.clone(), 1);
    let (grace, _) = app.signed_in("grace@example.com").await;
    link_account(&pool, grace, None).await;

    let (_, session) = app.signed_in("ada@example.com").await;
    assert_eq!(can_link(&app, &session).await, json!(true));
}

#[sqlx::test]
async fn history_starts_off_and_keeps_what_the_owner_chooses(pool: PgPool) {
    let app = TestApp::new(pool);
    let (user_id, session) = app.signed_in("ada@example.com").await;
    let history = call(&app, Method::GET, "/history", None, Some(&session)).await;
    assert_eq!(history.status, StatusCode::OK);
    assert_eq!(
        history.body,
        json!({ "keep": "off", "available": true, "import": null })
    );

    let month = json!({ "keep": { "days": 30 } });
    let saved = call(&app, Method::PUT, "/history", Some(month), Some(&session)).await;
    assert_eq!(saved.status, StatusCode::OK);
    assert_eq!(saved.body["keep"], json!({ "days": 30 }));

    let everything = json!({ "keep": "all" });
    let saved = call(
        &app,
        Method::PUT,
        "/history",
        Some(everything),
        Some(&session),
    )
    .await;
    assert_eq!(saved.body["keep"], "all");

    // A linked account shows how the phone's history is coming in
    link_account(&app.pool, user_id, Some("8801700000009")).await;
    sqlx::query(
        "UPDATE whatsapp_accounts SET history_progress = 40, history_imported = 12
         WHERE user_id = $1",
    )
    .bind(user_id)
    .execute(&app.pool)
    .await
    .unwrap();
    let history = call(&app, Method::GET, "/history", None, Some(&session)).await;
    assert_eq!(
        history.body["import"],
        json!({ "progress": 40, "imported": 12, "waiting": 0 })
    );

    let off = json!({ "keep": "off" });
    let saved = call(&app, Method::PUT, "/history", Some(off), Some(&session)).await;
    assert_eq!(saved.body["keep"], "off");
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM message_history")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

#[sqlx::test]
async fn history_refuses_a_length_out_of_bounds_and_a_server_that_doesnt_read(pool: PgPool) {
    let app = TestApp::new(pool.clone());
    let (_, session) = app.signed_in("ada@example.com").await;
    for days in [0, 3651] {
        let body = json!({ "keep": { "days": days } });
        let refused = call(&app, Method::PUT, "/history", Some(body), Some(&session)).await;
        assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY, "{days}");
    }

    let app = TestApp::without_agent_access(pool);
    let (_, session) = app.signed_in("grace@example.com").await;
    let body = json!({ "keep": "all" });
    let refused = call(&app, Method::PUT, "/history", Some(body), Some(&session)).await;
    assert_eq!(refused.status, StatusCode::CONFLICT);
    let history = call(&app, Method::GET, "/history", None, Some(&session)).await;
    assert_eq!(history.body["available"], false);
    assert_eq!(history.body["keep"], "off");
}
