use std::{num::NonZeroU32, sync::Arc};

use sqlx::PgPool;
use url::Url;

use crate::{
    agents::events::Events, auth::pwned::PwnedPasswords, crypto::Cipher, email::Mailer,
    oauth::client::Clients, push::Push, rate_limit::RateLimits, whatsapp::manager::WhatsApp,
};

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub mailer: Mailer,
    pub app_url: Arc<Url>,
    pub rate_limits: RateLimits,
    pub pwned_passwords: PwnedPasswords,
    /// The apps that sign in to agent connections
    pub oauth_clients: Clients,
    pub cipher: Cipher,
    pub whatsapp: WhatsApp,
    /// Notifications to the owner's browsers
    pub push: Push,
    /// What open pages are told as it happens
    pub events: Events,
    /// `AGENT_ACCESS`: whether the agent routes and the MCP endpoint answer
    pub agent_access: bool,
    /// `MAX_LINKED_ACCOUNTS`: how many WhatsApp accounts may be linked at once
    pub max_linked_accounts: Option<NonZeroU32>,
}

impl AppState {
    /// Browsers only send Secure cookies over https, so plain-http development goes without
    #[must_use]
    pub fn secure_cookies(&self) -> bool {
        self.app_url.scheme() == "https"
    }
}
