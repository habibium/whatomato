//! The consent page's API: what the owner is shown when an app asks to sign in, and their yes.
//! A no needs no call; the page sends the browser to `deny_url`.

use axum::{Json, extract::State};
use axum_extra::extract::cookie::CookieJar;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::{Validate, ValidationErrors};

use super::{
    dto::{AgentConnection, check_name},
    handlers::{AT_CAP, Enabled, checked},
};
use crate::{
    agents::{Grant, key::Issued},
    auth::{
        reauth::{self, Reauth},
        session::Session,
    },
    db::{
        agent_connections::{self, MAX_PER_USER},
        agent_sign_ins::{self, NewSignIn, Target},
    },
    error::{AppError, ErrorBody},
    extract::ValidatedJson,
    oauth::{
        self,
        authorize::{AuthorizationRequest, Refusal},
        client,
    },
    scheduler::draft::add,
    state::AppState,
};

/// What to do with an app's request to sign in
#[derive(Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum SignInCheck {
    /// The request is sound: ask the owner
    Ask(SignInAsk),
    /// Show `message` and stay on the page
    Refused { message: String },
    /// Send the browser to `url`, which tells the app what was wrong with its request
    Redirect { url: String },
}

#[derive(Serialize, ToSchema)]
pub(super) struct SignInAsk {
    /// What the app calls itself, which anyone can claim
    pub client_name: String,
    /// The host the app's identity is published on, such as `claude.ai`
    pub client_host: String,
    /// The host the browser is sent back to afterwards
    pub redirect_host: String,
    /// The app is reached only on this computer, where any program can claim to be it
    pub is_local_only: bool,
    /// The connections this app already signed in to, which it can sign in to again
    pub connections: Vec<AgentConnection>,
    /// Whether the account has room for another connection
    pub can_add: bool,
    /// Where to send the browser when the owner says no
    pub deny_url: String,
}

#[derive(Deserialize, ToSchema)]
pub(super) struct ApproveSignIn {
    pub request: AuthorizationRequest,
    #[serde(flatten)]
    pub reauth: Reauth,
    /// Sign in again to this connection, keeping its permissions. Left out for a new connection.
    pub connection_id: Option<Uuid>,
    /// A new connection's name, 1 to 60 characters
    pub name: Option<String>,
    /// A new connection's permissions
    pub grant: Option<Grant>,
}

impl Validate for ApproveSignIn {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();
        self.reauth.check(&mut errors);
        match (&self.connection_id, &self.name, &self.grant) {
            (Some(_), None, None) => {}
            (None, Some(name), Some(_)) => check_name(name, &mut errors),
            _ => add(
                &mut errors,
                "connection_id",
                "send either connection_id, or name and grant",
            ),
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

#[derive(Serialize, ToSchema)]
pub(super) struct SignInApproved {
    /// Where to send the browser: back to the app, with the code it exchanges for its tokens
    pub url: String,
}

/// Check an app's request to sign in
///
/// Looks the app up by its `client_id` and checks the request the browser arrived with. Nothing
/// is stored.
#[utoipa::path(post, path = "/sign-in/check", request_body = AuthorizationRequest, responses(
    (status = OK, body = SignInCheck),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = NOT_FOUND, description = "Agent access is off on this server", body = ErrorBody),
    (status = TOO_MANY_REQUESTS, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn check_agent_sign_in(
    _: Enabled,
    State(state): State<AppState>,
    session: Session,
    Json(request): Json<AuthorizationRequest>,
) -> Result<Json<SignInCheck>, AppError> {
    let asked = match oauth::authorize::check(&state, &request).await {
        Ok(asked) => asked,
        Err(Refusal::Page(message)) => return Ok(Json(SignInCheck::Refused { message })),
        Err(Refusal::Redirect(url)) => return Ok(Json(SignInCheck::Redirect { url })),
    };
    let user_id = session.user.id;
    let (all, mut usage) = tokio::try_join!(
        agent_connections::list(&state.pool, user_id),
        agent_connections::usage_all(&state.pool, user_id),
    )?;
    let can_add = i64::try_from(all.len()).is_ok_and(|count| count < MAX_PER_USER);
    let connections = all
        .into_iter()
        .filter(|connection| connection.client_id.as_deref() == Some(&asked.client.id))
        .map(|connection| {
            let used = usage.remove(&connection.id).unwrap_or_default();
            AgentConnection::new(connection, used)
        })
        .collect();
    Ok(Json(SignInCheck::Ask(SignInAsk {
        client_name: asked.client.name.clone(),
        client_host: client::host(&asked.client.id),
        redirect_host: client::redirect_host(&asked.redirect_uri),
        is_local_only: asked.client.is_local_only(),
        connections,
        can_add,
        deny_url: asked.denied(),
    })))
}

/// Let an app sign in
///
/// Checks the request again, then the password or a passkey, and answers with the address to send
/// the browser to. The connection is made, or signed in to again, when the app exchanges the code
/// in that address, which it has a minute to do; the owner is emailed about a new one then.
#[utoipa::path(post, path = "/sign-in/approve", request_body = ApproveSignIn, responses(
    (status = OK, body = SignInApproved),
    (status = BAD_REQUEST, description = "A field is missing or out of range, or the password or passkey is incorrect", body = ErrorBody),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = NOT_FOUND, description = "Agent access is off on this server, or the connection isn't one this app signed in to", body = ErrorBody),
    (status = CONFLICT, description = "The account already has 20 connections", body = ErrorBody),
    (status = UNPROCESSABLE_ENTITY, description = "The permissions have problems, under `details.grant`, or the app's request does, under `details.request`", body = ErrorBody),
    (status = TOO_MANY_REQUESTS, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn approve_agent_sign_in(
    _: Enabled,
    State(state): State<AppState>,
    jar: CookieJar,
    session: Session,
    ValidatedJson(input): ValidatedJson<ApproveSignIn>,
) -> Result<Json<SignInApproved>, AppError> {
    let user_id = session.user.id;
    let asked = oauth::authorize::check(&state, &input.request)
        .await
        .map_err(|refusal| AppError::Unprocessable {
            field: "request",
            problems: vec![match refusal {
                Refusal::Page(message) => message,
                Refusal::Redirect(_) => {
                    "The app's request is no longer valid. Start again from the app.".to_owned()
                }
            }],
        })?;
    let grant = input
        .grant
        .map(|grant| checked(grant, state.agent_reading))
        .transpose()?;
    let name = input.name.as_deref().map(str::trim);
    let target = match (input.connection_id, name, &grant) {
        (Some(id), ..) => {
            agent_connections::find(&state.pool, id, user_id)
                .await?
                .filter(|found| found.client_id.as_deref() == Some(&asked.client.id))
                .ok_or(AppError::NotFound)?;
            Target::Existing(id)
        }
        (None, Some(name), Some(grant)) => {
            let count = agent_connections::list(&state.pool, user_id).await?.len();
            if i64::try_from(count).is_ok_and(|count| count >= MAX_PER_USER) {
                return Err(AppError::Conflict(AT_CAP));
            }
            Target::New { name, grant }
        }
        // `validate` let only those two through
        _ => return Err(AppError::NotFound),
    };
    reauth::confirm(&state, &jar, user_id, input.reauth).await?;

    let (code, code_hash) = Issued::Code.generate()?;
    agent_sign_ins::create(
        &state.pool,
        NewSignIn {
            code_hash: &code_hash,
            user_id,
            client_id: &asked.client.id,
            client_name: &asked.client.name,
            redirect_uri: &asked.redirect_uri,
            code_challenge: &asked.code_challenge,
            target,
            expires_at: Utc::now() + oauth::CODE_TTL,
        },
    )
    .await?;
    Ok(Json(SignInApproved {
        url: asked.redirect(&[("code", &code)]),
    }))
}
