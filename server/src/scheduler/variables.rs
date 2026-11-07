//! `{{first_name}}` and the like in a message, filled in when the message is queued to send. A
//! fallback goes after a bar, `{{first_name|there}}`, for when the value is missing.

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use sqlx::PgExecutor;
use uuid::Uuid;

use super::RecipientType;
use crate::{crypto::Cipher, db::whatsapp_contacts, whatsapp::contacts::open_names};

/// Every variable a message can hold. The web app keeps its own list with a hint for each.
pub const NAMES: [&str; 7] = ["first_name", "name", "day", "date", "time", "month", "year"];

/// What a variable is filled with when a message goes out
pub struct Values<'a> {
    /// The recipient's first name as saved on the phone
    pub first_name: Option<&'a str>,
    /// The recipient's name as saved on the phone, or the group's
    pub name: Option<&'a str>,
    /// When it's due, in the message's time zone
    pub at: DateTime<Tz>,
}

enum Piece<'a> {
    Text(&'a str),
    Variable {
        name: &'a str,
        fallback: Option<&'a str>,
    },
}

/// Why a body can't be sent as written
#[derive(Debug, PartialEq, Eq)]
pub enum Problem {
    Unknown(String),
    Unclosed,
}

impl Problem {
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Unknown(name) if name.is_empty() => "{{}} needs a variable's name inside".into(),
            Self::Unknown(name) => format!("{{{{{name}}}}} isn't a variable"),
            Self::Unclosed => "a {{ needs a }} to close it".into(),
        }
    }
}

fn pieces(body: &str) -> Result<Vec<Piece<'_>>, Problem> {
    let mut pieces = Vec::new();
    let mut rest = body;
    while let Some(open) = rest.find("{{") {
        let (text, after) = rest.split_at(open);
        let inner_and_rest = &after[2..];
        let close = inner_and_rest.find("}}").ok_or(Problem::Unclosed)?;
        let (inner, tail) = inner_and_rest.split_at(close);
        let (name, fallback) = inner
            .split_once('|')
            .map_or((inner, None), |(name, fallback)| {
                (name, Some(fallback.trim()))
            });
        let name = name.trim();
        if !NAMES.contains(&name) {
            return Err(Problem::Unknown(name.to_owned()));
        }
        if !text.is_empty() {
            pieces.push(Piece::Text(text));
        }
        pieces.push(Piece::Variable { name, fallback });
        rest = &tail[2..];
    }
    if !rest.is_empty() {
        pieces.push(Piece::Text(rest));
    }
    Ok(pieces)
}

/// # Errors
/// A variable that doesn't exist, or a `{{` left open.
pub fn check(body: &str) -> Result<(), Problem> {
    pieces(body).map(drop)
}

/// The body as it goes out. One that doesn't check out goes as written, since sending it beats
/// dropping it; saving refuses such a body anyway.
#[must_use]
pub fn render(body: &str, values: &Values) -> String {
    let Ok(pieces) = pieces(body) else {
        return body.to_owned();
    };
    let mut rendered = String::with_capacity(body.len());
    for piece in pieces {
        match piece {
            Piece::Text(text) => rendered.push_str(text),
            Piece::Variable { name, fallback } => {
                let value = value(name, values);
                let value = value.as_deref().filter(|value| !value.is_empty());
                rendered.push_str(value.or(fallback).unwrap_or_default());
            }
        }
    }
    rendered
}

fn value(name: &str, values: &Values) -> Option<String> {
    let at = values.at;
    Some(match name {
        "first_name" => values.first_name?.to_owned(),
        "name" => values.name?.to_owned(),
        "day" => at.format("%A").to_string(),
        "date" => at.format("%-d %B").to_string(),
        "time" => at.format("%-I:%M %p").to_string(),
        "month" => at.format("%B").to_string(),
        "year" => at.format("%Y").to_string(),
        _ => return None,
    })
}

/// Who a message goes to, as it was saved
pub struct To<'a> {
    pub recipient_type: RecipientType,
    pub recipient: &'a str,
    pub recipient_name: Option<&'a str>,
}

/// The body as it goes to `to`, due `at` in `timezone`. Names come from the phone's contacts first,
/// then from the name saved with the message.
pub async fn fill_in(
    db: impl PgExecutor<'_>,
    cipher: &Cipher,
    user_id: Uuid,
    to: &To<'_>,
    body: &str,
    at: DateTime<Utc>,
    timezone: &str,
) -> String {
    if !body.contains("{{") {
        return body.to_owned();
    }
    let at = at.with_timezone(&timezone.parse::<Tz>().unwrap_or(Tz::UTC));
    let saved = match to.recipient_type {
        RecipientType::Contact => {
            match whatsapp_contacts::find_by_number(db, user_id, to.recipient).await {
                Ok(found) => found.and_then(|contact| {
                    open_names(cipher, contact.account_id, &contact.jid, &contact.names).ok()
                }),
                Err(error) => {
                    tracing::warn!(%user_id, ?error, "couldn't look up a contact's names");
                    None
                }
            }
        }
        RecipientType::Group => None,
    };
    let full_name = saved.as_ref().and_then(|names| names.full_name.as_deref());
    let name = full_name.or(to.recipient_name);
    let first_name = match to.recipient_type {
        RecipientType::Contact => saved
            .as_ref()
            .and_then(|names| names.first_name.as_deref())
            .or_else(|| name.and_then(first_word)),
        RecipientType::Group => None,
    };
    render(
        body,
        &Values {
            first_name,
            name,
            at,
        },
    )
}

/// "Amina" from "Amina Rahman", for a contact saved without a first name of its own
#[must_use]
pub fn first_word(name: &str) -> Option<&str> {
    name.split_whitespace().next()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn values<'a>(first_name: Option<&'a str>, name: Option<&'a str>) -> Values<'a> {
        Values {
            first_name,
            name,
            at: chrono_tz::Europe::London
                .with_ymd_and_hms(2026, 11, 27, 21, 5, 0)
                .unwrap(),
        }
    }

    #[test]
    fn fills_every_variable() {
        let body =
            "Hi {{first_name}} ({{ name }}), {{day}} {{date}} {{month}} {{year}} at {{time}}";
        assert_eq!(
            render(body, &values(Some("Amina"), Some("Amina Rahman"))),
            "Hi Amina (Amina Rahman), Friday 27 November November 2026 at 9:05 PM"
        );
    }

    #[test]
    fn a_missing_value_takes_its_fallback_or_nothing() {
        let none = values(None, None);
        assert_eq!(render("Hi {{first_name|there}}!", &none), "Hi there!");
        assert_eq!(render("Hi {{first_name}}!", &none), "Hi !");
        assert_eq!(
            render("Hi {{first_name|there}}!", &values(Some("Sam"), None)),
            "Hi Sam!"
        );
    }

    #[test]
    fn checking_names_the_problem() {
        assert_eq!(check("Hi {{first_name}}, {{date|soon}}"), Ok(()));
        assert_eq!(check("No variables at all"), Ok(()));
        assert_eq!(
            check("Hi {{firstname}}"),
            Err(Problem::Unknown("firstname".into()))
        );
        assert_eq!(check("Hi {{}}"), Err(Problem::Unknown(String::new())));
        assert_eq!(check("Hi {{first_name"), Err(Problem::Unclosed));
        assert_eq!(
            Problem::Unknown("firstname".into()).message(),
            "{{firstname}} isn't a variable"
        );
    }

    #[test]
    fn a_body_that_doesnt_check_goes_as_written() {
        assert_eq!(render("Hi {{who}}", &values(None, None)), "Hi {{who}}");
    }

    #[test]
    fn a_first_name_is_the_first_word() {
        assert_eq!(first_word("  Amina Rahman "), Some("Amina"));
        assert_eq!(first_word(" "), None);
    }
}
