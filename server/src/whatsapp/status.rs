//! An account's link status and how events move it. [`next`] is pure: the supervisor translates the
//! library's events and the user's commands into [`Event`]s, and carries out the [`Effect`]s it
//! returns.

use chrono::{DateTime, Utc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    NotLinked,
    Pairing(Pairing),
    /// The phone has linked us; the library runs its first sync before `Connected`
    Syncing {
        phone_number: String,
    },
    Connected {
        phone_number: String,
    },
    /// The connection dropped and the library is retrying on its own
    Reconnecting {
        phone_number: String,
    },
    Paused {
        phone_number: String,
    },
    /// Ended for a reason the user has to act on
    Stopped {
        phone_number: String,
        reason: StopReason,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pairing {
    pub method: Method,
    /// `None` until the library hands out the first code
    pub code: Option<Code>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Method {
    Qr,
    /// The 8-character code the user types on the phone that has this number
    PhoneNumber(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Code {
    pub value: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// Another session connected with this device's keys
    Replaced,
    TemporaryBan {
        until: DateTime<Utc>,
    },
    /// WhatsApp rejects this library version; only a server update fixes it
    Outdated,
    /// WhatsApp refused the connection for a reason that retrying won't fix
    Refused,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    // From the user, through the API
    Link(Method),
    Cancel,
    Pause,
    Resume,
    Unlink,
    // From the library, as the supervisor translates them
    QrCode(Code),
    PairCode(Code),
    /// The pair code ran out and the library wants a new one requested
    PairCodeExpired,
    QrCodesExhausted,
    Paired {
        phone_number: String,
    },
    PairFailed,
    Connected,
    Disconnected,
    /// The device was removed from the phone, or WhatsApp logged it out
    LoggedOut,
    Replaced,
    TemporaryBan {
        until: DateTime<Utc>,
    },
    Outdated,
    ConnectFailed {
        retrying: bool,
    },
    // From the supervisor itself
    PairingTimedOut,
    /// The client stopped without being asked to
    ClientEnded,
}

/// Work for the supervisor, in the order given: a client always stops before its keys are forgotten
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Build and spawn a client; while pairing by phone number it also requests the code
    StartClient,
    /// Like `StartClient`, after a pause that grows with each restart in a row
    RestartClient,
    StopClient,
    RequestPairCode,
    /// Ask WhatsApp to remove the device, then stop the client
    LogOut,
    SaveLink {
        phone_number: String,
    },
    SavePaused,
    /// Keep a stop the user has to act on, so a restart doesn't reconnect past it
    SaveStopped(StopReason),
    /// Clear a pause or a kept stop
    SaveResumed,
    /// Delete the account row, and with it every key
    Forget,
    Alert(Alert),
}

/// A security email to the account's owner
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Alert {
    Linked { phone_number: String },
    UnlinkedFromApp,
    UnlinkedFromPhone,
    Replaced,
    TemporaryBan { until: DateTime<Utc> },
}

/// A user command that makes no sense in the current status, such as pausing an account that
/// isn't linked
#[derive(Debug, PartialEq, Eq)]
pub struct Refused;

/// The status `event` leads to from `status`, and what the supervisor must do about it. Library
/// events that don't apply leave the status as it is; user commands that don't apply are refused.
///
/// # Errors
/// [`Refused`] for a user command the current status doesn't allow.
pub fn next(status: &Status, event: Event) -> Result<(Status, Vec<Effect>), Refused> {
    match status {
        Status::NotLinked => from_not_linked(event),
        Status::Pairing(pairing) => from_pairing(pairing, event),
        Status::Syncing { phone_number }
        | Status::Connected { phone_number }
        | Status::Reconnecting { phone_number } => from_linked(status, phone_number, &event),
        Status::Paused { phone_number } | Status::Stopped { phone_number, .. } => {
            from_idle(status, phone_number, &event)
        }
    }
}

fn from_not_linked(event: Event) -> Result<(Status, Vec<Effect>), Refused> {
    match event {
        Event::Link(method) => Ok((
            Status::Pairing(Pairing { method, code: None }),
            vec![Effect::StartClient],
        )),
        Event::Cancel | Event::Pause | Event::Resume | Event::Unlink => Err(Refused),
        _ => Ok((Status::NotLinked, vec![])),
    }
}

fn from_pairing(pairing: &Pairing, event: Event) -> Result<(Status, Vec<Effect>), Refused> {
    let unchanged = || Ok((Status::Pairing(pairing.clone()), vec![]));
    let given_up = || Ok((Status::NotLinked, vec![Effect::StopClient]));
    match (event, &pairing.method) {
        // The library hands out QR codes even while a phone number's code is outstanding
        (Event::QrCode(code), Method::Qr) | (Event::PairCode(code), Method::PhoneNumber(_)) => {
            Ok((
                Status::Pairing(Pairing {
                    method: pairing.method.clone(),
                    code: Some(code),
                }),
                vec![],
            ))
        }
        // With a code outstanding the client stays up after the QR codes run out
        (Event::QrCodesExhausted, Method::Qr)
        | (Event::Cancel | Event::Unlink | Event::PairFailed | Event::PairingTimedOut, _) => {
            given_up()
        }
        (Event::PairCodeExpired, Method::PhoneNumber(_)) => Ok((
            Status::Pairing(Pairing {
                method: pairing.method.clone(),
                code: None,
            }),
            vec![Effect::RequestPairCode],
        )),
        (Event::ClientEnded, _) => Ok((Status::NotLinked, vec![])),
        (Event::Paired { phone_number }, _) => Ok((
            Status::Syncing {
                phone_number: phone_number.clone(),
            },
            vec![
                Effect::SaveLink {
                    phone_number: phone_number.clone(),
                },
                Effect::Alert(Alert::Linked { phone_number }),
            ],
        )),
        (Event::Link(_) | Event::Pause | Event::Resume, _) => Err(Refused),
        _ => unchanged(),
    }
}

/// Syncing, connected or reconnecting: a client is running for a linked device
fn from_linked(
    status: &Status,
    phone_number: &str,
    event: &Event,
) -> Result<(Status, Vec<Effect>), Refused> {
    let phone_number = phone_number.to_owned();
    let stopped = |reason| Status::Stopped {
        phone_number: phone_number.clone(),
        reason,
    };
    match event {
        Event::Connected => Ok((Status::Connected { phone_number }, vec![])),
        Event::Disconnected | Event::ConnectFailed { retrying: true } => {
            Ok((Status::Reconnecting { phone_number }, vec![]))
        }
        Event::ClientEnded => Ok((
            Status::Reconnecting { phone_number },
            vec![Effect::RestartClient],
        )),
        Event::Pause => Ok((
            Status::Paused { phone_number },
            vec![Effect::StopClient, Effect::SavePaused],
        )),
        Event::Unlink => Ok(unlinked_from_app()),
        Event::LoggedOut => Ok(logged_out()),
        Event::Replaced => Ok((
            stopped(StopReason::Replaced),
            vec![
                Effect::StopClient,
                Effect::SaveStopped(StopReason::Replaced),
                Effect::Alert(Alert::Replaced),
            ],
        )),
        Event::TemporaryBan { until } => {
            let reason = StopReason::TemporaryBan { until: *until };
            Ok((
                stopped(reason.clone()),
                vec![
                    Effect::StopClient,
                    Effect::SaveStopped(reason),
                    Effect::Alert(Alert::TemporaryBan { until: *until }),
                ],
            ))
        }
        Event::Outdated => Ok((stopped(StopReason::Outdated), vec![Effect::StopClient])),
        Event::ConnectFailed { retrying: false } => {
            Ok((stopped(StopReason::Refused), vec![Effect::StopClient]))
        }
        Event::Link(_) | Event::Cancel | Event::Resume => Err(Refused),
        _ => Ok((status.clone(), vec![])),
    }
}

/// Paused or stopped: no client runs, so only the user can move it
fn from_idle(
    status: &Status,
    phone_number: &str,
    event: &Event,
) -> Result<(Status, Vec<Effect>), Refused> {
    match event {
        Event::Resume => Ok((
            Status::Reconnecting {
                phone_number: phone_number.to_owned(),
            },
            vec![Effect::StartClient, Effect::SaveResumed],
        )),
        Event::Unlink => Ok(unlinked_from_app()),
        // A logout queued before the client stopped still means its keys are dead
        Event::LoggedOut => Ok(logged_out()),
        Event::Link(_) | Event::Cancel | Event::Pause => Err(Refused),
        _ => Ok((status.clone(), vec![])),
    }
}

fn logged_out() -> (Status, Vec<Effect>) {
    (
        Status::NotLinked,
        vec![
            Effect::StopClient,
            Effect::Forget,
            Effect::Alert(Alert::UnlinkedFromPhone),
        ],
    )
}

fn unlinked_from_app() -> (Status, Vec<Effect>) {
    (
        Status::NotLinked,
        vec![
            Effect::LogOut,
            Effect::Forget,
            Effect::Alert(Alert::UnlinkedFromApp),
        ],
    )
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    const PHONE: &str = "15550001";

    fn at(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0).unwrap()
    }

    fn code(value: &str) -> Code {
        Code {
            value: value.to_owned(),
            expires_at: at(60),
        }
    }

    fn pairing(method: Method) -> Status {
        Status::Pairing(Pairing { method, code: None })
    }

    fn connected() -> Status {
        Status::Connected {
            phone_number: PHONE.to_owned(),
        }
    }

    fn paused() -> Status {
        Status::Paused {
            phone_number: PHONE.to_owned(),
        }
    }

    fn stopped(reason: StopReason) -> Status {
        Status::Stopped {
            phone_number: PHONE.to_owned(),
            reason,
        }
    }

    /// Runs `events` in order from `status`, returning where it ends and every effect on the way
    fn run(mut status: Status, events: impl IntoIterator<Item = Event>) -> (Status, Vec<Effect>) {
        let mut effects = Vec::new();
        for event in events {
            let (after, more) = next(&status, event).unwrap();
            status = after;
            effects.extend(more);
        }
        (status, effects)
    }

    #[test]
    fn qr_linking_runs_from_the_first_code_to_connected() {
        let (status, effects) = run(
            Status::NotLinked,
            [
                Event::Link(Method::Qr),
                Event::QrCode(code("qr-1")),
                Event::Paired {
                    phone_number: PHONE.to_owned(),
                },
                Event::Connected,
            ],
        );
        assert_eq!(status, connected());
        assert_eq!(
            effects,
            [
                Effect::StartClient,
                Effect::SaveLink {
                    phone_number: PHONE.to_owned()
                },
                Effect::Alert(Alert::Linked {
                    phone_number: PHONE.to_owned()
                }),
            ]
        );
    }

    #[test]
    fn pairing_shows_only_the_codes_of_its_method() {
        let by_qr = run(pairing(Method::Qr), [Event::PairCode(code("ABCD-EFGH"))]).0;
        assert_eq!(by_qr, pairing(Method::Qr));

        let method = Method::PhoneNumber(PHONE.to_owned());
        let (by_number, _) = run(
            pairing(method.clone()),
            [
                Event::QrCode(code("qr-1")),
                Event::PairCode(code("ABCD-EFGH")),
            ],
        );
        assert_eq!(
            by_number,
            Status::Pairing(Pairing {
                method,
                code: Some(code("ABCD-EFGH")),
            })
        );
    }

    #[test]
    fn an_expired_pair_code_is_replaced_but_a_qr_one_is_not() {
        let method = Method::PhoneNumber(PHONE.to_owned());
        let with_code = Status::Pairing(Pairing {
            method: method.clone(),
            code: Some(code("ABCD-EFGH")),
        });
        let (status, effects) = run(with_code, [Event::PairCodeExpired]);
        assert_eq!(status, pairing(method));
        assert_eq!(effects, [Effect::RequestPairCode]);

        let (status, effects) = run(pairing(Method::Qr), [Event::PairCodeExpired]);
        assert_eq!(status, pairing(Method::Qr));
        assert_eq!(effects, Vec::<Effect>::new());
    }

    #[test]
    fn running_out_of_qr_codes_ends_qr_pairing_only() {
        let (status, effects) = run(pairing(Method::Qr), [Event::QrCodesExhausted]);
        assert_eq!(status, Status::NotLinked);
        assert_eq!(effects, [Effect::StopClient]);

        let by_number = pairing(Method::PhoneNumber(PHONE.to_owned()));
        assert_eq!(
            run(by_number.clone(), [Event::QrCodesExhausted]).0,
            by_number
        );
        let (status, effects) = run(by_number, [Event::PairingTimedOut]);
        assert_eq!(status, Status::NotLinked);
        assert_eq!(effects, [Effect::StopClient]);
    }

    #[test]
    fn a_dropped_connection_reconnects_and_a_dead_client_restarts() {
        let (status, effects) = run(connected(), [Event::Disconnected]);
        assert_eq!(
            status,
            Status::Reconnecting {
                phone_number: PHONE.to_owned()
            }
        );
        assert_eq!(effects, Vec::<Effect>::new());

        let (status, effects) = run(status, [Event::ClientEnded]);
        assert!(matches!(status, Status::Reconnecting { .. }));
        assert_eq!(effects, [Effect::RestartClient]);
        assert_eq!(run(status, [Event::Connected]).0, connected());
    }

    #[test]
    fn pause_and_resume_stop_and_start_the_client() {
        let (status, effects) = run(connected(), [Event::Pause]);
        assert_eq!(status, paused());
        assert_eq!(effects, [Effect::StopClient, Effect::SavePaused]);

        let (status, effects) = run(status, [Event::Resume]);
        assert!(matches!(status, Status::Reconnecting { .. }));
        assert_eq!(effects, [Effect::StartClient, Effect::SaveResumed]);
    }

    #[test]
    fn unlinking_logs_out_before_forgetting_the_keys() {
        for linked in [connected(), paused(), stopped(StopReason::Replaced)] {
            let (status, effects) = run(linked, [Event::Unlink]);
            assert_eq!(status, Status::NotLinked);
            assert_eq!(
                effects,
                [
                    Effect::LogOut,
                    Effect::Forget,
                    Effect::Alert(Alert::UnlinkedFromApp)
                ]
            );
        }
    }

    #[test]
    fn a_logout_from_the_phone_forgets_the_keys_once_the_client_stops() {
        for linked in [connected(), paused(), stopped(StopReason::Refused)] {
            let (status, effects) = run(linked, [Event::LoggedOut]);
            assert_eq!(status, Status::NotLinked);
            assert_eq!(
                effects,
                [
                    Effect::StopClient,
                    Effect::Forget,
                    Effect::Alert(Alert::UnlinkedFromPhone)
                ]
            );
        }
    }

    #[test]
    fn terminal_failures_stop_with_their_reason() {
        let until = at(3_600);
        let cases = [
            (
                Event::Replaced,
                StopReason::Replaced,
                vec![
                    Effect::StopClient,
                    Effect::SaveStopped(StopReason::Replaced),
                    Effect::Alert(Alert::Replaced),
                ],
            ),
            (
                Event::TemporaryBan { until },
                StopReason::TemporaryBan { until },
                vec![
                    Effect::StopClient,
                    Effect::SaveStopped(StopReason::TemporaryBan { until }),
                    Effect::Alert(Alert::TemporaryBan { until }),
                ],
            ),
            (
                Event::Outdated,
                StopReason::Outdated,
                vec![Effect::StopClient],
            ),
            (
                Event::ConnectFailed { retrying: false },
                StopReason::Refused,
                vec![Effect::StopClient],
            ),
        ];
        for (event, reason, expected) in cases {
            let (status, effects) = run(connected(), [event]);
            assert_eq!(status, stopped(reason));
            assert_eq!(effects, expected);
        }

        let (status, effects) = run(connected(), [Event::ConnectFailed { retrying: true }]);
        assert!(matches!(status, Status::Reconnecting { .. }));
        assert_eq!(effects, Vec::<Effect>::new());
    }

    #[test]
    fn resuming_a_stopped_account_forgets_the_stop() {
        let (status, effects) = run(stopped(StopReason::Replaced), [Event::Resume]);
        assert!(matches!(status, Status::Reconnecting { .. }));
        assert_eq!(effects, [Effect::StartClient, Effect::SaveResumed]);
    }

    #[test]
    fn commands_that_do_not_fit_the_status_are_refused() {
        let cases = [
            (Status::NotLinked, Event::Pause),
            (Status::NotLinked, Event::Unlink),
            (pairing(Method::Qr), Event::Link(Method::Qr)),
            (pairing(Method::Qr), Event::Resume),
            (connected(), Event::Link(Method::Qr)),
            (connected(), Event::Resume),
            (paused(), Event::Pause),
            (stopped(StopReason::Outdated), Event::Cancel),
        ];
        for (status, event) in cases {
            assert_eq!(next(&status, event.clone()), Err(Refused), "{event:?}");
        }
    }

    #[test]
    fn library_events_without_a_client_change_nothing() {
        for idle in [Status::NotLinked, paused(), stopped(StopReason::Replaced)] {
            for event in [Event::Connected, Event::Disconnected, Event::ClientEnded] {
                assert_eq!(next(&idle, event), Ok((idle.clone(), vec![])));
            }
        }
    }
}
