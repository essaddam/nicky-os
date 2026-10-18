#!/bin/sh
set -eu
cargo fmt --all
cargo clippy -p moli-archive -p moli-rollup --all-targets --locked -- -D warnings
cargo test -p moli-archive -p moli-rollup --locked --no-fail-fast -- --test-threads=1
cargo tree -p moli-archive -e normal
