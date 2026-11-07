//! The account's address book, mirrored from the `contact` records the phone syncs to its linked
//! devices, so a message can go to a name rather than a number

use std::{collections::HashSet, sync::Arc};

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tokio::sync::mpsc;
use uuid::Uuid;
use whatsapp_rust::{
    WAPatchName,
    prelude::{Client, Event as LibraryEvent, EventHandler, EventInterest, EventKind, Jid, wa},
};

use crate::{crypto::Cipher, db::whatsapp_contacts};

/// Changes applied in one transaction; a first import brings the whole book at once
const BATCH: usize = 256;
/// Contacts live in this app-state collection, beside blocks and a few settings
const COLLECTION: WAPatchName = WAPatchName::CriticalUnblockLow;

/// What the phone saved for a contact. Sealed, since it's other people's details.
#[derive(Serialize, Deserialize)]
pub struct Names {
    pub full_name: Option<String>,
    pub first_name: Option<String>,
    pub username: Option<String>,
}

pub(super) enum Change {
    Saved {
        jid: String,
        phone_number: Option<String>,
        lid: Option<String>,
        names: Names,
    },
    Removed {
        jid: String,
    },
    /// A replay of the whole book comes next
    Cleared,
    /// The replay has been handed over in full
    Imported,
}

pub(super) type Changes = mpsc::UnboundedSender<(Uuid, Change)>;

/// Hands the library's contact events over as they're raised. It runs inline on the dispatch path,
/// which keeps them in order and, unlike the callback mailbox, never drops one when a sync floods.
pub(super) struct Feed {
    account_id: Uuid,
    changes: Changes,
}

impl Feed {
    pub(super) const fn new(account_id: Uuid, changes: Changes) -> Self {
        Self {
            account_id,
            changes,
        }
    }
}

impl EventHandler for Feed {
    fn handle_event(&self, event: Arc<LibraryEvent>) {
        let change = match &*event {
            LibraryEvent::ContactUpdate(update) => saved(&update.jid, &update.action),
            LibraryEvent::ContactRemoved(removed) => Change::Removed {
                jid: removed.jid.to_string(),
            },
            _ => return,
        };
        // Fails only once the writer has gone, at shutdown
        let _ = self.changes.send((self.account_id, change));
    }

    fn interest(&self) -> EventInterest {
        EventInterest::of(&[EventKind::ContactUpdate, EventKind::ContactRemoved])
    }
}

fn saved(jid: &Jid, action: &wa::sync_action_value::ContactAction) -> Change {
    let number = |jid: &Jid| jid.server.is_pn_family().then(|| jid.user.to_string());
    let lid = |jid: &Jid| jid.server.is_lid_family().then(|| jid.user.to_string());
    let other = |field: &Option<String>| field.as_deref().and_then(|jid| jid.parse::<Jid>().ok());
    let (pn_jid, lid_jid) = (other(&action.pn_jid), other(&action.lid_jid));
    Change::Saved {
        jid: jid.to_string(),
        phone_number: number(jid).or_else(|| pn_jid.as_ref().and_then(number)),
        lid: lid(jid).or_else(|| lid_jid.as_ref().and_then(lid)),
        names: Names {
            full_name: filled(action.full_name.as_deref()),
            first_name: filled(action.first_name.as_deref()),
            username: filled(action.username.as_deref()),
        },
    }
}

fn filled(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

pub(super) fn spawn_writer(pool: PgPool, cipher: Cipher) -> Changes {
    let (changes, incoming) = mpsc::unbounded_channel();
    tokio::spawn(write(pool, cipher, incoming));
    changes
}

/// Writes changes in the order they came, a run of one account's changes per transaction, until
/// every sender has gone
async fn write(
    pool: PgPool,
    cipher: Cipher,
    mut incoming: mpsc::UnboundedReceiver<(Uuid, Change)>,
) {
    // Accounts with a change lost since their last replay began; their import stays unfinished
    let mut gaps = HashSet::new();
    let mut batch = Vec::with_capacity(BATCH);
    while incoming.recv_many(&mut batch, BATCH).await > 0 {
        for run in batch.chunk_by(|a, b| a.0 == b.0) {
            let Some(&(account_id, _)) = run.first() else {
                continue;
            };
            let mut has_gap = gaps.contains(&account_id);
            if let Err(error) = apply(&pool, &cipher, account_id, run, &mut has_gap).await {
                tracing::warn!(%account_id, "couldn't save WhatsApp contacts: {error:#}");
                has_gap = true;
                let _ = whatsapp_contacts::forget_import(&pool, account_id).await;
            }
            if has_gap {
                gaps.insert(account_id);
            } else {
                gaps.remove(&account_id);
            }
        }
        batch.clear();
    }
}

async fn apply(
    pool: &PgPool,
    cipher: &Cipher,
    account_id: Uuid,
    run: &[(Uuid, Change)],
    has_gap: &mut bool,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    for (_, change) in run {
        match change {
            Change::Saved {
                jid,
                phone_number,
                lid,
                names,
            } => {
                let sealed = seal_names(cipher, account_id, jid, names)?;
                whatsapp_contacts::save(
                    &mut *tx,
                    account_id,
                    jid,
                    phone_number.as_deref(),
                    lid.as_deref(),
                    &sealed,
                )
                .await?;
            }
            Change::Removed { jid } => whatsapp_contacts::remove(&mut *tx, account_id, jid).await?,
            Change::Cleared => {
                whatsapp_contacts::clear(&mut *tx, account_id).await?;
                *has_gap = false;
            }
            // A replay with a hole in it isn't an import; the next connect replays again
            Change::Imported if *has_gap => {}
            Change::Imported => whatsapp_contacts::mark_imported(&mut *tx, account_id).await?,
        }
    }
    tx.commit().await?;
    Ok(())
}

/// Replays the book once per account: for one linked before contacts were kept, or whose last
/// import didn't finish. Later changes arrive on their own as the phone makes them.
pub(super) async fn import_once(
    client: Arc<Client>,
    pool: PgPool,
    account_id: Uuid,
    changes: Changes,
) {
    match whatsapp_contacts::needs_import(&pool, account_id).await {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            tracing::warn!(%account_id, "couldn't check the WhatsApp contacts import: {error}");
            return;
        }
    }
    // A snapshot carries what the phone holds now and nothing of what it deleted, so the old copy
    // goes first
    let _ = changes.send((account_id, Change::Cleared));
    match client.resync_app_state_collection(COLLECTION).await {
        Ok(report) if report.synced.contains(&COLLECTION) => {
            let _ = changes.send((account_id, Change::Imported));
        }
        Ok(_) => {
            tracing::warn!(%account_id, "WhatsApp didn't replay the contacts; next connect retries");
        }
        Err(error) => {
            tracing::warn!(%account_id, "WhatsApp contacts import failed: {error}");
        }
    }
}

/// # Errors
/// The system random number generator failed.
pub fn seal_names(
    cipher: &Cipher,
    account_id: Uuid,
    jid: &str,
    names: &Names,
) -> anyhow::Result<Vec<u8>> {
    cipher.encrypt(&serde_json::to_vec(names)?, &context(account_id, jid))
}

/// # Errors
/// The names didn't open: another key, another row, or altered.
pub fn open_names(
    cipher: &Cipher,
    account_id: Uuid,
    jid: &str,
    sealed: &[u8],
) -> anyhow::Result<Names> {
    let names = cipher.decrypt(sealed, &context(account_id, jid))?;
    Ok(serde_json::from_slice(&names)?)
}

/// Binds the names to their account and contact, the way the store binds its own sealed values
fn context(account_id: Uuid, jid: &str) -> Vec<u8> {
    [
        b"whatsapp:".as_slice(),
        account_id.as_bytes(),
        b"contacts:",
        jid.as_bytes(),
    ]
    .concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(full_name: &str, pn_jid: Option<&str>) -> wa::sync_action_value::ContactAction {
        let mut action = wa::sync_action_value::ContactAction::default();
        action.full_name = Some(full_name.to_owned());
        action.first_name = Some("  ".to_owned());
        action.pn_jid = pn_jid.map(str::to_owned);
        action
    }

    fn jid(value: &str) -> Jid {
        value.parse().unwrap()
    }

    #[test]
    fn a_contact_takes_its_number_from_its_jid_or_else_from_its_pn_jid() {
        let Change::Saved {
            phone_number,
            lid,
            names,
            ..
        } = saved(&jid("447700900123@s.whatsapp.net"), &action("Ada", None))
        else {
            panic!("not saved");
        };
        assert_eq!(phone_number.as_deref(), Some("447700900123"));
        assert_eq!(lid, None);
        assert_eq!(names.full_name.as_deref(), Some("Ada"));
        assert_eq!(names.first_name, None, "a blank name counts as none");

        let Change::Saved {
            phone_number, lid, ..
        } = saved(
            &jid("100000012345678@lid"),
            &action("Grace", Some("15550001@s.whatsapp.net")),
        )
        else {
            panic!("not saved");
        };
        assert_eq!(phone_number.as_deref(), Some("15550001"));
        assert_eq!(lid.as_deref(), Some("100000012345678"));

        let Change::Saved { phone_number, .. } =
            saved(&jid("100000012345679@lid"), &action("Linus", None))
        else {
            panic!("not saved");
        };
        assert_eq!(phone_number, None);
    }

    async fn account(pool: &PgPool) -> Uuid {
        let user_id = sqlx::query_scalar!(
            "INSERT INTO users (email, password) VALUES ('ada@example.com', 'unused') RETURNING id"
        )
        .fetch_one(pool)
        .await
        .unwrap();
        crate::db::whatsapp_accounts::find_or_create(pool, user_id)
            .await
            .unwrap()
            .id
    }

    fn save(name: &str, number: &str) -> Change {
        saved(
            &jid(&format!("{number}@s.whatsapp.net")),
            &action(name, None),
        )
    }

    async fn names_in(pool: &PgPool, cipher: &Cipher, account_id: Uuid) -> Vec<String> {
        let rows = sqlx::query!(
            "SELECT jid, names FROM whatsapp_contacts WHERE account_id = $1",
            account_id
        )
        .fetch_all(pool)
        .await
        .unwrap();
        let mut names: Vec<_> = rows
            .into_iter()
            .map(|row| {
                open_names(cipher, account_id, &row.jid, &row.names)
                    .unwrap()
                    .full_name
                    .unwrap()
            })
            .collect();
        names.sort();
        names
    }

    async fn run(pool: &PgPool, cipher: &Cipher, changes: Vec<(Uuid, Change)>) {
        let (sender, incoming) = mpsc::unbounded_channel();
        for change in changes {
            sender.send(change).unwrap();
        }
        drop(sender);
        write(pool.clone(), cipher.clone(), incoming).await;
    }

    #[sqlx::test]
    async fn a_replay_replaces_the_book_and_then_counts_as_imported(pool: PgPool) {
        let cipher = Cipher::new(&[7; 32]);
        let account_id = account(&pool).await;
        run(
            &pool,
            &cipher,
            vec![
                (account_id, save("Ada", "15550001")),
                (account_id, save("Grace", "15550002")),
                (account_id, save("Ada Lovelace", "15550001")),
                (
                    account_id,
                    Change::Removed {
                        jid: "15550002@s.whatsapp.net".into(),
                    },
                ),
            ],
        )
        .await;
        assert_eq!(names_in(&pool, &cipher, account_id).await, ["Ada Lovelace"]);
        assert!(
            whatsapp_contacts::needs_import(&pool, account_id)
                .await
                .unwrap()
        );

        run(
            &pool,
            &cipher,
            vec![
                (account_id, Change::Cleared),
                (account_id, save("Linus", "15550003")),
                (account_id, Change::Imported),
            ],
        )
        .await;
        assert_eq!(names_in(&pool, &cipher, account_id).await, ["Linus"]);
        assert!(
            !whatsapp_contacts::needs_import(&pool, account_id)
                .await
                .unwrap()
        );
    }

    #[sqlx::test]
    async fn a_lost_change_holds_the_import_back_until_the_next_replay(pool: PgPool) {
        let cipher = Cipher::new(&[7; 32]);
        let account_id = account(&pool).await;
        let other = Uuid::new_v4();
        // Postgres refuses a NUL in text, which fails the whole run, the clear included
        let broken = Change::Removed {
            jid: "15550009\0@s.whatsapp.net".into(),
        };
        run(
            &pool,
            &cipher,
            vec![
                (account_id, save("Ada", "15550001")),
                (account_id, Change::Cleared),
                (account_id, broken),
                // Another account's change between them makes the rest a run of its own
                (other, Change::Imported),
                (account_id, save("Linus", "15550003")),
                (account_id, Change::Imported),
                (account_id, Change::Cleared),
                (account_id, save("Grace", "15550002")),
            ],
        )
        .await;
        assert_eq!(
            names_in(&pool, &cipher, account_id).await,
            ["Grace"],
            "a new replay starts clean"
        );
        assert!(
            whatsapp_contacts::needs_import(&pool, account_id)
                .await
                .unwrap()
        );

        run(
            &pool,
            &cipher,
            vec![
                (account_id, Change::Cleared),
                (account_id, save("Grace", "15550002")),
                (account_id, Change::Imported),
            ],
        )
        .await;
        assert!(
            !whatsapp_contacts::needs_import(&pool, account_id)
                .await
                .unwrap()
        );
    }

    #[test]
    fn names_open_only_in_their_own_row() {
        let cipher = Cipher::new(&[7; 32]);
        let account_id = Uuid::new_v4();
        let names = Names {
            full_name: Some("Ada".into()),
            first_name: None,
            username: None,
        };
        let sealed = seal_names(&cipher, account_id, "15550001@s.whatsapp.net", &names).unwrap();
        assert!(open_names(&cipher, account_id, "15550001@s.whatsapp.net", &sealed).is_ok());
        assert!(open_names(&cipher, account_id, "15550002@s.whatsapp.net", &sealed).is_err());
        assert!(open_names(&cipher, Uuid::new_v4(), "15550001@s.whatsapp.net", &sealed).is_err());
    }
}
