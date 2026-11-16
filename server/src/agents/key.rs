//! Access keys: what an agent sends as `Authorization: Bearer`. Only the hash is stored; the key is
//! shown once, on creation and on rotation.

use crate::auth::token::{self, Token};

pub const PREFIX: &str = "whatomato_ak_";
/// 32 bytes in base64 without padding
const SECRET_LEN: usize = 43;
const HINT_LEN: usize = 4;

/// What a client that signed in holds. Each kind has its own prefix, so one never passes for
/// another and secret scanners can tell them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Issued {
    /// The authorization code, exchanged once for the two tokens
    Code,
    AccessToken,
    RefreshToken,
}

impl Issued {
    #[must_use]
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Code => "whatomato_ac_",
            Self::AccessToken => "whatomato_at_",
            Self::RefreshToken => "whatomato_rt_",
        }
    }

    /// The value, shown to the client once, and the hash to store
    ///
    /// # Errors
    /// When the system's random source fails.
    pub fn generate(self) -> anyhow::Result<(String, [u8; 32])> {
        let value = format!("{}{}", self.prefix(), Token::generate()?.value);
        let hash = hash(&value);
        Ok((value, hash))
    }

    /// Whether `value` has this kind's shape, to turn away anything else before hashing it
    #[must_use]
    pub fn is(self, value: &str) -> bool {
        has_shape(value, self.prefix())
    }
}

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
    has_shape(value, PREFIX)
}

fn has_shape(value: &str, prefix: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(|secret| {
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
    fn each_issued_kind_passes_only_for_itself() {
        let kinds = [Issued::Code, Issued::AccessToken, Issued::RefreshToken];
        for kind in kinds {
            let (value, stored) = kind.generate().unwrap();
            assert_eq!(stored, hash(&value));
            assert!(!is_access_key(&value));
            for other in kinds {
                assert_eq!(other.is(&value), other == kind, "{kind:?} as {other:?}");
            }
        }
        assert!(!Issued::AccessToken.is(&AccessKey::generate().unwrap().value));
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
