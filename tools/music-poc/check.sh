#!/bin/sh
set -eu
cd "$(dirname "$0")/../.."
export RUSTUP_TOOLCHAIN=1.99.1
export CARGO_BUILD_JOBS=2
export CARGO_TARGET_DIR="$PWD/target/music-poc"
manifest=tools/music-poc/Cargo.toml
cargo fmt --manifest-path "$manifest" -- --check
cargo clippy --manifest-path "$manifest" --all-targets -j 2 -- -D warnings
cargo test --manifest-path "$manifest" -j 2
cargo run --manifest-path "$manifest" -j 2 -- demo merged
cargo run --manifest-path "$manifest" -j 2 -- demo isolated
cargo run --manifest-path "$manifest" -j 2 -- native-status
