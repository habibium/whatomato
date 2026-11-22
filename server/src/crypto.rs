use std::sync::Arc;

use anyhow::{Context, anyhow};
use chacha20poly1305::{
    Key, XChaCha20Poly1305, XNonce,
    aead::{Aead, Generate, KeyInit, Payload},
};
use ring::hmac;
use zeroize::Zeroizing;

/// Leads every sealed value and names the key that sealed it, so a new key can take over while
/// values sealed with the old one still open
const VERSION: u8 = 1;
const NONCE_LEN: usize = 24;
/// Derives the tag key from the encryption key; changing it orphans every stored tag
const TAG_KEY_LABEL: &[u8] = b"whatomato tags v1";

/// Encrypts secrets the server has to read back, such as two-step verification keys, so a copy
/// of the database alone does not give them away. A sealed value is `version || nonce ||
/// ciphertext`.
#[derive(Clone)]
pub struct Cipher {
    aead: Arc<XChaCha20Poly1305>,
    tags: hmac::Key,
}

impl Cipher {
    #[must_use]
    pub fn new(key: &[u8; 32]) -> Self {
        let derived = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), TAG_KEY_LABEL);
        Self {
            aead: Arc::new(XChaCha20Poly1305::new(&Key::from(*key))),
            tags: hmac::Key::new(hmac::HMAC_SHA256, derived.as_ref()),
        }
    }

    /// A keyed hash of `data`: equal inputs give equal tags, so a stored tag can be matched
    /// without storing `data`, but nobody without the key can compute or reverse one. `context`
    /// keeps one row's or account's tags from matching another's.
    #[must_use]
    pub fn tag(&self, context: &[u8], data: &[u8]) -> [u8; 32] {
        let mut signing = hmac::Context::with_key(&self.tags);
        signing.update(
            &u64::try_from(context.len())
                .unwrap_or(u64::MAX)
                .to_be_bytes(),
        );
        signing.update(context);
        signing.update(data);
        let mut tag = [0; 32];
        tag.copy_from_slice(signing.sign().as_ref());
        tag
    }

    /// `context` is authenticated but not stored. Opening needs the same bytes, so a value copied
    /// into another row, such as another user's, fails to open.
    ///
    /// # Errors
    /// The system random number generator failed.
    pub fn encrypt(&self, plaintext: &[u8], context: &[u8]) -> anyhow::Result<Vec<u8>> {
        // 192 random bits never repeat in practice, which a 96-bit nonce cannot promise
        let nonce = XNonce::try_generate()?;
        let aad = [&[VERSION], context].concat();
        let ciphertext = self
            .aead
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| anyhow!("encryption failed"))?;

        Ok([&[VERSION], nonce.as_slice(), &ciphertext].concat())
    }

    /// # Errors
    /// `sealed` is malformed, came from another key or key version, was altered, or was sealed
    /// with a different `context`.
    pub fn decrypt(&self, sealed: &[u8], context: &[u8]) -> anyhow::Result<Zeroizing<Vec<u8>>> {
        let (&version, rest) = sealed.split_first().context("sealed value is empty")?;
        anyhow::ensure!(
            version == VERSION,
            "sealed with unknown key version {version}"
        );
        let (nonce, ciphertext) = rest
            .split_at_checked(NONCE_LEN)
            .context("sealed value is truncated")?;
        let aad = [&[version], context].concat();
        let plaintext = self
            .aead
            .decrypt(
                nonce.try_into()?,
                Payload {
                    msg: ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| anyhow!("sealed value failed authentication"))?;

        Ok(Zeroizing::new(plaintext))
    }
}

#[cfg(test)]
mod tests {
    use super::Cipher;

    const KEY: [u8; 32] = [7; 32];

    #[test]
    fn opens_what_it_sealed() {
        let cipher = Cipher::new(&KEY);
        let sealed = cipher.encrypt(b"secret", b"ada").unwrap();

        assert_eq!(
            cipher.decrypt(&sealed, b"ada").unwrap().as_slice(),
            b"secret"
        );
        assert!(!sealed.windows(6).any(|window| window == b"secret"));
    }

    #[test]
    fn every_seal_uses_a_fresh_nonce() {
        let cipher = Cipher::new(&KEY);

        assert_ne!(
            cipher.encrypt(b"secret", b"ada").unwrap(),
            cipher.encrypt(b"secret", b"ada").unwrap()
        );
    }

    #[test]
    fn refuses_tampered_values() {
        let cipher = Cipher::new(&KEY);
        let sealed = cipher.encrypt(b"secret", b"ada").unwrap();

        for i in 1..sealed.len() {
            let mut tampered = sealed.clone();
            tampered[i] ^= 1;
            assert!(cipher.decrypt(&tampered, b"ada").is_err(), "byte {i}");
        }
        assert!(cipher.decrypt(&sealed[..sealed.len() - 1], b"ada").is_err());
        assert!(cipher.decrypt(&sealed[..10], b"ada").is_err());
        assert!(cipher.decrypt(&[], b"ada").is_err());
    }

    #[test]
    fn tags_depend_on_the_key_the_context_and_the_data() {
        let cipher = Cipher::new(&KEY);
        let tag = cipher.tag(b"ada", b"hello");

        assert_eq!(tag, Cipher::new(&KEY).tag(b"ada", b"hello"));
        assert_ne!(tag, cipher.tag(b"grace", b"hello"));
        assert_ne!(tag, cipher.tag(b"ada", b"hellp"));
        assert_ne!(tag, Cipher::new(&[8; 32]).tag(b"ada", b"hello"));
        // The context's length is hashed too, so bytes can't move between context and data
        assert_ne!(cipher.tag(b"ad", b"ahello"), tag);
    }

    #[test]
    fn refuses_another_context_key_or_version() {
        let cipher = Cipher::new(&KEY);
        let sealed = cipher.encrypt(b"secret", b"ada").unwrap();

        assert!(cipher.decrypt(&sealed, b"grace").is_err());
        assert!(Cipher::new(&[8; 32]).decrypt(&sealed, b"ada").is_err());

        let mut future = sealed;
        future[0] = 2;
        assert!(cipher.decrypt(&future, b"ada").is_err());
    }
}
