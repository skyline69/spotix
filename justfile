default:
    @just --list

build:
    cargo build

build-release:
    cargo build --release

run *ARGS:
    cargo run --bin spotix-gui -- {{ARGS}}

run-release *ARGS:
    cargo run --bin spotix-gui --release -- {{ARGS}}

test *ARGS:
    cargo test {{ARGS}}

check:
    cargo check

clippy *ARGS:
    cargo clippy {{ARGS}} -- -D warnings

fmt:
    cargo fmt

fmt-check:
    cargo fmt -- --check

clean:
    cargo clean

bundle:
    cargo bundle --release

deps-debian:
    sudo apt-get install libssl-dev libgtk-3-dev libcairo2-dev libasound2-dev

deps-fedora:
    sudo dnf install openssl-devel gtk3-devel cairo-devel alsa-lib-devel
