# CURRENTLY BROKEN
Spotify recently blocked many essential features from their API, which allowed users to stream audio and do other things on their platform.
This decision broke many other open source applications which also relied on their API. Thanks Spotify for nothing.

[More information to this problem](https://community.spotify.com/t5/Spotify-for-Developers/Web-API-Get-Track-s-Audio-Features-403-error/m-p/6654507/highlight/true#M16618)

<div align="center">
  <img src="assets/logo.svg" alt="Spotix logo" width="96" height="96" />
  <h1>Spotix</h1>
  <p>Fast, native Spotify client written in Rust — low overhead, clean UI, lightweight runtime (no Electron).</p>
  <p>
    <a href="https://github.com/skyline69/spotix/releases/latest">Latest Release</a>
    •
    <a href="https://github.com/skyline69/spotix/issues">Issues</a>
  </p>
</div>

<img width="1917" height="1079" alt="image" src="https://github.com/user-attachments/assets/82fb24a9-62fd-4475-b59e-6804d3532e1a" />

<img width="1922" height="1080" alt="image" src="https://github.com/user-attachments/assets/f993a6c5-9d96-48d4-a0ba-2e05d0bf2ec3" />


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

## Download

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

## Build
- Rust stable (1.65.0 or newer)

### Linux dependencies
- Debian/Ubuntu: `sudo apt-get install libssl-dev libgtk-3-dev libcairo2-dev libasound2-dev`
- RHEL/Fedora: `sudo dnf install openssl-devel gtk3-devel cairo-devel alsa-lib-devel`

### OpenBSD (WIP)
```shell
doas pkg_add gtk+3 cairo llvm
export LIBCLANG_PATH=/usr/local/lib
```
If you hit rustc memory errors while building gtk:
```shell
ulimit -d $(( 2 * `ulimit -d` ))
```

### Build from source
```shell
cargo build
# Add --release for release builds.
```

### Run from source
```shell
cargo run --bin spotix-gui
# Add --release for release builds.
```

### Build app bundle (macOS)
```shell
cargo install cargo-bundle
cargo bundle --release
```
## Cool Features compared to psst

### Equalizer with all the Spotify presets
<img width="412" height="448" alt="image" src="https://github.com/user-attachments/assets/b0def49f-4b99-46fc-9fb1-20aea44cc913" />

<img width="249" height="290" alt="image" src="https://github.com/user-attachments/assets/76202a2e-54bf-468d-b521-400b57d1bb34" />

### Crossfade
<img width="250" height="78" alt="image" src="https://github.com/user-attachments/assets/c523446e-81ba-4085-a852-33632111e339" />

### Autoplay using Spotify's algorithm
<img width="299" height="70" alt="image" src="https://github.com/user-attachments/assets/c294b1ac-222d-4e86-a772-4d41c4aa8a9c" />

### Full Caching support for maximum performance!

<img width="407" height="452" alt="image" src="https://github.com/user-attachments/assets/f6fce925-6a01-4a2e-9ef5-4135fc864771" />

### And some more
- Up to date dependencies
- Clean codebase
- and much more...

## Built-in Themes

### Gruvbox Dark
<img width="938" height="501" alt="image" src="https://github.com/user-attachments/assets/d5ef9dff-8fe0-4450-90cf-2f63fa04a967" />

### Dracula
<img width="943" height="513" alt="image" src="https://github.com/user-attachments/assets/2979b8f8-04f1-437c-8610-62f315f1f0db" />

### Kanagawa
<img width="942" height="511" alt="image" src="https://github.com/user-attachments/assets/04bf1d41-fbd0-4960-80c6-8029f6296e64" />

### Any many more!

<img width="338" height="415" alt="image" src="https://github.com/user-attachments/assets/6e02bb5c-825a-4c0c-ae49-50c3335b9d53" />

## Theming
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

## Project layout
- `/spotix-core` core library (session, decoding, playback)
- `/spotix-gui` GUI app (Druid)
- `/spotix-cli` minimal CLI example

## Privacy
Spotix connects only to official Spotify servers.
Credentials are not stored; a reusable token is used instead.
Cached data is stored locally and can be deleted at any time.

## Credits
- librespot: https://github.com/librespot-org/librespot
- druid: https://github.com/linebender/druid
- ncspot: https://github.com/hrkfdn/ncspot
