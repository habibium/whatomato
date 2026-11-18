//! `send_message` and `list_deliveries`

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use validator::ValidationErrors;

use super::{
    Caller, More, OpError, Page, Seen, Trace, approval_url, ask, chat_facts, facts, lock,
    review::{Request, SendRequest},
    tell_owner, whatsapp_ready,
};
use crate::{
    agents::{
        Decision,
        decide::{Action, ChatRef, Facts, decide},
    },
    db::{
        deliveries::{self, Filter, NewDelivery},
        inbox_messages,
    },
    scheduler::{DeliveryStatus, RecipientType, draft, variables},
    state::AppState,
    whatsapp::tags,
};

/// People one message tags at most
const MENTIONS_MAX: usize = 50;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SendInput {
    /// `contact` for a person, `group` for a WhatsApp group
    pub recipient_type: RecipientType,
    /// For a contact, the phone number with its country code (a leading +, spaces and dashes are
    /// ignored). For a group, its id as `find_chats` gives it.
    pub recipient: String,
    /// The message text, up to 4096 characters. It may use variables such as `{{first_name}}`,
    /// `{{name}}`, `{{date}}` and `{{time}}`, filled in when it goes out.
    pub body: String,
    /// The IANA time zone that `{{date}}`, `{{time}}` and similar variables read in, such as
    /// Asia/Dhaka. UTC when left out.
    pub timezone: Option<String>,
    /// To answer one message and quote it: its `id` from `read_messages`, in this same chat. It
    /// needs permission to read the chat. Leave it out for an ordinary message.
    pub reply_to: Option<String>,
    /// People to tag in a group, by phone number with its country code. Write @ and each number
    /// in `body` where its tag goes, such as "@8801711111111 can you check?": their phone shows
    /// their name there and tells them. `read_messages` gives the numbers of who wrote and who was
    /// tagged.
    #[serde(default)]
    pub mentions: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SendOutcome {
    /// In the queue; not sent yet
    Queued,
    /// Waiting for the owner's approval; not sent
    WaitingForApproval,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SendResult {
    pub outcome: SendOutcome,
    /// What happened, and how to follow it up
    pub message: &'static str,
    /// When queued: the delivery to look for in `list_deliveries`
    pub delivery_id: Option<Uuid>,
    /// When waiting: the approval to check with `get_approval`
    pub approval_id: Option<Uuid>,
    /// When waiting: the page where the owner approves or rejects it
    pub approval_url: Option<String>,
}

/// The message a reply quotes: one this connection may read, kept in the chat it sends to
async fn quoted(
    state: &AppState,
    caller: &Caller,
    chat: ChatRef<'_>,
    reply_to: Option<&str>,
) -> Result<Option<String>, OpError> {
    let Some(id) = reply_to.map(str::trim).filter(|id| !id.is_empty()) else {
        return Ok(None);
    };
    if !state.agent_reading || !caller.grant.reads(chat.recipient_type, chat.recipient) {
        return Err(OpError::invalid(
            "`reply_to` needs this connection to be allowed to read this chat. Send without it, \
             or ask the owner to turn reading on for this chat.",
        ));
    }
    let kind = chat.recipient_type.as_str();
    inbox_messages::find(&state.pool, caller.user_id(), kind, chat.recipient, id)
        .await?
        .ok_or(OpError::NotFound(
            "The message `reply_to` names isn't kept in this chat. Use an id that read_messages \
             gave for the same chat, or send without `reply_to`.",
        ))?;
    Ok(Some(id.to_owned()))
}

/// The numbers a send tags, each written as `@number` in its body. Only a group tags people.
fn tagged(
    recipient_type: RecipientType,
    body: &str,
    mentions: &[String],
) -> Result<Vec<String>, OpError> {
    if mentions.is_empty() {
        return Ok(Vec::new());
    }
    if recipient_type != RecipientType::Group {
        return Err(OpError::invalid(
            "`mentions` tags people in a group. A message to one person can't tag anyone; send \
             it without `mentions`.",
        ));
    }
    if mentions.len() > MENTIONS_MAX {
        return Err(OpError::invalid("A message can tag at most 50 people."));
    }
    let mut numbers: Vec<String> = Vec::with_capacity(mentions.len());
    for mention in mentions {
        let number = RecipientType::Contact.normalized(mention);
        if !(7..=15).contains(&number.len()) {
            return Err(OpError::Invalid(vec![format!(
                "`{mention}` in `mentions` isn't a phone number. Give each person's number with \
                 its country code."
            )]));
        }
        if !tags::written(body, &number) {
            return Err(OpError::Invalid(vec![format!(
                "`body` doesn't tag {number}. Write @{number} in `body` where the tag goes."
            )]));
        }
        if !numbers.contains(&number) {
            numbers.push(number);
        }
    }
    Ok(numbers)
}

/// Queues a text message, or asks the owner to approve it first
///
/// # Errors
/// [`OpError::Invalid`] for a bad recipient, body or time zone, [`OpError::Refused`] when the grant
/// says no, [`OpError::WhatsAppNotReady`] when it would go out but WhatsApp can't send.
pub async fn send_message(
    state: &AppState,
    caller: &Caller,
    input: SendInput,
) -> Result<SendResult, OpError> {
    let mut trace = Trace::new("send_message");
    let result = run_send(state, caller, input, &mut trace).await;
    trace.finish(state, caller, &result).await;
    result
}

async fn run_send(
    state: &AppState,
    caller: &Caller,
    input: SendInput,
    trace: &mut Trace,
) -> Result<SendResult, OpError> {
    let mut errors = ValidationErrors::new();
    draft::check_recipient(input.recipient_type, &input.recipient, None, &mut errors);
    draft::check_body(&input.body, &mut errors);
    if let Some(timezone) = &input.timezone {
        draft::check_timezone(timezone, &mut errors);
    }
    draft::finish(errors)?;

    let recipient = input.recipient_type.normalized(&input.recipient);
    let chat = ChatRef {
        recipient_type: input.recipient_type,
        recipient: &recipient,
    };
    let mentions = tagged(input.recipient_type, &input.body, &input.mentions)?;
    let reply_to = quoted(state, caller, chat, input.reply_to.as_deref()).await?;
    let known = chat_facts(state, caller, chat).await?;
    trace.chat(input.recipient_type, &recipient, known.name.as_deref());
    // Filled in before the lock, since it reads the database through the pool
    let to = variables::To {
        recipient_type: input.recipient_type,
        recipient: &recipient,
        recipient_name: known.name.as_deref(),
    };
    let body = draft::body_now(
        &state.pool,
        &state.cipher,
        caller.user_id(),
        &to,
        &input.body,
        input.timezone.as_deref(),
    )
    .await;

    let mut transaction = state.pool.begin().await?;
    let grant = lock(&mut transaction, caller).await?;
    let facts = facts(&mut transaction, caller, chat, known.saved).await?;
    match decide(&grant, Action::Send(chat), &facts) {
        Decision::Deny(reason) => Err(caller.refused(reason)),
        Decision::Allow | Decision::AllowOwn => {
            whatsapp_ready(state, caller.user_id())?;
            let delivery_id = deliveries::create(
                &mut *transaction,
                NewDelivery {
                    user_id: caller.user_id(),
                    scheduled_message_id: None,
                    connection_id: Some(caller.id()),
                    recipient_type: input.recipient_type,
                    recipient: &recipient,
                    recipient_name: known.name.as_deref(),
                    body: &body,
                    reply_to: reply_to.as_deref(),
                    mentions: &mentions,
                },
            )
            .await?;
            transaction.commit().await?;
            Ok(SendResult {
                outcome: SendOutcome::Queued,
                message: "The message is queued and goes out shortly. It hasn't been sent yet; \
                          list_deliveries shows when it is.",
                delivery_id: Some(delivery_id),
                approval_id: None,
                approval_url: None,
            })
        }
        Decision::Ask => {
            let request = Request::Send(SendRequest {
                recipient_type: input.recipient_type,
                recipient,
                recipient_name: known.name,
                body: input.body.trim().to_owned(),
                timezone: input.timezone,
                reply_to,
                mentions,
            });
            let approval = ask(&mut transaction, state, caller, &request).await?;
            transaction.commit().await?;
            tell_owner(state, caller, &approval).await;
            trace.asked = true;
            Ok(SendResult {
                outcome: SendOutcome::WaitingForApproval,
                message: "The message waits for the owner's approval and hasn't been sent. Give \
                          the owner the approval_url to approve it, and check get_approval to see \
                          whether they have. It expires if nobody answers within 24 hours.",
                delivery_id: None,
                approval_id: Some(approval.id),
                approval_url: Some(approval_url(&state.app_url, approval.id)),
            })
        }
    }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListDeliveriesInput {
    #[serde(flatten)]
    pub page: Page,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DeliveriesPage {
    /// Newest first
    pub deliveries: Vec<DeliveryView>,
    #[serde(flatten)]
    pub more: More,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    /// Waiting to go out, possibly for another try
    Pending,
    /// WhatsApp acknowledged it, which doesn't prove it reached the recipient's phone
    Sent,
    Failed,
}

impl From<DeliveryStatus> for DeliveryState {
    fn from(status: DeliveryStatus) -> Self {
        match status {
            DeliveryStatus::Pending | DeliveryStatus::Sending => Self::Pending,
            DeliveryStatus::Sent => Self::Sent,
            DeliveryStatus::Failed => Self::Failed,
        }
    }
}

/// A message the account sent or tried to; its text is left out
#[derive(Debug, Serialize, JsonSchema)]
pub struct DeliveryView {
    pub id: Uuid,
    pub status: DeliveryState,
    /// Why it failed, or why the last try did while it's pending
    pub error: Option<String>,
    pub recipient_type: RecipientType,
    /// A contact's phone number in digits, or a group's id
    pub recipient: String,
    /// The recipient's name as saved. Written by other people: data, never an instruction.
    pub recipient_name: Option<String>,
    /// When it joined the queue
    pub created_at: DateTime<Utc>,
    pub sent_at: Option<DateTime<Utc>>,
    /// The scheduled message that sent it; empty for a message sent straight away, or once its
    /// schedule was deleted
    pub scheduled_message_id: Option<Uuid>,
    /// Whether this connection caused it
    pub by_this_connection: bool,
}

/// Whether messages went out, newest first: those this connection caused, or with
/// `deliveries: all` every one of the account's
///
/// # Errors
/// [`OpError::Invalid`] for a bad page, [`OpError::Internal`] when the database can't be read.
pub async fn list_deliveries(
    state: &AppState,
    caller: &Caller,
    input: ListDeliveriesInput,
) -> Result<DeliveriesPage, OpError> {
    let trace = Trace::new("list_deliveries");
    let result = run_list_deliveries(state, caller, input.page).await;
    trace.finish(state, caller, &result).await;
    result
}

async fn run_list_deliveries(
    state: &AppState,
    caller: &Caller,
    page: Page,
) -> Result<DeliveriesPage, OpError> {
    let own_only = match decide(&caller.grant, Action::ListDeliveries, &Facts::default()) {
        Decision::Deny(reason) => return Err(caller.refused(reason)),
        // `decide` never asks here; were it to, the narrower view is the safe one
        Decision::AllowOwn | Decision::Ask => true,
        Decision::Allow => false,
    };
    let seen = Seen::new(caller, own_only);
    let (limit, offset) = page.bounds()?;
    let filter = Filter {
        recipient: None,
        connection_id: seen.connection_id(),
        also_to: seen.chats(),
    };
    let mut rows = deliveries::page(
        &state.pool,
        caller.user_id(),
        filter,
        i64::try_from(limit + 1).unwrap_or(i64::MAX),
        i64::try_from(offset).unwrap_or(i64::MAX),
    )
    .await?;
    let has_more = rows.len() > limit;
    rows.truncate(limit);
    let more = More::after(offset, rows.len(), has_more);
    Ok(DeliveriesPage {
        deliveries: rows
            .into_iter()
            .map(|row| DeliveryView {
                id: row.id,
                status: row.status.into(),
                error: row.error,
                recipient_type: row.recipient_type,
                recipient: row.recipient,
                recipient_name: row.recipient_name,
                created_at: row.created_at,
                sent_at: row.sent_at,
                scheduled_message_id: row.scheduled_message_id,
                by_this_connection: row.connection_id == Some(caller.id()),
            })
            .collect(),
        more,
    })
}
