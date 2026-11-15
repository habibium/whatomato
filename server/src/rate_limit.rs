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
/// Lookups of well-formed keys an address over its failure limit still gets: 60, then one a second.
/// Three times the failure burst and six times its refill, so an agent sharing the address keeps
/// getting in, while a flood of wrong keys costs at most one indexed query a second per address.
const AGENT_KEY_LOOKUPS_BURST: NonZeroU32 = NonZeroU32::new(60).unwrap();
const AGENT_KEY_LOOKUPS_REFILL: Duration = Duration::from_secs(1);
/// Entries each of the agent key maps may hold
const AGENT_KEY_ENTRIES_MAX: usize = 10_000;
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
    /// Until when each address is over [`Self::agent_key_failures`]. A well-formed key from it
    /// still gets looked up, within [`Self::agent_key_lookups`].
    agent_key_blocked: Arc<Mutex<HashMap<IpAddr, Instant>>>,
    agent_key_lookups: Arc<DefaultKeyedRateLimiter<IpAddr>>,
    /// Hashes of keys that worked lately, and when: these skip [`Self::agent_key_lookups`]. Empty
    /// after a restart, so nothing may depend on it for a real key to get in.
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
            agent_key_lookups: Arc::new(RateLimiter::keyed(
                Quota::with_period(AGENT_KEY_LOOKUPS_REFILL)
                    .expect("refill period is nonzero")
                    .allow_burst(AGENT_KEY_LOOKUPS_BURST),
            )),
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
        sweep(&limits.agent_key_lookups);
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
                let now = Instant::now();
                remember(
                    &self.agent_key_blocked,
                    ip,
                    now + *wait,
                    Duration::ZERO,
                    now,
                );
            }
        })
    }

    /// Whether the well-formed key hashed to `key_hash`, from `ip`, may be looked up. Always, unless
    /// `ip` is over [`Self::agent_key_failure`]; then a key that worked lately may, and any other
    /// only within the address's lookup budget.
    pub(crate) fn agent_key_lookup(&self, ip: IpAddr, key_hash: &[u8; 32]) -> Result<(), AppError> {
        let now = Instant::now();
        let blocked = self
            .agent_key_blocked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&ip)
            .is_some_and(|&until| until > now);
        if !blocked {
            return Ok(());
        }
        let worked = self
            .agent_keys_worked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key_hash)
            .is_some_and(|&at| now.duration_since(at) < AGENT_KEY_REMEMBERED);
        if worked {
            return Ok(());
        }
        check(&self.agent_key_lookups, &ip)
    }

    /// Notes that the key hashed to `key_hash` just worked
    pub(crate) fn agent_key_worked(&self, key_hash: [u8; 32]) {
        let now = Instant::now();
        remember(
            &self.agent_keys_worked,
            key_hash,
            now,
            AGENT_KEY_REMEMBERED,
            now,
        );
    }

    /// Counts a `find_chats` call by the agent connection `connection_id`: 20 a minute, since each
    /// opens the whole sealed address book
    pub(crate) fn agent_search(&self, connection_id: Uuid) -> Result<(), AppError> {
        check(&self.agent_searches, &connection_id)
    }
}

/// Sets `key` to `at` unless the map is full of entries whose time, plus `kept`, is still ahead;
/// then the new one is dropped, which costs only a fast path
fn remember<K: Hash + Eq>(
    entries: &Mutex<HashMap<K, Instant>>,
    key: K,
    at: Instant,
    kept: Duration,
    now: Instant,
) {
    let mut entries = entries.lock().unwrap_or_else(PoisonError::into_inner);
    if entries.len() >= AGENT_KEY_ENTRIES_MAX && !entries.contains_key(&key) {
        entries.retain(|_, time| *time + kept > now);
        if entries.len() >= AGENT_KEY_ENTRIES_MAX {
            return;
        }
    }
    entries.insert(key, at);
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
    use std::{
        net::{IpAddr, Ipv4Addr},
        time::{Duration, Instant},
    };

    use uuid::Uuid;

    use super::{AGENT_KEY_ENTRIES_MAX, RateLimits};

    #[test]
    fn the_agent_key_maps_stay_bounded() {
        let limits = RateLimits::default();
        let count = u32::try_from(AGENT_KEY_ENTRIES_MAX).unwrap() + 100;
        for n in 0..count {
            let mut hash = [0; 32];
            hash[..4].copy_from_slice(&n.to_be_bytes());
            limits.agent_key_worked(hash);
            let ip = IpAddr::V4(Ipv4Addr::from_bits(n));
            for _ in 0..21 {
                let _ = limits.agent_key_failure(ip);
            }
        }
        let worked = limits.agent_keys_worked.lock().unwrap().len();
        let blocked = limits.agent_key_blocked.lock().unwrap().len();
        assert_eq!(worked, AGENT_KEY_ENTRIES_MAX);
        assert_eq!(blocked, AGENT_KEY_ENTRIES_MAX);

        // Entries that have run out make room
        let past = Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
        for until in limits.agent_key_blocked.lock().unwrap().values_mut() {
            *until = past;
        }
        let _ = limits.agent_key_failure(IpAddr::V4(Ipv4Addr::LOCALHOST));
        for _ in 0..21 {
            let _ = limits.agent_key_failure(IpAddr::V4(Ipv4Addr::BROADCAST));
        }
        assert_eq!(limits.agent_key_blocked.lock().unwrap().len(), 1);
    }

    #[test]
    fn the_wrong_codes_alert_goes_out_once_a_window_per_user() {
        let limits = RateLimits::default();
        let (ada, grace) = (Uuid::new_v4(), Uuid::new_v4());

        assert!(limits.wrong_codes_alert(ada));
        assert!(!limits.wrong_codes_alert(ada));
        assert!(limits.wrong_codes_alert(grace));
    }
}
