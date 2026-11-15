// The owner's agent API (stage 2b) calls the rest of these; until then only the operations, the
// cleanup and tests do
#[allow(dead_code, reason = "used by the owner's agent API, still to come")]
pub mod agent_activity;
#[allow(dead_code, reason = "used by the owner's agent API, still to come")]
pub mod agent_approvals;
#[allow(dead_code, reason = "used by the owner's agent API, still to come")]
pub mod agent_connections;
pub mod deliveries;
pub mod message_templates;
pub mod passkey_ceremonies;
pub mod passkeys;
pub mod scheduled_messages;
pub mod sessions;
pub mod tokens;
pub mod two_factor;
pub mod users;
pub mod whatsapp_accounts;
pub mod whatsapp_contacts;

#[cfg(test)]
mod agent_tests;

use std::time::Duration;

use sqlx::PgPool;

use crate::auth::two_factor::SETUP_TTL;

/// Hourly sweep of expired rows and of agent activity past 90 days.
///
/// Covers sessions, email links, passkey ceremonies, second-factor challenges, two-step setups and
/// agent approvals. Queries already ignore expired rows; this only keeps the tables from growing.
pub fn spawn_cleanup(pool: PgPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_hours(1));
        loop {
            interval.tick().await;
            match tokio::try_join!(
                sessions::delete_expired(&pool),
                tokens::delete_expired(&pool),
                passkey_ceremonies::delete_expired(&pool),
                two_factor::delete_expired_challenges(&pool),
                two_factor::delete_expired_setups(&pool, SETUP_TTL),
                agent_approvals::expire_overdue(&pool),
                agent_activity::delete_old(&pool)
            ) {
                Ok((sessions, tokens, ceremonies, challenges, setups, approvals, activity)) => {
                    tracing::debug!(
                        sessions,
                        tokens,
                        ceremonies,
                        challenges,
                        setups,
                        approvals,
                        activity,
                        "deleted expired sessions, tokens, passkey ceremonies, challenges and \
                         setups, expired agent approvals and dropped old agent activity"
                    );
                }
                Err(error) => {
                    tracing::warn!(?error, "failed to delete expired sessions and tokens");
                }
            }
        }
    });
}
