//! One operation per agent tool. Each runs decide, do, record: [`decide`] says whether the caller
//! may, the operation does it, and one activity row records how it ended. Nothing here knows about
//! MCP or HTTP; the account always comes from the [`Caller`], never from an input.
//!
//! Text other people or the owner wrote (names, notes, bodies) only ever sits in fields of its
//! own, never inside a sentence the server writes, so an agent can tell data from what we say.
//!
//! [`decide`]: super::decide

pub mod account;
pub mod approvals;
pub mod review;
pub mod schedules;
pub mod sends;
#[cfg(test)]
mod tests;

use anyhow::anyhow;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use url::Url;
use uuid::Uuid;
use validator::ValidationErrors;

use self::{
    account::{WhatsAppState, WhatsAppStatus},
    review::Request,
};
use super::{
    Grant, Outcome, Reason, alerts,
    decide::{ChatRef, Facts},
};
use crate::{
    db::{
        agent_activity::{self, ActivityChat, NewActivity},
        agent_approvals::{self, Approval, NewApproval},
        agent_connections::{self, Connection, KeyMatch},
        whatsapp_contacts,
    },
    email::templates::AgentAlert,
    scheduler::{RecipientType, draft::NAME_MAX},
    state::AppState,
    whatsapp::{contacts::open_names, status::Status},
};

/// Items a page holds when the agent doesn't say
pub const PAGE_DEFAULT: u32 = 20;
pub const PAGE_MAX: u32 = 50;
/// Characters of other people's text (bodies, names) one page may carry, about 10,000 tokens, so a
/// page stays far under the 25,000 tokens some clients cut tool output at
const TEXT_BUDGET: usize = 40_000;

/// Who is calling: a connection that may be used now, and its grant
pub struct Caller {
    connection: Connection,
    grant: Grant,
    /// Where the owner manages this connection
    settings_url: String,
}

impl Caller {
    /// # Errors
    /// [`OpError::Refused`] for a paused or expired connection, or one whose grant doesn't parse.
    pub fn from_key_match(app_url: &Url, found: KeyMatch) -> Result<Self, OpError> {
        Self::build(app_url, found.connection, found.paused, found.expired)
    }

    /// # Errors
    /// As [`Self::from_key_match`].
    #[cfg(test)]
    pub fn from_connection(app_url: &Url, connection: Connection) -> Result<Self, OpError> {
        let paused = connection.paused_at.is_some();
        let expired = connection
            .expires_at
            .is_some_and(|at| at <= chrono::Utc::now());
        Self::build(app_url, connection, paused, expired)
    }

    fn build(
        app_url: &Url,
        connection: Connection,
        paused: bool,
        expired: bool,
    ) -> Result<Self, OpError> {
        let settings_url = link(app_url, &format!("agents/{}", connection.id));
        if let Some(stopped) = stopped(&settings_url, paused, expired) {
            return Err(stopped);
        }
        let grant = Grant::parse(&connection.grant).map_err(|error| {
            tracing::warn!(connection_id = %connection.id, "an agent grant didn't parse: {error}");
            unreadable_grant(&settings_url)
        })?;
        Ok(Self {
            connection,
            grant,
            settings_url,
        })
    }

    #[must_use]
    pub const fn connection(&self) -> &Connection {
        &self.connection
    }

    #[must_use]
    pub const fn grant(&self) -> &Grant {
        &self.grant
    }

    const fn user_id(&self) -> Uuid {
        self.connection.user_id
    }

    const fn id(&self) -> Uuid {
        self.connection.id
    }

    fn refused(&self, reason: Reason) -> OpError {
        OpError::Refused {
            code: reason.code(),
            message: reason.message(),
            url: self.settings_url.clone(),
        }
    }
}

fn unreadable_grant(settings_url: &str) -> OpError {
    OpError::Refused {
        code: "grant_unreadable",
        message: "This connection's permissions couldn't be read, so it can't do anything. The \
                  owner can fix this by saving its permissions again."
            .to_owned(),
        url: settings_url.to_owned(),
    }
}

/// The refusal for a paused or expired connection
fn stopped(settings_url: &str, paused: bool, expired: bool) -> Option<OpError> {
    let (code, message) = if paused {
        (
            "paused",
            "The owner has paused this connection, so it can't do anything until they resume it.",
        )
    } else if expired {
        (
            "expired",
            "This connection has expired, so it can't do anything. The owner can create a new \
             connection.",
        )
    } else {
        return None;
    };
    Some(OpError::Refused {
        code,
        message: message.to_owned(),
        url: settings_url.to_owned(),
    })
}

/// Why an operation didn't do what was asked. Every message but [`OpError::Internal`]'s is meant
/// for the agent to read and relay.
#[derive(Debug, thiserror::Error)]
pub enum OpError {
    /// The grant doesn't allow it; `url` is where the owner changes that
    #[error("{message} The owner manages this connection at {url}")]
    Refused {
        code: &'static str,
        message: String,
        url: String,
    },
    /// What's wrong with the input, one sentence each
    #[error("This request has problems: {}", .0.join(" "))]
    Invalid(Vec<String>),
    #[error("{0}")]
    NotFound(&'static str),
    /// WhatsApp isn't linked, is paused or has stopped
    #[error("{0}")]
    WhatsAppNotReady(&'static str),
    /// Called too often; says how long to wait
    #[error("{0}")]
    TooOften(&'static str),
    /// A schedule that would send more often than an agent may schedule
    #[error("{0}")]
    TooFrequent(&'static str),
    /// Logged; the agent hears only that something went wrong
    #[error("Something went wrong on the server, so nothing was done. Try again in a few minutes.")]
    Internal(anyhow::Error),
}

impl OpError {
    /// The reason the activity log records
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Refused { code, .. } => code,
            Self::Invalid(_) => "invalid_input",
            Self::NotFound(_) => "not_found",
            Self::WhatsAppNotReady(_) => "whatsapp_not_ready",
            Self::TooOften(_) => "too_often",
            Self::TooFrequent(_) => "too_frequent",
            Self::Internal(_) => "internal",
        }
    }

    fn invalid(problem: &str) -> Self {
        Self::Invalid(vec![problem.to_owned()])
    }
}

impl From<sqlx::Error> for OpError {
    fn from(error: sqlx::Error) -> Self {
        Self::Internal(error.into())
    }
}

impl From<anyhow::Error> for OpError {
    fn from(error: anyhow::Error) -> Self {
        Self::Internal(error)
    }
}

impl From<ValidationErrors> for OpError {
    fn from(errors: ValidationErrors) -> Self {
        Self::Invalid(sentences(&errors))
    }
}

/// One sentence per problem, naming the input field it's about
fn sentences(errors: &ValidationErrors) -> Vec<String> {
    let mut problems: Vec<String> = errors
        .field_errors()
        .into_iter()
        .flat_map(|(field, errors)| {
            errors.iter().map(move |error| {
                let message = error.message.as_deref().unwrap_or(&error.code);
                let predicate = ["must ", "is ", "isn't ", "can't ", "never "]
                    .iter()
                    .any(|start| message.starts_with(start));
                if predicate {
                    format!("`{field}` {message}.")
                } else {
                    format!("`{field}` has a problem: {message}.")
                }
            })
        })
        .collect();
    problems.sort();
    problems
}

/// What one call did, written to the activity log when it ends
struct Trace {
    tool: &'static str,
    chat: Option<(RecipientType, String, Option<String>)>,
    asked: bool,
}

impl Trace {
    const fn new(tool: &'static str) -> Self {
        Self {
            tool,
            chat: None,
            asked: false,
        }
    }

    fn chat(&mut self, recipient_type: RecipientType, recipient: &str, name: Option<&str>) {
        self.chat = Some((
            recipient_type,
            recipient.to_owned(),
            name.map(str::to_owned),
        ));
    }

    /// Records the call. A failure to record is logged, never passed on.
    async fn finish<T: Sync>(self, state: &AppState, caller: &Caller, result: &Result<T, OpError>) {
        let (outcome, reason) = match result {
            Ok(_) if self.asked => (Outcome::Asked, None),
            Ok(_) => (Outcome::Allowed, None),
            Err(
                error @ (OpError::Refused { .. } | OpError::TooOften(_) | OpError::TooFrequent(_)),
            ) => (Outcome::Denied, Some(error.code())),
            Err(error) => (Outcome::Failed, Some(error.code())),
        };
        if let Err(OpError::Internal(error)) = result {
            tracing::error!(tool = self.tool, connection_id = %caller.id(), "an agent tool failed: {error:#}");
        }
        let chat = self
            .chat
            .as_ref()
            .map(|(recipient_type, recipient, name)| ActivityChat {
                recipient_type: *recipient_type,
                recipient,
                recipient_name: name.as_deref(),
            });
        let written = agent_activity::insert(
            &state.pool,
            NewActivity {
                user_id: caller.user_id(),
                connection_id: caller.id(),
                connection_name: &caller.connection.name,
                tool: self.tool,
                chat,
                outcome,
                reason,
            },
        )
        .await;
        if let Err(error) = written {
            tracing::error!(tool = self.tool, connection_id = %caller.id(), ?error, "couldn't record an agent's activity");
        }
        if let Err(error) = agent_connections::touch(&state.pool, caller.id()).await {
            tracing::warn!(connection_id = %caller.id(), ?error, "couldn't note an agent connection's use");
        }
    }
}

/// Records a call to `tool` whose arguments couldn't be read, so it reached no operation. A failure
/// to record is logged, never passed on.
pub async fn record_unreadable(state: &AppState, caller: &Caller, tool: &str) {
    let written = agent_activity::insert(
        &state.pool,
        NewActivity {
            user_id: caller.user_id(),
            connection_id: caller.id(),
            connection_name: &caller.connection.name,
            tool,
            chat: None,
            outcome: Outcome::Failed,
            reason: Some(UNREADABLE_ARGUMENTS),
        },
    )
    .await;
    if let Err(error) = written {
        tracing::error!(tool, connection_id = %caller.id(), ?error, "couldn't record an agent's activity");
    }
}

/// The reason recorded for a call whose arguments couldn't be read
pub const UNREADABLE_ARGUMENTS: &str = "unreadable_arguments";

/// Paging for every list. `limit` and `offset` are left out of a schema's required fields.
#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema)]
pub struct Page {
    /// How many items to return, from 1 to 50. 20 when left out.
    pub limit: Option<u32>,
    /// How many items to skip, to get the next page: the `next_offset` of the page before. 0 when
    /// left out.
    pub offset: Option<u32>,
}

impl Page {
    fn bounds(self) -> Result<(usize, usize), OpError> {
        let limit = self.limit.unwrap_or(PAGE_DEFAULT);
        if !(1..=PAGE_MAX).contains(&limit) {
            return Err(OpError::invalid("`limit` must be between 1 and 50."));
        }
        Ok((limit as usize, self.offset.unwrap_or(0) as usize))
    }
}

/// Where the next page starts, when there is one
#[derive(Debug, Serialize, JsonSchema)]
pub struct More {
    /// Whether there are more items after these
    pub has_more: bool,
    /// The `offset` to ask for the next page with; empty when there's none
    pub next_offset: Option<u32>,
}

impl More {
    fn after(offset: usize, shown: usize, has_more: bool) -> Self {
        Self {
            has_more,
            next_offset: has_more.then(|| u32::try_from(offset + shown).unwrap_or(u32::MAX)),
        }
    }
}

/// The items from `offset` that fit in a page of `limit` and in [`TEXT_BUDGET`], at least one
fn fit<T>(
    items: Vec<T>,
    limit: usize,
    offset: usize,
    text: impl Fn(&T) -> usize,
) -> (Vec<T>, More) {
    let mut rest = items.into_iter().skip(offset).peekable();
    let mut page = Vec::new();
    let mut used = 0;
    while let Some(item) = rest
        .next_if(|item| page.len() < limit && (page.is_empty() || used + text(item) <= TEXT_BUDGET))
    {
        used += text(&item);
        page.push(item);
    }
    let shown = page.len();
    (page, More::after(offset, shown, rest.peek().is_some()))
}

/// What the account's WhatsApp link can do now, in words for the agent
const fn whatsapp_state(status: &Status) -> WhatsAppState {
    let (status, can_send, description) = match status {
        Status::Connected { .. } => (
            WhatsAppStatus::Connected,
            true,
            "WhatsApp is connected. Queued messages go out within moments.",
        ),
        Status::Syncing { .. } => (
            WhatsAppStatus::Syncing,
            true,
            "WhatsApp is linked and finishing its first sync. Messages can be queued and go out \
             once it's done.",
        ),
        Status::Reconnecting { .. } => (
            WhatsAppStatus::Reconnecting,
            true,
            "WhatsApp's connection dropped and is being retried. Messages can be queued and go out \
             once it's back.",
        ),
        Status::Paused { .. } => (
            WhatsAppStatus::Paused,
            false,
            "The owner has paused WhatsApp, so no message can be sent until they resume it in \
             Whatomato.",
        ),
        Status::Stopped { .. } => (
            WhatsAppStatus::Stopped,
            false,
            "WhatsApp has stopped, so no message can be sent until the owner reconnects it in \
             Whatomato.",
        ),
        Status::Pairing(_) => (
            WhatsAppStatus::Linking,
            false,
            "The owner is linking WhatsApp right now, so no message can be sent until that's done.",
        ),
        Status::NotLinked | Status::LinkEnded(_) => (
            WhatsAppStatus::NotLinked,
            false,
            "WhatsApp isn't linked to this account, so no message can be sent. The owner can link \
             it in Whatomato.",
        ),
    };
    WhatsAppState {
        status,
        can_send,
        description,
    }
}

/// # Errors
/// [`OpError::WhatsAppNotReady`] unless the account can take a send now.
fn whatsapp_ready(state: &AppState, user_id: Uuid) -> Result<(), OpError> {
    let whatsapp = whatsapp_state(&state.whatsapp.status(user_id));
    if whatsapp.can_send {
        Ok(())
    } else {
        Err(OpError::WhatsAppNotReady(whatsapp.description))
    }
}

/// What the database knows of a chat that a decision and the owner's pages need
struct ChatFacts {
    /// A contact in the saved address book
    saved: bool,
    /// The rule's label, else the name saved on the phone
    name: Option<String>,
}

async fn chat_facts(
    state: &AppState,
    caller: &Caller,
    chat: ChatRef<'_>,
) -> Result<ChatFacts, OpError> {
    let rule_name = caller
        .grant
        .rule_for(chat.recipient_type, chat.recipient)
        .and_then(|rule| rule.chat.name.clone());
    let saved = match chat.recipient_type {
        RecipientType::Contact => {
            whatsapp_contacts::find_by_number(&state.pool, caller.user_id(), chat.recipient).await?
        }
        RecipientType::Group => None,
    };
    let saved_name = saved.as_ref().and_then(|found| {
        open_names(&state.cipher, found.account_id, &found.jid, &found.names)
            .inspect_err(|error| tracing::error!("a WhatsApp contact didn't open: {error:#}"))
            .ok()?
            .display()
            .map(str::to_owned)
    });
    let name = rule_name
        .or(saved_name)
        .map(|name| name.chars().take(NAME_MAX).collect());
    Ok(ChatFacts {
        saved: saved.is_some(),
        name,
    })
}

/// `path` under the app's URL
fn link(app_url: &Url, path: &str) -> String {
    app_url.join(path).map_or_else(
        |_| format!("{}/{path}", app_url.as_str().trim_end_matches('/')),
        Into::into,
    )
}

fn approval_url(app_url: &Url, id: Uuid) -> String {
    link(app_url, &format!("agents?approval={id}"))
}

/// Takes the connection's lock until `db`'s transaction ends, so that calls at the same moment
/// count each other: whatever is counted, decided and inserted under it can't race. Nothing slow
/// (WhatsApp, email, another pool connection) may happen while it's held. Gives the grant as the
/// locked row holds it, which is the one to decide with: the owner may have changed it since the
/// key was looked up.
///
/// # Errors
/// [`OpError::Refused`] when the connection was paused or expired since the call began, or its
/// grant no longer parses.
async fn lock(db: &mut PgConnection, caller: &Caller) -> Result<Grant, OpError> {
    let locked = agent_connections::lock(db, caller.id())
        .await?
        .ok_or_else(|| anyhow!("the connection was revoked during a call"))?;
    if let Some(stopped) = stopped(&caller.settings_url, locked.paused, locked.expired) {
        return Err(stopped);
    }
    Grant::parse(&locked.grant).map_err(|error| {
        tracing::warn!(connection_id = %caller.id(), "an agent grant didn't parse: {error}");
        unreadable_grant(&caller.settings_url)
    })
}

/// What [`decide`](super::decide) needs to know about `chat` and this connection's use of it.
/// Read under [`lock`].
async fn facts(
    db: &mut PgConnection,
    caller: &Caller,
    chat: ChatRef<'_>,
    saved_contact: bool,
) -> Result<Facts, OpError> {
    let counts =
        agent_connections::counts(db, caller.id(), chat.recipient_type, chat.recipient).await?;
    Ok(Facts {
        saved_contact,
        chat_sends: counts.chat_sends,
        connection_sends: counts.connection_sends,
        waiting_approvals: counts.waiting_approvals,
        schedules: counts.schedules,
    })
}

/// Stores `request` as an approval waiting for the owner, under [`lock`]. Once the transaction
/// is committed, [`tell_owner`] emails about it.
async fn ask(
    db: &mut PgConnection,
    state: &AppState,
    caller: &Caller,
    request: &Request,
) -> Result<Approval, OpError> {
    let id = Uuid::new_v4();
    let payload = review::seal(&state.cipher, id, request)?;
    let (recipient_type, recipient, recipient_name) = request.chat();
    let approval = agent_approvals::create(
        db,
        NewApproval {
            id,
            connection_id: caller.id(),
            kind: request.kind(),
            recipient_type,
            recipient,
            recipient_name,
            payload: &payload,
        },
    )
    .await?
    .ok_or_else(|| anyhow!("the connection was revoked while it asked"))?;
    Ok(approval)
}

/// Emails the owner about a new approval, at most once an hour
async fn tell_owner(state: &AppState, caller: &Caller, approval: &Approval) {
    match review::should_email(&state.pool, approval).await {
        Ok(true) => {
            let alert = AgentAlert::ApprovalWaiting {
                name: &caller.connection.name,
                kind: approval.kind,
                approval_id: approval.id,
            };
            alerts::send(state, caller.user_id(), alert).await;
        }
        Ok(false) => {}
        Err(error) => {
            tracing::error!(?error, "couldn't tell whether to email about an approval");
        }
    }
}
