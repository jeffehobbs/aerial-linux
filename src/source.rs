//! Known Apple Aerial manifest sources.
//!
//! URLs lifted verbatim from AerialScreensaver/Aerial
//! (`ScreenSaver/Source/Models/Sources/SourceList.swift`). Each points at a
//! `resources-*.tar` on Apple's CDN that contains an `entries.json`. Apple
//! periodically rotates these URLs and appends a tvOS version, so this list is
//! expected to drift — keep it easy to update.

/// A built-in manifest source.
#[derive(Debug, Clone)]
pub struct Source {
    pub name: &'static str,
    pub description: &'static str,
    pub manifest_url: &'static str,
}

/// The built-in sources, newest tvOS release first.
pub const SOURCES: &[Source] = &[
    Source {
        name: "tvOS26",
        description: "tvOS 26 aerials (4K SDR/HDR HEVC)",
        manifest_url: "https://sylvan.apple.com/itunes-assets/Aerials126/v4/82/2e/34/822e344c-f5d2-878c-3d56-508d5b09ed61/resources-26-0-1.tar",
    },
    Source {
        name: "tvOS23J",
        description: "tvOS 23J aerials",
        manifest_url: "https://sylvan.apple.com/itunes-assets/Aerials126/v4/c0/45/d9/c045d9d0-9606-1535-62fe-189edb4f79eb/resources-atv-23J-2.tar",
    },
    Source {
        name: "tvOS13",
        description: "tvOS 13 aerials (legacy, widely mirrored)",
        manifest_url: "https://sylvan.apple.com/Aerials/resources-13.tar",
    },
];

/// Look up a source by (case-insensitive) name.
pub fn by_name(name: &str) -> Option<&'static Source> {
    SOURCES.iter().find(|s| s.name.eq_ignore_ascii_case(name))
}
