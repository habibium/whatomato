use sqlx::PgPool;
use uuid::Uuid;
use whatsapp_rust::wacore::{
    appstate::{hash::HashState, processor::AppStateMutationMAC},
    store::{
        error::StoreError,
        traits::{
            AppStateSyncKey, AppSyncStore, DeviceInfo, DeviceListRecord, LidPnMappingEntry,
            MsgSecretEntry, MsgSecretStore, ProtocolStore, TcTokenEntry,
        },
    },
    time::now_secs,
};

use super::{PgStore, composite};
use crate::crypto::Cipher;

const KEY: [u8; 32] = [7; 32];

async fn account(pool: &PgPool) -> Uuid {
    let user_id = sqlx::query_scalar!(
        "INSERT INTO users (email, password) VALUES ($1, 'unused') RETURNING id",
        format!("{}@example.com", Uuid::new_v4()),
    )
    .fetch_one(pool)
    .await
    .unwrap();

    sqlx::query_scalar!(
        "INSERT INTO whatsapp_accounts (user_id) VALUES ($1) RETURNING id",
        user_id,
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn store(pool: &PgPool) -> PgStore {
    PgStore::new(pool.clone(), Cipher::new(&KEY), account(pool).await)
}

fn sync_key(byte: u8) -> AppStateSyncKey {
    AppStateSyncKey {
        key_data: vec![byte; 32],
        fingerprint: vec![byte; 4],
        timestamp: i64::from(byte),
    }
}

fn mac(index: u8, value: u8) -> AppStateMutationMAC {
    AppStateMutationMAC {
        index_mac: vec![index; 32],
        value_mac: vec![value; 32],
    }
}

/// Ported from whatsapp-rust's `wacore/tests/support/appstate_commit_contract.rs` (MIT)
#[sqlx::test]
async fn commits_a_patch_as_a_unit(pool: PgPool) {
    let store = store(&pool).await;
    let old = mac(1, 11);
    let replacement = mac(1, 22);
    let removed = mac(2, 33);
    for name in ["regular", "regular_low"] {
        store
            .commit_patch(
                name,
                HashState {
                    version: 1,
                    ..Default::default()
                },
                &[],
                &[old.clone(), removed.clone()],
            )
            .await
            .unwrap();
    }
    store
        .commit_patch(
            "regular",
            HashState {
                version: 2,
                hash: [2; 128],
                ..Default::default()
            },
            &[old.index_mac.clone(), removed.index_mac.clone()],
            std::slice::from_ref(&replacement),
        )
        .await
        .unwrap();

    let get = |name, index_mac| store.get_mutation_mac(name, index_mac);
    assert_eq!(
        get("regular", &old.index_mac).await.unwrap(),
        Some(replacement.value_mac.clone())
    );
    assert_eq!(get("regular", &removed.index_mac).await.unwrap(), None);
    assert_eq!(
        get("regular_low", &old.index_mac).await.unwrap(),
        Some(old.value_mac)
    );
    let version = |name| store.get_version(name);
    assert_eq!(version("regular_low").await.unwrap().unwrap().version, 1);
    let held = version("regular").await.unwrap().unwrap();
    assert_eq!(held.version, 2);
    assert_eq!(held.hash, [2; 128]);

    store
        .commit_patch("regular", HashState { version: 3, ..held }, &[], &[])
        .await
        .unwrap();
    assert_eq!(version("regular").await.unwrap().unwrap().version, 3);
    assert_eq!(
        get("regular", &old.index_mac).await.unwrap(),
        Some(replacement.value_mac)
    );
}

#[sqlx::test]
async fn a_never_synced_collection_has_no_version(pool: PgPool) {
    let store = store(&pool).await;
    assert!(store.get_version("regular").await.unwrap().is_none());

    store
        .set_version("regular", HashState::default())
        .await
        .unwrap();
    assert_eq!(
        store.get_version("regular").await.unwrap().unwrap().version,
        0
    );

    store.delete_version("regular").await.unwrap();
    assert!(store.get_version("regular").await.unwrap().is_none());
}

#[sqlx::test]
async fn a_repeated_index_keeps_its_last_value(pool: PgPool) {
    let store = store(&pool).await;
    store
        .put_mutation_macs("regular", 1, &[mac(1, 10), mac(2, 20), mac(1, 11)])
        .await
        .unwrap();

    let found = store
        .get_mutation_macs("regular", &[[1; 32], [2; 32], [3; 32]])
        .await
        .unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!(found[&[1; 32]], vec![11; 32]);
    assert_eq!(found[&[2; 32]], vec![20; 32]);

    store
        .delete_mutation_macs("regular", &[vec![1; 32]])
        .await
        .unwrap();
    assert_eq!(
        store.get_mutation_mac("regular", &[1; 32]).await.unwrap(),
        None
    );
    store.clear_mutation_macs("regular").await.unwrap();
    assert_eq!(
        store.get_mutation_mac("regular", &[2; 32]).await.unwrap(),
        None
    );
}

#[sqlx::test]
async fn the_latest_sync_key_is_the_one_stored_last(pool: PgPool) {
    let store = store(&pool).await;
    assert_eq!(store.get_latest_sync_key_id().await.unwrap(), None);

    for key_id in [[2], [1], [3], [1]] {
        store
            .set_sync_key(&key_id, sync_key(key_id[0]))
            .await
            .unwrap();
    }
    assert_eq!(store.get_latest_sync_key_id().await.unwrap(), Some(vec![1]));
    let stored = store.get_sync_key(&[3]).await.unwrap().unwrap();
    assert_eq!(stored.key_data, sync_key(3).key_data);
    assert_eq!(stored.timestamp, 3);
    assert!(store.get_sync_key(&[4]).await.unwrap().is_none());
}

#[sqlx::test]
async fn sealed_values_open_only_in_their_own_row(pool: PgPool) {
    let ada = store(&pool).await;
    let grace = store(&pool).await;
    ada.set_sync_key(&[1], sync_key(9)).await.unwrap();

    let sealed = sqlx::query_scalar!(
        "SELECT key FROM whatsapp_app_state_keys WHERE account_id = $1",
        ada.account_id,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!sealed.windows(32).any(|window| window == [9; 32]));

    // Grace never sees Ada's rows, and a copy of Ada's sealed value refuses to open as hers
    assert!(grace.get_sync_key(&[1]).await.unwrap().is_none());
    sqlx::query!(
        "INSERT INTO whatsapp_app_state_keys (account_id, key_id, key) VALUES ($1, $2, $3)",
        grace.account_id,
        &[1u8][..],
        sealed,
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(grace.get_sync_key(&[1]).await.is_err());
}

/// Ported from whatsapp-rust's `wacore/tests/support/tc_token_contract.rs` (MIT)
#[sqlx::test]
async fn privacy_tokens_merge_newer_wins_and_never_lose_the_sender_time(pool: PgPool) {
    let store = store(&pool).await;
    let jid = "100001@lid";
    store
        .touch_tc_token_sender_timestamp(jid, 9000)
        .await
        .unwrap();
    store
        .store_received_tc_token(jid, b"first", 3000)
        .await
        .unwrap();
    let row = store.get_tc_token(jid).await.unwrap().unwrap();
    assert_eq!(row.token, b"first", "placeholder cannot block real bytes");
    assert_eq!(row.token_timestamp, 3000);
    assert_eq!(row.sender_timestamp, Some(9000));

    store
        .store_received_tc_token(jid, b"new", 6000)
        .await
        .unwrap();
    store
        .store_received_tc_token(jid, b"old", 2000)
        .await
        .unwrap();
    store
        .touch_tc_token_sender_timestamp(jid, 1000)
        .await
        .unwrap();
    let row = store.get_tc_token(jid).await.unwrap().unwrap();
    assert_eq!(row.token, b"new");
    assert_eq!(row.token_timestamp, 6000);
    assert_eq!(row.sender_timestamp, Some(9000));

    store
        .store_received_tc_token(jid, b"equal", 6000)
        .await
        .unwrap();
    assert_eq!(
        store.get_tc_token(jid).await.unwrap().unwrap().token,
        b"equal"
    );

    for round in 0..16 {
        let jid = format!("race-{round}@lid");
        let (received, sender, stale, old_sender) = tokio::join!(
            store.store_received_tc_token(&jid, b"new", 6000),
            store.touch_tc_token_sender_timestamp(&jid, 9000),
            store.store_received_tc_token(&jid, b"old", 2000),
            store.touch_tc_token_sender_timestamp(&jid, 1000),
        );
        received.unwrap();
        sender.unwrap();
        stale.unwrap();
        old_sender.unwrap();
        let row = store.get_tc_token(&jid).await.unwrap().unwrap();
        assert_eq!(row.token, b"new");
        assert_eq!(row.token_timestamp, 6000);
        assert_eq!(row.sender_timestamp, Some(9000));
    }
}

#[sqlx::test]
async fn privacy_tokens_expire_only_once_both_halves_are_stale(pool: PgPool) {
    let store = store(&pool).await;
    let entry = |token: &[u8], token_timestamp, sender_timestamp| TcTokenEntry {
        token: token.to_vec(),
        token_timestamp,
        sender_timestamp,
    };
    store
        .put_tc_token("live-token@lid", &entry(b"t", 200, None))
        .await
        .unwrap();
    store
        .put_tc_token("live-sender@lid", &entry(b"t", 50, Some(200)))
        .await
        .unwrap();
    store
        .put_tc_token("placeholder@lid", &entry(b"", 200, Some(50)))
        .await
        .unwrap();
    store
        .put_tc_token("stale@lid", &entry(b"t", 50, Some(50)))
        .await
        .unwrap();

    assert_eq!(store.delete_expired_tc_tokens(100, 100).await.unwrap(), 2);
    let mut jids = store.get_all_tc_token_jids().await.unwrap();
    jids.sort();
    assert_eq!(jids, ["live-sender@lid", "live-token@lid"]);

    let found = store
        .get_tc_tokens(&[
            "live-token@lid".into(),
            "gone@lid".into(),
            "live-sender@lid".into(),
            "live-token@lid".into(),
        ])
        .await
        .unwrap();
    assert_eq!(found[0].as_ref().unwrap().token_timestamp, 200);
    assert!(found[1].is_none());
    assert_eq!(found[2].as_ref().unwrap().sender_timestamp, Some(200));
    // A JID asked for twice gets its entry both times
    assert_eq!(found[3].as_ref().unwrap().token_timestamp, 200);

    store.delete_tc_token("live-token@lid").await.unwrap();
    assert!(
        store
            .get_tc_token("live-token@lid")
            .await
            .unwrap()
            .is_none()
    );
}

#[sqlx::test]
async fn tracks_which_devices_hold_the_sender_key(pool: PgPool) {
    let store = store(&pool).await;
    store
        .set_sender_key_status(
            "g1@g.us",
            &[("a:1@lid", false), ("b:2@lid", true), ("a:1@lid", true)],
        )
        .await
        .unwrap();
    store
        .set_sender_key_status("g2@g.us", &[("a:1@lid", true)])
        .await
        .unwrap();

    let mut devices = store.get_sender_key_devices("g1@g.us").await.unwrap();
    devices.sort();
    assert_eq!(
        devices,
        [("a:1@lid".to_owned(), true), ("b:2@lid".to_owned(), true)]
    );

    store
        .delete_sender_key_device_rows(&["a:1@lid"])
        .await
        .unwrap();
    assert_eq!(
        store.get_sender_key_devices("g1@g.us").await.unwrap(),
        [("b:2@lid".to_owned(), true)]
    );
    assert_eq!(
        store.get_sender_key_devices("g2@g.us").await.unwrap(),
        Vec::<(String, bool)>::new()
    );

    store.clear_sender_key_devices("g1@g.us").await.unwrap();
    assert_eq!(
        store.get_sender_key_devices("g1@g.us").await.unwrap(),
        Vec::<(String, bool)>::new()
    );
    store
        .set_sender_key_status("g3@g.us", &[("c:3@lid", true)])
        .await
        .unwrap();
    store.clear_all_sender_key_devices().await.unwrap();
    assert_eq!(
        store.get_sender_key_devices("g3@g.us").await.unwrap(),
        Vec::<(String, bool)>::new()
    );
}

#[sqlx::test]
async fn lid_mappings_keep_their_first_sighting_and_find_the_newest_lid(pool: PgPool) {
    let store = store(&pool).await;
    let mapping = |lid: &str, phone: &str, at: i64| LidPnMappingEntry {
        lid: lid.into(),
        phone_number: phone.into(),
        created_at: at,
        updated_at: at,
        learning_source: "usync".into(),
    };
    store
        .put_lid_mapping(&mapping("100", "15550001", 10))
        .await
        .unwrap();
    store
        .put_lid_mappings(&[
            mapping("200", "15550001", 20),
            mapping("100", "15550002", 30),
            mapping("100", "15550003", 40),
        ])
        .await
        .unwrap();

    let moved = store.get_lid_mapping("100").await.unwrap().unwrap();
    assert_eq!(moved.phone_number, "15550003");
    assert_eq!(moved.created_at, 10);
    assert_eq!(moved.updated_at, 40);
    assert_eq!(
        store.get_pn_mapping("15550001").await.unwrap().unwrap().lid,
        "200"
    );
    assert!(store.get_pn_mapping("15550002").await.unwrap().is_none());

    // A new LID twice in one batch ends as SQLite's one-by-one upserts leave it
    store
        .put_lid_mappings(&[
            mapping("300", "15550004", 50),
            mapping("300", "15550005", 60),
        ])
        .await
        .unwrap();
    let repeated = store.get_lid_mapping("300").await.unwrap().unwrap();
    assert_eq!(repeated.phone_number, "15550005");
    assert_eq!(repeated.created_at, 50);
    assert_eq!(repeated.updated_at, 60);
    assert_eq!(store.get_all_lid_mappings().await.unwrap().len(), 3);
}

#[sqlx::test]
async fn base_keys_compare_and_expire(pool: PgPool) {
    let store = store(&pool).await;
    store
        .save_base_key("a:1@lid", "m1", &[1; 32])
        .await
        .unwrap();

    assert!(
        store
            .has_same_base_key("a:1@lid", "m1", &[1; 32])
            .await
            .unwrap()
    );
    assert!(
        !store
            .has_same_base_key("a:1@lid", "m1", &[2; 32])
            .await
            .unwrap()
    );
    assert!(
        !store
            .has_same_base_key("a:1@lid", "m2", &[1; 32])
            .await
            .unwrap()
    );

    assert_eq!(
        store
            .delete_expired_base_keys(now_secs() - 60)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .delete_expired_base_keys(now_secs() + 1)
            .await
            .unwrap(),
        1
    );
    store
        .save_base_key("a:1@lid", "m3", &[3; 32])
        .await
        .unwrap();
    store.delete_base_key("a:1@lid", "m3").await.unwrap();
    assert!(
        !store
            .has_same_base_key("a:1@lid", "m3", &[3; 32])
            .await
            .unwrap()
    );
}

#[sqlx::test]
async fn device_lists_round_trip_sealed(pool: PgPool) {
    let store = store(&pool).await;
    let record = |user: &str, timestamp| DeviceListRecord {
        user: user.into(),
        devices: Box::new([DeviceInfo::new(0, None), DeviceInfo::new(7, Some(2))]),
        timestamp,
        phash: Some("2:abc".into()),
        raw_id: Some(9),
    };
    store
        .update_device_lists(vec![
            record("1@s.whatsapp.net", 1),
            record("2@s.whatsapp.net", 2),
            record("1@s.whatsapp.net", 3),
        ])
        .await
        .unwrap();

    let stored = store
        .get_devices("1@s.whatsapp.net")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.timestamp, 3);
    assert_eq!(stored.devices.len(), 2);
    assert_eq!(stored.raw_id, Some(9));
    let batch = store
        .get_devices_batch(&["1@s.whatsapp.net", "2@s.whatsapp.net", "3@s.whatsapp.net"])
        .await
        .unwrap();
    assert_eq!(batch.len(), 2);

    store.delete_devices("1@s.whatsapp.net").await.unwrap();
    assert!(
        store
            .get_devices("1@s.whatsapp.net")
            .await
            .unwrap()
            .is_none()
    );
}

#[sqlx::test]
async fn a_sent_message_is_read_freely_but_taken_once(pool: PgPool) {
    let store = store(&pool).await;
    store
        .store_sent_message("1@s.whatsapp.net", "m1", b"payload")
        .await
        .unwrap();

    let sealed = sqlx::query_scalar!(
        "SELECT payload FROM whatsapp_sent_messages WHERE account_id = $1",
        store.account_id,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!sealed.windows(7).any(|window| window == b"payload"));

    assert_eq!(
        store
            .get_sent_message("1@s.whatsapp.net", "m1")
            .await
            .unwrap()
            .unwrap(),
        b"payload"
    );
    assert_eq!(
        store
            .take_sent_message("1@s.whatsapp.net", "m1")
            .await
            .unwrap()
            .unwrap(),
        b"payload"
    );
    assert!(
        store
            .take_sent_message("1@s.whatsapp.net", "m1")
            .await
            .unwrap()
            .is_none()
    );

    store
        .store_sent_message("1@s.whatsapp.net", "m2", b"later")
        .await
        .unwrap();
    assert_eq!(
        store
            .delete_expired_sent_messages(now_secs() - 60)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .delete_expired_sent_messages(now_secs() + 1)
            .await
            .unwrap(),
        1
    );
}

/// Ported from whatsapp-rust's `wacore/tests/stored_message_secret.rs` (MIT)
#[sqlx::test]
async fn message_secrets_merge_their_deadlines_and_times(pool: PgPool) {
    let store = store(&pool).await;
    store
        .put_msg_secret("c", "s", "unknown", &[177; 32])
        .await
        .unwrap();
    let unknown = store
        .get_stored_msg_secret("c", "s", "unknown")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unknown.message_ts, None);
    assert_eq!(unknown.secret.as_bytes(), &[177; 32]);

    let entry = |message_ts, expires_at| MsgSecretEntry {
        chat: "c".into(),
        sender: "s".into(),
        msg_id: "known".into(),
        secret: [177; 32],
        expires_at,
        message_ts,
    };
    for (message_ts, expires_at) in [(120, 250), (0, 150), (90, 190), (100, 200)] {
        store
            .put_msg_secrets(vec![entry(message_ts, expires_at)])
            .await
            .unwrap();
    }
    // The same merges apply between rows of one batch
    store
        .put_msg_secrets(vec![entry(130, 260), entry(0, 100)])
        .await
        .unwrap();
    let known = store
        .get_stored_msg_secret("c", "s", "known")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(known.message_ts, Some(130));

    assert_eq!(store.delete_expired_msg_secrets(259).await.unwrap(), 0);
    assert_eq!(store.delete_expired_msg_secrets(260).await.unwrap(), 1);
    assert!(
        store
            .get_stored_msg_secret("c", "s", "known")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .get_stored_msg_secret("c", "s", "unknown")
            .await
            .unwrap()
            .is_some()
    );
}

#[sqlx::test]
async fn a_stored_secret_of_the_wrong_length_is_an_error(pool: PgPool) {
    let store = store(&pool).await;
    let sealed = store
        .seal("msg_secrets", &composite(&["c", "s", "short"]), &[1; 31])
        .unwrap();
    sqlx::query!(
        "INSERT INTO whatsapp_msg_secrets (account_id, chat, sender, msg_id, secret, expires_at, message_ts) \
         VALUES ($1, 'c', 's', 'short', $2, 0, 0)",
        store.account_id,
        sealed,
    )
    .execute(&pool)
    .await
    .unwrap();

    assert!(matches!(
        store.get_stored_msg_secret("c", "s", "short").await,
        Err(StoreError::InvalidMessageSecret(_))
    ));
}
