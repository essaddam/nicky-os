#!/bin/sh
# One synthetic data volume, 64 MiB first, then 128 MiB with recovery/stress.
set -eu
moli_archive_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
mkdir -p "$HOME/tmp"
moli_archive_logs=$(mktemp -d "$HOME/tmp/moli-c1-budgets-XXXXXXXX")
moli_archive_name="moli-archive-c1-budget-$$"
moli_archive_net="$moli_archive_name-net"
moli_archive_volume="$moli_archive_name-data"
cleanup() {
  docker logs "$moli_archive_name" > "$moli_archive_logs/server-final.log" 2>&1 || true
  docker rm -f "$moli_archive_name" "$moli_archive_name-client" >/dev/null 2>&1 || true
  docker volume rm "$moli_archive_volume" >/dev/null 2>&1 || true
  docker network rm "$moli_archive_net" >/dev/null 2>&1 || true
}
trap 'moli_archive_exit=$?; cleanup; date -u +%Y-%m-%dT%H:%M:%SZ > "$moli_archive_logs/end-utc"; printf "%s\n" "$moli_archive_exit" > "$moli_archive_logs/exit-code"; exit "$moli_archive_exit"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
date -u +%Y-%m-%dT%H:%M:%SZ > "$moli_archive_logs/start-utc"
printf 'JOURNAUX=%s\n'  "$moli_archive_logs"
printf '%s' 'fixture-archive-token-for-tests' > "$moli_archive_logs/token"
chmod 444 "$moli_archive_logs/token"
docker image inspect moli-archive:c1-test > "$moli_archive_logs/image.json"
moli_archive_extract=$(docker create moli-archive:c1-test)
docker cp "$moli_archive_extract:/moli-archive" "$moli_archive_logs/moli-archive"
docker export "$moli_archive_extract" | wc -c > "$moli_archive_logs/unpacked-tar-bytes"
docker rm "$moli_archive_extract" >/dev/null
python3 -I - "$moli_archive_logs" <<'PY_SIZE'
import pathlib,sys,json
root=pathlib.Path(sys.argv[1]); binary=(root/'moli-archive').stat().st_size; unpacked=int((root/'unpacked-tar-bytes').read_text())
assert binary<=25_000_000 and unpacked<=25_000_000,(binary,unpacked)
(root/'size.json').write_text(json.dumps({'binary_bytes':binary,'unpacked_tar_bytes':unpacked})+'\n')
PY_SIZE
python3 -I - "$moli_archive_logs/image.json" <<'PY'
import json,sys
image=json.load(open(sys.argv[1]))[0]
assert image['Size']<=25_000_000, image['Size']
print(json.dumps({'phase':'image','bytes':image['Size'],'source':image['Config']['Labels']['io.moli.source-sha']}))
PY
docker run --rm --cpus 1 -v "$moli_archive_root:/src:ro" -v "$moli_archive_logs:/proof" moli-os-dev rustc --edition 2024 -C opt-level=s -C strip=symbols /src/tools/archive-tests/resources.rs -o /proof/resources-probe
docker network create "$moli_archive_net" >/dev/null
docker volume create "$moli_archive_volume" >/dev/null
moli_archive_loaded=0
for moli_archive_limit in 64 128; do
  docker run -d --name "$moli_archive_name" --network "$moli_archive_net" --cpus 0.8 --memory "${moli_archive_limit}m" --memory-swap "${moli_archive_limit}m" --read-only --cap-drop ALL --security-opt no-new-privileges:true --pids-limit 32 -v "$moli_archive_volume:/data/archive" -v "$moli_archive_logs/token:/run/secrets/token:ro" moli-archive:c1-test serve --installation 00000000000000000000000000000002 --listen 0.0.0.0:8791 --token-file /run/secrets/token >/dev/null
  docker cp "$moli_archive_logs/resources-probe" "$moli_archive_name:/data/archive/resources-probe"
  moli_archive_extra=""
  if [ "$moli_archive_loaded" -eq 1 ]; then moli_archive_extra="--stress-only"; fi
  set +e
  # Client sees the workload generator only, never the service's data volume.
  docker run --name "$moli_archive_name-client" --network "$moli_archive_net" --cpus 0.2 -v "$moli_archive_root/tools/archive-tests:/tests:ro" python:3-alpine python /tests/load.py --url "http://$moli_archive_name:8791" --points "${MOLI_ARCHIVE_POINTS:-3024}" --samples "${MOLI_ARCHIVE_SAMPLES:-2322732}" --meters "${MOLI_ARCHIVE_METERS:-128}" --energy-hours "${MOLI_ARCHIVE_ENERGY_HOURS:-127452}" $moli_archive_extra > "$moli_archive_logs/load-$moli_archive_limit.log" 2>&1
  moli_archive_result=$?
  set -e
  docker inspect "$moli_archive_name-client" > "$moli_archive_logs/client-$moli_archive_limit.json"
  docker rm "$moli_archive_name-client" >/dev/null
  docker inspect "$moli_archive_name" > "$moli_archive_logs/server-$moli_archive_limit.json"
  docker logs "$moli_archive_name" > "$moli_archive_logs/server-$moli_archive_limit.log" 2>&1
  printf '%s\n' "$moli_archive_result" > "$moli_archive_logs/load-$moli_archive_limit-exit"
  if [ "$moli_archive_result" -eq 0 ]; then
    docker exec "$moli_archive_name" /data/archive/resources-probe > "$moli_archive_logs/resources-$moli_archive_limit.json"
    moli_archive_loaded=1
  else
    python3 -I - "$moli_archive_logs/server-$moli_archive_limit.json" "$moli_archive_limit" <<'PY'
import json,sys
state=json.load(open(sys.argv[1]))[0]['State']
assert sys.argv[2]=='64' and state['OOMKilled'], state
print(json.dumps({'phase':'fallback_128','reason':'64MiB_OOM'}))
PY
  fi
  cat "$moli_archive_logs/load-$moli_archive_limit.log"
  docker rm -f "$moli_archive_name" >/dev/null
done
python3 -I - "$moli_archive_logs" <<'PY'
import json,pathlib,sys
root=pathlib.Path(sys.argv[1]);report={}
for limit in (64,128):
    path=root/f'resources-{limit}.json'
    if path.exists():
        resource=json.loads(path.read_text());assert resource['oom_kills']==0,resource
        assert resource['peak_rss_kib']*1024<=limit*1024*1024,resource
        report[str(limit)]=resource
    else:
        assert limit==64
        report[str(limit)]={'passed':False,'reason':'OOM'}
assert (root/'load-128-exit').read_text().strip()=='0'
(root/'resources-summary.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps({'phase':'resource_proof',**report}))
PY
shasum -a 256 "$moli_archive_logs"/*.json "$moli_archive_logs"/*.log > "$moli_archive_logs/logs.sha256"
printf 'EXIT=0\n'
