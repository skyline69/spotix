# Spotix

Spotix is a fast, native Spotify client written in Rust.
It focuses on low overhead, a clean UI, and a lightweight runtime (no Electron).

Fork notice
- This project is a fork of https://github.com/jpochyla/psst
- The fork is maintained by skyline69 and diverges in naming, packaging, and ongoing changes

Status
- Early development; expect missing features and rough edges
- Requires a Spotify Premium account

Screenshot
![Spotix UI](./spotix-gui/assets/screenshot.png)

Downloads
- Not published yet. Build from source for now.

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
