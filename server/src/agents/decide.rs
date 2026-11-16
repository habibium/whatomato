//! Whether a connection may do something. [`decide`] is pure: the caller gathers the [`Facts`]
//! from the database, and adds the owner's page to a refusal's message.

use super::grant::{
    Contacts, Deliveries, Grant, Schedules, Scope, SendLevel, Strangers, Templates,
};
use crate::scheduler::RecipientType;

/// Sends a connection may make in 24 hours, whatever its grant says
pub const DAILY_CEILING: u32 = 200;
/// Approvals a connection may have waiting at once
pub const WAITING_MAX: u32 = 20;
/// Scheduled messages a connection may have, counting its requests to create one still waiting
pub const SCHEDULES_MAX: u32 = 50;

/// A chat an action is aimed at, named as the scheduler names a recipient
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChatRef<'a> {
    pub recipient_type: RecipientType,
    /// As stored: digits for a contact, the group's id for a group
    pub recipient: &'a str,
}

/// Whose schedule an existing one is
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// This connection created it, or last changed it
    ThisConnection,
    /// The owner, or another connection
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action<'a> {
    Send(ChatRef<'a>),
    CreateSchedule(ChatRef<'a>),
    /// `chat` is where the schedule sends once changed
    UpdateSchedule {
        chat: ChatRef<'a>,
        owner: Owner,
    },
    DeleteSchedule {
        owner: Owner,
    },
    FindChats,
    ReadTemplates,
    ListSchedules,
    ListDeliveries,
    WhoAmI,
    ReadOwnApprovals,
    /// Reading one chat's messages
    ReadChat(ChatRef<'a>),
    /// Listing or searching the chats it may read
    ReadChats,
}

/// What the database knows that the decision needs. Only actions aimed at a chat read it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// The chat is a contact in the saved address book; ignored for a group
    pub saved_contact: bool,
    /// This connection's sends to the chat in the last 24 hours, its waiting approvals included
    pub chat_sends: u32,
    /// The same, across every chat
    pub connection_sends: u32,
    /// This connection's approvals still waiting
    pub waiting_approvals: u32,
    /// The scheduled messages this connection has, and its requests to create one still waiting
    pub schedules: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// Allowed, but only over what this connection created or caused
    AllowOwn,
    /// Allowed once the owner approves
    Ask,
    Deny(Reason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    SchedulesOff,
    NotOwnSchedule,
    ContactsOff,
    TemplatesOff,
    DailyCeiling,
    OutOfScope,
    StrangersNever,
    SendingOff,
    ChatLimit { limit: u32 },
    TooManyWaiting,
    TooManySchedules,
    ReadingOff,
}

impl Reason {
    /// A stable name, for the activity log
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::SchedulesOff => "schedules_off",
            Self::NotOwnSchedule => "not_own_schedule",
            Self::ContactsOff => "contacts_off",
            Self::TemplatesOff => "templates_off",
            Self::DailyCeiling => "daily_ceiling",
            Self::OutOfScope => "out_of_scope",
            Self::StrangersNever => "strangers_never",
            Self::SendingOff => "sending_off",
            Self::ChatLimit { .. } => "chat_limit",
            Self::TooManyWaiting => "too_many_waiting",
            Self::TooManySchedules => "too_many_schedules",
            Self::ReadingOff => "reading_off",
        }
    }

    /// For the agent to relay: what is missing, and what would change it. The caller adds the
    /// page where the owner changes it.
    #[must_use]
    pub fn message(self) -> String {
        match self {
            Self::SchedulesOff => {
                "This connection isn't allowed to see or manage scheduled messages. The owner can \
                 allow it in this connection's permissions."
                    .to_owned()
            }
            Self::NotOwnSchedule => {
                "This scheduled message doesn't belong to this connection, and it may only change \
                 its own. The owner can allow it to manage all scheduled messages."
                    .to_owned()
            }
            Self::ContactsOff => {
                "This connection isn't allowed to look up contacts and groups. The owner can allow \
                 it in this connection's permissions."
                    .to_owned()
            }
            Self::TemplatesOff => {
                "This connection isn't allowed to read message templates. The owner can allow it \
                 in this connection's permissions."
                    .to_owned()
            }
            Self::DailyCeiling => format!(
                "This connection has sent {DAILY_CEILING} messages in the last 24 hours, the most \
                 any connection may send. Try again later."
            ),
            Self::OutOfScope => {
                "This chat isn't one of the chats this connection may use. The owner can add it to \
                 this connection's chats."
                    .to_owned()
            }
            Self::StrangersNever => {
                "This number isn't a saved contact, and this connection may not message people who \
                 aren't. The owner can save the contact, give this chat a rule of its own, or allow \
                 messages to people who aren't saved contacts."
                    .to_owned()
            }
            Self::SendingOff => {
                "This connection isn't allowed to send messages to this chat. The owner can change \
                 that in this connection's chat rules."
                    .to_owned()
            }
            Self::ChatLimit { limit } => format!(
                "This connection has reached its limit of {limit} messages a day to this chat. Try \
                 again later, or ask the owner to raise the limit."
            ),
            Self::TooManyWaiting => format!(
                "{WAITING_MAX} messages from this connection are already waiting for the owner's \
                 approval. Wait until the owner has answered some of them."
            ),
            Self::TooManySchedules => format!(
                "This connection already has {SCHEDULES_MAX} scheduled messages, the most one \
                 connection may have, counting those waiting for the owner's approval. Delete some \
                 of them first, or ask the owner to."
            ),
            Self::ReadingOff => {
                "This connection isn't allowed to read messages here. The owner can turn reading \
                 on in this connection's chat rules."
                    .to_owned()
            }
        }
    }
}

/// Whether `grant` lets the connection take `action`, given `facts`
#[must_use]
pub fn decide(grant: &Grant, action: Action<'_>, facts: &Facts) -> Decision {
    match action {
        Action::Send(chat) => chat_action(grant, chat, facts),
        Action::CreateSchedule(chat) => match grant.schedules {
            Schedules::None => Decision::Deny(Reason::SchedulesOff),
            Schedules::Own | Schedules::All if facts.schedules >= SCHEDULES_MAX => {
                Decision::Deny(Reason::TooManySchedules)
            }
            Schedules::Own | Schedules::All => chat_action(grant, chat, facts),
        },
        Action::UpdateSchedule { chat, owner } => match may_change_schedule(grant, owner) {
            // Only a change made outright takes another's schedule over; an approved one keeps its
            // owner, so asking adds nothing to this connection's count
            Ok(()) => match chat_action(grant, chat, facts) {
                Decision::Allow if owner == Owner::Other && facts.schedules >= SCHEDULES_MAX => {
                    Decision::Deny(Reason::TooManySchedules)
                }
                decision => decision,
            },
            Err(reason) => Decision::Deny(reason),
        },
        Action::DeleteSchedule { owner } => match may_change_schedule(grant, owner) {
            Ok(()) => Decision::Allow,
            Err(reason) => Decision::Deny(reason),
        },
        Action::FindChats => match grant.contacts {
            Contacts::Find => Decision::Allow,
            Contacts::None => Decision::Deny(Reason::ContactsOff),
        },
        Action::ReadTemplates => match grant.templates {
            Templates::Read => Decision::Allow,
            Templates::None => Decision::Deny(Reason::TemplatesOff),
        },
        Action::ListSchedules => match grant.schedules {
            Schedules::None => Decision::Deny(Reason::SchedulesOff),
            Schedules::Own => Decision::AllowOwn,
            Schedules::All => Decision::Allow,
        },
        Action::ListDeliveries => match grant.deliveries {
            Deliveries::Own => Decision::AllowOwn,
            Deliveries::All => Decision::Allow,
        },
        Action::WhoAmI | Action::ReadOwnApprovals => Decision::Allow,
        Action::ReadChat(chat) => {
            let own = grant.rule_for(chat.recipient_type, chat.recipient);
            if own.is_none() && grant.chats.scope == Scope::Selected {
                Decision::Deny(Reason::OutOfScope)
            } else if grant.reads(chat.recipient_type, chat.recipient) {
                Decision::Allow
            } else {
                Decision::Deny(Reason::ReadingOff)
            }
        }
        Action::ReadChats if grant.reads_any() => Decision::Allow,
        Action::ReadChats => Decision::Deny(Reason::ReadingOff),
    }
}

const fn may_change_schedule(grant: &Grant, owner: Owner) -> Result<(), Reason> {
    match (grant.schedules, owner) {
        (Schedules::None, _) => Err(Reason::SchedulesOff),
        (Schedules::Own, Owner::Other) => Err(Reason::NotOwnSchedule),
        (Schedules::Own, Owner::ThisConnection) | (Schedules::All, _) => Ok(()),
    }
}

/// The ceiling, the chat's rule, a rule of `no`, strangers, the chat's limit and the approval
/// queue, in that order. A rule of `no` comes before strangers: saving the contact wouldn't help.
fn chat_action(grant: &Grant, chat: ChatRef<'_>, facts: &Facts) -> Decision {
    if facts.connection_sends >= DAILY_CEILING {
        return Decision::Deny(Reason::DailyCeiling);
    }

    let own = grant.rule_for(chat.recipient_type, chat.recipient);
    let (send, daily_limit) = match (own, grant.chats.scope) {
        (Some(rule), _) => (rule.send, rule.daily_limit),
        (None, Scope::All) => (grant.chats.default.send, grant.chats.default.daily_limit),
        (None, Scope::Selected) => return Decision::Deny(Reason::OutOfScope),
    };
    if send == SendLevel::No {
        return Decision::Deny(Reason::SendingOff);
    }

    let stranger =
        chat.recipient_type == RecipientType::Contact && !facts.saved_contact && own.is_none();
    let send = match (stranger, grant.chats.strangers, send) {
        (true, Strangers::Never, _) => return Decision::Deny(Reason::StrangersNever),
        (true, Strangers::Ask, SendLevel::Yes) => SendLevel::Ask,
        _ => send,
    };

    if let Some(limit) = daily_limit
        && facts.chat_sends >= limit
    {
        return Decision::Deny(Reason::ChatLimit { limit });
    }
    match send {
        SendLevel::Yes => Decision::Allow,
        SendLevel::Ask if facts.waiting_approvals >= WAITING_MAX => {
            Decision::Deny(Reason::TooManyWaiting)
        }
        SendLevel::Ask => Decision::Ask,
        // Returned above; kept as a refusal so a later edit can't turn "no" into "ask"
        SendLevel::No => Decision::Deny(Reason::SendingOff),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::grant::{Chat, ChatRule, Preset, Rule};

    const SAVED: &str = "8801700000001";
    const UNSAVED: &str = "8801700000002";
    const RULED: &str = "8801700000003";
    const GROUP: &str = "120363000000000000@g.us";

    fn contact(recipient: &str) -> ChatRef<'_> {
        ChatRef {
            recipient_type: RecipientType::Contact,
            recipient,
        }
    }

    fn group() -> ChatRef<'static> {
        ChatRef {
            recipient_type: RecipientType::Group,
            recipient: GROUP,
        }
    }

    fn saved() -> Facts {
        Facts {
            saved_contact: true,
            ..Facts::default()
        }
    }

    fn grant(scope: Scope, send: SendLevel, strangers: Strangers) -> Grant {
        let mut grant = Preset::AskBeforeSending.grant();
        grant.chats.scope = scope;
        grant.chats.default = Rule {
            read: false,
            send,
            daily_limit: None,
            note: None,
        };
        grant.chats.strangers = strangers;
        grant
    }

    fn with_rule(mut grant: Grant, id: &str, send: SendLevel, daily_limit: Option<u32>) -> Grant {
        let recipient_type = if id.ends_with("@g.us") {
            RecipientType::Group
        } else {
            RecipientType::Contact
        };
        grant.chats.rules.push(ChatRule {
            chat: Chat {
                recipient_type,
                id: id.to_owned(),
                name: None,
            },
            read: false,
            send,
            daily_limit,
            note: None,
        });
        grant
    }

    fn deny(reason: Reason) -> Decision {
        Decision::Deny(reason)
    }

    #[test]
    fn each_send_level_under_scope_all() {
        let cases = [
            (SendLevel::No, deny(Reason::SendingOff)),
            (SendLevel::Ask, Decision::Ask),
            (SendLevel::Yes, Decision::Allow),
        ];
        for (send, expected) in cases {
            let grant = grant(Scope::All, send, Strangers::Allowed);
            for chat in [contact(SAVED), group()] {
                assert_eq!(
                    decide(&grant, Action::Send(chat), &saved()),
                    expected,
                    "{send:?} {chat:?}"
                );
            }
        }
    }

    #[test]
    fn scope_selected_reaches_only_the_chats_with_rules() {
        for send in [SendLevel::No, SendLevel::Ask, SendLevel::Yes] {
            let selected = with_rule(
                grant(Scope::Selected, SendLevel::Yes, Strangers::Allowed),
                RULED,
                send,
                None,
            );
            let expected = match send {
                SendLevel::No => deny(Reason::SendingOff),
                SendLevel::Ask => Decision::Ask,
                SendLevel::Yes => Decision::Allow,
            };
            assert_eq!(
                decide(&selected, Action::Send(contact(RULED)), &saved()),
                expected
            );
            assert_eq!(
                decide(&selected, Action::Send(contact(SAVED)), &saved()),
                deny(Reason::OutOfScope)
            );
            assert_eq!(
                decide(&selected, Action::Send(group()), &saved()),
                deny(Reason::OutOfScope)
            );
        }
    }

    #[test]
    fn a_chat_rule_overrides_the_default_both_ways() {
        let loosened = with_rule(
            grant(Scope::All, SendLevel::No, Strangers::Allowed),
            RULED,
            SendLevel::Yes,
            None,
        );
        assert_eq!(
            decide(&loosened, Action::Send(contact(RULED)), &saved()),
            Decision::Allow
        );
        assert_eq!(
            decide(&loosened, Action::Send(contact(SAVED)), &saved()),
            deny(Reason::SendingOff)
        );

        let blocked = with_rule(
            grant(Scope::All, SendLevel::Yes, Strangers::Allowed),
            GROUP,
            SendLevel::No,
            None,
        );
        assert_eq!(
            decide(&blocked, Action::Send(group()), &saved()),
            deny(Reason::SendingOff)
        );
        assert_eq!(
            decide(&blocked, Action::Send(contact(SAVED)), &saved()),
            Decision::Allow
        );
    }

    #[test]
    fn strangers_against_saved_and_unsaved_contacts_and_groups() {
        let unsaved = Facts::default();
        // (strangers, default send, saved contact, unsaved contact, group)
        let cases = [
            (
                Strangers::Never,
                SendLevel::Yes,
                Decision::Allow,
                deny(Reason::StrangersNever),
                Decision::Allow,
            ),
            (
                Strangers::Ask,
                SendLevel::Yes,
                Decision::Allow,
                Decision::Ask,
                Decision::Allow,
            ),
            (
                Strangers::Allowed,
                SendLevel::Yes,
                Decision::Allow,
                Decision::Allow,
                Decision::Allow,
            ),
            (
                Strangers::Ask,
                SendLevel::Ask,
                Decision::Ask,
                Decision::Ask,
                Decision::Ask,
            ),
            (
                Strangers::Allowed,
                SendLevel::No,
                deny(Reason::SendingOff),
                deny(Reason::SendingOff),
                deny(Reason::SendingOff),
            ),
            // A rule of "no" is the reason, whatever the stranger rule says
            (
                Strangers::Never,
                SendLevel::No,
                deny(Reason::SendingOff),
                deny(Reason::SendingOff),
                deny(Reason::SendingOff),
            ),
            (
                Strangers::Ask,
                SendLevel::No,
                deny(Reason::SendingOff),
                deny(Reason::SendingOff),
                deny(Reason::SendingOff),
            ),
        ];
        for (strangers, send, on_saved, on_unsaved, on_group) in cases {
            let grant = grant(Scope::All, send, strangers);
            let label = format!("{strangers:?} {send:?}");
            assert_eq!(
                decide(&grant, Action::Send(contact(SAVED)), &saved()),
                on_saved,
                "{label}"
            );
            assert_eq!(
                decide(&grant, Action::Send(contact(UNSAVED)), &unsaved),
                on_unsaved,
                "{label}"
            );
            // A group is never a stranger, saved or not
            assert_eq!(
                decide(&grant, Action::Send(group()), &unsaved),
                on_group,
                "{label}"
            );
        }
    }

    #[test]
    fn a_stranger_with_a_rule_of_their_own_follows_it() {
        let unsaved = Facts::default();
        for strangers in [Strangers::Never, Strangers::Ask] {
            let grant = with_rule(
                grant(Scope::All, SendLevel::Ask, strangers),
                UNSAVED,
                SendLevel::Yes,
                None,
            );
            assert_eq!(
                decide(&grant, Action::Send(contact(UNSAVED)), &unsaved),
                Decision::Allow,
                "{strangers:?}"
            );
        }
    }

    #[test]
    fn the_daily_ceiling_holds_at_200_whatever_the_grant() {
        let full = Preset::FullAccess.grant();
        let at = |connection_sends| Facts {
            connection_sends,
            ..saved()
        };
        assert_eq!(
            decide(&full, Action::Send(group()), &at(199)),
            Decision::Allow
        );
        assert_eq!(
            decide(&full, Action::Send(group()), &at(200)),
            deny(Reason::DailyCeiling)
        );
        assert_eq!(
            decide(&full, Action::Send(group()), &at(201)),
            deny(Reason::DailyCeiling)
        );
        assert_eq!(
            decide(&full, Action::CreateSchedule(group()), &at(200)),
            deny(Reason::DailyCeiling)
        );
    }

    #[test]
    fn a_chat_limit_refuses_at_the_limit_for_yes_and_ask() {
        for send in [SendLevel::Yes, SendLevel::Ask] {
            let grant = with_rule(
                grant(Scope::All, SendLevel::Yes, Strangers::Allowed),
                RULED,
                send,
                Some(5),
            );
            let at = |chat_sends| Facts {
                chat_sends,
                connection_sends: chat_sends,
                ..saved()
            };
            let below = if send == SendLevel::Yes {
                Decision::Allow
            } else {
                Decision::Ask
            };
            let chat = Action::Send(contact(RULED));
            assert_eq!(decide(&grant, chat, &at(4)), below, "{send:?}");
            assert_eq!(
                decide(&grant, chat, &at(5)),
                deny(Reason::ChatLimit { limit: 5 })
            );
            assert_eq!(
                decide(&grant, chat, &at(6)),
                deny(Reason::ChatLimit { limit: 5 })
            );
        }
    }

    #[test]
    fn the_default_rules_limit_applies_to_each_chat() {
        let grant = Preset::SendWithoutAsking.grant();
        let at = |chat_sends| Facts {
            chat_sends,
            ..saved()
        };
        assert_eq!(
            decide(&grant, Action::Send(group()), &at(19)),
            Decision::Allow
        );
        assert_eq!(
            decide(&grant, Action::Send(group()), &at(20)),
            deny(Reason::ChatLimit { limit: 20 })
        );
    }

    #[test]
    fn an_ask_is_refused_once_20_approvals_wait() {
        let grant = grant(Scope::All, SendLevel::Ask, Strangers::Ask);
        let at = |waiting_approvals| Facts {
            waiting_approvals,
            ..saved()
        };
        assert_eq!(
            decide(&grant, Action::Send(group()), &at(19)),
            Decision::Ask
        );
        assert_eq!(
            decide(&grant, Action::Send(group()), &at(20)),
            deny(Reason::TooManyWaiting)
        );
        assert_eq!(
            decide(&grant, Action::Send(group()), &at(21)),
            deny(Reason::TooManyWaiting)
        );

        // A send that needs no approval doesn't wait in the queue
        let free = Preset::FullAccess.grant();
        assert_eq!(
            decide(&free, Action::Send(group()), &at(20)),
            Decision::Allow
        );
    }

    #[test]
    fn a_connection_holds_at_most_50_schedules_whatever_the_grant() {
        let at = |schedules| Facts {
            schedules,
            ..saved()
        };
        let create = Action::CreateSchedule(group());
        let take_over = Action::UpdateSchedule {
            chat: group(),
            owner: Owner::Other,
        };
        let own = Action::UpdateSchedule {
            chat: group(),
            owner: Owner::ThisConnection,
        };
        let full = Preset::FullAccess.grant();
        let asking = grant(Scope::All, SendLevel::Ask, Strangers::Ask);
        for (grant, below) in [(&full, Decision::Allow), (&asking, Decision::Ask)] {
            assert_eq!(decide(grant, create, &at(49)), below);
            assert_eq!(
                decide(grant, create, &at(50)),
                deny(Reason::TooManySchedules)
            );
            // Changing one it already has adds none
            assert_eq!(decide(grant, own, &at(50)), below);
        }
        assert_eq!(decide(&full, take_over, &at(49)), Decision::Allow);
        assert_eq!(
            decide(&full, take_over, &at(50)),
            deny(Reason::TooManySchedules)
        );

        // Asking to take one over adds none: an approved change keeps its owner
        let mut asking_all = asking;
        asking_all.schedules = Schedules::All;
        assert_eq!(decide(&asking_all, take_over, &at(50)), Decision::Ask);
        // A refusal for another reason keeps that reason
        let mut blocked = grant(Scope::All, SendLevel::No, Strangers::Allowed);
        blocked.schedules = Schedules::All;
        assert_eq!(
            decide(&blocked, take_over, &at(50)),
            deny(Reason::SendingOff)
        );
    }

    #[test]
    fn schedule_levels_against_own_and_foreign_schedules() {
        let level = |schedules| {
            let mut grant = grant(Scope::All, SendLevel::Yes, Strangers::Allowed);
            grant.schedules = schedules;
            grant
        };
        let update = |owner| Action::UpdateSchedule {
            chat: group(),
            owner,
        };
        let delete = |owner| Action::DeleteSchedule { owner };
        let off = deny(Reason::SchedulesOff);
        let not_own = deny(Reason::NotOwnSchedule);
        // (level, create, list, update own, update other, delete own, delete other)
        let cases = [
            (
                Schedules::None,
                off.clone(),
                off.clone(),
                off.clone(),
                off.clone(),
                off.clone(),
                off,
            ),
            (
                Schedules::Own,
                Decision::Allow,
                Decision::AllowOwn,
                Decision::Allow,
                not_own.clone(),
                Decision::Allow,
                not_own,
            ),
            (
                Schedules::All,
                Decision::Allow,
                Decision::Allow,
                Decision::Allow,
                Decision::Allow,
                Decision::Allow,
                Decision::Allow,
            ),
        ];
        for (schedules, create, list, own, other, delete_own, delete_other) in cases {
            let grant = level(schedules);
            let facts = saved();
            let label = format!("{schedules:?}");
            assert_eq!(
                decide(&grant, Action::CreateSchedule(group()), &facts),
                create,
                "{label}"
            );
            assert_eq!(
                decide(&grant, Action::ListSchedules, &facts),
                list,
                "{label}"
            );
            assert_eq!(
                decide(&grant, update(Owner::ThisConnection), &facts),
                own,
                "{label}"
            );
            assert_eq!(
                decide(&grant, update(Owner::Other), &facts),
                other,
                "{label}"
            );
            assert_eq!(
                decide(&grant, delete(Owner::ThisConnection), &facts),
                delete_own,
                "{label}"
            );
            assert_eq!(
                decide(&grant, delete(Owner::Other), &facts),
                delete_other,
                "{label}"
            );
        }
    }

    #[test]
    fn a_schedule_still_follows_the_chat_rules() {
        let grant = grant(Scope::All, SendLevel::Ask, Strangers::Never);
        assert_eq!(
            decide(&grant, Action::CreateSchedule(group()), &saved()),
            Decision::Ask
        );
        assert_eq!(
            decide(
                &grant,
                Action::UpdateSchedule {
                    chat: contact(UNSAVED),
                    owner: Owner::ThisConnection
                },
                &Facts::default()
            ),
            deny(Reason::StrangersNever)
        );
    }

    #[test]
    fn actions_without_a_chat() {
        let mut closed = Preset::ViewOnly.grant();
        closed.contacts = Contacts::None;
        closed.templates = Templates::None;
        let open = Preset::FullAccess.grant();
        let facts = Facts::default();

        assert_eq!(decide(&open, Action::FindChats, &facts), Decision::Allow);
        assert_eq!(
            decide(&closed, Action::FindChats, &facts),
            deny(Reason::ContactsOff)
        );
        assert_eq!(
            decide(&open, Action::ReadTemplates, &facts),
            Decision::Allow
        );
        assert_eq!(
            decide(&closed, Action::ReadTemplates, &facts),
            deny(Reason::TemplatesOff)
        );
        assert_eq!(
            decide(&open, Action::ListDeliveries, &facts),
            Decision::Allow
        );
        assert_eq!(
            decide(&closed, Action::ListDeliveries, &facts),
            Decision::AllowOwn
        );
        for grant in [&open, &closed] {
            assert_eq!(decide(grant, Action::WhoAmI, &facts), Decision::Allow);
            assert_eq!(
                decide(grant, Action::ReadOwnApprovals, &facts),
                Decision::Allow
            );
        }
    }

    #[test]
    fn every_reason_says_something() {
        let reasons = [
            Reason::SchedulesOff,
            Reason::NotOwnSchedule,
            Reason::ContactsOff,
            Reason::TemplatesOff,
            Reason::DailyCeiling,
            Reason::OutOfScope,
            Reason::StrangersNever,
            Reason::SendingOff,
            Reason::ChatLimit { limit: 7 },
            Reason::TooManyWaiting,
            Reason::TooManySchedules,
        ];
        for reason in reasons {
            assert!(reason.message().ends_with('.'), "{reason:?}");
        }
        assert!(
            Reason::ChatLimit { limit: 7 }
                .message()
                .contains("limit of 7 messages")
        );
    }
}
