//! Video selection: time-of-day filtering and shuffle.
//!
//! Mirrors Aerial's behaviour (`VideoList.swift`): optionally restrict to clips
//! matching the current day/night, then pick at random — falling back to the
//! full set if the filter would leave nothing. Uses a tiny self-contained PRNG
//! so we don't pull in a dependency just to shuffle.

use crate::catalog::{Catalog, TimeOfDay, Video};
use std::time::{SystemTime, UNIX_EPOCH};

/// Coarse local day/night guess from the system clock. A precise sunrise/sunset
/// calculation (with location) is a later-phase refinement; for now treat
/// 06:00–18:00 as day.
pub fn current_time_of_day() -> TimeOfDay {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let hour = (secs / 3600) % 24; // UTC hour; good enough until we add tz/location
    if (6..18).contains(&hour) {
        TimeOfDay::Day
    } else {
        TimeOfDay::Night
    }
}

/// Videos that have at least one playable URL, optionally restricted to a
/// time of day (clips tagged `Unknown` always pass — many manifests omit it).
pub fn playable<'a>(catalog: &'a Catalog, restrict_to: Option<TimeOfDay>) -> Vec<&'a Video> {
    catalog
        .videos
        .iter()
        .filter(|v| !v.urls.is_empty() && !v.is_live)
        .filter(|v| match restrict_to {
            Some(tod) => v.time_of_day == tod || v.time_of_day == TimeOfDay::Unknown,
            None => true,
        })
        .collect()
}

/// Pick a random video honouring the time-of-day restriction, falling back to
/// the unrestricted set if the restriction empties the list.
pub fn pick<'a>(catalog: &'a Catalog, restrict_to: Option<TimeOfDay>) -> Option<&'a Video> {
    let mut pool = playable(catalog, restrict_to);
    if pool.is_empty() {
        pool = playable(catalog, None);
    }
    if pool.is_empty() {
        return None;
    }
    let idx = (next_rand() % pool.len() as u64) as usize;
    Some(pool[idx])
}

/// xorshift64* seeded from the high-resolution clock. Adequate for "pick a
/// random clip"; not for anything that needs real randomness.
fn next_rand() -> u64 {
    let mut x = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E3779B97F4A7C15)
        | 1;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    x.wrapping_mul(0x2545F4914F6CDD1D)
}
