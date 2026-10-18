#!/bin/sh
# Build the real scratch image and run the same HTTP suite in a separate client.
set -eu
moli_archive_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
mkdir -p "$HOME/tmp"
moli_archive_logs=$(mktemp -d "$HOME/tmp/moli-c1-network-XXXXXXXX")
moli_archive_tag="moli-archive:c1-test"
moli_archive_name="moli-archive-c1-$$"
moli_archive_net="moli-archive-c1-net-$$"
moli_archive_volume="moli-archive-c1-data-$$"
cleanup() {
  docker logs "$moli_archive_name" > "$moli_archive_logs/server.log" 2>&1 || true
  docker rm -f "$moli_archive_name" >/dev/null 2>&1 || true
  docker volume rm "$moli_archive_volume" >/dev/null 2>&1 || true
  docker network rm "$moli_archive_net" >/dev/null 2>&1 || true
}
trap 'moli_archive_exit=$?; cleanup; printf "%s\n" "$moli_archive_exit" > "$moli_archive_logs/exit-code"; exit "$moli_archive_exit"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
cd "$moli_archive_root"
git rev-parse HEAD > "$moli_archive_logs/base"
git ls-files --cached --others --exclude-standard -- Cargo.toml Cargo.lock crates/moli-archive crates/moli-rollup crates/moli-history/Cargo.toml crates/moli-history/src/lib.rs docker/archive.Dockerfile locales/fr/archive.json locales/en/archive.json | sort | xargs shasum -a 256 > "$moli_archive_logs/sources-before.sha256"
moli_archive_source=$(shasum -a 256 "$moli_archive_logs/sources-before.sha256" | cut -d ' ' -f 1)
printf 'JOURNAUX=%s\n' "$moli_archive_logs"
docker build --cpu-period=100000 --cpu-quota=100000 --build-arg "BUILD_SOURCE_SHA=$moli_archive_source" -t "$moli_archive_tag" -f docker/archive.Dockerfile . > "$moli_archive_logs/build.log" 2>&1
docker image inspect "$moli_archive_tag" > "$moli_archive_logs/image.json"
docker run --rm --cpus 1 -v "$moli_archive_root:/src" -v moli-cargo:/usr/local/cargo/registry -v moli-target-stockage-c1-archive:/target -e CARGO_TARGET_DIR=/target -e CARGO_BUILD_JOBS=1 -w /src moli-os-dev cargo test -p moli-archive --test http --locked --no-run --message-format=json > "$moli_archive_logs/artifacts.jsonl" 2> "$moli_archive_logs/client-build.log"
moli_archive_test=$(python3 -I - "$moli_archive_logs/artifacts.jsonl" <<'PY'
import json,sys
matches=[]
for line in open(sys.argv[1]):
    row=json.loads(line)
    if row.get('reason')=='compiler-artifact' and row.get('target',{}).get('name')=='http' and row.get('executable'):
        matches.append(row['executable'])
assert len(matches)==1, matches
print(matches[0])
PY
)
printf '%s' 'fixture-archive-token-for-tests' > "$moli_archive_logs/token"
chmod 444 "$moli_archive_logs/token"
docker network create "$moli_archive_net" >/dev/null
docker volume create "$moli_archive_volume" >/dev/null
docker run -d --name "$moli_archive_name" --network "$moli_archive_net" --cpus 0.8 --memory 128m --memory-swap 128m --read-only --cap-drop ALL --security-opt no-new-privileges:true --pids-limit 32 -v "$moli_archive_volume:/data/archive" -v "$moli_archive_logs/token:/run/secrets/token:ro" "$moli_archive_tag" serve --installation 00000000000000000000000000000002 --listen 0.0.0.0:8791 --token-file /run/secrets/token > "$moli_archive_logs/server-id"
for moli_archive_round in 1 2; do
  if [ "$moli_archive_round" -eq 2 ]; then
    docker kill --signal KILL "$moli_archive_name" >/dev/null
    docker start "$moli_archive_name" >/dev/null
    docker run --rm --network "$moli_archive_net" --cpus 0.2 -v "$moli_archive_root/tools/archive-tests:/tests:ro" python:3-alpine python /tests/interrupted.py --url "http://$moli_archive_name:8791" --phase verify --first "$moli_archive_crash_first" > "$moli_archive_logs/crash-verify.log" 2>&1
  fi
  # Client mounts compiled test code only. It never sees the archive data volume.
  docker run --rm --name "$moli_archive_name-client" --network "$moli_archive_net" --cpus 0.2 -v moli-target-stockage-c1-archive:/target:ro -e "ARCHIVE_TEST_URL=http://$moli_archive_name:8791" moli-os-dev "$moli_archive_test" --test-threads=1 --nocapture > "$moli_archive_logs/http-$moli_archive_round.log" 2>&1
  cat "$moli_archive_logs/http-$moli_archive_round.log"
  if [ "$moli_archive_round" -eq 1 ]; then
    docker run --rm --network "$moli_archive_net" --cpus 0.2 -v "$moli_archive_root/tools/archive-tests:/tests:ro" python:3-alpine python /tests/interrupted.py --url "http://$moli_archive_name:8791" --phase send > "$moli_archive_logs/crash-send.json" 2> "$moli_archive_logs/crash-send.log"
    moli_archive_crash_first=$(python3 -I -c 'import json,sys;print(json.load(open(sys.argv[1]))["first"])' "$moli_archive_logs/crash-send.json")
  fi
done
python3 -I - "$moli_archive_logs" <<'PY'
import json,pathlib,sys
root=pathlib.Path(sys.argv[1]); proofs=[]
for path in [root/'http-1.log',root/'http-2.log']:
    for line in path.read_text().splitlines():
        if '"phase":"http_proof"' in line:
            proofs.append(json.loads(line[line.index('{'):line.rindex('}')+1]))
assert len(proofs)==2, proofs
assert proofs[0]['generation']==proofs[1]['generation'], proofs
assert proofs[1]['published_upto']==proofs[0]['published_upto']+1802, proofs
(root/'proof.json').write_text(json.dumps({'same_epoch_after_sigkill':True,'http_rounds':proofs},indent=2)+'\n')
PY
git ls-files --cached --others --exclude-standard -- Cargo.toml Cargo.lock crates/moli-archive crates/moli-rollup crates/moli-history/Cargo.toml crates/moli-history/src/lib.rs docker/archive.Dockerfile locales/fr/archive.json locales/en/archive.json | sort | xargs shasum -a 256 > "$moli_archive_logs/sources-after.sha256"
cmp "$moli_archive_logs/sources-before.sha256" "$moli_archive_logs/sources-after.sha256"
shasum -a 256 "$moli_archive_logs"/*.log "$moli_archive_logs/proof.json" > "$moli_archive_logs/logs.sha256"
printf '0\n' > "$moli_archive_logs/exit-code"
printf 'EXIT=0\n'
