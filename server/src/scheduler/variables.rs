//! `{{first_name}}` and the like in a message, filled in when the message is queued to send.
//!
//! A fallback goes after a bar, `{{first_name|there}}`, for when the value is missing. A time moves
//! by minutes, hours or days: `{{sunset + 10m}}`, `{{date + 1 day}}`.

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use chrono_tz::Tz;
use sqlx::PgExecutor;
use uuid::Uuid;

use super::{
    RecipientType,
    recurrence::{self, Place, SunEvent},
};
use crate::{crypto::Cipher, db::whatsapp_contacts, whatsapp::contacts::open_names};

/// Every variable a message can hold. The web app keeps its own list, with a hint for each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Variable {
    FirstName,
    Name,
    Day,
    Date,
    Time,
    Month,
    Year,
    Sunrise,
    Sunset,
}

impl Variable {
    const ALL: [Self; 9] = [
        Self::FirstName,
        Self::Name,
        Self::Day,
        Self::Date,
        Self::Time,
        Self::Month,
        Self::Year,
        Self::Sunrise,
        Self::Sunset,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::FirstName => "first_name",
            Self::Name => "name",
            Self::Day => "day",
            Self::Date => "date",
            Self::Time => "time",
            Self::Month => "month",
            Self::Year => "year",
            Self::Sunrise => "sunrise",
            Self::Sunset => "sunset",
        }
    }

    fn named(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|variable| variable.name() == name)
    }

    /// Whether it holds a time, which can be moved
    const fn is_time(self) -> bool {
        !matches!(self, Self::FirstName | Self::Name)
    }
}

/// How far a time moves: whole days on the calendar, then minutes on the clock
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Shift {
    days: i64,
    minutes: i64,
}

const SHIFT_LIMIT_DAYS: i64 = 366;
const TIME: &str = "%-I:%M %p";

/// What a variable is filled with when a message goes out
pub struct Values {
    /// The recipient's first name as saved on the phone
    pub first_name: Option<String>,
    /// The recipient's name as saved on the phone, or the group's
    pub name: Option<String>,
    /// When it's due, in the message's time zone
    pub at: DateTime<Tz>,
    /// Where sunrise and sunset are watched from
    pub place: Option<Place>,
}

/// A variable as a body uses it
struct Use<'a> {
    variable: Variable,
    shift: Shift,
    fallback: Option<&'a str>,
}

enum Piece<'a> {
    Text(&'a str),
    Variable(Use<'a>),
}

/// Why a body can't be sent as written
#[derive(Debug, PartialEq, Eq)]
pub enum Problem {
    Unknown(String),
    Unclosed,
    /// What follows the name isn't `+` or `-`, a number and a unit
    Shift(String),
    /// A name, which can't be moved
    NotATime(String),
    TooFar(String),
}

impl Problem {
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Unknown(name) if name.is_empty() => "{{}} needs a variable's name inside".into(),
            Self::Unknown(name) => format!("{{{{{name}}}}} isn't a variable"),
            Self::Unclosed => "a {{ needs a }} to close it".into(),
            Self::Shift(inner) => {
                format!("{{{{{inner}}}}} should move by a number and a unit, like + 10m")
            }
            Self::NotATime(name) => format!("{{{{{name}}}}} isn't a time, so it can't move"),
            Self::TooFar(inner) => format!("{{{{{inner}}}}} moves a time by more than a year"),
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
        let used = parse_use(inner)?;
        if !text.is_empty() {
            pieces.push(Piece::Text(text));
        }
        pieces.push(Piece::Variable(used));
        rest = &tail[2..];
    }
    if !rest.is_empty() {
        pieces.push(Piece::Text(rest));
    }
    Ok(pieces)
}

/// What's between the braces: a name, any moves, then any fallback after a bar
fn parse_use(inner: &str) -> Result<Use<'_>, Problem> {
    let (expression, fallback) = inner
        .split_once('|')
        .map_or((inner, None), |(expression, fallback)| {
            (expression, Some(fallback.trim()))
        });
    let expression = expression.trim();
    let name_end = expression.find(['+', '-']).unwrap_or(expression.len());
    let name = expression[..name_end].trim();
    let variable = Variable::named(name).ok_or_else(|| Problem::Unknown(name.to_owned()))?;
    let shift = parse_shift(&expression[name_end..])
        .ok_or_else(|| Problem::Shift(expression.to_owned()))?;
    if shift != Shift::default() && !variable.is_time() {
        return Err(Problem::NotATime(name.to_owned()));
    }
    if shift.days.abs() + shift.minutes.abs() / (24 * 60) > SHIFT_LIMIT_DAYS {
        return Err(Problem::TooFar(expression.to_owned()));
    }
    Ok(Use {
        variable,
        shift,
        fallback,
    })
}

/// `+ 10m`, `- 1 hour + 5 minutes`, `+1d`; `None` for anything else
fn parse_shift(mut rest: &str) -> Option<Shift> {
    let mut shift = Shift::default();
    loop {
        rest = rest.trim_start();
        let sign = match rest.chars().next() {
            None => return Some(shift),
            Some('+') => 1,
            Some('-') => -1,
            Some(_) => return None,
        };
        rest = rest[1..].trim_start();
        let digits = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        let amount = sign * rest[..digits].parse::<i64>().ok()?;
        rest = rest[digits..].trim_start();
        let letters = rest
            .find(|c: char| !c.is_ascii_alphabetic())
            .unwrap_or(rest.len());
        let unit = rest[..letters].to_ascii_lowercase();
        rest = &rest[letters..];
        match unit.as_str() {
            "m" | "min" | "mins" | "minute" | "minutes" => {
                shift.minutes = shift.minutes.checked_add(amount)?;
            }
            "h" | "hr" | "hrs" | "hour" | "hours" => {
                shift.minutes = shift.minutes.checked_add(amount.checked_mul(60)?)?;
            }
            "d" | "day" | "days" => shift.days = shift.days.checked_add(amount)?,
            _ => return None,
        }
    }
}

/// # Errors
/// A variable that doesn't exist, a `{{` left open, or a move that doesn't read.
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
            Piece::Variable(used) => {
                let value = value(used.variable, used.shift, values);
                let value = value.as_deref().filter(|value| !value.is_empty());
                rendered.push_str(value.or(used.fallback).unwrap_or_default());
            }
        }
    }
    rendered
}

/// Each variable as it stands, unmoved, for examples while a message is written
#[must_use]
pub fn all(values: &Values) -> Vec<(&'static str, Option<String>)> {
    Variable::ALL
        .into_iter()
        .map(|variable| (variable.name(), value(variable, Shift::default(), values)))
        .collect()
}

/// The variables `body` uses that would go out empty, having no fallback
#[must_use]
pub fn empty(body: &str, values: &Values) -> Vec<&'static str> {
    let Ok(pieces) = pieces(body) else {
        return Vec::new();
    };
    let mut empty = Vec::new();
    for piece in pieces {
        if let Piece::Variable(used) = piece
            && used.fallback.is_none()
            && value(used.variable, used.shift, values).is_none_or(|value| value.is_empty())
            && !empty.contains(&used.variable.name())
        {
            empty.push(used.variable.name());
        }
    }
    empty
}

fn value(variable: Variable, shift: Shift, values: &Values) -> Option<String> {
    let format = |format: &str| Some(moved(values.at, shift)?.format(format).to_string());
    match variable {
        Variable::FirstName => values.first_name.clone(),
        Variable::Name => values.name.clone(),
        Variable::Day => format("%A"),
        Variable::Date => format("%-d %B"),
        Variable::Time => format(TIME),
        Variable::Month => format("%B"),
        Variable::Year => format("%Y"),
        Variable::Sunrise => sun(SunEvent::Sunrise, shift, values),
        Variable::Sunset => sun(SunEvent::Sunset, shift, values),
    }
}

/// `at` moved by days on its own calendar, so a day stays a day across daylight saving
fn moved(at: DateTime<Tz>, shift: Shift) -> Option<DateTime<Tz>> {
    let days = TimeDelta::days(shift.days);
    let local = at.naive_local().checked_add_signed(days)?;
    let on_the_day = at
        .timezone()
        .from_local_datetime(&local)
        .earliest()
        // A clock time that daylight saving skips
        .or_else(|| at.checked_add_signed(days))?;
    on_the_day.checked_add_signed(TimeDelta::minutes(shift.minutes))
}

/// The sun on the day it goes out, or that many days on, then moved by the minutes
fn sun(event: SunEvent, shift: Shift, values: &Values) -> Option<String> {
    let zone = values.at.timezone();
    let day = values
        .at
        .date_naive()
        .checked_add_signed(TimeDelta::days(shift.days))?;
    let at = recurrence::event_on(event, day, values.place?, zone)?
        .with_timezone(&zone)
        .checked_add_signed(TimeDelta::minutes(shift.minutes))?;
    Some(at.format(TIME).to_string())
}

/// Who a message goes to, as it was saved
pub struct To<'a> {
    pub recipient_type: RecipientType,
    pub recipient: &'a str,
    pub recipient_name: Option<&'a str>,
}

/// When and where a message goes out, for the variables that read the clock or the sun
pub struct Due<'a> {
    pub at: DateTime<Utc>,
    pub timezone: &'a str,
    /// The place a sun schedule names; the time zone's main city stands in without one
    pub place: Option<Place>,
}

/// What the variables hold for a message to `to`. Names come from the phone's contacts first, then
/// from the name saved with the message.
pub async fn values(
    db: impl PgExecutor<'_>,
    cipher: &Cipher,
    user_id: Uuid,
    to: &To<'_>,
    due: &Due<'_>,
) -> Values {
    let zone = due.timezone.parse::<Tz>().unwrap_or(Tz::UTC);
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
    Values {
        first_name: first_name.map(str::to_owned),
        name: name.map(str::to_owned),
        at: due.at.with_timezone(&zone),
        place: due.place.or_else(|| recurrence::city_place(zone)),
    }
}

/// The body as it goes out to `to`
pub async fn fill_in(
    db: impl PgExecutor<'_>,
    cipher: &Cipher,
    user_id: Uuid,
    to: &To<'_>,
    body: &str,
    due: &Due<'_>,
) -> String {
    if !body.contains("{{") {
        return body.to_owned();
    }
    render(body, &values(db, cipher, user_id, to, due).await)
}

/// "Amina" from "Amina Rahman", for a contact saved without a first name of its own
#[must_use]
pub fn first_word(name: &str) -> Option<&str> {
    name.split_whitespace().next()
}

#[cfg(test)]
mod tests {
    use chrono_tz::{Asia::Dhaka, Europe::London};

    use super::*;

    fn values(first_name: Option<&str>, name: Option<&str>) -> Values {
        Values {
            first_name: first_name.map(str::to_owned),
            name: name.map(str::to_owned),
            at: London.with_ymd_and_hms(2026, 11, 27, 21, 5, 0).unwrap(),
            place: recurrence::city_place(London),
        }
    }

    /// 6 PM in Dhaka on 8 November 2026, when sunset was at 5:16 PM and sunrise at 6:08 AM
    fn dhaka() -> Values {
        Values {
            first_name: None,
            name: None,
            at: Dhaka.with_ymd_and_hms(2026, 11, 8, 18, 0, 0).unwrap(),
            place: recurrence::city_place(Dhaka),
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
    fn fills_in_the_sun_on_the_day_it_goes_out() {
        assert_eq!(
            render(
                "Iftar at {{sunset}}, sehri ends before {{sunrise + 1d}}",
                &dhaka()
            ),
            "Iftar at 5:16 PM, sehri ends before 6:09 AM"
        );
    }

    #[test]
    fn a_time_moves_by_minutes_hours_and_days() {
        for (body, filled) in [
            ("{{sunset + 10m}}", "5:26 PM"),
            ("{{sunset+10 minutes}}", "5:26 PM"),
            ("{{ sunset - 1 hour + 5 MIN }}", "4:21 PM"),
            ("{{time - 30m}}", "5:30 PM"),
            ("{{time + 2h}}", "8:00 PM"),
            ("{{day + 1d}} {{date + 1 day}}", "Monday 9 November"),
            ("{{date - 9 days}}", "30 October"),
        ] {
            assert_eq!(render(body, &dhaka()), filled, "{body}");
        }
    }

    #[test]
    fn a_day_on_stays_a_day_across_daylight_saving() {
        // London goes from GMT to BST at 1 AM on 29 March 2026, so that day is 23 hours long
        let night_before = Values {
            at: London.with_ymd_and_hms(2026, 3, 28, 23, 30, 0).unwrap(),
            ..values(None, None)
        };
        assert_eq!(
            render("{{date + 1d}} at {{time + 1d}}", &night_before),
            "29 March at 11:30 PM"
        );
    }

    #[test]
    fn without_a_place_the_sun_takes_its_fallback() {
        let nowhere = Values {
            place: None,
            ..dhaka()
        };
        assert_eq!(
            render("Iftar at {{sunset|sunset}}", &nowhere),
            "Iftar at sunset"
        );
        assert_eq!(empty("{{sunset}} {{sunrise|dawn}}", &nowhere), ["sunset"]);
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
    fn empty_names_each_variable_once() {
        let none = values(None, None);
        assert_eq!(
            empty("{{first_name}} {{first_name}} {{name|you}} {{time}}", &none),
            ["first_name"]
        );
    }

    #[test]
    fn checking_names_the_problem() {
        assert_eq!(check("Hi {{first_name}}, {{date|soon}}"), Ok(()));
        assert_eq!(check("No variables at all"), Ok(()));
        assert_eq!(check("{{sunset - 1h + 5m|later}}"), Ok(()));
        assert_eq!(check("{{date + 366d + 1439m}}"), Ok(()));
        for (body, problem) in [
            ("Hi {{firstname}}", Problem::Unknown("firstname".into())),
            ("Hi {{}}", Problem::Unknown(String::new())),
            ("Hi {{+ 10m}}", Problem::Unknown(String::new())),
            ("Hi {{first_name", Problem::Unclosed),
            ("{{sunset + soon}}", Problem::Shift("sunset + soon".into())),
            ("{{sunset + 10}}", Problem::Shift("sunset + 10".into())),
            (
                "{{sunset + 10 weeks}}",
                Problem::Shift("sunset + 10 weeks".into()),
            ),
            ("{{sunset 10m}}", Problem::Unknown("sunset 10m".into())),
            (
                "{{first_name + 1d}}",
                Problem::NotATime("first_name".into()),
            ),
            (
                "{{first_name + soon}}",
                Problem::Shift("first_name + soon".into()),
            ),
            ("{{date + 367d}}", Problem::TooFar("date + 367d".into())),
            (
                "{{date + 99999999999999999999d}}",
                Problem::Shift("date + 99999999999999999999d".into()),
            ),
        ] {
            assert_eq!(check(body), Err(problem), "{body}");
        }
    }

    #[test]
    fn problems_read_as_sentences() {
        assert_eq!(
            Problem::Unknown("firstname".into()).message(),
            "{{firstname}} isn't a variable"
        );
        assert_eq!(
            Problem::Shift("sunset + soon".into()).message(),
            "{{sunset + soon}} should move by a number and a unit, like + 10m"
        );
        assert_eq!(
            Problem::NotATime("name".into()).message(),
            "{{name}} isn't a time, so it can't move"
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
