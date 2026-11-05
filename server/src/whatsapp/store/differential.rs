//! Seeded random operation sequences run against `PgStore` and upstream's `InMemoryBackend`, the
//! reference for the trait contracts. Every answer has to match.

use std::{
    fmt::Debug,
    sync::{Arc, LazyLock},
};

use bytes::Bytes;
use sqlx::PgPool;
use whatsapp_rust::wacore::{
    appstate::{hash::HashState, processor::AppStateMutationMAC},
    store::{
        Device, InMemoryBackend,
        error::Result,
        traits::{
            AppStateSyncKey, Backend, DeviceInfo, DeviceListRecord, LidPnMappingEntry,
            MsgSecretEntry, TcTokenEntry,
        },
    },
    time::now_secs,
};

use super::{encode, tests::store};

const SEEDS: u64 = 16;
const STEPS: usize = 250;

// Small key spaces, so overwrites, repeats within a batch and misses all happen often
const ADDRESSES: [&str; 5] = [
    "111@s.whatsapp.net",
    "111:1@s.whatsapp.net",
    "222@lid",
    "222:3@lid",
    "333@s.whatsapp.net",
];
// Whole users, a prefix of one, and one with a longer number
const USERS: [&str; 5] = ["111", "222", "333", "11", "1111"];
const PREKEY_IDS: [u32; 4] = [1, 2, 3, 16_777_215];
const SIGNED_PREKEY_IDS: [u32; 2] = [1, 2];
const SYNC_KEY_IDS: [&[u8]; 3] = [b"k1", b"k2", b"k3"];
const COLLECTIONS: [&str; 2] = ["regular", "critical_block"];
const MAC_INDEXES: [u8; 4] = [1, 2, 3, 4];
const GROUPS: [&str; 2] = ["g1@g.us", "g2@g.us"];
// Each LID moves only between numbers of its own: where two share one, upstream's in-memory store
// drops that number's reverse entry, while SQLite, like ours, finds the other LID
const LIDS: [(&str, [&str; 2]); 3] = [
    ("1@lid", ["10", "11"]),
    ("2@lid", ["20", "21"]),
    ("3@lid", ["30", "31"]),
];
const PHONES: [&str; 6] = ["10", "11", "20", "21", "30", "31"];
const MESSAGE_IDS: [&str; 2] = ["m1", "m2"];
const CHATS: [&str; 2] = ["c1@g.us", "c2@s.whatsapp.net"];
const SENDERS: [&str; 2] = ["s1@lid", "s2@lid"];
const TIMES: [i64; 4] = [0, 100, 200, 300];

#[sqlx::test]
async fn pg_store_answers_like_the_in_memory_backend(pool: PgPool) {
    for seed in 0..SEEDS {
        let pg = store(&pool).await;
        let reference = InMemoryBackend::new();
        let mut rng = Rng(seed);
        for step in 0..STEPS {
            let op = Op::random(&mut rng, step);
            check(&pg, &reference, &op, seed, step).await;
        }
        for op in sweep() {
            check(&pg, &reference, &op, seed, STEPS).await;
        }
    }
}

async fn check(pg: &impl Backend, reference: &impl Backend, op: &Op, seed: u64, step: usize) {
    let expected = op.apply(reference).await;
    let actual = op.apply(pg).await;
    assert_eq!(actual, expected, "seed {seed}, step {step}: {op:?}");
}

/// `SplitMix64`, so a failing run replays from its seed
struct Rng(u64);

impl Rng {
    const fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next_u64() % u64::try_from(n).unwrap()).unwrap()
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())]
    }

    fn some<T: Copy>(&mut self, items: &[T]) -> Vec<T> {
        let len = self.below(4);
        (0..len).map(|_| self.pick(items)).collect()
    }

    const fn flag(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }

    const fn byte(&mut self) -> u8 {
        self.next_u64().to_le_bytes()[0]
    }

    fn value(&mut self) -> Vec<u8> {
        let len = 1 + self.below(48);
        vec![self.byte(); len]
    }
}

#[derive(Debug)]
enum Op {
    Signal(SignalOp),
    Session(SessionOp),
    Device(DeviceOp),
    AppSync(AppSyncOp),
    Protocol(ProtocolOp),
    Token(TokenOp),
    Secret(SecretOp),
}

impl Op {
    fn random(rng: &mut Rng, step: usize) -> Self {
        // Writes that carry a time get one that only moves forward, as the library's clock does
        let now = 1_000 + i64::try_from(step).unwrap();
        match rng.below(7) {
            0 => Self::Signal(SignalOp::random(rng)),
            1 => Self::Session(SessionOp::random(rng)),
            2 => Self::Device(DeviceOp::random(rng)),
            3 => Self::AppSync(AppSyncOp::random(rng)),
            4 => Self::Protocol(ProtocolOp::random(rng, now)),
            5 => Self::Token(TokenOp::random(rng)),
            _ => Self::Secret(SecretOp::random(rng)),
        }
    }

    async fn apply(&self, store: &impl Backend) -> String {
        match self {
            Self::Signal(op) => op.apply(store).await,
            Self::Session(op) => op.apply(store).await,
            Self::Device(op) => op.apply(store).await,
            Self::AppSync(op) => op.apply(store).await,
            Self::Protocol(op) => op.apply(store).await,
            Self::Token(op) => op.apply(store).await,
            Self::Secret(op) => op.apply(store).await,
        }
    }
}

/// Every read over the whole key space, to compare the final state
fn sweep() -> Vec<Op> {
    let mut ops = vec![
        Op::Signal(SignalOp::LoadPrekeys(PREKEY_IDS.to_vec())),
        Op::Signal(SignalOp::MaxPrekeyId),
        Op::Signal(SignalOp::LoadAllSignedPrekeys),
        Op::Session(SessionOp::GetSessions(ADDRESSES.to_vec())),
        Op::Device(DeviceOp::Load),
        Op::Device(DeviceOp::Exists),
        Op::AppSync(AppSyncOp::LatestSyncKeyId),
        Op::Protocol(ProtocolOp::AllLidMappings),
        Op::Protocol(ProtocolOp::GetDevicesBatch(ADDRESSES.to_vec())),
        Op::Token(TokenOp::AllTcTokenJids),
        Op::Token(TokenOp::GetTcTokens(ADDRESSES.to_vec())),
    ];
    for address in ADDRESSES {
        ops.push(Op::Signal(SignalOp::GetSenderKey(address)));
        ops.push(Op::Session(SessionOp::LoadIdentity(address)));
        ops.push(Op::Session(SessionOp::GetSession(address)));
        ops.push(Op::Session(SessionOp::HasSession(address)));
    }
    ops.extend(USERS.map(|user| Op::Session(SessionOp::HasSignalState(user))));
    ops.extend(SYNC_KEY_IDS.map(|id| Op::AppSync(AppSyncOp::GetSyncKey(id))));
    for collection in COLLECTIONS {
        ops.push(Op::AppSync(AppSyncOp::GetVersion(collection)));
        ops.push(Op::AppSync(AppSyncOp::GetMutationMacs(
            collection,
            MAC_INDEXES.to_vec(),
        )));
    }
    ops.extend(GROUPS.map(|group| Op::Protocol(ProtocolOp::GetSenderKeyDevices(group))));
    ops.extend(PHONES.map(|phone| Op::Protocol(ProtocolOp::GetPnMapping(phone))));
    for chat in CHATS {
        for id in MESSAGE_IDS {
            ops.push(Op::Token(TokenOp::GetSentMessage(chat, id)));
            for sender in SENDERS {
                ops.push(Op::Secret(SecretOp::GetStored(chat, sender, id)));
            }
        }
    }
    for address in ADDRESSES {
        for id in MESSAGE_IDS {
            for key in [1, 2] {
                ops.push(Op::Protocol(ProtocolOp::HasSameBaseKey(address, id, key)));
            }
        }
    }
    ops
}

fn show<T: Debug>(result: Result<T>) -> String {
    result.map_or_else(|_| "error".to_owned(), |value| format!("{value:?}"))
}

fn sorted<T: Ord>(mut items: Vec<T>) -> Vec<T> {
    items.sort();
    items
}

/// A batch read may repeat a key asked for twice, as the in-memory store does, or not, as ours doesn't
fn unique<T: Ord>(items: Vec<T>) -> Vec<T> {
    let mut items = sorted(items);
    items.dedup();
    items
}

#[derive(Debug)]
enum SignalOp {
    StorePrekey(u32, Vec<u8>, bool),
    StorePrekeys(Vec<(u32, Vec<u8>)>, bool),
    LoadPrekey(u32),
    LoadPrekeys(Vec<u32>),
    MarkPrekeysUploaded(Vec<u32>),
    RemovePrekey(u32),
    RemovePrekeys(Vec<u32>),
    MaxPrekeyId,
    StoreSignedPrekey(u32, Vec<u8>),
    LoadSignedPrekey(u32),
    LoadAllSignedPrekeys,
    RemoveSignedPrekey(u32),
    PutSenderKey(&'static str, Vec<u8>),
    PutSenderKeys(Vec<(&'static str, Vec<u8>)>),
    GetSenderKey(&'static str),
    DeleteSenderKey(&'static str),
    DeleteSenderKeys(Vec<&'static str>),
}

impl SignalOp {
    fn random(rng: &mut Rng) -> Self {
        match rng.below(17) {
            0 => Self::StorePrekey(rng.pick(&PREKEY_IDS), rng.value(), rng.flag()),
            1 => {
                let ids = rng.some(&PREKEY_IDS);
                Self::StorePrekeys(
                    ids.into_iter().map(|id| (id, rng.value())).collect(),
                    rng.flag(),
                )
            }
            2 => Self::LoadPrekey(rng.pick(&PREKEY_IDS)),
            3 => Self::LoadPrekeys(rng.some(&PREKEY_IDS)),
            4 => Self::MarkPrekeysUploaded(rng.some(&PREKEY_IDS)),
            5 => Self::RemovePrekey(rng.pick(&PREKEY_IDS)),
            6 => Self::RemovePrekeys(rng.some(&PREKEY_IDS)),
            7 => Self::MaxPrekeyId,
            8 => Self::StoreSignedPrekey(rng.pick(&SIGNED_PREKEY_IDS), rng.value()),
            9 => Self::LoadSignedPrekey(rng.pick(&SIGNED_PREKEY_IDS)),
            10 => Self::LoadAllSignedPrekeys,
            11 => Self::RemoveSignedPrekey(rng.pick(&SIGNED_PREKEY_IDS)),
            12 => Self::PutSenderKey(rng.pick(&ADDRESSES), rng.value()),
            13 => {
                let addresses = rng.some(&ADDRESSES);
                Self::PutSenderKeys(addresses.into_iter().map(|a| (a, rng.value())).collect())
            }
            14 => Self::GetSenderKey(rng.pick(&ADDRESSES)),
            15 => Self::DeleteSenderKey(rng.pick(&ADDRESSES)),
            _ => Self::DeleteSenderKeys(rng.some(&ADDRESSES)),
        }
    }

    async fn apply(&self, store: &impl Backend) -> String {
        match self {
            Self::StorePrekey(id, record, uploaded) => {
                show(store.store_prekey(*id, record, *uploaded).await)
            }
            Self::StorePrekeys(keys, uploaded) => {
                let keys: Vec<_> = keys
                    .iter()
                    .map(|(id, record)| (*id, Bytes::from(record.clone())))
                    .collect();
                show(store.store_prekeys_batch(&keys, *uploaded).await)
            }
            Self::LoadPrekey(id) => show(store.load_prekey(*id).await),
            Self::LoadPrekeys(ids) => show(store.load_prekeys_batch(ids).await.map(unique)),
            Self::MarkPrekeysUploaded(ids) => show(store.mark_prekeys_uploaded(ids).await),
            Self::RemovePrekey(id) => show(store.remove_prekey(*id).await),
            Self::RemovePrekeys(ids) => show(store.remove_prekeys_batch(ids).await),
            Self::MaxPrekeyId => show(store.get_max_prekey_id().await),
            Self::StoreSignedPrekey(id, record) => {
                show(store.store_signed_prekey(*id, record).await)
            }
            Self::LoadSignedPrekey(id) => show(store.load_signed_prekey(*id).await),
            Self::LoadAllSignedPrekeys => show(store.load_all_signed_prekeys().await.map(sorted)),
            Self::RemoveSignedPrekey(id) => show(store.remove_signed_prekey(*id).await),
            Self::PutSenderKey(address, record) => {
                show(store.put_sender_key(address, record).await)
            }
            Self::PutSenderKeys(keys) => {
                let keys: Vec<_> = keys
                    .iter()
                    .map(|(address, record)| (Arc::from(*address), Bytes::from(record.clone())))
                    .collect();
                show(store.put_sender_keys_batch(&keys).await)
            }
            Self::GetSenderKey(address) => show(store.get_sender_key(address).await),
            Self::DeleteSenderKey(address) => show(store.delete_sender_key(address).await),
            Self::DeleteSenderKeys(addresses) => {
                let addresses: Vec<Arc<str>> = addresses.iter().map(|a| Arc::from(*a)).collect();
                show(store.delete_sender_keys_batch(&addresses).await)
            }
        }
    }
}

#[derive(Debug)]
enum AppSyncOp {
    SetSyncKey(&'static [u8], AppStateSyncKey),
    GetSyncKey(&'static [u8]),
    LatestSyncKeyId,
    SetVersion(&'static str, u64, u8),
    GetVersion(&'static str),
    DeleteVersion(&'static str),
    PutMutationMacs(&'static str, u64, Vec<(u8, u8)>),
    GetMutationMac(&'static str, u8),
    GetMutationMacs(&'static str, Vec<u8>),
    DeleteMutationMacs(&'static str, Vec<u8>),
    ClearMutationMacs(&'static str),
    CommitPatch(&'static str, u64, Vec<u8>, Vec<(u8, u8)>),
}

impl AppSyncOp {
    fn random(rng: &mut Rng) -> Self {
        let collection = rng.pick(&COLLECTIONS);
        match rng.below(12) {
            0 => Self::SetSyncKey(
                rng.pick(&SYNC_KEY_IDS),
                AppStateSyncKey {
                    key_data: rng.value(),
                    fingerprint: rng.value(),
                    timestamp: rng.pick(&TIMES),
                },
            ),
            1 => Self::GetSyncKey(rng.pick(&SYNC_KEY_IDS)),
            2 => Self::LatestSyncKeyId,
            3 => Self::SetVersion(collection, rng.next_u64() % 4, rng.byte()),
            4 => Self::GetVersion(collection),
            5 => Self::DeleteVersion(collection),
            6 => Self::PutMutationMacs(collection, rng.next_u64() % 4, macs(rng)),
            7 => Self::GetMutationMac(collection, rng.pick(&MAC_INDEXES)),
            8 => Self::GetMutationMacs(collection, rng.some(&MAC_INDEXES)),
            9 => Self::DeleteMutationMacs(collection, rng.some(&MAC_INDEXES)),
            10 => Self::ClearMutationMacs(collection),
            _ => Self::CommitPatch(
                collection,
                rng.next_u64() % 4,
                rng.some(&MAC_INDEXES),
                macs(rng),
            ),
        }
    }

    async fn apply(&self, store: &impl Backend) -> String {
        match self {
            Self::SetSyncKey(id, key) => show(store.set_sync_key(id, key.clone()).await),
            Self::GetSyncKey(id) => show(store.get_sync_key(id).await),
            Self::LatestSyncKeyId => show(store.get_latest_sync_key_id().await),
            Self::SetVersion(name, version, hash) => {
                show(store.set_version(name, hash_state(*version, *hash)).await)
            }
            Self::GetVersion(name) => show(store.get_version(name).await),
            Self::DeleteVersion(name) => show(store.delete_version(name).await),
            Self::PutMutationMacs(name, version, added) => show(
                store
                    .put_mutation_macs(name, *version, &mutation_macs(added))
                    .await,
            ),
            Self::GetMutationMac(name, index) => {
                show(store.get_mutation_mac(name, &[*index; 32]).await)
            }
            Self::GetMutationMacs(name, indexes) => {
                let indexes: Vec<[u8; 32]> = indexes.iter().map(|index| [*index; 32]).collect();
                show(
                    store
                        .get_mutation_macs(name, &indexes)
                        .await
                        .map(|macs| sorted(macs.into_iter().collect())),
                )
            }
            Self::DeleteMutationMacs(name, indexes) => {
                show(store.delete_mutation_macs(name, &index_macs(indexes)).await)
            }
            Self::ClearMutationMacs(name) => show(store.clear_mutation_macs(name).await),
            Self::CommitPatch(name, version, removed, added) => show(
                store
                    .commit_patch(
                        name,
                        hash_state(*version, 9),
                        &index_macs(removed),
                        &mutation_macs(added),
                    )
                    .await,
            ),
        }
    }
}

fn macs(rng: &mut Rng) -> Vec<(u8, u8)> {
    let indexes = rng.some(&MAC_INDEXES);
    indexes
        .into_iter()
        .map(|index| (index, rng.byte()))
        .collect()
}

fn hash_state(version: u64, hash: u8) -> HashState {
    HashState {
        version,
        hash: [hash; 128],
        ..Default::default()
    }
}

fn mutation_macs(macs: &[(u8, u8)]) -> Vec<AppStateMutationMAC> {
    macs.iter()
        .map(|(index, value)| AppStateMutationMAC {
            index_mac: vec![*index; 32],
            value_mac: vec![*value; 32],
        })
        .collect()
}

fn index_macs(indexes: &[u8]) -> Vec<Vec<u8>> {
    indexes.iter().map(|index| vec![*index; 32]).collect()
}

#[derive(Debug)]
enum ProtocolOp {
    SetSenderKeyStatus(&'static str, Vec<(&'static str, bool)>),
    GetSenderKeyDevices(&'static str),
    ClearSenderKeyDevices(&'static str),
    DeleteSenderKeyDeviceRows(Vec<&'static str>),
    ClearAllSenderKeyDevices,
    PutLidMapping(LidPnMappingEntry),
    PutLidMappings(Vec<LidPnMappingEntry>),
    GetLidMapping(&'static str),
    GetPnMapping(&'static str),
    AllLidMappings,
    SaveBaseKey(&'static str, &'static str, u8),
    HasSameBaseKey(&'static str, &'static str, u8),
    DeleteBaseKey(&'static str, &'static str),
    DeleteExpiredBaseKeys(bool),
    UpdateDeviceList(DeviceListRecord),
    UpdateDeviceLists(Vec<DeviceListRecord>),
    GetDevices(&'static str),
    GetDevicesBatch(Vec<&'static str>),
    DeleteDevices(&'static str),
}

impl ProtocolOp {
    fn random(rng: &mut Rng, now: i64) -> Self {
        match rng.below(19) {
            0 => {
                let devices = rng.some(&ADDRESSES);
                Self::SetSenderKeyStatus(
                    rng.pick(&GROUPS),
                    devices.into_iter().map(|d| (d, rng.flag())).collect(),
                )
            }
            1 => Self::GetSenderKeyDevices(rng.pick(&GROUPS)),
            2 => Self::ClearSenderKeyDevices(rng.pick(&GROUPS)),
            3 => Self::DeleteSenderKeyDeviceRows(rng.some(&ADDRESSES)),
            4 => Self::ClearAllSenderKeyDevices,
            5 => Self::PutLidMapping(lid_mapping(rng, now)),
            6 => {
                let len = rng.below(4);
                Self::PutLidMappings((0..len).map(|_| lid_mapping(rng, now)).collect())
            }
            7 => Self::GetLidMapping(rng.pick(&LIDS).0),
            8 => Self::GetPnMapping(rng.pick(&PHONES)),
            9 => Self::AllLidMappings,
            10 => Self::SaveBaseKey(
                rng.pick(&ADDRESSES),
                rng.pick(&MESSAGE_IDS),
                rng.pick(&[1, 2]),
            ),
            11 => Self::HasSameBaseKey(
                rng.pick(&ADDRESSES),
                rng.pick(&MESSAGE_IDS),
                rng.pick(&[1, 2]),
            ),
            12 => Self::DeleteBaseKey(rng.pick(&ADDRESSES), rng.pick(&MESSAGE_IDS)),
            13 => Self::DeleteExpiredBaseKeys(rng.flag()),
            14 => Self::UpdateDeviceList(device_list(rng)),
            15 => {
                let len = rng.below(4);
                Self::UpdateDeviceLists((0..len).map(|_| device_list(rng)).collect())
            }
            16 => Self::GetDevices(rng.pick(&ADDRESSES)),
            17 => Self::GetDevicesBatch(rng.some(&ADDRESSES)),
            _ => Self::DeleteDevices(rng.pick(&ADDRESSES)),
        }
    }

    async fn apply(&self, store: &impl Backend) -> String {
        match self {
            Self::SetSenderKeyStatus(group, entries) => {
                show(store.set_sender_key_status(group, entries).await)
            }
            Self::GetSenderKeyDevices(group) => {
                show(store.get_sender_key_devices(group).await.map(sorted))
            }
            Self::ClearSenderKeyDevices(group) => show(store.clear_sender_key_devices(group).await),
            Self::DeleteSenderKeyDeviceRows(devices) => {
                show(store.delete_sender_key_device_rows(devices).await)
            }
            Self::ClearAllSenderKeyDevices => show(store.clear_all_sender_key_devices().await),
            Self::PutLidMapping(entry) => show(store.put_lid_mapping(entry).await),
            Self::PutLidMappings(entries) => show(store.put_lid_mappings(entries).await),
            Self::GetLidMapping(lid) => show(store.get_lid_mapping(lid).await),
            Self::GetPnMapping(phone) => show(store.get_pn_mapping(phone).await),
            Self::AllLidMappings => show(store.get_all_lid_mappings().await.map(|mut entries| {
                entries.sort_by(|a, b| a.lid.cmp(&b.lid));
                entries
            })),
            Self::SaveBaseKey(address, id, key) => {
                show(store.save_base_key(address, id, &[*key; 32]).await)
            }
            Self::HasSameBaseKey(address, id, key) => {
                show(store.has_same_base_key(address, id, &[*key; 32]).await)
            }
            Self::DeleteBaseKey(address, id) => show(store.delete_base_key(address, id).await),
            Self::DeleteExpiredBaseKeys(all) => {
                show(store.delete_expired_base_keys(far_cutoff(*all)).await)
            }
            Self::UpdateDeviceList(record) => show(store.update_device_list(record.clone()).await),
            Self::UpdateDeviceLists(records) => {
                show(store.update_device_lists(records.clone()).await)
            }
            Self::GetDevices(user) => show(store.get_devices(user).await),
            Self::GetDevicesBatch(users) => {
                show(store.get_devices_batch(users).await.map(|mut records| {
                    records.sort_by(|a, b| a.user.cmp(&b.user));
                    records.dedup_by(|a, b| a.user == b.user);
                    records
                }))
            }
            Self::DeleteDevices(user) => show(store.delete_devices(user).await),
        }
    }
}

fn lid_mapping(rng: &mut Rng, now: i64) -> LidPnMappingEntry {
    let (lid, phones) = rng.pick(&LIDS);
    LidPnMappingEntry {
        lid: lid.to_owned(),
        phone_number: rng.pick(&phones).to_owned(),
        // On a repeat, the in-memory store takes the new `created_at` while SQLite, like ours, keeps
        // the first; one fixed value keeps that known upstream disagreement out of the comparison
        created_at: 1,
        updated_at: now,
        learning_source: rng.pick(&["usync", "peer"]).to_owned(),
    }
}

fn device_list(rng: &mut Rng) -> DeviceListRecord {
    let ids = rng.some(&[0, 1, 7]);
    DeviceListRecord {
        user: rng.pick(&ADDRESSES).into(),
        devices: ids
            .into_iter()
            .map(|id| DeviceInfo::new(id, rng.flag().then_some(2)))
            .collect(),
        timestamp: rng.pick(&TIMES),
        phash: rng.flag().then(|| "2:abc".into()),
        raw_id: rng.flag().then_some(9),
    }
}

/// Rows stamped with the wall clock: a cutoff a day ahead removes them all, a day back none
fn far_cutoff(all: bool) -> i64 {
    let day = 86_400;
    if all {
        now_secs() + day
    } else {
        now_secs() - day
    }
}

#[derive(Debug)]
enum TokenOp {
    GetTcToken(&'static str),
    GetTcTokens(Vec<&'static str>),
    PutTcToken(&'static str, TcTokenEntry),
    DeleteTcToken(&'static str),
    AllTcTokenJids,
    DeleteExpiredTcTokens(i64, i64),
    TouchSenderTimestamp(&'static str, i64),
    StoreReceived(&'static str, Vec<u8>, i64),
    StoreSentMessage(&'static str, &'static str, Vec<u8>),
    GetSentMessage(&'static str, &'static str),
    TakeSentMessage(&'static str, &'static str),
    DeleteExpiredSentMessages(bool),
}

impl TokenOp {
    fn random(rng: &mut Rng) -> Self {
        match rng.below(12) {
            0 => Self::GetTcToken(rng.pick(&ADDRESSES)),
            1 => Self::GetTcTokens(rng.some(&ADDRESSES)),
            2 => Self::PutTcToken(
                rng.pick(&ADDRESSES),
                TcTokenEntry {
                    token: rng.value(),
                    token_timestamp: rng.pick(&TIMES),
                    sender_timestamp: rng.flag().then(|| rng.pick(&TIMES)),
                },
            ),
            3 => Self::DeleteTcToken(rng.pick(&ADDRESSES)),
            4 => Self::AllTcTokenJids,
            5 => Self::DeleteExpiredTcTokens(rng.pick(&TIMES), rng.pick(&TIMES)),
            6 => Self::TouchSenderTimestamp(rng.pick(&ADDRESSES), rng.pick(&TIMES)),
            7 => Self::StoreReceived(rng.pick(&ADDRESSES), rng.value(), rng.pick(&TIMES)),
            8 => Self::StoreSentMessage(rng.pick(&CHATS), rng.pick(&MESSAGE_IDS), rng.value()),
            9 => Self::GetSentMessage(rng.pick(&CHATS), rng.pick(&MESSAGE_IDS)),
            10 => Self::TakeSentMessage(rng.pick(&CHATS), rng.pick(&MESSAGE_IDS)),
            _ => Self::DeleteExpiredSentMessages(rng.flag()),
        }
    }

    async fn apply(&self, store: &impl Backend) -> String {
        match self {
            Self::GetTcToken(jid) => show(store.get_tc_token(jid).await),
            Self::GetTcTokens(jids) => {
                let jids: Vec<String> = jids.iter().map(|jid| (*jid).to_owned()).collect();
                show(store.get_tc_tokens(&jids).await)
            }
            Self::PutTcToken(jid, entry) => show(store.put_tc_token(jid, entry).await),
            Self::DeleteTcToken(jid) => show(store.delete_tc_token(jid).await),
            Self::AllTcTokenJids => show(store.get_all_tc_token_jids().await.map(sorted)),
            Self::DeleteExpiredTcTokens(token_cutoff, sender_cutoff) => show(
                store
                    .delete_expired_tc_tokens(*token_cutoff, *sender_cutoff)
                    .await,
            ),
            Self::TouchSenderTimestamp(jid, timestamp) => {
                show(store.touch_tc_token_sender_timestamp(jid, *timestamp).await)
            }
            Self::StoreReceived(jid, token, timestamp) => {
                show(store.store_received_tc_token(jid, token, *timestamp).await)
            }
            Self::StoreSentMessage(chat, id, payload) => {
                show(store.store_sent_message(chat, id, payload).await)
            }
            Self::GetSentMessage(chat, id) => show(store.get_sent_message(chat, id).await),
            Self::TakeSentMessage(chat, id) => show(store.take_sent_message(chat, id).await),
            Self::DeleteExpiredSentMessages(all) => {
                show(store.delete_expired_sent_messages(far_cutoff(*all)).await)
            }
        }
    }
}

#[derive(Debug)]
enum SecretOp {
    Put(Vec<MsgSecretEntry>),
    PutOne(&'static str, &'static str, &'static str, u8),
    GetStored(&'static str, &'static str, &'static str),
    Get(&'static str, &'static str, &'static str),
    DeleteExpired(i64),
}

impl SecretOp {
    fn random(rng: &mut Rng) -> Self {
        match rng.below(5) {
            0 => {
                let len = rng.below(4);
                Self::Put((0..len).map(|_| secret_entry(rng)).collect())
            }
            1 => Self::PutOne(
                rng.pick(&CHATS),
                rng.pick(&SENDERS),
                rng.pick(&MESSAGE_IDS),
                rng.byte(),
            ),
            2 => Self::GetStored(rng.pick(&CHATS), rng.pick(&SENDERS), rng.pick(&MESSAGE_IDS)),
            3 => Self::Get(rng.pick(&CHATS), rng.pick(&SENDERS), rng.pick(&MESSAGE_IDS)),
            _ => Self::DeleteExpired(rng.pick(&[100, 150, 200, 300])),
        }
    }

    async fn apply(&self, store: &impl Backend) -> String {
        match self {
            Self::Put(entries) => show(store.put_msg_secrets(entries.clone()).await),
            Self::PutOne(chat, sender, id, secret) => {
                show(store.put_msg_secret(chat, sender, id, &[*secret; 32]).await)
            }
            Self::GetStored(chat, sender, id) => {
                show(store.get_stored_msg_secret(chat, sender, id).await)
            }
            Self::Get(chat, sender, id) => show(store.get_msg_secret(chat, sender, id).await),
            Self::DeleteExpired(cutoff) => show(store.delete_expired_msg_secrets(*cutoff).await),
        }
    }
}

fn secret_entry(rng: &mut Rng) -> MsgSecretEntry {
    MsgSecretEntry {
        chat: rng.pick(&CHATS).into(),
        sender: rng.pick(&SENDERS).into(),
        msg_id: rng.pick(&MESSAGE_IDS).into(),
        secret: [rng.byte(); 32],
        expires_at: rng.pick(&TIMES),
        message_ts: rng.pick(&[0, 10, 20]),
    }
}

#[derive(Debug)]
enum SessionOp {
    PutIdentity(&'static str, u8),
    PutIdentities(Vec<(&'static str, u8)>),
    LoadIdentity(&'static str),
    DeleteIdentity(&'static str),
    DeleteIdentities(Vec<&'static str>),
    PutSession(&'static str, Vec<u8>),
    PutSessions(Vec<(&'static str, Vec<u8>)>),
    GetSession(&'static str),
    GetSessions(Vec<&'static str>),
    DeleteSession(&'static str),
    DeleteSessions(Vec<&'static str>),
    HasSession(&'static str),
    HasSignalState(&'static str),
}

impl SessionOp {
    fn random(rng: &mut Rng) -> Self {
        match rng.below(13) {
            0 => Self::PutIdentity(rng.pick(&ADDRESSES), rng.byte()),
            1 => {
                let addresses = rng.some(&ADDRESSES);
                Self::PutIdentities(addresses.into_iter().map(|a| (a, rng.byte())).collect())
            }
            2 => Self::LoadIdentity(rng.pick(&ADDRESSES)),
            3 => Self::DeleteIdentity(rng.pick(&ADDRESSES)),
            4 => Self::DeleteIdentities(rng.some(&ADDRESSES)),
            5 => Self::PutSession(rng.pick(&ADDRESSES), rng.value()),
            6 => {
                let addresses = rng.some(&ADDRESSES);
                Self::PutSessions(addresses.into_iter().map(|a| (a, rng.value())).collect())
            }
            7 => Self::GetSession(rng.pick(&ADDRESSES)),
            8 => Self::GetSessions(rng.some(&ADDRESSES)),
            9 => Self::DeleteSession(rng.pick(&ADDRESSES)),
            10 => Self::DeleteSessions(rng.some(&ADDRESSES)),
            11 => Self::HasSession(rng.pick(&ADDRESSES)),
            _ => Self::HasSignalState(rng.pick(&USERS)),
        }
    }

    async fn apply(&self, store: &impl Backend) -> String {
        match self {
            Self::PutIdentity(address, key) => show(store.put_identity(address, [*key; 32]).await),
            Self::PutIdentities(identities) => {
                let identities: Vec<_> = identities
                    .iter()
                    .map(|(address, key)| (Arc::from(*address), [*key; 32]))
                    .collect();
                show(store.put_identities_batch(&identities).await)
            }
            Self::LoadIdentity(address) => show(store.load_identity(address).await),
            Self::DeleteIdentity(address) => show(store.delete_identity(address).await),
            Self::DeleteIdentities(addresses) => {
                show(store.delete_identities_batch(&shared(addresses)).await)
            }
            Self::PutSession(address, session) => show(store.put_session(address, session).await),
            Self::PutSessions(sessions) => {
                let sessions: Vec<_> = sessions
                    .iter()
                    .map(|(address, session)| (Arc::from(*address), Bytes::from(session.clone())))
                    .collect();
                show(store.put_sessions_batch(&sessions).await)
            }
            Self::GetSession(address) => show(store.get_session(address).await),
            Self::GetSessions(addresses) => show(
                store
                    .get_sessions_batch(&shared(addresses))
                    .await
                    .map(unique),
            ),
            Self::DeleteSession(address) => show(store.delete_session(address).await),
            Self::DeleteSessions(addresses) => {
                show(store.delete_sessions_batch(&shared(addresses)).await)
            }
            Self::HasSession(address) => show(store.has_session(address).await),
            Self::HasSignalState(user) => show(store.has_signal_state_for_user(user).await),
        }
    }
}

fn shared(addresses: &[&'static str]) -> Vec<Arc<str>> {
    addresses
        .iter()
        .map(|address| Arc::from(*address))
        .collect()
}

/// `Device::new()` makes fresh keys, so both stores get copies of one device, never two new ones.
/// That also keeps `create` out of the mix.
static DEVICE: LazyLock<Device> = LazyLock::new(Device::new);

#[derive(Debug)]
enum DeviceOp {
    Save(u8),
    Load,
    Exists,
}

impl DeviceOp {
    fn random(rng: &mut Rng) -> Self {
        match rng.below(3) {
            0 => Self::Save(rng.byte()),
            1 => Self::Load,
            _ => Self::Exists,
        }
    }

    async fn apply(&self, store: &impl Backend) -> String {
        match self {
            Self::Save(version) => {
                let mut device = DEVICE.clone();
                device.push_name = format!("device {version}");
                device.login_counter = i32::from(*version);
                show(store.save(&device).await)
            }
            // The in-memory store keeps the runtime-only fields that ours resets, so compare what's saved
            Self::Load => show(store.load().await.and_then(|device| {
                device
                    .map(|device| {
                        encode(&device).map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                    })
                    .transpose()
            })),
            Self::Exists => show(store.exists().await),
        }
    }
}
