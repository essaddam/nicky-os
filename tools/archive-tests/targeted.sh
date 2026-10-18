#!/bin/sh
set -eu
moli_archive_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
moli_archive_logs=$(mktemp -d "$HOME/tmp/moli-c1-targeted-XXXXXXXX")
git -C "$moli_archive_root" rev-parse HEAD > "$moli_archive_logs/base"
(cd "$moli_archive_root" && rg --files Cargo.toml Cargo.lock crates/moli-archive crates/moli-rollup crates/moli-history/Cargo.toml crates/moli-history/src/lib.rs | sort | xargs shasum -a 256) > "$moli_archive_logs/sources-before.sha256"
printf 'JOURNAUX=%s\n' "$moli_archive_logs"
set +e
docker run --rm --name moli-archive-targeted --cpus 1 -v "$moli_archive_root:/src" -v moli-cargo:/usr/local/cargo/registry -v moli-target-stockage-c1-archive:/target -e CARGO_TARGET_DIR=/target -e CARGO_BUILD_JOBS=1 -w /src moli-os-dev sh tools/archive-tests/in-container.sh > "$moli_archive_logs/tests.log" 2>&1
moli_archive_exit=$?
set -e
printf '%s\n' "$moli_archive_exit" > "$moli_archive_logs/exit-code"
(cd "$moli_archive_root" && rg --files Cargo.toml Cargo.lock crates/moli-archive crates/moli-rollup crates/moli-history/Cargo.toml crates/moli-history/src/lib.rs | sort | xargs shasum -a 256) > "$moli_archive_logs/sources-after.sha256"
shasum -a 256 "$moli_archive_logs/tests.log" > "$moli_archive_logs/log.sha256"
tail -n 60 "$moli_archive_logs/tests.log"
printf 'EXIT=%s\n' "$moli_archive_exit"
exit "$moli_archive_exit"
