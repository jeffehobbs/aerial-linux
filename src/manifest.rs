//! Apple Aerial manifest (`entries.json`) schema.
//!
//! Reverse-engineered from AerialScreensaver/Aerial
//! (`ScreenSaver/Source/Models/Sources/Source.swift`). Apple ships an
//! `entries.json` inside a `resources-*.tar` tarball on `sylvan.apple.com`.
//! Over the years the manifest has taken two shapes, both handled here:
//!
//! * [`VideoManifest`] — the tvOS11-style flat list of [`VideoAsset`]s, each
//!   carrying several pre-rendered URLs (1080 H264/SDR/HDR, 4K SDR/HDR, …).
//! * [`MacManifest`] — the newer macOS shape: assets ([`MacAsset`]) only carry
//!   a single `url-4K-SDR-240FPS`, plus localization/category metadata.
//!
//! We decode whichever shape parses and normalize both into the crate's
//! internal [`crate::catalog::Video`] type.

use serde::Deserialize;
use std::collections::HashMap;

/// tvOS11-style manifest: a flat list of assets, each with multiple format URLs.
#[derive(Debug, Clone, Deserialize)]
pub struct VideoManifest {
    pub assets: Vec<VideoAsset>,
    #[serde(rename = "initialAssetCount")]
    pub initial_asset_count: Option<i64>,
    pub version: Option<i64>,
}

/// A single aerial clip in a [`VideoManifest`]. Every URL field is optional —
/// manifests in the wild omit formats freely. `*_md5` are sibling lowercase-hex
/// digests used to verify a downloaded file; absent → verification skipped.
#[derive(Debug, Clone, Deserialize)]
pub struct VideoAsset {
    #[serde(rename = "accessibilityLabel")]
    pub accessibility_label: String,
    pub id: String,
    pub title: Option<String>,
    #[serde(rename = "timeOfDay")]
    pub time_of_day: Option<String>,
    pub scene: Option<String>,
    /// Map of playback timecode (seconds, as string) → localized POI string key.
    #[serde(rename = "pointsOfInterest", default)]
    pub points_of_interest: HashMap<String, String>,

    #[serde(rename = "url-4K-HDR")]
    pub url_4k_hdr: Option<String>,
    #[serde(rename = "url-4K-SDR")]
    pub url_4k_sdr: Option<String>,
    #[serde(rename = "url-1080-H264")]
    pub url_1080_h264: Option<String>,
    #[serde(rename = "url-1080-HDR")]
    pub url_1080_hdr: Option<String>,
    #[serde(rename = "url-1080-SDR")]
    pub url_1080_sdr: Option<String>,
    #[serde(rename = "url-4K-SDR-120FPS")]
    pub url_4k_sdr_120fps: Option<String>,
    #[serde(rename = "url-4K-SDR-240FPS")]
    pub url_4k_sdr_240fps: Option<String>,
    pub url: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,

    /// Set by "Live Feeds" entries; absent from regular manifests.
    #[serde(rename = "isLive")]
    pub is_live: Option<bool>,
    #[serde(rename = "livePlaybackSeconds")]
    pub live_playback_seconds: Option<f64>,
    #[serde(rename = "previewImage")]
    pub preview_image: Option<String>,

    #[serde(rename = "url-4K-HDR-md5")]
    pub url_4k_hdr_md5: Option<String>,
    #[serde(rename = "url-4K-SDR-md5")]
    pub url_4k_sdr_md5: Option<String>,
    #[serde(rename = "url-1080-H264-md5")]
    pub url_1080_h264_md5: Option<String>,
    #[serde(rename = "url-1080-HDR-md5")]
    pub url_1080_hdr_md5: Option<String>,
    #[serde(rename = "url-1080-SDR-md5")]
    pub url_1080_sdr_md5: Option<String>,
    #[serde(rename = "url-4K-SDR-120FPS-md5")]
    pub url_4k_sdr_120fps_md5: Option<String>,
    #[serde(rename = "url-4K-SDR-240FPS-md5")]
    pub url_4k_sdr_240fps_md5: Option<String>,
}

/// Newer macOS-style manifest. Assets ([`MacAsset`]) only carry a single
/// `url-4K-SDR-240FPS`; everything else is metadata/localization.
#[derive(Debug, Clone, Deserialize)]
pub struct MacManifest {
    #[serde(rename = "initialAssetCount")]
    pub initial_asset_count: Option<i64>,
    pub assets: Vec<MacAsset>,
    pub version: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MacAsset {
    #[serde(rename = "shotID")]
    pub shot_id: Option<String>,
    #[serde(rename = "previewImage")]
    pub preview_image: Option<String>,
    #[serde(rename = "localizedNameKey")]
    pub localized_name_key: String,
    #[serde(rename = "accessibilityLabel")]
    pub accessibility_label: String,
    pub id: String,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub subcategories: Vec<String>,
    #[serde(rename = "pointsOfInterest", default)]
    pub points_of_interest: HashMap<String, String>,
    #[serde(rename = "url-4K-SDR-240FPS")]
    pub url_4k_sdr_240fps: String,
    #[serde(rename = "includeInShuffle", default = "default_true")]
    pub include_in_shuffle: bool,
}

fn default_true() -> bool {
    true
}

/// Result of decoding an `entries.json` of unknown shape.
pub enum ParsedManifest {
    Video(VideoManifest),
    Mac(MacManifest),
}

/// Decode an `entries.json` payload, trying both known shapes. The `Mac` shape
/// is distinctive (its assets require `localizedNameKey`), so we try it first
/// and fall back to the broader `Video` shape.
pub fn parse(bytes: &[u8]) -> anyhow::Result<ParsedManifest> {
    if let Ok(m) = serde_json::from_slice::<MacManifest>(bytes) {
        return Ok(ParsedManifest::Mac(m));
    }
    let v = serde_json::from_slice::<VideoManifest>(bytes)
        .map_err(|e| anyhow::anyhow!("entries.json matched neither manifest shape: {e}"))?;
    Ok(ParsedManifest::Video(v))
}
