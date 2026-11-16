use std::{env, num::NonZeroU32, path::PathBuf};

use anyhow::Context;
use axum::http::HeaderName;
use base64::prelude::{BASE64_STANDARD, Engine as _};
use lettre::message::Mailbox;
use url::Url;

pub struct Config {
    pub database_url: String,
    pub port: u16,
    pub web_dir: PathBuf,
    /// Origin the SPA is served from; links in emails point here, and https turns on secure cookies
    pub app_url: Url,
    /// `None` writes emails to the log instead of sending them
    pub smtp_url: Option<String>,
    pub email_from: Mailbox,
    /// Header a trusted proxy sets to the client's address, such as `CF-Connecting-IP`; `None`
    /// keys the rate limit on the peer address
    pub client_ip_header: Option<HeaderName>,
    /// Seals secrets stored in the database, such as two-step verification keys
    pub encryption_key: [u8; 32],
    /// Whether users may connect AI agents to their accounts
    pub agent_access: bool,
    /// Whether connections may be allowed to read chats, which keeps those chats' recent messages
    pub agent_reading: bool,
    /// How many WhatsApp accounts may be linked at once; `None` for no limit
    pub max_linked_accounts: Option<NonZeroU32>,
}

/// Anyone can read this key, so whatever a debug build encrypts with it is as good as plaintext
const DEV_ENCRYPTION_KEY: [u8; 32] = *b"development key, not for release";

impl Config {
    /// # Errors
    /// `DATABASE_URL` missing, `APP_URL` or `ENCRYPTION_KEY` missing from a release build, `PORT`,
    /// `APP_URL` or `ENCRYPTION_KEY` malformed, an https `APP_URL` without `SMTP_URL`, `SMTP_URL`
    /// without a valid `EMAIL_FROM`, a `CLIENT_IP_HEADER` that is malformed or one proxies
    /// append to, an `AGENT_ACCESS` or `AGENT_READING` other than `on` or `off`, or a `MAX_LINKED_ACCOUNTS` that isn't a
    /// positive whole number.
    pub fn from_env() -> anyhow::Result<Self> {
        let port = match env::var("PORT") {
            Ok(value) => value.parse().context("PORT must be a valid port number")?,
            Err(_) => 8000,
        };

        let app_url = match env::var("APP_URL") {
            Ok(url) => url,
            // The Vite dev server `just dev` runs. A release build has to be told, or its email
            // links would point at localhost and its cookies would lose `Secure`.
            Err(_) if cfg!(debug_assertions) => "http://localhost:5173".into(),
            Err(_) => anyhow::bail!("APP_URL is required, e.g. https://wa.example.com"),
        };
        let app_url = Url::parse(&app_url).context("APP_URL must be an absolute URL")?;
        anyhow::ensure!(
            matches!(app_url.scheme(), "http" | "https"),
            "APP_URL must start with http:// or https://"
        );

        let smtp_url = env::var("SMTP_URL").ok().filter(|url| !url.is_empty());
        // Without SMTP, live sign-in links land in the log, which is fine on a dev machine only
        anyhow::ensure!(
            smtp_url.is_some() || app_url.scheme() == "http",
            "SMTP_URL is required when APP_URL is https"
        );
        let email_from = match env::var("EMAIL_FROM") {
            Ok(from) => from
                .parse()
                .context("EMAIL_FROM must look like `Name <address@example.com>`")?,
            Err(_) if smtp_url.is_some() => anyhow::bail!("EMAIL_FROM is required with SMTP_URL"),
            Err(_) => "Whatomato <no-reply@localhost>".parse()?,
        };

        let client_ip_header = match env::var("CLIENT_IP_HEADER") {
            Ok(name) if !name.is_empty() => {
                Some(HeaderName::try_from(name).context("CLIENT_IP_HEADER must be a header name")?)
            }
            _ => None,
        };
        // A proxy appends to these rather than replacing them, so their first value is the client's
        // own claim
        anyhow::ensure!(
            !client_ip_header
                .as_ref()
                .is_some_and(|name| name == "x-forwarded-for" || name == "forwarded"),
            "CLIENT_IP_HEADER must be a header the proxy overwrites, such as CF-Connecting-IP"
        );

        let encryption_key = match env::var("ENCRYPTION_KEY") {
            Ok(key) => BASE64_STANDARD
                .decode(key.trim())
                .ok()
                .and_then(|key| key.try_into().ok())
                .context(
                    "ENCRYPTION_KEY must be 32 bytes in base64, from `openssl rand -base64 32`",
                )?,
            Err(_) if cfg!(debug_assertions) => {
                tracing::warn!(
                    "ENCRYPTION_KEY is not set, so secrets are encrypted with a well-known development key"
                );
                DEV_ENCRYPTION_KEY
            }
            Err(_) => anyhow::bail!(
                "ENCRYPTION_KEY is required; generate one with `openssl rand -base64 32`"
            ),
        };

        let agent_access = match env::var("AGENT_ACCESS").as_deref() {
            Err(_) | Ok("" | "on") => true,
            Ok("off") => false,
            Ok(_) => anyhow::bail!("AGENT_ACCESS must be on or off"),
        };

        let agent_reading = match env::var("AGENT_READING").as_deref() {
            Err(_) | Ok("" | "off") => false,
            Ok("on") => true,
            Ok(_) => anyhow::bail!("AGENT_READING must be on or off"),
        };

        let max_linked_accounts =
            parse_max_linked_accounts(env::var("MAX_LINKED_ACCOUNTS").ok().as_deref())?;

        Ok(Self {
            database_url: env::var("DATABASE_URL").context("DATABASE_URL is required")?,
            port,
            web_dir: env::var_os("WEB_DIR")
                .map_or_else(|| PathBuf::from("web/dist"), PathBuf::from),
            app_url,
            smtp_url,
            email_from,
            client_ip_header,
            encryption_key,
            agent_access,
            agent_reading,
            max_linked_accounts,
        })
    }
}

fn parse_max_linked_accounts(value: Option<&str>) -> anyhow::Result<Option<NonZeroU32>> {
    match value {
        None | Some("") => Ok(None),
        Some(value) => {
            value.trim().parse().map(Some).context(
                "MAX_LINKED_ACCOUNTS must be a whole number above 0, or unset for no limit",
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_max_linked_accounts;

    #[test]
    fn max_linked_accounts_is_unset_or_a_positive_whole_number() {
        assert_eq!(parse_max_linked_accounts(None).unwrap(), None);
        assert_eq!(parse_max_linked_accounts(Some("")).unwrap(), None);
        assert_eq!(
            parse_max_linked_accounts(Some("50"))
                .unwrap()
                .map(std::num::NonZeroU32::get),
            Some(50)
        );
        for bad in ["0", "-1", "1.5", "ten", "99999999999"] {
            assert!(parse_max_linked_accounts(Some(bad)).is_err(), "{bad}");
        }
    }
}
