//! One task per user's WhatsApp account. It runs the account's client, feeds the client's events
//! and the user's commands through [`status::next`], and carries out the effects in order.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError, RwLock},
    time::Duration,
};

use anyhow::anyhow;
use chrono::{DateTime, TimeDelta, Utc};
use sqlx::PgPool;
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::AbortHandle,
    time::{Instant, sleep_until, timeout},
};
use url::Url;
use uuid::Uuid;
use whatsapp_rust::{
    CacheConfig, MsgSecretPolicy, PresencePolicy, SendError, SendOptions, SendRequest,
    http::{HttpClient, UreqHttpClient},
    pair_code::PairCodeOptions,
    prelude::{
        Bot, BotHandle, Client, DeregistrationOutcome, Event as LibraryEvent, EventKind, Jid, wa,
    },
    wacore::{proto_helpers::MessageBuilderExt, store::DevicePropsOverride},
};

use super::{
    alerts,
    contacts::{self, Changes, Feed},
    pictures::{Image, Pictures},
    status::{self, Alert, Code, Effect, Event, Method, Refused, Status, StopReason},
    store::PgStore,
};
use crate::{crypto::Cipher, db::whatsapp_accounts, email::Mailer};

/// Shown on the phone as "Google Chrome (Whatomato)" after QR linking
const DEVICE_NAME: &str = "Whatomato";
/// How long linking may take either way; QR codes alone run out after about 160 seconds
const PAIRING_LIMIT: Duration = Duration::from_mins(5);
/// A graceful stop flushes the device and pending writes; past this the client is aborted
const STOP_LIMIT: Duration = Duration::from_secs(15);
/// How long an unlink waits for a stopped client to come online to tell WhatsApp
const LOGOUT_CONNECT_LIMIT: Duration = Duration::from_secs(20);
const FIRST_RESTART_DELAY: Duration = Duration::from_secs(5);
const MAX_RESTART_DELAY: Duration = Duration::from_mins(10);
/// For listing groups and sending through a connected client
const REQUEST_LIMIT: Duration = Duration::from_secs(30);
/// From handing a message to the library to WhatsApp acknowledging it
const SEND_LIMIT: Duration = Duration::from_secs(20);
const TEST_MESSAGE: &str = "Whatomato test: your WhatsApp is linked. You can delete this.";
/// Upstream's `manual_presence` example found a daily window this long clears the stale activity and
/// expiry warning a never-online device shows on the phone; it's empirical, not a WhatsApp rule
const PRESENCE_PULSE: Duration = Duration::from_secs(45);
const PRESENCE_EVERY: Duration = Duration::from_hours(24);
/// The first pulse after connecting comes this long at most, spread by user
const FIRST_PULSE_WITHIN: Duration = Duration::from_hours(6);
/// WhatsApp logs out a device offline for about 30 days, by secondary sources; its help pages don't
/// pin the number down, so a paused account shows up well before that
const KEEPALIVE_EVERY: Duration = Duration::from_hours(7 * 24);
const KEEPALIVE_RETRY: Duration = Duration::from_hours(1);
/// Long enough to connect, take in what queued up and finish a presence pulse
const KEEPALIVE_WINDOW: Duration = Duration::from_mins(2);

const WATCHED: [EventKind; 15] = [
    EventKind::PairingQrCode,
    EventKind::PairingCode,
    EventKind::PairingCodeRefresh,
    EventKind::PairingCodeError,
    EventKind::PairingQrCodesExhausted,
    EventKind::PairSuccess,
    EventKind::PairError,
    EventKind::Connected,
    EventKind::Disconnected,
    EventKind::LoggedOut,
    EventKind::StreamReplaced,
    EventKind::TemporaryBan,
    EventKind::ClientOutdated,
    EventKind::ConnectFailure,
    EventKind::ServerAck,
];

/// What every supervisor shares
#[derive(Clone)]
pub struct Context {
    pub pool: PgPool,
    pub cipher: Cipher,
    pub mailer: Mailer,
    pub app_url: Arc<Url>,
    pub(super) http: Arc<dyn HttpClient>,
    /// The WhatsApp Web version fetched for every client, the last one that worked; until the
    /// first fetch, each client fetches its own
    pub(super) version: Arc<RwLock<Option<(u32, u32, u32)>>>,
}

impl Context {
    #[must_use]
    pub fn new(pool: PgPool, cipher: Cipher, mailer: Mailer, app_url: Arc<Url>) -> Self {
        Self {
            pool,
            cipher,
            mailer,
            app_url,
            http: Arc::new(UreqHttpClient::new()),
            version: Arc::default(),
        }
    }
}

#[derive(Debug)]
pub enum CommandError {
    /// The command doesn't fit the account's status
    Refused,
    /// Listing groups and sending need a connected client
    NotConnected,
    /// The supervisor has ended, which only happens at shutdown
    Gone,
    Failed(anyhow::Error),
}

pub struct Group {
    pub id: String,
    pub name: String,
}

/// Where a message goes
#[derive(Debug)]
pub enum Recipient {
    /// A phone number's digits
    Number(String),
    /// A group's id
    Group(String),
}

/// Why a message didn't go out. The text shows in the delivery history, so it never carries the
/// library's own errors, which name numbers.
#[derive(Debug, thiserror::Error)]
pub enum SendFailure {
    #[error("WhatsApp wasn't connected")]
    Offline,
    #[error("WhatsApp couldn't take this message")]
    Unreachable,
    #[error("WhatsApp isn't linked")]
    NotLinked,
    #[error("WhatsApp is paused")]
    Paused,
    #[error("WhatsApp has stopped; reconnect it on the WhatsApp page")]
    Stopped,
    #[error("This number isn't on WhatsApp")]
    NotOnWhatsApp,
    #[error("WhatsApp rejected this message (error {0})")]
    Rejected(String),
    #[error("WhatsApp didn't confirm this message, so it may or may not have gone out")]
    Unconfirmed,
    #[error("This message couldn't be sent")]
    Failed,
}

impl SendFailure {
    /// Whether another try could work, and can't repeat a message that went out
    #[must_use]
    pub const fn can_retry(&self) -> bool {
        matches!(self, Self::Offline | Self::Unreachable)
    }

    /// The refusal for a send the account can't take on: the failures of [`Self::for_status`]
    #[must_use]
    pub const fn unavailable(&self) -> &'static str {
        match self {
            Self::Paused => "WhatsApp is paused",
            Self::Stopped => "WhatsApp has stopped; reconnect it on the WhatsApp page",
            _ => "WhatsApp isn't linked",
        }
    }

    /// Why an account in `status` can't send, if it's for the user to fix
    #[must_use]
    pub const fn for_status(status: &Status) -> Option<Self> {
        match status {
            Status::Connected { .. } | Status::Syncing { .. } | Status::Reconnecting { .. } => None,
            Status::Paused { .. } => Some(Self::Paused),
            Status::Stopped { .. } => Some(Self::Stopped),
            Status::NotLinked | Status::LinkEnded(_) | Status::Pairing(_) => Some(Self::NotLinked),
        }
    }
}

/// Every error here means the message never reached the wire, so trying again can't repeat it
impl From<SendError> for SendFailure {
    fn from(error: SendError) -> Self {
        match error {
            SendError::Client(_) | SendError::NotLoggedIn => Self::Offline,
            SendError::Iq(_)
            | SendError::NoRecipientDevice(_)
            | SendError::PrimaryDeviceRejected(_) => Self::Unreachable,
            _ => Self::Failed,
        }
    }
}

/// Sends waiting for WhatsApp's acknowledgement, by message id. `send` returning `Ok` proves nothing:
/// the library resolves it even for a send WhatsApp goes on to reject.
#[derive(Clone, Default)]
struct Acks(Arc<Mutex<HashMap<String, oneshot::Sender<Option<String>>>>>);

impl Acks {
    /// Resolves with WhatsApp's error code, if it rejected the message
    fn expect(&self, id: &str) -> oneshot::Receiver<Option<String>> {
        let (settle, settled) = oneshot::channel();
        self.lock().insert(id.to_owned(), settle);
        settled
    }

    fn settle(&self, id: &str, error: Option<String>) {
        let waiting = self.lock().remove(id);
        if let Some(waiting) = waiting {
            let _ = waiting.send(error);
        }
    }

    fn forget(&self, id: &str) {
        self.lock().remove(id);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, oneshot::Sender<Option<String>>>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Talks to one account's supervisor; the task ends when it's told to shut down or every clone of
/// this is dropped
#[derive(Clone)]
pub struct Handle {
    commands: mpsc::Sender<Command>,
    status: watch::Receiver<Status>,
}

enum Command {
    /// Link, Cancel, Pause or Resume
    Change(Event, oneshot::Sender<Result<(), CommandError>>),
    Unlink(oneshot::Sender<Result<bool, CommandError>>),
    Groups(oneshot::Sender<Result<Vec<Group>, CommandError>>),
    Picture(Jid, oneshot::Sender<Option<Image>>),
    SendTestMessage(oneshot::Sender<Result<(), CommandError>>),
    Send(Recipient, String, oneshot::Sender<Result<(), SendFailure>>),
    Shutdown(oneshot::Sender<()>),
}

impl Handle {
    #[must_use]
    pub fn status(&self) -> Status {
        self.status.borrow().clone()
    }

    /// # Errors
    /// [`CommandError::Refused`] unless the account is unlinked; `Failed` if no client could start.
    pub async fn link(&self, method: Method) -> Result<(), CommandError> {
        self.change(Event::Link(method)).await
    }

    /// # Errors
    /// [`CommandError::Refused`] unless the account is pairing.
    pub async fn cancel(&self) -> Result<(), CommandError> {
        self.change(Event::Cancel).await
    }

    /// # Errors
    /// [`CommandError::Refused`] unless a client runs for a linked device.
    pub async fn pause(&self) -> Result<(), CommandError> {
        self.change(Event::Pause).await
    }

    /// # Errors
    /// [`CommandError::Refused`] unless the account is paused or stopped.
    pub async fn resume(&self) -> Result<(), CommandError> {
        self.change(Event::Resume).await
    }

    /// Whether WhatsApp confirmed removing the device. When it didn't, the user has to remove it
    /// under Linked devices on the phone.
    ///
    /// # Errors
    /// [`CommandError::Refused`] while nothing is linked.
    pub async fn unlink(&self) -> Result<bool, CommandError> {
        self.ask(Command::Unlink).await?
    }

    /// # Errors
    /// [`CommandError::NotConnected`] unless connected; `Failed` if WhatsApp doesn't answer.
    pub async fn groups(&self) -> Result<Vec<Group>, CommandError> {
        self.ask(Command::Groups).await?
    }

    /// A contact's or group's picture: from memory, else from WhatsApp while connected
    pub async fn picture(&self, jid: Jid) -> Option<Image> {
        self.ask(|reply| Command::Picture(jid, reply))
            .await
            .ok()
            .flatten()
    }

    /// Sends a short text to the account's own number, to prove the link works
    ///
    /// # Errors
    /// [`CommandError::NotConnected`] unless connected; `Failed` if the send fails.
    pub async fn send_test_message(&self) -> Result<(), CommandError> {
        self.ask(Command::SendTestMessage).await?
    }

    /// Sends `body`, and answers once WhatsApp has acknowledged it
    ///
    /// # Errors
    /// Why it didn't go out, or may not have.
    pub async fn send_text(&self, to: Recipient, body: String) -> Result<(), SendFailure> {
        self.ask(|reply| Command::Send(to, body, reply))
            .await
            .map_err(|_| SendFailure::Offline)?
    }

    /// Stops the client and ends the task. The link stays for the next start.
    pub async fn shutdown(&self) {
        let _ = self.ask(Command::Shutdown).await;
    }

    async fn change(&self, event: Event) -> Result<(), CommandError> {
        self.ask(|reply| Command::Change(event, reply)).await?
    }

    async fn ask<T>(
        &self,
        command: impl FnOnce(oneshot::Sender<T>) -> Command,
    ) -> Result<T, CommandError> {
        let (reply, answer) = oneshot::channel();
        self.commands
            .send(command(reply))
            .await
            .map_err(|_| CommandError::Gone)?;
        answer.await.map_err(|_| CommandError::Gone)
    }
}

/// Starts the supervisor for `user_id`'s account, if any, which is in `status`.
///
/// A linked account that isn't paused connects after `start_in`, so a boot can space out its
/// connections, and a paused one runs its keepalive at `keepalive_at`, `start_in` at the soonest.
/// No client starts before `may_connect` turns true.
#[must_use]
pub fn spawn(
    context: Context,
    user_id: Uuid,
    account_id: Option<Uuid>,
    status: Status,
    start_in: Duration,
    keepalive_at: Option<DateTime<Utc>>,
    may_connect: watch::Receiver<bool>,
) -> Handle {
    let restart_at =
        matches!(status, Status::Reconnecting { .. }).then(|| Instant::now() + start_in);
    let keepalive_at = keepalive_at
        .filter(|_| matches!(status, Status::Paused { .. }))
        .map(|at| {
            let wait = (at - Utc::now()).to_std().unwrap_or_default();
            Instant::now() + wait.max(start_in)
        });
    let (commands_tx, commands) = mpsc::channel(16);
    let (status_tx, status_rx) = watch::channel(status);
    let (events_tx, events) = mpsc::unbounded_channel();
    let contacts = contacts::spawn_writer(context.pool.clone(), context.cipher.clone());
    tokio::spawn(
        Supervisor {
            context,
            contacts,
            pictures: Arc::new(Pictures::new()),
            user_id,
            account_id,
            status: status_tx,
            commands,
            events_tx,
            events,
            may_connect,
            acks: Acks::default(),
            client: None,
            generation: 0,
            restarts: 0,
            restart_at,
            pairing_deadline: None,
            keepalive_at,
            keepalive_ends: None,
            keepalive_connected: false,
        }
        .run(),
    );
    Handle {
        commands: commands_tx,
        status: status_rx,
    }
}

struct Supervisor {
    context: Context,
    contacts: Changes,
    pictures: Arc<Pictures>,
    user_id: Uuid,
    /// `None` until the first link, and again once an unlink deletes the row
    account_id: Option<Uuid>,
    status: watch::Sender<Status>,
    commands: mpsc::Receiver<Command>,
    // Unbounded: a callback blocked on a full channel could stall the client the loop waits on
    events_tx: mpsc::UnboundedSender<(u64, Event)>,
    events: mpsc::UnboundedReceiver<(u64, Event)>,
    /// Turns true once this server holds the lock that keeps other servers' clients off these keys
    may_connect: watch::Receiver<bool>,
    acks: Acks,
    client: Option<Running>,
    /// Counts the clients started, so events from an older one are ignored
    generation: u64,
    restarts: u32,
    restart_at: Option<Instant>,
    pairing_deadline: Option<Instant>,
    keepalive_at: Option<Instant>,
    /// When the keepalive's client stops, while one runs
    keepalive_ends: Option<Instant>,
    keepalive_connected: bool,
}

struct Running {
    handle: BotHandle,
    client: Arc<Client>,
    presence: Option<AbortOnDrop>,
    contacts_import: Option<AbortOnDrop>,
}

impl Running {
    fn new(handle: BotHandle) -> Self {
        Self {
            client: handle.client(),
            handle,
            presence: None,
            contacts_import: None,
        }
    }
}

/// Ends a task when its owner goes, as a client's presence pulse must end with the client
struct AbortOnDrop(AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

enum Wake {
    Command(Command),
    Event(u64, Event),
    ClientEnded,
    Restart,
    PairingTimedOut,
    KeepaliveDue,
    KeepaliveEnded,
    HandlesDropped,
}

impl Supervisor {
    async fn run(mut self) {
        loop {
            let wake = tokio::select! {
                // In order, so a terminal event is handled before the end of the client it explains
                biased;
                command = self.commands.recv() => command.map_or(Wake::HandlesDropped, Wake::Command),
                Some((generation, event)) = self.events.recv() => Wake::Event(generation, event),
                () = finished(&mut self.client) => Wake::ClientEnded,
                () = at(self.restart_at) => Wake::Restart,
                () = at(self.pairing_deadline) => Wake::PairingTimedOut,
                () = at(self.keepalive_at) => Wake::KeepaliveDue,
                () = at(self.keepalive_ends) => Wake::KeepaliveEnded,
            };
            match wake {
                Wake::Command(Command::Change(event, reply)) => {
                    let _ = reply.send(self.handle(event).await.map(drop));
                }
                Wake::Command(Command::Unlink(reply)) => {
                    let _ = reply.send(self.handle(Event::Unlink).await);
                }
                Wake::Command(Command::Groups(reply)) => self.groups(reply),
                Wake::Command(Command::Picture(jid, reply)) => self.picture(jid, reply),
                Wake::Command(Command::SendTestMessage(reply)) => self.send_test_message(reply),
                Wake::Command(Command::Send(to, body, reply)) => self.send(to, body, reply),
                Wake::Command(Command::Shutdown(reply)) => {
                    self.stop_client().await;
                    let _ = reply.send(());
                    return;
                }
                Wake::HandlesDropped => {
                    self.stop_client().await;
                    return;
                }
                Wake::Event(generation, event) if generation == self.generation => {
                    if event == Event::Connected {
                        self.restarts = 0;
                        self.keepalive_connected = true;
                        self.keep_presence_fresh();
                        self.import_contacts();
                    }
                    let _ = self.handle(event).await;
                }
                Wake::Event(..) => {}
                Wake::ClientEnded => {
                    self.client = None;
                    let _ = self.handle(Event::ClientEnded).await;
                }
                Wake::Restart => {
                    self.restart_at = None;
                    if let Err(error) = self.start_client().await {
                        tracing::warn!(user_id = %self.user_id, "WhatsApp client failed to restart: {error:#}");
                        self.schedule_restart();
                    }
                }
                Wake::PairingTimedOut => {
                    self.pairing_deadline = None;
                    let _ = self.handle(Event::PairingTimedOut).await;
                }
                Wake::KeepaliveDue => {
                    self.keepalive_at = None;
                    let _ = self.handle(Event::KeepaliveDue).await;
                }
                Wake::KeepaliveEnded => self.end_keepalive().await,
            }
        }
    }

    /// Moves the status for `event` and carries out the effects. The answer says whether WhatsApp
    /// confirmed an unlink.
    async fn handle(&mut self, event: Event) -> Result<bool, CommandError> {
        let current = self.status.borrow().clone();
        let (next, effects) =
            status::next(&current, event).map_err(|Refused| CommandError::Refused)?;
        self.set_status(next);
        self.apply(effects).await
    }

    fn set_status(&mut self, next: Status) {
        let was_pairing = matches!(*self.status.borrow(), Status::Pairing(_));
        match next {
            Status::Pairing(_) if !was_pairing => {
                self.pairing_deadline = Some(Instant::now() + PAIRING_LIMIT);
            }
            Status::Pairing(_) => {}
            _ => self.pairing_deadline = None,
        }
        if !matches!(next, Status::Reconnecting { .. }) {
            self.restart_at = None;
        }
        if !matches!(next, Status::Paused { .. }) {
            self.keepalive_at = None;
            self.keepalive_ends = None;
        }
        self.status.send_replace(next);
    }

    /// Carries out every effect even when starting the client fails, so a resume still clears the
    /// pause while its client retries
    async fn apply(&mut self, effects: Vec<Effect>) -> Result<bool, CommandError> {
        let mut told_whatsapp = false;
        let mut failed = None;
        for effect in effects {
            match effect {
                Effect::StartClient => {
                    if let Err(error) = self.start_client().await {
                        tracing::warn!(user_id = %self.user_id, "WhatsApp client failed to start: {error:#}");
                        self.after_failed_start();
                        failed = Some(error);
                    }
                }
                Effect::RestartClient => self.schedule_restart(),
                Effect::StartKeepalive => self.start_keepalive().await,
                Effect::StopClient => self.stop_client().await,
                Effect::LogOut => told_whatsapp = self.log_out().await,
                Effect::RequestPairCode => self.request_pair_code().await,
                Effect::SaveLink { phone_number } => {
                    if let Some(account_id) = self.account_id {
                        let saved = whatsapp_accounts::save_link(
                            &self.context.pool,
                            account_id,
                            &phone_number,
                        )
                        .await;
                        self.log_failed_write("the link", saved);
                    }
                }
                Effect::SavePaused => {
                    self.keepalive_at = Some(Instant::now() + KEEPALIVE_EVERY);
                    if let Some(account_id) = self.account_id {
                        let saved = whatsapp_accounts::pause(
                            &self.context.pool,
                            account_id,
                            next_keepalive(),
                        )
                        .await;
                        self.log_failed_write("the pause", saved);
                    }
                }
                Effect::SaveResumed => {
                    if let Some(account_id) = self.account_id {
                        let saved = whatsapp_accounts::resume(&self.context.pool, account_id).await;
                        self.log_failed_write("the resume", saved);
                    }
                }
                Effect::SaveStopped(reason) => self.save_stop(&reason).await,
                Effect::Forget => {
                    self.pictures.clear();
                    if let Some(account_id) = self.account_id.take() {
                        let deleted =
                            whatsapp_accounts::delete(&self.context.pool, account_id).await;
                        self.log_failed_write("the unlink", deleted);
                    }
                }
                Effect::Alert(alert) => self.alert(alert),
            }
        }
        failed.map_or(Ok(told_whatsapp), |error| Err(CommandError::Failed(error)))
    }

    async fn start_keepalive(&mut self) {
        self.keepalive_connected = false;
        match self.start_client().await {
            Ok(()) => self.keepalive_ends = Some(Instant::now() + KEEPALIVE_WINDOW),
            Err(error) => {
                tracing::warn!(user_id = %self.user_id, "WhatsApp keepalive failed to start: {error:#}");
                self.keepalive_at = Some(Instant::now() + KEEPALIVE_RETRY);
            }
        }
    }

    /// Only a connection counts, so one that never came up tries again soon
    async fn end_keepalive(&mut self) {
        self.keepalive_ends = None;
        self.stop_client().await;
        if !self.keepalive_connected {
            tracing::warn!(user_id = %self.user_id, "WhatsApp keepalive didn't connect");
            self.keepalive_at = Some(Instant::now() + KEEPALIVE_RETRY);
            return;
        }
        self.keepalive_at = Some(Instant::now() + KEEPALIVE_EVERY);
        if let Some(account_id) = self.account_id {
            let saved =
                whatsapp_accounts::save_keepalive(&self.context.pool, account_id, next_keepalive())
                    .await;
            self.log_failed_write("the keepalive", saved);
        }
    }

    /// Only a takeover or a ban needs the user, so only they are kept; a deploy may fix the rest
    async fn save_stop(&self, reason: &StopReason) {
        let (name, until) = match reason {
            StopReason::Replaced => ("replaced", None),
            StopReason::TemporaryBan { until } => ("temporary_ban", Some(*until)),
            StopReason::Outdated | StopReason::Refused => return,
        };
        if let Some(account_id) = self.account_id {
            let saved =
                whatsapp_accounts::save_stop(&self.context.pool, account_id, name, until).await;
            self.log_failed_write("the stop", saved);
        }
    }

    /// Emails the owner beside the loop, which a slow mail lookup mustn't hold up
    fn alert(&self, alert: Alert) {
        let Context {
            pool,
            mailer,
            app_url,
            ..
        } = self.context.clone();
        let user_id = self.user_id;
        tokio::spawn(async move {
            if let Err(error) = alerts::send(&pool, &mailer, &app_url, user_id, alert).await {
                tracing::error!(%user_id, "couldn't email a WhatsApp alert: {error:#}");
            }
        });
    }

    fn log_failed_write(&self, what: &str, result: sqlx::Result<()>) {
        if let Err(error) = result {
            tracing::error!(user_id = %self.user_id, "couldn't save {what} for a WhatsApp account: {error}");
        }
    }

    /// A client that can't start leaves pairing for good, while a linked one tries again later
    fn after_failed_start(&mut self) {
        let status = self.status.borrow().clone();
        match status {
            Status::Pairing(_) => self.set_status(Status::NotLinked),
            Status::Reconnecting { .. } => self.schedule_restart(),
            _ => {}
        }
    }

    fn schedule_restart(&mut self) {
        let delay = FIRST_RESTART_DELAY
            .saturating_mul(2_u32.saturating_pow(self.restarts))
            .min(MAX_RESTART_DELAY);
        self.restarts = self.restarts.saturating_add(1);
        self.restart_at = Some(Instant::now() + delay);
    }

    async fn start_client(&mut self) -> anyhow::Result<()> {
        // Two clients on one store would knock each other off and corrupt its Signal state
        self.stop_client().await;
        self.restart_at = None;
        self.may_connect
            .wait_for(|open| *open)
            .await
            .map_err(|_| anyhow!("WhatsApp is shutting down"))?;
        let account_id = if let Some(account_id) = self.account_id {
            account_id
        } else {
            let account =
                whatsapp_accounts::find_or_create(&self.context.pool, self.user_id).await?;
            self.account_id = Some(account.id);
            account.id
        };
        self.generation += 1;
        let generation = self.generation;
        let events = self.events_tx.clone();
        let acks = self.acks.clone();

        let mut cache = CacheConfig::default();
        cache.msg_secret_policy = MsgSecretPolicy::Disabled;
        let mut builder = Bot::builder()
            .with_backend(PgStore::new(
                self.context.pool.clone(),
                self.context.cipher.clone(),
                account_id,
            ))
            .with_device_props(
                DevicePropsOverride::new()
                    .with_os(DEVICE_NAME)
                    .with_platform_type(wa::device_props::PlatformType::CHROME),
            )
            .with_presence_policy(PresencePolicy::Manual)
            .skip_history_sync()
            .with_cache_config(cache)
            .with_http_client_arc(Arc::clone(&self.context.http))
            .with_event_handler(Feed::new(account_id, self.contacts.clone()))
            .on_event_for(&WATCHED, move |event, _client| {
                let events = events.clone();
                let acks = acks.clone();
                async move {
                    if let LibraryEvent::ServerAck(ack) = &*event {
                        // Receipts and calls are acknowledged too, under their own ids
                        if ack.class.as_deref() == Some("message") {
                            acks.settle(&ack.id, ack.error.clone());
                        }
                    } else if let Some(event) = translate(&event) {
                        // Fails only once the supervisor has ended, when nobody needs the event
                        let _ = events.send((generation, event));
                    }
                }
            });
        if let Some(number) = self.pairing_number() {
            builder = builder.with_pair_code(pair_code_options(&number));
        }
        let version = *self
            .context
            .version
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(version) = version {
            builder = builder.with_version(version);
        }
        self.client = Some(Running::new(builder.build().await?.spawn()));
        Ok(())
    }

    /// Flushes the client's state and waits for it to let go of the store
    async fn stop_client(&mut self) {
        let Some(Running {
            handle,
            client,
            presence,
            contacts_import,
        }) = self.client.take()
        else {
            return;
        };
        drop((presence, contacts_import));
        let released = client.store_release();
        drop(client);
        // Dropping the handle when time runs out aborts the client
        match timeout(STOP_LIMIT, handle.shutdown()).await {
            Ok(report) if report.shutdown.device.is_err() => {
                tracing::warn!(user_id = %self.user_id, "WhatsApp device state didn't flush on stop");
            }
            Ok(_) => {}
            Err(_) => {
                tracing::warn!(user_id = %self.user_id, "WhatsApp client didn't stop in time; aborted");
            }
        }
        let _ = timeout(STOP_LIMIT, released.wait()).await;
    }

    /// Asks WhatsApp to remove the device, then stops the client. A paused or stopped account gets
    /// a client just long enough to say so. True when WhatsApp confirmed the removal.
    async fn log_out(&mut self) -> bool {
        if self.client.is_none()
            && (self.start_client().await.is_err() || !self.wait_connected().await)
        {
            self.stop_client().await;
            return false;
        }
        let Some(Running {
            handle,
            client,
            presence,
            contacts_import,
        }) = self.client.take()
        else {
            return false;
        };
        drop((presence, contacts_import));
        let released = client.store_release();
        let report = timeout(STOP_LIMIT, client.logout()).await;
        drop(client);
        let _ = timeout(STOP_LIMIT, handle.shutdown()).await;
        let _ = timeout(STOP_LIMIT, released.wait()).await;
        matches!(report, Ok(report) if matches!(report.deregistration, DeregistrationOutcome::Confirmed))
    }

    async fn wait_connected(&mut self) -> bool {
        let generation = self.generation;
        let connected = async {
            while let Some((from, event)) = self.events.recv().await {
                if from == generation && event == Event::Connected {
                    return true;
                }
            }
            false
        };
        timeout(LOGOUT_CONNECT_LIMIT, connected)
            .await
            .unwrap_or(false)
    }

    /// Starts the client's daily presence pulse, or after a reconnect marks it unavailable again in
    /// case the drop cut a pulse short
    fn keep_presence_fresh(&mut self) {
        // A keepalive's client doesn't last until a spread-out pulse
        let first_in = if matches!(*self.status.borrow(), Status::Paused { .. }) {
            Duration::ZERO
        } else {
            Duration::from_secs(
                u64::try_from(self.user_id.as_u128() % u128::from(FIRST_PULSE_WITHIN.as_secs()))
                    .unwrap_or_default(),
            )
        };
        let Some(running) = &mut self.client else {
            return;
        };
        let client = Arc::clone(&running.client);
        if running.presence.is_some() {
            tokio::spawn(async move {
                let _ = client.presence().set_unavailable().await;
            });
        } else {
            running.presence = Some(AbortOnDrop(
                tokio::spawn(pulse_presence(client, first_in)).abort_handle(),
            ));
        }
    }

    /// A keepalive's client is gone before a replay would finish, so a paused account waits
    fn import_contacts(&mut self) {
        if matches!(*self.status.borrow(), Status::Paused { .. }) {
            return;
        }
        let (Some(running), Some(account_id)) = (&mut self.client, self.account_id) else {
            return;
        };
        if running
            .contacts_import
            .as_ref()
            .is_some_and(|task| !task.0.is_finished())
        {
            return;
        }
        let import = contacts::import_once(
            Arc::clone(&running.client),
            self.context.pool.clone(),
            account_id,
            self.contacts.clone(),
        );
        running.contacts_import = Some(AbortOnDrop(tokio::spawn(import).abort_handle()));
    }

    async fn request_pair_code(&self) {
        let (Some(running), Some(number)) = (&self.client, self.pairing_number()) else {
            return;
        };
        // The error can carry the number, so it stays out of the log
        if running
            .client
            .pair_with_code(pair_code_options(&number))
            .await
            .is_err()
        {
            tracing::warn!(user_id = %self.user_id, "WhatsApp pair code request failed");
        }
    }

    fn pairing_number(&self) -> Option<String> {
        match &*self.status.borrow() {
            Status::Pairing(status::Pairing {
                method: Method::PhoneNumber(number),
                ..
            }) => Some(number.clone()),
            _ => None,
        }
    }

    fn connected_client(&self) -> Result<Arc<Client>, CommandError> {
        match (&*self.status.borrow(), &self.client) {
            (Status::Connected { .. }, Some(running)) => Ok(Arc::clone(&running.client)),
            _ => Err(CommandError::NotConnected),
        }
    }

    // Requests run beside the loop, so a slow answer from WhatsApp holds up no other command

    fn groups(&self, reply: oneshot::Sender<Result<Vec<Group>, CommandError>>) {
        let client = match self.connected_client() {
            Ok(client) => client,
            Err(error) => {
                let _ = reply.send(Err(error));
                return;
            }
        };
        tokio::spawn(async move {
            let listed = timeout(REQUEST_LIMIT, client.groups().list_participating()).await;
            let groups = match listed {
                Ok(Ok(groups)) => Ok(groups
                    .into_iter()
                    .map(|group| Group {
                        id: group.id.to_string(),
                        name: group.subject.unwrap_or_default(),
                    })
                    .collect()),
                Ok(Err(error)) => Err(CommandError::Failed(error.into())),
                Err(_) => Err(CommandError::Failed(anyhow!(
                    "WhatsApp didn't answer in time"
                ))),
            };
            let _ = reply.send(groups);
        });
    }

    fn picture(&self, jid: Jid, reply: oneshot::Sender<Option<Image>>) {
        // Memory answers even while WhatsApp is offline
        if let Some(known) = self.pictures.cached(&jid.to_string()) {
            let _ = reply.send(known.image());
            return;
        }
        let Ok(client) = self.connected_client() else {
            let _ = reply.send(None);
            return;
        };
        let pictures = Arc::clone(&self.pictures);
        let http = Arc::clone(&self.context.http);
        tokio::spawn(async move {
            let _ = reply.send(pictures.fetch(&client, &*http, &jid).await);
        });
    }

    fn send_test_message(&self, reply: oneshot::Sender<Result<(), CommandError>>) {
        let client = match self.connected_client() {
            Ok(client) => client,
            Err(error) => {
                let _ = reply.send(Err(error));
                return;
            }
        };
        tokio::spawn(async move {
            let sent = match client.pn() {
                Some(own) => timeout(
                    REQUEST_LIMIT,
                    client.send_text(Jid::pn(own.user), TEST_MESSAGE),
                )
                .await
                .map_err(|_| CommandError::Failed(anyhow!("WhatsApp didn't answer in time")))
                .and_then(|sent| {
                    sent.map(drop)
                        .map_err(|error| CommandError::Failed(error.into()))
                }),
                None => Err(CommandError::NotConnected),
            };
            let _ = reply.send(sent);
        });
    }

    fn send(&self, to: Recipient, body: String, reply: oneshot::Sender<Result<(), SendFailure>>) {
        let Ok(client) = self.connected_client() else {
            let failure =
                SendFailure::for_status(&self.status.borrow()).unwrap_or(SendFailure::Offline);
            let _ = reply.send(Err(failure));
            return;
        };
        let acks = self.acks.clone();
        tokio::spawn(async move {
            let _ = reply.send(send_and_confirm(client, acks, to, body).await);
        });
    }
}

async fn send_and_confirm(
    client: Arc<Client>,
    acks: Acks,
    to: Recipient,
    body: String,
) -> Result<(), SendFailure> {
    let jid = match to {
        Recipient::Number(number) => registered(&client, number).await?,
        Recipient::Group(id) => id.parse::<Jid>().map_err(|_| SendFailure::Failed)?,
    };

    // Set up before sending, since the acknowledgement can beat `send`'s return
    let id = client.generate_message_id();
    let acknowledged = acks.expect(id.as_str());
    let key = id.as_str().to_owned();
    let request = SendRequest::new(&jid, wa::Message::text(body))
        .with_options(SendOptions::default().with_message_id(id));
    let outcome = timeout(SEND_LIMIT, async {
        client.send(request).await?;
        Ok::<_, SendFailure>(acknowledged.await)
    })
    .await;
    acks.forget(&key);

    match outcome {
        Ok(Ok(Ok(None))) => Ok(()),
        Ok(Ok(Ok(Some(code)))) => Err(SendFailure::Rejected(code)),
        Ok(Err(failure)) => Err(failure),
        Ok(Ok(Err(_))) | Err(_) => Err(SendFailure::Unconfirmed),
    }
}

/// The address WhatsApp has for `number`. Without this check a send to a number that isn't on
/// WhatsApp looks like it worked.
async fn registered(client: &Client, number: String) -> Result<Jid, SendFailure> {
    let asked = [Jid::pn(number)];
    let answers = timeout(REQUEST_LIMIT, client.contacts().is_on_whatsapp(&asked))
        .await
        .map_err(|_| SendFailure::Unreachable)?
        .map_err(|_| SendFailure::Unreachable)?;
    answers
        .into_iter()
        .find(|answer| answer.is_registered)
        .map(|answer| answer.jid)
        .ok_or(SendFailure::NotOnWhatsApp)
}

async fn pulse_presence(client: Arc<Client>, first_in: Duration) {
    // A pulse cut short by a stop or a crash would leave the account shown online
    let _ = client.presence().set_unavailable().await;
    let mut wait = first_in;
    loop {
        tokio::time::sleep(wait).await;
        if client.presence().set_available().await.is_ok() {
            tokio::time::sleep(PRESENCE_PULSE).await;
        }
        let _ = client.presence().set_unavailable().await;
        wait = PRESENCE_EVERY;
    }
}

fn next_keepalive() -> DateTime<Utc> {
    Utc::now() + TimeDelta::from_std(KEEPALIVE_EVERY).unwrap_or_default()
}

async fn finished(client: &mut Option<Running>) {
    match client {
        Some(running) => {
            let _ = (&mut running.handle).await;
        }
        None => std::future::pending().await,
    }
}

async fn at(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

fn pair_code_options(number: &str) -> PairCodeOptions {
    PairCodeOptions {
        phone_number: number.to_owned(),
        ..Default::default()
    }
}

/// The library's event as `next` sees it, or `None` for one it doesn't care about
fn translate(event: &LibraryEvent) -> Option<Event> {
    let now = Utc::now();
    let code = |value: &str, valid_for: Duration| Code {
        value: value.to_owned(),
        expires_at: now + TimeDelta::from_std(valid_for).unwrap_or_default(),
    };
    Some(match event {
        LibraryEvent::PairingQrCode(qr) => Event::QrCode(code(&qr.code, qr.timeout)),
        LibraryEvent::PairingCode(pair) => Event::PairCode(code(&pair.code, pair.timeout)),
        LibraryEvent::PairingCodeRefresh(_) => Event::PairCodeExpired,
        LibraryEvent::PairingCodeError(_) | LibraryEvent::PairError(_) => Event::PairFailed,
        LibraryEvent::PairingQrCodesExhausted(_) => Event::QrCodesExhausted,
        LibraryEvent::PairSuccess(paired) => Event::Paired {
            phone_number: paired.id.user.to_string(),
        },
        LibraryEvent::Connected(_) => Event::Connected,
        LibraryEvent::Disconnected(_) => Event::Disconnected,
        LibraryEvent::LoggedOut(_) => Event::LoggedOut,
        LibraryEvent::StreamReplaced(_) => Event::Replaced,
        LibraryEvent::TemporaryBan(ban) => Event::TemporaryBan {
            until: now + ban.expire,
        },
        LibraryEvent::ClientOutdated(_) => Event::Outdated,
        LibraryEvent::ConnectFailure(failure) => Event::ConnectFailed {
            retrying: failure.reason.should_reconnect(),
        },
        _ => return None,
    })
}
