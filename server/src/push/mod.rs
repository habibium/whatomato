//! Notifications to the owner's browsers through the Web Push protocol (RFC 8030), which reach a
//! browser whether or not Whatomato is open in it

pub mod message;

use std::{sync::Arc, time::Duration};

use anyhow::Context as _;
use reqwest::{StatusCode, header, redirect};
use rustls::{ClientConfig, RootCertStore};
use serde::Serialize;
use sqlx::PgPool;
use url::Url;
use uuid::Uuid;

use self::message::Vapid;
use crate::{
    agents::ApprovalKind,
    crypto::Cipher,
    db::{
        agent_approvals::Approval,
        instance_keys,
        push_subscriptions::{self, Subscription},
    },
    scheduler::RecipientType,
};

const VAPID_KEY: &str = "vapid";
/// Binds the sealed pair to its row
const VAPID_CONTEXT: &[u8] = b"instance_keys:vapid";
const TIMEOUT: Duration = Duration::from_secs(10);
/// How long a push service keeps a notification for a browser that is offline: as long as an
/// approval waits
const TTL_SECONDS: u32 = 24 * 60 * 60;
const ENDPOINT_MAX: usize = 2048;

/// What the browser shows. No message text: a notification can sit on a lock screen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Notification {
    pub title: String,
    pub body: String,
    /// The page to open when it's clicked, as a path
    pub url: String,
    /// A newer notification with the same tag takes the older one's place
    pub tag: &'static str,
}

impl Notification {
    /// An agent asked for something that waits for the owner
    #[must_use]
    pub fn approval_waiting(connection_name: &str, approval: &Approval) -> Self {
        let wants = match approval.kind {
            ApprovalKind::Send => "wants to send a message",
            ApprovalKind::ScheduleCreate => "wants to schedule a message",
            ApprovalKind::ScheduleUpdate => "wants to change a scheduled message",
        };
        let to = match (&approval.recipient_name, approval.recipient_type) {
            (Some(name), _) => name.clone(),
            (None, RecipientType::Contact) => format!("+{}", approval.recipient),
            (None, RecipientType::Group) => "a group".to_owned(),
        };
        Self {
            title: format!("{connection_name} {wants}"),
            body: format!("To {to}. It waits for your yes or no."),
            url: format!("/agents?approval={}", approval.id),
            tag: "approval",
        }
    }

    /// Sent from Settings, to show that notifications reach this browser
    #[must_use]
    pub fn test() -> Self {
        Self {
            title: "Notifications are on".to_owned(),
            body: "This is how Whatomato tells you an agent needs your approval.".to_owned(),
            url: "/settings".to_owned(),
            tag: "test",
        }
    }
}

/// The address a browser gave for its push service, when it is one this server will send to.
///
/// Only the services browsers are known to use: the address comes from whoever is signed in, and
/// the server must not be made to post anywhere else.
///
/// # Errors
/// A sentence for the user.
pub fn check_endpoint(endpoint: &str) -> Result<Url, &'static str> {
    const UNKNOWN: &str = "This browser's notification service isn't one Whatomato knows.";
    let url = Url::parse(endpoint).map_err(|_| UNKNOWN)?;
    let host = url.host_str().unwrap_or_default();
    let known = host == "fcm.googleapis.com"
        || host == "android.googleapis.com"
        || host == "web.push.apple.com"
        || [
            ".push.services.mozilla.com",
            ".notify.windows.com",
            ".push.apple.com",
        ]
        .iter()
        .any(|suffix| host.ends_with(suffix));
    let plain = url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && endpoint.len() <= ENDPOINT_MAX;
    if known && plain {
        Ok(url)
    } else {
        Err(UNKNOWN)
    }
}

#[derive(Clone)]
pub struct Push {
    vapid: Arc<Vapid>,
    sender: Sender,
}

#[derive(Clone)]
enum Sender {
    Http(reqwest::Client),
    /// Keeps what would be sent, by endpoint, without the network
    #[cfg(test)]
    Outbox(Arc<std::sync::Mutex<Vec<(String, Notification)>>>),
}

enum Delivery {
    Sent,
    /// The push service no longer knows the subscription
    Gone,
}

impl Push {
    /// Reads the instance's push key pair, making it on the first start
    ///
    /// # Errors
    /// The database couldn't be reached, or the HTTP client couldn't be built.
    pub async fn start(pool: &PgPool, cipher: &Cipher) -> anyhow::Result<Self> {
        // ring and webpki-roots, as the breached-password check uses
        let tls =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()?
                .with_root_certificates(RootCertStore {
                    roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
                })
                .with_no_client_auth();
        let http = reqwest::Client::builder()
            .tls_backend_preconfigured(tls)
            .user_agent(concat!("whatomato/", env!("CARGO_PKG_VERSION")))
            .timeout(TIMEOUT)
            .https_only(true)
            .redirect(redirect::Policy::none())
            .build()?;
        Ok(Self {
            vapid: Arc::new(load_or_create(pool, cipher).await?),
            sender: Sender::Http(http),
        })
    }

    /// Sends nothing, and keeps what would have gone out for [`Self::sent`]
    #[cfg(test)]
    pub(crate) fn outbox() -> Self {
        let vapid = Vapid::from_pkcs8(&Vapid::generate().unwrap()).unwrap();
        Self {
            vapid: Arc::new(vapid),
            sender: Sender::Outbox(Arc::default()),
        }
    }

    /// The notifications handed to push services so far, with the endpoint each went to
    #[cfg(test)]
    pub(crate) fn sent(&self) -> Vec<(String, Notification)> {
        match &self.sender {
            Sender::Outbox(sent) => sent.lock().unwrap().clone(),
            Sender::Http(_) => Vec::new(),
        }
    }

    /// What a browser subscribes with
    #[must_use]
    pub fn public_key(&self) -> &str {
        self.vapid.public_key()
    }

    /// Notifies every browser of `user_id`, without holding the caller up. A failure is logged,
    /// never passed on: what the notification is about has happened already.
    pub fn notify(
        &self,
        pool: PgPool,
        app_url: Arc<Url>,
        user_id: Uuid,
        notification: Notification,
    ) {
        let push = self.clone();
        tokio::spawn(async move {
            push.notify_now(&pool, &app_url, user_id, &notification)
                .await;
        });
    }

    /// [`Self::notify`], waited for: how many browsers the notification was handed over for
    pub async fn notify_now(
        &self,
        pool: &PgPool,
        app_url: &Url,
        user_id: Uuid,
        notification: &Notification,
    ) -> usize {
        let subscriptions = match push_subscriptions::list(pool, user_id).await {
            Ok(subscriptions) => subscriptions,
            Err(error) => {
                tracing::error!(%user_id, ?error, "couldn't read push subscriptions");
                return 0;
            }
        };
        let mut sent = 0;
        for subscription in subscriptions {
            match self.deliver(&subscription, app_url, notification).await {
                Ok(Delivery::Sent) => sent += 1,
                Ok(Delivery::Gone) => {
                    if let Err(error) =
                        push_subscriptions::delete_by_id(pool, subscription.id).await
                    {
                        tracing::error!(?error, "couldn't delete a push subscription that is gone");
                    }
                }
                Err(error) => {
                    tracing::warn!(%user_id, "couldn't send a push notification: {error:#}");
                }
            }
        }
        sent
    }

    async fn deliver(
        &self,
        subscription: &Subscription,
        app_url: &Url,
        notification: &Notification,
    ) -> anyhow::Result<Delivery> {
        match &self.sender {
            Sender::Http(http) => self.post(http, subscription, app_url, notification).await,
            #[cfg(test)]
            Sender::Outbox(sent) => {
                sent.lock()
                    .unwrap()
                    .push((subscription.endpoint.clone(), notification.clone()));
                Ok(Delivery::Sent)
            }
        }
    }

    async fn post(
        &self,
        http: &reqwest::Client,
        subscription: &Subscription,
        app_url: &Url,
        notification: &Notification,
    ) -> anyhow::Result<Delivery> {
        let endpoint = check_endpoint(&subscription.endpoint).map_err(anyhow::Error::msg)?;
        let payload = serde_json::to_vec(notification)?;
        let body = message::encrypt(&subscription.p256dh, &subscription.auth, &payload)?;
        let authorization = self.vapid.authorization(&endpoint, app_url.as_str())?;
        let response = http
            .post(endpoint)
            .header(header::AUTHORIZATION, authorization)
            .header(header::CONTENT_ENCODING, "aes128gcm")
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header("TTL", TTL_SECONDS)
            .header("Urgency", "high")
            .body(body)
            .send()
            .await
            .context("the push service couldn't be reached")?;
        match response.status() {
            status if status.is_success() => Ok(Delivery::Sent),
            // 403: the subscription was made with another key pair than this server's
            StatusCode::NOT_FOUND | StatusCode::GONE | StatusCode::FORBIDDEN => Ok(Delivery::Gone),
            status => anyhow::bail!("the push service answered {status}"),
        }
    }
}

async fn load_or_create(pool: &PgPool, cipher: &Cipher) -> anyhow::Result<Vapid> {
    if let Some(sealed) = instance_keys::find(pool, VAPID_KEY).await? {
        match cipher.decrypt(&sealed, VAPID_CONTEXT) {
            Ok(pkcs8) => return Vapid::from_pkcs8(&pkcs8),
            Err(error) => {
                // ENCRYPTION_KEY was replaced, and every subscription was tied to the lost pair
                tracing::warn!(
                    "the push key pair can't be opened with this ENCRYPTION_KEY, so a new one is \
                     made and browsers have to turn notifications on again: {error:#}"
                );
                let pkcs8 = Vapid::generate()?;
                let sealed = cipher.encrypt(&pkcs8, VAPID_CONTEXT)?;
                instance_keys::replace(pool, VAPID_KEY, &sealed).await?;
                push_subscriptions::delete_all(pool).await?;
                return Vapid::from_pkcs8(&pkcs8);
            }
        }
    }
    let sealed = cipher.encrypt(&Vapid::generate()?, VAPID_CONTEXT)?;
    instance_keys::insert_if_absent(pool, VAPID_KEY, &sealed).await?;
    // Another server starting at the same moment may have stored its own first
    let sealed = instance_keys::find(pool, VAPID_KEY)
        .await?
        .context("the push key pair vanished as it was stored")?;
    Vapid::from_pkcs8(&cipher.decrypt(&sealed, VAPID_CONTEXT)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_push_services_are_sent_to() {
        for known in [
            "https://fcm.googleapis.com/fcm/send/abc:def",
            "https://updates.push.services.mozilla.com/wpush/v2/abc",
            "https://wns2-by3p.notify.windows.com/w/?token=abc",
            "https://web.push.apple.com/abc",
        ] {
            assert!(check_endpoint(known).is_ok(), "{known}");
        }
        let long = format!("https://fcm.googleapis.com/{}", "a".repeat(ENDPOINT_MAX));
        for other in [
            "http://fcm.googleapis.com/fcm/send/abc",
            "https://fcm.googleapis.com.evil.example/abc",
            "https://evil.example/fcm.googleapis.com",
            "https://user@fcm.googleapis.com/abc",
            "https://fcm.googleapis.com:8443/abc",
            "https://notify.windows.com.evil.example/abc",
            "https://localhost/abc",
            "https://169.254.169.254/latest/meta-data",
            long.as_str(),
            "not a url",
            "",
        ] {
            assert!(check_endpoint(other).is_err(), "{other}");
        }
    }

    #[sqlx::test]
    async fn the_key_pair_is_made_once_and_remade_when_it_cant_be_opened(pool: PgPool) {
        let cipher = Cipher::new(&[1; 32]);
        let first = load_or_create(&pool, &cipher).await.unwrap();
        let again = load_or_create(&pool, &cipher).await.unwrap();
        assert_eq!(first.public_key(), again.public_key());

        let user: Uuid = sqlx::query_scalar(
            "INSERT INTO users (email, password) VALUES ('ada@example.com', 'unused') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        push_subscriptions::save(
            &pool,
            push_subscriptions::NewSubscription {
                user_id: user,
                endpoint: "https://fcm.googleapis.com/fcm/send/abc",
                p256dh: &[4; 65],
                auth: &[0; 16],
                label: "Chrome on Linux",
            },
        )
        .await
        .unwrap();

        // Another ENCRYPTION_KEY: the pair is lost, and with it what was subscribed to it
        let replaced = load_or_create(&pool, &Cipher::new(&[2; 32])).await.unwrap();
        assert_ne!(replaced.public_key(), first.public_key());
        assert!(
            push_subscriptions::list(&pool, user)
                .await
                .unwrap()
                .is_empty()
        );
    }
}
