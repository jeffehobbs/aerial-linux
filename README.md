# aerial-linux

A port of the macOS [Aerial](https://github.com/AerialScreensaver/Aerial)
screensaver to Ubuntu/Linux — playing Apple TV's aerial videos as an
idle-display screensaver.

**Target:** GNOME / Wayland first · **Language:** Rust · **Scope:** idle-display
(not a screen locker).

This is a ground-up reimplementation, not a code port: Aerial is macOS-native
Swift (ScreenSaver app-extension + AVFoundation + AppKit), none of which crosses
to Linux. What this project reuses is Aerial's *domain knowledge* — the Apple
manifest URLs, the `entries.json` schema, and the video-selection heuristics —
lifted from the MIT-licensed upstream. Apple's video content itself is streamed
from Apple's CDN exactly as Aerial does; nothing is redistributed.

## Status — Phase 1 (catalog + cache)

| Phase | Component | State |
|-------|-----------|-------|
| 0 | Recon (manifest schema, URLs, license) | ✅ done |
| 1 | **Catalog + cache layer** | ✅ this milestone |
| 2 | libmpv fullscreen player (Wayland xdg-toplevel) | ⬜ next |
| 3 | GNOME idle daemon (`org.gnome.Mutter.IdleMonitor`) | ⬜ |
| 4 | Overlays (clock/weather via mpv Lua; MPRIS now-playing) | ⬜ |
| 5 | Packaging (Flatpak / .deb, systemd --user unit) | ⬜ |

## Building

```sh
cargo build --release
```

The catalog/cache layer is platform-agnostic Rust and builds/runs on
macOS or Linux. The later player/idle phases are Linux-only.

## Usage

```sh
aerial-linux sources              # list Apple's aerial manifest sources
aerial-linux fetch                # download + merge manifests → catalog.json
aerial-linux list --time night    # list catalog, filtered by time of day
aerial-linux list --urls          # ...with the chosen download URL
aerial-linux cache --random 3     # pre-cache 3 random clips
aerial-linux cache <video-id> ... # pre-cache specific clips
aerial-linux status               # show cache/config paths + catalog size
```

## Layout

```
src/
  manifest.rs   entries.json schema (both tvOS and macOS manifest shapes)
  source.rs     known Apple manifest tarball URLs
  catalog.rs    fetch tarball → extract entries.json → normalized Video catalog
  cache.rs      XDG cache: catalog.json + streamed, md5-verified video downloads
  selector.rs   time-of-day filtering + random pick
  config.rs     ~/.config/aerial-linux/config.toml (quality, time-of-day)
  main.rs       CLI
```

Paths follow the XDG spec: catalog & videos under `~/.cache/aerial-linux/`,
config under `~/.config/aerial-linux/`.

## License

MIT, matching upstream Aerial.
