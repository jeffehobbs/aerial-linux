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

## Status — Phase 2 (fullscreen player)

| Phase | Component | State |
|-------|-----------|-------|
| 0 | Recon (manifest schema, URLs, license) | ✅ done |
| 1 | Catalog + cache layer | ✅ done |
| 2 | **mpv fullscreen player (Wayland)** | ✅ this milestone |
| 3 | GNOME idle daemon (`org.gnome.Mutter.IdleMonitor`) | ⬜ next |
| 4 | Overlays (clock/weather via mpv Lua; MPRIS now-playing) | ⬜ |
| 5 | Packaging (Flatpak / .deb, systemd --user unit) | ⬜ |

Validated on a GNOME/Wayland + NVIDIA box: clips stream/play fullscreen via
the Vulkan pipeline (`--vo=gpu-next --gpu-api=vulkan --gpu-context=waylandvk`),
hardware-decoded.

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
aerial-linux play                 # play cached clips fullscreen (Esc/q quits)
aerial-linux play --stream        # ...including not-yet-cached clips (streamed)
aerial-linux play --time night --count 5 --windowed
aerial-linux status               # show cache/config paths + catalog size
```

`play` drives the `mpv` binary (see `src/player.rs` for why a subprocess rather
than libmpv FFI). On Wayland it uses the Vulkan video pipeline; override mpv
flags via `$AERIAL_MPV_ARGS` if your hardware needs something different.

### Notes / known issues

- **Apple CDN TLS:** `sylvan.apple.com` uses Apple's private "Apple Server
  Authentication CA" and doesn't send the intermediate, so the chain can't be
  built on Linux. The Apple-CDN HTTP client therefore skips chain verification
  (it only ever fetches public video assets). See `apple_cdn_client()` in
  `src/main.rs`.
- **NVIDIA/Wayland:** needs the NVIDIA GL/EGL userspace (`libnvidia-gl-*`) and
  `libnvidia-egl-wayland1` installed and matching the kernel module, plus the
  Vulkan render path above — the OpenGL/EGL path can mis-select Mesa and fall
  back to software rendering.

## Layout

```
src/
  manifest.rs   entries.json schema (both tvOS and macOS manifest shapes)
  source.rs     known Apple manifest tarball URLs
  catalog.rs    fetch tarball → extract entries.json → normalized Video catalog
  cache.rs      XDG cache: catalog.json + streamed, md5-verified video downloads
  player.rs     fullscreen playback by driving the mpv binary (Wayland/Vulkan)
  selector.rs   time-of-day filtering + random pick
  config.rs     ~/.config/aerial-linux/config.toml (quality, time-of-day)
  main.rs       CLI
```

Paths follow the XDG spec: catalog & videos under `~/.cache/aerial-linux/`,
config under `~/.config/aerial-linux/`.

## License

MIT, matching upstream Aerial.
