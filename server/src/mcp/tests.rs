// Through the real router, as an agent calls it. WhatsApp shows as connected through
// `WhatsApp::pretend`, which starts no client, so nothing is sent.

use axum::{
    body::Body,
    http::{Request, StatusCode, header, request::Builder},
};
use chrono::{TimeDelta, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::tools::INSTRUCTIONS;
use crate::{
    agents::{
        Grant, Preset,
        grant::{Chat, ChatRule, Contacts, Schedules, Scope, SendLevel, Strangers, Templates},
        key::AccessKey,
    },
    db::{
        agent_connections::{self, NewConnection},
        deliveries::{self, NewDelivery},
        message_templates, whatsapp_accounts, whatsapp_contacts,
    },
    routes::auth::tests::{TestApp, TestResponse},
    scheduler::RecipientType,
    whatsapp::{
        contacts::{Names, seal_names},
        status::Status,
    },
};

const LEGACY: &str = "2025-11-25";
const MODERN: &str = "2026-07-28";
const HOST: &str = "localhost:5173";
const ADA: &str = "8801700000001";
const BLOCKED: &str = "8801700000002";

const ALL_TOOLS: [&str; 11] = [
    "delete_scheduled_message",
    "find_chats",
    "get_approval",
    "list_approvals",
    "list_deliveries",
    "list_scheduled_messages",
    "list_templates",
    "schedule_message",
    "send_message",
    "update_scheduled_message",
    "whoami",
];
const ALWAYS: [&str; 4] = [
    "get_approval",
    "list_approvals",
    "list_deliveries",
    "whoami",
];

async fn user(app: &TestApp) -> Uuid {
    sqlx::query_scalar("INSERT INTO users (email, password) VALUES ($1, 'unused') RETURNING id")
        .bind(format!("{}@example.com", Uuid::new_v4()))
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

/// A connection made straight in the database, and its key
async fn connection(app: &TestApp, owner: Uuid, grant: &Grant) -> (String, Uuid) {
    let key = AccessKey::generate().unwrap();
    let connection = agent_connections::create(
        &app.pool,
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
    (key.value, connection.id)
}

fn connected(app: &TestApp, owner: Uuid) {
    app.state.whatsapp.pretend(
        owner,
        Status::Connected {
            phone_number: "8801700000000".into(),
        },
    );
}

async fn save_contact(app: &TestApp, owner: Uuid, number: &str, name: &str) {
    let account = whatsapp_accounts::find_or_create(&app.pool, owner)
        .await
        .unwrap();
    let jid = format!("{number}@s.whatsapp.net");
    let names = Names {
        full_name: Some(name.to_owned()),
        first_name: None,
        username: None,
    };
    let sealed = seal_names(&app.state.cipher, account.id, &jid, &names).unwrap();
    whatsapp_contacts::save(&app.pool, account.id, &jid, Some(number), None, &sealed)
        .await
        .unwrap();
}

/// Everything off, and no chat to send to
fn nothing() -> Grant {
    let mut grant = Preset::ViewOnly.grant();
    grant.contacts = Contacts::None;
    grant.schedules = Schedules::None;
    grant.templates = Templates::None;
    grant.chats.scope = Scope::Selected;
    grant.chats.strangers = Strangers::Never;
    grant
}

/// Asks before sending, except never to [`BLOCKED`]
fn asks_but_blocks() -> Grant {
    let mut grant = Preset::AskBeforeSending.grant();
    grant.chats.rules.push(ChatRule {
        chat: Chat {
            recipient_type: RecipientType::Contact,
            id: BLOCKED.to_owned(),
            name: None,
        },
        send: SendLevel::No,
        daily_limit: None,
        note: None,
    });
    grant
}

fn post(key: Option<&str>) -> Builder {
    post_to(HOST, key)
}

fn post_to(host: &str, key: Option<&str>) -> Builder {
    let mut request = Request::post("/mcp")
        .header(header::HOST, host)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream");
    if let Some(key) = key {
        request = request.header(header::AUTHORIZATION, format!("Bearer {key}"));
    }
    request
}

/// A request as a client of the 2025-11-25 era sends it, after `initialize`
async fn legacy(app: &TestApp, key: Option<&str>, method: &str, params: Value) -> TestResponse {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let request = post(key)
        .header("mcp-protocol-version", LEGACY)
        .body(Body::from(body.to_string()))
        .unwrap();
    app.send(request).await
}

/// A single request of the 2026-07-28 era, with the headers and `_meta` it needs
async fn modern(app: &TestApp, key: Option<&str>, method: &str, mut params: Value) -> TestResponse {
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion": MODERN,
        "io.modelcontextprotocol/clientCapabilities": {},
    });
    let mut request = post(key)
        .header("mcp-protocol-version", MODERN)
        .header("mcp-method", method);
    if let Some(name) = params["name"].as_str() {
        request = request.header("mcp-name", name);
    }
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    app.send(request.body(Body::from(body.to_string())).unwrap())
        .await
}

async fn call(app: &TestApp, key: &str, tool: &str, arguments: Value) -> Value {
    let response = legacy(
        app,
        Some(key),
        "tools/call",
        json!({ "name": tool, "arguments": arguments }),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    response.body["result"].clone()
}

/// A tool's structured answer, which must also come as text
async fn ok(app: &TestApp, key: &str, tool: &str, arguments: Value) -> Value {
    let result = call(app, key, tool, arguments).await;
    assert_eq!(result["isError"], false, "{tool}: {result}");
    let text: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, result["structuredContent"]);
    result["structuredContent"].clone()
}

/// A tool error the model reads, and its text
async fn tool_error(app: &TestApp, key: &str, tool: &str, arguments: Value) -> String {
    let result = call(app, key, tool, arguments).await;
    assert_eq!(result["isError"], true, "{tool}: {result}");
    assert!(result.get("structuredContent").is_none());
    result["content"][0]["text"].as_str().unwrap().to_owned()
}

fn names(tools: &Value) -> Vec<String> {
    tools
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect()
}

async fn listed(app: &TestApp, key: &str) -> Vec<String> {
    let legacy = legacy(app, Some(key), "tools/list", json!({})).await;
    let modern = modern(app, Some(key), "tools/list", json!({})).await;
    assert_eq!(
        legacy.body["result"]["tools"],
        modern.body["result"]["tools"]
    );
    names(&legacy.body["result"]["tools"])
}

fn later() -> String {
    (Utc::now() + TimeDelta::days(1)).to_rfc3339()
}

fn assert_no_store(response: &TestResponse) {
    assert_eq!(response.headers[header::CACHE_CONTROL], "no-store");
}

#[sqlx::test]
async fn the_legacy_flow_initializes_without_a_session_and_calls_a_tool(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = user(&app).await;
    let (key, _) = connection(&app, owner, &Preset::AskBeforeSending.grant()).await;

    let initialize = json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": LEGACY,
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "1" },
        },
    });
    let response = app
        .send(
            post(Some(&key))
                .body(Body::from(initialize.to_string()))
                .unwrap(),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_no_store(&response);
    assert!(!response.headers.contains_key("mcp-session-id"));
    assert_eq!(
        response.headers[header::CONTENT_TYPE],
        "application/json",
        "a plain JSON answer, not a stream"
    );
    let result = &response.body["result"];
    assert_eq!(result["protocolVersion"], LEGACY);
    assert_eq!(result["serverInfo"]["name"], "whatomato");
    assert_eq!(result["serverInfo"]["title"], "Whatomato");
    assert_eq!(result["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(result["instructions"], INSTRUCTIONS);
    assert!(INSTRUCTIONS.len() < 2_000);
    assert_eq!(result["capabilities"], json!({ "tools": {} }));

    let initialized = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    let response = app
        .send(
            post(Some(&key))
                .header("mcp-protocol-version", LEGACY)
                .body(Body::from(initialized.to_string()))
                .unwrap(),
        )
        .await;
    assert_eq!(response.status, StatusCode::ACCEPTED);

    let list = legacy(&app, Some(&key), "tools/list", json!({})).await;
    assert_eq!(names(&list.body["result"]["tools"]), ALL_TOOLS);
    assert!(list.body["result"].get("ttlMs").is_none());

    let me = ok(&app, &key, "whoami", json!({})).await;
    assert_eq!(me["connection_name"], "Assistant");
    assert_eq!(me["whatsapp"]["status"], "not_linked");

    for method in ["prompts/list", "resources/list"] {
        let empty = legacy(&app, Some(&key), method, json!({})).await;
        assert_eq!(empty.status, StatusCode::OK);
        assert!(empty.body["error"].is_null(), "{method}: {}", empty.body);
    }
}

#[sqlx::test]
async fn modern_requests_discover_list_and_call_on_their_own(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = user(&app).await;
    let (key, _) = connection(&app, owner, &Preset::AskBeforeSending.grant()).await;

    let discover = modern(&app, Some(&key), "server/discover", json!({})).await;
    assert_eq!(discover.status, StatusCode::OK, "{}", discover.body);
    assert_no_store(&discover);
    let result = &discover.body["result"];
    assert!(
        result["supportedVersions"]
            .as_array()
            .unwrap()
            .contains(&json!(MODERN))
    );
    assert_eq!(result["instructions"], INSTRUCTIONS);
    assert_eq!(
        result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "whatomato"
    );

    let list = modern(&app, Some(&key), "tools/list", json!({})).await;
    assert_eq!(list.body["result"]["ttlMs"], 0);
    assert_eq!(list.body["result"]["cacheScope"], "private");
    assert_eq!(names(&list.body["result"]["tools"]), ALL_TOOLS);

    let response = modern(
        &app,
        Some(&key),
        "tools/call",
        json!({ "name": "whoami", "arguments": {} }),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body["result"]["isError"], false);
    assert_eq!(
        response.body["result"]["structuredContent"]["connection_name"],
        "Assistant"
    );

    let missing_header = app
        .send(
            post(Some(&key))
                .header("mcp-protocol-version", MODERN)
                .body(Body::from(
                    json!({
                        "jsonrpc": "2.0", "id": 1, "method": "tools/list",
                        "params": { "_meta": {
                            "io.modelcontextprotocol/protocolVersion": MODERN,
                            "io.modelcontextprotocol/clientCapabilities": {},
                        } },
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(missing_header.status, StatusCode::BAD_REQUEST);
}

#[sqlx::test]
async fn every_unusable_key_gets_401_with_a_bearer_challenge(pool: PgPool) {
    let app = TestApp::new(pool);
    let (owner, session) = app.signed_in("ada@example.com").await;
    let (paused, paused_id) = connection(&app, owner, &Preset::AskBeforeSending.grant()).await;
    let (expired, expired_id) = connection(&app, owner, &Preset::AskBeforeSending.grant()).await;
    sqlx::query("UPDATE agent_connections SET paused_at = now() WHERE id = $1")
        .bind(paused_id)
        .execute(&app.pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE agent_connections SET expires_at = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(expired_id)
    .execute(&app.pool)
    .await
    .unwrap();
    let unknown = AccessKey::generate().unwrap().value;

    let cases: [(&str, Option<String>, bool); 6] = [
        ("no header", None, false),
        ("malformed", Some("Bearer not-a-key".into()), false),
        ("not bearer", Some(format!("Basic {unknown}")), false),
        ("unknown", Some(format!("Bearer {unknown}")), false),
        ("paused", Some(format!("Bearer {paused}")), true),
        ("expired", Some(format!("Bearer {expired}")), true),
    ];
    let generic = {
        let response = app.send(post(None).body(Body::from("{}")).unwrap()).await;
        response.body["error"].clone()
    };
    for (case, authorization, says_why) in cases {
        for modern_era in [false, true] {
            let mut request = post(None).header("mcp-protocol-version", LEGACY);
            if modern_era {
                request = post(None)
                    .header("mcp-protocol-version", MODERN)
                    .header("mcp-method", "tools/list");
            }
            if let Some(authorization) = &authorization {
                request = request.header(header::AUTHORIZATION, authorization);
            }
            let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} });
            let response = app
                .send(request.body(Body::from(body.to_string())).unwrap())
                .await;
            assert_eq!(response.status, StatusCode::UNAUTHORIZED, "{case}");
            let challenge = response.headers[header::WWW_AUTHENTICATE].to_str().unwrap();
            assert!(challenge.starts_with("Bearer"), "{case}: {challenge}");
            assert_no_store(&response);
            if says_why {
                let message = response.body["error"].as_str().unwrap();
                assert!(message.contains(case), "{case}: {message}");
            } else {
                assert_eq!(response.body["error"], generic, "{case}");
            }
        }
    }

    let cookie_only = app
        .send(
            post(None)
                .header(header::COOKIE, &session)
                .body(Body::from(
                    json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }).to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(cookie_only.status, StatusCode::UNAUTHORIZED);
    assert!(cookie_only.headers.contains_key(header::WWW_AUTHENTICATE));
}

#[sqlx::test]
async fn failed_keys_are_limited_per_address_and_working_keys_are_not(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = user(&app).await;
    let (key, _) = connection(&app, owner, &Preset::AskBeforeSending.grant()).await;
    let unknown = AccessKey::generate().unwrap().value;

    let mut statuses = Vec::new();
    for _ in 0..21 {
        statuses.push(
            legacy(&app, Some(&unknown), "tools/list", json!({}))
                .await
                .status,
        );
    }
    assert!(
        statuses[..20]
            .iter()
            .all(|status| *status == StatusCode::UNAUTHORIZED)
    );
    assert_eq!(statuses[20], StatusCode::TOO_MANY_REQUESTS);

    let working = legacy(&app, Some(&key), "tools/list", json!({})).await;
    assert_eq!(working.status, StatusCode::OK);
}

#[sqlx::test]
async fn the_tool_list_holds_what_the_grant_allows(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = user(&app).await;

    for preset in [
        Preset::AskBeforeSending,
        Preset::SendWithoutAsking,
        Preset::FullAccess,
    ] {
        let (key, _) = connection(&app, owner, &preset.grant()).await;
        assert_eq!(listed(&app, &key).await, ALL_TOOLS, "{preset:?}");
    }
    let (view_only, _) = connection(&app, owner, &Preset::ViewOnly.grant()).await;
    assert_eq!(
        listed(&app, &view_only).await,
        [
            "find_chats",
            "get_approval",
            "list_approvals",
            "list_deliveries",
            "list_templates",
            "whoami",
        ]
    );
    let (off, _) = connection(&app, owner, &nothing()).await;
    assert_eq!(listed(&app, &off).await, ALWAYS);

    // Under selected chats, one chat that may be asked about is enough to send
    let mut one_chat = nothing();
    one_chat.schedules = Schedules::Own;
    one_chat.chats.rules = asks_but_blocks().chats.rules;
    one_chat.chats.rules[0].send = SendLevel::Ask;
    let (one_chat, _) = connection(&app, owner, &one_chat).await;
    let names = listed(&app, &one_chat).await;
    assert!(names.contains(&"send_message".to_owned()));
    assert!(names.contains(&"schedule_message".to_owned()));

    let list = legacy(&app, Some(&off), "tools/list", json!({})).await;
    let full = legacy(
        &app,
        Some(&connection(&app, owner, &Preset::FullAccess.grant()).await.0),
        "tools/list",
        json!({}),
    )
    .await;
    for tool in full.body["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .chain(list.body["result"]["tools"].as_array().unwrap())
    {
        let name = &tool["name"];
        assert!(
            tool["title"]
                .as_str()
                .is_some_and(|title| !title.is_empty()),
            "{name}"
        );
        assert!(tool["description"].as_str().unwrap().len() > 80, "{name}");
        assert_eq!(tool["inputSchema"]["type"], "object", "{name}");
        assert_eq!(tool["outputSchema"]["type"], "object", "{name}");
        for hint in [
            "readOnlyHint",
            "destructiveHint",
            "idempotentHint",
            "openWorldHint",
        ] {
            assert!(tool["annotations"][hint].is_boolean(), "{name} {hint}");
        }
    }
    let annotations = |name: &str| {
        full.body["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap()["annotations"]
            .clone()
    };
    assert_eq!(annotations("send_message")["openWorldHint"], true);
    assert_eq!(annotations("send_message")["readOnlyHint"], false);
    assert_eq!(
        annotations("delete_scheduled_message")["destructiveHint"],
        true
    );
    assert_eq!(
        annotations("update_scheduled_message")["destructiveHint"],
        true
    );
    assert_eq!(annotations("list_deliveries")["readOnlyHint"], true);
}

#[sqlx::test]
async fn each_tool_answers_on_its_allowed_path(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = user(&app).await;
    connected(&app, owner);
    save_contact(&app, owner, ADA, "Ada Lovelace").await;
    message_templates::create(&app.pool, owner, "Greeting", "Hello {{first_name}}")
        .await
        .unwrap();
    let (key, _) = connection(&app, owner, &Preset::FullAccess.grant()).await;

    let me = ok(&app, &key, "whoami", json!({})).await;
    assert_eq!(me["whatsapp"]["can_send"], true);

    let found = ok(&app, &key, "find_chats", json!({ "query": "ada" })).await;
    assert_eq!(found["chats"][0]["recipient"], ADA);
    assert_eq!(found["chats"][0]["name"], "Ada Lovelace");
    assert_eq!(found["chats"][0]["rule"]["send"], "yes");

    let templates = ok(&app, &key, "list_templates", json!({})).await;
    assert_eq!(templates["templates"][0]["name"], "Greeting");

    let sent = ok(
        &app,
        &key,
        "send_message",
        json!({ "recipient_type": "contact", "recipient": ADA, "body": "Hello" }),
    )
    .await;
    assert_eq!(sent["outcome"], "queued");
    let deliveries = ok(&app, &key, "list_deliveries", json!({ "limit": 5 })).await;
    assert_eq!(deliveries["deliveries"][0]["id"], sent["delivery_id"]);

    let scheduled = ok(
        &app,
        &key,
        "schedule_message",
        json!({
            "recipient_type": "contact",
            "recipient": ADA,
            "body": "Good morning",
            "schedule_kind": "once",
            "run_at": later(),
        }),
    )
    .await;
    assert_eq!(scheduled["outcome"], "created");
    let id = scheduled["schedule"]["id"].as_str().unwrap().to_owned();
    let listed = ok(&app, &key, "list_scheduled_messages", json!({})).await;
    assert_eq!(listed["schedules"][0]["id"], id.as_str());
    let updated = ok(
        &app,
        &key,
        "update_scheduled_message",
        json!({ "id": id, "body": "Good evening" }),
    )
    .await;
    assert_eq!(updated["schedule"]["body"], "Good evening");
    let deleted = ok(&app, &key, "delete_scheduled_message", json!({ "id": id })).await;
    assert_eq!(deleted["id"], id.as_str());

    let approvals = ok(&app, &key, "list_approvals", json!({})).await;
    assert_eq!(approvals["approvals"], json!([]));
}

#[sqlx::test]
async fn a_send_that_must_ask_waits_and_a_blocked_one_is_refused(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = user(&app).await;
    connected(&app, owner);
    let (key, _) = connection(&app, owner, &asks_but_blocks()).await;
    let count = |sql: &'static str| {
        let pool = app.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(sql)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    let (approvals, deliveries) = (
        "SELECT count(*) FROM agent_approvals",
        "SELECT count(*) FROM deliveries",
    );

    let waiting = ok(
        &app,
        &key,
        "send_message",
        json!({ "recipient_type": "contact", "recipient": ADA, "body": "Hello" }),
    )
    .await;
    assert_eq!(waiting["outcome"], "waiting_for_approval");
    assert!(
        waiting["approval_url"]
            .as_str()
            .unwrap()
            .starts_with("http://localhost:5173/agents?approval=")
    );
    assert_eq!(count(approvals).await, 1);
    assert_eq!(count(deliveries).await, 0);

    let approval = ok(
        &app,
        &key,
        "get_approval",
        json!({ "id": waiting["approval_id"] }),
    )
    .await;
    assert_eq!(approval["status"], "waiting");
    let listed = ok(&app, &key, "list_approvals", json!({ "limit": 1 })).await;
    assert_eq!(listed["approvals"][0]["id"], waiting["approval_id"]);

    let refused = tool_error(
        &app,
        &key,
        "send_message",
        json!({ "recipient_type": "contact", "recipient": BLOCKED, "body": "Hello" }),
    )
    .await;
    assert!(
        refused.contains("isn't allowed to send messages to this chat"),
        "{refused}"
    );
    assert!(
        refused.contains("http://localhost:5173/agents/"),
        "{refused}"
    );
    assert_eq!(count(approvals).await, 1);
    assert_eq!(count(deliveries).await, 0);
}

#[sqlx::test]
async fn a_hidden_tool_called_anyway_is_a_readable_refusal(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = user(&app).await;
    let (key, _) = connection(&app, owner, &nothing()).await;
    let some_id = Uuid::new_v4();

    let calls = [
        ("find_chats", json!({ "query": "ada" })),
        ("list_templates", json!({})),
        (
            "send_message",
            json!({ "recipient_type": "contact", "recipient": ADA, "body": "Hello" }),
        ),
        (
            "schedule_message",
            json!({
                "recipient_type": "contact", "recipient": ADA, "body": "Hello",
                "schedule_kind": "once", "run_at": later(),
            }),
        ),
        ("list_scheduled_messages", json!({})),
        (
            "update_scheduled_message",
            json!({ "id": some_id, "enabled": false }),
        ),
        ("delete_scheduled_message", json!({ "id": some_id })),
    ];
    for (tool, arguments) in calls {
        let text = tool_error(&app, &key, tool, arguments).await;
        assert!(!text.is_empty(), "{tool}");
    }
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_activity WHERE user_id = $1")
        .bind(owner)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(rows, 7, "each refusal is in the activity log");
}

#[sqlx::test]
async fn an_unknown_tool_is_a_protocol_error_and_bad_input_a_tool_error(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = user(&app).await;
    let (key, _) = connection(&app, owner, &Preset::AskBeforeSending.grant()).await;

    let unknown = legacy(
        &app,
        Some(&key),
        "tools/call",
        json!({ "name": "read_messages", "arguments": {} }),
    )
    .await;
    assert_eq!(unknown.body["error"]["code"], -32602);

    let missing = tool_error(
        &app,
        &key,
        "send_message",
        json!({ "recipient": ADA, "body": "Hello" }),
    )
    .await;
    assert!(missing.contains("recipient_type"), "{missing}");

    let invalid = tool_error(
        &app,
        &key,
        "send_message",
        json!({ "recipient_type": "contact", "recipient": "12", "body": " " }),
    )
    .await;
    assert!(
        invalid.starts_with("This request has problems:"),
        "{invalid}"
    );
}

#[sqlx::test]
async fn other_sites_are_refused_and_agents_are_not(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = user(&app).await;
    let (key, _) = connection(&app, owner, &Preset::AskBeforeSending.grant()).await;
    let list = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }).to_string();
    let send = |request: Builder| {
        let list = list.clone();
        let app = app.clone();
        async move {
            app.send(
                request
                    .header("mcp-protocol-version", LEGACY)
                    .body(Body::from(list))
                    .unwrap(),
            )
            .await
        }
    };

    for key in [Some(key.as_str()), None] {
        let foreign = send(post(key).header(header::ORIGIN, "https://evil.example")).await;
        assert_eq!(foreign.status, StatusCode::FORBIDDEN);
        assert_no_store(&foreign);
        let null = send(post(key).header(header::ORIGIN, "null")).await;
        assert_eq!(null.status, StatusCode::FORBIDDEN);
        let other_port = send(post(key).header(header::ORIGIN, "http://localhost:3000")).await;
        assert_eq!(other_port.status, StatusCode::FORBIDDEN);
    }
    let foreign_host = send(post_to("evil.example", Some(&key))).await;
    assert_eq!(foreign_host.status, StatusCode::FORBIDDEN);

    let agent = send(post(Some(&key))).await;
    assert_eq!(agent.status, StatusCode::OK);
    assert_no_store(&agent);
    let own_page = send(post(Some(&key)).header(header::ORIGIN, "http://localhost:5173")).await;
    assert_eq!(own_page.status, StatusCode::OK);
    // A debug build also answers on the API's own port, as a client may skip the Vite proxy
    let direct = send(post_to("127.0.0.1:8000", Some(&key))).await;
    assert_eq!(direct.status, StatusCode::OK);
}

#[sqlx::test]
async fn each_connection_has_its_own_limit_of_120_requests_a_minute(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = user(&app).await;
    let (flood, _) = connection(&app, owner, &Preset::AskBeforeSending.grant()).await;
    let (other, _) = connection(&app, owner, &Preset::AskBeforeSending.grant()).await;

    for _ in 0..120 {
        let response = legacy(&app, Some(&flood), "tools/list", json!({})).await;
        assert_eq!(response.status, StatusCode::OK);
    }
    let limited = legacy(&app, Some(&flood), "tools/list", json!({})).await;
    assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(limited.headers.contains_key(header::RETRY_AFTER));
    assert_no_store(&limited);

    let unaffected = legacy(&app, Some(&other), "tools/list", json!({})).await;
    assert_eq!(unaffected.status, StatusCode::OK);
}

#[sqlx::test]
async fn with_agent_access_off_mcp_is_not_found(pool: PgPool) {
    let app = TestApp::without_agent_access(pool);
    let owner = user(&app).await;
    let (key, _) = connection(&app, owner, &Preset::AskBeforeSending.grant()).await;

    let response = legacy(&app, Some(&key), "tools/list", json!({})).await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    let modern = modern(&app, Some(&key), "tools/list", json!({})).await;
    assert_eq!(modern.status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn a_key_never_reaches_another_accounts_data(pool: PgPool) {
    let app = TestApp::new(pool);
    let (ada, grace) = (user(&app).await, user(&app).await);
    connected(&app, ada);
    connected(&app, grace);
    save_contact(&app, grace, BLOCKED, "Grace Hopper").await;
    message_templates::create(&app.pool, grace, "Grace's", "Hi")
        .await
        .unwrap();
    deliveries::create(
        &app.pool,
        NewDelivery {
            user_id: grace,
            scheduled_message_id: None,
            connection_id: None,
            recipient_type: RecipientType::Contact,
            recipient: BLOCKED,
            recipient_name: None,
            body: "Grace's message",
        },
    )
    .await
    .unwrap();
    let (graces_key, _) = connection(&app, grace, &Preset::FullAccess.grant()).await;
    let schedule = ok(
        &app,
        &graces_key,
        "schedule_message",
        json!({
            "recipient_type": "contact", "recipient": BLOCKED, "body": "Grace's schedule",
            "schedule_kind": "once", "run_at": later(),
        }),
    )
    .await["schedule"]["id"]
        .clone();
    let (graces_asking, _) = connection(&app, grace, &Preset::AskBeforeSending.grant()).await;
    let approval = ok(
        &app,
        &graces_asking,
        "send_message",
        json!({ "recipient_type": "contact", "recipient": BLOCKED, "body": "Hi" }),
    )
    .await["approval_id"]
        .clone();

    let (key, _) = connection(&app, ada, &Preset::FullAccess.grant()).await;
    let found = ok(&app, &key, "find_chats", json!({ "query": "grace" })).await;
    assert_eq!(found["chats"], json!([]));
    let templates = ok(&app, &key, "list_templates", json!({})).await;
    assert_eq!(templates["templates"], json!([]));
    let deliveries = ok(&app, &key, "list_deliveries", json!({})).await;
    assert_eq!(deliveries["deliveries"], json!([]));
    let schedules = ok(&app, &key, "list_scheduled_messages", json!({})).await;
    assert_eq!(schedules["schedules"], json!([]));
    let approvals = ok(&app, &key, "list_approvals", json!({})).await;
    assert_eq!(approvals["approvals"], json!([]));

    for (tool, arguments) in [
        ("get_approval", json!({ "id": approval })),
        (
            "update_scheduled_message",
            json!({ "id": schedule, "body": "Mine now" }),
        ),
        ("delete_scheduled_message", json!({ "id": schedule })),
    ] {
        let text = tool_error(&app, &key, tool, arguments).await;
        assert!(text.contains("no "), "{tool}: {text}");
    }
    let still: String =
        sqlx::query_scalar("SELECT body FROM scheduled_messages WHERE user_id = $1")
            .bind(grace)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(still, "Grace's schedule");
}

#[sqlx::test]
async fn a_request_notes_when_the_connection_was_last_used(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = user(&app).await;
    let (key, id) = connection(&app, owner, &Preset::AskBeforeSending.grant()).await;
    let last_used = || async {
        agent_connections::find(&app.pool, id, owner)
            .await
            .unwrap()
            .unwrap()
            .last_used_at
    };
    assert!(last_used().await.is_none());

    let response = legacy(&app, Some(&key), "tools/list", json!({})).await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(last_used().await.is_some());
}
