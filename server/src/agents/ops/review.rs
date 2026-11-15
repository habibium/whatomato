//! What an approval holds, and the owner's answer to it. The request is sealed with [`Cipher`]
//! and bound to the approval's id, so the database alone doesn't give a message away and a payload
//! copied onto another approval doesn't open.

use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, PgPool};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::ValidationErrors;

use super::{sentences, whatsapp_state};
use crate::{
    agents::{ApprovalKind, ApprovalStatus},
    crypto::Cipher,
    db::{
        agent_approvals::{self, Approval, Became},
        deliveries::{self, NewDelivery},
        scheduled_messages,
    },
    scheduler::{
        RecipientType, ScheduleKind,
        draft::{self, Draft},
        recurrence::Recurrence,
        variables,
    },
    state::AppState,
};

const PAYLOAD_VERSION: u32 = 1;
/// Runs shown for a schedule waiting for approval
const NEXT_RUNS: usize = 3;

/// The sealed form, versioned so it can change while older approvals still open
#[derive(Serialize, Deserialize)]
struct Payload {
    version: u32,
    request: Request,
}

/// What an agent asked for, as it was when it asked. Recipients are stored normalized.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Request {
    Send(SendRequest),
    ScheduleCreate(ScheduleRequest),
    ScheduleUpdate {
        scheduled_message_id: Uuid,
        schedule: ScheduleRequest,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendRequest {
    pub recipient_type: RecipientType,
    pub recipient: String,
    pub recipient_name: Option<String>,
    /// As written, variables not yet filled in
    pub body: String,
    pub timezone: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScheduleRequest {
    pub recipient_type: RecipientType,
    pub recipient: String,
    pub recipient_name: Option<String>,
    pub body: String,
    pub schedule_kind: ScheduleKind,
    pub run_at: Option<DateTime<Utc>>,
    pub recurrence: Option<Recurrence>,
    pub timezone: String,
    pub template_id: Option<Uuid>,
    pub enabled: bool,
}

impl ScheduleRequest {
    pub(super) fn draft(&self) -> Draft<'_> {
        Draft {
            recipient_type: self.recipient_type,
            recipient: &self.recipient,
            recipient_name: self.recipient_name.as_deref(),
            body: &self.body,
            schedule_kind: self.schedule_kind,
            run_at: self.run_at,
            recurrence: self.recurrence.as_ref(),
            timezone: &self.timezone,
            template_id: self.template_id,
            enabled: self.enabled,
        }
    }
}

impl Request {
    #[must_use]
    pub const fn kind(&self) -> ApprovalKind {
        match self {
            Self::Send(_) => ApprovalKind::Send,
            Self::ScheduleCreate(_) => ApprovalKind::ScheduleCreate,
            Self::ScheduleUpdate { .. } => ApprovalKind::ScheduleUpdate,
        }
    }

    /// The chat it's aimed at
    #[must_use]
    pub fn chat(&self) -> (RecipientType, &str, Option<&str>) {
        match self {
            Self::Send(send) => (
                send.recipient_type,
                &send.recipient,
                send.recipient_name.as_deref(),
            ),
            Self::ScheduleCreate(schedule) | Self::ScheduleUpdate { schedule, .. } => (
                schedule.recipient_type,
                &schedule.recipient,
                schedule.recipient_name.as_deref(),
            ),
        }
    }

    const fn body_mut(&mut self) -> &mut String {
        match self {
            Self::Send(send) => &mut send.body,
            Self::ScheduleCreate(schedule) | Self::ScheduleUpdate { schedule, .. } => {
                &mut schedule.body
            }
        }
    }
}

fn context(approval_id: Uuid) -> Vec<u8> {
    [b"agent_approval:".as_slice(), approval_id.as_bytes()].concat()
}

/// # Errors
/// The request couldn't be written, or the random source failed.
pub fn seal(cipher: &Cipher, approval_id: Uuid, request: &Request) -> anyhow::Result<Vec<u8>> {
    let payload = Payload {
        version: PAYLOAD_VERSION,
        request: request.clone(),
    };
    cipher.encrypt(&serde_json::to_vec(&payload)?, &context(approval_id))
}

/// # Errors
/// The payload didn't open, is of a version this server can't read, or isn't the approval's kind.
pub fn open(cipher: &Cipher, approval: &Approval) -> anyhow::Result<Request> {
    let plain = cipher
        .decrypt(&approval.payload, &context(approval.id))
        .context("an approval's payload didn't open")?;
    let payload: Payload = serde_json::from_slice(&plain)?;
    anyhow::ensure!(
        payload.version == PAYLOAD_VERSION,
        "an approval's payload has unknown version {}",
        payload.version
    );
    anyhow::ensure!(
        payload.request.kind() == approval.kind,
        "an approval's payload isn't of its kind"
    );
    Ok(payload.request)
}

/// An approval's request as the owner sees it: who it goes to, the whole text, and for a schedule
/// when it runs
#[derive(Debug, Serialize, ToSchema)]
pub struct Requested {
    pub kind: ApprovalKind,
    pub recipient_type: RecipientType,
    pub recipient: String,
    #[schema(required = true)]
    pub recipient_name: Option<String>,
    /// The whole text, as the agent wrote it; variables are filled in when it goes out
    pub body: String,
    /// The IANA zone its times and variables read in
    pub timezone: String,
    /// For a schedule
    #[schema(required = true)]
    pub schedule: Option<RequestedSchedule>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RequestedSchedule {
    /// The scheduled message it changes; empty for a new one
    #[schema(required = true)]
    pub scheduled_message_id: Option<Uuid>,
    pub schedule_kind: ScheduleKind,
    #[schema(required = true)]
    pub run_at: Option<DateTime<Utc>>,
    #[schema(required = true)]
    pub recurrence: Option<Recurrence>,
    pub enabled: bool,
    /// When it would go out, soonest first
    pub next_runs: Vec<DateTime<Utc>>,
    /// For a sun schedule, the city it watches the sun from unless it names a place
    #[schema(required = true)]
    pub city: Option<&'static str>,
}

impl Requested {
    #[must_use]
    pub fn new(request: Request) -> Self {
        let kind = request.kind();
        let (schedule, scheduled_message_id) = match request {
            Request::Send(send) => {
                return Self {
                    kind,
                    recipient_type: send.recipient_type,
                    recipient: send.recipient,
                    recipient_name: send.recipient_name,
                    body: send.body,
                    timezone: send.timezone.unwrap_or_else(|| "UTC".to_owned()),
                    schedule: None,
                };
            }
            Request::ScheduleCreate(schedule) => (schedule, None),
            Request::ScheduleUpdate {
                scheduled_message_id,
                schedule,
            } => (schedule, Some(scheduled_message_id)),
        };
        let now = Utc::now();
        let next_runs = match (
            schedule.schedule_kind,
            &schedule.recurrence,
            schedule.run_at,
        ) {
            (ScheduleKind::Recurring, Some(recurrence), _) => {
                recurrence.next_runs(&schedule.timezone, now, NEXT_RUNS)
            }
            (ScheduleKind::Once, _, Some(at)) if at > now => vec![at],
            _ => Vec::new(),
        };
        let city = schedule
            .recurrence
            .as_ref()
            .and_then(|recurrence| recurrence.city(&schedule.timezone));
        Self {
            kind,
            recipient_type: schedule.recipient_type,
            recipient: schedule.recipient,
            recipient_name: schedule.recipient_name,
            body: schedule.body,
            timezone: schedule.timezone,
            schedule: Some(RequestedSchedule {
                scheduled_message_id,
                schedule_kind: schedule.schedule_kind,
                run_at: schedule.run_at,
                recurrence: schedule.recurrence,
                enabled: schedule.enabled,
                next_runs,
                city,
            }),
        }
    }
}

/// Why the owner's answer couldn't be carried out. An approval that couldn't be approved stays
/// waiting, so the owner can still reject it.
#[derive(Debug, thiserror::Error)]
pub enum ReviewError {
    #[error("Not Found")]
    NotFound,
    /// It was answered already, or expired
    #[error("This request is no longer waiting")]
    NotWaiting(ApprovalStatus),
    /// Doing it now would fail, in sentences the owner can read
    #[error("{}", .0.join(" "))]
    CantApprove(Vec<String>),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<sqlx::Error> for ReviewError {
    fn from(error: sqlx::Error) -> Self {
        Self::Internal(error.into())
    }
}

/// Opens a waiting approval of `user_id`'s, or says why it can't be answered
async fn waiting(pool: &PgPool, id: Uuid, user_id: Uuid) -> Result<Approval, ReviewError> {
    let approval = agent_approvals::find(pool, id, user_id)
        .await?
        .ok_or(ReviewError::NotFound)?;
    if approval.status == ApprovalStatus::Waiting {
        Ok(approval)
    } else {
        Err(ReviewError::NotWaiting(approval.status))
    }
}

/// The status of an approval a racing answer just decided
async fn decided(pool: &PgPool, id: Uuid, user_id: Uuid) -> ReviewError {
    match agent_approvals::find(pool, id, user_id).await {
        Ok(Some(approval)) => ReviewError::NotWaiting(approval.status),
        Ok(None) => ReviewError::NotFound,
        Err(error) => error.into(),
    }
}

/// Carries out a waiting approval as the owner, with `body` in place of the agent's text if it's
/// given. The request is checked as if it were made now; the grant isn't asked again, since the
/// owner's yes is final.
///
/// # Errors
/// [`ReviewError::NotWaiting`] once answered or expired, including by a racing second click;
/// [`ReviewError::CantApprove`] when it would fail now (a time that has passed, a deleted schedule,
/// a bad variable, WhatsApp not linked), leaving it waiting.
pub async fn approve(
    state: &AppState,
    id: Uuid,
    user_id: Uuid,
    body: Option<String>,
) -> Result<Approval, ReviewError> {
    let approval = waiting(&state.pool, id, user_id).await?;
    let mut request = open(&state.cipher, &approval)?;
    if let Some(body) = body {
        body.trim().clone_into(request.body_mut());
    }
    let filled = check_now(state, user_id, &request).await?;

    let mut transaction = state.pool.begin().await?;
    let became = carry_out(
        &mut transaction,
        user_id,
        approval.connection_id,
        &request,
        filled.as_deref(),
    )
    .await?;
    let Some(approval) = agent_approvals::approve(&mut *transaction, id, user_id, became).await?
    else {
        transaction.rollback().await?;
        return Err(decided(&state.pool, id, user_id).await);
    };
    transaction.commit().await?;
    Ok(approval)
}

/// Checks `request` as if it were made now, and for a send gives its text with the variables
/// filled in
async fn check_now(
    state: &AppState,
    user_id: Uuid,
    request: &Request,
) -> Result<Option<String>, ReviewError> {
    let mut errors = ValidationErrors::new();
    match request {
        Request::Send(send) => {
            draft::check_recipient(send.recipient_type, &send.recipient, None, &mut errors);
            draft::check_body(&send.body, &mut errors);
        }
        Request::ScheduleCreate(schedule) | Request::ScheduleUpdate { schedule, .. } => {
            schedule.draft().check(&mut errors);
            draft::check_template(&state.pool, schedule.template_id, user_id, &mut errors).await?;
        }
    }
    if !errors.is_empty() {
        return Err(ReviewError::CantApprove(sentences(&errors)));
    }
    let Request::Send(send) = request else {
        return Ok(None);
    };
    let whatsapp = whatsapp_state(&state.whatsapp.status(user_id));
    if !whatsapp.can_send {
        return Err(ReviewError::CantApprove(vec![
            whatsapp.description.to_owned(),
        ]));
    }
    let to = variables::To {
        recipient_type: send.recipient_type,
        recipient: &send.recipient,
        recipient_name: send.recipient_name.as_deref(),
    };
    let timezone = send.timezone.as_deref();
    Ok(Some(
        draft::body_now(
            &state.pool,
            &state.cipher,
            user_id,
            &to,
            &send.body,
            timezone,
        )
        .await,
    ))
}

/// Creates the delivery or creates or changes the schedule, in the approval's transaction
async fn carry_out(
    transaction: &mut PgConnection,
    user_id: Uuid,
    connection_id: Uuid,
    request: &Request,
    filled: Option<&str>,
) -> Result<Became, ReviewError> {
    Ok(match request {
        Request::Send(send) => {
            let delivery_id = deliveries::create(
                &mut *transaction,
                NewDelivery {
                    user_id,
                    scheduled_message_id: None,
                    connection_id: Some(connection_id),
                    recipient_type: send.recipient_type,
                    recipient: &send.recipient,
                    recipient_name: send.recipient_name.as_deref(),
                    body: filled.unwrap_or(&send.body),
                },
            )
            .await?;
            Became {
                delivery_id: Some(delivery_id),
                scheduled_message_id: None,
            }
        }
        Request::ScheduleCreate(schedule) => {
            let message = scheduled_messages::create(
                &mut *transaction,
                user_id,
                Some(connection_id),
                schedule.draft().fields(&schedule.recipient),
            )
            .await?;
            Became {
                delivery_id: None,
                scheduled_message_id: Some(message.id),
            }
        }
        Request::ScheduleUpdate {
            scheduled_message_id,
            schedule,
        } => {
            let message = scheduled_messages::update(
                &mut *transaction,
                *scheduled_message_id,
                user_id,
                schedule.draft().fields(&schedule.recipient),
            )
            .await?
            .ok_or_else(|| {
                ReviewError::CantApprove(vec![
                    "The scheduled message this would change has been deleted.".to_owned(),
                ])
            })?;
            Became {
                delivery_id: None,
                scheduled_message_id: Some(message.id),
            }
        }
    })
}

/// # Errors
/// [`ReviewError::NotFound`], or [`ReviewError::NotWaiting`] once answered or expired.
pub async fn reject(pool: &PgPool, id: Uuid, user_id: Uuid) -> Result<Approval, ReviewError> {
    waiting(pool, id, user_id).await?;
    match agent_approvals::reject(pool, id, user_id).await? {
        Some(approval) => Ok(approval),
        None => Err(decided(pool, id, user_id).await),
    }
}

/// Whether a new approval should email its owner: only when they had no other in the last hour
///
/// # Errors
/// The database couldn't be read.
pub async fn should_email(pool: &PgPool, approval: &Approval) -> sqlx::Result<bool> {
    Ok(!agent_approvals::others_in_last_hour(pool, approval).await?)
}
