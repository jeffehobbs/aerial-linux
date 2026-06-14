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
}

impl Default for Config {
    fn default() -> Self {
        Config {
            quality: Quality::Best,
            match_time_of_day: false,
            idle_timeout_secs: 300,
            allow_stream: true,
        }
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
