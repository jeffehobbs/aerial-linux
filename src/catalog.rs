//! Unified video catalog.
//!
//! Normalizes the two Apple manifest shapes (see [`crate::manifest`]) into a
//! single [`Video`] type, fetches+extracts the `entries.json` from each
//! source's tarball, and persists the merged catalog as a single
//! `catalog.json` in the cache dir.

use crate::manifest::{self, MacAsset, ParsedManifest, VideoAsset};
use crate::source::{self, Source};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;

/// Playable formats, roughly ordered worst→best quality. The numeric value is
/// only used for the preference ordering in [`VideoFormat::PREFERENCE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum VideoFormat {
    /// H.264 1080p — most broadly hardware-decodable; safest default on Linux.
    H264_1080,
    Sdr1080,
    Hdr1080,
    Sdr4K,
    Sdr4K120,
    Sdr4K240,
    Hdr4K,
}

impl VideoFormat {
    /// Default selection order for Linux/mpv playback: prefer SDR HEVC at 4K for
    /// quality, fall back toward the most compatible H.264 1080p, and treat HDR
    /// as a last resort (HDR tone-mapping on Linux is inconsistent).
    pub const PREFERENCE: &'static [VideoFormat] = &[
        VideoFormat::Sdr4K,
        VideoFormat::Sdr4K240,
        VideoFormat::Sdr4K120,
        VideoFormat::Sdr1080,
        VideoFormat::H264_1080,
        VideoFormat::Hdr4K,
        VideoFormat::Hdr1080,
    ];

    /// Most-compatible-first order, for `--quality compatible`.
    pub const COMPATIBLE: &'static [VideoFormat] = &[
        VideoFormat::H264_1080,
        VideoFormat::Sdr1080,
        VideoFormat::Sdr4K,
        VideoFormat::Sdr4K120,
        VideoFormat::Sdr4K240,
        VideoFormat::Hdr1080,
        VideoFormat::Hdr4K,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            VideoFormat::H264_1080 => "1080-H264",
            VideoFormat::Sdr1080 => "1080-SDR",
            VideoFormat::Hdr1080 => "1080-HDR",
            VideoFormat::Sdr4K => "4K-SDR",
            VideoFormat::Sdr4K120 => "4K-SDR-120FPS",
            VideoFormat::Sdr4K240 => "4K-SDR-240FPS",
            VideoFormat::Hdr4K => "4K-HDR",
        }
    }
}

/// Time-of-day classification, used by the selector to match wall-clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeOfDay {
    Day,
    Night,
    Unknown,
}

impl TimeOfDay {
    fn parse(s: Option<&str>) -> TimeOfDay {
        match s.map(|s| s.to_ascii_lowercase()).as_deref() {
            Some("day") => TimeOfDay::Day,
            Some("night") | Some("sunset") | Some("sunrise") => TimeOfDay::Night,
            _ => TimeOfDay::Unknown,
        }
    }
}

/// One normalized aerial clip.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub source: String,
    pub time_of_day: TimeOfDay,
    pub scene: Option<String>,
    /// format → download URL
    pub urls: BTreeMap<VideoFormat, String>,
    /// format → lowercase-hex md5 (subset of `urls`)
    #[serde(default)]
    pub md5s: BTreeMap<VideoFormat, String>,
    pub is_live: bool,
}

impl Video {
    /// Pick the best available URL for the given preference order.
    pub fn best_url(&self, preference: &[VideoFormat]) -> Option<(VideoFormat, &str)> {
        preference
            .iter()
            .find_map(|f| self.urls.get(f).map(|u| (*f, u.as_str())))
    }
}

/// The persisted, merged catalog across all fetched sources.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Catalog {
    pub videos: Vec<Video>,
}

impl Catalog {
    pub fn len(&self) -> usize {
        self.videos.len()
    }

    pub fn is_empty(&self) -> bool {
        self.videos.is_empty()
    }

    pub fn find(&self, id: &str) -> Option<&Video> {
        self.videos.iter().find(|v| v.id == id)
    }

    /// Merge in another set of videos, de-duplicating by `id` (first wins).
    pub fn merge(&mut self, more: Vec<Video>) {
        let existing: std::collections::HashSet<String> =
            self.videos.iter().map(|v| v.id.clone()).collect();
        for v in more {
            if !existing.contains(&v.id) {
                self.videos.push(v);
            }
        }
    }
}

/// Download a source's tarball, extract `entries.json`, and normalize it.
pub async fn fetch_source(client: &reqwest::Client, src: &Source) -> Result<Vec<Video>> {
    let bytes = client
        .get(src.manifest_url)
        .send()
        .await
        .with_context(|| format!("requesting manifest for {}", src.name))?
        .error_for_status()
        .with_context(|| format!("manifest HTTP error for {}", src.name))?
        .bytes()
        .await
        .with_context(|| format!("reading manifest body for {}", src.name))?;

    let entries = extract_entries_json(&bytes)
        .with_context(|| format!("extracting entries.json for {}", src.name))?;
    let parsed = manifest::parse(&entries)
        .with_context(|| format!("parsing entries.json for {}", src.name))?;

    Ok(match parsed {
        ParsedManifest::Video(m) => m
            .assets
            .into_iter()
            .map(|a| normalize_video_asset(src.name, a))
            .collect(),
        ParsedManifest::Mac(m) => m
            .assets
            .into_iter()
            .map(|a| normalize_mac_asset(src.name, a))
            .collect(),
    })
}

/// Extract `entries.json` from the resources tarball. Apple ships a plain
/// (uncompressed) tar, but some mirrors gzip it — detect the gzip magic and
/// decompress transparently.
fn extract_entries_json(bytes: &[u8]) -> Result<Vec<u8>> {
    let is_gzip = bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b;
    let reader: Box<dyn Read> = if is_gzip {
        Box::new(flate2::read::GzDecoder::new(bytes))
    } else {
        Box::new(bytes)
    };
    let mut archive = tar::Archive::new(reader);
    for entry in archive.entries().context("reading tar entries")? {
        let mut entry = entry?;
        let path = entry.path()?;
        if path.file_name().and_then(|n| n.to_str()) == Some("entries.json") {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            return Ok(buf);
        }
    }
    anyhow::bail!("no entries.json found in tarball")
}

fn normalize_video_asset(source: &str, a: VideoAsset) -> Video {
    let mut urls = BTreeMap::new();
    let mut md5s = BTreeMap::new();
    let formats = [
        (VideoFormat::H264_1080, a.url_1080_h264, a.url_1080_h264_md5),
        (VideoFormat::Sdr1080, a.url_1080_sdr, a.url_1080_sdr_md5),
        (VideoFormat::Hdr1080, a.url_1080_hdr, a.url_1080_hdr_md5),
        (VideoFormat::Sdr4K, a.url_4k_sdr, a.url_4k_sdr_md5),
        (VideoFormat::Sdr4K120, a.url_4k_sdr_120fps, a.url_4k_sdr_120fps_md5),
        (VideoFormat::Sdr4K240, a.url_4k_sdr_240fps, a.url_4k_sdr_240fps_md5),
        (VideoFormat::Hdr4K, a.url_4k_hdr, a.url_4k_hdr_md5),
    ];
    for (fmt, url, md5) in formats {
        if let Some(u) = url.filter(|s| !s.is_empty()) {
            urls.insert(fmt, u);
            if let Some(m) = md5.filter(|s| !s.is_empty()) {
                md5s.insert(fmt, m.to_ascii_lowercase());
            }
        }
    }
    // Some manifests only populate the generic `url` field.
    if urls.is_empty() {
        if let Some(u) = a.url.filter(|s| !s.is_empty()) {
            urls.insert(VideoFormat::H264_1080, u);
        }
    }

    Video {
        id: a.id,
        title: a
            .title
            .filter(|s| !s.is_empty())
            .unwrap_or(a.accessibility_label),
        source: source.to_string(),
        time_of_day: TimeOfDay::parse(a.time_of_day.as_deref()),
        scene: a.scene.filter(|s| !s.is_empty()),
        urls,
        md5s,
        is_live: a.is_live.unwrap_or(false),
    }
}

fn normalize_mac_asset(source: &str, a: MacAsset) -> Video {
    let mut urls = BTreeMap::new();
    if !a.url_4k_sdr_240fps.is_empty() {
        urls.insert(VideoFormat::Sdr4K240, a.url_4k_sdr_240fps);
    }
    Video {
        id: a.id,
        title: a.accessibility_label,
        source: source.to_string(),
        // The Mac manifest drops the timeOfDay key; subcategories sometimes
        // encode it, but treat as Unknown until we map subcategories.
        time_of_day: TimeOfDay::Unknown,
        scene: a.subcategories.first().cloned(),
        urls,
        md5s: BTreeMap::new(),
        is_live: false,
    }
}

/// Fetch every built-in source (or just `only`, if given) and merge into one
/// catalog.
pub async fn build(client: &reqwest::Client, only: Option<&str>) -> Result<Catalog> {
    let mut catalog = Catalog::default();
    for src in source::SOURCES {
        if let Some(name) = only {
            if !src.name.eq_ignore_ascii_case(name) {
                continue;
            }
        }
        match fetch_source(client, src).await {
            Ok(videos) => {
                eprintln!("  {} → {} videos", src.name, videos.len());
                catalog.merge(videos);
            }
            // One bad/rotated URL shouldn't sink the whole fetch.
            Err(e) => eprintln!("  {} → skipped: {e:#}", src.name),
        }
    }
    Ok(catalog)
}
