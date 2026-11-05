use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use whatsapp_rust::wacore::{
    store::{
        error::Result,
        traits::{DeviceListRecord, LidPnMappingEntry, ProtocolStore, TcTokenEntry},
    },
    time::now_secs,
};

use super::{PgStore, composite, count, db, decode, encode};

const DEVICE_LISTS: &str = "device_lists";
const TC_TOKENS: &str = "tc_tokens";
const SENT_MESSAGES: &str = "sent_messages";

#[async_trait]
impl ProtocolStore for PgStore {
    async fn get_sender_key_devices(&self, group_jid: &str) -> Result<Vec<(String, bool)>> {
        // Behind the write lock: a stale "has the key" would skip giving a device our sender key
        let _order = self.writes.lock().await;
        let rows = sqlx::query!(
            "SELECT device_jid, has_key FROM whatsapp_sender_key_devices WHERE account_id = $1 AND group_jid = $2",
            self.account_id,
            group_jid,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;

        Ok(rows
            .into_iter()
            .map(|row| (row.device_jid, row.has_key))
            .collect())
    }

    async fn set_sender_key_status(&self, group_jid: &str, entries: &[(&str, bool)]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let (device_jids, has_keys): (Vec<&str>, Vec<bool>) =
            last_per_key(entries.iter().copied(), |(device_jid, _)| *device_jid).unzip();
        let _order = self.writes.lock().await;
        sqlx::query!(
            "INSERT INTO whatsapp_sender_key_devices (account_id, group_jid, device_jid, has_key) \
             SELECT $1, $2, device_jid, has_key FROM UNNEST($3::TEXT[], $4::BOOL[]) AS d (device_jid, has_key) \
             ON CONFLICT (account_id, group_jid, device_jid) DO UPDATE SET has_key = EXCLUDED.has_key",
            self.account_id,
            group_jid,
            &device_jids as &[&str],
            &has_keys,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn clear_sender_key_devices(&self, group_jid: &str) -> Result<()> {
        let _order = self.writes.lock().await;
        sqlx::query!(
            "DELETE FROM whatsapp_sender_key_devices WHERE account_id = $1 AND group_jid = $2",
            self.account_id,
            group_jid,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn delete_sender_key_device_rows(&self, device_jids: &[&str]) -> Result<()> {
        if device_jids.is_empty() {
            return Ok(());
        }
        let _order = self.writes.lock().await;
        sqlx::query!(
            "DELETE FROM whatsapp_sender_key_devices WHERE account_id = $1 AND device_jid = ANY($2)",
            self.account_id,
            device_jids as &[&str],
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn clear_all_sender_key_devices(&self) -> Result<()> {
        let _order = self.writes.lock().await;
        sqlx::query!(
            "DELETE FROM whatsapp_sender_key_devices WHERE account_id = $1",
            self.account_id,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn get_lid_mapping(&self, lid: &str) -> Result<Option<LidPnMappingEntry>> {
        // Behind the write lock: a stale mapping would undo a newer one in the caller's cache
        let _order = self.writes.lock().await;
        sqlx::query_as!(
            LidPnMappingEntry,
            "SELECT lid, phone_number, created_at, updated_at, learning_source FROM whatsapp_lid_mappings \
             WHERE account_id = $1 AND lid = $2",
            self.account_id,
            lid,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db)
    }

    async fn get_pn_mapping(&self, phone: &str) -> Result<Option<LidPnMappingEntry>> {
        let _order = self.writes.lock().await;
        sqlx::query_as!(
            LidPnMappingEntry,
            "SELECT lid, phone_number, created_at, updated_at, learning_source FROM whatsapp_lid_mappings \
             WHERE account_id = $1 AND phone_number = $2 ORDER BY updated_at DESC LIMIT 1",
            self.account_id,
            phone,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db)
    }

    async fn put_lid_mapping(&self, entry: &LidPnMappingEntry) -> Result<()> {
        self.put_lid_mappings(std::slice::from_ref(entry)).await
    }

    async fn put_lid_mappings(&self, entries: &[LidPnMappingEntry]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let entries = merge_lid_repeats(entries);
        let lids: Vec<&str> = entries.iter().map(|entry| entry.lid.as_str()).collect();
        let phone_numbers: Vec<&str> = entries
            .iter()
            .map(|entry| entry.phone_number.as_str())
            .collect();
        let sources: Vec<&str> = entries
            .iter()
            .map(|entry| entry.learning_source.as_str())
            .collect();
        let created_at: Vec<i64> = entries.iter().map(|entry| entry.created_at).collect();
        let updated_at: Vec<i64> = entries.iter().map(|entry| entry.updated_at).collect();
        let _order = self.writes.lock().await;
        // An update keeps the first `created_at`, as upstream's SQLite store does
        sqlx::query!(
            "INSERT INTO whatsapp_lid_mappings (account_id, lid, phone_number, learning_source, created_at, updated_at) \
             SELECT $1, * FROM UNNEST($2::TEXT[], $3::TEXT[], $4::TEXT[], $5::BIGINT[], $6::BIGINT[]) \
             ON CONFLICT (account_id, lid) DO UPDATE SET phone_number = EXCLUDED.phone_number, \
             learning_source = EXCLUDED.learning_source, updated_at = EXCLUDED.updated_at",
            self.account_id,
            &lids as &[&str],
            &phone_numbers as &[&str],
            &sources as &[&str],
            &created_at,
            &updated_at,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn get_all_lid_mappings(&self) -> Result<Vec<LidPnMappingEntry>> {
        let _order = self.writes.lock().await;
        sqlx::query_as!(
            LidPnMappingEntry,
            "SELECT lid, phone_number, created_at, updated_at, learning_source FROM whatsapp_lid_mappings \
             WHERE account_id = $1",
            self.account_id,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db)
    }

    async fn save_base_key(&self, address: &str, message_id: &str, base_key: &[u8]) -> Result<()> {
        let _order = self.writes.lock().await;
        sqlx::query!(
            "INSERT INTO whatsapp_base_keys (account_id, address, message_id, base_key, created_at) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (account_id, address, message_id) DO UPDATE SET base_key = EXCLUDED.base_key, created_at = EXCLUDED.created_at",
            self.account_id,
            address,
            message_id,
            base_key,
            now_secs(),
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn has_same_base_key(
        &self,
        address: &str,
        message_id: &str,
        current_base_key: &[u8],
    ) -> Result<bool> {
        let stored = sqlx::query_scalar!(
            "SELECT base_key FROM whatsapp_base_keys WHERE account_id = $1 AND address = $2 AND message_id = $3",
            self.account_id,
            address,
            message_id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;

        Ok(stored.is_some_and(|stored| stored == current_base_key))
    }

    async fn delete_base_key(&self, address: &str, message_id: &str) -> Result<()> {
        let _order = self.writes.lock().await;
        sqlx::query!(
            "DELETE FROM whatsapp_base_keys WHERE account_id = $1 AND address = $2 AND message_id = $3",
            self.account_id,
            address,
            message_id,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn delete_expired_base_keys(&self, cutoff_timestamp: i64) -> Result<u32> {
        let _order = self.writes.lock().await;
        let deleted = sqlx::query!(
            "DELETE FROM whatsapp_base_keys WHERE account_id = $1 AND created_at < $2",
            self.account_id,
            cutoff_timestamp,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(count(deleted.rows_affected()))
    }

    async fn update_device_list(&self, record: DeviceListRecord) -> Result<()> {
        self.update_device_lists(vec![record]).await
    }

    async fn update_device_lists(&self, records: Vec<DeviceListRecord>) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let mut users = Vec::new();
        let mut sealed = Vec::new();
        for record in last_per_key(records.iter(), |record| &*record.user) {
            sealed.push(self.seal(DEVICE_LISTS, record.user.as_bytes(), &encode(record)?)?);
            users.push(&*record.user);
        }
        let _order = self.writes.lock().await;
        sqlx::query!(
            "INSERT INTO whatsapp_device_lists (account_id, user_jid, record) \
             SELECT $1, * FROM UNNEST($2::TEXT[], $3::BYTEA[]) \
             ON CONFLICT (account_id, user_jid) DO UPDATE SET record = EXCLUDED.record",
            self.account_id,
            &users as &[&str],
            &sealed,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn get_devices(&self, user: &str) -> Result<Option<DeviceListRecord>> {
        let sealed = sqlx::query_scalar!(
            "SELECT record FROM whatsapp_device_lists WHERE account_id = $1 AND user_jid = $2",
            self.account_id,
            user,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;

        sealed
            .map(|sealed| decode(&self.open(DEVICE_LISTS, user.as_bytes(), &sealed)?))
            .transpose()
    }

    async fn get_devices_batch(&self, users: &[&str]) -> Result<Vec<DeviceListRecord>> {
        if users.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query!(
            "SELECT user_jid, record FROM whatsapp_device_lists WHERE account_id = $1 AND user_jid = ANY($2)",
            self.account_id,
            users as &[&str],
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;

        rows.into_iter()
            .map(|row| decode(&self.open(DEVICE_LISTS, row.user_jid.as_bytes(), &row.record)?))
            .collect()
    }

    async fn delete_devices(&self, user: &str) -> Result<()> {
        let _order = self.writes.lock().await;
        sqlx::query!(
            "DELETE FROM whatsapp_device_lists WHERE account_id = $1 AND user_jid = $2",
            self.account_id,
            user,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn get_tc_token(&self, jid: &str) -> Result<Option<TcTokenEntry>> {
        Ok(self.get_tc_tokens(&[jid.to_owned()]).await?.pop().flatten())
    }

    async fn get_tc_tokens(&self, jids: &[String]) -> Result<Vec<Option<TcTokenEntry>>> {
        if jids.is_empty() {
            return Ok(Vec::new());
        }
        // Behind the write lock: a stale timestamp would issue a token twice
        let _order = self.writes.lock().await;
        let rows = sqlx::query!(
            "SELECT jid, token, token_timestamp, sender_timestamp FROM whatsapp_tc_tokens \
             WHERE account_id = $1 AND jid = ANY($2)",
            self.account_id,
            jids,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        let mut found = HashMap::with_capacity(rows.len());
        for row in rows {
            let token = match row.token {
                Some(sealed) => {
                    std::mem::take(&mut *self.open(TC_TOKENS, row.jid.as_bytes(), &sealed)?)
                }
                None => Vec::new(),
            };
            let entry = TcTokenEntry {
                token,
                token_timestamp: row.token_timestamp,
                sender_timestamp: row.sender_timestamp,
            };
            found.insert(row.jid, entry);
        }

        // `get`, not `remove`: a JID asked for twice gets its entry in both places
        Ok(jids.iter().map(|jid| found.get(jid).cloned()).collect())
    }

    async fn put_tc_token(&self, jid: &str, entry: &TcTokenEntry) -> Result<()> {
        let token = self.seal_token(jid, &entry.token)?;
        let _order = self.writes.lock().await;
        sqlx::query!(
            "INSERT INTO whatsapp_tc_tokens (account_id, jid, token, token_timestamp, sender_timestamp) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (account_id, jid) DO UPDATE SET token = EXCLUDED.token, \
             token_timestamp = EXCLUDED.token_timestamp, sender_timestamp = EXCLUDED.sender_timestamp",
            self.account_id,
            jid,
            token,
            entry.token_timestamp,
            entry.sender_timestamp,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn delete_tc_token(&self, jid: &str) -> Result<()> {
        let _order = self.writes.lock().await;
        sqlx::query!(
            "DELETE FROM whatsapp_tc_tokens WHERE account_id = $1 AND jid = $2",
            self.account_id,
            jid,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn get_all_tc_token_jids(&self) -> Result<Vec<String>> {
        sqlx::query_scalar!(
            "SELECT jid FROM whatsapp_tc_tokens WHERE account_id = $1",
            self.account_id,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db)
    }

    async fn delete_expired_tc_tokens(&self, token_cutoff: i64, sender_cutoff: i64) -> Result<u32> {
        let _order = self.writes.lock().await;
        let deleted = sqlx::query!(
            "DELETE FROM whatsapp_tc_tokens WHERE account_id = $1 \
             AND (token IS NULL OR token_timestamp < $2) AND (sender_timestamp IS NULL OR sender_timestamp < $3)",
            self.account_id,
            token_cutoff,
            sender_cutoff,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(count(deleted.rows_affected()))
    }

    async fn touch_tc_token_sender_timestamp(
        &self,
        jid: &str,
        sender_timestamp: i64,
    ) -> Result<()> {
        let _order = self.writes.lock().await;
        // One statement, so the row lock serializes it against every other writer; GREATEST skips
        // a NULL
        sqlx::query!(
            "INSERT INTO whatsapp_tc_tokens (account_id, jid, token, token_timestamp, sender_timestamp) \
             VALUES ($1, $2, NULL, $3, $3) \
             ON CONFLICT (account_id, jid) DO UPDATE \
             SET sender_timestamp = GREATEST(whatsapp_tc_tokens.sender_timestamp, EXCLUDED.sender_timestamp)",
            self.account_id,
            jid,
            sender_timestamp,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn store_received_tc_token(
        &self,
        jid: &str,
        token: &[u8],
        token_timestamp: i64,
    ) -> Result<()> {
        let token = self.seal_token(jid, token)?;
        let _order = self.writes.lock().await;
        // Newer wins, and a placeholder always gives way; the sender timestamp is the other
        // writer's and stays
        sqlx::query!(
            "INSERT INTO whatsapp_tc_tokens (account_id, jid, token, token_timestamp, sender_timestamp) \
             VALUES ($1, $2, $3, $4, NULL) \
             ON CONFLICT (account_id, jid) DO UPDATE SET token = EXCLUDED.token, token_timestamp = EXCLUDED.token_timestamp \
             WHERE whatsapp_tc_tokens.token IS NULL OR EXCLUDED.token_timestamp >= whatsapp_tc_tokens.token_timestamp",
            self.account_id,
            jid,
            token,
            token_timestamp,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn store_sent_message(
        &self,
        chat_jid: &str,
        message_id: &str,
        payload: &[u8],
    ) -> Result<()> {
        let sealed = self.seal(SENT_MESSAGES, &composite(&[chat_jid, message_id]), payload)?;
        let _order = self.writes.lock().await;
        sqlx::query!(
            "INSERT INTO whatsapp_sent_messages (account_id, chat_jid, message_id, payload, created_at) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (account_id, chat_jid, message_id) DO UPDATE SET payload = EXCLUDED.payload, created_at = EXCLUDED.created_at",
            self.account_id,
            chat_jid,
            message_id,
            sealed,
            now_secs(),
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(())
    }

    async fn get_sent_message(&self, chat_jid: &str, message_id: &str) -> Result<Option<Vec<u8>>> {
        let sealed = sqlx::query_scalar!(
            "SELECT payload FROM whatsapp_sent_messages WHERE account_id = $1 AND chat_jid = $2 AND message_id = $3",
            self.account_id,
            chat_jid,
            message_id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;

        sealed
            .map(|sealed| self.open_sent_message(chat_jid, message_id, &sealed))
            .transpose()
    }

    async fn take_sent_message(&self, chat_jid: &str, message_id: &str) -> Result<Option<Vec<u8>>> {
        let _order = self.writes.lock().await;
        let mut tx = self.pool.begin().await.map_err(db)?;
        let sealed = sqlx::query_scalar!(
            "DELETE FROM whatsapp_sent_messages WHERE account_id = $1 AND chat_jid = $2 AND message_id = $3 RETURNING payload",
            self.account_id,
            chat_jid,
            message_id,
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?;
        // Opened before the commit, so a payload that won't open stays in place
        let payload = sealed
            .map(|sealed| self.open_sent_message(chat_jid, message_id, &sealed))
            .transpose()?;
        tx.commit().await.map_err(db)?;

        Ok(payload)
    }

    async fn delete_expired_sent_messages(&self, cutoff_timestamp: i64) -> Result<u32> {
        let _order = self.writes.lock().await;
        let deleted = sqlx::query!(
            "DELETE FROM whatsapp_sent_messages WHERE account_id = $1 AND created_at < $2",
            self.account_id,
            cutoff_timestamp,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(count(deleted.rows_affected()))
    }
}

impl PgStore {
    /// An empty token is a placeholder, stored as NULL
    fn seal_token(&self, jid: &str, token: &[u8]) -> Result<Option<Vec<u8>>> {
        (!token.is_empty())
            .then(|| self.seal(TC_TOKENS, jid.as_bytes(), token))
            .transpose()
    }

    fn open_sent_message(
        &self,
        chat_jid: &str,
        message_id: &str,
        sealed: &[u8],
    ) -> Result<Vec<u8>> {
        let mut payload = self.open(SENT_MESSAGES, &composite(&[chat_jid, message_id]), sealed)?;
        Ok(std::mem::take(&mut *payload))
    }
}

/// One entry per LID, as SQLite's one-by-one upserts would leave it: the last entry, keeping the
/// first one's `created_at`
fn merge_lid_repeats(entries: &[LidPnMappingEntry]) -> Vec<LidPnMappingEntry> {
    let mut merged: Vec<LidPnMappingEntry> = Vec::with_capacity(entries.len());
    let mut positions: HashMap<&str, usize> = HashMap::new();
    for entry in entries {
        if let Some(&position) = positions.get(entry.lid.as_str()) {
            let created_at = merged[position].created_at;
            merged[position] = LidPnMappingEntry {
                created_at,
                ..entry.clone()
            };
        } else {
            positions.insert(&entry.lid, merged.len());
            merged.push(entry.clone());
        }
    }
    merged
}

/// Postgres refuses an upsert that touches one row twice, so a repeated key keeps only its last
/// entry, the one applying the batch in order would leave
fn last_per_key<T, K: Eq + std::hash::Hash>(
    entries: impl DoubleEndedIterator<Item = T>,
    key: impl Fn(&T) -> K,
) -> impl Iterator<Item = T> {
    let mut seen = HashSet::new();
    let mut kept: Vec<T> = entries
        .rev()
        .filter(|entry| seen.insert(key(entry)))
        .collect();
    kept.reverse();
    kept.into_iter()
}
