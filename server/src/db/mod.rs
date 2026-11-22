pub mod agent_activity;
pub mod agent_approvals;
pub mod agent_connections;
pub mod agent_sign_ins;
pub mod agent_tokens;
pub mod deliveries;
pub mod history_chunks;
pub mod inbox_messages;
pub mod instance_keys;
pub mod message_history;
pub mod message_templates;
pub mod outgoing_files;
pub mod passkey_ceremonies;
pub mod passkeys;
pub mod push_subscriptions;
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

/// Chats named as two parallel arrays, the form Postgres's `unnest` takes; empty for none
#[derive(Clone, Copy, Default)]
pub struct Chats<'a> {
    pub types: &'a [String],
    pub recipients: &'a [String],
}

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
                agent_activity::delete_old(&pool),
                agent_sign_ins::delete_expired(&pool),
                agent_tokens::delete_expired(&pool),
                outgoing_files::release_stale(&pool)
            ) {
                Ok((
                    sessions,
                    tokens,
                    ceremonies,
                    challenges,
                    setups,
                    approvals,
                    activity,
                    agent_sign_ins,
                    agent_tokens,
                    files,
                )) => {
                    tracing::debug!(
                        sessions,
                        tokens,
                        ceremonies,
                        challenges,
                        setups,
                        approvals,
                        activity,
                        agent_sign_ins,
                        agent_tokens,
                        files,
                        "deleted expired sessions, tokens, passkey ceremonies, challenges and \
                         setups, expired agent approvals, sign-ins and tokens, dropped old \
                         agent activity, and released files no send waits for"
                    );
                }
                Err(error) => {
                    tracing::warn!(?error, "failed to delete expired sessions and tokens");
                }
            }
        }
    });
}
