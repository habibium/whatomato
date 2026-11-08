use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::Utc;
use uuid::Uuid;
use validator::{ValidationError, ValidationErrors};

use super::dto::{
    MessageInput, NextRuns, NextRunsQuery, Preview, PreviewQuery, ScheduledMessage, SendNowRequest,
    SendNowResponse, ToggleRequest, normalized_recipient,
};
use crate::{
    auth::session::Session,
    db::{
        deliveries::{self, NewDelivery},
        message_templates, scheduled_messages,
    },
    error::{AppError, ErrorBody},
    extract::{ValidatedJson, ValidatedQuery},
    scheduler::{RecipientType, ScheduleKind, recurrence::Recurrence, variables},
    state::AppState,
    whatsapp::supervisor::SendFailure,
};

/// Enough to see a sun schedule move from day to day
const NEXT_RUNS: usize = 3;

/// The account's scheduled messages, oldest first
#[utoipa::path(get, path = "/", responses(
    (status = OK, body = Vec<ScheduledMessage>),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn list_messages(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Vec<ScheduledMessage>>, AppError> {
    let messages = scheduled_messages::list(&state.pool, session.user.id).await?;
    Ok(Json(messages.into_iter().map(Into::into).collect()))
}

/// Schedule a message
#[utoipa::path(post, path = "/", request_body = MessageInput, responses(
    (status = CREATED, body = ScheduledMessage),
    (status = BAD_REQUEST, body = ErrorBody),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = TOO_MANY_REQUESTS, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn create_message(
    State(state): State<AppState>,
    session: Session,
    ValidatedJson(input): ValidatedJson<MessageInput>,
) -> Result<(StatusCode, Json<ScheduledMessage>), AppError> {
    let user_id = session.user.id;
    check_template(&state, &input, user_id).await?;
    let recipient = normalized_recipient(input.recipient_type, &input.recipient);
    let message =
        scheduled_messages::create(&state.pool, user_id, input.fields(&recipient)).await?;
    Ok((StatusCode::CREATED, Json(message.into())))
}

/// Send a message straight away
///
/// It joins the queue, so it may wait behind others, and the outcome shows under deliveries.
#[utoipa::path(post, path = "/send-now", request_body = SendNowRequest, responses(
    (status = ACCEPTED, body = SendNowResponse),
    (status = BAD_REQUEST, body = ErrorBody),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = CONFLICT, description = "WhatsApp isn't linked, is paused or has stopped", body = ErrorBody),
    (status = TOO_MANY_REQUESTS, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn send_now(
    State(state): State<AppState>,
    session: Session,
    ValidatedJson(input): ValidatedJson<SendNowRequest>,
) -> Result<(StatusCode, Json<SendNowResponse>), AppError> {
    let user_id = session.user.id;
    if let Some(failure) = SendFailure::for_status(&state.whatsapp.status(user_id)) {
        return Err(AppError::Conflict(failure.unavailable()));
    }
    let recipient = normalized_recipient(input.recipient_type, &input.recipient);
    let recipient_name = input.recipient_name.as_deref().map(str::trim);
    let to = variables::To {
        recipient_type: input.recipient_type,
        recipient: &recipient,
        recipient_name,
    };
    let due = variables::Due {
        at: Utc::now(),
        timezone: input.timezone.as_deref().unwrap_or("UTC"),
        place: None,
    };
    let body = variables::fill_in(
        &state.pool,
        &state.cipher,
        user_id,
        &to,
        input.body.trim(),
        &due,
    )
    .await;
    let delivery_id = deliveries::create(
        &state.pool,
        NewDelivery {
            user_id,
            scheduled_message_id: None,
            recipient_type: input.recipient_type,
            recipient: &recipient,
            recipient_name,
            body: &body,
        },
    )
    .await?;
    Ok((StatusCode::ACCEPTED, Json(SendNowResponse { delivery_id })))
}

/// When a schedule would go out, to check it before it's saved
#[utoipa::path(get, path = "/next-runs", params(NextRunsQuery), responses(
    (status = OK, body = NextRuns),
    (status = BAD_REQUEST, body = ErrorBody),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn next_runs(
    _session: Session,
    ValidatedQuery(query): ValidatedQuery<NextRunsQuery>,
) -> Json<NextRuns> {
    let recurrence = query.recurrence.normalized();
    Json(NextRuns {
        runs: recurrence.next_runs(&query.timezone, Utc::now(), NEXT_RUNS),
        city: recurrence.city(&query.timezone),
    })
}

/// A message as it would go out, filled in by the same code that fills it in to send
#[utoipa::path(get, path = "/preview", params(PreviewQuery), responses(
    (status = OK, body = Preview),
    (status = BAD_REQUEST, body = ErrorBody),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn preview(
    State(state): State<AppState>,
    session: Session,
    ValidatedQuery(query): ValidatedQuery<PreviewQuery>,
) -> Json<Preview> {
    let recipient_type = query.recipient_type.unwrap_or(RecipientType::Contact);
    let recipient = normalized_recipient(recipient_type, query.recipient.as_deref().unwrap_or(""));
    let to = variables::To {
        recipient_type,
        recipient: &recipient,
        recipient_name: query.recipient_name.as_deref().map(str::trim),
    };
    let now = Utc::now();
    let recurrence = query.recurrence.as_ref().map(Recurrence::normalized);
    let due = variables::Due {
        at: query
            .run_at
            .or_else(|| recurrence.as_ref()?.next_after(&query.timezone, now))
            .unwrap_or(now),
        timezone: &query.timezone,
        place: recurrence.as_ref().and_then(Recurrence::place),
    };
    let values = variables::values(&state.pool, &state.cipher, session.user.id, &to, &due).await;
    let body = query.body.trim();
    Json(Preview {
        text: variables::render(body, &values),
        problem: variables::check(body)
            .err()
            .map(|problem| problem.message()),
        values: variables::all(&values).into_iter().collect(),
        empty: variables::empty(body, &values),
    })
}

/// One scheduled message
#[utoipa::path(get, path = "/{id}", params(("id" = Uuid, Path)), responses(
    (status = OK, body = ScheduledMessage),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = NOT_FOUND, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn get_message(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<Json<ScheduledMessage>, AppError> {
    let message = scheduled_messages::find(&state.pool, id, session.user.id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(message.into()))
}

/// Replace a scheduled message
#[utoipa::path(put, path = "/{id}", params(("id" = Uuid, Path)), request_body = MessageInput, responses(
    (status = OK, body = ScheduledMessage),
    (status = BAD_REQUEST, body = ErrorBody),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = NOT_FOUND, body = ErrorBody),
    (status = TOO_MANY_REQUESTS, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn update_message(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    ValidatedJson(input): ValidatedJson<MessageInput>,
) -> Result<Json<ScheduledMessage>, AppError> {
    let user_id = session.user.id;
    check_template(&state, &input, user_id).await?;
    let recipient = normalized_recipient(input.recipient_type, &input.recipient);
    let message = scheduled_messages::update(&state.pool, id, user_id, input.fields(&recipient))
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(message.into()))
}

/// Switch a scheduled message on or off
#[utoipa::path(patch, path = "/{id}", params(("id" = Uuid, Path)), request_body = ToggleRequest, responses(
    (status = OK, body = ScheduledMessage),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = NOT_FOUND, body = ErrorBody),
    (status = CONFLICT, description = "A one-time message's send time has passed", body = ErrorBody),
    (status = TOO_MANY_REQUESTS, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn toggle_message(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    ValidatedJson(input): ValidatedJson<ToggleRequest>,
) -> Result<Json<ScheduledMessage>, AppError> {
    let user_id = session.user.id;
    let message = scheduled_messages::find(&state.pool, id, user_id)
        .await?
        .ok_or(AppError::NotFound)?;

    let now = Utc::now();
    let next_run_at = match (input.enabled, message.schedule_kind) {
        (false, _) => None,
        // Switching this back on would send straight away, which nobody means by a switch
        (true, ScheduleKind::Once) if message.run_at.is_none_or(|at| at <= now) => {
            return Err(AppError::Conflict(
                "This message's send time has passed; edit it to pick a new one",
            ));
        }
        (true, ScheduleKind::Once) => message.run_at,
        (true, ScheduleKind::Recurring) => message
            .recurrence
            .as_ref()
            .and_then(|recurrence| recurrence.next_after(&message.timezone, now)),
    };

    let message =
        scheduled_messages::set_enabled(&state.pool, id, user_id, input.enabled, next_run_at)
            .await?
            .ok_or(AppError::NotFound)?;
    Ok(Json(message.into()))
}

/// Delete a scheduled message. Its deliveries stay in the history.
#[utoipa::path(delete, path = "/{id}", params(("id" = Uuid, Path)), responses(
    (status = NO_CONTENT),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = NOT_FOUND, body = ErrorBody),
    (status = TOO_MANY_REQUESTS, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn delete_message(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    if scheduled_messages::delete(&state.pool, id, session.user.id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound)
    }
}

/// A template id must be the user's own, or the foreign key would confirm other users' ids
async fn check_template(
    state: &AppState,
    input: &MessageInput,
    user_id: Uuid,
) -> Result<(), AppError> {
    let Some(template_id) = input.template_id else {
        return Ok(());
    };
    if message_templates::find(&state.pool, template_id, user_id)
        .await?
        .is_some()
    {
        return Ok(());
    }
    let mut errors = ValidationErrors::new();
    errors.add(
        "template_id",
        ValidationError::new("invalid").with_message("isn't one of your templates".into()),
    );
    Err(errors.into())
}
