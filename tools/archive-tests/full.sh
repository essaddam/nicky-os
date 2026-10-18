#!/bin/sh
# Required workspace check, language catalogues, then the approved isolated budgets.
set -eu
moli_archive_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
mkdir -p "$HOME/tmp"
moli_archive_logs=$(mktemp -d "$HOME/tmp/moli-c1-full-XXXXXXXX")
date -u +%Y-%m-%dT%H:%M:%SZ > "$moli_archive_logs/start-utc"
trap 'moli_archive_exit=$?; date -u +%Y-%m-%dT%H:%M:%SZ > "$moli_archive_logs/end-utc"; printf "%s\n" "$moli_archive_exit" > "$moli_archive_logs/exit-code"; exit "$moli_archive_exit"' EXIT
cd "$moli_archive_root"
moli_archive_common=$(git rev-parse --path-format=absolute --git-common-dir)
moli_archive_gitdir=$(git rev-parse --absolute-git-dir)
moli_archive_subdir=${moli_archive_gitdir#"$moli_archive_common/"}
if [ "$moli_archive_gitdir" = "$moli_archive_common" ]; then moli_archive_subdir=""; fi
printf 'JOURNAUX=%s\n' "$moli_archive_logs"
git rev-parse HEAD > "$moli_archive_logs/base"
git diff --binary HEAD > "$moli_archive_logs/candidate.patch"
set +e
docker run --name moli-archive-full-check --cpus 1 -v "$moli_archive_root:/src" -v "$moli_archive_common:/git:ro" -v moli-cargo:/usr/local/cargo/registry -v moli-target-stockage-c1-archive:/target -e CARGO_TARGET_DIR=/target -e CARGO_BUILD_JOBS=1 -e RUST_TEST_THREADS=1 -e GIT_OPTIONAL_LOCKS=0 -e "GIT_DIR=/git/$moli_archive_subdir" -e GIT_WORK_TREE=/src -e GIT_CONFIG_COUNT=1 -e GIT_CONFIG_KEY_0=safe.directory -e GIT_CONFIG_VALUE_0=/src -w /src moli-os-dev sh -ec 'apk add --no-cache git >/dev/null
sh scripts/check.sh' > "$moli_archive_logs/check.log" 2>&1
moli_archive_result=$?
set -e
docker inspect moli-archive-full-check > "$moli_archive_logs/check-container.json"
docker rm moli-archive-full-check >/dev/null
printf '%s\n' "$moli_archive_result" > "$moli_archive_logs/check-exit"
tail -n 45 "$moli_archive_logs/check.log"
if [ "$moli_archive_result" -ne 0 ]; then exit "$moli_archive_result"; fi
docker run --rm --cpus 1 -v "$moli_archive_root:/src:ro" -w /src node:24-alpine node scripts/i18n-check.mjs > "$moli_archive_logs/i18n.log" 2>&1
cat "$moli_archive_logs/i18n.log"
git diff --binary HEAD > "$moli_archive_logs/candidate-after.patch"
cmp "$moli_archive_logs/candidate.patch" "$moli_archive_logs/candidate-after.patch"
shasum -a 256 "$moli_archive_logs/check.log" "$moli_archive_logs/i18n.log" "$moli_archive_logs/candidate.patch" > "$moli_archive_logs/logs.sha256"
docker run --rm --name moli-archive-msrv --cpus 1 -v "$moli_archive_root:/src:ro" -v moli-cargo:/usr/local/cargo/registry -v moli-target-stockage-c1-msrv:/target -e CARGO_TARGET_DIR=/target -e CARGO_BUILD_JOBS=1 -w /src rust:1.88-alpine sh -ec 'apk add --no-cache musl-dev >/dev/null
cargo check -p moli-archive --all-targets --locked' > "$moli_archive_logs/msrv.log" 2>&1
sh "$moli_archive_root/tools/archive-tests/budgets.sh"
printf '0\n' > "$moli_archive_logs/exit-code"
printf 'EXIT=0\n'
