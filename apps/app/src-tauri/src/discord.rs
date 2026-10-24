//! Discord Rich Presence: while a game runs, the user's Discord activity shows the instance, the
//! Minecraft version with the loader, and the time since the session started. With no game running
//! the activity is cleared.
//!
//! All IPC happens on one dedicated thread fed through a channel, so a missing or stuck Discord can
//! never slow a launch. Failures are logged at debug level and ignored; the connection is
//! re-established on the next state change. Without a client id (`K0T1K_DISCORD_CLIENT_ID` at build
//! time, see `build.rs`) every function does nothing.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Utc};
use discord_rich_presence::activity::{Activity, Assets, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};
use k0t1k_core::component::MINECRAFT_UID;
use k0t1k_core::instance::Instance;
use k0t1k_core::meta::Loader;

use crate::state::SessionId;

/// Key of the large image in the Discord application's Rich Presence assets (uploaded there by hand).
const LARGE_IMAGE: &str = "logo";

#[derive(Clone)]
struct Info {
    details: String,
    state: String,
    started_secs: i64,
}

struct Presence {
    /// Running sessions in start order; the last one is shown.
    sessions: Mutex<Vec<(SessionId, Info)>>,
    to_thread: Sender<Option<Info>>,
}

static PRESENCE: OnceLock<Option<Presence>> = OnceLock::new();

fn presence() -> Option<&'static Presence> {
    PRESENCE
        .get_or_init(|| {
            let Some(client_id) = option_env!("K0T1K_DISCORD_CLIENT_ID").filter(|id| !id.is_empty()) else {
                tracing::info!("Discord Rich Presence is off: no client id was embedded at build time");
                return None;
            };
            let (to_thread, updates) = channel();
            match std::thread::Builder::new().name("discord-presence".into()).spawn(move || run(client_id, &updates)) {
                Ok(_) => Some(Presence { sessions: Mutex::new(Vec::new()), to_thread }),
                Err(error) => {
                    tracing::warn!("cannot start the Discord presence thread: {error}");
                    None
                }
            }
        })
        .as_ref()
}

impl Presence {
    fn publish(&self, sessions: &[(SessionId, Info)]) {
        // The thread only ends with the process, so a failed send is not worth reporting.
        let _ = self.to_thread.send(sessions.last().map(|(_, info)| info.clone()));
    }
}

fn run(client_id: &str, updates: &Receiver<Option<Info>>) {
    let mut client: Option<DiscordIpcClient> = None;
    for wanted in updates {
        let Some(info) = wanted else {
            // Nothing to clear when Discord was never reached.
            if let Some(connected) = &mut client
                && let Err(error) = connected.clear_activity()
            {
                tracing::debug!("Discord: cannot clear the activity: {error}");
                client = None;
            }
            continue;
        };
        if client.is_none() {
            let mut connecting = DiscordIpcClient::new(client_id);
            match connecting.connect() {
                Ok(()) => client = Some(connecting),
                Err(error) => {
                    tracing::debug!("Discord is not reachable: {error}");
                    continue;
                }
            }
        }
        if let Some(connected) = &mut client {
            let activity = Activity::new()
                .details(info.details.as_str())
                .state(info.state.as_str())
                .timestamps(Timestamps::new().start(info.started_secs))
                .assets(Assets::new().large_image(LARGE_IMAGE));
            if let Err(error) = connected.set_activity(activity) {
                tracing::debug!("Discord: cannot set the activity: {error}");
                client = None;
            }
        }
    }
}

/// "1.21.4 · Fabric", or just the Minecraft version for a vanilla instance.
fn describe(instance: &Instance) -> String {
    let version = |uid: &str| instance.components.iter().find(|component| component.uid == uid).map(|component| component.version.as_str());
    let loader = [(Loader::Fabric, "Fabric"), (Loader::Quilt, "Quilt"), (Loader::Forge, "Forge"), (Loader::NeoForge, "NeoForge")]
        .into_iter()
        .find(|(loader, _)| version(loader.uid()).is_some())
        .map(|(_, label)| label);
    match (version(MINECRAFT_UID), loader) {
        (Some(minecraft), Some(loader)) => format!("{minecraft} · {loader}"),
        (Some(minecraft), None) => minecraft.to_owned(),
        (None, Some(loader)) => loader.to_owned(),
        (None, None) => String::new(),
    }
}

/// A game of `instance` started; it becomes the one shown. Call only when the user has Rich Presence on.
pub fn session_started(session_id: SessionId, instance: &Instance, started_at: DateTime<Utc>) {
    let Some(presence) = presence() else { return };
    let info = Info { details: instance.name.clone(), state: describe(instance), started_secs: started_at.timestamp() };
    let mut sessions = presence.sessions.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    sessions.push((session_id, info));
    presence.publish(&sessions);
}

/// The game of the session ended (or was never counted); shows the previous session, or clears.
pub fn session_stopped(session_id: SessionId) {
    // Not `presence()`: with Rich Presence never used there is nothing to remove or to start a thread for.
    let Some(Some(presence)) = PRESENCE.get() else { return };
    let mut sessions = presence.sessions.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let before = sessions.len();
    sessions.retain(|(id, _)| *id != session_id);
    if sessions.len() != before {
        presence.publish(&sessions);
    }
}
