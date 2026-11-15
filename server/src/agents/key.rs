//! Access keys: what an agent sends as `Authorization: Bearer`. Only the hash is stored; the key is
//! shown once, on creation and on rotation.

use crate::auth::token::{self, Token};

pub const PREFIX: &str = "whatomato_ak_";
/// 32 bytes in base64 without padding
const SECRET_LEN: usize = 43;
const HINT_LEN: usize = 4;

pub struct AccessKey {
    /// Shown once, never logged
    pub value: String,
    pub hash: [u8; 32],
    /// The last characters, so the owner can tell keys apart
    pub hint: String,
}

impl AccessKey {
    /// # Errors
    /// When the system's random source fails.
    pub fn generate() -> anyhow::Result<Self> {
        let value = format!("{PREFIX}{}", Token::generate()?.value);
        Ok(Self {
            hash: hash(&value),
            hint: hint(&value),
            value,
        })
    }
}

#[must_use]
pub fn hash(key: &str) -> [u8; 32] {
    token::hash(key)
}

#[must_use]
pub fn hint(key: &str) -> String {
    let start = key.len().saturating_sub(HINT_LEN);
    key.get(start..).unwrap_or_default().to_owned()
}

/// Whether `value` has the shape of an access key, to turn away anything else before hashing it
#[must_use]
pub fn is_access_key(value: &str) -> bool {
    value.strip_prefix(PREFIX).is_some_and(|secret| {
        secret.len() == SECRET_LEN
            && secret
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_has_the_prefix_and_its_hint_and_hash() {
        let key = AccessKey::generate().unwrap();
        assert!(key.value.starts_with(PREFIX));
        assert_eq!(key.value.len(), PREFIX.len() + SECRET_LEN);
        assert!(is_access_key(&key.value));
        assert_eq!(key.hint, key.value[key.value.len() - 4..]);
        assert_eq!(key.hash, hash(&key.value));
        assert_ne!(key.value, AccessKey::generate().unwrap().value);
    }

    #[test]
    fn only_the_shape_of_a_key_is_recognised() {
        let secret = "a".repeat(SECRET_LEN);
        assert!(is_access_key(&format!("{PREFIX}{secret}")));
        for other in [
            secret.clone(),
            format!("whatomato_xx_{secret}"),
            format!("{PREFIX}{}", &secret[1..]),
            format!("{PREFIX}{secret}a"),
            format!("{PREFIX}{}+", &secret[1..]),
            String::new(),
        ] {
            assert!(!is_access_key(&other), "{other}");
        }
    }
}
