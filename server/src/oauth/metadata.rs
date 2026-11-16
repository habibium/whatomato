//! The two documents a client reads to find out how to sign in

use axum::{
    Json,
    extract::State,
    http::header,
    response::{IntoResponse, Response},
};
use serde_json::json;

use super::{SCOPE, issuer, resource};
use crate::{error::AppError, state::AppState};

/// RFC 9728: what the MCP endpoint is, and which server signs clients in to it
pub async fn protected_resource(State(state): State<AppState>) -> Response {
    if !state.agent_access {
        return AppError::NotFound.into_response();
    }
    public(json!({
        "resource": resource(&state.app_url),
        "authorization_servers": [issuer(&state.app_url)],
        "scopes_supported": [SCOPE],
        "bearer_methods_supported": ["header"],
        "resource_name": "Whatomato",
    }))
}

/// RFC 8414: where and how a client signs in
pub async fn authorization_server(State(state): State<AppState>) -> Response {
    if !state.agent_access {
        return AppError::NotFound.into_response();
    }
    let issuer = issuer(&state.app_url);
    public(json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{issuer}/oauth/authorize"),
        "token_endpoint": format!("{issuer}/oauth/token"),
        "revocation_endpoint": format!("{issuer}/oauth/revoke"),
        "scopes_supported": [SCOPE],
        "response_types_supported": ["code"],
        "response_modes_supported": ["query"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "revocation_endpoint_auth_methods_supported": ["none"],
        "client_id_metadata_document_supported": true,
        "authorization_response_iss_parameter_supported": true,
    }))
}

/// Nothing here is secret, and a client in a browser reads it from another origin
fn public(document: serde_json::Value) -> Response {
    (
        [
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
            (header::CACHE_CONTROL, "public, max-age=300"),
        ],
        Json(document),
    )
        .into_response()
}
