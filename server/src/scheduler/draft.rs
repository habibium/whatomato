//! A message as someone asks for it, checked and normalized the same way whether it comes from the
//! web app or from an agent.

use chrono::{DateTime, Utc};
use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;
use validator::{ValidationError, ValidationErrors};

use super::{
    RecipientType, ScheduleKind,
    recurrence::{self, Recurrence},
    variables,
};
use crate::{
    crypto::Cipher,
    db::{
        message_templates,
        scheduled_messages::{Fields, Message},
    },
};

/// WhatsApp allows far more; this keeps payloads sane
pub const BODY_MAX: usize = 4096;
pub const NAME_MAX: usize = 255;

/// A scheduled message as asked for, before it's checked
pub struct Draft<'a> {
    pub recipient_type: RecipientType,
    pub recipient: &'a str,
    pub recipient_name: Option<&'a str>,
    pub body: &'a str,
    pub schedule_kind: ScheduleKind,
    pub run_at: Option<DateTime<Utc>>,
    pub recurrence: Option<&'a Recurrence>,
    pub timezone: &'a str,
    pub template_id: Option<Uuid>,
    pub enabled: bool,
}

impl<'a> Draft<'a> {
    /// Everything but the template, which needs the database: see [`check_template`]
    pub fn check(&self, errors: &mut ValidationErrors) {
        check_recipient(
            self.recipient_type,
            self.recipient,
            self.recipient_name,
            errors,
        );
        check_body(self.body, errors);
        check_timezone(self.timezone, errors);
        match self.schedule_kind {
            ScheduleKind::Once => match self.run_at {
                Some(at) if at > Utc::now() => {}
                Some(_) => add(errors, "run_at", "must be in the future"),
                None => add(errors, "run_at", "is required"),
            },
            ScheduleKind::Recurring => {
                check_recurrence(self.recurrence, self.timezone, errors);
            }
        }
    }

    /// The columns for this draft, normalized. `recipient` comes in from
    /// [`RecipientType::normalized`], so it outlives the borrow.
    #[must_use]
    pub fn fields(&self, recipient: &'a str) -> Fields<'a> {
        let (run_at, recurrence) = match self.schedule_kind {
            ScheduleKind::Once => (self.run_at, None),
            ScheduleKind::Recurring => (None, self.recurrence.map(Recurrence::normalized)),
        };
        let next_run_at = if !self.enabled {
            None
        } else if let Some(recurrence) = &recurrence {
            recurrence.next_after(self.timezone, Utc::now())
        } else {
            run_at
        };
        Fields {
            recipient_type: self.recipient_type,
            recipient,
            recipient_name: self.recipient_name.map(str::trim),
            body: self.body.trim(),
            schedule_kind: self.schedule_kind,
            run_at,
            recurrence,
            timezone: self.timezone,
            template_id: self.template_id,
            enabled: self.enabled,
            next_run_at,
        }
    }
}

/// When a message switched on or off next goes out, or why it can't be switched on
///
/// # Errors
/// A one-time message whose time has passed: switching it back on would send straight away,
/// which nobody means by a switch.
pub fn next_run_when_switched(
    message: &Message,
    enabled: bool,
    now: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, &'static str> {
    match (enabled, message.schedule_kind) {
        (false, _) => Ok(None),
        (true, ScheduleKind::Once) if message.run_at.is_none_or(|at| at <= now) => {
            Err("This message's send time has passed; edit it to pick a new one")
        }
        (true, ScheduleKind::Once) => Ok(message.run_at),
        (true, ScheduleKind::Recurring) => Ok(message
            .recurrence
            .as_ref()
            .and_then(|recurrence| recurrence.next_after(&message.timezone, now))),
    }
}

/// A template id must be the user's own, or the foreign key would confirm other users' ids
///
/// # Errors
/// The database couldn't be read.
pub async fn check_template(
    db: impl PgExecutor<'_>,
    template_id: Option<Uuid>,
    user_id: Uuid,
    errors: &mut ValidationErrors,
) -> sqlx::Result<()> {
    let Some(template_id) = template_id else {
        return Ok(());
    };
    if message_templates::find(db, template_id, user_id)
        .await?
        .is_none()
    {
        add(errors, "template_id", "isn't one of your templates");
    }
    Ok(())
}

/// The body of a message sent now, its variables filled in for `to` as of this moment
pub async fn body_now(
    pool: &PgPool,
    cipher: &Cipher,
    user_id: Uuid,
    to: &variables::To<'_>,
    body: &str,
    timezone: Option<&str>,
) -> String {
    let due = variables::Due {
        at: Utc::now(),
        timezone: timezone.unwrap_or("UTC"),
        place: None,
    };
    variables::fill_in(pool, cipher, user_id, to, body.trim(), &due).await
}

pub fn check_recipient(
    recipient_type: RecipientType,
    recipient: &str,
    name: Option<&str>,
    errors: &mut ValidationErrors,
) {
    if let Some(problem) = recipient_type.problem(recipient) {
        add(errors, "recipient", problem);
    }
    if name.is_some_and(|name| name.trim().chars().count() > NAME_MAX) {
        add(errors, "recipient_name", "must be at most 255 characters");
    }
}

pub fn check_timezone(timezone: &str, errors: &mut ValidationErrors) {
    if !recurrence::is_timezone(timezone) {
        add(errors, "timezone", "must be an IANA time zone");
    }
}

pub fn check_recurrence(
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

pub fn add(errors: &mut ValidationErrors, field: &'static str, message: &'static str) {
    errors.add(
        field,
        ValidationError::new("invalid").with_message(message.into()),
    );
}

/// # Errors
/// `errors`, when it holds any.
pub fn finish(errors: ValidationErrors) -> Result<(), ValidationErrors> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
