//! User configuration (`$XDG_CONFIG_HOME/aerial-linux/config.toml`).
//!
//! Phase 1 only needs a couple of knobs; the full prefs surface (overlays,
//! location, units) lands with later phases. Missing file → defaults.

use crate::catalog::VideoFormat;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Quality/compatibility preference for which format to download & play.
    pub quality: Quality,
    /// Restrict playback to match the wall-clock time of day where possible.
    pub match_time_of_day: bool,
    /// Seconds of inactivity before the daemon starts the screensaver.
    pub idle_timeout_secs: u64,
    /// Whether the daemon may stream clips that aren't cached yet.
    pub allow_stream: bool,

    /// Show a clock overlay.
    pub show_clock: bool,
    /// Show a "now playing" overlay sourced from MPRIS.
    pub show_now_playing: bool,
    /// OpenWeather API key. Weather overlay is shown only when this and a
    /// location are set.
    pub weather_api_key: Option<String>,
    pub weather_lat: Option<f64>,
    pub weather_lon: Option<f64>,
    /// OpenWeather units: "metric" | "imperial" | "standard".
    pub weather_units: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            quality: Quality::Best,
            match_time_of_day: false,
            idle_timeout_secs: 300,
            allow_stream: true,
            show_clock: true,
            show_now_playing: true,
            weather_api_key: None,
            weather_lat: None,
            weather_lon: None,
            weather_units: "metric".to_string(),
        }
    }
}

impl Config {
    /// Whether the weather overlay is fully configured.
    pub fn weather_enabled(&self) -> bool {
        self.weather_api_key.is_some() && self.weather_lat.is_some() && self.weather_lon.is_some()
    }

    /// Whether any overlay should be shown at all.
    pub fn overlays_enabled(&self) -> bool {
        self.show_clock || self.show_now_playing || self.weather_enabled()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    /// Highest quality that decodes well on Linux (4K SDR HEVC first).
    Best,
    /// Most broadly hardware-decodable (H.264 1080p first).
    Compatible,
}

impl Quality {
    pub fn preference(self) -> &'static [VideoFormat] {
        match self {
            Quality::Best => VideoFormat::PREFERENCE,
            Quality::Compatible => VideoFormat::COMPATIBLE,
        }
    }
}

impl Config {
    pub fn path() -> Result<PathBuf> {
        let dirs = directories::ProjectDirs::from("", "", "aerial-linux")
            .context("could not determine config directory")?;
        Ok(dirs.config_dir().join("config.toml"))
    }

    /// Load config, falling back to defaults if the file is absent.
    pub fn load() -> Result<Config> {
        let path = Self::path()?;
        match std::fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).with_context(|| format!("parsing {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }
}
