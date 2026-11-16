//! What goes to a browser's push service: the notification encrypted for that browser alone
//! (RFC 8291, over RFC 8188's `aes128gcm`), and the signed header that says who sends it
//! (RFC 8292, "VAPID").

use anyhow::{Context as _, anyhow, ensure};
use base64::prelude::{BASE64_URL_SAFE_NO_PAD, Engine as _};
use chrono::{TimeDelta, Utc};
use ring::{
    aead::{AES_128_GCM, Aad, LessSafeKey, Nonce, UnboundKey},
    agreement::{self, ECDH_P256, EphemeralPrivateKey, UnparsedPublicKey},
    hkdf::{self, HKDF_SHA256, KeyType, Salt},
    rand::{SecureRandom, SystemRandom},
    signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair},
};
use serde_json::json;
use url::Url;

/// An uncompressed P-256 point
pub const PUBLIC_KEY_LEN: usize = 65;
pub const AUTH_LEN: usize = 16;
const SALT_LEN: usize = 16;
/// One record holds the whole message; push services take 4,096 bytes at most
const RECORD_SIZE: u32 = 4096;
/// The header, the padding delimiter and the tag leave this much of a record for the text
pub const PLAINTEXT_MAX: usize = 3800;
const TOKEN_TTL: TimeDelta = TimeDelta::hours(12);

/// Encrypts `plaintext` for the browser that holds the private half of `ua_public` and `auth`
///
/// # Errors
/// The keys aren't a browser's, the message is too long, or the system's random source failed.
pub fn encrypt(ua_public: &[u8], auth: &[u8], plaintext: &[u8]) -> anyhow::Result<Vec<u8>> {
    let random = SystemRandom::new();
    let private = EphemeralPrivateKey::generate(&ECDH_P256, &random)
        .map_err(|_| anyhow!("couldn't make a key for a push message"))?;
    let as_public = private
        .compute_public_key()
        .map_err(|_| anyhow!("couldn't make a key for a push message"))?;
    let mut salt = [0; SALT_LEN];
    random
        .fill(&mut salt)
        .map_err(|_| anyhow!("the system's random source failed"))?;
    let peer = UnparsedPublicKey::new(&ECDH_P256, ua_public);
    agreement::agree_ephemeral(private, &peer, |shared| {
        seal(
            shared,
            auth,
            ua_public,
            as_public.as_ref(),
            &salt,
            plaintext,
        )
    })
    .map_err(|_| anyhow!("the browser's key isn't a P-256 point"))?
}

struct Len(usize);

impl KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

fn expand<const N: usize>(prk: &hkdf::Prk, info: &[&[u8]]) -> anyhow::Result<[u8; N]> {
    let mut out = [0; N];
    prk.expand(info, Len(N))
        .and_then(|okm| okm.fill(&mut out))
        .map_err(|_| anyhow!("key derivation failed"))?;
    Ok(out)
}

/// RFC 8291 sections 3.3 and 3.4, given the ECDH secret: the body of the request
fn seal(
    shared: &[u8],
    auth: &[u8],
    ua_public: &[u8],
    as_public: &[u8],
    salt: &[u8; SALT_LEN],
    plaintext: &[u8],
) -> anyhow::Result<Vec<u8>> {
    ensure!(
        ua_public.len() == PUBLIC_KEY_LEN && as_public.len() == PUBLIC_KEY_LEN,
        "a public key isn't an uncompressed P-256 point"
    );
    ensure!(auth.len() == AUTH_LEN, "the auth secret isn't 16 bytes");
    ensure!(
        plaintext.len() <= PLAINTEXT_MAX,
        "the message is too long to push"
    );

    let combined = Salt::new(HKDF_SHA256, auth).extract(shared);
    let ikm: [u8; 32] = expand(&combined, &[b"WebPush: info\0", ua_public, as_public])?;
    let prk = Salt::new(HKDF_SHA256, salt).extract(&ikm);
    let key: [u8; 16] = expand(&prk, &[b"Content-Encoding: aes128gcm\0"])?;
    let nonce: [u8; 12] = expand(&prk, &[b"Content-Encoding: nonce\0"])?;

    // The last (here, only) record ends with the delimiter 2
    let mut record = [plaintext, &[2]].concat();
    let key = UnboundKey::new(&AES_128_GCM, &key).map_err(|_| anyhow!("encryption failed"))?;
    LessSafeKey::new(key)
        .seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::empty(),
            &mut record,
        )
        .map_err(|_| anyhow!("encryption failed"))?;

    let key_id_len = u8::try_from(as_public.len()).context("the key id is too long")?;
    Ok([
        salt.as_slice(),
        &RECORD_SIZE.to_be_bytes(),
        &[key_id_len],
        as_public,
        &record,
    ]
    .concat())
}

/// The key pair that identifies this server to push services. A browser's subscription is tied
/// to the public half, so replacing the pair ends every subscription.
pub struct Vapid {
    key_pair: EcdsaKeyPair,
    /// The public key as browsers and the `Authorization` header take it
    public_key: String,
}

impl Vapid {
    /// A new pair, as the PKCS#8 document to keep
    ///
    /// # Errors
    /// The system's random source failed.
    pub fn generate() -> anyhow::Result<Vec<u8>> {
        let document =
            EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &SystemRandom::new())
                .map_err(|_| anyhow!("couldn't generate the push key pair"))?;
        Ok(document.as_ref().to_vec())
    }

    /// # Errors
    /// `pkcs8` isn't a P-256 key pair.
    pub fn from_pkcs8(pkcs8: &[u8]) -> anyhow::Result<Self> {
        let key_pair = EcdsaKeyPair::from_pkcs8(
            &ECDSA_P256_SHA256_FIXED_SIGNING,
            pkcs8,
            &SystemRandom::new(),
        )
        .map_err(|error| anyhow!("the stored push key pair is unusable: {error}"))?;
        let public_key = BASE64_URL_SAFE_NO_PAD.encode(key_pair.public_key());
        Ok(Self {
            key_pair,
            public_key,
        })
    }

    #[must_use]
    pub fn public_key(&self) -> &str {
        &self.public_key
    }

    /// The `Authorization` header for a request to `endpoint`: a token good for 12 hours at that
    /// push service only, naming `contact` as who to reach about this server
    ///
    /// # Errors
    /// `endpoint` has no origin, or signing failed.
    pub fn authorization(&self, endpoint: &Url, contact: &str) -> anyhow::Result<String> {
        let audience = endpoint.origin().ascii_serialization();
        ensure!(audience != "null", "the push endpoint has no origin");
        let encode = |value: &serde_json::Value| BASE64_URL_SAFE_NO_PAD.encode(value.to_string());
        let header = encode(&json!({ "typ": "JWT", "alg": "ES256" }));
        let claims = encode(&json!({
            "aud": audience,
            "exp": (Utc::now() + TOKEN_TTL).timestamp(),
            "sub": contact,
        }));
        let signed = format!("{header}.{claims}");
        let signature = self
            .key_pair
            .sign(&SystemRandom::new(), signed.as_bytes())
            .map_err(|_| anyhow!("couldn't sign a push request"))?;
        let signature = BASE64_URL_SAFE_NO_PAD.encode(signature);
        Ok(format!(
            "vapid t={signed}.{signature}, k={}",
            self.public_key
        ))
    }
}

#[cfg(test)]
mod tests {
    use ring::signature::{ECDSA_P256_SHA256_FIXED, UnparsedPublicKey as Verifier};

    use super::*;

    fn decode(value: &str) -> Vec<u8> {
        BASE64_URL_SAFE_NO_PAD.decode(value).unwrap()
    }

    // RFC 8291, appendix A
    const UA_PUBLIC: &str =
        "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
    const AS_PUBLIC: &str =
        "BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8";
    const AUTH: &str = "BTBZMqHH6r4Tts7J_aSIgg";
    const SALT: &str = "DGv6ra1nlYgDCS1FRnbzlw";
    const ECDH_SECRET: &str = "kyrL1jIIOHEzg3sM2ZWRHDRB62YACZhhSlknJ672kSs";
    const PLAINTEXT: &str = "When I grow up, I want to be a watermelon";
    const HEADER: &str = "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8";
    const CIPHERTEXT: &str =
        "8pfeW0KbunFT06SuDKoJH9Ql87S1QUrdirN6GcG7sFz1y1sqLgVi1VhjVkHsUoEsbI_0LpXMuGvnzQ";

    #[test]
    fn a_message_is_sealed_as_the_rfc_works_it_out() {
        let salt: [u8; SALT_LEN] = decode(SALT).try_into().unwrap();
        let body = seal(
            &decode(ECDH_SECRET),
            &decode(AUTH),
            &decode(UA_PUBLIC),
            &decode(AS_PUBLIC),
            &salt,
            PLAINTEXT.as_bytes(),
        )
        .unwrap();
        let expected = [decode(HEADER), decode(CIPHERTEXT)].concat();
        assert_eq!(
            BASE64_URL_SAFE_NO_PAD.encode(&body),
            BASE64_URL_SAFE_NO_PAD.encode(expected)
        );
    }

    #[test]
    fn each_message_gets_its_own_key_and_salt() {
        let (ua_public, auth) = (decode(UA_PUBLIC), decode(AUTH));
        let first = encrypt(&ua_public, &auth, b"hello").unwrap();
        let second = encrypt(&ua_public, &auth, b"hello").unwrap();
        assert_ne!(first, second);
        // salt, record size, key id length, key id, then the text, its delimiter and the tag
        assert_eq!(first.len(), 16 + 4 + 1 + 65 + 5 + 1 + 16);
        assert_eq!(first[16..21], [0, 0, 16, 0, 65]);

        assert!(encrypt(&ua_public[..64], &auth, b"hello").is_err());
        assert!(encrypt(&[4; 65], &auth, b"hello").is_err());
        assert!(encrypt(&ua_public, &auth[..15], b"hello").is_err());
        assert!(encrypt(&ua_public, &auth, &[b'a'; PLAINTEXT_MAX + 1]).is_err());
    }

    #[test]
    fn the_authorization_is_a_token_the_public_key_verifies() {
        let vapid = Vapid::from_pkcs8(&Vapid::generate().unwrap()).unwrap();
        let endpoint = Url::parse("https://fcm.googleapis.com/fcm/send/abc").unwrap();
        let header = vapid
            .authorization(&endpoint, "https://whatomato.example")
            .unwrap();

        let (token, key) = header
            .strip_prefix("vapid t=")
            .and_then(|rest| rest.split_once(", k="))
            .unwrap();
        assert_eq!(key, vapid.public_key());
        assert_eq!(decode(key).len(), PUBLIC_KEY_LEN);
        let parts: Vec<_> = token.split('.').collect();
        assert_eq!(parts.len(), 3);
        let header: serde_json::Value = serde_json::from_slice(&decode(parts[0])).unwrap();
        assert_eq!(header, json!({ "typ": "JWT", "alg": "ES256" }));
        let claims: serde_json::Value = serde_json::from_slice(&decode(parts[1])).unwrap();
        assert_eq!(claims["aud"], "https://fcm.googleapis.com");
        assert_eq!(claims["sub"], "https://whatomato.example");
        let left = claims["exp"].as_i64().unwrap() - Utc::now().timestamp();
        assert!((11 * 3600..=12 * 3600).contains(&left), "{left}");

        let signed = format!("{}.{}", parts[0], parts[1]);
        Verifier::new(&ECDSA_P256_SHA256_FIXED, decode(key))
            .verify(signed.as_bytes(), &decode(parts[2]))
            .unwrap();
        // Another server's key pair makes a different header
        let other = Vapid::from_pkcs8(&Vapid::generate().unwrap()).unwrap();
        assert_ne!(other.public_key(), vapid.public_key());
    }
}
