//! Sends messages. One task, on the server holding the WhatsApp lock: every tick it turns due
//! schedules into deliveries, then starts at most one delivery per account.

pub(crate) mod draft;
pub mod recurrence;
pub mod variables;

use std::{collections::HashMap, future::Future, time::Duration};

use chrono::{DateTime, TimeDelta, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tokio::{
    sync::oneshot,
    task::{JoinHandle, JoinSet},
    time::{MissedTickBehavior, interval, timeout},
};
use utoipa::ToSchema;
use uuid::Uuid;
use whatsapp_rust::prelude::Jid;

use crate::{
    agents::{
        ChatRef,
        firing::{self, Firing},
    },
    crypto::Cipher,
    db::{
        deliveries::{self, Claimed, NewDelivery},
        scheduled_messages,
    },
    whatsapp::{
        manager::WhatsApp,
        supervisor::{Recipient, SendFailure},
    },
};

/// Also the pacing, since an account sends at most once a tick: WhatsApp bans accounts that send
/// like a machine
const TICK: Duration = Duration::from_secs(3);
const DUE_PER_TICK: i64 = 100;
/// A send in progress gets this long at shutdown; one cut off counts as interrupted at the next start
const DRAIN_LIMIT: Duration = Duration::from_secs(8);
/// Between tries when nothing went out, about half an hour in all, to outlast a restart or an outage
const RETRY_WAITS: [Duration; 5] = [
    Duration::from_mins(1),
    Duration::from_mins(2),
    Duration::from_mins(4),
    Duration::from_mins(8),
    Duration::from_mins(15),
];
const INTERRUPTED: &str =
    "The server restarted during this send, so it may or may not have gone out";

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema, sqlx::Type,
)]
#[sqlx(type_name = "text", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum RecipientType {
    /// `recipient` holds a phone number's digits
    Contact,
    /// `recipient` holds an `id` from the group list
    Group,
}

impl RecipientType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Contact => "contact",
            Self::Group => "group",
        }
    }

    /// How `recipient` is stored: digits for a contact, without the international prefix `00`;
    /// trimmed for a group. One chat has one stored spelling, which per-chat rules match.
    #[must_use]
    pub fn normalized(self, recipient: &str) -> String {
        match self {
            Self::Contact => {
                let digits: String = recipient.chars().filter(char::is_ascii_digit).collect();
                digits
                    .strip_prefix("00")
                    .map_or_else(|| digits.clone(), str::to_owned)
            }
            Self::Group => recipient.trim().to_owned(),
        }
    }

    /// Why `recipient` can't be a recipient of this type, if so
    #[must_use]
    pub fn problem(self, recipient: &str) -> Option<&'static str> {
        match self {
            Self::Contact => {
                let only_formatting = recipient
                    .chars()
                    .all(|c| c.is_ascii_digit() || matches!(c, '+' | ' ' | '-' | '(' | ')'));
                let digits = self.normalized(recipient);
                // E.164 allows up to 15 digits, country code included; fewer than 8 can't be a full
                // number, and no country code starts with 0
                if !only_formatting || !(8..=15).contains(&digits.len()) {
                    Some("must be a phone number with its country code")
                } else if digits.starts_with('0') {
                    Some("must start with its country code, not with 0")
                } else {
                    None
                }
            }
            Self::Group => {
                (!is_group_id(recipient.trim())).then_some("must be one of your WhatsApp groups")
            }
        }
    }
}

/// Whether `id` is a group's id as WhatsApp lists it: `<digits>@g.us`, or `<digits>-<digits>@g.us`
/// for an old group. Anything else the library would parse (a device or agent part, another
/// server's spelling) could name the same group a second time, so it's refused.
fn is_group_id(id: &str) -> bool {
    let Some(user) = id.strip_suffix("@g.us") else {
        return false;
    };
    let number = |part: &str| {
        !part.is_empty() && !part.starts_with('0') && part.bytes().all(|b| b.is_ascii_digit())
    };
    let canonical = match user.split_once('-') {
        Some((creator, created)) => number(creator) && number(created),
        None => number(user),
    };
    canonical
        && id
            .parse::<Jid>()
            .is_ok_and(|jid| jid.server.as_str() == "g.us" && jid.user == user)
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema, sqlx::Type,
)]
#[sqlx(type_name = "text", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum ScheduleKind {
    /// Goes out at `run_at`
    Once,
    /// Goes out as `recurrence` says, its days read in `timezone`
    Recurring,
}

impl ScheduleKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Once => "once",
            Self::Recurring => "recurring",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "lowercase")]
pub enum DeliveryStatus {
    Pending,
    Sending,
    Sent,
    Failed,
}

/// What the scheduler needs from WhatsApp, apart so tests can run without a connection
pub trait Sender: Clone + Send + Sync + 'static {
    /// Resolves once this server may send
    fn ready(&self) -> impl Future<Output = ()> + Send;

    fn send(
        &self,
        user_id: Uuid,
        to: Recipient,
        body: String,
    ) -> impl Future<Output = Result<(), SendFailure>> + Send;
}

impl Sender for WhatsApp {
    async fn ready(&self) {
        self.may_send().await;
    }

    async fn send(&self, user_id: Uuid, to: Recipient, body: String) -> Result<(), SendFailure> {
        self.send_text(user_id, to, body).await
    }
}

/// The running scheduler
pub struct Scheduler {
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl Scheduler {
    /// `agent_access` is the instance switch: while it's off, agents' schedules are skipped
    #[must_use]
    pub fn spawn(pool: PgPool, cipher: Cipher, sender: impl Sender, agent_access: bool) -> Self {
        let (stop, stopped) = oneshot::channel();
        Self {
            stop,
            task: tokio::spawn(run(pool, cipher, sender, agent_access, stopped)),
        }
    }

    /// Stops starting sends and lets those in progress finish. Comes before WhatsApp's own
    /// shutdown, which would otherwise cut them off.
    pub async fn shutdown(self) {
        let _ = self.stop.send(());
        let _ = self.task.await;
    }
}

async fn run(
    pool: PgPool,
    cipher: Cipher,
    sender: impl Sender,
    agent_access: bool,
    mut stopped: oneshot::Receiver<()>,
) {
    tokio::select! {
        () = sender.ready() => {}
        _ = &mut stopped => return,
    }
    match deliveries::fail_interrupted(&pool, INTERRUPTED).await {
        Ok(0) => {}
        Ok(failed) => tracing::warn!(failed, "failed deliveries the last server left mid-send"),
        Err(error) => tracing::warn!(?error, "couldn't fail interrupted deliveries"),
    }

    let mut sends = JoinSet::new();
    // By task, so a send that panics can't leave its account busy for good
    let mut busy = HashMap::new();
    let mut ticks = interval(TICK);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = &mut stopped => break,
            Some(finished) = sends.join_next_with_id() => {
                let task = finished.map_or_else(|error| error.id(), |(task, ())| task);
                busy.remove(&task);
            }
            _ = ticks.tick() => {
                if let Err(error) = enqueue_due(&pool, &cipher, Utc::now(), agent_access).await {
                    tracing::warn!(?error, "couldn't enqueue due messages");
                }
                let holding: Vec<Uuid> = busy.values().copied().collect();
                match deliveries::claim(&pool, &holding).await {
                    Ok(claimed) => {
                        for delivery in claimed {
                            let user_id = delivery.user_id;
                            let send = sends.spawn(deliver(pool.clone(), sender.clone(), delivery));
                            busy.insert(send.id(), user_id);
                        }
                    }
                    Err(error) => tracing::warn!(?error, "couldn't claim deliveries"),
                }
            }
        }
    }
    let _ = timeout(DRAIN_LIMIT, sends.join_all()).await;
}

/// Queues everything due by `now` and moves each schedule on, in one transaction: a crash can't
/// leave a schedule that queued a send and would queue it again. A run of an agent's schedule is
/// decided again first, and one its connection may no longer make is recorded as skipped.
async fn enqueue_due(
    pool: &PgPool,
    cipher: &Cipher,
    now: DateTime<Utc>,
    agent_access: bool,
) -> sqlx::Result<usize> {
    let mut transaction = pool.begin().await?;
    let due = scheduled_messages::lock_due(&mut *transaction, now, DUE_PER_TICK).await?;
    let mut queued = 0;
    for message in &due {
        let firing = match message.connection_id {
            None => Firing::Send {
                connection_id: None,
            },
            Some(connection_id) => {
                let chat = ChatRef {
                    recipient_type: message.recipient_type,
                    recipient: &message.recipient,
                };
                firing::check(
                    &mut transaction,
                    agent_access,
                    message.user_id,
                    connection_id,
                    chat,
                )
                .await?
            }
        };
        let connection_id = match &firing {
            Firing::Later => continue,
            Firing::Send { connection_id } => *connection_id,
            Firing::Skip(_) => message.connection_id,
        };
        let to = variables::To {
            recipient_type: message.recipient_type,
            recipient: &message.recipient,
            recipient_name: message.recipient_name.as_deref(),
        };
        let due = variables::Due {
            at: message.next_run_at.unwrap_or(now),
            timezone: &message.timezone,
            place: message.recurrence.as_ref().and_then(|json| json.place()),
        };
        // Filled in now, so History shows what went out and a retry sends the same text
        let body = variables::fill_in(
            &mut *transaction,
            cipher,
            message.user_id,
            &to,
            &message.body,
            &due,
        )
        .await;
        let delivery = NewDelivery {
            user_id: message.user_id,
            scheduled_message_id: Some(message.id),
            connection_id,
            recipient_type: message.recipient_type,
            recipient: &message.recipient,
            recipient_name: message.recipient_name.as_deref(),
            body: &body,
        };
        match &firing {
            Firing::Skip(why) => {
                deliveries::create_skipped(&mut *transaction, delivery, why).await?;
            }
            Firing::Send { .. } | Firing::Later => {
                deliveries::create(&mut *transaction, delivery).await?;
            }
        }
        let next_run_at = message
            .recurrence
            .as_ref()
            .and_then(|recurrence| recurrence.next_after(&message.timezone, now));
        scheduled_messages::advance(&mut *transaction, message.id, next_run_at, now).await?;
        queued += 1;
    }
    transaction.commit().await?;
    Ok(queued)
}

/// Sends one delivery and records how it went
async fn deliver(pool: PgPool, sender: impl Sender, delivery: Claimed) {
    let Claimed {
        id,
        user_id,
        connection_id,
        recipient_type,
        recipient,
        body,
        attempts,
    } = delivery;
    let to = match (recipient_type, connection_id) {
        // An agent's rules name the number as stored, so its send may not reach another spelling
        (RecipientType::Contact, Some(_)) => Recipient::ExactNumber(recipient),
        (RecipientType::Contact, None) => Recipient::Number(recipient),
        (RecipientType::Group, _) => Recipient::Group(recipient),
    };
    let outcome = sender.send(user_id, to, body).await;
    if let Err(failure) = &outcome {
        tracing::warn!(delivery = %id, %user_id, ?failure, attempts, "a send failed");
    }
    if let Err(error) = record(&pool, id, attempts, &outcome).await {
        tracing::error!(delivery = %id, ?error, "couldn't record a send's outcome");
    }
}

async fn record(
    pool: &PgPool,
    id: Uuid,
    attempts: i32,
    outcome: &Result<(), SendFailure>,
) -> sqlx::Result<()> {
    match outcome {
        Ok(()) => deliveries::mark_sent(pool, id).await,
        Err(failure) => match retry_wait(failure, attempts) {
            Some(wait) => {
                let at = Utc::now() + TimeDelta::from_std(wait).unwrap_or_default();
                deliveries::retry_at(pool, id, at, &failure.to_string()).await
            }
            None => deliveries::mark_failed(pool, id, &failure.to_string()).await,
        },
    }
}

/// How long to wait after try number `attempts`, unless that was the last
fn retry_wait(failure: &SendFailure, attempts: i32) -> Option<Duration> {
    if !failure.can_retry() {
        return None;
    }
    let tried = usize::try_from(attempts).ok()?;
    RETRY_WAITS.get(tried.checked_sub(1)?).copied()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use chrono::TimeZone;

    use super::*;
    use crate::{
        agents::{
            Preset,
            grant::{Chat, ChatRule, Scope, SendLevel},
        },
        db::agent_connections,
    };

    #[derive(Clone)]
    struct FakeSender {
        sent: Arc<Mutex<Vec<(Uuid, String)>>>,
        /// Whether each send asked for its number exactly
        exact: Arc<Mutex<Vec<bool>>>,
        answers: Arc<Mutex<Vec<Result<(), SendFailure>>>>,
    }

    impl FakeSender {
        fn answering(answers: Vec<Result<(), SendFailure>>) -> Self {
            Self {
                sent: Arc::default(),
                exact: Arc::default(),
                answers: Arc::new(Mutex::new(answers)),
            }
        }
    }

    impl Sender for FakeSender {
        async fn ready(&self) {
            tokio::task::yield_now().await;
        }

        async fn send(
            &self,
            user_id: Uuid,
            to: Recipient,
            _body: String,
        ) -> Result<(), SendFailure> {
            tokio::task::yield_now().await;
            self.exact
                .lock()
                .unwrap()
                .push(matches!(to, Recipient::ExactNumber(_)));
            let number = match to {
                Recipient::Number(number)
                | Recipient::ExactNumber(number)
                | Recipient::Group(number) => number,
            };
            self.sent.lock().unwrap().push((user_id, number));
            let mut answers = self.answers.lock().unwrap();
            if answers.is_empty() {
                Ok(())
            } else {
                answers.remove(0)
            }
        }
    }

    async fn user(pool: &PgPool) -> Uuid {
        sqlx::query_scalar!(
            "INSERT INTO users (email, password) VALUES ($1, 'unused') RETURNING id",
            format!("{}@example.com", Uuid::new_v4()),
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    fn past() -> DateTime<Utc> {
        Utc::now() - TimeDelta::minutes(5)
    }

    async fn once_due(pool: &PgPool, owner: Uuid) -> Uuid {
        sqlx::query_scalar!(
            "INSERT INTO scheduled_messages (user_id, recipient_type, recipient, body, \
             schedule_kind, run_at, timezone, next_run_at) \
             VALUES ($1, 'contact', '15550001', 'hello', 'once', $2, 'UTC', $2) RETURNING id",
            owner,
            past(),
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[sqlx::test]
    async fn variables_are_filled_in_as_the_message_is_queued(pool: PgPool) {
        use chrono::TimeZone;

        use crate::{
            db::whatsapp_accounts,
            whatsapp::contacts::{Names, seal_names},
        };

        let cipher = Cipher::new(&[0; 32]);
        let owner = user(&pool).await;
        let account_id = whatsapp_accounts::find_or_create(&pool, owner)
            .await
            .unwrap()
            .id;
        let jid = "15550001@s.whatsapp.net";
        let names = Names {
            full_name: Some("Amina Rahman".into()),
            first_name: None,
            username: None,
        };
        let sealed = seal_names(&cipher, account_id, jid, &names).unwrap();
        sqlx::query!(
            "INSERT INTO whatsapp_contacts (account_id, jid, phone_number, names) \
             VALUES ($1, $2, '15550001', $3)",
            account_id,
            jid,
            sealed,
        )
        .execute(&pool)
        .await
        .unwrap();
        // 21:05 in London on a Friday
        let due = Utc.with_ymd_and_hms(2026, 11, 27, 21, 5, 0).unwrap();
        for (recipient, body) in [
            ("15550001", "Hi {{first_name}}, see you {{day}} at {{time}}"),
            ("15550002", "Hi {{first_name|there}}, it's {{date}}"),
        ] {
            sqlx::query!(
                "INSERT INTO scheduled_messages (user_id, recipient_type, recipient, body, \
                 schedule_kind, run_at, timezone, next_run_at) \
                 VALUES ($1, 'contact', $2, $3, 'once', $4, 'Europe/London', $4)",
                owner,
                recipient,
                body,
                due,
            )
            .execute(&pool)
            .await
            .unwrap();
        }

        let now = due + TimeDelta::minutes(1);
        assert_eq!(enqueue_due(&pool, &cipher, now, true).await.unwrap(), 2);
        let mut sent = sqlx::query_scalar!("SELECT body FROM deliveries WHERE user_id = $1", owner)
            .fetch_all(&pool)
            .await
            .unwrap();
        sent.sort();
        assert_eq!(
            sent,
            [
                "Hi Amina, see you Friday at 9:05 PM",
                "Hi there, it's 27 November",
            ]
        );
    }

    async fn recurring_due(pool: &PgPool, owner: Uuid, cron: &str) -> Uuid {
        sqlx::query_scalar!(
            "INSERT INTO scheduled_messages (user_id, recipient_type, recipient, body, \
             schedule_kind, recurrence, timezone, next_run_at) \
             VALUES ($1, 'contact', '15550001', 'hello', 'recurring', \
                     jsonb_build_object('type', 'cron', 'expression', $2::text), 'UTC', $3) \
             RETURNING id",
            owner,
            cron,
            past(),
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn queued(pool: &PgPool, owner: Uuid) -> Uuid {
        deliveries::create(
            pool,
            NewDelivery {
                user_id: owner,
                scheduled_message_id: None,
                connection_id: None,
                recipient_type: RecipientType::Contact,
                recipient: "15550001",
                recipient_name: None,
                body: "hello",
            },
        )
        .await
        .unwrap()
    }

    struct Row {
        status: String,
        attempts: i32,
        error: Option<String>,
        waits: bool,
    }

    async fn row(pool: &PgPool, id: Uuid) -> Row {
        sqlx::query_as!(
            Row,
            r#"SELECT status, attempts, error, next_attempt_at > now() AS "waits!"
               FROM deliveries WHERE id = $1"#,
            id
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn claim_one(pool: &PgPool) -> Claimed {
        deliveries::claim(pool, &[]).await.unwrap().pop().unwrap()
    }

    #[sqlx::test]
    async fn a_firing_schedule_carries_its_agent_connection(pool: PgPool) {
        let owner = user(&pool).await;
        let connection = agent(&pool, owner, &full_access()).await;
        let by_agent = once_due(&pool, owner).await;
        once_due(&pool, owner).await;
        sqlx::query!(
            "UPDATE scheduled_messages SET connection_id = $1 WHERE id = $2",
            connection,
            by_agent
        )
        .execute(&pool)
        .await
        .unwrap();

        enqueue_due(&pool, &Cipher::new(&[0; 32]), Utc::now(), true)
            .await
            .unwrap();

        let mut rows = sqlx::query!(
            "SELECT scheduled_message_id, connection_id FROM deliveries WHERE user_id = $1",
            owner
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        rows.sort_by_key(|row| row.scheduled_message_id != Some(by_agent));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].connection_id, Some(connection));
        assert_eq!(rows[1].connection_id, None);
    }

    #[sqlx::test]
    async fn a_one_time_message_queues_once_and_switches_off(pool: PgPool) {
        let owner = user(&pool).await;
        let message = once_due(&pool, owner).await;

        assert_eq!(
            enqueue_due(&pool, &Cipher::new(&[0; 32]), Utc::now(), true)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            enqueue_due(&pool, &Cipher::new(&[0; 32]), Utc::now(), true)
                .await
                .unwrap(),
            0
        );

        let row = scheduled_messages::find(&pool, message, owner)
            .await
            .unwrap()
            .unwrap();
        assert!(!row.enabled);
        assert_eq!(row.next_run_at, None);
        assert!(row.last_run_at.is_some());
        assert_eq!(
            deliveries::count(&pool, owner, deliveries::Filter::default())
                .await
                .unwrap(),
            1
        );
    }

    #[sqlx::test]
    async fn a_recurring_message_moves_to_its_next_match(pool: PgPool) {
        let owner = user(&pool).await;
        let message = recurring_due(&pool, owner, "0 9 * * *").await;
        let now = Utc.with_ymd_and_hms(2030, 1, 1, 12, 0, 0).unwrap();

        assert_eq!(
            enqueue_due(&pool, &Cipher::new(&[0; 32]), now, true)
                .await
                .unwrap(),
            1
        );

        let row = scheduled_messages::find(&pool, message, owner)
            .await
            .unwrap()
            .unwrap();
        assert!(row.enabled);
        assert_eq!(
            row.next_run_at,
            Some(Utc.with_ymd_and_hms(2030, 1, 2, 9, 0, 0).unwrap())
        );
    }

    #[sqlx::test]
    async fn a_sun_schedule_moves_to_the_next_days_sunset_and_reads_the_sun(pool: PgPool) {
        let owner = user(&pool).await;
        // Sunset in Dhaka on 8 November, and just after
        let sunset = Utc.with_ymd_and_hms(2026, 11, 8, 11, 16, 0).unwrap();
        let now = sunset + TimeDelta::minutes(1);
        let message = sqlx::query_scalar!(
            "INSERT INTO scheduled_messages (user_id, recipient_type, recipient, body, \
             schedule_kind, recurrence, timezone, next_run_at) \
             VALUES ($1, 'contact', '15550001', 'Maghrib at {{sunset + 10m}}, iftar at {{time}}', \
                     'recurring', \
                     '{\"type\": \"sun\", \"event\": \"sunset\", \"offset_minutes\": 0, \
                       \"days\": \"* * *\", \"place\": null}', 'Asia/Dhaka', $2) \
             RETURNING id",
            owner,
            sunset,
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!(
            enqueue_due(&pool, &Cipher::new(&[0; 32]), now, true)
                .await
                .unwrap(),
            1
        );

        let sent = sqlx::query_scalar!("SELECT body FROM deliveries WHERE user_id = $1", owner)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(sent, "Maghrib at 5:26 PM, iftar at 5:16 PM");
        let row = scheduled_messages::find(&pool, message, owner)
            .await
            .unwrap()
            .unwrap();
        let next = row.next_run_at.unwrap();
        assert_eq!(next.date_naive(), now.date_naive().succ_opt().unwrap());
        assert_eq!(next.format("%H").to_string(), "11");
    }

    #[sqlx::test]
    async fn a_schedule_that_can_no_longer_match_switches_off(pool: PgPool) {
        let owner = user(&pool).await;
        let message = recurring_due(&pool, owner, "0 0 31 2 *").await;

        assert_eq!(
            enqueue_due(&pool, &Cipher::new(&[0; 32]), Utc::now(), true)
                .await
                .unwrap(),
            1
        );

        let row = scheduled_messages::find(&pool, message, owner)
            .await
            .unwrap()
            .unwrap();
        assert!(!row.enabled);
    }

    #[sqlx::test]
    async fn a_disabled_message_stays_put(pool: PgPool) {
        let owner = user(&pool).await;
        let message = once_due(&pool, owner).await;
        sqlx::query!(
            "UPDATE scheduled_messages SET enabled = false WHERE id = $1",
            message
        )
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(
            enqueue_due(&pool, &Cipher::new(&[0; 32]), Utc::now(), true)
                .await
                .unwrap(),
            0
        );
    }

    #[sqlx::test]
    async fn each_account_gets_one_delivery_at_a_time(pool: PgPool) {
        let (first, second) = (user(&pool).await, user(&pool).await);
        queued(&pool, first).await;
        queued(&pool, first).await;
        queued(&pool, second).await;

        let claimed = deliveries::claim(&pool, &[]).await.unwrap();
        assert_eq!(claimed.len(), 2);

        let claimed = deliveries::claim(&pool, &[first]).await.unwrap();
        assert!(claimed.is_empty());

        let claimed = deliveries::claim(&pool, &[]).await.unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].user_id, first);
    }

    #[sqlx::test]
    async fn a_good_send_is_marked_sent(pool: PgPool) {
        let owner = user(&pool).await;
        let id = queued(&pool, owner).await;
        let sender = FakeSender::answering(vec![Ok(())]);

        deliver(pool.clone(), sender.clone(), claim_one(&pool).await).await;

        assert_eq!(row(&pool, id).await.status, "sent");
        assert_eq!(
            *sender.sent.lock().unwrap(),
            vec![(owner, "15550001".to_owned())]
        );
    }

    #[sqlx::test]
    async fn while_offline_a_send_waits_and_tries_again(pool: PgPool) {
        let owner = user(&pool).await;
        let id = queued(&pool, owner).await;
        let sender = FakeSender::answering(vec![Err(SendFailure::Offline)]);

        deliver(pool.clone(), sender, claim_one(&pool).await).await;

        let waiting = row(&pool, id).await;
        assert_eq!(waiting.status, "pending");
        assert_eq!(waiting.attempts, 1);
        assert!(waiting.waits);
        assert_eq!(waiting.error.as_deref(), Some("WhatsApp wasn't connected"));
        assert!(deliveries::claim(&pool, &[]).await.unwrap().is_empty());
    }

    #[sqlx::test]
    async fn offline_past_the_last_retry_fails(pool: PgPool) {
        let owner = user(&pool).await;
        let id = queued(&pool, owner).await;
        sqlx::query!("UPDATE deliveries SET attempts = 5 WHERE id = $1", id)
            .execute(&pool)
            .await
            .unwrap();
        let sender = FakeSender::answering(vec![Err(SendFailure::Offline)]);

        deliver(pool.clone(), sender, claim_one(&pool).await).await;

        assert_eq!(row(&pool, id).await.status, "failed");
    }

    #[sqlx::test]
    async fn what_may_have_gone_out_is_never_retried(pool: PgPool) {
        let owner = user(&pool).await;
        let id = queued(&pool, owner).await;
        let sender = FakeSender::answering(vec![Err(SendFailure::Unconfirmed)]);

        deliver(pool.clone(), sender, claim_one(&pool).await).await;

        assert_eq!(row(&pool, id).await.status, "failed");
    }

    #[sqlx::test]
    async fn a_failure_only_the_user_can_fix_fails_at_once(pool: PgPool) {
        let owner = user(&pool).await;
        let id = queued(&pool, owner).await;
        let sender = FakeSender::answering(vec![Err(SendFailure::NotOnWhatsApp)]);

        deliver(pool.clone(), sender, claim_one(&pool).await).await;

        let failed = row(&pool, id).await;
        assert_eq!(failed.status, "failed");
        assert_eq!(
            failed.error.as_deref(),
            Some("This number isn't on WhatsApp")
        );
    }

    #[sqlx::test]
    async fn a_send_cut_off_by_a_crash_fails_instead_of_repeating(pool: PgPool) {
        let owner = user(&pool).await;
        let id = queued(&pool, owner).await;
        claim_one(&pool).await;

        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(run(
            pool.clone(),
            Cipher::new(&[0; 32]),
            FakeSender::answering(vec![]),
            true,
            stopped,
        ));
        tokio::time::sleep(Duration::from_millis(200)).await;
        stop.send(()).unwrap();
        task.await.unwrap();

        let failed = row(&pool, id).await;
        assert_eq!(failed.status, "failed");
        assert_eq!(failed.error.as_deref(), Some(INTERRUPTED));
    }

    #[sqlx::test]
    async fn the_loop_sends_what_comes_due(pool: PgPool) {
        let owner = user(&pool).await;
        once_due(&pool, owner).await;
        let sender = FakeSender::answering(vec![]);

        let scheduler = Scheduler::spawn(pool.clone(), Cipher::new(&[0; 32]), sender.clone(), true);
        tokio::time::sleep(Duration::from_millis(300)).await;
        scheduler.shutdown().await;

        assert_eq!(sender.sent.lock().unwrap().len(), 1);
        let status = sqlx::query_scalar!("SELECT status FROM deliveries WHERE user_id = $1", owner)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(status, "sent");
    }

    fn grant_json(grant: &crate::agents::Grant) -> serde_json::Value {
        serde_json::to_value(grant).unwrap()
    }

    fn full_access() -> serde_json::Value {
        grant_json(&Preset::FullAccess.grant())
    }

    /// Full access, with a rule of its own for 15550001
    fn ruled(send: SendLevel, daily_limit: Option<u32>) -> serde_json::Value {
        let mut grant = Preset::FullAccess.grant();
        grant.chats.rules.push(ChatRule {
            chat: Chat {
                recipient_type: RecipientType::Contact,
                id: "15550001".to_owned(),
                name: None,
            },
            send,
            daily_limit,
            note: None,
        });
        grant_json(&grant)
    }

    async fn agent(pool: &PgPool, owner: Uuid, grant: &serde_json::Value) -> Uuid {
        let key_hash = Uuid::new_v4();
        sqlx::query_scalar!(
            r#"INSERT INTO agent_connections (user_id, name, key_hash, key_hint, "grant")
               VALUES ($1, 'Agent', $2, 'abcd', $3) RETURNING id"#,
            owner,
            key_hash.as_bytes().as_slice(),
            grant,
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn made_by(pool: &PgPool, schedule: Uuid, connection: Uuid) {
        sqlx::query!(
            "UPDATE scheduled_messages SET connection_id = $1 WHERE id = $2",
            connection,
            schedule
        )
        .execute(pool)
        .await
        .unwrap();
    }

    /// A user, an agent connection with `grant`, and a one-time message it scheduled, now due
    async fn agents_once(pool: &PgPool, grant: &serde_json::Value) -> (Uuid, Uuid, Uuid) {
        let owner = user(pool).await;
        let connection = agent(pool, owner, grant).await;
        let schedule = once_due(pool, owner).await;
        made_by(pool, schedule, connection).await;
        (owner, connection, schedule)
    }

    #[derive(Debug)]
    struct Fired {
        status: String,
        skipped: bool,
        error: Option<String>,
        connection_id: Option<Uuid>,
    }

    async fn fired(pool: &PgPool, schedule: Uuid) -> Vec<Fired> {
        sqlx::query_as!(
            Fired,
            "SELECT status, skipped, error, connection_id FROM deliveries \
             WHERE scheduled_message_id = $1",
            schedule
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    async fn sent_by(pool: &PgPool, owner: Uuid, connection: Uuid, count: i32) {
        sqlx::query!(
            "INSERT INTO deliveries (user_id, connection_id, recipient_type, recipient, body) \
             SELECT $1, $2, 'contact', '15550001', 'earlier' FROM generate_series(1, $3)",
            owner,
            connection,
            count,
        )
        .execute(pool)
        .await
        .unwrap();
    }

    async fn enqueue(pool: &PgPool, agent_access: bool) -> usize {
        enqueue_due(pool, &Cipher::new(&[0; 32]), Utc::now(), agent_access)
            .await
            .unwrap()
    }

    #[sqlx::test]
    async fn an_agents_schedule_it_may_still_send_fires(pool: PgPool) {
        let (_, connection, schedule) = agents_once(&pool, &full_access()).await;

        assert_eq!(enqueue(&pool, true).await, 1);

        let fired = fired(&pool, schedule).await;
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].status, "pending");
        assert!(!fired[0].skipped);
        assert_eq!(fired[0].error, None);
        assert_eq!(fired[0].connection_id, Some(connection));
    }

    #[sqlx::test]
    async fn a_rule_of_ask_doesnt_ask_again_when_the_schedule_fires(pool: PgPool) {
        let (_, _, schedule) = agents_once(&pool, &ruled(SendLevel::Ask, None)).await;

        enqueue(&pool, true).await;

        let fired = fired(&pool, schedule).await;
        assert_eq!(fired[0].status, "pending");
        assert!(!fired[0].skipped);
    }

    #[derive(Clone, Copy, Debug)]
    enum Stop {
        Paused,
        Expired,
        SwitchOff,
        Unreadable,
        RuleSaysNo,
        OutOfScope,
        ChatLimit,
        Ceiling,
    }

    #[sqlx::test]
    async fn a_run_its_agent_may_no_longer_make_is_skipped_and_says_why(pool: PgPool) {
        let mut selected = Preset::FullAccess.grant();
        selected.chats.scope = Scope::Selected;
        let cases = [
            (
                Stop::Paused,
                full_access(),
                "the agent that scheduled this is paused",
            ),
            (
                Stop::Expired,
                full_access(),
                "the agent that scheduled this has expired",
            ),
            (
                Stop::SwitchOff,
                full_access(),
                "agent access is switched off",
            ),
            (
                Stop::Unreadable,
                serde_json::json!({ "version": 99 }),
                "couldn't be read",
            ),
            (
                Stop::RuleSaysNo,
                ruled(SendLevel::No, None),
                "no longer allowed to send to this chat",
            ),
            (
                Stop::OutOfScope,
                grant_json(&selected),
                "no longer one of the chats",
            ),
            (
                Stop::ChatLimit,
                ruled(SendLevel::Yes, Some(2)),
                "limit of 2 messages a day to this chat",
            ),
            (Stop::Ceiling, full_access(), "has sent 200 messages"),
        ];
        for (stop, grant, why) in cases {
            let (owner, connection, schedule) = agents_once(&pool, &grant).await;
            match stop {
                Stop::Paused => {
                    agent_connections::pause(&pool, connection, owner)
                        .await
                        .unwrap();
                }
                Stop::Expired => {
                    sqlx::query!(
                        "UPDATE agent_connections SET expires_at = now() - interval '1 minute' \
                         WHERE id = $1",
                        connection
                    )
                    .execute(&pool)
                    .await
                    .unwrap();
                }
                Stop::ChatLimit => sent_by(&pool, owner, connection, 2).await,
                Stop::Ceiling => sent_by(&pool, owner, connection, 200).await,
                Stop::SwitchOff | Stop::Unreadable | Stop::RuleSaysNo | Stop::OutOfScope => {}
            }

            assert_eq!(
                enqueue(&pool, !matches!(stop, Stop::SwitchOff)).await,
                1,
                "{stop:?}"
            );

            let fired = fired(&pool, schedule).await;
            assert_eq!(fired.len(), 1, "{stop:?}");
            assert_eq!(fired[0].status, "failed", "{stop:?}");
            assert!(fired[0].skipped, "{stop:?}");
            assert_eq!(fired[0].connection_id, Some(connection), "{stop:?}");
            let error = fired[0].error.as_deref().unwrap();
            assert!(error.starts_with("Not sent: "), "{stop:?}: {error}");
            assert!(error.contains(why), "{stop:?}: {error}");
            let row = scheduled_messages::find(&pool, schedule, owner)
                .await
                .unwrap()
                .unwrap();
            assert!(!row.enabled, "{stop:?}");
            assert_eq!(row.next_run_at, None, "{stop:?}");
            assert!(row.last_run_at.is_some(), "{stop:?}");
        }
    }

    #[sqlx::test]
    async fn a_skipped_recurring_run_moves_to_its_next_match(pool: PgPool) {
        let owner = user(&pool).await;
        let connection = agent(&pool, owner, &full_access()).await;
        let schedule = recurring_due(&pool, owner, "0 9 * * *").await;
        made_by(&pool, schedule, connection).await;
        agent_connections::pause(&pool, connection, owner)
            .await
            .unwrap();
        let now = Utc.with_ymd_and_hms(2030, 1, 1, 12, 0, 0).unwrap();

        enqueue_due(&pool, &Cipher::new(&[0; 32]), now, true)
            .await
            .unwrap();

        assert!(fired(&pool, schedule).await[0].skipped);
        let row = scheduled_messages::find(&pool, schedule, owner)
            .await
            .unwrap()
            .unwrap();
        assert!(row.enabled);
        assert_eq!(
            row.next_run_at,
            Some(Utc.with_ymd_and_hms(2030, 1, 2, 9, 0, 0).unwrap())
        );
    }

    #[sqlx::test]
    async fn skipped_runs_dont_count_toward_limits_and_failed_sends_do(pool: PgPool) {
        let owner = user(&pool).await;
        let connection = agent(&pool, owner, &full_access()).await;
        for _ in 0..3 {
            let schedule = once_due(&pool, owner).await;
            made_by(&pool, schedule, connection).await;
        }
        agent_connections::pause(&pool, connection, owner)
            .await
            .unwrap();
        assert_eq!(enqueue(&pool, true).await, 3);

        let counted = || async {
            let counts =
                agent_connections::counts(&pool, connection, RecipientType::Contact, "15550001")
                    .await
                    .unwrap();
            let (sends, _) = agent_connections::usage(&pool, connection).await.unwrap();
            let all = agent_connections::usage_all(&pool, owner).await.unwrap()[&connection].0;
            (counts.chat_sends, counts.connection_sends, sends, all)
        };
        assert_eq!(counted().await, (0, 0, 0, 0));

        sent_by(&pool, owner, connection, 1).await;
        sqlx::query!(
            "UPDATE deliveries SET status = 'failed', attempts = 1, error = 'WhatsApp said no' \
             WHERE connection_id = $1 AND NOT skipped",
            connection
        )
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(counted().await, (1, 1, 1, 1));
    }

    #[sqlx::test]
    async fn a_revoked_agents_kept_schedule_fires_as_the_owners(pool: PgPool) {
        let (owner, connection, schedule) = agents_once(&pool, &full_access()).await;
        agent_connections::delete(&pool, connection, owner, false)
            .await
            .unwrap();

        assert_eq!(enqueue(&pool, false).await, 1);

        let fired = fired(&pool, schedule).await;
        assert_eq!(fired[0].status, "pending");
        assert!(!fired[0].skipped);
        assert_eq!(fired[0].connection_id, None);
    }

    #[sqlx::test]
    async fn the_owners_schedules_fire_whatever_agents_may_do(pool: PgPool) {
        let owner = user(&pool).await;
        let connection = agent(&pool, owner, &full_access()).await;
        agent_connections::pause(&pool, connection, owner)
            .await
            .unwrap();
        let schedule = once_due(&pool, owner).await;

        assert_eq!(enqueue(&pool, false).await, 1);

        let fired = fired(&pool, schedule).await;
        assert_eq!(fired[0].status, "pending");
        assert_eq!(fired[0].connection_id, None);
    }

    #[sqlx::test]
    async fn a_run_waits_while_a_call_of_its_connection_holds_the_lock(pool: PgPool) {
        let (owner, connection, schedule) = agents_once(&pool, &full_access()).await;
        let mut call = pool.begin().await.unwrap();
        agent_connections::lock(&mut *call, connection)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(enqueue(&pool, true).await, 0);
        assert!(fired(&pool, schedule).await.is_empty());
        let row = scheduled_messages::find(&pool, schedule, owner)
            .await
            .unwrap()
            .unwrap();
        assert!(row.enabled);

        call.commit().await.unwrap();
        assert_eq!(enqueue(&pool, true).await, 1);
        assert_eq!(fired(&pool, schedule).await[0].status, "pending");
    }

    #[sqlx::test]
    async fn an_agents_message_to_a_number_goes_only_to_that_number(pool: PgPool) {
        let owner = user(&pool).await;
        let connection = agent(&pool, owner, &full_access()).await;
        queued(&pool, owner).await;
        let sender = FakeSender::answering(vec![]);
        deliver(pool.clone(), sender.clone(), claim_one(&pool).await).await;
        sent_by(&pool, owner, connection, 1).await;
        deliver(pool.clone(), sender.clone(), claim_one(&pool).await).await;

        assert_eq!(*sender.exact.lock().unwrap(), [false, true]);
    }

    #[test]
    fn a_group_has_one_spelling() {
        let group = RecipientType::Group;
        for id in [
            "120363000000000000@g.us",
            "8801700000000-1600000000@g.us",
            " 120363000000000000@g.us ",
        ] {
            assert_eq!(group.problem(id), None, "{id}");
        }
        assert_eq!(
            group.normalized(" 120363000000000000@g.us "),
            "120363000000000000@g.us"
        );
        for id in [
            "120363000000000000:1@g.us",
            "120363000000000000:0@g.us",
            "120363000000000000.1@g.us",
            "120363000000000000.0:0@g.us",
            "0120363000000000000@g.us",
            "120363000000000000@G.US",
            "120363000000000000@g.us@g.us",
            "120363000000000000@s.whatsapp.net",
            "-1600000000@g.us",
            "8801700000000-@g.us",
            "1-2-3@g.us",
            "abc@g.us",
            "@g.us",
        ] {
            assert!(group.problem(id).is_some(), "{id}");
        }
    }

    #[test]
    fn a_number_with_the_international_prefix_is_the_same_number() {
        let contact = RecipientType::Contact;
        for spelling in [
            "8801700000001",
            "+880 1700-000001",
            "008801700000001",
            "+00 8801700000001",
        ] {
            assert_eq!(contact.problem(spelling), None, "{spelling}");
            assert_eq!(contact.normalized(spelling), "8801700000001", "{spelling}");
        }
        assert_eq!(
            contact.problem("01700000001"),
            Some("must start with its country code, not with 0")
        );
        assert_eq!(
            contact.problem("000801700000001"),
            Some("must start with its country code, not with 0")
        );
    }
}
