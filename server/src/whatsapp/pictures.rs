//! Profile pictures of contacts and groups, looked up as the app shows them and kept in memory only

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use tokio::{
    sync::Semaphore,
    time::{Instant, sleep, timeout},
};
use whatsapp_rust::{
    ProfilePictureLookup, ProfilePictureRequest, ProfilePictureTarget, ProfilePictureType,
    http::{HttpClient, HttpRequest},
    prelude::{Client, Jid},
};

pub type Image = Arc<[u8]>;

const FOUND_FOR: Duration = Duration::from_hours(6);
const MISSING_FOR: Duration = Duration::from_hours(1);
/// WhatsApp Web looks pictures up as chats scroll into view; a burst well past that pace is the
/// kind of traffic that gets a number flagged, so lookups go two at a time with a gap after each
const AT_ONCE: usize = 2;
const GAP: Duration = Duration::from_millis(300);
/// A refusal, possibly a rate limit, stops new lookups for this long
const PAUSE: Duration = Duration::from_mins(5);
const LOOKUP_LIMIT: Duration = Duration::from_secs(15);
/// A preview is a few kilobytes; anything far larger isn't one
const MAX_BYTES: usize = 256 * 1024;
const KEPT: usize = 2_000;

/// What memory knows about one picture
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Known {
    Picture(Image),
    /// None set, private, or one that didn't download
    Missing,
}

impl Known {
    pub(super) fn image(self) -> Option<Image> {
        match self {
            Self::Picture(image) => Some(image),
            Self::Missing => None,
        }
    }
}

struct Entry {
    image: Option<Image>,
    until: Instant,
}

/// One account's pictures
pub struct Pictures {
    entries: Mutex<HashMap<String, Entry>>,
    paused_until: Mutex<Option<Instant>>,
    lookups: Semaphore,
}

impl Pictures {
    pub(super) fn new() -> Self {
        Self {
            entries: Mutex::default(),
            paused_until: Mutex::default(),
            lookups: Semaphore::new(AT_ONCE),
        }
    }

    /// While the answer is fresh
    pub(super) fn cached(&self, jid: &str) -> Option<Known> {
        lock(&self.entries)
            .get(jid)
            .filter(|entry| entry.until > Instant::now())
            .map(|entry| entry.image.clone().map_or(Known::Missing, Known::Picture))
    }

    pub(super) fn clear(&self) {
        lock(&self.entries).clear();
    }

    /// `None` covers no picture, a private one, and a lookup that failed or was held back
    pub(super) async fn fetch(
        &self,
        client: &Client,
        http: &dyn HttpClient,
        jid: &Jid,
    ) -> Option<Image> {
        let key = jid.to_string();
        if let Some(known) = self.cached(&key) {
            return known.image();
        }
        if self.is_paused() {
            return None;
        }
        let _permit = self.lookups.acquire().await.ok()?;
        // Another request for the same picture may have answered while this one waited
        if let Some(known) = self.cached(&key) {
            return known.image();
        }
        let target = if jid.server == whatsapp_rust::prelude::Server::Group {
            ProfilePictureTarget::Group(jid)
        } else {
            ProfilePictureTarget::Contact(jid)
        };
        let request = ProfilePictureRequest::new(target, ProfilePictureType::Preview);
        let image = match timeout(LOOKUP_LIMIT, client.pictures().lookup(request)).await {
            Ok(Ok(ProfilePictureLookup::Found(picture))) => download(http, &picture.url).await,
            Ok(Ok(_)) => None,
            Ok(Err(_)) | Err(_) => {
                self.pause();
                return None;
            }
        };
        self.store(key, image.clone());
        sleep(GAP).await;
        image
    }

    fn store(&self, key: String, image: Option<Image>) {
        let now = Instant::now();
        let until = now
            + if image.is_some() {
                FOUND_FOR
            } else {
                MISSING_FOR
            };
        let mut entries = lock(&self.entries);
        if entries.len() >= KEPT {
            entries.retain(|_, entry| entry.until > now);
        }
        if entries.len() < KEPT {
            entries.insert(key, Entry { image, until });
        }
    }

    fn is_paused(&self) -> bool {
        lock(&self.paused_until).is_some_and(|until| until > Instant::now())
    }

    fn pause(&self) {
        tracing::warn!("a WhatsApp picture lookup failed; holding the rest back for 5 minutes");
        *lock(&self.paused_until) = Some(Instant::now() + PAUSE);
    }
}

async fn download(http: &dyn HttpClient, url: &str) -> Option<Image> {
    let response = timeout(LOOKUP_LIMIT, http.execute(HttpRequest::get(url)))
        .await
        .ok()?
        .ok()?;
    let fits = (1..=MAX_BYTES).contains(&response.body.len());
    ((200..300).contains(&response.status_code) && fits).then(|| response.body.into())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn answers_last_as_long_as_they_should() {
        let pictures = Pictures::new();
        let image: Image = Arc::from(&b"jpeg"[..]);
        pictures.store("a@s.whatsapp.net".into(), Some(Arc::clone(&image)));
        pictures.store("b@s.whatsapp.net".into(), None);
        assert_eq!(
            pictures.cached("a@s.whatsapp.net"),
            Some(Known::Picture(image))
        );
        assert_eq!(pictures.cached("b@s.whatsapp.net"), Some(Known::Missing));
        assert_eq!(pictures.cached("c@s.whatsapp.net"), None);

        tokio::time::advance(MISSING_FOR).await;
        assert!(pictures.cached("a@s.whatsapp.net").is_some());
        assert_eq!(
            pictures.cached("b@s.whatsapp.net"),
            None,
            "a missing picture is asked about again"
        );
        tokio::time::advance(FOUND_FOR).await;
        assert_eq!(pictures.cached("a@s.whatsapp.net"), None);
    }

    #[tokio::test(start_paused = true)]
    async fn a_full_cache_makes_room_from_stale_answers_only() {
        let pictures = Pictures::new();
        for index in 0..KEPT {
            pictures.store(format!("{index}@s.whatsapp.net"), None);
        }
        pictures.store("new@s.whatsapp.net".into(), None);
        assert_eq!(pictures.cached("new@s.whatsapp.net"), None);

        tokio::time::advance(MISSING_FOR).await;
        pictures.store("new@s.whatsapp.net".into(), None);
        assert_eq!(pictures.cached("new@s.whatsapp.net"), Some(Known::Missing));
    }

    #[tokio::test(start_paused = true)]
    async fn a_pause_ends() {
        let pictures = Pictures::new();
        pictures.pause();
        assert!(pictures.is_paused());
        tokio::time::advance(PAUSE).await;
        assert!(!pictures.is_paused());
    }
}
