use anyhow::anyhow;
use axum::{Json, extract::State, http::StatusCode};

use super::dto::{
    GroupResponse, LinkRequest, PasswordConfirmation, Unlinked, WhatsAppStatus, digits,
};
use crate::{
    auth::{reauth, session::Session},
    error::{AppError, ErrorBody},
    extract::ValidatedJson,
    state::AppState,
    whatsapp::{status::Method, supervisor::CommandError},
};

/// The account's WhatsApp status
#[utoipa::path(get, path = "/status", responses(
    (status = OK, body = WhatsAppStatus),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn get_status(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<WhatsAppStatus>, AppError> {
    let status = state.whatsapp.status(session.user.id);
    Ok(Json(WhatsAppStatus::new(status)?))
}

/// Start linking WhatsApp
///
/// Checks the password, then starts linking: by QR code, or with `phone_number` by a code typed on
/// that phone. The status then carries the code. Linking stops after 5 minutes, or once the QR codes
/// run out after about 3.
#[utoipa::path(post, path = "/link", request_body = LinkRequest, responses(
    (status = NO_CONTENT),
    (status = BAD_REQUEST, description = "The password is incorrect", body = ErrorBody),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = CONFLICT, description = "WhatsApp is already linked or linking", body = ErrorBody),
    (status = TOO_MANY_REQUESTS, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn link(
    State(state): State<AppState>,
    session: Session,
    ValidatedJson(payload): ValidatedJson<LinkRequest>,
) -> Result<StatusCode, AppError> {
    let user = session.user;
    reauth::confirm_password(&state, user.id, payload.password).await?;
    state.rate_limits.whatsapp_link(user.id)?;

    let method = payload
        .phone_number
        .map_or(Method::Qr, |number| Method::PhoneNumber(digits(&number)));
    state
        .whatsapp
        .handle(user.id)
        .link(method)
        .await
        .map_err(refused_with("WhatsApp is already linked or linking"))?;

    Ok(StatusCode::NO_CONTENT)
}

/// Stop linking WhatsApp
#[utoipa::path(post, path = "/link/cancel", responses(
    (status = NO_CONTENT),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = CONFLICT, description = "WhatsApp isn't being linked", body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn cancel_link(
    State(state): State<AppState>,
    session: Session,
) -> Result<StatusCode, AppError> {
    state
        .whatsapp
        .handle(session.user.id)
        .cancel()
        .await
        .map_err(refused_with("WhatsApp isn't being linked"))?;

    Ok(StatusCode::NO_CONTENT)
}

/// Disconnect WhatsApp and keep the link
#[utoipa::path(post, path = "/pause", responses(
    (status = NO_CONTENT),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = CONFLICT, description = "WhatsApp isn't connected", body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn pause(
    State(state): State<AppState>,
    session: Session,
) -> Result<StatusCode, AppError> {
    state
        .whatsapp
        .handle(session.user.id)
        .pause()
        .await
        .map_err(refused_with("WhatsApp isn't connected"))?;

    Ok(StatusCode::NO_CONTENT)
}

/// Reconnect WhatsApp after a pause or a stop
#[utoipa::path(post, path = "/resume", responses(
    (status = NO_CONTENT),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = CONFLICT, description = "WhatsApp isn't paused or stopped", body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn resume(
    State(state): State<AppState>,
    session: Session,
) -> Result<StatusCode, AppError> {
    state
        .whatsapp
        .handle(session.user.id)
        .resume()
        .await
        .map_err(refused_with("WhatsApp isn't paused or stopped"))?;

    Ok(StatusCode::NO_CONTENT)
}

/// Unlink WhatsApp
///
/// Checks the password, asks WhatsApp to remove the device from the phone, and deletes its keys.
/// When WhatsApp can't be reached, the device stays under Linked devices on the phone until it's
/// removed there.
#[utoipa::path(post, path = "/unlink", request_body = PasswordConfirmation, responses(
    (status = OK, body = Unlinked),
    (status = BAD_REQUEST, description = "The password is incorrect", body = ErrorBody),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = CONFLICT, description = "WhatsApp isn't linked", body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn unlink(
    State(state): State<AppState>,
    session: Session,
    ValidatedJson(payload): ValidatedJson<PasswordConfirmation>,
) -> Result<Json<Unlinked>, AppError> {
    let user = session.user;
    reauth::confirm_password(&state, user.id, payload.password).await?;

    let removed_from_phone = state
        .whatsapp
        .handle(user.id)
        .unlink()
        .await
        .map_err(refused_with("WhatsApp isn't linked"))?;

    Ok(Json(Unlinked { removed_from_phone }))
}

/// The groups the account is in, to choose where messages go
#[utoipa::path(get, path = "/groups", responses(
    (status = OK, body = Vec<GroupResponse>),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = CONFLICT, description = "WhatsApp isn't connected", body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn groups(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Vec<GroupResponse>>, AppError> {
    let groups = state
        .whatsapp
        .handle(session.user.id)
        .groups()
        .await
        .map_err(refused_with("WhatsApp isn't connected"))?;

    Ok(Json(groups.into_iter().map(GroupResponse::from).collect()))
}

/// Send a test message to the account's own number, to check the link works
#[utoipa::path(post, path = "/test-message", responses(
    (status = NO_CONTENT),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = CONFLICT, description = "WhatsApp isn't connected", body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn send_test_message(
    State(state): State<AppState>,
    session: Session,
) -> Result<StatusCode, AppError> {
    state
        .whatsapp
        .handle(session.user.id)
        .send_test_message()
        .await
        .map_err(refused_with("WhatsApp isn't connected"))?;

    Ok(StatusCode::NO_CONTENT)
}

/// Maps a supervisor's answer onto a response, with `message` for a command the status refuses
fn refused_with(message: &'static str) -> impl FnOnce(CommandError) -> AppError {
    move |error| match error {
        CommandError::Refused | CommandError::NotConnected => AppError::Conflict(message),
        CommandError::Gone => AppError::Internal(anyhow!("WhatsApp is shutting down")),
        CommandError::Failed(error) => AppError::Internal(error),
    }
}
