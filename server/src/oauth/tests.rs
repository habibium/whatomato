// Through the real router: the consent page's API with a session, then the token endpoint and
// `/mcp` as a client would call them. The apps come from `Clients::listed`, not the network.

use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use base64::prelude::{BASE64_URL_SAFE_NO_PAD, Engine as _};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use url::Url;
use uuid::Uuid;

use crate::{
    agents::{
        Preset,
        key::{AccessKey, Issued},
    },
    db::agent_connections::{self, Credential, MAX_PER_USER, NewConnection},
    routes::auth::tests::{PASSWORD, TestApp, TestResponse},
};

const APP: &str = "http://localhost:5173";
const CLIENT: &str = "https://app.example/oauth/client.json";
const REDIRECT: &str = "https://app.example/callback";
const LOCAL_CLIENT: &str = "https://cli.example/client.json";
const VERIFIER: &str = "a-code-verifier-that-is-at-least-43-characters-long";
const EMAIL: &str = "ada@example.com";

fn challenge() -> String {
    BASE64_URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER))
}

fn request() -> Value {
    json!({
        "response_type": "code",
        "client_id": CLIENT,
        "redirect_uri": REDIRECT,
        "code_challenge": challenge(),
        "code_challenge_method": "S256",
        "state": "xyz",
        "resource": format!("{APP}/mcp"),
        "scope": "whatomato",
    })
}

fn with(mut request: Value, field: &str, value: Value) -> Value {
    request[field] = value;
    request
}

async fn check(app: &TestApp, session: &str, request: Value) -> Value {
    let response = app
        .call(
            Method::POST,
            "/agents/sign-in/check",
            Some(request),
            Some(session),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    response.body
}

async fn approve(app: &TestApp, session: &str, body: Value) -> TestResponse {
    app.call(
        Method::POST,
        "/agents/sign-in/approve",
        Some(body),
        Some(session),
    )
    .await
}

fn new_connection(request: &Value) -> Value {
    json!({
        "request": request,
        "password": PASSWORD,
        "name": "Claude",
        "grant": Preset::AskBeforeSending.grant(),
    })
}

/// The query of the address the browser is sent back to
fn query(url: &str) -> Vec<(String, String)> {
    Url::parse(url)
        .unwrap()
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect()
}

fn param(url: &str, name: &str) -> Option<String> {
    query(url)
        .into_iter()
        .find_map(|(key, value)| (key == name).then_some(value))
}

/// Says yes to a new connection and gives the code the app is sent
async fn approved_code(app: &TestApp, session: &str) -> String {
    let approved = approve(app, session, new_connection(&request())).await;
    assert_eq!(approved.status, StatusCode::OK, "{}", approved.body);
    param(approved.body["url"].as_str().unwrap(), "code").unwrap()
}

async fn form(app: &TestApp, path: &str, fields: &[(&str, &str)]) -> TestResponse {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields)
        .finish();
    let request = Request::post(path)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .unwrap();
    app.send(request).await
}

async fn exchange(app: &TestApp, code: &str) -> TestResponse {
    form(
        app,
        "/oauth/token",
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", REDIRECT),
            ("client_id", CLIENT),
            ("code_verifier", VERIFIER),
            ("resource", &format!("{APP}/mcp")),
        ],
    )
    .await
}

async fn refresh(app: &TestApp, refresh_token: &str) -> TestResponse {
    form(
        app,
        "/oauth/token",
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CLIENT),
        ],
    )
    .await
}

fn tokens(response: &TestResponse) -> (String, String) {
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    (
        response.body["access_token"].as_str().unwrap().to_owned(),
        response.body["refresh_token"].as_str().unwrap().to_owned(),
    )
}

/// A new connection, signed in: its access and refresh tokens
async fn signed_in(app: &TestApp, session: &str) -> (String, String) {
    let code = approved_code(app, session).await;
    tokens(&exchange(app, &code).await)
}

/// `tools/list` on `/mcp` with `bearer`
async fn tools(app: &TestApp, bearer: Option<&str>) -> TestResponse {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} });
    let mut request = Request::post("/mcp")
        .header(header::HOST, "localhost:5173")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header("mcp-protocol-version", "2025-11-25");
    if let Some(bearer) = bearer {
        request = request.header(header::AUTHORIZATION, format!("Bearer {bearer}"));
    }
    app.send(request.body(Body::from(body.to_string())).unwrap())
        .await
}

async fn get(app: &TestApp, path: &str) -> TestResponse {
    app.send(Request::get(path).body(Body::empty()).unwrap())
        .await
}

fn assert_grant_refused(response: &TestResponse) {
    assert_eq!(
        response.status,
        StatusCode::BAD_REQUEST,
        "{}",
        response.body
    );
    assert_eq!(response.body["error"], "invalid_grant");
    assert!(response.body["error_description"].is_string());
    assert_eq!(response.headers[header::CACHE_CONTROL], "no-store");
}

async fn connections(app: &TestApp, session: &str) -> Vec<Value> {
    let overview = app.call(Method::GET, "/agents", None, Some(session)).await;
    overview.body["connections"].as_array().unwrap().clone()
}

#[sqlx::test]
async fn discovery_says_where_and_how_to_sign_in(pool: PgPool) {
    let app = TestApp::new(pool);

    for path in [
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-protected-resource/mcp",
    ] {
        let resource = get(&app, path).await;
        assert_eq!(resource.status, StatusCode::OK, "{path}");
        assert_eq!(resource.body["resource"], format!("{APP}/mcp"));
        assert_eq!(resource.body["authorization_servers"], json!([APP]));
        assert_eq!(resource.body["scopes_supported"], json!(["whatomato"]));
        assert_eq!(resource.headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    }

    let server = get(&app, "/.well-known/oauth-authorization-server").await;
    assert_eq!(server.status, StatusCode::OK);
    let expected = json!({
        "issuer": APP,
        "authorization_endpoint": format!("{APP}/oauth/authorize"),
        "token_endpoint": format!("{APP}/oauth/token"),
        "revocation_endpoint": format!("{APP}/oauth/revoke"),
        "code_challenge_methods_supported": ["S256"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "response_types_supported": ["code"],
        "token_endpoint_auth_methods_supported": ["none"],
        "client_id_metadata_document_supported": true,
        "authorization_response_iss_parameter_supported": true,
    });
    for (field, value) in expected.as_object().unwrap() {
        assert_eq!(&server.body[field], value, "{field}");
    }
    // Registering a client by request is deprecated and not offered
    assert!(server.body.get("registration_endpoint").is_none());
}

#[sqlx::test]
async fn the_browser_reaches_the_consent_page_with_the_request_untouched(pool: PgPool) {
    let app = TestApp::new(pool);
    // A router that reads search values as JSON would hand this `state` back as 1000
    let sent = "response_type=code&state=1e3&client_id=https%3A%2F%2Fapp.example%2Fc.json&x=%2B+";

    let response = get(&app, &format!("/oauth/authorize?{sent}")).await;
    assert_eq!(response.status, StatusCode::SEE_OTHER);
    assert_eq!(response.headers[header::CACHE_CONTROL], "no-store");
    let location = response.headers[header::LOCATION].to_str().unwrap();
    let packed = location
        .strip_prefix("/agents/sign-in?request=q.")
        .unwrap_or_else(|| panic!("{location}"));
    assert_eq!(
        BASE64_URL_SAFE_NO_PAD.decode(packed).unwrap(),
        sent.as_bytes()
    );

    let bare = get(&app, "/oauth/authorize").await;
    assert_eq!(bare.headers[header::LOCATION], "/agents/sign-in?request=q.");
    let endless = format!("/oauth/authorize?state={}", "s".repeat(9000));
    assert_eq!(get(&app, &endless).await.status, StatusCode::URI_TOO_LONG);
}

#[sqlx::test]
async fn with_agent_access_off_sign_in_is_not_found(pool: PgPool) {
    let app = TestApp::without_agent_access(pool);
    let (_, session) = app.signed_in(EMAIL).await;

    for path in [
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-protected-resource/mcp",
        "/.well-known/oauth-authorization-server",
        "/oauth/authorize?response_type=code",
    ] {
        assert_eq!(
            get(&app, path).await.status,
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
    for path in ["/oauth/token", "/oauth/revoke"] {
        let response = form(&app, path, &[("grant_type", "refresh_token")]).await;
        assert_eq!(response.status, StatusCode::NOT_FOUND, "{path}");
    }
    let checked = app
        .call(
            Method::POST,
            "/agents/sign-in/check",
            Some(request()),
            Some(&session),
        )
        .await;
    assert_eq!(checked.status, StatusCode::NOT_FOUND);
    let approved = approve(&app, &session, new_connection(&request())).await;
    assert_eq!(approved.status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn a_refused_call_says_where_to_sign_in(pool: PgPool) {
    let app = TestApp::new(pool);
    let metadata = format!(r#"resource_metadata="{APP}/.well-known/oauth-protected-resource""#);

    let no_key = tools(&app, None).await;
    assert_eq!(no_key.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        no_key.headers[header::WWW_AUTHENTICATE],
        format!(r#"Bearer {metadata}, scope="whatomato""#)
    );

    let (unknown, _) = Issued::AccessToken.generate().unwrap();
    for wrong in [unknown, AccessKey::generate().unwrap().value] {
        let refused = tools(&app, Some(&wrong)).await;
        assert_eq!(refused.status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            refused.headers[header::WWW_AUTHENTICATE],
            format!(r#"Bearer error="invalid_token", {metadata}, scope="whatomato""#)
        );
    }
}

#[sqlx::test]
async fn the_consent_page_is_told_who_asks(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in(EMAIL).await;

    let asked = check(&app, &session, request()).await;
    assert_eq!(asked["status"], "ask");
    assert_eq!(asked["client_name"], "Example App");
    assert_eq!(asked["client_host"], "app.example");
    assert_eq!(asked["redirect_host"], "app.example");
    assert_eq!(asked["is_local_only"], false);
    assert_eq!(asked["can_add"], true);
    assert_eq!(asked["connections"], json!([]));
    let deny = asked["deny_url"].as_str().unwrap();
    assert!(deny.starts_with(&format!("{REDIRECT}?")), "{deny}");
    assert_eq!(param(deny, "error").unwrap(), "access_denied");
    assert_eq!(param(deny, "state").unwrap(), "xyz");
    assert_eq!(param(deny, "iss").unwrap(), APP);
    assert_eq!(param(deny, "code"), None);

    // An app on the owner's own machine listens on whatever port it found free
    let local = with(request(), "client_id", json!(LOCAL_CLIENT));
    let local = with(
        local,
        "redirect_uri",
        json!("http://localhost:4321/callback"),
    );
    let asked = check(&app, &session, local).await;
    assert_eq!(asked["status"], "ask", "{asked}");
    assert_eq!(asked["client_name"], "Example CLI");
    assert_eq!(asked["redirect_host"], "localhost:4321");
    assert_eq!(asked["is_local_only"], true);

    let signed_out = app
        .call(Method::POST, "/agents/sign-in/check", Some(request()), None)
        .await;
    assert_eq!(signed_out.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn a_request_that_cant_be_trusted_never_leaves_the_page(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in(EMAIL).await;

    let cases = [
        with(
            request(),
            "client_id",
            json!("https://unknown.example/client.json"),
        ),
        with(
            request(),
            "client_id",
            json!("http://app.example/oauth/client.json"),
        ),
        with(request(), "client_id", json!("not a url")),
        with(request(), "client_id", Value::Null),
        with(
            request(),
            "redirect_uri",
            json!("https://evil.example/callback"),
        ),
        with(request(), "redirect_uri", json!(format!("{REDIRECT}/"))),
        with(request(), "redirect_uri", Value::Null),
        with(request(), "state", json!("s".repeat(2049))),
        // Flawed twice over: where to send the browser back is never taken on trust
        with(
            with(request(), "response_type", json!("token")),
            "redirect_uri",
            json!("https://evil.example/callback"),
        ),
    ];
    for case in cases {
        let refused = check(&app, &session, case.clone()).await;
        assert_eq!(refused["status"], "refused", "{case}");
        assert!(
            refused["message"].as_str().unwrap().ends_with('.'),
            "{case}"
        );
        assert!(refused.get("url").is_none(), "{case}");

        let approved = approve(&app, &session, new_connection(&case)).await;
        assert_eq!(approved.status, StatusCode::UNPROCESSABLE_ENTITY, "{case}");
        assert!(approved.body["details"]["request"].is_array(), "{case}");
    }
}

#[sqlx::test]
async fn a_flawed_request_goes_back_to_the_app_with_the_error(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in(EMAIL).await;

    let cases = [
        ("response_type", json!("token"), "unsupported_response_type"),
        ("response_type", Value::Null, "unsupported_response_type"),
        ("code_challenge_method", json!("plain"), "invalid_request"),
        ("code_challenge_method", Value::Null, "invalid_request"),
        ("code_challenge", json!("too-short"), "invalid_request"),
        ("code_challenge", Value::Null, "invalid_request"),
        (
            "resource",
            json!("https://other.example/mcp"),
            "invalid_target",
        ),
        ("resource", json!(APP), "invalid_target"),
    ];
    for (field, value, error) in cases {
        let flawed = with(request(), field, value.clone());
        let sent_back = check(&app, &session, flawed.clone()).await;
        assert_eq!(sent_back["status"], "redirect", "{field}: {value}");
        let url = sent_back["url"].as_str().unwrap();
        assert!(url.starts_with(&format!("{REDIRECT}?")), "{url}");
        assert_eq!(param(url, "error").unwrap(), error, "{field}: {value}");
        assert_eq!(param(url, "state").unwrap(), "xyz");
        assert_eq!(param(url, "iss").unwrap(), APP);

        let approved = approve(&app, &session, new_connection(&flawed)).await;
        assert_eq!(approved.status, StatusCode::UNPROCESSABLE_ENTITY, "{field}");
    }

    for sound in [
        with(request(), "resource", json!(format!("{APP}/mcp/"))),
        with(request(), "resource", Value::Null),
        with(request(), "scope", Value::Null),
        with(request(), "state", Value::Null),
    ] {
        assert_eq!(
            check(&app, &session, sound.clone()).await["status"],
            "ask",
            "{sound}"
        );
    }
}

#[sqlx::test]
async fn signing_in_makes_a_connection_when_the_code_is_exchanged(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in(EMAIL).await;

    let approved = approve(&app, &session, new_connection(&request())).await;
    assert_eq!(approved.status, StatusCode::OK, "{}", approved.body);
    let url = approved.body["url"].as_str().unwrap();
    assert!(url.starts_with(&format!("{REDIRECT}?")), "{url}");
    assert_eq!(param(url, "state").unwrap(), "xyz");
    assert_eq!(param(url, "iss").unwrap(), APP);
    assert_eq!(param(url, "error"), None);
    let code = param(url, "code").unwrap();
    assert!(Issued::Code.is(&code));
    // A yes alone makes nothing: the app might never come back for it
    assert_eq!(connections(&app, &session).await.len(), 0);
    assert_eq!(app.emails_to(EMAIL).len(), 0);

    let issued = exchange(&app, &code).await;
    assert_eq!(issued.status, StatusCode::OK, "{}", issued.body);
    assert_eq!(issued.headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(issued.body["token_type"], "Bearer");
    assert_eq!(issued.body["expires_in"], 3600);
    assert_eq!(issued.body["scope"], "whatomato");
    let (access, refresh_token) = tokens(&issued);
    assert!(Issued::AccessToken.is(&access));
    assert!(Issued::RefreshToken.is(&refresh_token));

    let made = connections(&app, &session).await;
    assert_eq!(made.len(), 1);
    assert_eq!(made[0]["name"], "Claude");
    assert_eq!(made[0]["key_hint"], Value::Null);
    assert_eq!(
        made[0]["signed_in"],
        json!({ "client_name": "Example App", "client_host": "app.example" })
    );
    assert_eq!(made[0]["preset"], "ask_before_sending");
    let emails = app.emails_to(EMAIL);
    assert_eq!(emails.len(), 1);
    assert_eq!(emails[0].0, "An AI agent was connected to your account");

    let listed = tools(&app, Some(&access)).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.body);
    assert!(listed.body["result"]["tools"].is_array());
    // Only the access token opens the endpoint
    for other in [&refresh_token, &code] {
        assert_eq!(
            tools(&app, Some(other)).await.status,
            StatusCode::UNAUTHORIZED
        );
    }
}

#[sqlx::test]
async fn approving_needs_the_password_and_one_sound_choice(pool: PgPool) {
    let app = TestApp::new(pool);
    let (ada, session) = app.signed_in(EMAIL).await;
    let (_, graces_session) = app.signed_in("grace@example.com").await;
    signed_in(&app, &graces_session).await;
    let graces = connections(&app, &graces_session).await[0]["id"].clone();
    let key = AccessKey::generate().unwrap();
    let with_key = agent_connections::create(
        &app.pool,
        NewConnection {
            user_id: ada,
            name: "With a key",
            credential: Credential::Key {
                hash: &key.hash,
                hint: &key.hint,
            },
            grant: &Preset::ViewOnly.grant(),
            expires_at: None,
        },
    )
    .await
    .unwrap()
    .unwrap();

    let wrong_password = with(new_connection(&request()), "password", json!("not it"));
    let refused = approve(&app, &session, wrong_password).await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.body);
    assert!(refused.body["details"]["password"].is_array());

    let both = with(
        new_connection(&request()),
        "connection_id",
        json!(with_key.id),
    );
    let neither = json!({ "request": request(), "password": PASSWORD });
    let nameless = with(new_connection(&request()), "name", json!("  "));
    for unsound in [both, neither, nameless] {
        let refused = approve(&app, &session, unsound.clone()).await;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{unsound}");
    }

    let mut bad_grant = new_connection(&request());
    bad_grant["grant"]["chats"]["default"]["daily_limit"] = json!(0);
    let refused = approve(&app, &session, bad_grant).await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        refused.body
    );
    assert!(refused.body["details"]["grant"].is_array());

    // Signing in again is only for a connection this app made, on this account
    for not_this_apps in [json!(with_key.id), graces, json!(Uuid::new_v4())] {
        let again = json!({
            "request": request(),
            "password": PASSWORD,
            "connection_id": not_this_apps,
        });
        let refused = approve(&app, &session, again).await;
        assert_eq!(refused.status, StatusCode::NOT_FOUND, "{not_this_apps}");
    }
    assert_eq!(connections(&app, &session).await.len(), 1);
}

#[sqlx::test]
async fn an_account_at_its_limit_cant_sign_another_app_in(pool: PgPool) {
    let app = TestApp::new(pool);
    let (ada, session) = app.signed_in(EMAIL).await;
    let code = approved_code(&app, &session).await;
    for n in 0..MAX_PER_USER {
        let key = AccessKey::generate().unwrap();
        agent_connections::create(
            &app.pool,
            NewConnection {
                user_id: ada,
                name: &format!("Agent {n}"),
                credential: Credential::Key {
                    hash: &key.hash,
                    hint: &key.hint,
                },
                grant: &Preset::ViewOnly.grant(),
                expires_at: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
    }

    assert_eq!(check(&app, &session, request()).await["can_add"], false);
    let refused = approve(&app, &session, new_connection(&request())).await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
    // A yes given while there was room finds none left
    assert_grant_refused(&exchange(&app, &code).await);
    assert_eq!(connections(&app, &session).await.len(), 20);
}

#[sqlx::test]
async fn a_code_works_once_and_only_for_the_app_that_asked(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in(EMAIL).await;
    let code = approved_code(&app, &session).await;
    let exchange_with = |field: &'static str, value: &'static str| {
        let (app, code) = (&app, code.clone());
        async move {
            let mut fields = vec![
                ("grant_type", "authorization_code"),
                ("code", code.as_str()),
                ("redirect_uri", REDIRECT),
                ("client_id", CLIENT),
                ("code_verifier", VERIFIER),
            ];
            fields.retain(|(name, _)| *name != field);
            if !value.is_empty() {
                fields.push((field, value));
            }
            form(app, "/oauth/token", &fields).await
        }
    };

    for (field, value) in [
        (
            "code_verifier",
            "another-verifier-that-is-also-43-characters-long",
        ),
        ("code_verifier", "short"),
        ("code_verifier", ""),
        ("client_id", LOCAL_CLIENT),
        ("client_id", ""),
        ("redirect_uri", "https://app.example/other"),
        ("redirect_uri", ""),
    ] {
        assert_grant_refused(&exchange_with(field, value).await);
    }
    let wrong_resource = exchange_with("resource", "https://other.example/mcp").await;
    assert_eq!(wrong_resource.body["error"], "invalid_target");
    assert_eq!(connections(&app, &session).await.len(), 0);

    // None of that spent the code
    let (access, refresh_token) = tokens(&exchange(&app, &code).await);
    assert_eq!(tools(&app, Some(&access)).await.status, StatusCode::OK);

    // Shown a second time, it was seen by someone else: what it gave stops working
    assert_grant_refused(&exchange(&app, &code).await);
    assert_eq!(
        tools(&app, Some(&access)).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_grant_refused(&refresh(&app, &refresh_token).await);
    // The connection and its permissions stay, for the owner to sign in to again
    assert_eq!(connections(&app, &session).await.len(), 1);
}

#[sqlx::test]
async fn a_code_runs_out(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in(EMAIL).await;
    let code = approved_code(&app, &session).await;
    let left: i64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM expires_at - now())::bigint FROM agent_sign_ins",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert!((50..=60).contains(&left), "{left}");

    sqlx::query("UPDATE agent_sign_ins SET expires_at = now() - interval '1 second'")
        .execute(&app.pool)
        .await
        .unwrap();
    assert_grant_refused(&exchange(&app, &code).await);
    assert_eq!(connections(&app, &session).await.len(), 0);
}

#[sqlx::test]
async fn refreshing_gives_the_next_pair(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in(EMAIL).await;
    let (access, first) = signed_in(&app, &session).await;

    let refreshed = refresh(&app, &first).await;
    assert_eq!(refreshed.headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(refreshed.body["expires_in"], 3600);
    let (next_access, second) = tokens(&refreshed);
    assert_ne!(next_access, access);
    assert_ne!(second, first);
    assert_eq!(tools(&app, Some(&next_access)).await.status, StatusCode::OK);
    // The earlier access token runs its hour out
    assert_eq!(tools(&app, Some(&access)).await.status, StatusCode::OK);

    // A client that refreshes twice at once, or lost the first answer, isn't taken for a thief
    let (again_access, _) = tokens(&refresh(&app, &first).await);
    assert_eq!(
        tools(&app, Some(&again_access)).await.status,
        StatusCode::OK
    );

    // Past that grace a spent refresh token means it leaked, and everyone is signed out
    sqlx::query("UPDATE agent_tokens SET replaced_at = now() - interval '2 minutes' WHERE replaced_at IS NOT NULL")
        .execute(&app.pool)
        .await
        .unwrap();
    assert_grant_refused(&refresh(&app, &first).await);
    for dead in [&access, &next_access, &again_access] {
        assert_eq!(
            tools(&app, Some(dead)).await.status,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_grant_refused(&refresh(&app, &second).await);
    assert_eq!(connections(&app, &session).await.len(), 1);
}

#[sqlx::test]
async fn a_refresh_token_is_its_apps_alone_and_runs_out(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in(EMAIL).await;
    let (access, refresh_token) = signed_in(&app, &session).await;
    let refresh_as = |fields: Vec<(&'static str, String)>| {
        let app = &app;
        async move {
            let fields: Vec<_> = fields
                .iter()
                .map(|(name, value)| (*name, value.as_str()))
                .collect();
            form(app, "/oauth/token", &fields).await
        }
    };
    let grant = ("grant_type", "refresh_token".to_owned());
    let token = ("refresh_token", refresh_token.clone());

    let other_app = refresh_as(vec![
        grant.clone(),
        token.clone(),
        ("client_id", LOCAL_CLIENT.into()),
    ]);
    assert_grant_refused(&other_app.await);
    assert_grant_refused(&refresh_as(vec![grant.clone(), token.clone()]).await);
    let (unknown, _) = Issued::RefreshToken.generate().unwrap();
    for wrong in [
        unknown,
        access.clone(),
        "nonsense".to_owned(),
        String::new(),
    ] {
        let refused = refresh_as(vec![
            grant.clone(),
            ("refresh_token", wrong),
            ("client_id", CLIENT.into()),
        ]);
        assert_grant_refused(&refused.await);
    }
    let elsewhere = refresh_as(vec![
        grant.clone(),
        token.clone(),
        ("client_id", CLIENT.into()),
        ("resource", "https://other.example/mcp".into()),
    ]);
    assert_eq!(elsewhere.await.body["error"], "invalid_target");
    for grant_type in ["password", "client_credentials", ""] {
        let refused = refresh_as(vec![("grant_type", grant_type.into()), token.clone()]).await;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST);
        assert_eq!(
            refused.body["error"], "unsupported_grant_type",
            "{grant_type}"
        );
    }
    let as_json = app
        .send(
            Request::post("/oauth/token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "grant_type": "refresh_token" }).to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(as_json.status, StatusCode::BAD_REQUEST);
    assert_eq!(as_json.body["error"], "invalid_request");

    // None of that spent it
    sqlx::query("UPDATE agent_tokens SET refresh_expires_at = now() - interval '1 second'")
        .execute(&app.pool)
        .await
        .unwrap();
    assert_grant_refused(&refresh(&app, &refresh_token).await);
    sqlx::query("UPDATE agent_tokens SET refresh_expires_at = now() + interval '1 day'")
        .execute(&app.pool)
        .await
        .unwrap();
    tokens(&refresh(&app, &refresh_token).await);
}

#[sqlx::test]
async fn an_access_token_runs_out_after_an_hour(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in(EMAIL).await;
    let (access, refresh_token) = signed_in(&app, &session).await;
    let left: i64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM access_expires_at - now())::bigint FROM agent_tokens",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert!((3590..=3600).contains(&left), "{left}");

    sqlx::query("UPDATE agent_tokens SET access_expires_at = now() - interval '1 second'")
        .execute(&app.pool)
        .await
        .unwrap();
    let expired = tools(&app, Some(&access)).await;
    assert_eq!(expired.status, StatusCode::UNAUTHORIZED);
    let challenge = expired.headers[header::WWW_AUTHENTICATE].to_str().unwrap();
    assert!(
        challenge.contains(r#"error="invalid_token""#),
        "{challenge}"
    );

    let (next, _) = tokens(&refresh(&app, &refresh_token).await);
    assert_eq!(tools(&app, Some(&next)).await.status, StatusCode::OK);
}

#[sqlx::test]
async fn signing_in_again_keeps_the_connection_and_its_permissions(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in(EMAIL).await;
    let (access, refresh_token) = signed_in(&app, &session).await;

    let asked = check(&app, &session, request()).await;
    let offered = asked["connections"].as_array().unwrap();
    assert_eq!(offered.len(), 1);
    assert_eq!(offered[0]["name"], "Claude");
    let id = offered[0]["id"].clone();
    // Another app's connections aren't offered to this one
    let other = with(request(), "client_id", json!(LOCAL_CLIENT));
    let other = with(other, "redirect_uri", json!("http://localhost/callback"));
    assert_eq!(check(&app, &session, other).await["connections"], json!([]));

    let again = json!({ "request": request(), "password": PASSWORD, "connection_id": id });
    let approved = approve(&app, &session, again).await;
    assert_eq!(approved.status, StatusCode::OK, "{}", approved.body);
    let code = param(approved.body["url"].as_str().unwrap(), "code").unwrap();
    // Until the app comes back with the code, the earlier sign-in stands
    assert_eq!(tools(&app, Some(&access)).await.status, StatusCode::OK);

    let (next, _) = tokens(&exchange(&app, &code).await);
    assert_eq!(tools(&app, Some(&next)).await.status, StatusCode::OK);
    assert_eq!(
        tools(&app, Some(&access)).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_grant_refused(&refresh(&app, &refresh_token).await);
    let all = connections(&app, &session).await;
    assert_eq!(all.len(), 1);
    assert_eq!(all[0]["id"], id);
    assert_eq!(app.emails_to(EMAIL).len(), 1);
}

#[sqlx::test]
async fn revoking_a_token_signs_the_connection_out(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in(EMAIL).await;

    for by_refresh_token in [true, false] {
        let (access, refresh_token) = signed_in(&app, &session).await;
        let token = if by_refresh_token {
            &refresh_token
        } else {
            &access
        };
        let revoked = form(&app, "/oauth/revoke", &[("token", token)]).await;
        assert_eq!(revoked.status, StatusCode::OK);
        assert_eq!(
            tools(&app, Some(&access)).await.status,
            StatusCode::UNAUTHORIZED
        );
        assert_grant_refused(&refresh(&app, &refresh_token).await);
    }
    // Unknown tokens answer the same, as RFC 7009 asks
    let (unknown, _) = Issued::RefreshToken.generate().unwrap();
    for token in [unknown.as_str(), "nonsense", ""] {
        let revoked = form(&app, "/oauth/revoke", &[("token", token)]).await;
        assert_eq!(revoked.status, StatusCode::OK, "{token}");
    }
    assert_eq!(connections(&app, &session).await.len(), 2);
}

#[sqlx::test]
async fn a_paused_connection_is_refused_without_being_told_to_sign_in_again(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in(EMAIL).await;
    let (access, refresh_token) = signed_in(&app, &session).await;
    let id = connections(&app, &session).await[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let paused = app
        .call(
            Method::POST,
            &format!("/agents/{id}/pause"),
            None,
            Some(&session),
        )
        .await;
    assert_eq!(paused.status, StatusCode::OK);
    let refused = tools(&app, Some(&access)).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);
    assert!(refused.body["error"].as_str().unwrap().contains("paused"));
    assert!(!refused.headers.contains_key(header::WWW_AUTHENTICATE));
    // Its tokens keep refreshing, so it works again the moment it's resumed
    let (next, _) = tokens(&refresh(&app, &refresh_token).await);
    assert_eq!(tools(&app, Some(&next)).await.status, StatusCode::FORBIDDEN);

    let resumed = app
        .call(
            Method::POST,
            &format!("/agents/{id}/resume"),
            None,
            Some(&session),
        )
        .await;
    assert_eq!(resumed.status, StatusCode::OK);
    assert_eq!(tools(&app, Some(&next)).await.status, StatusCode::OK);

    // It has no key to replace, and revoking it takes its tokens along
    let rotated = app
        .call(
            Method::POST,
            &format!("/agents/{id}/rotate-key"),
            Some(json!({ "password": PASSWORD })),
            Some(&session),
        )
        .await;
    assert_eq!(rotated.status, StatusCode::CONFLICT, "{}", rotated.body);
    let revoked = app
        .call(
            Method::DELETE,
            &format!("/agents/{id}"),
            None,
            Some(&session),
        )
        .await;
    assert_eq!(revoked.status, StatusCode::OK);
    assert_eq!(
        tools(&app, Some(&next)).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_grant_refused(&refresh(&app, &refresh_token).await);
}

#[sqlx::test]
async fn wrong_codes_are_limited_per_address(pool: PgPool) {
    let app = TestApp::new(pool);
    let (_, session) = app.signed_in(EMAIL).await;
    let code = approved_code(&app, &session).await;

    let mut statuses = Vec::new();
    for _ in 0..21 {
        let (wrong, _) = Issued::Code.generate().unwrap();
        statuses.push(exchange(&app, &wrong).await.status);
    }
    assert!(
        statuses[..20]
            .iter()
            .all(|status| *status == StatusCode::BAD_REQUEST)
    );
    assert_eq!(statuses[20], StatusCode::TOO_MANY_REQUESTS);
    // A real code from the same address still gets through
    tokens(&exchange(&app, &code).await);
}
