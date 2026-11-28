use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::{Validate, ValidationErrors};

use crate::{
    agents::{
        ApprovalKind, ApprovalStatus, DAILY_CEILING, Grant, Outcome, Preset, WAITING_MAX,
        ops::{account::summary, review::Requested},
    },
    auth::reauth::Reauth,
    db::{
        agent_activity::Activity,
        agent_approvals::Listed,
        agent_connections::{Connection, Revoked},
    },
    oauth,
    routes::deliveries::DeliveryState,
    scheduler::{
        RecipientType,
        draft::{add, finish},
    },
    whatsapp::inbox::History,
};

pub(super) const NAME_MAX: usize = 60;
const EXPIRY_DAYS_MAX: u32 = 365;

/// The Agents page: whether agent access is on, the connections and how many approvals wait
#[derive(Serialize, ToSchema)]
pub(super) struct AgentsOverview {
    /// Whether this server lets AI agents connect at all. When false, `connections` is empty,
    /// `approvals_waiting` is 0 and every other route under `/api/agents` answers 404.
    pub enabled: bool,
    /// Oldest first
    pub connections: Vec<AgentConnection>,
    /// Approvals waiting for an answer, for the count in the navigation
    pub approvals_waiting: i64,
}

/// Whether a connection can be used now
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum ConnectionStatus {
    Active,
    /// Paused by the owner; resuming it makes it active again
    Paused,
    /// Past `expires_at`; it can't be used again, only revoked
    Expired,
    /// Its stored permissions can't be read, so it can do nothing until they're saved again
    Unusable,
}

impl ConnectionStatus {
    const fn new(readable: bool, paused: bool, expired: bool) -> Self {
        if !readable {
            Self::Unusable
        } else if expired {
            Self::Expired
        } else if paused {
            Self::Paused
        } else {
            Self::Active
        }
    }
}

/// An AI agent connection. Never holds the key, only its last characters.
#[derive(Serialize, ToSchema)]
pub(super) struct AgentConnection {
    pub id: Uuid,
    pub name: String,
    /// The key's last 4 characters, to tell keys apart; empty when it was made by signing in
    #[schema(required = true)]
    pub key_hint: Option<String>,
    /// The client that signed in; empty when it uses an access key
    #[schema(required = true)]
    pub signed_in: Option<SignedIn>,
    pub status: ConnectionStatus,
    /// What it may do; empty when the stored permissions can't be read
    #[schema(required = true)]
    pub grant: Option<Grant>,
    /// Why `grant` is empty, in a sentence; empty when it isn't
    #[schema(required = true)]
    pub grant_problem: Option<String>,
    /// The preset `grant` equals; empty for custom permissions
    #[schema(required = true)]
    pub preset: Option<Preset>,
    /// What it may do, in a few sentences
    pub summary: Vec<String>,
    /// When it was paused; empty while it isn't
    #[schema(required = true)]
    pub paused_at: Option<DateTime<Utc>>,
    /// When it stops working; empty for never
    #[schema(required = true)]
    pub expires_at: Option<DateTime<Utc>>,
    /// To the minute; empty until an agent first uses it
    #[schema(required = true)]
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    /// When its name, permissions, key or pause last changed
    pub updated_at: DateTime<Utc>,
    pub usage: Usage,
}

/// The client a connection was made with by signing in. Both come from the client itself.
#[derive(Serialize, ToSchema)]
pub(super) struct SignedIn {
    /// What the client calls itself, which anyone can claim
    pub client_name: String,
    /// The host the client's identity is published on, such as `claude.ai`
    pub client_host: String,
}

impl SignedIn {
    pub fn new(client_id: &str, client_name: String) -> Self {
        Self {
            client_name,
            client_host: oauth::client::host(client_id),
        }
    }
}

/// How much of its limits a connection has used
#[derive(Serialize, ToSchema)]
pub(super) struct Usage {
    /// Messages it sent or queued in the last 24 hours, counting those waiting for approval
    pub sends_last_24_hours: u32,
    /// The most any connection may send in 24 hours
    pub daily_ceiling: u32,
    /// Its requests waiting for approval
    pub approvals_waiting: u32,
    /// The most that may wait at once; past it, its next request is refused
    pub approvals_waiting_max: u32,
}

impl AgentConnection {
    pub fn new(connection: Connection, (sends, waiting): (u32, u32)) -> Self {
        let grant = Grant::parse(&connection.grant)
            .inspect_err(|error| {
                tracing::warn!(connection_id = %connection.id, "an agent grant didn't parse: {error}");
            })
            .ok();
        let expired = connection.expires_at.is_some_and(|at| at <= Utc::now());
        let status =
            ConnectionStatus::new(grant.is_some(), connection.paused_at.is_some(), expired);
        Self {
            id: connection.id,
            name: connection.name,
            key_hint: connection.key_hint,
            signed_in: connection
                .client_id
                .zip(connection.client_name)
                .map(|(id, name)| SignedIn::new(&id, name)),
            status,
            grant_problem: grant.is_none().then(|| {
                "This connection's permissions couldn't be read, so it can't do anything. Choose \
                 its permissions again and save them to fix it."
                    .to_owned()
            }),
            preset: grant.as_ref().and_then(Preset::of),
            summary: grant.as_ref().map_or_default(summary),
            grant,
            paused_at: connection.paused_at,
            expires_at: connection.expires_at,
            last_used_at: connection.last_used_at,
            created_at: connection.created_at,
            updated_at: connection.updated_at,
            usage: Usage {
                sends_last_24_hours: sends,
                daily_ceiling: DAILY_CEILING,
                approvals_waiting: waiting,
                approvals_waiting_max: WAITING_MAX,
            },
        }
    }
}

/// One connection, with what revoking it would leave behind
#[derive(Serialize, ToSchema)]
pub(super) struct AgentConnectionDetail {
    #[serde(flatten)]
    pub connection: AgentConnection,
    /// Scheduled messages it created or changed that still exist, which revoking can delete or keep
    pub schedules: i64,
}

/// A connection with its key. The key is shown here once and can't be read again.
#[derive(Serialize, ToSchema)]
pub(super) struct AgentKeyIssued {
    pub connection: AgentConnection,
    /// The whole access key, for the agent's `Authorization: Bearer` header. Only its hint is
    /// kept, so it can't be shown again; a lost key is replaced by rotating it.
    pub key: String,
}

/// What this server allows of reading chats
#[derive(Serialize, ToSchema)]
pub(super) struct AgentReading {
    /// Whether a connection may be allowed to read chats; set by `AGENT_READING`
    pub available: bool,
    /// What the owner keeps: `off` keeps a readable chat's last 7 days, otherwise every chat is kept
    /// for that many `days` or `all` of it. A connection reads no further back than this.
    pub history: History,
}

/// A starting point for a connection's permissions
#[derive(Serialize, ToSchema)]
pub(super) struct AgentPreset {
    pub id: Preset,
    pub name: &'static str,
    /// What it lets an agent do, in a sentence
    pub description: &'static str,
    pub grant: Grant,
}

impl AgentPreset {
    pub const fn new(preset: Preset) -> Self {
        let (name, description) = match preset {
            Preset::AskBeforeSending => (
                "Ask before sending",
                "It can find chats, read templates and schedule messages, and every message it \
                 writes waits for your approval before it goes out.",
            ),
            Preset::ViewOnly => (
                "View only",
                "It can find chats and read templates, but it can't send or schedule anything.",
            ),
            Preset::SendWithoutAsking => (
                "Send without asking",
                "It sends without asking, up to 20 messages a day in each chat; messages to \
                 people who aren't saved contacts still wait for your approval.",
            ),
            Preset::FullAccess => (
                "Full access",
                "It sends to anyone without asking, people who aren't saved contacts included, \
                 and can see and change every scheduled message.",
            ),
        };
        Self {
            id: preset,
            name,
            description,
            grant: preset.grant(),
        }
    }
}

#[derive(Deserialize, ToSchema)]
pub(super) struct CreateConnection {
    /// 1 to 60 characters, such as the agent's or the computer's name
    pub name: String,
    pub grant: Grant,
    /// Days until it stops working, 1 to 365; left out, it never expires
    pub expires_in_days: Option<u32>,
    #[serde(flatten)]
    pub reauth: Reauth,
}

impl Validate for CreateConnection {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();
        check_name(&self.name, &mut errors);
        if self
            .expires_in_days
            .is_some_and(|days| !(1..=EXPIRY_DAYS_MAX).contains(&days))
        {
            add(
                &mut errors,
                "expires_in_days",
                "must be between 1 and 365 days",
            );
        }
        self.reauth.check(&mut errors);
        finish(errors)
    }
}

/// A new name, new permissions, or both
#[derive(Deserialize, ToSchema)]
pub(super) struct UpdateConnection {
    /// 1 to 60 characters
    pub name: Option<String>,
    /// Needs the password or a passkey
    pub grant: Option<Grant>,
    /// Needed only with `grant`
    #[serde(flatten)]
    pub reauth: Reauth,
}

impl Validate for UpdateConnection {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();
        if let Some(name) = &self.name {
            check_name(name, &mut errors);
        }
        if self.name.is_none() && self.grant.is_none() {
            add(&mut errors, "name", "give a new name or new permissions");
        }
        if self.grant.is_some() {
            if self.reauth.is_empty() {
                add(
                    &mut errors,
                    "password",
                    "or a passkey is needed to change what an agent may do",
                );
            } else {
                self.reauth.check(&mut errors);
            }
        }
        finish(errors)
    }
}

pub(super) fn check_name(name: &str, errors: &mut ValidationErrors) {
    if !(1..=NAME_MAX).contains(&name.trim().chars().count()) {
        add(errors, "name", "must be between 1 and 60 characters");
    }
}

#[derive(Deserialize, IntoParams, Validate)]
#[into_params(parameter_in = Query)]
pub(super) struct RevokeQuery {
    /// Also delete the scheduled messages it created or changed. Left out, they're kept as your own.
    #[serde(default)]
    pub delete_schedules: bool,
}

/// What revoking did with the scheduled messages the connection created or changed
#[derive(Serialize, ToSchema)]
pub(super) struct AgentRevoked {
    pub schedules_deleted: u64,
    /// Kept, and now shown as your own
    pub schedules_kept: u64,
}

impl From<Revoked> for AgentRevoked {
    fn from(revoked: Revoked) -> Self {
        Self {
            schedules_deleted: revoked.schedules_deleted,
            schedules_kept: revoked.schedules_kept,
        }
    }
}

#[derive(Serialize, ToSchema)]
pub(super) struct AgentsPaused {
    /// How many were running and are now paused
    pub paused: u64,
}

const fn default_limit() -> i64 {
    50
}

#[derive(Deserialize, IntoParams, Validate)]
#[into_params(parameter_in = Query)]
pub(super) struct ApprovalsQuery {
    /// 1 to 100, 50 by default
    #[serde(default = "default_limit")]
    #[validate(range(min = 1, max = 100, message = "must be between 1 and 100"))]
    pub limit: i64,
    #[serde(default)]
    #[validate(range(min = 0, message = "can't be negative"))]
    pub offset: i64,
}

#[derive(Deserialize, IntoParams, Validate)]
#[into_params(parameter_in = Query)]
pub(super) struct ActivityQuery {
    /// Only this connection's
    pub connection_id: Option<Uuid>,
    /// 1 to 100, 50 by default
    #[serde(default = "default_limit")]
    #[validate(range(min = 1, max = 100, message = "must be between 1 and 100"))]
    pub limit: i64,
    #[serde(default)]
    #[validate(range(min = 0, message = "can't be negative"))]
    pub offset: i64,
}

/// What an agent asked to do, waiting for the owner or answered
#[derive(Serialize, ToSchema)]
pub(super) struct AgentApproval {
    pub id: Uuid,
    pub connection_id: Uuid,
    pub connection_name: String,
    /// Where the connection that asked stands now. Approving doesn't depend on it: the owner's
    /// yes goes ahead even for a paused or expired connection.
    pub connection_status: ConnectionStatus,
    pub kind: ApprovalKind,
    pub status: ApprovalStatus,
    pub recipient_type: RecipientType,
    pub recipient: String,
    #[schema(required = true)]
    pub recipient_name: Option<String>,
    /// The whole request; empty when it couldn't be read, and then it can only be rejected
    #[schema(required = true)]
    pub request: Option<Requested>,
    /// Why `request` is empty, in a sentence; empty when it isn't
    #[schema(required = true)]
    pub problem: Option<String>,
    /// Whether approving it can be tried: it's waiting and could be read
    pub can_approve: bool,
    /// Whether it's still waiting, so it can be rejected
    pub can_reject: bool,
    pub created_at: DateTime<Utc>,
    /// When it expires unanswered, 24 hours after it was made
    pub expires_at: DateTime<Utc>,
    /// When it was approved, rejected or expired
    #[schema(required = true)]
    pub decided_at: Option<DateTime<Utc>>,
    /// The message approving it queued; empty until then, or once that's deleted
    #[schema(required = true)]
    pub delivery_id: Option<Uuid>,
    /// Where that message stands
    #[schema(required = true)]
    pub delivery_status: Option<DeliveryState>,
    /// The scheduled message approving it created or changed; empty until then, or once it's
    /// deleted
    #[schema(required = true)]
    pub scheduled_message_id: Option<Uuid>,
}

pub(super) const UNREADABLE: &str =
    "This request couldn't be read, so it can't be approved. Reject it, and ask the agent again.";

impl AgentApproval {
    pub fn new(listed: Listed, request: Option<Requested>) -> Self {
        let Listed {
            approval,
            connection_name,
            connection_paused,
            connection_expired,
            connection_grant,
            delivery_status,
        } = listed;
        let waiting = approval.status == ApprovalStatus::Waiting;
        let readable = Grant::parse(&connection_grant).is_ok();
        Self {
            id: approval.id,
            connection_id: approval.connection_id,
            connection_name,
            connection_status: ConnectionStatus::new(
                readable,
                connection_paused,
                connection_expired,
            ),
            kind: approval.kind,
            status: approval.status,
            recipient_type: approval.recipient_type,
            recipient: approval.recipient,
            recipient_name: approval.recipient_name,
            problem: request.is_none().then(|| UNREADABLE.to_owned()),
            can_approve: waiting && request.is_some(),
            can_reject: waiting,
            request,
            created_at: approval.created_at,
            expires_at: approval.expires_at,
            decided_at: approval.decided_at,
            delivery_id: approval.delivery_id,
            delivery_status: delivery_status.map(Into::into),
            scheduled_message_id: approval.scheduled_message_id,
        }
    }
}

#[derive(Serialize, ToSchema)]
pub(super) struct AgentApprovalPage {
    /// Waiting first, then newest first
    pub items: Vec<AgentApproval>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Deserialize, Validate, ToSchema)]
pub(super) struct ApproveRequest {
    /// The text to send instead of the agent's, checked as the scheduling form checks it. Left
    /// out, the agent's text goes as written.
    pub body: Option<String>,
}

/// One call an agent made. Never holds message text.
#[derive(Serialize, ToSchema)]
pub(super) struct AgentActivity {
    pub id: Uuid,
    /// Empty once the connection is revoked; `connection_name` still says which it was
    #[schema(required = true)]
    pub connection_id: Option<Uuid>,
    pub connection_name: String,
    /// The tool it called, such as `send_message`
    pub tool: String,
    /// The chat it was about; the three are empty when no chat was involved
    #[schema(required = true)]
    pub recipient_type: Option<RecipientType>,
    #[schema(required = true)]
    pub recipient: Option<String>,
    #[schema(required = true)]
    pub recipient_name: Option<String>,
    pub outcome: Outcome,
    /// A stable code for why it was refused or failed, such as `chat_limit`
    #[schema(required = true)]
    pub reason: Option<String>,
    /// `reason` in a sentence
    #[schema(required = true)]
    pub reason_text: Option<&'static str>,
    pub created_at: DateTime<Utc>,
}

impl From<Activity> for AgentActivity {
    fn from(activity: Activity) -> Self {
        Self {
            id: activity.id,
            connection_id: activity.connection_id,
            connection_name: activity.connection_name,
            tool: activity.tool,
            recipient_type: activity.recipient_type,
            recipient: activity.recipient,
            recipient_name: activity.recipient_name,
            outcome: activity.outcome,
            reason_text: activity.reason.as_deref().and_then(reason_text),
            reason: activity.reason,
            created_at: activity.created_at,
        }
    }
}

fn reason_text(code: &str) -> Option<&'static str> {
    Some(match code {
        "schedules_off" => "It isn't allowed to see or manage scheduled messages.",
        "not_own_schedule" => "It tried to change a scheduled message that isn't its own.",
        "contacts_off" => "It isn't allowed to look up contacts and groups.",
        "templates_off" => "It isn't allowed to read templates.",
        "daily_ceiling" => "It had already sent the most it may in 24 hours.",
        "out_of_scope" => "That chat isn't one it may use.",
        "strangers_never" => "It isn't allowed to message people who aren't saved contacts.",
        "sending_off" => "It isn't allowed to send to that chat.",
        "chat_limit" => "It had reached its daily limit for that chat.",
        "too_many_waiting" => "Too many of its requests were already waiting for your approval.",
        "too_many_schedules" => "It already held the most scheduled messages it may: 50.",
        "paused" => "The connection was paused.",
        "expired" => "The connection had expired.",
        "grant_unreadable" => "Its permissions couldn't be read.",
        "invalid_input" => "Its request had a problem, so nothing was done.",
        "unreadable_arguments" => {
            "Its request was missing a value or had one of the wrong kind, so nothing was done."
        }
        "too_frequent" => {
            "It asked for a schedule that sends more often than once an hour, which an agent may \
             not make."
        }
        "not_found" => "What it asked for wasn't found.",
        "whatsapp_not_ready" => "WhatsApp wasn't ready to send.",
        "too_often" => "It searched chats more often than it may, so it was asked to wait.",
        "internal" => "Something went wrong on the server.",
        _ => return None,
    })
}

#[derive(Serialize, ToSchema)]
pub(super) struct AgentActivityPage {
    /// Newest first
    pub items: Vec<AgentActivity>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}
