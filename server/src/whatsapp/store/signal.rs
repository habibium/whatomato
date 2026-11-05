use std::{collections::HashSet, sync::Arc};

use async_trait::async_trait;
use bytes::Bytes;
use whatsapp_rust::wacore::store::{
    error::{Result, StoreError},
    traits::SignalStore,
};

use super::{PgStore, db};

const IDENTITIES: &str = "identities";
const SESSIONS: &str = "sessions";
const PREKEYS: &str = "prekeys";
const SIGNED_PREKEYS: &str = "signed_prekeys";
const SENDER_KEYS: &str = "sender_keys";

#[async_trait]
impl SignalStore for PgStore {
    async fn put_identity(&self, address: &str, key: [u8; 32]) -> Result<()> {
        self.put_identities_batch(&[(Arc::from(address), key)])
            .await
    }

    async fn put_identities_batch(&self, identities: &[(Arc<str>, [u8; 32])]) -> Result<()> {
        if identities.is_empty() {
            return Ok(());
        }
        let mut seen = HashSet::new();
        let mut addresses = Vec::with_capacity(identities.len());
        let mut sealed = Vec::with_capacity(identities.len());
        for (address, key) in identities
            .iter()
            .rev()
            .filter(|(address, _)| seen.insert(&**address))
        {
            sealed.push(self.seal(IDENTITIES, address.as_bytes(), key)?);
            addresses.push(&**address);
        }
        let _order = self.writes.lock().await;
        sqlx::query!(
            "INSERT INTO whatsapp_identities (account_id, address, key) \
             SELECT $1, * FROM UNNEST($2::TEXT[], $3::BYTEA[]) \
             ON CONFLICT (account_id, address) DO UPDATE SET key = EXCLUDED.key",
            self.account_id,
            &addresses as &[&str],
            &sealed,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn load_identity(&self, address: &str) -> Result<Option<[u8; 32]>> {
        let sealed = sqlx::query_scalar!(
            "SELECT key FROM whatsapp_identities WHERE account_id = $1 AND address = $2",
            self.account_id,
            address,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;

        sealed
            .map(|sealed| {
                let opened = self.open(IDENTITIES, address.as_bytes(), &sealed)?;
                let key: [u8; 32] = opened.as_slice().try_into().map_err(|_| {
                    StoreError::Validation(format!(
                        "a stored identity key is {} bytes, not 32",
                        opened.len()
                    ))
                })?;
                Ok(key)
            })
            .transpose()
    }

    async fn delete_identity(&self, address: &str) -> Result<()> {
        self.delete_identities_batch(&[Arc::from(address)]).await
    }

    async fn delete_identities_batch(&self, addresses: &[Arc<str>]) -> Result<()> {
        if addresses.is_empty() {
            return Ok(());
        }
        let addresses: Vec<&str> = addresses.iter().map(|address| &**address).collect();
        let _order = self.writes.lock().await;
        sqlx::query!(
            "DELETE FROM whatsapp_identities WHERE account_id = $1 AND address = ANY($2)",
            self.account_id,
            &addresses as &[&str],
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn get_session(&self, address: &str) -> Result<Option<Bytes>> {
        Ok(self
            .get_sessions_batch(&[Arc::from(address)])
            .await?
            .pop()
            .map(|(_, session)| session))
    }

    async fn put_session(&self, address: &str, session: &[u8]) -> Result<()> {
        self.put_sessions_batch(&[(Arc::from(address), Bytes::copy_from_slice(session))])
            .await
    }

    async fn put_sessions_batch(&self, sessions: &[(Arc<str>, Bytes)]) -> Result<()> {
        if sessions.is_empty() {
            return Ok(());
        }
        let mut seen = HashSet::new();
        let mut addresses = Vec::with_capacity(sessions.len());
        let mut sealed = Vec::with_capacity(sessions.len());
        for (address, session) in sessions
            .iter()
            .rev()
            .filter(|(address, _)| seen.insert(&**address))
        {
            sealed.push(self.seal(SESSIONS, address.as_bytes(), session)?);
            addresses.push(&**address);
        }
        let _order = self.writes.lock().await;
        sqlx::query!(
            "INSERT INTO whatsapp_sessions (account_id, address, record) \
             SELECT $1, * FROM UNNEST($2::TEXT[], $3::BYTEA[]) \
             ON CONFLICT (account_id, address) DO UPDATE SET record = EXCLUDED.record",
            self.account_id,
            &addresses as &[&str],
            &sealed,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn get_sessions_batch(&self, addresses: &[Arc<str>]) -> Result<Vec<(Arc<str>, Bytes)>> {
        if addresses.is_empty() {
            return Ok(Vec::new());
        }
        let addresses: Vec<&str> = addresses.iter().map(|address| &**address).collect();
        let rows = sqlx::query!(
            "SELECT address, record FROM whatsapp_sessions WHERE account_id = $1 AND address = ANY($2)",
            self.account_id,
            &addresses as &[&str],
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;

        rows.into_iter()
            .map(|row| {
                let mut session = self.open(SESSIONS, row.address.as_bytes(), &row.record)?;
                Ok((
                    Arc::from(row.address),
                    Bytes::from(std::mem::take(&mut *session)),
                ))
            })
            .collect()
    }

    async fn delete_session(&self, address: &str) -> Result<()> {
        self.delete_sessions_batch(&[Arc::from(address)]).await
    }

    async fn delete_sessions_batch(&self, addresses: &[Arc<str>]) -> Result<()> {
        if addresses.is_empty() {
            return Ok(());
        }
        let addresses: Vec<&str> = addresses.iter().map(|address| &**address).collect();
        let _order = self.writes.lock().await;
        sqlx::query!(
            "DELETE FROM whatsapp_sessions WHERE account_id = $1 AND address = ANY($2)",
            self.account_id,
            &addresses as &[&str],
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn has_session(&self, address: &str) -> Result<bool> {
        sqlx::query_scalar!(
            r#"SELECT EXISTS (SELECT 1 FROM whatsapp_sessions WHERE account_id = $1 AND address = $2) AS "exists!""#,
            self.account_id,
            address,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(db)
    }

    async fn has_signal_state_for_user(&self, user: &str) -> Result<bool> {
        // Behind the write lock: a stale "none" would skip a PN-to-LID migration nothing retries
        let _order = self.writes.lock().await;
        // Device 0 is keyed `user@server` and the others `user:device@server`
        sqlx::query_scalar!(
            "SELECT EXISTS (SELECT 1 FROM whatsapp_sessions WHERE account_id = $1 \
             AND (starts_with(address, $2) OR starts_with(address, $3))) \
             OR EXISTS (SELECT 1 FROM whatsapp_identities WHERE account_id = $1 \
             AND (starts_with(address, $2) OR starts_with(address, $3))) AS \"exists!\"",
            self.account_id,
            format!("{user}@"),
            format!("{user}:"),
        )
        .fetch_one(&self.pool)
        .await
        .map_err(db)
    }

    async fn store_prekey(&self, id: u32, record: &[u8], uploaded: bool) -> Result<()> {
        self.store_prekeys_batch(&[(id, Bytes::copy_from_slice(record))], uploaded)
            .await
    }

    async fn store_prekeys_batch(&self, keys: &[(u32, Bytes)], uploaded: bool) -> Result<()> {
        if keys.is_empty() {
            return Ok(());
        }
        let mut seen = HashSet::new();
        let mut ids = Vec::with_capacity(keys.len());
        let mut sealed = Vec::with_capacity(keys.len());
        // Postgres refuses an upsert that touches one row twice; the last entry for an id wins
        for (id, record) in keys.iter().rev().filter(|(id, _)| seen.insert(*id)) {
            sealed.push(self.seal(PREKEYS, &id.to_be_bytes(), record)?);
            ids.push(to_i32(*id)?);
        }
        let _order = self.writes.lock().await;
        sqlx::query!(
            "INSERT INTO whatsapp_prekeys (account_id, id, record, uploaded) \
             SELECT $1, id, record, $4 FROM UNNEST($2::INTEGER[], $3::BYTEA[]) AS k (id, record) \
             ON CONFLICT (account_id, id) DO UPDATE SET record = EXCLUDED.record, uploaded = EXCLUDED.uploaded",
            self.account_id,
            &ids,
            &sealed,
            uploaded,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn load_prekey(&self, id: u32) -> Result<Option<Bytes>> {
        Ok(self
            .load_prekeys_batch(&[id])
            .await?
            .pop()
            .map(|(_, record)| record))
    }

    async fn load_prekeys_batch(&self, ids: &[u32]) -> Result<Vec<(u32, Bytes)>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids = ids
            .iter()
            .map(|&id| to_i32(id))
            .collect::<Result<Vec<_>>>()?;
        let rows = sqlx::query!(
            "SELECT id, record FROM whatsapp_prekeys WHERE account_id = $1 AND id = ANY($2)",
            self.account_id,
            &ids,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;

        rows.into_iter()
            .map(|row| {
                let id = to_u32(row.id)?;
                let mut record = self.open(PREKEYS, &id.to_be_bytes(), &row.record)?;
                Ok((id, Bytes::from(std::mem::take(&mut *record))))
            })
            .collect()
    }

    async fn mark_prekeys_uploaded(&self, ids: &[u32]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let ids = ids
            .iter()
            .map(|&id| to_i32(id))
            .collect::<Result<Vec<_>>>()?;
        let _order = self.writes.lock().await;
        // An UPDATE, never an upsert: a prekey consumed since the upload must stay deleted
        sqlx::query!(
            "UPDATE whatsapp_prekeys SET uploaded = true WHERE account_id = $1 AND id = ANY($2)",
            self.account_id,
            &ids,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn remove_prekey(&self, id: u32) -> Result<()> {
        self.remove_prekeys_batch(&[id]).await
    }

    async fn remove_prekeys_batch(&self, ids: &[u32]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let ids = ids
            .iter()
            .map(|&id| to_i32(id))
            .collect::<Result<Vec<_>>>()?;
        let _order = self.writes.lock().await;
        sqlx::query!(
            "DELETE FROM whatsapp_prekeys WHERE account_id = $1 AND id = ANY($2)",
            self.account_id,
            &ids,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn get_max_prekey_id(&self) -> Result<u32> {
        let max = sqlx::query_scalar!(
            r#"SELECT COALESCE(MAX(id), 0) AS "max!" FROM whatsapp_prekeys WHERE account_id = $1"#,
            self.account_id,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(db)?;

        to_u32(max)
    }

    async fn store_signed_prekey(&self, id: u32, record: &[u8]) -> Result<()> {
        let sealed = self.seal(SIGNED_PREKEYS, &id.to_be_bytes(), record)?;
        let _order = self.writes.lock().await;
        sqlx::query!(
            "INSERT INTO whatsapp_signed_prekeys (account_id, id, record) VALUES ($1, $2, $3) \
             ON CONFLICT (account_id, id) DO UPDATE SET record = EXCLUDED.record",
            self.account_id,
            to_i32(id)?,
            sealed,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn load_signed_prekey(&self, id: u32) -> Result<Option<Vec<u8>>> {
        let sealed = sqlx::query_scalar!(
            "SELECT record FROM whatsapp_signed_prekeys WHERE account_id = $1 AND id = $2",
            self.account_id,
            to_i32(id)?,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;

        sealed
            .map(|sealed| {
                let mut record = self.open(SIGNED_PREKEYS, &id.to_be_bytes(), &sealed)?;
                Ok(std::mem::take(&mut *record))
            })
            .transpose()
    }

    async fn load_all_signed_prekeys(&self) -> Result<Vec<(u32, Vec<u8>)>> {
        let rows = sqlx::query!(
            "SELECT id, record FROM whatsapp_signed_prekeys WHERE account_id = $1 ORDER BY id",
            self.account_id,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;

        rows.into_iter()
            .map(|row| {
                let id = to_u32(row.id)?;
                let mut record = self.open(SIGNED_PREKEYS, &id.to_be_bytes(), &row.record)?;
                Ok((id, std::mem::take(&mut *record)))
            })
            .collect()
    }

    async fn remove_signed_prekey(&self, id: u32) -> Result<()> {
        let _order = self.writes.lock().await;
        sqlx::query!(
            "DELETE FROM whatsapp_signed_prekeys WHERE account_id = $1 AND id = $2",
            self.account_id,
            to_i32(id)?,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn put_sender_key(&self, address: &str, record: &[u8]) -> Result<()> {
        self.put_sender_keys_batch(&[(Arc::from(address), Bytes::copy_from_slice(record))])
            .await
    }

    async fn put_sender_keys_batch(&self, sender_keys: &[(Arc<str>, Bytes)]) -> Result<()> {
        if sender_keys.is_empty() {
            return Ok(());
        }
        let mut seen = HashSet::new();
        let mut addresses = Vec::with_capacity(sender_keys.len());
        let mut sealed = Vec::with_capacity(sender_keys.len());
        for (address, record) in sender_keys
            .iter()
            .rev()
            .filter(|(address, _)| seen.insert(&**address))
        {
            sealed.push(self.seal(SENDER_KEYS, address.as_bytes(), record)?);
            addresses.push(&**address);
        }
        let _order = self.writes.lock().await;
        sqlx::query!(
            "INSERT INTO whatsapp_sender_keys (account_id, address, record) \
             SELECT $1, * FROM UNNEST($2::TEXT[], $3::BYTEA[]) \
             ON CONFLICT (account_id, address) DO UPDATE SET record = EXCLUDED.record",
            self.account_id,
            &addresses as &[&str],
            &sealed,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn get_sender_key(&self, address: &str) -> Result<Option<Vec<u8>>> {
        let sealed = sqlx::query_scalar!(
            "SELECT record FROM whatsapp_sender_keys WHERE account_id = $1 AND address = $2",
            self.account_id,
            address,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;

        sealed
            .map(|sealed| {
                let mut record = self.open(SENDER_KEYS, address.as_bytes(), &sealed)?;
                Ok(std::mem::take(&mut *record))
            })
            .transpose()
    }

    async fn delete_sender_key(&self, address: &str) -> Result<()> {
        self.delete_sender_keys_batch(&[Arc::from(address)]).await
    }

    async fn delete_sender_keys_batch(&self, addresses: &[Arc<str>]) -> Result<()> {
        if addresses.is_empty() {
            return Ok(());
        }
        let addresses: Vec<&str> = addresses.iter().map(|address| &**address).collect();
        let _order = self.writes.lock().await;
        sqlx::query!(
            "DELETE FROM whatsapp_sender_keys WHERE account_id = $1 AND address = ANY($2)",
            self.account_id,
            &addresses as &[&str],
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }
}

/// Prekey ids are 24 bits, so they fit an INTEGER column
fn to_i32(id: u32) -> Result<i32> {
    i32::try_from(id).map_err(|_| StoreError::Validation(format!("prekey id {id} is out of range")))
}

fn to_u32(id: i32) -> Result<u32> {
    u32::try_from(id)
        .map_err(|_| StoreError::Validation(format!("stored prekey id {id} is negative")))
}
