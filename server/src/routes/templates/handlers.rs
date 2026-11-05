use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use uuid::Uuid;

use super::dto::{MessageTemplate, TemplateInput};
use crate::{
    auth::session::Session,
    db::message_templates,
    error::{AppError, ErrorBody},
    extract::ValidatedJson,
    state::AppState,
};

/// The account's templates, by name
#[utoipa::path(get, path = "/", responses(
    (status = OK, body = Vec<MessageTemplate>),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn list_templates(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Vec<MessageTemplate>>, AppError> {
    let templates = message_templates::list(&state.pool, session.user.id).await?;
    Ok(Json(templates.into_iter().map(Into::into).collect()))
}

/// Save a template
#[utoipa::path(post, path = "/", request_body = TemplateInput, responses(
    (status = CREATED, body = MessageTemplate),
    (status = BAD_REQUEST, body = ErrorBody),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = TOO_MANY_REQUESTS, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn create_template(
    State(state): State<AppState>,
    session: Session,
    ValidatedJson(input): ValidatedJson<TemplateInput>,
) -> Result<(StatusCode, Json<MessageTemplate>), AppError> {
    let template = message_templates::create(
        &state.pool,
        session.user.id,
        input.name.trim(),
        input.body.trim(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(template.into())))
}

/// One template
#[utoipa::path(get, path = "/{id}", params(("id" = Uuid, Path)), responses(
    (status = OK, body = MessageTemplate),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = NOT_FOUND, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn get_template(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<Json<MessageTemplate>, AppError> {
    let template = message_templates::find(&state.pool, id, session.user.id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(template.into()))
}

/// Change a template. Messages already made from it keep their text.
#[utoipa::path(put, path = "/{id}", params(("id" = Uuid, Path)), request_body = TemplateInput, responses(
    (status = OK, body = MessageTemplate),
    (status = BAD_REQUEST, body = ErrorBody),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = NOT_FOUND, body = ErrorBody),
    (status = TOO_MANY_REQUESTS, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn update_template(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    ValidatedJson(input): ValidatedJson<TemplateInput>,
) -> Result<Json<MessageTemplate>, AppError> {
    let template = message_templates::update(
        &state.pool,
        id,
        session.user.id,
        input.name.trim(),
        input.body.trim(),
    )
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(template.into()))
}

/// Delete a template. Messages made from it keep their text.
#[utoipa::path(delete, path = "/{id}", params(("id" = Uuid, Path)), responses(
    (status = NO_CONTENT),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = NOT_FOUND, body = ErrorBody),
    (status = TOO_MANY_REQUESTS, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
pub async fn delete_template(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    if message_templates::delete(&state.pool, id, session.user.id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound)
    }
}
