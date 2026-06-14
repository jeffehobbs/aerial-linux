//! Overlay data: weather + "now playing", written to a JSON state file that the
//! mpv Lua script (`assets/overlay.lua`) reads and renders.
//!
//! The clock is rendered entirely in Lua (no Rust needed). Anything requiring
//! the network or D-Bus lives here: weather from OpenWeather, and the current
//! track from any MPRIS media player on the session bus. A background task
//! refreshes the file while the player runs.

use crate::config::Config;
use anyhow::Result;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::task::JoinHandle;

/// What the Lua script reads each tick.
#[derive(Debug, Default, Serialize)]
pub struct OverlayState {
    pub show_clock: bool,
    pub weather: Option<String>,
    pub now_playing: Option<String>,
}

/// Everything needed to wire overlays into a player launch.
pub struct OverlaySetup {
    pub script: PathBuf,
    pub state_file: PathBuf,
    config: Config,
}

/// Prepare overlays for a player launch, or `None` if disabled / the Lua script
/// can't be found. Writes an initial state file so the first frame has data.
pub fn prepare(config: &Config) -> Option<OverlaySetup> {
    if !config.overlays_enabled() {
        return None;
    }
    let script = lua_script_path()?;
    let state_file = state_path();
    let _ = write_state(
        &state_file,
        &OverlayState {
            show_clock: config.show_clock,
            ..Default::default()
        },
    );
    Some(OverlaySetup {
        script,
        state_file,
        config: config.clone(),
    })
}

impl OverlaySetup {
    /// Spawn the background refresher; abort the handle to stop it.
    pub fn spawn_refresher(&self) -> JoinHandle<()> {
        let config = self.config.clone();
        let path = self.state_file.clone();
        tokio::spawn(async move { refresh_loop(config, path).await })
    }
}

/// Where the state file lives — session runtime dir if available, else temp.
pub fn state_path() -> PathBuf {
    if let Some(rt) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(rt).join("aerial-overlay.json");
    }
    std::env::temp_dir().join("aerial-overlay.json")
}

/// Locate `overlay.lua`. Honors `$AERIAL_OVERLAY_LUA`, then checks paths
/// relative to the executable and the usual install locations, then the dev
/// tree.
fn lua_script_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("AERIAL_OVERLAY_LUA") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("overlay.lua"));
            // dev tree: target/<profile>/aerial-linux → ../../assets/overlay.lua
            candidates.push(dir.join("../../assets/overlay.lua"));
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join(".local/share/aerial-linux/overlay.lua"));
    }
    candidates.push(PathBuf::from("/usr/share/aerial-linux/overlay.lua"));
    candidates.push(PathBuf::from("assets/overlay.lua"));
    candidates.into_iter().find(|p| p.is_file())
}

/// Atomically write the state file (write to a temp sibling, then rename).
fn write_state(path: &Path, state: &OverlayState) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(state)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

async fn refresh_loop(config: Config, path: PathBuf) {
    let client = reqwest::Client::new();
    let conn = if config.show_now_playing {
        zbus::Connection::session().await.ok()
    } else {
        None
    };

    let mut weather: Option<String> = None;
    let mut tick: u64 = 0;
    loop {
        // Weather changes slowly: refresh every ~10 min (120 × 5s ticks).
        if config.weather_enabled() && tick % 120 == 0 {
            weather = fetch_weather(&client, &config).await;
        }
        let now_playing = match &conn {
            Some(c) => fetch_now_playing(c).await,
            None => None,
        };
        let _ = write_state(
            &path,
            &OverlayState {
                show_clock: config.show_clock,
                weather: weather.clone(),
                now_playing,
            },
        );
        tick = tick.wrapping_add(1);
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

// ---- weather ---------------------------------------------------------------

#[derive(serde::Deserialize)]
struct OwResponse {
    weather: Vec<OwWeather>,
    main: OwMain,
    name: String,
}
#[derive(serde::Deserialize)]
struct OwWeather {
    description: String,
}
#[derive(serde::Deserialize)]
struct OwMain {
    temp: f64,
}

async fn fetch_weather(client: &reqwest::Client, config: &Config) -> Option<String> {
    let (key, lat, lon) = (
        config.weather_api_key.as_ref()?,
        config.weather_lat?,
        config.weather_lon?,
    );
    let url = format!(
        "https://api.openweathermap.org/data/2.5/weather?lat={lat}&lon={lon}&units={}&appid={key}",
        config.weather_units
    );
    let resp = client.get(&url).send().await.ok()?.error_for_status().ok()?;
    let data: OwResponse = resp.json().await.ok()?;
    let unit = match config.weather_units.as_str() {
        "imperial" => "°F",
        "standard" => "K",
        _ => "°C",
    };
    let desc = data
        .weather
        .first()
        .map(|w| capitalize(&w.description))
        .unwrap_or_default();
    Some(format!(
        "{}{}  {}  ·  {}",
        data.main.temp.round() as i64,
        unit,
        desc,
        data.name
    ))
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

// ---- now playing (MPRIS) ---------------------------------------------------

/// Query the session bus for the first MPRIS player that is currently playing,
/// and format "♪ Artist — Title". Defensive: any failure → `None`.
async fn fetch_now_playing(conn: &zbus::Connection) -> Option<String> {
    let dbus = zbus::fdo::DBusProxy::new(conn).await.ok()?;
    let names = dbus.list_names().await.ok()?;
    let iface = zbus::names::InterfaceName::try_from("org.mpris.MediaPlayer2.Player").ok()?;

    for name in names {
        let n = name.as_str();
        if !n.starts_with("org.mpris.MediaPlayer2.") {
            continue;
        }
        let props = match zbus::fdo::PropertiesProxy::builder(conn)
            .destination(n.to_string())
            .and_then(|b| b.path("/org/mpris/MediaPlayer2"))
        {
            Ok(b) => match b.build().await {
                Ok(p) => p,
                Err(_) => continue,
            },
            Err(_) => continue,
        };

        let status = props
            .get(iface.clone(), "PlaybackStatus")
            .await
            .ok()
            .and_then(|v| String::try_from(v).ok());
        if status.as_deref() != Some("Playing") {
            continue;
        }

        let meta = props.get(iface.clone(), "Metadata").await.ok()?;
        let dict: HashMap<String, zbus::zvariant::OwnedValue> = meta.try_into().ok()?;
        let title = dict
            .get("xesam:title")
            .and_then(|v| v.try_clone().ok())
            .and_then(|v| String::try_from(v).ok());
        let artist = dict.get("xesam:artist").and_then(|v| {
            v.try_clone().ok().and_then(|v| {
                Vec::<String>::try_from(v.clone())
                    .map(|a| a.join(", "))
                    .ok()
                    .or_else(|| String::try_from(v).ok())
            })
        });

        return match (artist, title) {
            (Some(a), Some(t)) => Some(format!("♪  {a} — {t}")),
            (None, Some(t)) => Some(format!("♪  {t}")),
            _ => None,
        };
    }
    None
}
