//! Signing in: the OAuth 2.1 authorization server an MCP client uses in place of an access key.
//! The consent page is the web app's, at `/agents/sign-in`; its API is under `routes::agents`.

pub mod authorize;
pub mod client;
mod metadata;
mod token;

#[cfg(test)]
mod tests;

use axum::{
    Router,
    routing::{any, get, post},
};
use chrono::TimeDelta;
use url::Url;

use crate::{routes, state::AppState};

/// The one scope. What a connection may do is its grant, chosen on the consent page.
pub const SCOPE: &str = "whatomato";
pub const CODE_TTL: TimeDelta = TimeDelta::seconds(60);
pub const ACCESS_TTL: TimeDelta = TimeDelta::hours(1);
/// Counted from the last refresh, so only a connection left unused this long has to sign in again
pub const REFRESH_TTL: TimeDelta = TimeDelta::days(90);
/// How long a refresh token just exchanged still works, for a client that refreshes twice at once
/// or never got the first answer
const REFRESH_GRACE: TimeDelta = TimeDelta::seconds(60);
/// How long an exchanged refresh token is kept, to catch it being shown again
const REPLACED_KEPT_DAYS: i32 = 7;

pub const PROTECTED_RESOURCE_PATH: &str = "/.well-known/oauth-protected-resource";
const AUTHORIZATION_SERVER_PATH: &str = "/.well-known/oauth-authorization-server";

/// This server's name as an authorization server: `APP_URL` without a trailing slash
#[must_use]
pub fn issuer(app_url: &Url) -> String {
    app_url.as_str().trim_end_matches('/').to_owned()
}

/// What tokens are for: the MCP endpoint
#[must_use]
pub fn resource(app_url: &Url) -> String {
    format!("{}/mcp", issuer(app_url))
}

/// Whether `asked`, a request's `resource`, names the MCP endpoint
fn is_our_resource(app_url: &Url, asked: &str) -> bool {
    asked.trim_end_matches('/') == resource(app_url)
}

/// Adds the discovery documents, the token endpoint and the revocation endpoint.
///
/// Discovery paths this server doesn't publish answer 404 as JSON: the web app's HTML answering
/// 200 reads to a client as broken JSON.
pub fn routes(router: Router<AppState>) -> Router<AppState> {
    let mut router = router
        .route(PROTECTED_RESOURCE_PATH, get(metadata::protected_resource))
        .route(
            &format!("{PROTECTED_RESOURCE_PATH}/mcp"),
            get(metadata::protected_resource),
        )
        .route(
            AUTHORIZATION_SERVER_PATH,
            get(metadata::authorization_server),
        )
        .route("/oauth/authorize", get(authorize::to_consent_page))
        .route("/oauth/token", post(token::token))
        .route("/oauth/revoke", post(token::revoke));
    for path in [
        PROTECTED_RESOURCE_PATH,
        AUTHORIZATION_SERVER_PATH,
        "/.well-known/openid-configuration",
    ] {
        router = router.route(&format!("{path}/{{*suffix}}"), any(routes::not_found));
    }
    router.route("/.well-known/openid-configuration", any(routes::not_found))
}
