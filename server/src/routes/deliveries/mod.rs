use axum::{Json, extract::State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;
use validator::Validate;

use crate::{
    auth::session::Session,
    db::deliveries::{self, Delivery as Row},
    error::{AppError, ErrorBody},
    extract::ValidatedQuery,
    scheduler::{DeliveryStatus, RecipientType},
    state::AppState,
};

#[cfg(test)]
mod tests;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(list_deliveries))
}

const fn default_limit() -> i64 {
    50
}

#[derive(Deserialize, IntoParams, Validate)]
#[into_params(parameter_in = Query)]
struct Page {
    /// 1 to 100, 50 by default
    #[serde(default = "default_limit")]
    #[validate(range(min = 1, max = 100, message = "must be between 1 and 100"))]
    limit: i64,
    #[serde(default)]
    #[validate(range(min = 0, message = "can't be negative"))]
    offset: i64,
    /// Only deliveries to this number's digits or this group's id
    #[validate(length(min = 1, max = 128, message = "must be between 1 and 128 characters"))]
    recipient: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
enum DeliveryState {
    /// Waiting to go out, possibly for another try
    Pending,
    /// WhatsApp acknowledged it, which doesn't prove it reached the recipient's phone
    Sent,
    Failed,
}

#[derive(Serialize, ToSchema)]
struct DeliveryResponse {
    id: Uuid,
    /// Empty for a message sent straight away, or once its schedule is deleted
    #[schema(required = true)]
    scheduled_message_id: Option<Uuid>,
    recipient_type: RecipientType,
    recipient: String,
    #[schema(required = true)]
    recipient_name: Option<String>,
    body: String,
    status: DeliveryState,
    /// Why it failed, or why the last try did while it's pending
    #[schema(required = true)]
    error: Option<String>,
    created_at: DateTime<Utc>,
    #[schema(required = true)]
    sent_at: Option<DateTime<Utc>>,
}

impl From<Row> for DeliveryResponse {
    fn from(row: Row) -> Self {
        Self {
            id: row.id,
            scheduled_message_id: row.scheduled_message_id,
            recipient_type: row.recipient_type,
            recipient: row.recipient,
            recipient_name: row.recipient_name,
            body: row.body,
            status: match row.status {
                DeliveryStatus::Pending | DeliveryStatus::Sending => DeliveryState::Pending,
                DeliveryStatus::Sent => DeliveryState::Sent,
                DeliveryStatus::Failed => DeliveryState::Failed,
            },
            error: row.error,
            created_at: row.created_at,
            sent_at: row.sent_at,
        }
    }
}

#[derive(Serialize, ToSchema)]
struct DeliveryPage {
    items: Vec<DeliveryResponse>,
    total: i64,
    limit: i64,
    offset: i64,
}

/// Every message the account has sent or tried to, newest first
#[utoipa::path(get, path = "/", params(Page), responses(
    (status = OK, body = DeliveryPage),
    (status = BAD_REQUEST, body = ErrorBody),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
async fn list_deliveries(
    State(state): State<AppState>,
    session: Session,
    ValidatedQuery(page): ValidatedQuery<Page>,
) -> Result<Json<DeliveryPage>, AppError> {
    let user_id = session.user.id;
    let recipient = page.recipient.as_deref();
    let (items, total) = tokio::try_join!(
        deliveries::page(&state.pool, user_id, recipient, page.limit, page.offset),
        deliveries::count(&state.pool, user_id, recipient),
    )?;
    Ok(Json(DeliveryPage {
        items: items.into_iter().map(Into::into).collect(),
        total,
        limit: page.limit,
        offset: page.offset,
    }))
}
