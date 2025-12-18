# Spotix

Spotix is a fast, native Spotify client written in Rust.
It focuses on low overhead, a clean UI, and a lightweight runtime (no Electron).

Fork notice
- This project is a fork of https://github.com/jpochyla/psst
- The fork is maintained by skyline69 and diverges in naming, packaging, and ongoing changes

Additional features over upstream psst
- Theme support with TOML themes (including custom colors and lyric highlighting)
- Configurable cache size limit and cache usage display in preferences
- Playlist pagination toggle and real-time library search (playlists, albums, tracks, podcasts, albums)
- Saved playback state restoration (resume last track/position) and improved lyrics view (highlight, focus)
- More fluid seek bar, bottom-bar cover click opens album, and platform release binaries
- More up-to-date dependencies and ongoing maintenance on the fork

Status
- Early development; expect missing features and rough edges
- Requires a Spotify Premium account

Screenshot
![Spotix UI](./spotix-gui/assets/screenshot.png)

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
- Each theme file must include a `name` field (e.g. `name = "catppuccin"`) and color keys. Example:
```toml
name = "catppuccin"
primary = "#b4befe"
secondary = "#cba6f7"
background = "#1e1e2e"
foreground = "#cdd6f4"
highlight = "#f38ba8"
lyric_highlight = "#cba6f7"
lyric_past = "#6c7086"
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
