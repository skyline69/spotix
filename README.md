# Spotix

Spotix is a fast, native Spotify client written in Rust.
It focuses on low overhead, a clean UI, and a lightweight runtime (no Electron).

<img width="1091" height="1019" alt="Spotix UI" src="https://github.com/user-attachments/assets/6f993dd5-8000-407b-8980-9cdc5a0855bc" />

## Fork notice
- This project is a fork of https://github.com/jpochyla/psst
- The fork is maintained by skyline69 and diverges in naming, packaging, and ongoing changes

## Additional features over upstream psst
- Theme support with TOML themes (including custom colors and lyric highlighting)
- Spotify-style default dark theme with bundled Spotify Mix fonts
- Configurable cache size limit and cache usage display in preferences
- Playlist pagination toggle and real-time library search (playlists, albums, tracks, podcasts, albums)
- Saved playback state restoration (resume last track/position) and improved lyrics view (highlight, focus, auto-scroll)
- More fluid seek bar, bottom-bar cover click opens album, and platform release binaries
- More up-to-date dependencies and ongoing maintenance on the fork
- Automatic retry for transient network timeouts and throttling
- Built-in 10-band equalizer with presets and custom tuning
- Multi-select playlist mode with select all and bulk remove actions

## Status
- Early development; expect missing features and rough edges
- Requires a Spotify Premium account




Download

GitHub Actions build and publish releases when changes land on `main`.
Grab the latest installers from the [Releases page](https://github.com/skyline69/spotix/releases/latest).

| Platform               | Download Link                                                                               |
| ---------------------- | ------------------------------------------------------------------------------------------- |
| Linux (x86_64)         | [Download](https://github.com/skyline69/spotix/releases/latest/download/spotix-linux-x86_64)  |
| Linux (aarch64)        | [Download](https://github.com/skyline69/spotix/releases/latest/download/spotix-linux-aarch64)|
| Debian Package (amd64) | [Download](https://github.com/skyline69/spotix/releases/latest/download/spotix-amd64.deb)    |
| Debian Package (arm64) | [Download](https://github.com/skyline69/spotix/releases/latest/download/spotix-arm64.deb)    |
| macOS                  | [Download](https://github.com/skyline69/spotix/releases/latest/download/Spotix.dmg)          |
| Windows                | [Download](https://github.com/skyline69/spotix/releases/latest/download/Spotix.exe)          |

Build
- Rust stable (1.65.0 or newer)

Linux dependencies
- Debian/Ubuntu: `sudo apt-get install libssl-dev libgtk-3-dev libcairo2-dev libasound2-dev`
- RHEL/Fedora: `sudo dnf install openssl-devel gtk3-devel cairo-devel alsa-lib-devel`

OpenBSD (WIP)
```shell
doas pkg_add gtk+3 cairo llvm
export LIBCLANG_PATH=/usr/local/lib
```
If you hit rustc memory errors while building gtk:
```shell
ulimit -d $(( 2 * `ulimit -d` ))
```

Build from source
```shell
cargo build
# Add --release for release builds.
```

Run from source
```shell
cargo run --bin spotix-gui
# Add --release for release builds.
```

Build app bundle (macOS)
```shell
cargo install cargo-bundle
cargo bundle --release
```

Theming
- Place TOML theme files in `~/.config/Spotix/themes/`.
- Spotix ships with multiple preset themes that auto-install into that folder on first run.
- Each theme file must include a `name` field (e.g. `name = "catppuccin"`) and color keys. Example:
```toml
name = "catppuccin"
base = "dark"

[colors]
grey_000 = "#cdd6f4"
grey_100 = "#bac2de"
grey_200 = "#a6adc8"
grey_300 = "#585b70"
grey_400 = "#45475a"
grey_500 = "#313244"
grey_600 = "#181825"
grey_700 = "#1e1e2e"
blue_100 = "#a6e3a1"
blue_200 = "#89b4fa"
red = "#f38ba8"
link_hot = "#ffffff14"
link_active = "#ffffff0f"
link_cold = "#00000000"
lyric_highlight = "#cba6f7"
lyric_past = "#6c7086"
lyric_hover = "#cdd6f4"
playback_toggle_bg_active = "#a6e3a1"
playback_toggle_bg_inactive = "#313244"
playback_toggle_fg_active = "#1e1e2e"
icon_color = "#8e95b4"
icon_color_muted = "#6c7086"
media_control_icon = "#cdd6f4"
media_control_icon_muted = "#a6adc8"
media_control_border = "#585b70"
status_text_color = "#bac2de"
```
- Select themes in Settings → General. Custom themes are listed by their `name`.

Project layout
- `/spotix-core` core library (session, decoding, playback)
- `/spotix-gui` GUI app (Druid)
- `/spotix-cli` minimal CLI example

Privacy
Spotix connects only to official Spotify servers.
Credentials are not stored; a reusable token is used instead.
Cached data is stored locally and can be deleted at any time.

Credits
- librespot: https://github.com/librespot-org/librespot
- druid: https://github.com/linebender/druid
- ncspot: https://github.com/hrkfdn/ncspot
