use axum::{
    Json,
    extract::{FromRequestParts, Path, State},
    http::{StatusCode, request::Parts},
};
use chrono::{TimeDelta, Utc};
use uuid::Uuid;

use super::dto::{
    ActivityQuery, AgentActivityPage, AgentApproval, AgentApprovalPage, AgentConnection,
    AgentConnectionDetail, AgentKeyIssued, AgentPreset, AgentRevoked, AgentsOverview, AgentsPaused,
    ApprovalsQuery, ApproveRequest, CreateConnection, RevokeQuery, RotateKeyRequest, UNREADABLE,
    UpdateConnection,
};
use crate::{
    agents::{
        ApprovalStatus, Grant, Preset, alerts,
        key::AccessKey,
        ops::review::{self, Requested, ReviewError},
    },
    auth::{reauth, session::Session},
    db::{
        agent_activity, agent_approvals,
        agent_connections::{self, Connection, MAX_PER_USER, NewConnection},
    },
    email::templates::AgentAlert,
    error::{AppError, ErrorBody},
    extract::{ValidatedJson, ValidatedQuery},
    state::AppState,
};

/// Answers 404 when the instance has agent access off, as if the route didn't exist
pub(super) struct Enabled;

impl FromRequestParts<AppState> for Enabled {
    type Rejection = AppError;

    fn from_request_parts(
        _: &mut Parts,
        state: &AppState,
    ) -> impl Future<Output = Result<Self, AppError>> + Send {
        let enabled = state.agent_access;
        async move {
            if enabled {
                Ok(Self)
            } else {
                Err(AppError::NotFound)
            }
        }
    }
}

const AT_CAP: &str = "You already have 20 agent connections, the most an account can have. \
                      Revoke one you no longer use, then try again.";

/// Whether agent access is on, the account's agent connections and how many approvals wait
///
/// Answers even when agent access is off on this server, with `enabled: false`.
#[utoipa::path(get, path = "/", responses(
    (status = OK, body = AgentsOverview),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn list_agents(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<AgentsOverview>, AppError> {
    if !state.agent_access {
        return Ok(Json(AgentsOverview {
            enabled: false,
            connections: Vec::new(),
            approvals_waiting: 0,
        }));
    }
    let user_id = session.user.id;
    let (connections, mut usage, approvals_waiting) = tokio::try_join!(
        agent_connections::list(&state.pool, user_id),
        agent_connections::usage_all(&state.pool, user_id),
        agent_approvals::count_waiting(&state.pool, user_id),
    )?;
    let connections = connections
        .into_iter()
        .map(|connection| {
            let used = usage.remove(&connection.id).unwrap_or_default();
            AgentConnection::new(connection, used)
        })
        .collect();
    Ok(Json(AgentsOverview {
        enabled: true,
        connections,
        approvals_waiting,
    }))
}

/// The permission presets, in the order to offer them
#[utoipa::path(get, path = "/presets", responses(
    (status = OK, body = Vec<AgentPreset>),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = NOT_FOUND, description = "Agent access is off on this server", body = ErrorBody),
))]
pub async fn list_agent_presets(_: Enabled, _session: Session) -> Json<Vec<AgentPreset>> {
    Json(Preset::ALL.into_iter().map(AgentPreset::new).collect())
}

/// Connect an AI agent
///
/// Checks the permissions, then the password, and answers with the access key, shown this once.
/// Emails the owner.
#[utoipa::path(post, path = "/", request_body = CreateConnection, responses(
    (status = CREATED, body = AgentKeyIssued),
    (status = BAD_REQUEST, description = "A field is missing or out of range, or the password is incorrect", body = ErrorBody),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = NOT_FOUND, description = "Agent access is off on this server", body = ErrorBody),
    (status = CONFLICT, description = "The account already has 20 connections", body = ErrorBody),
    (status = UNPROCESSABLE_ENTITY, description = "The permissions have problems, one sentence each under `details.grant`", body = ErrorBody),
    (status = TOO_MANY_REQUESTS, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn create_agent(
    _: Enabled,
    State(state): State<AppState>,
    session: Session,
    ValidatedJson(input): ValidatedJson<CreateConnection>,
) -> Result<(StatusCode, Json<AgentKeyIssued>), AppError> {
    let user_id = session.user.id;
    let grant = checked(input.grant)?;
    reauth::confirm_password(&state, user_id, input.password).await?;
    let key = AccessKey::generate()?;
    let connection = agent_connections::create(
        &state.pool,
        NewConnection {
            user_id,
            name: input.name.trim(),
            key_hash: &key.hash,
            key_hint: &key.hint,
            grant: &grant,
            expires_at: input
                .expires_in_days
                .map(|days| Utc::now() + TimeDelta::days(days.into())),
        },
    )
    .await?
    .ok_or(AppError::Conflict(AT_CAP))?;
    alerts::send(
        &state,
        user_id,
        AgentAlert::Created {
            name: &connection.name,
        },
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(AgentKeyIssued {
            connection: AgentConnection::new(connection, (0, 0)),
            key: key.value,
        }),
    ))
}

/// One connection, with how many of its scheduled messages still exist
#[utoipa::path(get, path = "/{id}", params(("id" = Uuid, Path)), responses(
    (status = OK, body = AgentConnectionDetail),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = NOT_FOUND, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn get_agent(
    _: Enabled,
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<Json<AgentConnectionDetail>, AppError> {
    let user_id = session.user.id;
    let connection = agent_connections::find(&state.pool, id, user_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let (usage, schedules) = tokio::try_join!(
        agent_connections::usage(&state.pool, id),
        agent_connections::schedules(&state.pool, id, user_id),
    )?;
    Ok(Json(AgentConnectionDetail {
        connection: AgentConnection::new(connection, usage),
        schedules,
    }))
}

/// Rename a connection, change its permissions, or both
///
/// A rename alone needs nothing more. New permissions need the password and email the owner when
/// they differ from the old ones.
#[utoipa::path(patch, path = "/{id}", params(("id" = Uuid, Path)), request_body = UpdateConnection, responses(
    (status = OK, body = AgentConnection),
    (status = BAD_REQUEST, description = "Nothing to change, a field is out of range, or the password is missing or incorrect", body = ErrorBody),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = NOT_FOUND, body = ErrorBody),
    (status = UNPROCESSABLE_ENTITY, description = "The permissions have problems, one sentence each under `details.grant`", body = ErrorBody),
    (status = TOO_MANY_REQUESTS, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn update_agent(
    _: Enabled,
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    ValidatedJson(input): ValidatedJson<UpdateConnection>,
) -> Result<Json<AgentConnection>, AppError> {
    let user_id = session.user.id;
    let current = agent_connections::find(&state.pool, id, user_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let grant = input.grant.map(checked).transpose()?;
    if grant.is_some() {
        let password = input.password.unwrap_or_default();
        reauth::confirm_password(&state, user_id, password).await?;
    }

    let mut transaction = state.pool.begin().await?;
    let renamed = match &input.name {
        Some(name) => {
            agent_connections::rename(&mut *transaction, id, user_id, name.trim()).await?
        }
        None => None,
    };
    let updated = match &grant {
        Some(grant) => agent_connections::set_grant(&mut *transaction, id, user_id, grant).await?,
        None => renamed,
    };
    let updated = updated.ok_or(AppError::NotFound)?;
    transaction.commit().await?;

    let changed = grant.is_some_and(|grant| Grant::parse(&current.grant).ok() != Some(grant));
    if changed {
        alerts::send(
            &state,
            user_id,
            AgentAlert::PermissionsChanged {
                name: &updated.name,
            },
        )
        .await;
    }
    with_usage(&state, updated).await
}

/// Pause a connection: every request it makes is refused until it's resumed
#[utoipa::path(post, path = "/{id}/pause", params(("id" = Uuid, Path)), responses(
    (status = OK, body = AgentConnection),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = NOT_FOUND, body = ErrorBody),
    (status = TOO_MANY_REQUESTS, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn pause_agent(
    _: Enabled,
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<Json<AgentConnection>, AppError> {
    let connection = agent_connections::pause(&state.pool, id, session.user.id)
        .await?
        .ok_or(AppError::NotFound)?;
    with_usage(&state, connection).await
}

/// Resume a paused connection
#[utoipa::path(post, path = "/{id}/resume", params(("id" = Uuid, Path)), responses(
    (status = OK, body = AgentConnection),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = NOT_FOUND, body = ErrorBody),
    (status = TOO_MANY_REQUESTS, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn resume_agent(
    _: Enabled,
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<Json<AgentConnection>, AppError> {
    let connection = agent_connections::resume(&state.pool, id, session.user.id)
        .await?
        .ok_or(AppError::NotFound)?;
    with_usage(&state, connection).await
}

/// Pause every connection of the account at once
#[utoipa::path(post, path = "/pause-all", responses(
    (status = OK, body = AgentsPaused),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = NOT_FOUND, description = "Agent access is off on this server", body = ErrorBody),
    (status = TOO_MANY_REQUESTS, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn pause_all_agents(
    _: Enabled,
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<AgentsPaused>, AppError> {
    let paused = agent_connections::pause_all(&state.pool, session.user.id).await?;
    Ok(Json(AgentsPaused { paused }))
}

/// Replace a connection's key
///
/// The old key stops working at once. Answers with the new key, shown this once, and emails the
/// owner.
#[utoipa::path(post, path = "/{id}/rotate-key", params(("id" = Uuid, Path)), request_body = RotateKeyRequest, responses(
    (status = OK, body = AgentKeyIssued),
    (status = BAD_REQUEST, description = "The password is incorrect", body = ErrorBody),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = NOT_FOUND, body = ErrorBody),
    (status = TOO_MANY_REQUESTS, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn rotate_agent_key(
    _: Enabled,
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    ValidatedJson(input): ValidatedJson<RotateKeyRequest>,
) -> Result<Json<AgentKeyIssued>, AppError> {
    let user_id = session.user.id;
    agent_connections::find(&state.pool, id, user_id)
        .await?
        .ok_or(AppError::NotFound)?;
    reauth::confirm_password(&state, user_id, input.password).await?;
    let key = AccessKey::generate()?;
    let connection = agent_connections::rotate_key(&state.pool, id, user_id, &key.hash, &key.hint)
        .await?
        .ok_or(AppError::NotFound)?;
    alerts::send(
        &state,
        user_id,
        AgentAlert::KeyReplaced {
            name: &connection.name,
        },
    )
    .await;
    let Json(connection) = with_usage(&state, connection).await?;
    Ok(Json(AgentKeyIssued {
        connection,
        key: key.value,
    }))
}

/// Revoke a connection
///
/// Its key stops working and its waiting approvals go. Its activity stays, under its name. The
/// scheduled messages it created or changed are deleted with `delete_schedules=true`, else kept as the
/// owner's own.
#[utoipa::path(delete, path = "/{id}", params(("id" = Uuid, Path), RevokeQuery), responses(
    (status = OK, body = AgentRevoked),
    (status = BAD_REQUEST, body = ErrorBody),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = NOT_FOUND, body = ErrorBody),
    (status = TOO_MANY_REQUESTS, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn revoke_agent(
    _: Enabled,
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    ValidatedQuery(query): ValidatedQuery<RevokeQuery>,
) -> Result<Json<AgentRevoked>, AppError> {
    let revoked =
        agent_connections::delete(&state.pool, id, session.user.id, query.delete_schedules)
            .await?
            .ok_or(AppError::NotFound)?;
    Ok(Json(revoked.into()))
}

/// What agents asked to do, waiting first, then newest first, each with the whole text
#[utoipa::path(get, path = "/approvals", params(ApprovalsQuery), responses(
    (status = OK, body = AgentApprovalPage),
    (status = BAD_REQUEST, body = ErrorBody),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = NOT_FOUND, description = "Agent access is off on this server", body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn list_agent_approvals(
    _: Enabled,
    State(state): State<AppState>,
    session: Session,
    ValidatedQuery(page): ValidatedQuery<ApprovalsQuery>,
) -> Result<Json<AgentApprovalPage>, AppError> {
    let user_id = session.user.id;
    let (items, total) = tokio::try_join!(
        agent_approvals::list(&state.pool, user_id, None, page.limit, page.offset),
        agent_approvals::count(&state.pool, user_id),
    )?;
    Ok(Json(AgentApprovalPage {
        items: items
            .into_iter()
            .map(|listed| {
                let request = opened(&state, &listed.approval);
                AgentApproval::new(listed, request)
            })
            .collect(),
        total,
        limit: page.limit,
        offset: page.offset,
    }))
}

/// One approval, waiting or answered
#[utoipa::path(get, path = "/approvals/{id}", params(("id" = Uuid, Path)), responses(
    (status = OK, body = AgentApproval),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = NOT_FOUND, description = "Agent access is off, or there's no such approval", body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn get_agent_approval(
    _: Enabled,
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<Json<AgentApproval>, AppError> {
    answered(&state, id, session.user.id).await
}

/// Approve what an agent asked, optionally with your own text
///
/// The request is checked as if it were made now, as the scheduling form checks it; one that
/// can't go ahead stays waiting, so it can still be rejected. A send joins the queue; a schedule
/// is created or changed.
#[utoipa::path(post, path = "/approvals/{id}/approve", params(("id" = Uuid, Path)), request_body = ApproveRequest, responses(
    (status = OK, body = AgentApproval),
    (status = BAD_REQUEST, body = ErrorBody),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = NOT_FOUND, body = ErrorBody),
    (status = CONFLICT, description = "It's no longer waiting; `details.status` says what it is now", body = ErrorBody),
    (status = UNPROCESSABLE_ENTITY, description = "It can't go ahead now, one sentence each under `details.request`", body = ErrorBody),
    (status = TOO_MANY_REQUESTS, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn approve_agent_request(
    _: Enabled,
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    ValidatedJson(input): ValidatedJson<ApproveRequest>,
) -> Result<Json<AgentApproval>, AppError> {
    let user_id = session.user.id;
    // So a payload that doesn't open reads as a refusal the owner can act on, not a server error
    let approval = agent_approvals::find(&state.pool, id, user_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if approval.status == ApprovalStatus::Waiting && opened(&state, &approval).is_none() {
        return Err(AppError::Unprocessable {
            field: "request",
            problems: vec![UNREADABLE.to_owned()],
        });
    }
    review::approve(&state, id, user_id, input.body)
        .await
        .map_err(refused)?;
    answered(&state, id, user_id).await
}

/// Reject what an agent asked
#[utoipa::path(post, path = "/approvals/{id}/reject", params(("id" = Uuid, Path)), responses(
    (status = OK, body = AgentApproval),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = NOT_FOUND, body = ErrorBody),
    (status = CONFLICT, description = "It's no longer waiting; `details.status` says what it is now", body = ErrorBody),
    (status = TOO_MANY_REQUESTS, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn reject_agent_request(
    _: Enabled,
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<Json<AgentApproval>, AppError> {
    let user_id = session.user.id;
    review::reject(&state.pool, id, user_id)
        .await
        .map_err(refused)?;
    answered(&state, id, user_id).await
}

/// What agents did, newest first, for one connection when `connection_id` is given
#[utoipa::path(get, path = "/activity", params(ActivityQuery), responses(
    (status = OK, body = AgentActivityPage),
    (status = BAD_REQUEST, body = ErrorBody),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = NOT_FOUND, description = "Agent access is off, or there's no such connection", body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn list_agent_activity(
    _: Enabled,
    State(state): State<AppState>,
    session: Session,
    ValidatedQuery(query): ValidatedQuery<ActivityQuery>,
) -> Result<Json<AgentActivityPage>, AppError> {
    let user_id = session.user.id;
    if let Some(id) = query.connection_id {
        agent_connections::find(&state.pool, id, user_id)
            .await?
            .ok_or(AppError::NotFound)?;
    }
    let (items, total) = tokio::try_join!(
        agent_activity::page(
            &state.pool,
            user_id,
            query.connection_id,
            query.limit,
            query.offset
        ),
        agent_activity::count(&state.pool, user_id, query.connection_id),
    )?;
    Ok(Json(AgentActivityPage {
        items: items.into_iter().map(Into::into).collect(),
        total,
        limit: query.limit,
        offset: query.offset,
    }))
}

fn checked(grant: Grant) -> Result<Grant, AppError> {
    grant.check().map_err(|problems| AppError::Unprocessable {
        field: "grant",
        problems,
    })
}

async fn with_usage(
    state: &AppState,
    connection: Connection,
) -> Result<Json<AgentConnection>, AppError> {
    let usage = agent_connections::usage(&state.pool, connection.id).await?;
    Ok(Json(AgentConnection::new(connection, usage)))
}

/// The request inside an approval, or `None` when it doesn't open
fn opened(state: &AppState, approval: &agent_approvals::Approval) -> Option<Requested> {
    review::open(&state.cipher, approval)
        .inspect_err(|error| {
            tracing::error!(approval_id = %approval.id, "an agent approval didn't open: {error:#}");
        })
        .ok()
        .map(Requested::new)
}

async fn answered(
    state: &AppState,
    id: Uuid,
    user_id: Uuid,
) -> Result<Json<AgentApproval>, AppError> {
    let listed = agent_approvals::list(&state.pool, user_id, Some(id), 1, 0)
        .await?
        .pop()
        .ok_or(AppError::NotFound)?;
    let request = opened(state, &listed.approval);
    Ok(Json(AgentApproval::new(listed, request)))
}

fn refused(error: ReviewError) -> AppError {
    match error {
        ReviewError::NotFound => AppError::NotFound,
        ReviewError::NotWaiting(status) => {
            let (message, status) = match status {
                ApprovalStatus::Approved => ("This request was approved already.", "approved"),
                ApprovalStatus::Rejected => ("This request was rejected already.", "rejected"),
                ApprovalStatus::Expired => (
                    "This request expired, since nobody answered it within 24 hours. The agent \
                     can ask again.",
                    "expired",
                ),
                ApprovalStatus::Waiting => ("This request is still waiting.", "waiting"),
            };
            AppError::Decided { message, status }
        }
        ReviewError::CantApprove(problems) => AppError::Unprocessable {
            field: "request",
            problems,
        },
        ReviewError::Internal(error) => AppError::Internal(error),
    }
}

const _: () = assert!(MAX_PER_USER == 20, "AT_CAP names the cap");
