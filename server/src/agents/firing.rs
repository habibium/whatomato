//! Whether an agent's schedule may still send when it comes due. Creating it was decided once;
//! each run is decided again, so a pause, a narrowed grant or a used-up limit stops it.

use sqlx::PgConnection;
use uuid::Uuid;

use super::{
    Grant,
    decide::{Action, ChatRef, DAILY_CEILING, Decision, Facts, Reason, decide},
};
use crate::{
    db::{agent_connections, whatsapp_contacts},
    scheduler::RecipientType,
};

#[derive(Debug, PartialEq, Eq)]
pub enum Firing {
    /// Send it, carrying `connection_id`, which is empty once the connection is gone
    Send { connection_id: Option<Uuid> },
    /// Record the run as skipped, with this sentence for the owner
    Skip(String),
    /// A call of the connection holds its lock; try again at the next tick
    Later,
}

/// Decides a run of a schedule `connection_id` created, aimed at `chat`. Takes the connection's
/// lock, without waiting for it, until `db`'s transaction ends.
///
/// # Errors
/// When the database can't be read.
pub async fn check(
    db: &mut PgConnection,
    agent_access: bool,
    user_id: Uuid,
    connection_id: Uuid,
    chat: ChatRef<'_>,
) -> sqlx::Result<Firing> {
    if !agent_access {
        return Ok(skip(
            "Not sent: an agent scheduled this, and agent access is switched off on this server.",
        ));
    }
    // Not waiting: a call holding the lock may be waiting for this very schedule's row
    let Some(locked) = agent_connections::try_lock(&mut *db, connection_id).await? else {
        return Ok(
            if agent_connections::exists(&mut *db, connection_id).await? {
                Firing::Later
            } else {
                Firing::Send {
                    connection_id: None,
                }
            },
        );
    };
    if locked.paused {
        return Ok(skip("Not sent: the agent that scheduled this is paused."));
    }
    if locked.expired {
        return Ok(skip("Not sent: the agent that scheduled this has expired."));
    }
    let Ok(grant) = Grant::parse(&locked.grant) else {
        return Ok(skip(
            "Not sent: the permissions of the agent that scheduled this couldn't be read. Save \
             its permissions again to fix this.",
        ));
    };
    let saved_contact = match chat.recipient_type {
        RecipientType::Contact => {
            whatsapp_contacts::find_by_number(&mut *db, user_id, chat.recipient)
                .await?
                .is_some()
        }
        RecipientType::Group => false,
    };
    let counts =
        agent_connections::counts(&mut *db, connection_id, chat.recipient_type, chat.recipient)
            .await?;
    let facts = Facts {
        saved_contact,
        chat_sends: counts.chat_sends,
        connection_sends: counts.connection_sends,
        // A run never asks, so the approval queue has no say
        waiting_approvals: 0,
    };
    Ok(match decide(&grant, Action::Send(chat), &facts) {
        // The schedule was approved or allowed when it was made; a rule of `ask` doesn't ask again
        Decision::Allow | Decision::AllowOwn | Decision::Ask => Firing::Send {
            connection_id: Some(connection_id),
        },
        Decision::Deny(reason) => Firing::Skip(skipped_because(reason)),
    })
}

fn skip(message: &str) -> Firing {
    Firing::Skip(message.to_owned())
}

fn skipped_because(reason: Reason) -> String {
    match reason {
        Reason::DailyCeiling => format!(
            "Not sent: the agent that scheduled this has sent {DAILY_CEILING} messages in the \
             last 24 hours, the most any agent may."
        ),
        Reason::ChatLimit { limit } => format!(
            "Not sent: the agent that scheduled this has reached its limit of {limit} messages a \
             day to this chat."
        ),
        Reason::OutOfScope => "Not sent: this chat is no longer one of the chats the agent that \
                               scheduled this may use."
            .to_owned(),
        Reason::StrangersNever => "Not sent: this number isn't a saved contact, and the agent \
                                   that scheduled this may not message people who aren't."
            .to_owned(),
        Reason::SendingOff
        | Reason::SchedulesOff
        | Reason::NotOwnSchedule
        | Reason::ContactsOff
        | Reason::TemplatesOff
        | Reason::TooManyWaiting => "Not sent: the agent that scheduled this is no longer \
                                     allowed to send to this chat."
            .to_owned(),
    }
}
