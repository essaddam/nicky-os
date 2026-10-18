#!/bin/sh
# Exercise the unchanged deployment model, with no house network, data or token.
set -eu
moli_archive_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
moli_archive_logs=$(mktemp -d "$HOME/tmp/moli-c1-compose-XXXXXXXX")
moli_archive_project="moli-c1-compose-$$"
moli_archive_name="$moli_archive_project-core"
mkdir -p "$moli_archive_logs/data"
chmod 777 "$moli_archive_logs/data"
cat > "$moli_archive_logs/data/moli.toml" <<'TOML'
[server]
listen = "127.0.0.1:8790"
data_dir = "/data"
worker_threads = 1
TOML
cat > "$moli_archive_logs/override.yaml" <<YAML
services:
  moli-os:
    image: moli-os-dev:latest
    container_name: $moli_archive_name
    user: '1000:1000'
    restart: 'no'
    network_mode: none
    cpus: 1
    entrypoint: [/target/debug/moli-os]
    command: [serve, --config, /data/moli.toml]
    environment:
      MOLI_CONFIG: /data/moli.toml
      # Public fixture material; never read or replace a house's master key.
      MOLI_MASTER_KEY: fixture-compose-only-not-a-real-secret
    volumes: !override
      - $moli_archive_logs/data:/data
      - moli-c1-compiled:/target:ro
volumes:
  moli-c1-compiled:
    external: true
    name: moli-target-stockage-c1-archive
YAML
compose() {
  env -u COMPOSE_PROFILES MOLI_ARCHIVE_TOKEN_FILE="$moli_archive_logs/absent-token" docker compose --env-file /dev/null -p "$moli_archive_project" -f "$moli_archive_root/compose.yaml" -f "$moli_archive_logs/override.yaml" "$@"
}
cleanup() {
  docker logs "$moli_archive_name" > "$moli_archive_logs/core.log" 2>&1 || true
  compose down > "$moli_archive_logs/down.log" 2>&1 || true
}
trap 'moli_archive_exit=$?; cleanup; date -u +%Y-%m-%dT%H:%M:%SZ > "$moli_archive_logs/end-utc"; printf "%s\n" "$moli_archive_exit" > "$moli_archive_logs/exit-code"; exit "$moli_archive_exit"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
date -u +%Y-%m-%dT%H:%M:%SZ > "$moli_archive_logs/start-utc"
printf 'JOURNAUX=%s\n' "$moli_archive_logs"
docker compose version > "$moli_archive_logs/version"
printf 'docker compose --env-file /dev/null -p %s -f compose.yaml -f override.yaml up -d --no-build\nCOMPOSE_PROFILES unset; token path does not exist; core binary from check.sh; network none\n' "$moli_archive_project" > "$moli_archive_logs/command"
test ! -e "$moli_archive_logs/absent-token"
compose config --services > "$moli_archive_logs/services"
test "$(cat "$moli_archive_logs/services")" = moli-os
compose up -d --no-build > "$moli_archive_logs/up.log" 2>&1
cat "$moli_archive_logs/up.log"
moli_archive_ready=0
for moli_archive_attempt in $(seq 1 20); do
  if docker exec "$moli_archive_name" /target/debug/moli-os health > "$moli_archive_logs/health.log" 2>&1; then
    moli_archive_ready=1
    break
  fi
  sleep 1
done
test "$moli_archive_ready" -eq 1
compose ps --format json > "$moli_archive_logs/ps.json"
python3 -I - "$moli_archive_logs/ps.json" <<'PY'
import json,sys
rows=[json.loads(line) for line in open(sys.argv[1]) if line.strip()]
assert len(rows)==1 and rows[0]['Service']=='moli-os' and rows[0]['State']=='running',rows
print('CORE_RUNNING=1 ARCHIVE_STARTED=0 TOKEN_FILE=absent')
PY
test ! -e "$moli_archive_logs/absent-token"
shasum -a 256 "$moli_archive_root/compose.yaml" "$moli_archive_logs"/*.log > "$moli_archive_logs/logs.sha256"
printf 'EXIT=0\n'
