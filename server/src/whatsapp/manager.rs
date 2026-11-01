//! Every user's WhatsApp supervisor, in one map that `AppState` shares

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use sqlx::{Connection, PgConnection};
use tokio::{sync::watch, task::JoinSet, time::timeout};
use uuid::Uuid;
use whatsapp_rust::version;

use super::{
    status::{Status, StopReason},
    supervisor::{self, Context, Handle},
};
use crate::db::whatsapp_accounts::{self, Account};

/// Between two accounts connecting at boot, so a restart doesn't dial every account at once
const BOOT_SPACING: Duration = Duration::from_secs(2);
/// Each client gets this long to flush and stop when the server shuts down
const SHUTDOWN_LIMIT: Duration = Duration::from_secs(20);
const VERSION_EVERY: Duration = Duration::from_hours(24);
const VERSION_RETRY: Duration = Duration::from_hours(1);
/// "whatsapp" in ASCII. Two servers running clients on the same keys would knock each other off, so
/// only the holder of this advisory lock connects, whichever order a deploy stops and starts them in.
const LOCK_KEY: i64 = 0x7768_6174_7361_7070;

#[derive(Clone)]
pub struct WhatsApp {
    context: Context,
    // A std mutex, since it's held only for a map lookup and never across an `.await`
    supervisors: Arc<Mutex<HashMap<Uuid, Handle>>>,
    may_connect: Arc<watch::Sender<bool>>,
    /// Holds the lock from boot to shutdown, outside the pool so closing it lets go at once
    lock: Arc<Mutex<Option<PgConnection>>>,
}

impl WhatsApp {
    #[must_use]
    pub fn new(context: Context) -> Self {
        Self {
            context,
            supervisors: Arc::default(),
            may_connect: Arc::new(watch::channel(false).0),
            lock: Arc::default(),
        }
    }

    /// Starts a supervisor for every account, then takes the lock in the background. Once it holds
    /// it, linked, unpaused accounts connect `BOOT_SPACING` apart.
    ///
    /// # Errors
    /// The accounts couldn't be read.
    pub async fn boot(&self) -> sqlx::Result<()> {
        let accounts = whatsapp_accounts::all(&self.context.pool).await?;
        let mut start_in = Duration::ZERO;
        let mut handles = Vec::with_capacity(accounts.len());
        for account in accounts {
            let status = status_at_boot(&account);
            let connects = matches!(status, Status::Reconnecting { .. });
            let handle = supervisor::spawn(
                self.context.clone(),
                account.user_id,
                Some(account.id),
                status,
                start_in,
                self.may_connect.subscribe(),
            );
            handles.push((account.user_id, handle));
            if connects {
                start_in += BOOT_SPACING;
            }
        }
        self.lock().extend(handles);

        let manager = self.clone();
        tokio::spawn(async move {
            match manager.take_lock().await {
                Ok(()) => {
                    manager.may_connect.send_replace(true);
                }
                Err(error) => {
                    tracing::error!(
                        "couldn't take the WhatsApp lock, so no account connects: {error}"
                    );
                }
            }
        });
        Ok(())
    }

    /// Fetches the WhatsApp Web version once a day for every client. One failed fetch would
    /// otherwise stop each client whose own cached version had expired from connecting.
    pub fn refresh_version_daily(&self) {
        let context = self.context.clone();
        tokio::spawn(async move {
            loop {
                let wait = match version::fetch_latest_app_version(&context.http).await {
                    Ok(latest) => {
                        *context
                            .version
                            .write()
                            .unwrap_or_else(PoisonError::into_inner) = Some(latest);
                        VERSION_EVERY
                    }
                    Err(error) => {
                        tracing::warn!(
                            "couldn't fetch the WhatsApp Web version; keeping the last: {error:#}"
                        );
                        VERSION_RETRY
                    }
                };
                tokio::time::sleep(wait).await;
            }
        });
    }

    /// Waits while another server, the one a deploy is replacing, still runs the clients
    async fn take_lock(&self) -> sqlx::Result<()> {
        let mut connection =
            PgConnection::connect_with(&self.context.pool.connect_options()).await?;
        let free: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
            .bind(LOCK_KEY)
            .fetch_one(&mut connection)
            .await?;
        if !free {
            tracing::info!("another server runs the WhatsApp clients; waiting for it to stop");
            sqlx::query("SELECT pg_advisory_lock($1)")
                .bind(LOCK_KEY)
                .execute(&mut connection)
                .await?;
        }
        *self.held_lock() = Some(connection);
        Ok(())
    }

    /// `user_id`'s link status; a user without a supervisor has never linked
    #[must_use]
    pub fn status(&self, user_id: Uuid) -> Status {
        self.lock()
            .get(&user_id)
            .map_or(Status::NotLinked, Handle::status)
    }

    /// `user_id`'s supervisor, started on first use
    #[must_use]
    pub fn handle(&self, user_id: Uuid) -> Handle {
        self.lock()
            .entry(user_id)
            .or_insert_with(|| {
                supervisor::spawn(
                    self.context.clone(),
                    user_id,
                    None,
                    Status::NotLinked,
                    Duration::ZERO,
                    self.may_connect.subscribe(),
                )
            })
            .clone()
    }

    /// Stops every client, each within `SHUTDOWN_LIMIT`, keeping the links for the next start, then
    /// lets go of the lock for the server that replaces this one
    pub async fn shutdown(&self) {
        let handles: Vec<Handle> = self.lock().drain().map(|(_, handle)| handle).collect();
        let mut stops = JoinSet::new();
        for handle in handles {
            stops.spawn(async move { timeout(SHUTDOWN_LIMIT, handle.shutdown()).await });
        }
        stops.join_all().await;

        let held = self.held_lock().take();
        if let Some(connection) = held {
            let _ = connection.close().await;
        }
    }

    fn held_lock(&self) -> MutexGuard<'_, Option<PgConnection>> {
        self.lock.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<Uuid, Handle>> {
        // Nothing panics halfway through changing the map, so a poisoned lock still guards a whole one
        self.supervisors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

fn status_at_boot(account: &Account) -> Status {
    let Some(phone_number) = account.phone_number.clone() else {
        return Status::NotLinked;
    };
    let kept_stop = match (account.stopped_reason.as_deref(), account.stopped_until) {
        (Some("replaced"), _) => Some(StopReason::Replaced),
        (Some("temporary_ban"), Some(until)) => Some(StopReason::TemporaryBan { until }),
        _ => None,
    };
    match kept_stop {
        Some(reason) => Status::Stopped {
            phone_number,
            reason,
        },
        None if account.paused => Status::Paused { phone_number },
        None => Status::Reconnecting { phone_number },
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use sqlx::PgPool;
    use url::Url;

    use super::*;
    use crate::{crypto::Cipher, email::Mailer, whatsapp::supervisor::CommandError};

    // No test here links or resumes an account, since that would open a real WhatsApp connection

    fn manager(pool: &PgPool) -> WhatsApp {
        WhatsApp::new(Context::new(
            pool.clone(),
            Cipher::new(&[7; 32]),
            Mailer::Outbox(Arc::default()),
            Arc::new(Url::parse("http://localhost:5173").unwrap()),
        ))
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

    #[sqlx::test]
    async fn boot_restores_every_account_that_needs_no_connection(pool: PgPool) {
        let (paused, unlinked, replaced, banned, stranger) = (
            user(&pool).await,
            user(&pool).await,
            user(&pool).await,
            user(&pool).await,
            user(&pool).await,
        );
        let until = Utc.timestamp_opt(1_800_000_000, 0).unwrap();
        sqlx::query!(
            "INSERT INTO whatsapp_accounts \
             (user_id, phone_number, linked_at, paused_at, stopped_reason, stopped_until) VALUES \
             ($1, '15550001', now(), now(), NULL, NULL), \
             ($2, NULL, NULL, NULL, NULL, NULL), \
             ($3, '15550003', now(), NULL, 'replaced', NULL), \
             ($4, '15550004', now(), NULL, 'temporary_ban', $5)",
            paused,
            unlinked,
            replaced,
            banned,
            until,
        )
        .execute(&pool)
        .await
        .unwrap();

        let whatsapp = manager(&pool);
        whatsapp.boot().await.unwrap();
        assert_eq!(
            whatsapp.status(paused),
            Status::Paused {
                phone_number: "15550001".into()
            }
        );
        assert_eq!(whatsapp.status(unlinked), Status::NotLinked);
        assert_eq!(
            whatsapp.status(replaced),
            Status::Stopped {
                phone_number: "15550003".into(),
                reason: StopReason::Replaced,
            }
        );
        assert_eq!(
            whatsapp.status(banned),
            Status::Stopped {
                phone_number: "15550004".into(),
                reason: StopReason::TemporaryBan { until },
            }
        );
        assert_eq!(whatsapp.status(stranger), Status::NotLinked);
        whatsapp.shutdown().await;
    }

    /// Whether `whatsapp` lets clients start within a second
    async fn opens(whatsapp: &WhatsApp) -> bool {
        let mut may_connect = whatsapp.may_connect.subscribe();
        timeout(Duration::from_secs(1), may_connect.wait_for(|open| *open))
            .await
            .is_ok_and(|waited| waited.is_ok())
    }

    #[sqlx::test]
    async fn a_second_server_connects_only_once_the_first_lets_go(pool: PgPool) {
        let first = manager(&pool);
        first.boot().await.unwrap();
        assert!(opens(&first).await);

        let second = manager(&pool);
        second.boot().await.unwrap();
        assert!(!opens(&second).await);

        first.shutdown().await;
        assert!(opens(&second).await);
        second.shutdown().await;
    }

    #[sqlx::test]
    async fn the_database_refuses_a_ban_without_its_end(pool: PgPool) {
        let owner = user(&pool).await;
        let refused = sqlx::query!(
            "INSERT INTO whatsapp_accounts (user_id, stopped_reason) VALUES ($1, 'temporary_ban')",
            owner,
        )
        .execute(&pool)
        .await;
        assert!(refused.is_err());
    }

    #[sqlx::test]
    async fn commands_reach_a_supervisor_started_on_first_use(pool: PgPool) {
        let whatsapp = manager(&pool);
        let handle = whatsapp.handle(user(&pool).await);
        assert!(matches!(handle.pause().await, Err(CommandError::Refused)));
        assert!(matches!(handle.unlink().await, Err(CommandError::Refused)));
        assert!(matches!(
            handle.groups().await,
            Err(CommandError::NotConnected)
        ));
        assert!(matches!(
            handle.send_test_message().await,
            Err(CommandError::NotConnected)
        ));

        whatsapp.shutdown().await;
        assert!(matches!(handle.cancel().await, Err(CommandError::Gone)));
    }

    #[test]
    fn a_kept_stop_outranks_a_pause_and_a_linked_account_reconnects() {
        let account = |paused, stopped_reason: Option<&str>| Account {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            phone_number: Some("15550001".into()),
            paused,
            stopped_reason: stopped_reason.map(Into::into),
            stopped_until: None,
        };
        assert!(matches!(
            status_at_boot(&account(false, None)),
            Status::Reconnecting { .. }
        ));
        assert!(matches!(
            status_at_boot(&account(true, None)),
            Status::Paused { .. }
        ));
        assert!(matches!(
            status_at_boot(&account(true, Some("replaced"))),
            Status::Stopped {
                reason: StopReason::Replaced,
                ..
            }
        ));
        let unlinked = Account {
            phone_number: None,
            ..account(false, None)
        };
        assert_eq!(status_at_boot(&unlinked), Status::NotLinked);
    }
}
