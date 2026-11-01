//! Every user's WhatsApp supervisor, in one map that `AppState` shares

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use tokio::{task::JoinSet, time::timeout};
use uuid::Uuid;

use super::{
    status::{Status, StopReason},
    supervisor::{self, Context, Handle},
};
use crate::db::whatsapp_accounts::{self, Account};

/// Between two accounts connecting at boot, so a restart doesn't dial every account at once
const BOOT_SPACING: Duration = Duration::from_secs(2);
/// Each client gets this long to flush and stop when the server shuts down
const SHUTDOWN_LIMIT: Duration = Duration::from_secs(20);

#[derive(Clone)]
pub struct WhatsApp {
    context: Context,
    // A std mutex, since it's held only for a map lookup and never across an `.await`
    supervisors: Arc<Mutex<HashMap<Uuid, Handle>>>,
}

impl WhatsApp {
    #[must_use]
    pub fn new(context: Context) -> Self {
        Self {
            context,
            supervisors: Arc::default(),
        }
    }

    /// Starts a supervisor for every account. Linked, unpaused ones connect `BOOT_SPACING` apart.
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
            );
            handles.push((account.user_id, handle));
            if connects {
                start_in += BOOT_SPACING;
            }
        }
        self.lock().extend(handles);
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
                )
            })
            .clone()
    }

    /// Stops every client, each within `SHUTDOWN_LIMIT`, keeping the links for the next start
    pub async fn shutdown(&self) {
        let handles: Vec<Handle> = self.lock().drain().map(|(_, handle)| handle).collect();
        let mut stops = JoinSet::new();
        for handle in handles {
            stops.spawn(async move { timeout(SHUTDOWN_LIMIT, handle.shutdown()).await });
        }
        stops.join_all().await;
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
        WhatsApp::new(Context {
            pool: pool.clone(),
            cipher: Cipher::new(&[7; 32]),
            mailer: Mailer::Outbox(Arc::default()),
            app_url: Arc::new(Url::parse("http://localhost:5173").unwrap()),
        })
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
