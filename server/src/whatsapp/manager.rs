//! Every user's WhatsApp supervisor, in one map that `AppState` shares

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use tokio::{task::JoinSet, time::timeout};
use uuid::Uuid;

use super::{
    status::Status,
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
    match &account.phone_number {
        Some(phone_number) if account.paused => Status::Paused {
            phone_number: phone_number.clone(),
        },
        Some(phone_number) => Status::Reconnecting {
            phone_number: phone_number.clone(),
        },
        None => Status::NotLinked,
    }
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::*;
    use crate::{crypto::Cipher, whatsapp::supervisor::CommandError};

    // No test here links or resumes an account, since that would open a real WhatsApp connection

    fn manager(pool: &PgPool) -> WhatsApp {
        WhatsApp::new(Context {
            pool: pool.clone(),
            cipher: Cipher::new(&[7; 32]),
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
    async fn boot_restores_each_account_without_connecting_paused_or_unlinked_ones(pool: PgPool) {
        let (paused, unlinked, stranger) =
            (user(&pool).await, user(&pool).await, user(&pool).await);
        sqlx::query!(
            "INSERT INTO whatsapp_accounts (user_id, phone_number, linked_at, paused_at) \
             VALUES ($1, '15550001', now(), now()), ($2, NULL, NULL, NULL)",
            paused,
            unlinked,
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
        assert_eq!(whatsapp.status(stranger), Status::NotLinked);
        whatsapp.shutdown().await;
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
    fn a_linked_account_reconnects_at_boot_unless_paused() {
        let account = |phone_number: Option<&str>, paused| Account {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            phone_number: phone_number.map(Into::into),
            paused,
        };
        assert_eq!(
            status_at_boot(&account(Some("15550001"), false)),
            Status::Reconnecting {
                phone_number: "15550001".into()
            }
        );
        assert!(matches!(
            status_at_boot(&account(Some("15550001"), true)),
            Status::Paused { .. }
        ));
        assert_eq!(status_at_boot(&account(None, false)), Status::NotLinked);
    }
}
