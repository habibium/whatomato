//! Checking an authorization request: what a client sends the browser to the consent page with

use std::sync::Arc;

use axum::{
    extract::{RawQuery, State},
    http::{StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use base64::prelude::{BASE64_URL_SAFE_NO_PAD, Engine as _};
use serde::Deserialize;
use url::Url;
use utoipa::ToSchema;

use super::{client::Client, is_our_resource, issuer};
use crate::{error::AppError, state::AppState};

const STATE_MAX: usize = 2048;
const QUERY_MAX: usize = 8 * 1024;

/// `GET /oauth/authorize`: sends the browser on to the consent page.
///
/// The query travels packed into one parameter, as the web app's router would otherwise read a
/// `state` of digits as a number and hand it back changed.
pub async fn to_consent_page(State(state): State<AppState>, RawQuery(query): RawQuery) -> Response {
    if !state.agent_access {
        return AppError::NotFound.into_response();
    }
    let query = query.unwrap_or_default();
    if query.len() > QUERY_MAX {
        return StatusCode::URI_TOO_LONG.into_response();
    }
    let packed = BASE64_URL_SAFE_NO_PAD.encode(query);
    (
        [(header::CACHE_CONTROL, "no-store")],
        Redirect::to(&format!("/agents/sign-in?request=q.{packed}")),
    )
        .into_response()
}

/// The query the client sent the browser to `/oauth/authorize` with, passed on as it came
#[derive(Clone, Debug, Default, Deserialize, ToSchema)]
pub struct AuthorizationRequest {
    pub response_type: Option<String>,
    pub client_id: Option<String>,
    pub redirect_uri: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub state: Option<String>,
    pub resource: Option<String>,
    pub scope: Option<String>,
}

/// A request that may go to the owner for a yes or no
#[derive(Debug)]
pub struct Checked {
    pub client: Arc<Client>,
    pub redirect_uri: String,
    pub code_challenge: String,
    state: Option<String>,
    issuer: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Shown to the owner. Until the redirect address is proven the client's, the browser is
    /// never sent to it.
    Page(String),
    /// The address to send the browser back to, carrying the error for the client
    Redirect(String),
}

/// # Errors
/// See [`Refusal`].
pub async fn check(state: &AppState, request: &AuthorizationRequest) -> Result<Checked, Refusal> {
    let page = |message: &str| Refusal::Page(message.to_owned());
    let client_id = request.client_id.as_deref().ok_or_else(|| {
        page("This sign-in link is incomplete: it doesn't say which app is asking.")
    })?;
    let client = state
        .oauth_clients
        .get(client_id)
        .await
        .map_err(|error| Refusal::Page(error.to_string()))?;
    let redirect_uri = request
        .redirect_uri
        .as_deref()
        .filter(|uri| client.allows_redirect(uri))
        .ok_or_else(|| {
            page(
                "This sign-in link would send you somewhere the app hasn't registered, so it was \
                 stopped. Start again from the app.",
            )
        })?;
    if request
        .state
        .as_ref()
        .is_some_and(|state| state.len() > STATE_MAX)
    {
        return Err(page("This sign-in link is too long to be real."));
    }
    let checked = Checked {
        client,
        redirect_uri: redirect_uri.to_owned(),
        code_challenge: request.code_challenge.clone().unwrap_or_default(),
        state: request.state.clone(),
        issuer: issuer(&state.app_url),
    };

    let refuse = |error: &str, description: &str| {
        Err(Refusal::Redirect(checked.redirect(&[
            ("error", error),
            ("error_description", description),
        ])))
    };
    if request.response_type.as_deref() != Some("code") {
        return refuse(
            "unsupported_response_type",
            "response_type must be \"code\"",
        );
    }
    if request.code_challenge_method.as_deref() != Some("S256") {
        return refuse("invalid_request", "code_challenge_method must be S256");
    }
    if !is_challenge(&checked.code_challenge) {
        return refuse(
            "invalid_request",
            "code_challenge must be the base64url SHA-256 of the code_verifier",
        );
    }
    if request
        .resource
        .as_deref()
        .is_some_and(|asked| !is_our_resource(&state.app_url, asked))
    {
        return refuse("invalid_target", "resource must be this server's /mcp URL");
    }
    Ok(checked)
}

impl Checked {
    /// The redirect address with `params`, the client's `state` and this server's `iss` (RFC 9207)
    #[must_use]
    pub fn redirect(&self, params: &[(&str, &str)]) -> String {
        // Parsed once already, by `Client::allows_redirect`
        let Ok(mut url) = Url::parse(&self.redirect_uri) else {
            return self.redirect_uri.clone();
        };
        {
            let mut query = url.query_pairs_mut();
            query.extend_pairs(params);
            if let Some(state) = &self.state {
                query.append_pair("state", state);
            }
            query.append_pair("iss", &self.issuer);
        }
        url.into()
    }

    /// Where the browser goes when the owner says no
    #[must_use]
    pub fn denied(&self) -> String {
        self.redirect(&[
            ("error", "access_denied"),
            ("error_description", "The owner didn't allow the connection"),
        ])
    }
}

/// A SHA-256 in base64url without padding
fn is_challenge(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
