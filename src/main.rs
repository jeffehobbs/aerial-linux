//! aerial-linux — Apple TV Aerial screensaver for Linux.
//!
//! Phase 1 scope: the catalog + cache layer. This binary can list Apple's
//! aerial sources, fetch & merge their manifests into a local catalog, list
//! the resulting videos, and pre-cache clips to disk. Playback, the GNOME
//! Wayland idle daemon, and overlays arrive in later phases.

mod cache;
mod catalog;
mod config;
mod manifest;
mod player;
mod selector;
mod source;

use anyhow::{Context, Result};
use cache::Cache;
use catalog::TimeOfDay;
use clap::{Parser, Subcommand};
use config::Config;

const USER_AGENT: &str = "AppleCoreMedia/1.0.0.20G75 (Apple TV; U; CPU OS 16_0 like Mac OS X; en_us)";

/// Build the HTTP client used for Apple's aerial CDN.
///
/// `sylvan.apple.com` presents a leaf certificate issued by Apple's *private*
/// "Apple Server Authentication CA" and does **not** send the intermediate.
/// macOS/iOS trust that CA inherently (and cache the intermediate), but the
/// Mozilla/Linux trust store does not, and rustls does not do AIA fetching — so
/// the chain cannot be built on Linux, and Apple does not publish the
/// intermediate for bundling.
///
/// We therefore disable chain verification *for this client only*. The trade-off
/// is acceptable here because every request goes to Apple's CDN for **public,
/// non-sensitive** video assets — the same content the macOS Aerial app fetches
/// — and we send no credentials. Any non-Apple host (e.g. the weather API in a
/// later phase) must use a separate, fully-verifying client.
///
/// Hardening path: if Apple's "Apple Server Authentication CA" intermediate can
/// be obtained, bundle it and switch to `add_root_certificate` + a pinned
/// issuer check instead of disabling verification.
fn apple_cdn_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .danger_accept_invalid_certs(true)
        .build()
        .context("building Apple CDN HTTP client")
}

#[derive(Parser)]
#[command(name = "aerial-linux", version, about = "Apple TV Aerial screensaver for Linux (catalog + cache)")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List the known Apple aerial manifest sources.
    Sources,
    /// Download & merge source manifests into the local catalog.
    Fetch {
        /// Only fetch this source (by name); default fetches all.
        #[arg(long)]
        source: Option<String>,
    },
    /// List videos in the local catalog.
    List {
        /// Restrict to a time of day: day | night.
        #[arg(long)]
        time: Option<String>,
        /// Show download URLs.
        #[arg(long)]
        urls: bool,
    },
    /// Pre-cache one or more videos to disk by id (or `--random N`).
    Cache {
        /// Specific video ids to download.
        ids: Vec<String>,
        /// Instead of ids, download N randomly chosen clips.
        #[arg(long)]
        random: Option<usize>,
    },
    /// Play aerials fullscreen via mpv (foreground; Esc/q to quit).
    Play {
        /// Restrict to a time of day: day | night.
        #[arg(long)]
        time: Option<String>,
        /// Max clips in the playlist (0 = all).
        #[arg(long, default_value_t = 0)]
        count: usize,
        /// Allow streaming clips that aren't cached yet.
        #[arg(long)]
        stream: bool,
        /// Play in a window instead of fullscreen (for testing).
        #[arg(long)]
        windowed: bool,
    },
    /// Print cache/config locations and catalog status.
    Status,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = apple_cdn_client()?;

    match cli.command {
        Command::Sources => cmd_sources(),
        Command::Fetch { source } => cmd_fetch(&client, source.as_deref()).await,
        Command::List { time, urls } => cmd_list(time.as_deref(), urls),
        Command::Cache { ids, random } => cmd_cache(&client, ids, random).await,
        Command::Play {
            time,
            count,
            stream,
            windowed,
        } => cmd_play(time.as_deref(), count, stream, windowed),
        Command::Status => cmd_status(),
    }
}

fn cmd_sources() -> Result<()> {
    println!("Known aerial sources:");
    for s in source::SOURCES {
        println!("  {:<8}  {}", s.name, s.description);
        println!("            {}", s.manifest_url);
    }
    Ok(())
}

async fn cmd_fetch(client: &reqwest::Client, only: Option<&str>) -> Result<()> {
    if let Some(name) = only {
        if source::by_name(name).is_none() {
            anyhow::bail!("unknown source '{name}' (see `aerial-linux sources`)");
        }
    }
    eprintln!("Fetching manifests…");
    let catalog = catalog::build(client, only).await?;
    if catalog.is_empty() {
        anyhow::bail!("no videos fetched — all sources failed (URLs may have rotated)");
    }
    let cache = Cache::open()?;
    cache.save_catalog(&catalog)?;
    println!(
        "Catalog: {} videos saved to {}",
        catalog.len(),
        cache.root().join("catalog.json").display()
    );
    Ok(())
}

fn parse_time(time: Option<&str>) -> Result<Option<TimeOfDay>> {
    match time.map(|t| t.to_ascii_lowercase()).as_deref() {
        None => Ok(None),
        Some("day") => Ok(Some(TimeOfDay::Day)),
        Some("night") => Ok(Some(TimeOfDay::Night)),
        Some(other) => anyhow::bail!("invalid --time '{other}' (use day|night)"),
    }
}

fn cmd_list(time: Option<&str>, show_urls: bool) -> Result<()> {
    let restrict = parse_time(time)?;
    let cache = Cache::open()?;
    let catalog = cache.load_catalog()?;
    let config = Config::load()?;
    let pref = config.quality.preference();

    let videos = selector::playable(&catalog, restrict);
    println!("{} videos:", videos.len());
    for v in videos {
        let tod = match v.time_of_day {
            TimeOfDay::Day => "day",
            TimeOfDay::Night => "night",
            TimeOfDay::Unknown => "—",
        };
        let cached = if cache.is_cached(&v.id) { "✓" } else { " " };
        println!("  {cached} {:<10} [{:<5}] {}", v.id, tod, v.title);
        if show_urls {
            if let Some((fmt, url)) = v.best_url(pref) {
                println!("        {} {}", fmt.as_str(), url);
            }
        }
    }
    Ok(())
}

async fn cmd_cache(
    client: &reqwest::Client,
    ids: Vec<String>,
    random: Option<usize>,
) -> Result<()> {
    let cache = Cache::open()?;
    let catalog = cache.load_catalog()?;
    let config = Config::load()?;
    let pref = config.quality.preference();

    // Resolve the target video ids.
    let targets: Vec<String> = if let Some(n) = random {
        let mut picked = Vec::new();
        for _ in 0..n {
            if let Some(v) = selector::pick(&catalog, None) {
                if !picked.contains(&v.id) {
                    picked.push(v.id.clone());
                }
            }
        }
        picked
    } else if !ids.is_empty() {
        ids
    } else {
        anyhow::bail!("specify video ids or --random N");
    };

    for id in &targets {
        let Some(video) = catalog.find(id) else {
            eprintln!("  {id} → not in catalog, skipping");
            continue;
        };
        match cache.download_video(client, video, pref).await {
            Ok(path) => println!("  {id} → {}", path.display()),
            Err(e) => eprintln!("  {id} → failed: {e:#}"),
        }
    }
    Ok(())
}

fn cmd_play(time: Option<&str>, count: usize, stream: bool, windowed: bool) -> Result<()> {
    let restrict = parse_time(time)?;
    let cache = Cache::open()?;
    let catalog = cache.load_catalog()?;
    let config = Config::load()?;
    let pref = config.quality.preference();

    let playlist = player::build_playlist(&catalog, &cache, pref, restrict, count, stream);
    if playlist.is_empty() {
        anyhow::bail!(
            "no clips to play — cache some with `aerial-linux cache --random N`, or pass --stream"
        );
    }
    let cached = playlist.iter().filter(|p| !p.starts_with("http")).count();
    eprintln!(
        "Playing {} clips ({cached} cached, {} streamed) on {:?}…",
        playlist.len(),
        playlist.len() - cached,
        player::DisplayServer::detect()
    );
    let opts = player::PlayOptions {
        fullscreen: !windowed,
        ..Default::default()
    };
    player::play(&playlist, &opts)?;
    Ok(())
}

fn cmd_status() -> Result<()> {
    let cache = Cache::open()?;
    println!("config: {}", Config::path()?.display());
    println!("cache:  {}", cache.root().display());
    match cache.load_catalog() {
        Ok(c) => println!("catalog: {} videos", c.len()),
        Err(_) => println!("catalog: (none — run `aerial-linux fetch`)"),
    }
    Ok(())
}
