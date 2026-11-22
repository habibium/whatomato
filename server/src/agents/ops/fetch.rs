//! Fetching a file an agent names by address. The address is the agent's word, so only https on
//! the public internet is reached: no redirect, no proxy, and no name that resolves inward.

use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

use reqwest::{header, redirect};
use rustls::{ClientConfig, RootCertStore};
use url::Url;

use super::OpError;
use crate::{oauth::client::PublicOnly, state::AppState, whatsapp::media::FILE_MAX};

const TIMEOUT: Duration = Duration::from_secs(30);

fn client() -> Result<&'static reqwest::Client, OpError> {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return Ok(client);
    }
    let tls =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|error| OpError::Internal(error.into()))?
            .with_root_certificates(RootCertStore {
                roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
            })
            .with_no_client_auth();
    let built = reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        .user_agent(concat!("whatomato/", env!("CARGO_PKG_VERSION")))
        .timeout(TIMEOUT)
        .https_only(true)
        .redirect(redirect::Policy::none())
        .no_proxy()
        .dns_resolver(Arc::new(PublicOnly))
        .build()
        .map_err(|error| OpError::Internal(error.into()))?;
    Ok(CLIENT.get_or_init(|| built))
}

/// The file at `address`, with the type its answer named and the name its path ends in
///
/// # Errors
/// [`OpError::Invalid`] for an address that isn't public https, doesn't answer with a file, or
/// answers with more than 16 MB.
pub async fn file(
    _state: &AppState,
    address: &str,
) -> Result<(Vec<u8>, Option<String>, Option<String>), OpError> {
    let url = Url::parse(address.trim())
        .ok()
        .filter(|url| url.scheme() == "https" && url.host().is_some())
        .ok_or_else(|| OpError::invalid("`attachment.url` must be an https address."))?;
    let name = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|name| !name.is_empty())
        .map(percent_decoded);
    let unreachable = |why: &str| {
        OpError::invalid(&format!(
            "The file at `attachment.url` couldn't be fetched: {why}."
        ))
    };
    let mut response =
        client()?.get(url).send().await.map_err(|_| {
            unreachable("the address didn't answer, or isn't on the public internet")
        })?;
    let status = response.status();
    if status.is_redirection() {
        return Err(unreachable(
            "it redirects elsewhere; give the address it leads to",
        ));
    }
    if !status.is_success() {
        return Err(unreachable(&format!("it answered {status}")));
    }
    if response
        .content_length()
        .is_some_and(|length| length > FILE_MAX as u64)
    {
        return Err(OpError::invalid("The attachment is larger than 16 MB."));
    }
    let mime_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .filter(|mime| !mime.starts_with("text/html"));
    let mut data = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| unreachable("the download broke off"))?
    {
        if data.len() + chunk.len() > FILE_MAX {
            return Err(OpError::invalid("The attachment is larger than 16 MB."));
        }
        data.extend_from_slice(&chunk);
    }
    Ok((data, mime_type, name))
}

fn percent_decoded(text: &str) -> String {
    url::form_urlencoded::parse(format!("n={}", text.replace('+', "%2B")).as_bytes())
        .next()
        .map_or_else(|| text.to_owned(), |(_, value)| value.into_owned())
}

#[cfg(test)]
mod tests {
    use super::percent_decoded;

    #[test]
    fn a_names_escapes_are_undone() {
        assert_eq!(percent_decoded("March%20invoice.pdf"), "March invoice.pdf");
        assert_eq!(percent_decoded("a+b.txt"), "a+b.txt");
    }
}
