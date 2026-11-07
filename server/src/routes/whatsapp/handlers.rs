use anyhow::anyhow;
use axum::{
    Json,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use whatsapp_rust::prelude::{Jid, Server};

use super::dto::{
    ContactBook, ContactResponse, GroupResponse, LinkRequest, PasswordConfirmation, Unlinked,
    WhatsAppStatus, digits,
};
use crate::{
    auth::{reauth, session::Session},
    db::whatsapp_contacts,
    error::{AppError, ErrorBody},
    extract::ValidatedJson,
    state::AppState,
    whatsapp::{contacts, status::Method, supervisor::CommandError},
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

/// The contacts saved on the phone, to choose where messages go by name
///
/// The phone shares its address book with linked devices; this is the copy kept from it, so it
/// answers while WhatsApp is paused or offline too. Unlinking deletes it.
#[utoipa::path(get, path = "/contacts", responses(
    (status = OK, body = ContactBook),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn contacts(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<ContactBook>, AppError> {
    let Some(book) = whatsapp_contacts::book(&state.pool, session.user.id).await? else {
        return Ok(Json(ContactBook {
            imported: false,
            contacts: Vec::new(),
        }));
    };
    let account_id = book.account_id;
    let mut list: Vec<_> = book
        .contacts
        .into_iter()
        .filter_map(|row| {
            match contacts::open_names(&state.cipher, account_id, &row.jid, &row.names) {
                Ok(names) => ContactResponse::new(row.jid, row.phone_number, names),
                Err(error) => {
                    tracing::error!(%account_id, "a WhatsApp contact didn't open: {error:#}");
                    None
                }
            }
        })
        .collect();
    list.sort_by_cached_key(|contact| contact.name.to_lowercase());

    Ok(Json(ContactBook {
        imported: book.imported,
        contacts: list,
    }))
}

/// A saved contact's or a group's profile picture, a small JPEG
///
/// Looked up on WhatsApp while it's connected and kept in memory for a few hours. Not found when
/// there's none, when the contact keeps it private, or when it can't be looked up right now.
#[utoipa::path(get, path = "/pictures/{id}", params(("id" = String, Path, description = "The contact's or group's id")), responses(
    (status = OK, content_type = "image/jpeg", body = Vec<u8>),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = NOT_FOUND, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn picture(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let user_id = session.user.id;
    let jid: Jid = id.parse().map_err(|_| AppError::NotFound)?;
    // Only the people the user saved, so this can't look up any number on WhatsApp
    let allowed = jid.server == Server::Group
        || whatsapp_contacts::is_saved(&state.pool, user_id, &id).await?;
    if !allowed {
        return Err(AppError::NotFound);
    }
    let image = state
        .whatsapp
        .handle(user_id)
        .picture(jid)
        .await
        .ok_or(AppError::NotFound)?;

    Ok((
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::CACHE_CONTROL, "private, max-age=3600"),
        ],
        image.to_vec(),
    )
        .into_response())
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
