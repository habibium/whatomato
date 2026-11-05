use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use whatsapp_rust::wacore::{
    appstate::{hash::HashState, processor::AppStateMutationMAC},
    store::{
        error::{Result, StoreError},
        traits::{AppStateSyncKey, AppSyncStore},
    },
};

use super::{PgStore, db, decode, encode};

const SYNC_KEYS: &str = "app_state_keys";
const VERSIONS: &str = "app_state_versions";

#[async_trait]
impl AppSyncStore for PgStore {
    async fn get_sync_key(&self, key_id: &[u8]) -> Result<Option<AppStateSyncKey>> {
        // Behind the write lock: a key reported missing while its write is queued would leave the
        // phone's key share unanswered
        let _order = self.writes.lock().await;
        let sealed = sqlx::query_scalar!(
            "SELECT key FROM whatsapp_app_state_keys WHERE account_id = $1 AND key_id = $2",
            self.account_id,
            key_id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;

        sealed
            .map(|sealed| decode(&self.open(SYNC_KEYS, key_id, &sealed)?))
            .transpose()
    }

    async fn set_sync_key(&self, key_id: &[u8], key: AppStateSyncKey) -> Result<()> {
        let sealed = self.seal(SYNC_KEYS, key_id, &encode(&key)?)?;
        let _order = self.writes.lock().await;
        sqlx::query!(
            "INSERT INTO whatsapp_app_state_keys (account_id, key_id, key) VALUES ($1, $2, $3) \
             ON CONFLICT (account_id, key_id) DO UPDATE SET key = EXCLUDED.key, stored = EXCLUDED.stored",
            self.account_id,
            key_id,
            sealed,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn get_version(&self, name: &str) -> Result<Option<HashState>> {
        let sealed = sqlx::query_scalar!(
            "SELECT state FROM whatsapp_app_state_versions WHERE account_id = $1 AND name = $2",
            self.account_id,
            name,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;

        sealed
            .map(|sealed| decode(&self.open(VERSIONS, name.as_bytes(), &sealed)?))
            .transpose()
    }

    async fn delete_version(&self, name: &str) -> Result<()> {
        let _order = self.writes.lock().await;
        sqlx::query!(
            "DELETE FROM whatsapp_app_state_versions WHERE account_id = $1 AND name = $2",
            self.account_id,
            name,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn set_version(&self, name: &str, state: HashState) -> Result<()> {
        let sealed = self.seal(VERSIONS, name.as_bytes(), &encode(&state)?)?;
        let _order = self.writes.lock().await;
        sqlx::query!(
            "INSERT INTO whatsapp_app_state_versions (account_id, name, state) VALUES ($1, $2, $3) \
             ON CONFLICT (account_id, name) DO UPDATE SET state = EXCLUDED.state",
            self.account_id,
            name,
            sealed,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn put_mutation_macs(
        &self,
        name: &str,
        version: u64,
        mutations: &[AppStateMutationMAC],
    ) -> Result<()> {
        if mutations.is_empty() {
            return Ok(());
        }
        let version = to_i64(version)?;
        let (index_macs, value_macs) = last_per_index(mutations);
        let _order = self.writes.lock().await;
        sqlx::query!(
            "INSERT INTO whatsapp_app_state_mutation_macs (account_id, name, index_mac, value_mac, version) \
             SELECT $1, $2, index_mac, value_mac, $5 FROM UNNEST($3::BYTEA[], $4::BYTEA[]) AS m (index_mac, value_mac) \
             ON CONFLICT (account_id, name, index_mac) DO UPDATE SET value_mac = EXCLUDED.value_mac, version = EXCLUDED.version",
            self.account_id,
            name,
            &index_macs,
            &value_macs,
            version,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn get_mutation_mac(&self, name: &str, index_mac: &[u8]) -> Result<Option<Vec<u8>>> {
        sqlx::query_scalar!(
            "SELECT value_mac FROM whatsapp_app_state_mutation_macs \
             WHERE account_id = $1 AND name = $2 AND index_mac = $3",
            self.account_id,
            name,
            index_mac,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db)
    }

    async fn get_mutation_macs(
        &self,
        name: &str,
        index_macs: &[[u8; 32]],
    ) -> Result<HashMap<[u8; 32], Vec<u8>>> {
        if index_macs.is_empty() {
            return Ok(HashMap::new());
        }
        let index_macs: Vec<Vec<u8>> = index_macs.iter().map(|mac| mac.to_vec()).collect();
        let rows = sqlx::query!(
            "SELECT index_mac, value_mac FROM whatsapp_app_state_mutation_macs \
             WHERE account_id = $1 AND name = $2 AND index_mac = ANY($3)",
            self.account_id,
            name,
            &index_macs,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;

        Ok(rows
            .into_iter()
            .filter_map(|row| Some((row.index_mac.try_into().ok()?, row.value_mac)))
            .collect())
    }

    async fn delete_mutation_macs(&self, name: &str, index_macs: &[Vec<u8>]) -> Result<()> {
        if index_macs.is_empty() {
            return Ok(());
        }
        let _order = self.writes.lock().await;
        sqlx::query!(
            "DELETE FROM whatsapp_app_state_mutation_macs WHERE account_id = $1 AND name = $2 AND index_mac = ANY($3)",
            self.account_id,
            name,
            index_macs,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn commit_patch(
        &self,
        name: &str,
        state: HashState,
        removed_index_macs: &[Vec<u8>],
        added: &[AppStateMutationMAC],
    ) -> Result<()> {
        let version = to_i64(state.version)?;
        let sealed = self.seal(VERSIONS, name.as_bytes(), &encode(&state)?)?;
        let (index_macs, value_macs) = last_per_index(added);
        let _order = self.writes.lock().await;
        let mut tx = self.pool.begin().await.map_err(db)?;
        sqlx::query!(
            "INSERT INTO whatsapp_app_state_versions (account_id, name, state) VALUES ($1, $2, $3) \
             ON CONFLICT (account_id, name) DO UPDATE SET state = EXCLUDED.state",
            self.account_id,
            name,
            sealed,
        )
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        // Removals first, so an index the patch both removes and adds keeps the added value
        sqlx::query!(
            "DELETE FROM whatsapp_app_state_mutation_macs WHERE account_id = $1 AND name = $2 AND index_mac = ANY($3)",
            self.account_id,
            name,
            removed_index_macs,
        )
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        sqlx::query!(
            "INSERT INTO whatsapp_app_state_mutation_macs (account_id, name, index_mac, value_mac, version) \
             SELECT $1, $2, index_mac, value_mac, $5 FROM UNNEST($3::BYTEA[], $4::BYTEA[]) AS m (index_mac, value_mac) \
             ON CONFLICT (account_id, name, index_mac) DO UPDATE SET value_mac = EXCLUDED.value_mac, version = EXCLUDED.version",
            self.account_id,
            name,
            &index_macs,
            &value_macs,
            version,
        )
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(db)?;

        Ok(())
    }

    async fn clear_mutation_macs(&self, name: &str) -> Result<()> {
        let _order = self.writes.lock().await;
        sqlx::query!(
            "DELETE FROM whatsapp_app_state_mutation_macs WHERE account_id = $1 AND name = $2",
            self.account_id,
            name,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn get_latest_sync_key_id(&self) -> Result<Option<Vec<u8>>> {
        let _order = self.writes.lock().await;
        sqlx::query_scalar!(
            "SELECT key_id FROM whatsapp_app_state_keys WHERE account_id = $1 ORDER BY stored DESC LIMIT 1",
            self.account_id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db)
    }
}

fn to_i64(version: u64) -> Result<i64> {
    i64::try_from(version)
        .map_err(|_| StoreError::Validation(format!("version {version} is out of range")))
}

/// Postgres refuses an upsert that touches one row twice, so a repeated index keeps only its last
/// value, the one applying the batch in order would leave
fn last_per_index(mutations: &[AppStateMutationMAC]) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let mut seen = HashSet::new();
    mutations
        .iter()
        .rev()
        .filter(|mutation| seen.insert(mutation.index_mac.as_slice()))
        .map(|mutation| (mutation.index_mac.clone(), mutation.value_mac.clone()))
        .unzip()
}
