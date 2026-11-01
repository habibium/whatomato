//! whatsapp-rust's storage traits over Postgres, one [`PgStore`] per linked account

mod app_sync;
mod device;
mod msg_secret;
mod protocol;
mod signal;
#[cfg(test)]
mod tests;

use serde::{Serialize, de::DeserializeOwned};
use sqlx::PgPool;
use tokio::sync::Mutex;
use uuid::Uuid;
use whatsapp_rust::wacore::store::error::StoreError;
use zeroize::Zeroizing;

use crate::crypto::Cipher;

/// One linked account's rows in the `whatsapp_*` tables. Every query filters on `account_id`, so a
/// handle can't reach another account.
pub struct PgStore {
    pool: PgPool,
    cipher: Cipher,
    account_id: Uuid,
    /// Upstream's SQLite store gets its ordering from a single writer connection. This lock gives
    /// the same: writes, and the reads that must not overtake one, run one at a time, in order.
    writes: Mutex<()>,
}

impl PgStore {
    #[must_use]
    pub fn new(pool: PgPool, cipher: Cipher, account_id: Uuid) -> Self {
        Self {
            pool,
            cipher,
            account_id,
            writes: Mutex::new(()),
        }
    }

    fn seal(&self, table: &str, key: &[u8], value: &[u8]) -> Result<Vec<u8>, StoreError> {
        self.cipher
            .encrypt(value, &self.context(table, key))
            .map_err(sealing)
    }

    fn open(
        &self,
        table: &str,
        key: &[u8],
        sealed: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, StoreError> {
        self.cipher
            .decrypt(sealed, &self.context(table, key))
            .map_err(sealing)
    }

    /// Binds a sealed value to its account, table and key, so it can't be moved to another row. The
    /// account id has a fixed length and no table name holds a `:`, so no two rows share a context.
    fn context(&self, table: &str, key: &[u8]) -> Vec<u8> {
        [
            b"whatsapp:".as_slice(),
            self.account_id.as_bytes(),
            table.as_bytes(),
            b":",
            key,
        ]
        .concat()
    }
}

/// `StoreError` and `sqlx::Error` are both foreign types, so the orphan rule rules out a `From` impl
fn db(error: sqlx::Error) -> StoreError {
    StoreError::Database(Box::new(error))
}

/// A value that fails to open is an error, never treated as absent: the core would quietly replace
/// a missing session with a new one
fn sealing(error: anyhow::Error) -> StoreError {
    StoreError::Database(error.into())
}

/// Joins the parts of a composite key with NUL, which no JID or message id contains
fn composite(parts: &[&str]) -> Vec<u8> {
    parts.join("\0").into_bytes()
}

fn count(rows: u64) -> u32 {
    u32::try_from(rows).unwrap_or(u32::MAX)
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, StoreError> {
    serde_json::to_vec(value).map_err(|error| StoreError::Serialization(Box::new(error)))
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, StoreError> {
    serde_json::from_slice(bytes).map_err(|error| StoreError::Serialization(Box::new(error)))
}
