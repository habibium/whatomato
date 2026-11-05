use std::sync::Arc;

use async_trait::async_trait;
use whatsapp_rust::wacore::{
    client_profile::ClientProfile,
    stats::StorageResourceReport,
    store::{
        Device,
        device::DEVICE_PROPS,
        error::{Result, StoreError},
        traits::DeviceStore,
    },
};

use super::{PgStore, db, decode, encode};

const ACCOUNTS: &str = "accounts";
const DEVICE: &[u8] = b"device";
/// One device per account, so `create` always returns the id upstream's stores give a first device
const DEVICE_ID: i32 = 1;

#[async_trait]
impl DeviceStore for PgStore {
    async fn save(&self, device: &Device) -> Result<()> {
        let sealed = self.seal(ACCOUNTS, DEVICE, &encode(device)?)?;
        let _order = self.writes.lock().await;
        // An UPDATE, never an upsert: a save after the account is deleted must not revive its keys
        let updated = sqlx::query!(
            "UPDATE whatsapp_accounts SET device = $2, updated_at = now() WHERE id = $1",
            self.account_id,
            sealed,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;
        if updated.rows_affected() == 0 {
            return Err(StoreError::DeviceNotFound(DEVICE_ID));
        }

        Ok(())
    }

    async fn load(&self) -> Result<Option<Device>> {
        let sealed = sqlx::query_scalar!(
            "SELECT device FROM whatsapp_accounts WHERE id = $1",
            self.account_id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?
        .flatten();

        sealed
            .map(|sealed| {
                let mut device: Device = decode(&self.open(ACCOUNTS, DEVICE, &sealed)?)?;
                // serde skips the runtime-only fields; set them as upstream's SQLite store does
                device.device_props = Arc::new(DEVICE_PROPS.clone());
                device.client_profile = ClientProfile::web();
                device.nct_salt_sync_seen = false;
                Ok(device)
            })
            .transpose()
    }

    async fn exists(&self) -> Result<bool> {
        sqlx::query_scalar!(
            r#"SELECT EXISTS (SELECT 1 FROM whatsapp_accounts WHERE id = $1 AND device IS NOT NULL) AS "exists!""#,
            self.account_id,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(db)
    }

    async fn create(&self) -> Result<i32> {
        let sealed = self.seal(ACCOUNTS, DEVICE, &encode(&Device::new())?)?;
        let _order = self.writes.lock().await;
        // COALESCE keeps a device that's already stored: its keys must never be replaced
        let updated = sqlx::query!(
            "UPDATE whatsapp_accounts SET device = COALESCE(device, $2), updated_at = now() WHERE id = $1",
            self.account_id,
            sealed,
        )
        .execute(&self.pool)
        .await
        .map_err(db)?;
        if updated.rows_affected() == 0 {
            return Err(StoreError::DeviceNotFound(DEVICE_ID));
        }

        Ok(DEVICE_ID)
    }

    /// The trait asks a store-backed backend for a positive zero: its data isn't process memory
    async fn resource_report(&self) -> StorageResourceReport {
        StorageResourceReport {
            memory_bytes: Some(0),
            ..Default::default()
        }
    }
}
