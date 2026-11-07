use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::{Validate, ValidationError, ValidationErrors};
use whatsapp_rust::prelude::Jid;

use crate::{
    db::scheduled_messages::{Fields, Message},
    scheduler::{RecipientType, ScheduleKind, cron, variables},
};

/// WhatsApp allows far more; this keeps payloads sane
const BODY_MAX: usize = 4096;
const NAME_MAX: usize = 255;

#[derive(Serialize, ToSchema)]
pub(super) struct ScheduledMessage {
    pub id: Uuid,
    pub recipient_type: RecipientType,
    pub recipient: String,
    #[schema(required = true)]
    pub recipient_name: Option<String>,
    pub body: String,
    pub schedule_kind: ScheduleKind,
    #[schema(required = true)]
    pub run_at: Option<DateTime<Utc>>,
    #[schema(required = true)]
    pub cron: Option<String>,
    pub timezone: String,
    #[schema(required = true)]
    pub template_id: Option<Uuid>,
    pub enabled: bool,
    /// Empty while disabled, and once a one-time message has gone out
    #[schema(required = true)]
    pub next_run_at: Option<DateTime<Utc>>,
    #[schema(required = true)]
    pub last_run_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Message> for ScheduledMessage {
    fn from(message: Message) -> Self {
        Self {
            id: message.id,
            recipient_type: message.recipient_type,
            recipient: message.recipient,
            recipient_name: message.recipient_name,
            body: message.body,
            schedule_kind: message.schedule_kind,
            run_at: message.run_at,
            cron: message.cron,
            timezone: message.timezone,
            template_id: message.template_id,
            enabled: message.enabled,
            next_run_at: message.next_run_at,
            last_run_at: message.last_run_at,
            created_at: message.created_at,
            updated_at: message.updated_at,
        }
    }
}

fn utc() -> String {
    "UTC".into()
}

const fn enabled() -> bool {
    true
}

#[derive(Deserialize, ToSchema)]
pub(super) struct MessageInput {
    pub recipient_type: RecipientType,
    /// For a contact, a phone number with its country code; a leading +, spaces and dashes are
    /// ignored. For a group, an `id` from `/api/whatsapp/groups`.
    pub recipient: String,
    /// How the recipient shows in lists
    pub recipient_name: Option<String>,
    /// Up to 4096 characters
    pub body: String,
    pub schedule_kind: ScheduleKind,
    /// Required for `once`, and in the future
    pub run_at: Option<DateTime<Utc>>,
    /// Required for `recurring`: 5 fields, minute hour day month weekday
    pub cron: Option<String>,
    /// The IANA zone `cron` is read in
    #[serde(default = "utc")]
    pub timezone: String,
    /// The template the text came from, if any
    pub template_id: Option<Uuid>,
    #[serde(default = "enabled")]
    pub enabled: bool,
}

impl Validate for MessageInput {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();
        check_recipient(
            self.recipient_type,
            &self.recipient,
            self.recipient_name.as_deref(),
            &mut errors,
        );
        check_body(&self.body, &mut errors);
        if !cron::is_timezone(&self.timezone) {
            add(&mut errors, "timezone", "must be an IANA time zone");
        }
        match self.schedule_kind {
            ScheduleKind::Once => match self.run_at {
                Some(at) if at > Utc::now() => {}
                Some(_) => add(&mut errors, "run_at", "must be in the future"),
                None => add(&mut errors, "run_at", "is required"),
            },
            ScheduleKind::Recurring => match self.cron.as_deref() {
                Some(expression) if cron::is_timezone(&self.timezone) => {
                    if let Some(problem) = cron::problem(expression, &self.timezone) {
                        add(&mut errors, "cron", problem);
                    }
                }
                Some(_) => {}
                None => add(&mut errors, "cron", "is required"),
            },
        }
        finish(errors)
    }
}

impl MessageInput {
    /// The columns for this input, normalized. `recipient` comes in from [`normalized_recipient`],
    /// so it outlives the borrow.
    pub(super) fn fields<'a>(&'a self, recipient: &'a str) -> Fields<'a> {
        let (run_at, cron_expression) = match self.schedule_kind {
            ScheduleKind::Once => (self.run_at, None),
            ScheduleKind::Recurring => (None, self.cron.as_deref().map(str::trim)),
        };
        let next_run_at = if !self.enabled {
            None
        } else if let Some(expression) = cron_expression {
            cron::next_run(expression, &self.timezone, Utc::now())
        } else {
            run_at
        };
        Fields {
            recipient_type: self.recipient_type,
            recipient,
            recipient_name: self.recipient_name.as_deref().map(str::trim),
            body: self.body.trim(),
            schedule_kind: self.schedule_kind,
            run_at,
            cron: cron_expression,
            timezone: &self.timezone,
            template_id: self.template_id,
            enabled: self.enabled,
            next_run_at,
        }
    }
}

#[derive(Deserialize, Validate, ToSchema)]
pub(super) struct ToggleRequest {
    pub enabled: bool,
}

#[derive(Deserialize, ToSchema)]
pub(super) struct SendNowRequest {
    pub recipient_type: RecipientType,
    /// As in a scheduled message
    pub recipient: String,
    pub recipient_name: Option<String>,
    /// Up to 4096 characters, variables included
    pub body: String,
    /// The IANA zone `{{date}}`, `{{time}}` and the like read in; UTC when left out
    pub timezone: Option<String>,
}

impl Validate for SendNowRequest {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();
        check_recipient(
            self.recipient_type,
            &self.recipient,
            self.recipient_name.as_deref(),
            &mut errors,
        );
        check_body(&self.body, &mut errors);
        finish(errors)
    }
}

#[derive(Serialize, ToSchema)]
pub(super) struct SendNowResponse {
    pub delivery_id: Uuid,
}

/// How `recipient` is stored: digits for a contact, as given for a group
pub(super) fn normalized_recipient(recipient_type: RecipientType, recipient: &str) -> String {
    match recipient_type {
        RecipientType::Contact => digits(recipient),
        RecipientType::Group => recipient.trim().to_owned(),
    }
}

fn digits(number: &str) -> String {
    number.chars().filter(char::is_ascii_digit).collect()
}

fn check_recipient(
    recipient_type: RecipientType,
    recipient: &str,
    name: Option<&str>,
    errors: &mut ValidationErrors,
) {
    match recipient_type {
        RecipientType::Contact => {
            let only_formatting = recipient
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '+' | ' ' | '-' | '(' | ')'));
            // E.164 allows up to 15 digits, country code included; fewer than 8 can't be a full number
            if !only_formatting || !(8..=15).contains(&digits(recipient).len()) {
                add(
                    errors,
                    "recipient",
                    "must be a phone number with its country code",
                );
            }
        }
        RecipientType::Group => {
            let is_group = recipient
                .trim()
                .parse::<Jid>()
                .is_ok_and(|jid| jid.server.as_str() == "g.us");
            if !is_group {
                add(errors, "recipient", "must be one of your WhatsApp groups");
            }
        }
    }
    if name.is_some_and(|name| name.trim().chars().count() > NAME_MAX) {
        add(errors, "recipient_name", "must be at most 255 characters");
    }
}

pub fn check_body(body: &str, errors: &mut ValidationErrors) {
    let length = body.trim().chars().count();
    if length == 0 {
        add(errors, "body", "can't be empty");
    } else if length > BODY_MAX {
        add(errors, "body", "must be at most 4096 characters");
    } else if let Err(problem) = variables::check(body) {
        errors.add(
            "body",
            ValidationError::new("invalid").with_message(problem.message().into()),
        );
    }
}

fn add(errors: &mut ValidationErrors, field: &'static str, message: &'static str) {
    errors.add(
        field,
        ValidationError::new("invalid").with_message(message.into()),
    );
}

fn finish(errors: ValidationErrors) -> Result<(), ValidationErrors> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
