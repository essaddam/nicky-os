#!/bin/sh
# One allocated synthetic check; all retained output goes through the security tool.
set -eu
: "${MOLI_SECURITY_PROOF_ROOT:?prepare temporary security artifact storage first}"
moli_security_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
moli_security_logs=$(mktemp -d "$MOLI_SECURITY_PROOF_ROOT/targeted-XXXXXXXX")
cd "$moli_security_root"
printf 'JOURNAUX=%s\n' "$moli_security_logs"
date -u +%Y-%m-%dT%H:%M:%SZ > "$moli_security_logs/start-utc"
git rev-parse HEAD > "$moli_security_logs/base"
git diff --binary HEAD > "$moli_security_logs/candidate.patch"
(rg --files Cargo.toml Cargo.lock crates/moli-z2m crates/moli-core crates/moli-runtime crates/moli-history locales/fr/systeme.json locales/en/systeme.json web/src/components/Journal.svelte tools/z2m-privacy | sort | xargs shasum -a 256) > "$moli_security_logs/sources-before.sha256"
set +e
docker run --name moli-security-z2m-targeted --cpus 1 --network none -v "$moli_security_root:/src:ro" -v moli-cargo:/usr/local/cargo/registry -v moli-target-security-z2m:/target -e CARGO_TARGET_DIR=/target -e CARGO_BUILD_JOBS=1 -e RUST_TEST_THREADS=1 -w /src moli-os-dev sh -c 'cargo fmt --all -- --check
moli_security_fmt=$?
cargo clippy -p moli-core -p moli-runtime -p moli-history -p moli-z2m --all-targets --locked --offline --keep-going -- -D warnings
moli_security_lint=$?
cargo test -p moli-core -p moli-runtime -p moli-history -p moli-z2m --locked --offline --no-fail-fast -- --test-threads=1
moli_security_tests=$?
printf "FMT_EXIT=%s CLIPPY_EXIT=%s TEST_EXIT=%s\n" "$moli_security_fmt" "$moli_security_lint" "$moli_security_tests"
if [ "$moli_security_tests" -ne 0 ]; then exit "$moli_security_tests"; fi
if [ "$moli_security_lint" -ne 0 ]; then exit "$moli_security_lint"; fi
exit "$moli_security_fmt"' > "$moli_security_logs/tests.log" 2>&1
moli_security_exit=$?
set -e
docker inspect moli-security-z2m-targeted > "$moli_security_logs/container.json"
docker rm moli-security-z2m-targeted >/dev/null
if [ "$moli_security_exit" -eq 0 ]; then
    # Repeat just the legacy 64 x 2 MiB repair under the production hard
    # ceiling. No compiler or Cargo process shares this memory allowance.
    moli_security_history=$(sed -n 's@.*Running unittests.*(\(/target/debug/deps/moli_history-[[:xdigit:]]*\))@\1@p' "$moli_security_logs/tests.log")
    test -n "$moli_security_history"
    set +e
    docker run --name moli-security-z2m-memory --cpus 1 --memory 128m --memory-swap 128m --network none -v moli-target-security-z2m:/target:ro -e "MOLI_PRIVACY_TEST_BINARY=$moli_security_history" moli-os-dev sh -c '"$MOLI_PRIVACY_TEST_BINARY" --exact privacy::tests::large_ordinary_json_is_scanned_without_accumulating_a_text_batch --test-threads=1 &
moli_security_pid=$!
(sleep 120
if kill -0 "$moli_security_pid" 2>/dev/null; then
    printf "MEMORY_TIMEOUT=120s\n" > /tmp/moli-memory-timeout
    kill -KILL "$moli_security_pid" 2>/dev/null
fi) &
moli_security_timer=$!
wait "$moli_security_pid"
moli_security_test=$?
kill "$moli_security_timer" 2>/dev/null
wait "$moli_security_timer" 2>/dev/null
printf "MEMORY_PROCESS_EXIT=%s\n" "$moli_security_test"
if [ -f /tmp/moli-memory-timeout ]; then
    cat /tmp/moli-memory-timeout
    moli_security_test=124
fi
cat /sys/fs/cgroup/memory.peak
printf "MEMORY_EXIT=%s\n" "$moli_security_test"
exit "$moli_security_test"' > "$moli_security_logs/memory.log" 2>&1
    moli_security_exit=$?
    set -e
    if [ "$moli_security_exit" -eq 0 ] && ! rg -q '^test privacy::tests::large_ordinary_json_is_scanned_without_accumulating_a_text_batch \.\.\. ok$' "$moli_security_logs/memory.log"; then
        printf 'MEMORY_TEST_MISSING\n' >> "$moli_security_logs/memory.log"
        moli_security_exit=1
    fi
    docker inspect moli-security-z2m-memory > "$moli_security_logs/memory-container.json"
    docker rm moli-security-z2m-memory >/dev/null
    printf '%s\n' "$moli_security_exit" > "$moli_security_logs/memory-exit"
fi
printf '%s\n' "$moli_security_exit" > "$moli_security_logs/exit-code"
date -u +%Y-%m-%dT%H:%M:%SZ > "$moli_security_logs/end-utc"
(rg --files Cargo.toml Cargo.lock crates/moli-z2m crates/moli-core crates/moli-runtime crates/moli-history locales/fr/systeme.json locales/en/systeme.json web/src/components/Journal.svelte tools/z2m-privacy | sort | xargs shasum -a 256) > "$moli_security_logs/sources-after.sha256"
cmp "$moli_security_logs/sources-before.sha256" "$moli_security_logs/sources-after.sha256"
shasum -a 256 "$moli_security_logs"/*.log "$moli_security_logs/candidate.patch" > "$moli_security_logs/logs.sha256"
tail -n 45 "$moli_security_logs/tests.log"
printf 'EXIT=%s\n' "$moli_security_exit"
exit "$moli_security_exit"
