//! The MCP endpoint at `/mcp`, where AI agents call the operations in [`crate::agents::ops`].
//!
//! Outside `/api`: an agent proves who it is with `Authorization: Bearer <key>` alone, so neither
//! cookies nor the CSRF layer play a part. Both protocol eras share the one path, without sessions,
//! answering in plain JSON.

mod tools;

#[cfg(test)]
mod tests;

use std::{
    net::{IpAddr, Ipv6Addr},
    sync::Arc,
};

use axum::{
    Json, Router,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, Uri, header, uri::Authority},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::never::NeverSessionManager,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;
use tower_governor::key_extractor::KeyExtractor;
use tower_http::set_header::SetResponseHeaderLayer;
use url::Url;

use crate::{
    agents::{
        key,
        ops::{Caller, OpError},
    },
    db::agent_connections,
    error::AppError,
    rate_limit::ClientIp,
    state::AppState,
};

/// Far over a tool call's needs: a message is at most 4,096 characters, 24 kB even escaped
const BODY_LIMIT: usize = 64 * 1024;

const NO_KEY: &str = "This needs a working Whatomato access key, sent as `Authorization: Bearer \
                      <key>`. The account's owner creates connections and their keys in the \
                      Whatomato web app.";

/// `/mcp`, and the token that stops rmcp's own tasks. Cancel it only once the server has stopped
/// taking requests, or calls still running are cut off.
pub fn router(state: AppState, port: u16) -> (Router, CancellationToken) {
    let shutdown = CancellationToken::new();
    let sites = Arc::new(Sites::new(&state.app_url, port));
    let tools = tools::Whatomato::new(state.clone());
    // rmcp checks Host and Origin again; `same_site` runs first, so its answer is the one seen
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_allowed_hosts(sites.host_names())
        .with_allowed_origins(sites.origins())
        .with_max_request_body_bytes(BODY_LIMIT)
        .with_cancellation_token(shutdown.child_token());
    let service = StreamableHttpService::new(
        move || Ok(tools.clone()),
        Arc::new(NeverSessionManager::default()),
        config,
    );
    let router = Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn_with_state(state.clone(), bearer))
        // Outside the key check, so a too-long `Content-Length` costs no lookup; a body without
        // one is cut off by rmcp as it streams, with the same limit
        .layer(middleware::from_fn(declared_length))
        .layer(middleware::from_fn_with_state(sites, same_site))
        .layer(middleware::from_fn_with_state(state, switch))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ));
    (router, shutdown)
}

async fn switch(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if state.agent_access {
        next.run(request).await
    } else {
        AppError::NotFound.into_response()
    }
}

/// The hosts and browser origins `/mcp` answers: the app's, and in a debug build also the API port
/// on loopback, which a client may call directly instead of through the Vite proxy
struct Sites {
    /// Host name, port, and the port a `Host` without one means
    hosts: Vec<(String, u16, u16)>,
    /// Scheme, host name, port
    origins: Vec<(String, String, u16)>,
}

impl Sites {
    fn new(app_url: &Url, port: u16) -> Self {
        let mut sites = Self {
            hosts: Vec::new(),
            origins: Vec::new(),
        };
        if let (Some(host), Some(app_port)) = (app_url.host_str(), app_url.port_or_known_default())
        {
            let default = if app_url.scheme() == "https" { 443 } else { 80 };
            sites.add(app_url.scheme(), host, app_port, default);
        }
        if cfg!(debug_assertions) {
            for host in ["localhost", "127.0.0.1"] {
                sites.add("http", host, port, 80);
            }
        }
        sites
    }

    fn add(&mut self, scheme: &str, host: &str, port: u16, default: u16) {
        let host = host.to_ascii_lowercase();
        self.hosts.push((host.clone(), port, default));
        self.origins.push((scheme.to_owned(), host, port));
    }

    /// For rmcp, which then takes any port on them
    fn host_names(&self) -> Vec<String> {
        self.hosts.iter().map(|(host, ..)| host.clone()).collect()
    }

    /// For rmcp, with the port spelled out: it reads a portless entry as any port
    fn origins(&self) -> Vec<String> {
        self.origins
            .iter()
            .map(|(scheme, host, port)| format!("{scheme}://{host}:{port}"))
            .collect()
    }

    fn allows_host(&self, uri: &Uri, headers: &HeaderMap) -> bool {
        // HTTP/2 carries the host in `:authority`, which reaches the URI instead of `Host`
        let authority: Option<Authority> = headers.get(header::HOST).map_or_else(
            || uri.authority().cloned(),
            |value| value.to_str().ok().and_then(|value| value.parse().ok()),
        );
        let Some(authority) = authority else {
            return false;
        };
        let host = authority.host().to_ascii_lowercase();
        self.hosts.iter().any(|(allowed, port, default)| {
            *allowed == host && authority.port_u16().unwrap_or(*default) == *port
        })
    }

    /// A request without `Origin` comes from no browser, which is how agents call
    fn allows_origin(&self, headers: &HeaderMap) -> bool {
        let Some(origin) = headers.get(header::ORIGIN) else {
            return true;
        };
        let Some(origin) = origin
            .to_str()
            .ok()
            .and_then(|value| Url::parse(value).ok())
        else {
            return false;
        };
        let (Some(host), Some(port)) = (origin.host_str(), origin.port_or_known_default()) else {
            return false;
        };
        let host = host.to_ascii_lowercase();
        self.origins.iter().any(|(scheme, allowed, allowed_port)| {
            scheme == origin.scheme() && *allowed == host && *allowed_port == port
        })
    }
}

/// Refuses a body declared longer than [`BODY_LIMIT`] before it's read
async fn declared_length(request: Request, next: Next) -> Response {
    let declared = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok()?.parse::<u64>().ok());
    if declared.is_some_and(|length| length > BODY_LIMIT as u64) {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({ "error": "This request is too large. A tool call needs far less." })),
        )
            .into_response();
    }
    next.run(request).await
}

/// Refuses another site's page, key or not, so it sits outside [`bearer`]
async fn same_site(State(sites): State<Arc<Sites>>, request: Request, next: Next) -> Response {
    if !sites.allows_host(request.uri(), request.headers()) {
        return forbidden("This host isn't one Whatomato answers on.");
    }
    if !sites.allows_origin(request.headers()) {
        return forbidden("Requests from web pages on other sites are refused.");
    }
    next.run(request).await
}

fn forbidden(message: &str) -> Response {
    (StatusCode::FORBIDDEN, Json(json!({ "error": message }))).into_response()
}

/// Why a request didn't authenticate
enum Refusal {
    /// A key no connection has
    NoKey,
    /// The connection exists but its grant can't be read
    Unusable,
    /// Paused or expired. The caller holds a real key, so it may hear which.
    Stopped(String),
    Internal(sqlx::Error),
}

/// Finds the connection behind the bearer key, limits its requests, and hands the [`Caller`] on in
/// the request's extensions. Keys are never logged.
async fn bearer(State(state): State<AppState>, mut request: Request, next: Next) -> Response {
    let ip = ClientIp
        .extract(&request)
        .unwrap_or(IpAddr::V6(Ipv6Addr::UNSPECIFIED));
    let has_header = request.headers().contains_key(header::AUTHORIZATION);
    // Refused by its form alone, so no key and a malformed one never cost a lookup
    let Some(key_hash) = presented(request.headers()).map(key::hash) else {
        return failed(&state, ip, has_header, NO_KEY);
    };
    if let Err(limited) = state.rate_limits.agent_key_lookup(ip, &key_hash) {
        return limited.into_response();
    }
    let caller = match authenticate(&state, key_hash).await {
        Ok(caller) => caller,
        Err(Refusal::Internal(error)) => return AppError::Db(error).into_response(),
        Err(Refusal::NoKey) => return failed(&state, ip, has_header, NO_KEY),
        Err(Refusal::Stopped(message)) => return unauthorized(has_header, &message),
        Err(Refusal::Unusable) => return unauthorized(has_header, NO_KEY),
    };
    let id = caller.connection().id;
    if let Err(limited) = state.rate_limits.agent_call(id) {
        return limited.into_response();
    }
    if let Err(error) = agent_connections::touch(&state.pool, id).await {
        tracing::warn!(connection_id = %id, ?error, "couldn't note an agent connection's use");
    }
    request.extensions_mut().insert(Arc::new(caller));
    next.run(request).await
}

/// The access key in `Authorization`, if it's shaped like one
fn presented(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map(|(_, key)| key.trim())
        .filter(|key| key::is_access_key(key))
}

async fn authenticate(state: &AppState, key_hash: [u8; 32]) -> Result<Caller, Refusal> {
    let found = agent_connections::find_by_key(&state.pool, &key_hash)
        .await
        .map_err(Refusal::Internal)?
        .ok_or(Refusal::NoKey)?;
    state.rate_limits.agent_key_worked(key_hash);
    Caller::from_key_match(&state.app_url, found).map_err(|error| match error {
        OpError::Refused {
            code: "paused" | "expired",
            ..
        } => Refusal::Stopped(error.to_string()),
        _ => Refusal::Unusable,
    })
}

/// Counts a request without a usable key against its address, then refuses it
fn failed(state: &AppState, ip: IpAddr, presented: bool, message: &str) -> Response {
    match state.rate_limits.agent_key_failure(ip) {
        Ok(()) => unauthorized(presented, message),
        Err(limited) => limited.into_response(),
    }
}

fn unauthorized(presented: bool, message: &str) -> Response {
    let challenge = if presented {
        r#"Bearer error="invalid_token""#
    } else {
        "Bearer"
    };
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, challenge)],
        Json(json!({ "error": message })),
    )
        .into_response()
}
