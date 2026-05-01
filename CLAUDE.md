# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

A `justfile` at the repo root wraps the common workflows (`just --list` to see them). The raw cargo equivalents:

```bash
# Build all crates (debug)
cargo build

# Build release
cargo build --release

# Run the GUI app
cargo run --bin spotix-gui
cargo run --bin spotix-gui --release

# Check code style (matches CI)
cargo clippy -- -D warnings

# Format code
cargo fmt

# Build macOS app bundle
cargo install cargo-bundle
cargo bundle --release
```

### Platform Dependencies

**Linux (Debian/Ubuntu):**
```bash
sudo apt-get install libssl-dev libgtk-3-dev libcairo2-dev libasound2-dev
```

**Linux (RHEL/Fedora):**
```bash
sudo dnf install openssl-devel gtk3-devel cairo-devel alsa-lib-devel
```

## Architecture

Spotix is a native Spotify client (fork of psst) organized as a Rust workspace with two crates:

### spotix-core
Core library handling Spotify connectivity and audio:
- `session/` - Spotify authentication (OAuth, login5, tokens) and Mercury protocol messaging
- `player/` - Playback control, queue management, audio file loading, and worker threads
- `audio/` - Audio pipeline: decryption, decoding (symphonia), resampling, normalization, and 10-band equalizer
- `connection/` - Low-level Spotify protocol connection (Shannon encryption)
- `cache.rs`, `cdn.rs` - Track caching and CDN file fetching

### spotix-gui
Druid-based GUI application:
- `ui/` - View modules for each screen (home, library, playlist, album, artist, lyrics, preferences, etc.)
- `controller/` - Event handlers including `playback.rs` (main playback controller)
- `data/` - Application state models and configuration (`config.rs` for user preferences)
- `widget/` - Custom Druid widgets
- `webapi/` - Spotify Web API client
- `delegate.rs` - Main Druid app delegate connecting UI to core

## Audio Backend Features

- Default audio backend: cpal (cross-platform)
- Alternative: cubeb (Mozilla's audio library) - enable with `--features cubeb`
- Audio processing: decryption -> Vorbis/MP3 decode -> resample -> normalize -> EQ -> output

## Theming

Custom themes are TOML files in `~/.config/Spotix/themes/`. Each theme defines color keys and a `name` field. Theme selection is in Settings -> General.
