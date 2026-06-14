//! Video playback by driving the `mpv` binary as a subprocess.
//!
//! ## Why a subprocess, not libmpv FFI
//!
//! The plan originally sketched libmpv embedding. In practice, getting smooth
//! playback on this project's primary target (GNOME/Wayland on NVIDIA) hinges
//! on a specific render pipeline — `--vo=gpu-next --gpu-api=vulkan
//! --gpu-context=waylandvk` — because the OpenGL/EGL path mis-selects Mesa and
//! falls back to software rendering. Driving the `mpv` binary lets us pass
//! exactly those flags and lean on mpv's own Wayland VO for surface/fullscreen
//! handling, instead of reimplementing Vulkan context selection over FFI. It
//! also keeps the build free of a libmpv link, and overlays (a later phase) can
//! attach via mpv's Lua scripting + JSON IPC. If we later need GL-level
//! compositing of overlays, libmpv's render API remains an option.

use crate::cache::Cache;
use crate::catalog::{Catalog, TimeOfDay, VideoFormat};
use crate::selector;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

/// Which windowing system we're talking to — selects mpv's video-output flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayServer {
    Wayland,
    X11,
    Unknown,
}

impl DisplayServer {
    pub fn detect() -> DisplayServer {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            DisplayServer::Wayland
        } else if std::env::var_os("DISPLAY").is_some() {
            DisplayServer::X11
        } else {
            DisplayServer::Unknown
        }
    }
}

/// How to play. Distinguishes the standalone `play` command (user can quit with
/// a keypress) from the eventual idle daemon (lifecycle driven externally).
pub struct PlayOptions {
    pub fullscreen: bool,
    pub loop_playlist: bool,
    pub shuffle: bool,
    /// Allow Esc/q and other default key bindings to control/quit mpv. The idle
    /// daemon sets this false and tears mpv down itself on user activity.
    pub allow_input_quit: bool,
    /// If set, expose an mpv JSON IPC socket here (used by the daemon/overlays).
    pub ipc_socket: Option<PathBuf>,
    /// If set, load this mpv Lua script (the overlay renderer).
    pub script: Option<PathBuf>,
    /// If set, passed to mpv's environment as `AERIAL_OVERLAY_FILE` for the
    /// overlay script to read.
    pub overlay_file: Option<PathBuf>,
    /// Extra raw mpv args, appended last (also seeded from `$AERIAL_MPV_ARGS`).
    pub extra_args: Vec<String>,
}

impl Default for PlayOptions {
    fn default() -> Self {
        PlayOptions {
            fullscreen: true,
            loop_playlist: true,
            shuffle: true,
            allow_input_quit: true,
            ipc_socket: None,
            script: None,
            overlay_file: None,
            extra_args: Vec::new(),
        }
    }
}

/// Video-output flags for the detected display server.
///
/// Wayland → Vulkan pipeline (validated on NVIDIA; also the most broadly
/// hardware-accelerated path across AMD/Intel on modern Mesa). X11/Unknown →
/// let mpv auto-detect. Override via `$AERIAL_MPV_ARGS` if a box needs
/// something different.
fn video_args(display: DisplayServer) -> Vec<String> {
    match display {
        DisplayServer::Wayland => vec![
            "--vo=gpu-next".into(),
            "--gpu-api=vulkan".into(),
            "--gpu-context=waylandvk".into(),
            "--hwdec=auto".into(),
        ],
        DisplayServer::X11 | DisplayServer::Unknown => {
            vec!["--vo=gpu".into(), "--hwdec=auto".into()]
        }
    }
}

/// Assemble the full mpv argument vector for a playlist.
fn build_args(playlist: &[String], opts: &PlayOptions) -> Vec<String> {
    let mut args = video_args(DisplayServer::detect());

    // Screensaver hygiene: silent, no UI chrome, hidden cursor.
    args.push("--no-audio".into());
    args.push("--no-osc".into());
    args.push("--no-osd-bar".into());
    args.push("--cursor-autohide=always".into());
    args.push("--really-quiet".into());
    // Fill the screen rather than letterboxing — crops the overflow when the
    // clip's aspect (16:9) doesn't match the display (e.g. 21:9 ultrawides).
    args.push("--panscan=1.0".into());
    // Apple's aerials are 240fps masters; on a 60Hz display, sync presentation
    // to the display refresh so the 240→60 reduction is evenly paced (smooth)
    // instead of the ragged frame-dropping of the default audio sync.
    args.push("--video-sync=display-resample".into());

    if opts.fullscreen {
        args.push("--fullscreen".into());
    }
    if opts.loop_playlist {
        args.push("--loop-playlist=inf".into());
    }
    if opts.shuffle {
        args.push("--shuffle".into());
    }
    if !opts.allow_input_quit {
        // Daemon mode: ignore input entirely; teardown is external.
        args.push("--no-input-default-bindings".into());
        args.push("--input-conf=/dev/null".into());
    }
    if let Some(sock) = &opts.ipc_socket {
        args.push(format!("--input-ipc-server={}", sock.display()));
    }
    if let Some(script) = &opts.script {
        args.push(format!("--script={}", script.display()));
    }

    if let Ok(env_args) = std::env::var("AERIAL_MPV_ARGS") {
        args.extend(env_args.split_whitespace().map(String::from));
    }
    args.extend(opts.extra_args.iter().cloned());

    // End-of-options guard, then the media items.
    args.push("--".into());
    args.extend(playlist.iter().cloned());
    args
}

/// Build a playlist of file paths / URLs from the catalog.
///
/// Cached clips are preferred (played from disk). If `allow_stream` is set,
/// not-yet-cached clips contribute their best remote URL so playback works
/// before anything is downloaded. `count` caps the list (0 = unlimited).
pub fn build_playlist(
    catalog: &Catalog,
    cache: &Cache,
    preference: &[VideoFormat],
    restrict_to: Option<TimeOfDay>,
    count: usize,
    allow_stream: bool,
) -> Vec<String> {
    // Collect cached and stream-only candidates separately so we can always
    // prefer on-disk clips when `count` caps the list (avoids streaming when a
    // cached clip would do).
    let mut cached = Vec::new();
    let mut streamable = Vec::new();
    for v in selector::playable(catalog, restrict_to) {
        let Some((_, url)) = v.best_url(preference) else {
            continue;
        };
        let path = cache.video_path(&v.id, url);
        if path.exists() {
            cached.push(path.to_string_lossy().into_owned());
        } else if allow_stream {
            streamable.push(url.to_string());
        }
    }

    let mut out = cached;
    out.extend(streamable);
    if count != 0 && out.len() > count {
        out.truncate(count);
    }
    out
}

/// Spawn mpv for the given playlist, returning the child handle (does not wait).
pub fn spawn(playlist: &[String], opts: &PlayOptions) -> Result<Child> {
    if playlist.is_empty() {
        anyhow::bail!("nothing to play (empty playlist)");
    }
    let args = build_args(playlist, opts);
    let mut cmd = Command::new("mpv");
    cmd.args(&args).stdin(Stdio::null());
    if let Some(file) = &opts.overlay_file {
        cmd.env("AERIAL_OVERLAY_FILE", file);
    }
    cmd.spawn()
        .context("launching mpv (is it installed and on PATH?)")
}

/// Spawn mpv and await its exit by polling, so the async runtime stays free to
/// run a concurrent overlay refresher. Returns whether it exited successfully.
pub async fn play_async(playlist: &[String], opts: &PlayOptions) -> Result<bool> {
    let mut child = spawn(playlist, opts)?;
    loop {
        if let Some(status) = child.try_wait().context("polling mpv")? {
            return Ok(status.success());
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}
