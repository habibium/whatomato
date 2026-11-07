use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use validator::{Validate, ValidationError};

use crate::{
    auth::two_factor::qr_code,
    whatsapp::{
        contacts::Names,
        status::{LinkEnded, Method, Status, StopReason},
        supervisor::Group,
    },
};

/// The account's link to WhatsApp. While linking, poll it every couple of seconds: the code changes
/// every 20 to 60 seconds.
#[derive(Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum WhatsAppStatus {
    NotLinked {
        /// Why the last link or linking attempt ended, unless the user ended it. Lost on a restart.
        #[schema(required = true)]
        reason: Option<NotLinkedBecause>,
    },
    /// Waiting for the phone. Linking by QR code fills `qr_code`, linking by phone number fills
    /// `pair_code`, each once the first code arrives.
    Pairing {
        method: PairingMethod,
        /// The number being linked, when linking by phone number
        #[schema(required = true)]
        phone_number: Option<String>,
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
pub(super) enum NotLinkedBecause {
    /// Linking ran out of codes or time before the phone answered
    Expired,
    /// WhatsApp didn't finish linking
    Failed,
    /// The device was removed from the phone, or WhatsApp logged it out
    LoggedOut,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum PairingMethod {
    QrCode,
    PhoneNumber,
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
            Status::NotLinked => Self::NotLinked { reason: None },
            Status::LinkEnded(ended) => Self::NotLinked {
                reason: Some(match ended {
                    LinkEnded::Expired => NotLinkedBecause::Expired,
                    LinkEnded::Failed => NotLinkedBecause::Failed,
                    LinkEnded::LoggedOut => NotLinkedBecause::LoggedOut,
                }),
            },
            Status::Pairing(pairing) => {
                let expires_at = pairing.code.as_ref().map(|code| code.expires_at);
                let code = pairing.code.map(|code| code.value);
                let (method, phone_number, qr_code, pair_code) = match pairing.method {
                    Method::Qr => (
                        PairingMethod::QrCode,
                        None,
                        code.as_deref().map(qr_code).transpose()?,
                        None,
                    ),
                    Method::PhoneNumber(number) => {
                        (PairingMethod::PhoneNumber, Some(number), None, code)
                    }
                };
                Self::Pairing {
                    method,
                    phone_number,
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

#[derive(Serialize, ToSchema)]
pub(super) struct ContactBook {
    /// False until the phone's address book has come through in full, which starts once WhatsApp
    /// connects. Poll every few seconds meanwhile.
    pub imported: bool,
    /// By name
    pub contacts: Vec<ContactResponse>,
}

/// A contact saved on the phone
#[derive(Serialize, ToSchema)]
pub(super) struct ContactResponse {
    /// WhatsApp's address for the contact
    pub id: String,
    /// As saved on the phone, else the first name or the WhatsApp username
    pub name: String,
    #[schema(required = true)]
    pub first_name: Option<String>,
    /// Digits with the country code. Empty when WhatsApp hasn't shared the number with linked
    /// devices, and then nothing can be sent to the contact.
    #[schema(required = true)]
    pub phone_number: Option<String>,
    #[schema(required = true)]
    pub username: Option<String>,
}

impl ContactResponse {
    /// `None` for a contact with no name at all, which no one could pick out of a list
    pub(super) fn new(id: String, phone_number: Option<String>, names: Names) -> Option<Self> {
        let name = names
            .full_name
            .clone()
            .or_else(|| names.first_name.clone())
            .or_else(|| names.username.clone())?;
        Some(Self {
            id,
            name,
            first_name: names.first_name,
            phone_number,
            username: names.username,
        })
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
            json!({ "status": "not_linked", "reason": null })
        );
        assert_eq!(
            json_of(Status::LinkEnded(LinkEnded::LoggedOut)),
            json!({ "status": "not_linked", "reason": "logged_out" })
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
        assert_eq!(by_number["method"], "phone_number");
        assert_eq!(by_number["phone_number"], "15550001");

        let waiting = json_of(Status::Pairing(Pairing {
            method: Method::Qr,
            code: None,
        }));
        assert_eq!(
            waiting,
            json!({
                "status": "pairing",
                "method": "qr_code",
                "phone_number": null,
                "qr_code": null,
                "pair_code": null,
                "expires_at": null,
            })
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
