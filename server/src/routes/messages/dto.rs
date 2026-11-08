use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::{Validate, ValidationError, ValidationErrors};
use whatsapp_rust::prelude::Jid;

use crate::{
    db::scheduled_messages::{Fields, Message},
    scheduler::{
        RecipientType, ScheduleKind,
        recurrence::{self, Recurrence},
        variables,
    },
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
    pub recurrence: Option<Recurrence>,
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
            recurrence: message.recurrence.map(|json| json.0),
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
    /// Required for `recurring`
    pub recurrence: Option<Recurrence>,
    /// The IANA zone `recurrence` is read in
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
        check_timezone(&self.timezone, &mut errors);
        match self.schedule_kind {
            ScheduleKind::Once => match self.run_at {
                Some(at) if at > Utc::now() => {}
                Some(_) => add(&mut errors, "run_at", "must be in the future"),
                None => add(&mut errors, "run_at", "is required"),
            },
            ScheduleKind::Recurring => {
                check_recurrence(self.recurrence.as_ref(), &self.timezone, &mut errors);
            }
        }
        finish(errors)
    }
}

impl MessageInput {
    /// The columns for this input, normalized. `recipient` comes in from [`normalized_recipient`],
    /// so it outlives the borrow.
    pub(super) fn fields<'a>(&'a self, recipient: &'a str) -> Fields<'a> {
        let (run_at, recurrence) = match self.schedule_kind {
            ScheduleKind::Once => (self.run_at, None),
            ScheduleKind::Recurring => (None, self.recurrence.as_ref().map(Recurrence::normalized)),
        };
        let next_run_at = if !self.enabled {
            None
        } else if let Some(recurrence) = &recurrence {
            recurrence.next_after(&self.timezone, Utc::now())
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
            recurrence,
            timezone: &self.timezone,
            template_id: self.template_id,
            enabled: self.enabled,
            next_run_at,
        }
    }
}

/// A schedule to try out before it's saved
#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(super) struct NextRunsQuery {
    /// A `Recurrence`, as JSON
    #[param(value_type = String)]
    #[serde(deserialize_with = "from_json_text")]
    pub recurrence: Recurrence,
    /// The IANA zone its days are read in
    pub timezone: String,
}

impl Validate for NextRunsQuery {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();
        check_timezone(&self.timezone, &mut errors);
        check_recurrence(Some(&self.recurrence), &self.timezone, &mut errors);
        finish(errors)
    }
}

fn from_json_text<'de, D: Deserializer<'de>, T: DeserializeOwned>(
    deserializer: D,
) -> Result<T, D::Error> {
    let text = String::deserialize(deserializer)?;
    serde_json::from_str(&text).map_err(serde::de::Error::custom)
}

fn from_optional_json_text<'de, D: Deserializer<'de>, T: DeserializeOwned>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<String>::deserialize(deserializer)?
        .filter(|text| !text.is_empty())
        .map(|text| serde_json::from_str(&text).map_err(serde::de::Error::custom))
        .transpose()
}

/// A message as it's being written
#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(super) struct PreviewQuery {
    /// Up to 4096 characters, variables included
    pub body: String,
    /// The IANA zone times read in
    pub timezone: String,
    /// `contact` when left out
    pub recipient_type: Option<RecipientType>,
    /// As in a scheduled message; left out until one's chosen
    pub recipient: Option<String>,
    pub recipient_name: Option<String>,
    /// When a one-time message goes out
    pub run_at: Option<DateTime<Utc>>,
    /// How a recurring message repeats, as JSON: the preview reads its next run, and the place a
    /// sun schedule names. Without either, it reads now.
    #[param(value_type = Option<String>)]
    #[serde(default, deserialize_with = "from_optional_json_text")]
    pub recurrence: Option<Recurrence>,
}

/// A body with a problem still previews, since the variables' examples help most while one's
/// being typed
impl Validate for PreviewQuery {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();
        if self.body.trim().chars().count() > BODY_MAX {
            add(&mut errors, "body", "must be at most 4096 characters");
        }
        check_timezone(&self.timezone, &mut errors);
        finish(errors)
    }
}

#[derive(Serialize, ToSchema)]
pub(super) struct Preview {
    /// The body as it would go out; as written while it has a problem
    pub text: String,
    /// Why saving would refuse the body's variables, if it would
    #[schema(required = true)]
    pub problem: Option<String>,
    /// What each variable holds, unmoved; `null` where it would be empty
    pub values: BTreeMap<&'static str, Option<String>>,
    /// The variables the body uses that would go out empty, having no fallback
    pub empty: Vec<&'static str>,
}

#[derive(Serialize, ToSchema)]
pub(super) struct NextRuns {
    /// Soonest first
    pub runs: Vec<DateTime<Utc>>,
    /// For a sun schedule, the time zone's main city, which it goes by unless it names a place
    #[schema(required = true)]
    pub city: Option<&'static str>,
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

fn check_timezone(timezone: &str, errors: &mut ValidationErrors) {
    if !recurrence::is_timezone(timezone) {
        add(errors, "timezone", "must be an IANA time zone");
    }
}

fn check_recurrence(
    recurrence: Option<&Recurrence>,
    timezone: &str,
    errors: &mut ValidationErrors,
) {
    match recurrence.map(|recurrence| recurrence.problem(timezone)) {
        None => add(errors, "recurrence", "is required"),
        Some(Some(problem)) => add(errors, "recurrence", problem),
        Some(None) => {}
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
