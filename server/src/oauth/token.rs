//! The token endpoint: a code becomes tokens, and a refresh token becomes the next pair

use std::net::{IpAddr, Ipv6Addr};

use axum::{
    Form, Json,
    extract::{Request, State, rejection::FormRejection},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::prelude::{BASE64_URL_SAFE_NO_PAD, Engine as _};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgConnection;
use subtle::ConstantTimeEq;
use tower_governor::key_extractor::KeyExtractor;
use uuid::Uuid;

use super::{ACCESS_TTL, REFRESH_GRACE, REFRESH_TTL, REPLACED_KEPT_DAYS, SCOPE, is_our_resource};
use crate::{
    agents::{
        Grant, alerts,
        key::{self, Issued},
    },
    db::{
        agent_connections::{self, Credential, NewConnection},
        agent_sign_ins::{self, SignIn},
        agent_tokens::{self, NewTokens},
    },
    email::templates::AgentAlert,
    error::AppError,
    rate_limit::ClientIp,
    state::AppState,
};

#[derive(Deserialize)]
pub struct TokenRequest {
    grant_type: Option<String>,
    code: Option<String>,
    redirect_uri: Option<String>,
    client_id: Option<String>,
    code_verifier: Option<String>,
    refresh_token: Option<String>,
    resource: Option<String>,
}

#[derive(Deserialize)]
pub struct RevokeRequest {
    token: Option<String>,
}

/// An error as RFC 6749 section 5.2 shapes it
enum Refused {
    /// The code or refresh token doesn't work: wrong, used, expired, or not this client's
    Grant(&'static str),
    Request(&'static str),
    UnsupportedGrantType,
    Target,
    Internal(AppError),
}

impl<E: Into<AppError>> From<E> for Refused {
    fn from(error: E) -> Self {
        Self::Internal(error.into())
    }
}

impl IntoResponse for Refused {
    fn into_response(self) -> Response {
        let (error, description) = match self {
            Self::Grant(description) => ("invalid_grant", description),
            Self::Request(description) => ("invalid_request", description),
            Self::UnsupportedGrantType => (
                "unsupported_grant_type",
                "grant_type must be authorization_code or refresh_token",
            ),
            Self::Target => ("invalid_target", "resource must be this server's /mcp URL"),
            Self::Internal(error) => return error.into_response(),
        };
        (
            StatusCode::BAD_REQUEST,
            no_store(),
            Json(json!({ "error": error, "error_description": description })),
        )
            .into_response()
    }
}

const fn no_store() -> [(header::HeaderName, &'static str); 2] {
    [
        (header::CACHE_CONTROL, "no-store"),
        (header::PRAGMA, "no-cache"),
    ]
}

/// `POST /oauth/token`. Clients are public: none holds a secret, and PKCE ties a code to the
/// client that asked for it.
pub async fn token(State(state): State<AppState>, request: Request) -> Response {
    if !state.agent_access {
        return AppError::NotFound.into_response();
    }
    let ip = client_ip(&request);
    let form = match read::<TokenRequest>(&state, request).await {
        Ok(form) => form,
        Err(refused) => return refused.into_response(),
    };
    let presented = match form.grant_type.as_deref() {
        Some("authorization_code") => form.code.as_deref().filter(|code| Issued::Code.is(code)),
        Some("refresh_token") => form
            .refresh_token
            .as_deref()
            .filter(|token| Issued::RefreshToken.is(token)),
        _ => return Refused::UnsupportedGrantType.into_response(),
    };
    // Counted per address like a wrong access key, so guesses cost the database nothing for long
    let Some(hash) = presented.map(key::hash) else {
        return failed(
            &state,
            ip,
            Refused::Grant("The code or token isn't one of ours"),
        );
    };
    if let Err(limited) = state.rate_limits.agent_key_lookup(ip, &hash) {
        return limited.into_response();
    }
    if form
        .resource
        .as_deref()
        .is_some_and(|asked| !is_our_resource(&state.app_url, asked))
    {
        return Refused::Target.into_response();
    }

    let issued = if form.grant_type.as_deref() == Some("authorization_code") {
        exchange(&state, &hash, &form).await
    } else {
        refresh(&state, &hash, &form).await
    };
    match issued {
        Ok(tokens) => {
            state.rate_limits.agent_key_worked(hash);
            (
                no_store(),
                Json(json!({
                    "access_token": tokens.access,
                    "token_type": "Bearer",
                    "expires_in": ACCESS_TTL.num_seconds(),
                    "refresh_token": tokens.refresh,
                    "scope": SCOPE,
                })),
            )
                .into_response()
        }
        Err(refused @ Refused::Grant(_)) => failed(&state, ip, refused),
        Err(refused) => refused.into_response(),
    }
}

/// `POST /oauth/revoke` (RFC 7009): signs out the connection the token belongs to. Answers 200
/// whether or not the token was known.
pub async fn revoke(State(state): State<AppState>, request: Request) -> Response {
    if !state.agent_access {
        return AppError::NotFound.into_response();
    }
    let ip = client_ip(&request);
    let form = match read::<RevokeRequest>(&state, request).await {
        Ok(form) => form,
        Err(refused) => return refused.into_response(),
    };
    let token = form
        .token
        .as_deref()
        .filter(|token| Issued::AccessToken.is(token) || Issued::RefreshToken.is(token));
    let Some(hash) = token.map(key::hash) else {
        return (no_store(), Json(json!({}))).into_response();
    };
    if let Err(limited) = state.rate_limits.agent_key_lookup(ip, &hash) {
        return limited.into_response();
    }
    let signed_out = async {
        let Some(connection_id) = agent_tokens::connection_of(&state.pool, &hash).await? else {
            return Ok(false);
        };
        agent_tokens::delete_all(&state.pool, connection_id).await?;
        sqlx::Result::Ok(true)
    }
    .await;
    match signed_out {
        Ok(true) => (no_store(), Json(json!({}))).into_response(),
        Ok(false) => match state.rate_limits.agent_key_failure(ip) {
            Ok(()) => (no_store(), Json(json!({}))).into_response(),
            Err(limited) => limited.into_response(),
        },
        Err(error) => AppError::Db(error).into_response(),
    }
}

fn client_ip(request: &Request) -> IpAddr {
    ClientIp
        .extract(request)
        .unwrap_or(IpAddr::V6(Ipv6Addr::UNSPECIFIED))
}

async fn read<T: serde::de::DeserializeOwned>(
    state: &AppState,
    request: Request,
) -> Result<T, Refused> {
    use axum::extract::FromRequest;

    match Form::<T>::from_request(request, state).await {
        Ok(Form(form)) => Ok(form),
        Err(FormRejection::InvalidFormContentType(_)) => Err(Refused::Request(
            "The body must be application/x-www-form-urlencoded",
        )),
        Err(_) => Err(Refused::Request("The form couldn't be read")),
    }
}

/// Counts a code or token that didn't work against the address it came from
fn failed(state: &AppState, ip: IpAddr, refused: Refused) -> Response {
    match state.rate_limits.agent_key_failure(ip) {
        Ok(()) => refused.into_response(),
        Err(limited) => limited.into_response(),
    }
}

struct Tokens {
    access: String,
    refresh: String,
}

/// Issues a pair for `connection_id` inside the caller's transaction
async fn issue(transaction: &mut PgConnection, connection_id: Uuid) -> Result<Tokens, Refused> {
    let (access, access_hash) = Issued::AccessToken.generate()?;
    let (refresh, refresh_hash) = Issued::RefreshToken.generate()?;
    let now = Utc::now();
    agent_tokens::insert(
        transaction,
        NewTokens {
            connection_id,
            access_hash: &access_hash,
            refresh_hash: &refresh_hash,
            access_expires_at: now + ACCESS_TTL,
            refresh_expires_at: now + REFRESH_TTL,
        },
    )
    .await?;
    Ok(Tokens { access, refresh })
}

const WRONG_CODE: &str = "The code is wrong, was used already or has expired";

async fn exchange(
    state: &AppState,
    hash: &[u8; 32],
    form: &TokenRequest,
) -> Result<Tokens, Refused> {
    let mut transaction = state.pool.begin().await?;
    let Some(sign_in) = agent_sign_ins::find_locked(&mut transaction, hash).await? else {
        return Err(Refused::Grant(WRONG_CODE));
    };
    if sign_in.used {
        // Shown twice, so it was seen by someone else: what it led to stops working (OAuth 2.1,
        // section 4.1.3)
        if let Some(connection_id) = sign_in.connection_id {
            agent_tokens::delete_all(&mut *transaction, connection_id).await?;
            transaction.commit().await?;
            tracing::warn!(%connection_id, "an authorization code was used twice; signed out");
        }
        return Err(Refused::Grant(WRONG_CODE));
    }
    if sign_in.expired {
        return Err(Refused::Grant(WRONG_CODE));
    }
    if form.client_id.as_deref() != Some(&sign_in.client_id)
        || form.redirect_uri.as_deref() != Some(&sign_in.redirect_uri)
    {
        return Err(Refused::Grant(
            "client_id and redirect_uri must be the ones the code was asked for with",
        ));
    }
    if !form
        .code_verifier
        .as_deref()
        .is_some_and(|verifier| verifies(verifier, &sign_in.code_challenge))
    {
        return Err(Refused::Grant(
            "code_verifier doesn't match the code_challenge",
        ));
    }

    let (connection_id, created) = connect(&mut transaction, &sign_in).await?;
    let tokens = issue(&mut transaction, connection_id).await?;
    agent_sign_ins::mark_used(&mut *transaction, sign_in.id, connection_id).await?;
    transaction.commit().await?;
    if let Some(name) = created {
        alerts::send(state, sign_in.user_id, AgentAlert::Created { name: &name }).await;
    }
    Ok(tokens)
}

/// The connection the code is for: the one it names, signed out of what it held, or a new one,
/// whose name is handed back
async fn connect(
    transaction: &mut PgConnection,
    sign_in: &SignIn,
) -> Result<(Uuid, Option<String>), Refused> {
    const GONE: Refused = Refused::Grant("The connection this code was for no longer exists");
    if let (Some(name), Some(grant)) = (&sign_in.name, &sign_in.grant) {
        let grant = Grant::parse(grant).map_err(|_| GONE)?;
        let connection = agent_connections::create_in(
            transaction,
            NewConnection {
                user_id: sign_in.user_id,
                name,
                credential: Credential::SignIn {
                    client_id: &sign_in.client_id,
                    client_name: &sign_in.client_name,
                },
                grant: &grant,
                expires_at: None,
            },
        )
        .await?
        .ok_or(Refused::Grant(
            "The account already has the most connections it can have",
        ))?;
        return Ok((connection.id, Some(connection.name)));
    }
    let id = sign_in.connection_id.ok_or(GONE)?;
    let same_client = agent_connections::find(&mut *transaction, id, sign_in.user_id)
        .await?
        .is_some_and(|connection| connection.client_id.as_deref() == Some(&sign_in.client_id));
    if !same_client {
        return Err(GONE);
    }
    agent_tokens::delete_all(&mut *transaction, id).await?;
    Ok((id, None))
}

/// PKCE with S256 (RFC 7636): the challenge is the SHA-256 of the verifier
fn verifies(verifier: &str, challenge: &str) -> bool {
    let well_formed = (43..=128).contains(&verifier.len())
        && verifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'));
    let expected = BASE64_URL_SAFE_NO_PAD.encode(Sha256::digest(verifier));
    well_formed && bool::from(expected.as_bytes().ct_eq(challenge.as_bytes()))
}

async fn refresh(
    state: &AppState,
    hash: &[u8; 32],
    form: &TokenRequest,
) -> Result<Tokens, Refused> {
    const WRONG: Refused = Refused::Grant("The refresh token is wrong or has expired");
    let mut transaction = state.pool.begin().await?;
    let Some(found) = agent_tokens::find_refresh_locked(&mut transaction, hash).await? else {
        return Err(WRONG);
    };
    if found.expired {
        return Err(WRONG);
    }
    if form.client_id.as_deref() != found.client_id.as_deref() {
        return Err(Refused::Grant(
            "client_id must be the one the refresh token was issued to",
        ));
    }
    match found.replaced_at {
        None => agent_tokens::replace(&mut *transaction, found.id, REPLACED_KEPT_DAYS).await?,
        Some(at) if Utc::now() - at <= REFRESH_GRACE => {}
        Some(_) => {
            // A token already exchanged, shown again later: one of the two holders stole it, and
            // there's no telling which, so both are signed out
            agent_tokens::delete_all(&mut *transaction, found.connection_id).await?;
            transaction.commit().await?;
            tracing::warn!(
                connection_id = %found.connection_id,
                "a refresh token was used twice; signed out"
            );
            return Err(WRONG);
        }
    }
    let tokens = issue(&mut transaction, found.connection_id).await?;
    transaction.commit().await?;
    Ok(tokens)
}
