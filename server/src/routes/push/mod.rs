//! Notifications in the owner's browsers: what a browser subscribes with, and its subscription

use axum::{Json, extract::State, http::StatusCode};
use base64::prelude::{BASE64_URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;
use validator::{Validate, ValidationErrors};

use crate::{
    auth::session::Session,
    db::push_subscriptions::{self, NewSubscription, Subscription},
    error::{AppError, ErrorBody},
    extract::ValidatedJson,
    push::{
        Notification, check_endpoint,
        message::{AUTH_LEN, PUBLIC_KEY_LEN},
    },
    scheduler::draft::add,
    state::AppState,
};

#[cfg(test)]
mod tests;

const LABEL_MAX: usize = 60;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(push_status))
        .routes(routes!(subscribe_to_push))
        .routes(routes!(unsubscribe_from_push))
        .routes(routes!(test_push))
}

#[derive(Serialize, ToSchema)]
struct PushStatus {
    /// What a browser subscribes with: `applicationServerKey`, in base64url
    public_key: String,
    /// The account's browsers with notifications on, oldest first
    subscriptions: Vec<PushSubscription>,
}

/// A browser with notifications on
#[derive(Serialize, ToSchema)]
struct PushSubscription {
    id: Uuid,
    /// The browser's address at its push service, by which a browser recognises itself here
    endpoint: String,
    /// What the browser called itself, such as "Chrome on Linux"
    label: String,
    created_at: DateTime<Utc>,
}

impl From<Subscription> for PushSubscription {
    fn from(subscription: Subscription) -> Self {
        Self {
            id: subscription.id,
            endpoint: subscription.endpoint,
            label: subscription.label,
            created_at: subscription.created_at,
        }
    }
}

/// What `PushSubscription.toJSON()` gives in the browser, and a name for it
#[derive(Deserialize, ToSchema)]
struct Subscribe {
    endpoint: String,
    /// The browser's public key, in base64url
    p256dh: String,
    /// The browser's authentication secret, in base64url
    auth: String,
    /// Up to 60 characters, such as "Chrome on Linux"
    label: String,
}

impl Subscribe {
    fn keys(&self) -> Option<(Vec<u8>, Vec<u8>)> {
        let decode = |value: &str| {
            BASE64_URL_SAFE_NO_PAD
                .decode(value.trim_end_matches('='))
                .ok()
        };
        let p256dh = decode(&self.p256dh).filter(|key| key.len() == PUBLIC_KEY_LEN)?;
        let auth = decode(&self.auth).filter(|secret| secret.len() == AUTH_LEN)?;
        Some((p256dh, auth))
    }
}

impl Validate for Subscribe {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();
        if let Err(unknown) = check_endpoint(&self.endpoint) {
            add(&mut errors, "endpoint", unknown);
        }
        if self.keys().is_none() {
            add(&mut errors, "p256dh", "the keys aren't a browser's");
        }
        let label = self.label.trim().chars().count();
        if !(1..=LABEL_MAX).contains(&label) {
            add(&mut errors, "label", "must be between 1 and 60 characters");
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

#[derive(Deserialize, ToSchema, Validate)]
struct Unsubscribe {
    #[validate(length(min = 1, max = 2048, message = "must be between 1 and 2048 characters"))]
    endpoint: String,
}

#[derive(Serialize, ToSchema)]
struct PushTested {
    /// How many browsers the notification was handed over for
    sent: usize,
}

/// The key browsers subscribe with, and the account's browsers that have notifications on
#[utoipa::path(get, path = "/", responses(
    (status = OK, body = PushStatus),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
async fn push_status(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<PushStatus>, AppError> {
    let subscriptions = push_subscriptions::list(&state.pool, session.user.id).await?;
    Ok(Json(PushStatus {
        public_key: state.push.public_key().to_owned(),
        subscriptions: subscriptions.into_iter().map(Into::into).collect(),
    }))
}

/// Turn notifications on for a browser
///
/// Sending the same subscription again keeps it fresh. An account keeps its 10 newest; a browser
/// notifies for whichever account turned notifications on in it last.
#[utoipa::path(post, path = "/subscriptions", request_body = Subscribe, responses(
    (status = OK, body = PushSubscription),
    (status = BAD_REQUEST, description = "The subscription isn't one a known browser makes", body = ErrorBody),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = TOO_MANY_REQUESTS, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
async fn subscribe_to_push(
    State(state): State<AppState>,
    session: Session,
    ValidatedJson(input): ValidatedJson<Subscribe>,
) -> Result<Json<PushSubscription>, AppError> {
    // `validate` let no other keys through
    let (p256dh, auth) = input.keys().ok_or(AppError::NotFound)?;
    let subscription = push_subscriptions::save(
        &state.pool,
        NewSubscription {
            user_id: session.user.id,
            endpoint: &input.endpoint,
            p256dh: &p256dh,
            auth: &auth,
            label: input.label.trim(),
        },
    )
    .await?;
    Ok(Json(subscription.into()))
}

/// Turn notifications off for a browser
#[utoipa::path(post, path = "/unsubscribe", request_body = Unsubscribe, responses(
    (status = NO_CONTENT, description = "It no longer gets notifications, whether or not it did"),
    (status = BAD_REQUEST, body = ErrorBody),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = TOO_MANY_REQUESTS, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
async fn unsubscribe_from_push(
    State(state): State<AppState>,
    session: Session,
    ValidatedJson(input): ValidatedJson<Unsubscribe>,
) -> Result<StatusCode, AppError> {
    push_subscriptions::delete(&state.pool, session.user.id, &input.endpoint).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Send a test notification to every browser of the account that has them on
#[utoipa::path(post, path = "/test", responses(
    (status = OK, body = PushTested),
    (status = UNAUTHORIZED, body = ErrorBody),
    (status = TOO_MANY_REQUESTS, body = ErrorBody),
    (status = INTERNAL_SERVER_ERROR, body = ErrorBody),
))]
async fn test_push(State(state): State<AppState>, session: Session) -> Json<PushTested> {
    let sent = state
        .push
        .notify_now(
            &state.pool,
            &state.app_url,
            session.user.id,
            &Notification::test(),
        )
        .await;
    Json(PushTested { sent })
}
