use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use validator::{Validate, ValidationError};

use crate::{
    auth::two_factor::qr_code,
    whatsapp::{
        status::{Method, Status, StopReason},
        supervisor::Group,
    },
};

/// The account's link to WhatsApp. While linking, poll it every couple of seconds: the code changes
/// every 20 to 60 seconds.
#[derive(Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum WhatsAppStatus {
    NotLinked,
    /// Waiting for the phone. Linking by QR code fills `qr_code`, linking by phone number fills
    /// `pair_code`, each once the first code arrives.
    Pairing {
        /// An SVG `data:` URL
        #[schema(required = true)]
        qr_code: Option<String>,
        /// Typed on the phone under Linked devices, "Link with phone number instead"
        #[schema(required = true)]
        pair_code: Option<String>,
        #[schema(required = true)]
        expires_at: Option<DateTime<Utc>>,
    },
    /// Linked; WhatsApp runs its first sync, which can take a few minutes
    Syncing {
        phone_number: String,
    },
    Connected {
        phone_number: String,
    },
    /// The connection dropped and is being retried
    Reconnecting {
        phone_number: String,
    },
    Paused {
        phone_number: String,
    },
    /// Ended for a reason that needs the user: reconnect or unlink
    Stopped {
        phone_number: String,
        reason: StoppedBecause,
        /// When a temporary ban ends
        #[schema(required = true)]
        until: Option<DateTime<Utc>>,
    },
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum StoppedBecause {
    /// Another session took over the device's keys
    Replaced,
    TemporaryBan,
    /// WhatsApp no longer accepts this server's version; an update fixes it
    Outdated,
    /// WhatsApp refused the connection
    Refused,
}

impl WhatsAppStatus {
    pub(super) fn new(status: Status) -> anyhow::Result<Self> {
        Ok(match status {
            Status::NotLinked => Self::NotLinked,
            Status::Pairing(pairing) => {
                let (qr_code, pair_code, expires_at) = match (pairing.method, pairing.code) {
                    (Method::Qr, Some(code)) => {
                        (Some(qr_code(&code.value)?), None, Some(code.expires_at))
                    }
                    (Method::PhoneNumber(_), Some(code)) => {
                        (None, Some(code.value), Some(code.expires_at))
                    }
                    (_, None) => (None, None, None),
                };
                Self::Pairing {
                    qr_code,
                    pair_code,
                    expires_at,
                }
            }
            Status::Syncing { phone_number } => Self::Syncing { phone_number },
            Status::Connected { phone_number } => Self::Connected { phone_number },
            Status::Reconnecting { phone_number } => Self::Reconnecting { phone_number },
            Status::Paused { phone_number } => Self::Paused { phone_number },
            Status::Stopped {
                phone_number,
                reason,
            } => {
                let (reason, until) = match reason {
                    StopReason::Replaced => (StoppedBecause::Replaced, None),
                    StopReason::TemporaryBan { until } => {
                        (StoppedBecause::TemporaryBan, Some(until))
                    }
                    StopReason::Outdated => (StoppedBecause::Outdated, None),
                    StopReason::Refused => (StoppedBecause::Refused, None),
                };
                Self::Stopped {
                    phone_number,
                    reason,
                    until,
                }
            }
        })
    }
}

#[derive(Deserialize, Validate, ToSchema)]
pub(super) struct LinkRequest {
    #[validate(length(min = 1, max = 128, message = "must be between 1 and 128 characters"))]
    pub password: String,
    /// Link with a code typed on the phone that has this number, instead of a QR code. Digits with
    /// the country code; a leading +, spaces and dashes are ignored.
    #[validate(custom(function = "full_phone_number"))]
    pub phone_number: Option<String>,
}

#[derive(Deserialize, Validate, ToSchema)]
pub(super) struct PasswordConfirmation {
    #[validate(length(min = 1, max = 128, message = "must be between 1 and 128 characters"))]
    pub password: String,
}

#[derive(Serialize, ToSchema)]
pub(super) struct Unlinked {
    /// False when WhatsApp couldn't be told, so the device still shows under Linked devices on the
    /// phone until it's removed there
    pub removed_from_phone: bool,
}

#[derive(Serialize, ToSchema)]
pub(super) struct GroupResponse {
    pub id: String,
    pub name: String,
}

impl From<Group> for GroupResponse {
    fn from(group: Group) -> Self {
        Self {
            id: group.id,
            name: group.name,
        }
    }
}

/// The digits of a phone number as WhatsApp expects them
pub(super) fn digits(number: &str) -> String {
    number.chars().filter(char::is_ascii_digit).collect()
}

/// E.164 allows up to 15 digits, country code included; fewer than 8 can't be a full number
fn full_phone_number(number: &str) -> Result<(), ValidationError> {
    let only_formatting = number
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '+' | ' ' | '-' | '(' | ')'));
    let length = digits(number).len();
    if only_formatting && (8..=15).contains(&length) {
        Ok(())
    } else {
        Err(ValidationError::new("phone_number")
            .with_message("must be a phone number with its country code".into()))
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::{Value, json};

    use super::*;
    use crate::whatsapp::status::{Code, Pairing};

    fn json_of(status: Status) -> Value {
        serde_json::to_value(WhatsAppStatus::new(status).unwrap()).unwrap()
    }

    #[test]
    fn statuses_serialize_as_a_tagged_union() {
        assert_eq!(
            json_of(Status::NotLinked),
            json!({ "status": "not_linked" })
        );
        assert_eq!(
            json_of(Status::Paused {
                phone_number: "15550001".into()
            }),
            json!({ "status": "paused", "phone_number": "15550001" })
        );
        let until = Utc.timestamp_opt(1_800_000_000, 0).unwrap();
        assert_eq!(
            json_of(Status::Stopped {
                phone_number: "15550001".into(),
                reason: StopReason::TemporaryBan { until },
            }),
            json!({
                "status": "stopped",
                "phone_number": "15550001",
                "reason": "temporary_ban",
                "until": "2027-01-15T08:00:00Z",
            })
        );
    }

    #[test]
    fn pairing_shows_the_code_of_its_method_only() {
        let code = Code {
            value: "2@abc,def".into(),
            expires_at: Utc.timestamp_opt(1_800_000_000, 0).unwrap(),
        };
        let by_qr = json_of(Status::Pairing(Pairing {
            method: Method::Qr,
            code: Some(code.clone()),
        }));
        assert!(
            by_qr["qr_code"]
                .as_str()
                .unwrap()
                .starts_with("data:image/svg+xml;base64,")
        );
        assert_eq!(by_qr["pair_code"], Value::Null);

        let by_number = json_of(Status::Pairing(Pairing {
            method: Method::PhoneNumber("15550001".into()),
            code: Some(Code {
                value: "ABCD1234".into(),
                ..code
            }),
        }));
        assert_eq!(by_number["pair_code"], "ABCD1234");
        assert_eq!(by_number["qr_code"], Value::Null);

        let waiting = json_of(Status::Pairing(Pairing {
            method: Method::Qr,
            code: None,
        }));
        assert_eq!(
            waiting,
            json!({ "status": "pairing", "qr_code": null, "pair_code": null, "expires_at": null })
        );
    }

    #[test]
    fn phone_numbers_need_8_to_15_digits_and_nothing_but_formatting() {
        for good in ["+880 1700-000000", "15550001", "(212) 555-0100 1"] {
            assert!(full_phone_number(good).is_ok(), "{good}");
        }
        for bad in ["1555000", "1234567890123456", "+1 555 CALL NOW", ""] {
            assert!(full_phone_number(bad).is_err(), "{bad}");
        }
        assert_eq!(digits("+880 1700-000000"), "8801700000000");
    }
}
