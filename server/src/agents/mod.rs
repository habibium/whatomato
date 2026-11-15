//! Agent access: what a connected AI agent may do in its owner's account. The grant, the pure
//! decision over it, and access keys. Nothing here knows about HTTP or MCP.

pub mod decide;
pub mod grant;
pub mod key;

pub use decide::{
    Action, ChatRef, DAILY_CEILING, Decision, Facts, Owner, Reason, WAITING_MAX, decide,
};
pub use grant::{Grant, GrantError, Preset};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ApprovalKind {
    Send,
    ScheduleCreate,
    ScheduleUpdate,
}

impl ApprovalKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Send => "send",
            Self::ScheduleCreate => "schedule_create",
            Self::ScheduleUpdate => "schedule_update",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Waiting,
    Approved,
    Rejected,
    /// Not answered within 24 hours
    Expired,
}

/// How a tool call ended, for the activity log
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Allowed,
    /// Waiting for the owner's approval
    Asked,
    Denied,
    /// Allowed, but doing it went wrong
    Failed,
}

impl Outcome {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Asked => "asked",
            Self::Denied => "denied",
            Self::Failed => "failed",
        }
    }
}
