//! Probe of whatsapp-rust against a real phone, on our Postgres store, before the real integration
//! exists.
//!
//! Prints only `spike:` lines on stdout: event names, counts and outcomes, never message content,
//! contacts or group names. Library logs go to stderr, filtered by `RUST_LOG` (default `warn`).
//!
//! - `WA_SPIKE_EMAIL=<email>` names the dev user whose account the device lives under, default
//!   `whatsapp-spike@localhost`; the user and the account row are created on first use. The device
//!   survives restarts, so a linked one reconnects without pairing.
//! - `WA_SPIKE_PHONE=<digits>` also requests a pair code for that number while the device is
//!   unlinked; the QR code works either way, and whichever the phone completes first wins.
//! - `WA_SPIKE_DIR=<dir>` receives `qr.svg` and a self-refreshing `qr.html`, and is watched for two
//!   files: `stop` (like Ctrl-C or SIGTERM) disconnects and keeps the link, and `logout` unlinks the
//!   device and deletes the account row.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};

use qrcode::{QrCode, render::svg};
use server::{config::Config, crypto::Cipher, whatsapp::store::PgStore};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::signal::unix::{SignalKind, signal};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;
use whatsapp_rust::{
    CacheConfig, MsgSecretPolicy, PresencePolicy,
    bot::BotHandle,
    pair_code::PairCodeOptions,
    prelude::*,
    wacore::store::{DevicePropsOverride, traits::DeviceStore},
};

const MAX_PAIR_CODES: u32 = 3;

#[derive(Clone)]
struct Probe {
    started: Instant,
    phone: Option<String>,
    dir: PathBuf,
    pair_codes: Arc<AtomicU32>,
    checked: Arc<AtomicBool>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stop {
    Keep(&'static str),
    Logout,
}

fn say(line: &str) {
    println!("spike: {line}");
}

/// Last three digits only, enough to tell numbers apart in a shared log
fn masked(user: &str) -> String {
    let tail: String = user
        .chars()
        .rev()
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("…{tail}")
}

#[tokio::main]
#[expect(
    clippy::significant_drop_tightening,
    reason = "fires on a temporary inside tokio::select!'s expansion"
)]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    dotenvy::dotenv().ok();
    let config = Config::from_env()?;
    let email =
        std::env::var("WA_SPIKE_EMAIL").unwrap_or_else(|_| "whatsapp-spike@localhost".into());
    let dir = PathBuf::from(std::env::var("WA_SPIKE_DIR").unwrap_or_else(|_| ".".into()));
    let phone = std::env::var("WA_SPIKE_PHONE").ok().map(|number| {
        number
            .chars()
            .filter(char::is_ascii_digit)
            .collect::<String>()
    });
    for leftover in ["stop", "logout"] {
        let _ = std::fs::remove_file(dir.join(leftover));
    }

    let pool = PgPoolOptions::new().connect(&config.database_url).await?;
    let account_id = account(&pool, &email).await?;
    let store = PgStore::new(
        pool.clone(),
        Cipher::new(&config.encryption_key),
        account_id,
    );
    let linked = store
        .load()
        .await?
        .is_some_and(|device| device.pn.is_some());
    say(if linked {
        "account ready, device linked"
    } else {
        "account ready, device not linked yet"
    });

    let probe = Probe {
        started: Instant::now(),
        phone: phone.clone(),
        dir: dir.clone(),
        pair_codes: Arc::new(AtomicU32::new(0)),
        checked: Arc::new(AtomicBool::new(false)),
    };
    std::fs::write(
        dir.join("qr.html"),
        r#"<!doctype html><meta http-equiv="refresh" content="2"><body style="display:grid;place-items:center;height:100vh;margin:0;background:#fff"><img src="qr.svg" width="320" height="320" alt="QR code">"#,
    )?;

    let mut cache = CacheConfig::default();
    cache.msg_secret_policy = MsgSecretPolicy::Disabled;
    let mut builder = Bot::builder()
        .with_backend(store)
        .with_device_props(
            DevicePropsOverride::new()
                .with_os("Whatomato")
                .with_platform_type(wa::device_props::PlatformType::CHROME),
        )
        .with_presence_policy(PresencePolicy::Manual)
        .skip_history_sync()
        .with_cache_config(cache)
        .on_event_for(&WATCHED, {
            let probe = probe.clone();
            move |event, client| {
                let probe = probe.clone();
                async move { probe.on_event(&event, &client).await }
            }
        });
    if let Some(phone) = phone.as_deref().filter(|_| !linked) {
        builder = builder.with_pair_code(pair_code_options(phone));
    }

    let mut handle = builder.build().await?.spawn();
    let client = handle.client();
    say("started");

    let stop = tokio::select! {
        outcome = &mut handle => {
            // A code left on the page after its client is gone can only fail to link
            let _ = std::fs::remove_file(dir.join("qr.svg"));
            say(&format!("bot finished on its own: {outcome:?}"));
            return Ok(());
        }
        stop = stop_requested(&dir) => stop?,
    };
    finish(handle, client, stop, &pool, account_id).await?;
    pool.close().await;
    Ok(())
}

/// Stops the bot after flushing its state, and with `Stop::Logout` unlinks it and deletes the row
async fn finish(
    handle: BotHandle,
    client: Arc<Client>,
    stop: Stop,
    pool: &PgPool,
    account_id: Uuid,
) -> anyhow::Result<()> {
    // Taken before the shutdown, so the wait below sees every owner of the store let go
    let released = client.store_release();
    if stop == Stop::Logout {
        let report = client.logout().await;
        say(&format!(
            "logout deregistration: {:?}",
            report.deregistration
        ));
    }
    drop(client);
    let report = handle.shutdown().await;
    released.wait().await;
    say(&format!(
        "stopped ({}): device flush {}, inbound {:?}, outbound {:?}",
        match stop {
            Stop::Keep(reason) => reason,
            Stop::Logout => "logout file",
        },
        if report.shutdown.device.is_ok() {
            "ok"
        } else {
            "failed"
        },
        report.shutdown.inbound,
        report.shutdown.outbound,
    ));

    if stop == Stop::Logout {
        sqlx::query("DELETE FROM whatsapp_accounts WHERE id = $1")
            .bind(account_id)
            .execute(pool)
            .await?;
        say("account row deleted");
    }
    Ok(())
}

/// The account row of the dev user `email`, both created on first use
async fn account(pool: &PgPool, email: &str) -> anyhow::Result<Uuid> {
    // No-op updates, because DO NOTHING returns no row on a conflict. The password isn't a hash, so
    // nobody can sign in as a user made here.
    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, password) VALUES ($1, 'unusable') \
         ON CONFLICT (email) DO UPDATE SET email = EXCLUDED.email RETURNING id",
    )
    .bind(email)
    .fetch_one(pool)
    .await?;
    let account_id = sqlx::query_scalar(
        "INSERT INTO whatsapp_accounts (user_id) VALUES ($1) \
         ON CONFLICT (user_id) DO UPDATE SET user_id = EXCLUDED.user_id RETURNING id",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(account_id)
}

async fn stop_requested(dir: &Path) -> anyhow::Result<Stop> {
    let mut terminate = signal(SignalKind::terminate())?;
    let (stop_file, logout_file) = (dir.join("stop"), dir.join("logout"));
    Ok(tokio::select! {
        _ = tokio::signal::ctrl_c() => Stop::Keep("ctrl-c"),
        _ = terminate.recv() => Stop::Keep("sigterm"),
        () = wait_for(&stop_file) => Stop::Keep("stop file"),
        () = wait_for(&logout_file) => Stop::Logout,
    })
}

const WATCHED: [EventKind; 14] = [
    EventKind::PairingCode,
    EventKind::PairingCodeRefresh,
    EventKind::PairingCodeError,
    EventKind::PairingQrCode,
    EventKind::PairingQrCodesExhausted,
    EventKind::PairSuccess,
    EventKind::PairError,
    EventKind::Connected,
    EventKind::Disconnected,
    EventKind::LoggedOut,
    EventKind::StreamReplaced,
    EventKind::TemporaryBan,
    EventKind::ClientOutdated,
    EventKind::ConnectFailure,
];

fn pair_code_options(phone: &str) -> PairCodeOptions {
    PairCodeOptions {
        phone_number: phone.to_owned(),
        ..Default::default()
    }
}

async fn wait_for(path: &Path) {
    while !path.exists() {
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

impl Probe {
    fn elapsed(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    async fn on_event(&self, event: &Event, client: &Arc<Client>) {
        match event {
            Event::PairingCode(code) => {
                self.pair_codes.fetch_add(1, Ordering::Relaxed);
                say(&format!(
                    "PAIR CODE {} (valid {}s)",
                    code.code,
                    code.timeout.as_secs()
                ));
            }
            Event::PairingCodeRefresh(_) => self.refresh_pair_code(client).await,
            Event::PairingQrCode(qr) => self.write_qr(&qr.code, qr.timeout),
            Event::PairSuccess(paired) => say(&format!(
                "paired at {}s as {} on {:?}",
                self.elapsed(),
                masked(&paired.id.user),
                paired.platform
            )),
            Event::Connected(_) if !self.checked.swap(true, Ordering::Relaxed) => {
                say(&format!("connected at {}s", self.elapsed()));
                tokio::spawn(checks(Arc::clone(client)));
            }
            Event::Disconnected(gone) => say(&format!("disconnected: {:?}", gone.reason)),
            other => say(&format!("{:?} at {}s", other.kind(), self.elapsed())),
        }
    }

    async fn refresh_pair_code(&self, client: &Arc<Client>) {
        let Some(phone) = &self.phone else {
            return;
        };
        if self.pair_codes.load(Ordering::Relaxed) >= MAX_PAIR_CODES {
            say("pair code expired; giving up after three codes");
            return;
        }
        say("pair code expired; requesting another");
        if let Err(error) = client.pair_with_code(pair_code_options(phone)).await {
            say(&format!("pair code request failed: {error}"));
        }
    }

    fn write_qr(&self, code: &str, timeout: Duration) {
        let written = QrCode::new(code)
            .map(|qr| qr.render::<svg::Color>().min_dimensions(320, 320).build())
            .map_err(anyhow::Error::from)
            .and_then(|svg| Ok(std::fs::write(self.dir.join("qr.svg"), svg)?));
        match written {
            Ok(()) => say(&format!("QR code written (valid {}s)", timeout.as_secs())),
            Err(error) => say(&format!("QR code not written: {error}")),
        }
    }
}

async fn checks(client: Arc<Client>) {
    let Some(own) = client.pn() else {
        say("no own number after connecting");
        return;
    };
    say(&format!("own number {}", masked(&own.user)));

    match client.groups().list_participating().await {
        Ok(groups) => say(&format!("groups: {}", groups.len())),
        Err(error) => say(&format!("groups failed: {error}")),
    }

    match client
        .send_text(
            Jid::pn(own.user.clone()),
            "Whatomato test: sent through the Postgres store. You can delete this.",
        )
        .await
    {
        Ok(sent) => say(&format!("sent to self: {}", sent.message_id)),
        Err(error) => say(&format!("send to self failed: {error:?}")),
    }
    say("checks done; leave it connected while you look at the phone");
}
