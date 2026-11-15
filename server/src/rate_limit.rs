use std::{
    collections::HashMap,
    hash::Hash,
    net::{IpAddr, Ipv6Addr, SocketAddr},
    num::NonZeroU32,
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{HeaderName, Method, Request, Response},
    response::IntoResponse,
};
use governor::{
    DefaultKeyedRateLimiter, Quota, RateLimiter, clock::Clock, middleware::NoOpMiddleware,
};
use tower_governor::{
    GovernorError, GovernorLayer, governor::GovernorConfigBuilder, key_extractor::KeyExtractor,
};
use uuid::Uuid;

use crate::error::AppError;

const SWEEP_EVERY: Duration = Duration::from_mins(10);
const ACCOUNT_BURST: NonZeroU32 = NonZeroU32::new(10).unwrap();
const ACCOUNT_REFILL: Duration = Duration::from_secs(90);
const WRONG_CODES_ALERT_EVERY: Duration = Duration::from_hours(6);
const LINK_BURST: NonZeroU32 = NonZeroU32::new(5).unwrap();
const LINK_REFILL: Duration = Duration::from_mins(10);
const AGENT_CALLS_PER_MINUTE: NonZeroU32 = NonZeroU32::new(120).unwrap();
const AGENT_KEY_FAILURES_BURST: NonZeroU32 = NonZeroU32::new(20).unwrap();
const AGENT_KEY_FAILURES_REFILL: Duration = Duration::from_secs(6);
/// How long a key that worked may still reach the lookup from an address over its failure limit
const AGENT_KEY_REMEMBERED: Duration = Duration::from_hours(24);
const AGENT_SEARCHES_PER_MINUTE: NonZeroU32 = NonZeroU32::new(20).unwrap();

/// Limits writes per client IP to a burst of 20, then one every 6 seconds (10 a minute). Reads
/// pass untouched: route guards poll the session on every page load.
pub fn client_ip_layer() -> GovernorLayer<ClientIp, NoOpMiddleware, Body> {
    let config = GovernorConfigBuilder::default()
        .period(Duration::from_secs(6))
        .burst_size(20)
        .methods(vec![
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .key_extractor(ClientIp)
        .finish()
        .expect("period and burst size are nonzero");
    sweep(config.limiter());

    GovernorLayer::new(config).error_handler(too_many_requests)
}

fn too_many_requests(error: GovernorError) -> Response<Body> {
    match error {
        // governor truncates the wait to whole seconds
        GovernorError::TooManyRequests { wait_time, .. } => {
            AppError::TooManyRequests(Duration::from_secs(wait_time + 1)).into_response()
        }
        error => AppError::Internal(error.into()).into_response(),
    }
}

/// Names the header a trusted proxy sets to the client's address. Added as a request extension,
/// since the rate limit runs before handlers and cannot read `AppState`.
#[derive(Clone)]
pub struct ClientIpHeader(pub Option<HeaderName>);

/// The address in the [`ClientIpHeader`] when one is named and present, else the peer address
#[derive(Clone, Copy)]
pub struct ClientIp;

impl KeyExtractor for ClientIp {
    type Key = IpAddr;

    fn name(&self) -> &'static str {
        "client IP"
    }

    fn extract<T>(&self, request: &Request<T>) -> Result<Self::Key, GovernorError> {
        let extensions = request.extensions();
        let forwarded = extensions
            .get::<ClientIpHeader>()
            .and_then(|ClientIpHeader(name)| request.headers().get(name.as_ref()?))
            .and_then(|value| value.to_str().ok()?.trim().parse().ok());
        let peer = || {
            extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|ConnectInfo(addr)| addr.ip())
        };

        // Only requests that skip `main`, as tests do, lack a peer address; they share one key
        Ok(forwarded
            .or_else(peer)
            .map_or(IpAddr::V6(Ipv6Addr::UNSPECIFIED), key))
    }

    fn key_name(&self, key: &Self::Key) -> Option<String> {
        Some(key.to_string())
    }
}

/// An IPv6 client usually holds a whole /64, so it counts as one client, or it could rotate
/// through addresses to dodge the limit
fn key(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(ip) => IpAddr::V6(Ipv6Addr::from_bits(ip.to_bits() & !u128::from(u64::MAX))),
        ip @ IpAddr::V4(_) => ip,
    }
}

/// Attempts per account, however many addresses they come from.
///
/// Passwords get 10, then one every 90 seconds. Anyone can spend an account's quota and so lock
/// its owner out of password sign-in for up to 15 minutes, which is why it stays generous. Wrong
/// second-factor codes are capped in the database instead (`two_factor::WRONG_CODE_LIMIT`), so
/// right ones cost nothing.
#[derive(Clone)]
pub struct RateLimits {
    signin: Arc<DefaultKeyedRateLimiter<String>>,
    reauth: Arc<DefaultKeyedRateLimiter<Uuid>>,
    wrong_codes_alert: Arc<DefaultKeyedRateLimiter<Uuid>>,
    whatsapp_link: Arc<DefaultKeyedRateLimiter<Uuid>>,
    agent_calls: Arc<DefaultKeyedRateLimiter<Uuid>>,
    agent_key_failures: Arc<DefaultKeyedRateLimiter<IpAddr>>,
    /// Until when each address over [`Self::agent_key_failures`] stays refused, so a request can
    /// be turned away before its key is looked up, without spending what a good key would
    agent_key_blocked: Arc<Mutex<HashMap<IpAddr, Instant>>>,
    /// Hashes of keys that worked lately, and when. Only a working key gets in, so an attacker
    /// can't grow it, and cloud agents sharing an address with one keep working.
    agent_keys_worked: Arc<Mutex<HashMap<[u8; 32], Instant>>>,
    agent_searches: Arc<DefaultKeyedRateLimiter<Uuid>>,
}

impl Default for RateLimits {
    /// Also starts sweeping the limiters in the background
    fn default() -> Self {
        let account = Quota::with_period(ACCOUNT_REFILL)
            .expect("refill period is nonzero")
            .allow_burst(ACCOUNT_BURST);
        let limits = Self {
            signin: Arc::new(RateLimiter::keyed(account)),
            reauth: Arc::new(RateLimiter::keyed(account)),
            wrong_codes_alert: Arc::new(RateLimiter::keyed(
                Quota::with_period(WRONG_CODES_ALERT_EVERY).expect("alert period is nonzero"),
            )),
            whatsapp_link: Arc::new(RateLimiter::keyed(
                Quota::with_period(LINK_REFILL)
                    .expect("refill period is nonzero")
                    .allow_burst(LINK_BURST),
            )),
            agent_calls: Arc::new(RateLimiter::keyed(Quota::per_minute(
                AGENT_CALLS_PER_MINUTE,
            ))),
            agent_key_failures: Arc::new(RateLimiter::keyed(
                Quota::with_period(AGENT_KEY_FAILURES_REFILL)
                    .expect("refill period is nonzero")
                    .allow_burst(AGENT_KEY_FAILURES_BURST),
            )),
            agent_key_blocked: Arc::default(),
            agent_keys_worked: Arc::default(),
            agent_searches: Arc::new(RateLimiter::keyed(Quota::per_minute(
                AGENT_SEARCHES_PER_MINUTE,
            ))),
        };
        sweep(&limits.signin);
        sweep(&limits.reauth);
        sweep(&limits.wrong_codes_alert);
        sweep(&limits.whatsapp_link);
        sweep(&limits.agent_calls);
        sweep(&limits.agent_key_failures);
        sweep(&limits.agent_searches);
        sweep_until(&limits.agent_key_blocked, Duration::ZERO);
        sweep_until(&limits.agent_keys_worked, AGENT_KEY_REMEMBERED);

        limits
    }
}

impl RateLimits {
    /// Counts a sign-in attempt for `email`, already normalized
    pub(crate) fn signin(&self, email: &str) -> Result<(), AppError> {
        check(&self.signin, &email.to_owned())
    }

    /// Counts a password re-check by the signed-in `user_id`
    pub(crate) fn reauth(&self, user_id: Uuid) -> Result<(), AppError> {
        check(&self.reauth, &user_id)
    }

    /// Whether `user_id` may be emailed about another sign-in that ran out of codes. One every 6
    /// hours, or whoever holds the password could flood the owner's inbox.
    pub(crate) fn wrong_codes_alert(&self, user_id: Uuid) -> bool {
        self.wrong_codes_alert.check_key(&user_id).is_ok()
    }

    /// Counts an attempt by `user_id` to link WhatsApp: 5, then one every 10 minutes. Each opens a
    /// connection to WhatsApp, which limits pair codes per number as well.
    pub(crate) fn whatsapp_link(&self, user_id: Uuid) -> Result<(), AppError> {
        check(&self.whatsapp_link, &user_id)
    }

    /// Counts a request on `/mcp` by the agent connection `connection_id`: 120 a minute
    pub(crate) fn agent_call(&self, connection_id: Uuid) -> Result<(), AppError> {
        check(&self.agent_calls, &connection_id)
    }

    /// Counts a request on `/mcp` from `ip` that brought no usable key: 20, then one every 6
    /// seconds. Requests with a key that works never count, as cloud agents share addresses.
    pub(crate) fn agent_key_failure(&self, ip: IpAddr) -> Result<(), AppError> {
        check(&self.agent_key_failures, &ip).inspect_err(|refused| {
            if let AppError::TooManyRequests(wait) = refused {
                self.agent_key_blocked
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(ip, Instant::now() + *wait);
            }
        })
    }

    /// Refuses `ip` while [`Self::agent_key_failure`] would, without counting this request,
    /// unless it brings a key that worked lately
    pub(crate) fn agent_key_blocked(
        &self,
        ip: IpAddr,
        key_hash: Option<&[u8; 32]>,
    ) -> Result<(), AppError> {
        let now = Instant::now();
        let until = {
            let blocked = self
                .agent_key_blocked
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            match blocked.get(&ip) {
                Some(&until) if until > now => until,
                _ => return Ok(()),
            }
        };
        let worked = key_hash.is_some_and(|hash| {
            self.agent_keys_worked
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(hash)
                .is_some_and(|&at| now.duration_since(at) < AGENT_KEY_REMEMBERED)
        });
        if worked {
            Ok(())
        } else {
            Err(AppError::TooManyRequests(until - now))
        }
    }

    /// Notes that the key hashed to `key_hash` just worked
    pub(crate) fn agent_key_worked(&self, key_hash: [u8; 32]) {
        self.agent_keys_worked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key_hash, Instant::now());
    }

    /// Counts a `find_chats` call by the agent connection `connection_id`: 20 a minute, since each
    /// opens the whole sealed address book
    pub(crate) fn agent_search(&self, connection_id: Uuid) -> Result<(), AppError> {
        check(&self.agent_searches, &connection_id)
    }
}

fn check<K>(limiter: &DefaultKeyedRateLimiter<K>, key: &K) -> Result<(), AppError>
where
    K: Hash + Eq + Clone,
{
    limiter
        .check_key(key)
        .map_err(|refused| AppError::TooManyRequests(refused.wait_time_from(limiter.clock().now())))
}

/// Drops keys whose quota has refilled, which the limiter would treat as new anyway. Holds the
/// limiter weakly, so the sweep ends along with it.
fn sweep<K>(limiter: &Arc<DefaultKeyedRateLimiter<K>>)
where
    K: Hash + Eq + Clone + Send + Sync + 'static,
{
    // The openapi binary builds the router outside a runtime, and never serves it
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let limiter = Arc::downgrade(limiter);
    runtime.spawn(async move {
        let mut interval = tokio::time::interval(SWEEP_EVERY);
        loop {
            interval.tick().await;
            let Some(limiter) = limiter.upgrade() else {
                break;
            };
            limiter.retain_recent();
            limiter.shrink_to_fit();
        }
    });
}

/// Drops the entries whose time, plus `kept`, has passed
fn sweep_until<K>(entries: &Arc<Mutex<HashMap<K, Instant>>>, kept: Duration)
where
    K: Hash + Eq + Send + 'static,
{
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let entries = Arc::downgrade(entries);
    runtime.spawn(async move {
        let mut interval = tokio::time::interval(SWEEP_EVERY);
        loop {
            interval.tick().await;
            let Some(entries) = entries.upgrade() else {
                break;
            };
            let now = Instant::now();
            entries
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .retain(|_, at| *at + kept > now);
        }
    });
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::RateLimits;

    #[test]
    fn the_wrong_codes_alert_goes_out_once_a_window_per_user() {
        let limits = RateLimits::default();
        let (ada, grace) = (Uuid::new_v4(), Uuid::new_v4());

        assert!(limits.wrong_codes_alert(ada));
        assert!(!limits.wrong_codes_alert(ada));
        assert!(limits.wrong_codes_alert(grace));
    }
}
