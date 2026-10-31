use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use whatsapp_rust::wacore::store::{
    error::Result,
    traits::{
        MsgSecretEntry, MsgSecretStore, StoredMessageSecret, merge_msg_secret_expiry,
        merge_msg_secret_message_ts,
    },
};

use super::{PgStore, composite, count, db};

const MSG_SECRETS: &str = "msg_secrets";

#[async_trait]
impl MsgSecretStore for PgStore {
    async fn put_msg_secrets(&self, entries: Vec<MsgSecretEntry>) -> Result<usize> {
        let stored = entries.len();
        if stored == 0 {
            return Ok(0);
        }
        let entries = merge_repeats(entries);
        let mut chats = Vec::with_capacity(entries.len());
        let mut senders = Vec::with_capacity(entries.len());
        let mut msg_ids = Vec::with_capacity(entries.len());
        let mut secrets = Vec::with_capacity(entries.len());
        for entry in &entries {
            let key = composite(&[&entry.chat, &entry.sender, &entry.msg_id]);
            secrets.push(self.seal(MSG_SECRETS, &key, &entry.secret)?);
            chats.push(&*entry.chat);
            senders.push(&*entry.sender);
            msg_ids.push(&*entry.msg_id);
        }
        let expires_at: Vec<i64> = entries.iter().map(|entry| entry.expires_at).collect();
        let message_ts: Vec<i64> = entries.iter().map(|entry| entry.message_ts).collect();
        let _order = self.writes.lock().await;
        // The merges match `merge_msg_secret_expiry` (0 means never, so it wins) and
        // `merge_msg_secret_message_ts` (the later time wins)
        sqlx::query!(
            "INSERT INTO whatsapp_msg_secrets (account_id, chat, sender, msg_id, secret, expires_at, message_ts) \
             SELECT $1, * FROM UNNEST($2::TEXT[], $3::TEXT[], $4::TEXT[], $5::BYTEA[], $6::BIGINT[], $7::BIGINT[]) \
             ON CONFLICT (account_id, chat, sender, msg_id) DO UPDATE SET secret = EXCLUDED.secret, \
             expires_at = CASE WHEN whatsapp_msg_secrets.expires_at = 0 OR EXCLUDED.expires_at = 0 THEN 0 \
             ELSE GREATEST(whatsapp_msg_secrets.expires_at, EXCLUDED.expires_at) END, \
             message_ts = GREATEST(whatsapp_msg_secrets.message_ts, EXCLUDED.message_ts)",
            self.account_id,
            &chats as &[&str],
            &senders as &[&str],
            &msg_ids as &[&str],
            &secrets,
            &expires_at,
            &message_ts,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(stored)
    }

    async fn get_stored_msg_secret(
        &self,
        chat: &str,
        sender: &str,
        msg_id: &str,
    ) -> Result<Option<StoredMessageSecret>> {
        // Behind the write lock: a secret reported missing while its write is queued is lost for good
        let _order = self.writes.lock().await;
        let row = sqlx::query!(
            "SELECT secret, message_ts FROM whatsapp_msg_secrets \
             WHERE account_id = $1 AND chat = $2 AND sender = $3 AND msg_id = $4",
            self.account_id,
            chat,
            sender,
            msg_id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;

        row.map(|row| {
            let secret = self.open(
                MSG_SECRETS,
                &composite(&[chat, sender, msg_id]),
                &row.secret,
            )?;
            StoredMessageSecret::from_stored_bytes(&secret, row.message_ts)
        })
        .transpose()
    }

    async fn delete_expired_msg_secrets(&self, cutoff_timestamp: i64) -> Result<u32> {
        let _order = self.writes.lock().await;
        let deleted = sqlx::query!(
            "DELETE FROM whatsapp_msg_secrets WHERE account_id = $1 AND expires_at <> 0 AND expires_at <= $2",
            self.account_id,
            cutoff_timestamp,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(count(deleted.rows_affected()))
    }
}

/// Postgres refuses an upsert that touches one row twice, so repeats of a key are merged first, by
/// the same rules the upsert applies to the stored row
fn merge_repeats(entries: Vec<MsgSecretEntry>) -> Vec<MsgSecretEntry> {
    let mut merged: Vec<MsgSecretEntry> = Vec::with_capacity(entries.len());
    let mut positions: HashMap<_, usize> = HashMap::new();
    for entry in entries {
        let key = (
            Arc::clone(&entry.chat),
            Arc::clone(&entry.sender),
            Arc::clone(&entry.msg_id),
        );
        if let Some(&position) = positions.get(&key) {
            let earlier = &mut merged[position];
            earlier.secret = entry.secret;
            earlier.expires_at = merge_msg_secret_expiry(earlier.expires_at, entry.expires_at);
            earlier.message_ts = merge_msg_secret_message_ts(earlier.message_ts, entry.message_ts);
        } else {
            positions.insert(key, merged.len());
            merged.push(entry);
        }
    }
    merged
}
