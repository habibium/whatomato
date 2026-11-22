//! The app that signs in, known by its Client ID Metadata Document: a JSON file at the https URL
//! the app gives as its `client_id`. Everything in it is the app's own word.

use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use reqwest::{
    dns::{Addrs, Name, Resolve, Resolving},
    header, redirect,
};
use rustls::{ClientConfig, RootCertStore};
use serde::Deserialize;
use url::{Host, Url};

const TIMEOUT: Duration = Duration::from_secs(5);
/// The draft suggests 5 kB; documents that carry keys run larger
const SIZE_MAX: usize = 16 * 1024;
const CACHE_FOR: Duration = Duration::from_mins(10);
const CACHE_MAX: usize = 256;
/// As `agent_connections.client_id` and `client_name` allow
const ID_MAX: usize = 2048;
const NAME_MAX: usize = 120;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Client {
    pub id: String,
    /// What the app calls itself, trimmed to what a page can show
    pub name: String,
    redirect_uris: Vec<String>,
}

/// Why an app can't be let in. Each reads as the sentence the owner sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ClientError {
    #[error(
        "This app didn't identify itself in a way Whatomato accepts, so it can't sign in. Connect \
         it with an access key instead."
    )]
    BadId,
    #[error("Whatomato couldn't check who this app is. Try again in a moment.")]
    Unreachable,
    #[error("This app's published details aren't valid, so it can't sign in.")]
    Invalid,
}

impl Client {
    /// Whether the app registered `redirect_uri`. The match is exact, except that an address on
    /// the user's own machine may use any port (RFC 8252 section 7.3), and anything that is neither
    /// https nor on that machine is refused.
    #[must_use]
    pub fn allows_redirect(&self, redirect_uri: &str) -> bool {
        let Ok(asked) = Url::parse(redirect_uri) else {
            return false;
        };
        if asked.fragment().is_some() {
            return false;
        }
        if is_loopback(&asked) {
            return self.redirect_uris.iter().any(|registered| {
                Url::parse(registered).is_ok_and(|mut registered| {
                    let mut asked = asked.clone();
                    is_loopback(&registered)
                        && registered.set_port(None).is_ok()
                        && asked.set_port(None).is_ok()
                        && registered == asked
                })
            });
        }
        asked.scheme() == "https" && self.redirect_uris.iter().any(|uri| uri == redirect_uri)
    }

    /// Whether the app can only be reached on the user's own machine, where any program may claim
    /// to be it
    #[must_use]
    pub fn is_local_only(&self) -> bool {
        self.redirect_uris
            .iter()
            .all(|uri| Url::parse(uri).is_ok_and(|uri| is_loopback(&uri)))
    }
}

fn is_loopback(url: &Url) -> bool {
    url.scheme() == "http"
        && match url.host() {
            Some(Host::Domain(name)) => name == "localhost",
            Some(Host::Ipv4(ip)) => ip.is_loopback(),
            Some(Host::Ipv6(ip)) => ip.is_loopback(),
            None => false,
        }
}

/// The host a `client_id` is published on, or the whole of it when it isn't a URL
#[must_use]
pub fn host(client_id: &str) -> String {
    Url::parse(client_id)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| client_id.to_owned())
}

/// The host of a redirect address, with its port when it has one
#[must_use]
pub fn redirect_host(redirect_uri: &str) -> String {
    Url::parse(redirect_uri)
        .ok()
        .and_then(|url| {
            let host = url.host_str()?;
            Some(
                url.port()
                    .map_or_else(|| host.to_owned(), |port| format!("{host}:{port}")),
            )
        })
        .unwrap_or_else(|| redirect_uri.to_owned())
}

/// Each app by its `client_id`, with when it was fetched
type Cache = Mutex<HashMap<String, (Instant, Arc<Client>)>>;

/// Looks apps up by `client_id`, remembering each for ten minutes
#[derive(Clone)]
pub enum Clients {
    Fetched {
        http: reqwest::Client,
        cache: Arc<Cache>,
    },
    /// Knows these apps and no others, without the network
    #[cfg(test)]
    Listed(Arc<Vec<Client>>),
}

impl Clients {
    /// # Errors
    /// The HTTP client cannot be built.
    pub fn fetched() -> anyhow::Result<Self> {
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
            // A redirect could lead anywhere, and a proxy would do the resolving in our place
            .redirect(redirect::Policy::none())
            .no_proxy()
            .dns_resolver(Arc::new(PublicOnly))
            .build()?;
        Ok(Self::Fetched {
            http,
            cache: Arc::default(),
        })
    }

    #[cfg(test)]
    pub fn listed(clients: impl IntoIterator<Item = Client>) -> Self {
        Self::Listed(Arc::new(clients.into_iter().collect()))
    }

    /// # Errors
    /// See [`ClientError`]. Failures are never remembered.
    pub async fn get(&self, client_id: &str) -> Result<Arc<Client>, ClientError> {
        let url = parse_id(client_id)?;
        match self {
            Self::Fetched { http, cache } => {
                let now = Instant::now();
                let cached = cache
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(client_id)
                    .filter(|(at, _)| now.duration_since(*at) < CACHE_FOR)
                    .map(|(_, client)| Arc::clone(client));
                if let Some(client) = cached {
                    return Ok(client);
                }
                let client = Arc::new(fetch(http, client_id, url).await?);
                remember(cache, client_id, &client, now);
                Ok(client)
            }
            #[cfg(test)]
            Self::Listed(clients) => clients
                .iter()
                .find(|client| client.id == client_id)
                .map(|client| Arc::new(client.clone()))
                .ok_or(ClientError::Unreachable),
        }
    }
}

fn remember(cache: &Cache, client_id: &str, client: &Arc<Client>, now: Instant) {
    let mut cache = cache.lock().unwrap_or_else(PoisonError::into_inner);
    if cache.len() >= CACHE_MAX {
        cache.retain(|_, (at, _)| now.duration_since(*at) < CACHE_FOR);
        if cache.len() >= CACHE_MAX {
            cache.clear();
        }
    }
    cache.insert(client_id.to_owned(), (now, Arc::clone(client)));
}

/// A `client_id` as the Client ID Metadata Document draft, section 3, requires it
fn parse_id(client_id: &str) -> Result<Url, ClientError> {
    let url = Url::parse(client_id).map_err(|_| ClientError::BadId)?;
    // `Url` resolves dot segments while parsing, so they only show in what was sent
    let path = client_id.split(['?', '#']).next().unwrap_or_default();
    let has_dot_segment = path
        .split('/')
        .skip(3)
        .any(|part| matches!(part, "." | ".."));
    let ok = client_id.len() <= ID_MAX
        && url.scheme() == "https"
        && url.host().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && !has_dot_segment
        // The resolver below never sees an address written out in the URL
        && match url.host() {
            Some(Host::Ipv4(ip)) => is_public(IpAddr::V4(ip)),
            Some(Host::Ipv6(ip)) => is_public(IpAddr::V6(ip)),
            _ => true,
        };
    if ok { Ok(url) } else { Err(ClientError::BadId) }
}

#[derive(Deserialize)]
struct Document {
    client_id: String,
    client_name: Option<String>,
    redirect_uris: Vec<String>,
    token_endpoint_auth_method: Option<String>,
    client_secret: Option<serde_json::Value>,
}

async fn fetch(http: &reqwest::Client, client_id: &str, url: Url) -> Result<Client, ClientError> {
    let mut response = http
        .get(url)
        .header(header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|error| {
            tracing::debug!(%error, "couldn't fetch a client's metadata document");
            ClientError::Unreachable
        })?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(ClientError::Unreachable);
    }
    let too_large = |len: usize| len > SIZE_MAX;
    if response
        .content_length()
        .is_some_and(|len| usize::try_from(len).map_or(true, too_large))
    {
        return Err(ClientError::Invalid);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ClientError::Unreachable)?
    {
        if too_large(body.len() + chunk.len()) {
            return Err(ClientError::Invalid);
        }
        body.extend_from_slice(&chunk);
    }
    read(client_id, &body)
}

/// The app a fetched document describes, when the document is one `client_id` may use
fn read(client_id: &str, body: &[u8]) -> Result<Client, ClientError> {
    let document: Document = serde_json::from_slice(body).map_err(|_| ClientError::Invalid)?;
    let shares_a_secret = document.client_secret.is_some()
        || document
            .token_endpoint_auth_method
            .is_some_and(|method| method.starts_with("client_secret"));
    if document.client_id != client_id || document.redirect_uris.is_empty() || shares_a_secret {
        return Err(ClientError::Invalid);
    }
    let name: String = document
        .client_name
        .unwrap_or_default()
        .chars()
        .filter(|char| !char.is_control())
        .take(NAME_MAX)
        .collect();
    let name = name.trim();
    Ok(Client {
        name: if name.is_empty() {
            host(client_id)
        } else {
            name.to_owned()
        },
        id: document.client_id,
        redirect_uris: document.redirect_uris,
    })
}

/// Resolves names to public addresses only, so a document can't be fetched from this server's own
/// network. The connection is made to an address this returned, which rules out a second lookup
/// that answers differently.
pub(crate) struct PublicOnly;

impl Resolve for PublicOnly {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((name.as_str(), 0))
                .await?
                .filter(|addr| is_public(addr.ip()))
                .collect();
            if addrs.is_empty() {
                return Err("the name has no public address".into());
            }
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

fn is_public(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || ip.is_multicast()
                || ip.is_documentation()
                || a == 0
                // Shared address space, protocol assignments, benchmarking, reserved
                || (a == 100 && b & 0xc0 == 64)
                || (a == 192 && b == 0 && c == 0)
                || (a == 198 && b & 0xfe == 18)
                || a >= 240)
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                // IPv4-compatible, documentation, and NAT64, which reaches IPv4 addresses
                || segments[..6] == [0; 6]
                || (segments[0] == 0x2001 && segments[1] == 0xdb8)
                || (segments[0] == 0x64 && segments[1] == 0xff9b))
        }
    }
}

/// The apps a test server knows: one on the web, one on the user's machine
#[cfg(test)]
pub(crate) fn test_clients() -> [Client; 2] {
    [
        test_client(
            "https://app.example/oauth/client.json",
            "Example App",
            &["https://app.example/callback"],
        ),
        test_client(
            "https://cli.example/client.json",
            "Example CLI",
            &["http://localhost/callback", "http://127.0.0.1/callback"],
        ),
    ]
}

#[cfg(test)]
pub(crate) fn test_client(id: &str, name: &str, redirect_uris: &[&str]) -> Client {
    Client {
        id: id.to_owned(),
        name: name.to_owned(),
        redirect_uris: redirect_uris.iter().map(|&uri| uri.to_owned()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "https://app.example/oauth/client.json";

    fn document(extra: &str) -> String {
        format!(
            r#"{{"client_id":"{ID}","client_name":"Example","redirect_uris":["https://app.example/callback","http://localhost/callback","http://127.0.0.1:3000/cb"]{extra}}}"#
        )
    }

    #[test]
    fn a_client_id_is_an_https_url_without_tricks() {
        for good in [
            ID,
            "https://app.example/",
            "https://app.example:8443/client.json?v=2",
        ] {
            assert!(parse_id(good).is_ok(), "{good}");
        }
        let long = format!("https://app.example/{}", "a".repeat(ID_MAX));
        for bad in [
            "app.example/client.json",
            "http://app.example/client.json",
            "https://user@app.example/client.json",
            "https://user:pass@app.example/client.json",
            "https://app.example/client.json#frag",
            "https://app.example/a/../client.json",
            "https://app.example/./client.json",
            "https://127.0.0.1/client.json",
            "https://10.0.0.8/client.json",
            "https://[::1]/client.json",
            "https://[fd00::1]/client.json",
            long.as_str(),
            "",
        ] {
            assert_eq!(parse_id(bad), Err(ClientError::BadId), "{bad}");
        }
    }

    #[test]
    fn a_document_must_be_the_one_at_its_own_address() {
        let client = read(ID, document("").as_bytes()).unwrap();
        assert_eq!(client.id, ID);
        assert_eq!(client.name, "Example");

        for bad in [
            document("").replace(ID, "https://other.example/client.json"),
            document(r#","client_secret":"s""#),
            document(r#","token_endpoint_auth_method":"client_secret_post""#),
            format!(r#"{{"client_id":"{ID}","client_name":"Example","redirect_uris":[]}}"#),
            format!(r#"{{"client_id":"{ID}","client_name":"Example"}}"#),
            "not json".to_owned(),
        ] {
            assert_eq!(read(ID, bad.as_bytes()), Err(ClientError::Invalid), "{bad}");
        }
        // A public client, or one that signs its requests, is fine: every client is held to PKCE
        for method in ["none", "private_key_jwt"] {
            let extra = format!(r#","token_endpoint_auth_method":"{method}""#);
            assert!(read(ID, document(&extra).as_bytes()).is_ok(), "{method}");
        }
    }

    #[test]
    fn a_name_is_cut_to_what_a_page_can_show() {
        let named = |name: &str| {
            let body = document("").replace("Example", name);
            read(ID, body.as_bytes()).unwrap().name
        };
        assert_eq!(named("  Claude  "), "Claude");
        assert_eq!(named(r"Cl\u0000au\nde"), "Claude");
        assert_eq!(named(&"a".repeat(500)).len(), NAME_MAX);
        assert_eq!(named("   "), "app.example");
        let nameless = format!(r#"{{"client_id":"{ID}","redirect_uris":["https://a.example/"]}}"#);
        assert_eq!(read(ID, nameless.as_bytes()).unwrap().name, "app.example");
    }

    #[test]
    fn a_redirect_must_be_registered_and_only_a_local_one_may_change_port() {
        let client = read(ID, document("").as_bytes()).unwrap();
        for allowed in [
            "https://app.example/callback",
            "http://localhost/callback",
            "http://localhost:51234/callback",
            "http://127.0.0.1:3000/cb",
            "http://127.0.0.1:9/cb",
        ] {
            assert!(client.allows_redirect(allowed), "{allowed}");
        }
        for refused in [
            "https://app.example/callback/",
            "https://app.example/callback?x=1",
            "https://app.example:8443/callback",
            "https://evil.example/callback",
            "http://app.example/callback",
            "http://localhost/other",
            "http://localhost/callback#frag",
            "http://127.0.0.1/callback",
            "http://localhost.evil.example/callback",
            "https://localhost/callback",
            "custom://callback",
            "",
        ] {
            assert!(!client.allows_redirect(refused), "{refused}");
        }
        assert!(!client.is_local_only());
        let local = test_client(ID, "Local", &["http://localhost/cb", "http://[::1]/cb"]);
        assert!(local.is_local_only());
        assert!(local.allows_redirect("http://[::1]:4000/cb"));
    }

    #[test]
    fn only_public_addresses_are_fetched_from() {
        for public in ["1.1.1.1", "93.184.216.34", "2606:4700::1111"] {
            assert!(is_public(public.parse().unwrap()), "{public}");
        }
        for private in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "198.18.0.1",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "::",
            "fe80::1",
            "fd12:3456::1",
            "::ffff:10.0.0.1",
            "::ffff:127.0.0.1",
            "::10.0.0.1",
            "64:ff9b::a00:1",
        ] {
            assert!(!is_public(private.parse().unwrap()), "{private}");
        }
    }

    #[test]
    fn hosts_read_from_what_the_client_sent() {
        assert_eq!(host(ID), "app.example");
        assert_eq!(host("not a url"), "not a url");
        assert_eq!(
            redirect_host("http://localhost:8080/callback"),
            "localhost:8080"
        );
        assert_eq!(
            redirect_host("https://claude.ai/api/mcp/auth_callback"),
            "claude.ai"
        );
    }
}
