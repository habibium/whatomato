//! What a connection may do, stored as versioned JSON on the connection as [`Recurrence`] is.
//!
//! A grant that can't be read or doesn't pass [`Grant::check`] gives no access at all: callers get
//! a [`GrantError`], never a fallback grant.
//!
//! [`Recurrence`]: crate::scheduler::recurrence::Recurrence

use std::collections::HashSet;

use chrono::{DateTime, Months, TimeDelta, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::decide::DAILY_CEILING;
use crate::scheduler::RecipientType;

pub const VERSION: u32 = 1;
pub const RULES_MAX: usize = 500;
pub const NOTE_MAX: usize = 500;
/// The same bound as a scheduled message's recipient name
pub const CHAT_NAME_MAX: usize = 255;
/// How far back a connection reads when its grant doesn't say: the inbox's window before it
/// could keep more
pub const REACH_DEFAULT_DAYS: u32 = 7;
pub const REACH_DAYS_MAX: u32 = 3650;
pub const REACH_MONTHS_MAX: u32 = 120;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    /// Always 1 for now
    pub version: u32,
    pub contacts: Contacts,
    pub schedules: Schedules,
    pub templates: Templates,
    pub deliveries: Deliveries,
    pub chats: Chats,
    /// Pictures and files. Left out of the stored grant while both are off.
    #[serde(default, skip_serializing_if = "Attachments::is_off")]
    pub attachments: Attachments,
}

/// What a connection may do with pictures and files, apart from text
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct Attachments {
    /// Open the pictures and files of the chats it may read
    #[serde(default)]
    pub open: bool,
    /// Send pictures and files where it may send
    #[serde(default)]
    pub send: bool,
}

impl Attachments {
    #[allow(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    const fn is_off(&self) -> bool {
        !self.open && !self.send
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Contacts {
    None,
    /// Search contacts and groups by name
    Find,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Schedules {
    None,
    /// Only the schedules this connection created or changed
    Own,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Templates {
    None,
    Read,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Deliveries {
    /// Only the sends this connection caused
    Own,
    All,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Chats {
    pub scope: Scope,
    /// Every chat's rule under scope `all`; under `selected`, only what a newly added chat starts with
    pub default: Rule,
    /// A chat's own rule, replacing `default`. Under scope `selected`, the chats in scope.
    pub rules: Vec<ChatRule>,
    /// People who aren't saved contacts and have no rule of their own
    pub strangers: Strangers,
    /// How old a message in a chat it may read can be. Left out of the stored grant at its
    /// default of 7 days, so grants that don't change it stay readable by older servers.
    #[serde(default, skip_serializing_if = "Reach::is_default")]
    pub reach: Reach,
}

/// How far back a connection may read
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Reach {
    Days(u32),
    /// Calendar months
    Months(u32),
    /// Everything Whatomato keeps
    All,
}

impl Default for Reach {
    fn default() -> Self {
        Self::Days(REACH_DEFAULT_DAYS)
    }
}

impl Reach {
    #[allow(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The oldest a message within reach may be, counted back from `now`; `None` when nothing is
    /// out of reach
    #[must_use]
    pub fn since(self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            Self::Days(days) => now.checked_sub_signed(TimeDelta::days(i64::from(days))),
            Self::Months(months) => now.checked_sub_months(Months::new(months)),
            Self::All => None,
        }
    }

    fn problem(self) -> Option<String> {
        match self {
            Self::Days(days) if !(1..=REACH_DAYS_MAX).contains(&days) => Some(format!(
                "How far back it may read must be between 1 and {REACH_DAYS_MAX} days."
            )),
            Self::Months(months) if !(1..=REACH_MONTHS_MAX).contains(&months) => Some(format!(
                "How far back it may read must be between 1 and {REACH_MONTHS_MAX} months."
            )),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    All,
    Selected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SendLevel {
    No,
    Ask,
    Yes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Strangers {
    Never,
    /// A send that would go out without asking waits for approval instead
    Ask,
    Allowed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// Whether the connection may read the chat's messages. Off unless the owner turns it on;
    /// left out of the stored grant while off, so a grant that reads nothing is unchanged.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub read: bool,
    pub send: SendLevel,
    /// Sends a day by this connection in one chat
    #[schema(required = true)]
    pub daily_limit: Option<u32>,
    /// Guidance the agent sees; not enforced
    #[schema(required = true)]
    pub note: Option<String>,
}

/// A rule for one chat. Its fields are spelled out rather than flattened from [`Rule`], since
/// serde can't refuse unknown fields through a flatten.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ChatRule {
    pub chat: Chat,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub read: bool,
    pub send: SendLevel,
    #[schema(required = true)]
    pub daily_limit: Option<u32>,
    #[schema(required = true)]
    pub note: Option<String>,
}

/// A chat named as the scheduler names a recipient
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Chat {
    #[serde(rename = "type")]
    pub recipient_type: RecipientType,
    /// As stored: digits for a contact, the group's id for a group
    pub id: String,
    /// Only a label for the owner
    #[serde(default)]
    #[schema(required = true)]
    pub name: Option<String>,
}

/// Why a grant gives no access
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GrantError {
    #[error("the permissions couldn't be read: {0}")]
    Unreadable(String),
    #[error("the permissions have problems: {}", .0.join(" "))]
    Invalid(Vec<String>),
}

impl Grant {
    /// Reads a stored or submitted grant and checks it, giving back its stored form
    ///
    /// # Errors
    /// [`GrantError::Unreadable`] when the JSON isn't a grant, [`GrantError::Invalid`] when it is
    /// but [`Grant::check`] finds problems.
    pub fn parse(json: &serde_json::Value) -> Result<Self, GrantError> {
        let grant =
            Self::deserialize(json).map_err(|error| GrantError::Unreadable(error.to_string()))?;
        grant.check().map_err(GrantError::Invalid)
    }

    /// The grant as it's stored, trimmed and with chat ids normalized, or every problem with it
    /// in sentences the owner can read
    ///
    /// # Errors
    /// The problems, when there are any.
    pub fn check(self) -> Result<Self, Vec<String>> {
        let grant = self.normalized();
        let mut problems = Vec::new();
        if grant.version != VERSION {
            problems.push(
                "These permissions were saved by a newer version of Whatomato and can't be read."
                    .to_owned(),
            );
        }
        if grant.chats.rules.len() > RULES_MAX {
            problems.push(format!(
                "There are {} chat rules; a connection can have at most {RULES_MAX}.",
                grant.chats.rules.len()
            ));
        }
        if let Some(problem) = grant.chats.reach.problem() {
            problems.push(problem);
        }
        check_rule(
            "The default rule",
            grant.chats.default.daily_limit,
            grant.chats.default.note.as_deref(),
            &mut problems,
        );
        let mut seen = HashSet::new();
        for rule in &grant.chats.rules {
            let chat = &rule.chat;
            let label = chat.label();
            if let Some(problem) = chat.recipient_type.problem(&chat.id) {
                problems.push(format!("The chat {label} {problem}."));
            }
            if chat
                .name
                .as_ref()
                .is_some_and(|name| name.chars().count() > CHAT_NAME_MAX)
            {
                problems.push(format!(
                    "The name of the chat {label} is longer than {CHAT_NAME_MAX} characters."
                ));
            }
            if !seen.insert((chat.recipient_type.as_str(), chat.id.as_str())) {
                problems.push(format!("The chat {label} has more than one rule."));
            }
            check_rule(
                &format!("The rule for {label}"),
                rule.daily_limit,
                rule.note.as_deref(),
                &mut problems,
            );
        }
        if problems.is_empty() {
            Ok(grant)
        } else {
            Err(problems)
        }
    }

    fn normalized(mut self) -> Self {
        self.chats.default.note = normalized_text(self.chats.default.note);
        for rule in &mut self.chats.rules {
            rule.chat.id = rule.chat.recipient_type.normalized(&rule.chat.id);
            rule.chat.name = normalized_text(rule.chat.name.take());
            rule.note = normalized_text(rule.note.take());
        }
        self
    }

    /// The rule written for this chat, if it has one of its own
    #[must_use]
    pub fn rule_for(&self, recipient_type: RecipientType, recipient: &str) -> Option<&ChatRule> {
        self.chats
            .rules
            .iter()
            .find(|rule| rule.chat.recipient_type == recipient_type && rule.chat.id == recipient)
    }

    /// Whether the connection may read this chat's messages
    #[must_use]
    pub fn reads(&self, recipient_type: RecipientType, recipient: &str) -> bool {
        self.rule_for(recipient_type, recipient).map_or(
            self.chats.scope == Scope::All && self.chats.default.read,
            |rule| rule.read,
        )
    }

    /// The same grant, reading no chat
    #[must_use]
    pub fn without_reading(mut self) -> Self {
        self.chats.reach = Reach::default();
        self.attachments.open = false;
        self.chats.default.read = false;
        for rule in &mut self.chats.rules {
            rule.read = false;
        }
        self
    }

    /// Whether at least one chat could be read
    #[must_use]
    pub fn reads_any(&self) -> bool {
        (self.chats.scope == Scope::All && self.chats.default.read)
            || self.chats.rules.iter().any(|rule| rule.read)
    }
}

impl Chat {
    /// How a problem names the chat: its name if it has one, else its id
    fn label(&self) -> String {
        format!("\"{}\"", self.name.as_deref().unwrap_or(&self.id))
    }
}

fn check_rule(
    what: &str,
    daily_limit: Option<u32>,
    note: Option<&str>,
    problems: &mut Vec<String>,
) {
    match daily_limit {
        Some(0) => problems.push(format!(
            "{what} has a daily limit of 0. To stop sending there, set it to not send instead."
        )),
        Some(limit) if limit > DAILY_CEILING => problems.push(format!(
            "{what} has a daily limit of {limit}, but a connection can send at most \
             {DAILY_CEILING} messages a day."
        )),
        _ => {}
    }
    if note.is_some_and(|note| note.chars().count() > NOTE_MAX) {
        problems.push(format!(
            "{what} has a note longer than {NOTE_MAX} characters."
        ));
    }
}

fn normalized_text(text: Option<String>) -> Option<String> {
    text.map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

/// A starting value for the editor. The stored grant doesn't remember which one it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    /// The default
    AskBeforeSending,
    ViewOnly,
    SendWithoutAsking,
    FullAccess,
}

impl Preset {
    pub const ALL: [Self; 4] = [
        Self::AskBeforeSending,
        Self::ViewOnly,
        Self::SendWithoutAsking,
        Self::FullAccess,
    ];

    /// The daily limit per chat that "Send without asking" sets
    pub const SEND_WITHOUT_ASKING_LIMIT: u32 = 20;

    #[must_use]
    pub const fn grant(self) -> Grant {
        let (send, daily_limit, strangers, schedules, deliveries) = match self {
            Self::AskBeforeSending => (
                SendLevel::Ask,
                None,
                Strangers::Ask,
                Schedules::Own,
                Deliveries::Own,
            ),
            Self::ViewOnly => (
                SendLevel::No,
                None,
                Strangers::Never,
                Schedules::None,
                Deliveries::Own,
            ),
            Self::SendWithoutAsking => (
                SendLevel::Yes,
                Some(Self::SEND_WITHOUT_ASKING_LIMIT),
                Strangers::Ask,
                Schedules::Own,
                Deliveries::Own,
            ),
            Self::FullAccess => (
                SendLevel::Yes,
                None,
                Strangers::Allowed,
                Schedules::All,
                Deliveries::All,
            ),
        };
        Grant {
            version: VERSION,
            contacts: Contacts::Find,
            schedules,
            templates: Templates::Read,
            deliveries,
            attachments: Attachments {
                open: false,
                send: false,
            },
            chats: Chats {
                scope: Scope::All,
                default: Rule {
                    read: false,
                    send,
                    daily_limit,
                    note: None,
                },
                rules: Vec::new(),
                strangers,
                reach: Reach::Days(REACH_DEFAULT_DAYS),
            },
        }
    }

    /// The preset `grant` equals, or `None` for a custom one. Reading and attachments are chosen
    /// on their own, so they don't make a preset custom.
    #[must_use]
    pub fn of(grant: &Grant) -> Option<Self> {
        let mut sending = grant.clone().without_reading();
        sending.attachments = Attachments::default();
        Self::ALL
            .into_iter()
            .find(|preset| preset.grant() == sending)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn rule(recipient_type: RecipientType, id: &str) -> ChatRule {
        ChatRule {
            chat: Chat {
                recipient_type,
                id: id.to_owned(),
                name: None,
            },
            read: false,
            send: SendLevel::Yes,
            daily_limit: None,
            note: None,
        }
    }

    fn with_rules(rules: Vec<ChatRule>) -> Grant {
        let mut grant = Preset::AskBeforeSending.grant();
        grant.chats.rules = rules;
        grant
    }

    #[test]
    fn attachments_are_off_and_left_out_until_allowed() {
        let grant = Preset::FullAccess.grant();
        assert_eq!(grant.attachments, Attachments::default());
        assert!(
            serde_json::to_value(&grant)
                .unwrap()
                .get("attachments")
                .is_none()
        );

        let mut allowed = grant;
        allowed.attachments.send = true;
        let stored = serde_json::to_value(&allowed).unwrap();
        assert_eq!(
            stored["attachments"],
            json!({ "open": false, "send": true })
        );
        assert!(Grant::parse(&stored).unwrap().attachments.send);
        // Chosen on their own, so the preset still shows
        assert_eq!(Preset::of(&allowed), Some(Preset::FullAccess));

        allowed.attachments.open = true;
        let without = allowed.without_reading();
        assert!(!without.attachments.open);
        assert!(without.attachments.send);
    }

    #[test]
    fn reach_defaults_to_a_week_and_is_left_out_while_it_does() {
        let grant = Preset::ViewOnly.grant();
        assert_eq!(grant.chats.reach, Reach::Days(7));
        let stored = serde_json::to_value(&grant).unwrap();
        assert!(stored["chats"].get("reach").is_none());

        let mut longer = grant;
        longer.chats.reach = Reach::Months(3);
        let stored = serde_json::to_value(&longer).unwrap();
        assert_eq!(stored["chats"]["reach"], json!({ "months": 3 }));
        assert_eq!(Grant::parse(&stored).unwrap().chats.reach, Reach::Months(3));

        longer.chats.reach = Reach::All;
        let stored = serde_json::to_value(&longer).unwrap();
        assert_eq!(stored["chats"]["reach"], json!("all"));
    }

    #[test]
    fn reach_counts_back_from_now() {
        let now = "2026-03-31T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        assert_eq!(
            Reach::Days(7).since(now),
            Some("2026-03-24T12:00:00Z".parse().unwrap())
        );
        // A month back from the 31st lands on the last day of a shorter month
        assert_eq!(
            Reach::Months(1).since(now),
            Some("2026-02-28T12:00:00Z".parse().unwrap())
        );
        assert_eq!(Reach::All.since(now), None);
    }

    #[test]
    fn refuses_a_reach_out_of_bounds() {
        for reach in [
            Reach::Days(0),
            Reach::Days(REACH_DAYS_MAX + 1),
            Reach::Months(0),
            Reach::Months(REACH_MONTHS_MAX + 1),
        ] {
            let mut grant = Preset::ViewOnly.grant();
            grant.chats.reach = reach;
            assert_eq!(grant.check().unwrap_err().len(), 1, "{reach:?}");
        }
        let mut grant = Preset::ViewOnly.grant();
        grant.chats.reach = Reach::Days(REACH_DAYS_MAX);
        assert!(grant.check().is_ok());
    }

    #[test]
    fn reach_doesnt_make_a_preset_custom() {
        let mut grant = Preset::FullAccess.grant();
        grant.chats.default.read = true;
        grant.chats.reach = Reach::All;
        assert_eq!(Preset::of(&grant), Some(Preset::FullAccess));
        assert_eq!(grant.without_reading().chats.reach, Reach::default());
    }

    #[test]
    fn reads_the_documented_shape() {
        let json = json!({
            "version": 1,
            "contacts": "find",
            "schedules": "own",
            "templates": "read",
            "deliveries": "own",
            "chats": {
                "scope": "all",
                "default": { "send": "ask", "daily_limit": null, "note": null },
                "rules": [{
                    "chat": { "type": "contact", "id": "8801700000000", "name": "Ada" },
                    "read": true,
                    "send": "yes",
                    "daily_limit": 5,
                    "note": "Reply in Bangla"
                }],
                "strangers": "ask"
            }
        });
        let grant = Grant::parse(&json).unwrap();
        assert_eq!(grant.chats.rules[0].daily_limit, Some(5));
        assert_eq!(grant.chats.rules[0].chat.name.as_deref(), Some("Ada"));
        assert_eq!(serde_json::to_value(&grant).unwrap(), json);
        assert!(grant.reads(RecipientType::Contact, "8801700000000"));
        assert!(!grant.reads(RecipientType::Contact, "8801700000001"));

        // A grant saved before reading existed reads nothing
        let mut older = json;
        older["chats"]["default"]
            .as_object_mut()
            .unwrap()
            .remove("read");
        older["chats"]["rules"][0]
            .as_object_mut()
            .unwrap()
            .remove("read");
        assert!(!Grant::parse(&older).unwrap().reads_any());
    }

    #[test]
    fn json_that_is_not_a_grant_is_unreadable() {
        for json in [
            json!(null),
            json!({}),
            json!({ "version": 1 }),
            json!({ "version": 1, "contacts": "everything", "schedules": "own", "templates": "read",
                    "deliveries": "own", "chats": { "scope": "all",
                    "default": { "send": "ask", "daily_limit": null, "note": null },
                    "rules": [], "strangers": "ask" } }),
        ] {
            assert!(
                matches!(Grant::parse(&json), Err(GrantError::Unreadable(_))),
                "{json}"
            );
        }

        let mut extra = serde_json::to_value(Preset::AskBeforeSending.grant()).unwrap();
        extra["read"] = json!("on");
        assert!(matches!(
            Grant::parse(&extra),
            Err(GrantError::Unreadable(_))
        ));
    }

    #[test]
    fn another_version_is_refused() {
        let mut json = serde_json::to_value(Preset::AskBeforeSending.grant()).unwrap();
        json["version"] = json!(2);
        assert!(matches!(Grant::parse(&json), Err(GrantError::Invalid(_))));
    }

    #[test]
    fn chat_ids_and_notes_are_stored_normalized() {
        let mut ada = rule(RecipientType::Contact, "+880 1700-000000");
        ada.note = Some("  ".to_owned());
        ada.chat.name = Some(" Ada ".to_owned());
        let grant = with_rules(vec![ada]).check().unwrap();
        let ada = &grant.chats.rules[0];
        assert_eq!(ada.chat.id, "8801700000000");
        assert_eq!(ada.chat.name.as_deref(), Some("Ada"));
        assert_eq!(ada.note, None);
        assert!(
            grant
                .rule_for(RecipientType::Contact, "8801700000000")
                .is_some()
        );
        assert!(
            grant
                .rule_for(RecipientType::Group, "8801700000000")
                .is_none()
        );
    }

    #[test]
    fn says_what_is_wrong_in_words() {
        let mut limitless = rule(RecipientType::Contact, "8801700000000");
        limitless.daily_limit = Some(0);
        let mut chatty = rule(RecipientType::Group, "120363000000000000@g.us");
        chatty.note = Some("x".repeat(NOTE_MAX + 1));
        chatty.daily_limit = Some(DAILY_CEILING + 1);
        let cases = [
            (
                vec![rule(RecipientType::Contact, "12345")],
                "The chat \"12345\" must be a phone number with its country code.",
            ),
            (
                vec![rule(RecipientType::Group, "8801700000000")],
                "The chat \"8801700000000\" must be one of your WhatsApp groups.",
            ),
            (
                vec![
                    rule(RecipientType::Contact, "8801700000000"),
                    rule(RecipientType::Contact, "+880 1700 000000"),
                ],
                "The chat \"8801700000000\" has more than one rule.",
            ),
            (
                vec![limitless],
                "The rule for \"8801700000000\" has a daily limit of 0. To stop sending there, \
                 set it to not send instead.",
            ),
        ];
        for (rules, problem) in cases {
            assert_eq!(with_rules(rules).check(), Err(vec![problem.to_owned()]));
        }

        let problems = with_rules(vec![chatty]).check().unwrap_err();
        assert_eq!(problems.len(), 2, "{problems:?}");

        let mut default_note = Preset::AskBeforeSending.grant();
        default_note.chats.default.note = Some("x".repeat(NOTE_MAX + 1));
        assert_eq!(
            default_note.check(),
            Err(vec![
                "The default rule has a note longer than 500 characters.".to_owned()
            ])
        );
    }

    #[test]
    fn a_contact_and_a_group_may_share_an_id_shape_without_clashing() {
        let grant = with_rules(vec![
            rule(RecipientType::Contact, "8801700000000"),
            rule(RecipientType::Group, "120363000000000000@g.us"),
        ]);
        assert!(grant.check().is_ok());
    }

    #[test]
    fn at_most_500_rules() {
        let rules = |count: u64| {
            (0..count)
                .map(|n| rule(RecipientType::Contact, &(10_000_000_000 + n).to_string()))
                .collect()
        };
        assert!(with_rules(rules(500)).check().is_ok());
        assert_eq!(
            with_rules(rules(501)).check(),
            Err(vec![
                "There are 501 chat rules; a connection can have at most 500.".to_owned()
            ])
        );
    }

    #[test]
    fn every_preset_is_valid_and_recognised() {
        for preset in Preset::ALL {
            let grant = preset.grant();
            assert_eq!(grant.clone().check(), Ok(grant.clone()), "{preset:?}");
            assert_eq!(Preset::of(&grant), Some(preset));
        }
    }

    #[test]
    fn reading_is_chosen_apart_from_a_preset() {
        let mut grant = Preset::AskBeforeSending.grant();
        grant.chats.default.read = true;
        grant.chats.rules.push(ChatRule {
            read: true,
            ..rule(RecipientType::Contact, "8801700000000")
        });
        assert!(grant.reads_any());
        let sending = grant.clone().without_reading();
        assert!(!sending.reads_any());
        assert_eq!(sending.chats.rules.len(), 1);

        grant.chats.rules.clear();
        assert_eq!(Preset::of(&grant), Some(Preset::AskBeforeSending));
    }

    #[test]
    fn a_changed_preset_is_custom() {
        let mut grant = Preset::FullAccess.grant();
        grant.templates = Templates::None;
        assert_eq!(Preset::of(&grant), None);

        let with_rule = with_rules(vec![rule(RecipientType::Contact, "8801700000000")]);
        assert_eq!(Preset::of(&with_rule), None);
    }

    #[test]
    fn presets_set_what_the_spec_says() {
        let send_freely = Preset::SendWithoutAsking.grant();
        assert_eq!(send_freely.chats.default.send, SendLevel::Yes);
        assert_eq!(send_freely.chats.default.daily_limit, Some(20));
        assert_eq!(send_freely.chats.strangers, Strangers::Ask);

        let view = Preset::ViewOnly.grant();
        assert_eq!(view.chats.default.send, SendLevel::No);
        assert_eq!(view.chats.strangers, Strangers::Never);
        assert_eq!(view.schedules, Schedules::None);

        let full = Preset::FullAccess.grant();
        assert_eq!(full.chats.strangers, Strangers::Allowed);
        assert_eq!(full.schedules, Schedules::All);
        assert_eq!(full.deliveries, Deliveries::All);
    }
}
