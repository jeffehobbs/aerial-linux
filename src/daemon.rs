//! GNOME idle daemon.
//!
//! On GNOME/Wayland there is no third-party screensaver plug-in point, so the
//! screensaver *is* a daemon that watches the session's idle time and drives
//! the player. We use Mutter's D-Bus idle monitor:
//!
//! * `AddIdleWatch(interval_ms)` fires `WatchFired` once the user has been idle
//!   that long → we start the player.
//! * `AddUserActiveWatch()` fires `WatchFired` the moment the user is active
//!   again → we stop the player. (It auto-removes after firing.)
//!
//! The idle watch persists and re-fires on each subsequent idle period, so the
//! cycle repeats for the life of the daemon. Teardown is driven entirely by the
//! active watch — not by the mpv window — which is what lets a plain fullscreen
//! `xdg-toplevel` work on Mutter (no layer-shell / always-on-top needed).

use crate::overlay::OverlaySetup;
use crate::player::{self, PlayOptions};
use anyhow::{Context, Result};
use futures_util::StreamExt;
use std::process::Child;
use tokio::task::JoinHandle;

/// Mutter's idle-monitor interface (the "Core" seat-wide monitor).
#[zbus::proxy(
    interface = "org.gnome.Mutter.IdleMonitor",
    default_service = "org.gnome.Mutter.IdleMonitor",
    default_path = "/org/gnome/Mutter/IdleMonitor/Core"
)]
trait IdleMonitor {
    /// Milliseconds since the last user input.
    fn get_idletime(&self) -> zbus::Result<u64>;
    /// Fire `WatchFired` once idle reaches `interval` ms. Returns the watch id.
    fn add_idle_watch(&self, interval: u64) -> zbus::Result<u32>;
    /// Fire `WatchFired` the next time the user becomes active. Returns the id.
    fn add_user_active_watch(&self) -> zbus::Result<u32>;
    fn remove_watch(&self, id: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    fn watch_fired(&self, id: u32) -> zbus::Result<()>;
}

/// Run the idle loop until a shutdown signal (SIGINT/SIGTERM).
///
/// `make_playlist` is called each time we go idle, so newly-cached clips are
/// picked up without restarting the daemon.
pub async fn run(
    idle_timeout_secs: u64,
    overlay: Option<OverlaySetup>,
    make_playlist: impl Fn() -> Vec<String>,
) -> Result<()> {
    let conn = zbus::Connection::session()
        .await
        .context("connecting to the session bus (is a GNOME session running?)")?;
    let monitor = IdleMonitorProxy::new(&conn)
        .await
        .context("creating Mutter IdleMonitor proxy")?;

    // Subscribe before arming so we can't miss an immediate fire.
    let mut fired = monitor.receive_watch_fired().await?;

    let idle_id = monitor
        .add_idle_watch(idle_timeout_secs * 1000)
        .await
        .context("adding idle watch")?;
    eprintln!(
        "aerial-linux daemon: idle watch #{idle_id} armed at {idle_timeout_secs}s; waiting…"
    );

    let mut active_id: Option<u32> = None;
    let mut child: Option<Child> = None;
    let mut refresher: Option<JoinHandle<()>> = None;

    // `AddIdleWatch` only fires on an *upward* crossing of the threshold, so if
    // the session is already idle past it when we start (e.g. the service was
    // enabled and the user walked away), the watch won't fire until the next
    // activity→idle cycle. Handle that by checking the current idle time once.
    if monitor.get_idletime().await.unwrap_or(0) >= idle_timeout_secs * 1000 {
        eprintln!("aerial-linux daemon: already idle at startup → playing");
        start_player(
            &monitor,
            overlay.as_ref(),
            &make_playlist,
            &mut child,
            &mut active_id,
            &mut refresher,
        )
        .await;
    }

    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                eprintln!("aerial-linux daemon: shutting down");
                stop_player(&mut child, &mut refresher);
                let _ = monitor.remove_watch(idle_id).await;
                if let Some(id) = active_id {
                    let _ = monitor.remove_watch(id).await;
                }
                return Ok(());
            }
            sig = fired.next() => {
                let Some(sig) = sig else { break };
                let id = sig.args().context("decoding WatchFired args")?.id;

                if id == idle_id {
                    // Went idle → start the screensaver and arm the active watch.
                    start_player(
                        &monitor,
                        overlay.as_ref(),
                        &make_playlist,
                        &mut child,
                        &mut active_id,
                        &mut refresher,
                    )
                    .await;
                } else if Some(id) == active_id {
                    // User active → stop the screensaver. (Watch auto-removed.)
                    eprintln!("aerial-linux daemon: active → stopping player");
                    stop_player(&mut child, &mut refresher);
                    active_id = None;
                }
            }
        }
    }

    stop_player(&mut child, &mut refresher);
    Ok(())
}

/// Start the player (if not already running) and arm the user-active watch so
/// we get told when to stop. No-op if a player is already running.
#[allow(clippy::too_many_arguments)]
async fn start_player(
    monitor: &IdleMonitorProxy<'_>,
    overlay: Option<&OverlaySetup>,
    make_playlist: &impl Fn() -> Vec<String>,
    child: &mut Option<Child>,
    active_id: &mut Option<u32>,
    refresher: &mut Option<JoinHandle<()>>,
) {
    if child.is_some() {
        return;
    }
    let playlist = make_playlist();
    if playlist.is_empty() {
        eprintln!("aerial-linux daemon: idle, but no clips to play — skipping");
        return;
    }
    eprintln!("aerial-linux daemon: idle → playing {} clips", playlist.len());
    match player::spawn(&playlist, &daemon_play_opts(overlay)) {
        Ok(c) => {
            *child = Some(c);
            if let Some(o) = overlay {
                *refresher = Some(o.spawn_refresher());
            }
            match monitor.add_user_active_watch().await {
                Ok(id) => *active_id = Some(id),
                Err(e) => eprintln!("aerial-linux daemon: failed to arm active watch: {e:#}"),
            }
        }
        Err(e) => eprintln!("aerial-linux daemon: failed to start player: {e:#}"),
    }
}

/// Player options for daemon-managed playback: fullscreen, looping, and
/// input-inert (the daemon owns the lifecycle, not keypresses in the window).
fn daemon_play_opts(overlay: Option<&OverlaySetup>) -> PlayOptions {
    PlayOptions {
        fullscreen: true,
        loop_playlist: true,
        shuffle: true,
        allow_input_quit: false,
        ipc_socket: None,
        script: overlay.map(|o| o.script.clone()),
        overlay_file: overlay.map(|o| o.state_file.clone()),
        extra_args: Vec::new(),
    }
}

fn stop_player(child: &mut Option<Child>, refresher: &mut Option<JoinHandle<()>>) {
    if let Some(mut c) = child.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
    if let Some(r) = refresher.take() {
        r.abort();
    }
}

/// Resolves when SIGINT or SIGTERM arrives (systemd stop / Ctrl-C).
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => return std::future::pending().await,
        };
        let mut int = match signal(SignalKind::interrupt()) {
            Ok(s) => s,
            Err(_) => return std::future::pending().await,
        };
        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
