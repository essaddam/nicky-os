#!/bin/sh
# Banc Turso de l'ADR D13, d'une seule commande :
#
#   sh tools/storage-bench/run.sh <dashboard-workload-shape.json> [répétitions]
#
# - construit les binaires statiques musl pour linux/arm64 (image rust:1-alpine),
#   puis l'image FROM scratch qui ne contient que le banc (Dockerfile voisin),
#   et mesure les sondes de taille (sans moteur, SQLite, Turso, les deux,
#   `turso_core` seul) ;
# - vérifie la compilation au minimum Rust du dépôt (1.88) ;
# - lance chaque essai dans l'image FROM scratch, profil seccomp par défaut de
#   Docker, 1 CPU et 128 Mio, base sur un volume Docker (ext4, pas un dossier
#   partagé avec l'hôte : fsync y garde son sens) ;
# - compte les fsync par validation sous strace ;
# - répète la charge ×2 et ses mesures, puis résume la dispersion.
#
# Le fichier de forme vient de l'étude privée : il est monté en lecture seule,
# jamais copié. Les résultats (results/, ignoré par git) ne contiennent que
# des durées, des tailles et des comptes.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
shape=$(cd "$(dirname "${1:?chemin de dashboard-workload-shape.json}")" && pwd)/$(basename "$1")
repeat=${2:-3}
platform=${PLATFORM:-linux/arm64}
volume=moli-storage-bench
out=$here/results/$(date -u +%Y%m%dT%H%M%SZ)
mkdir -p "$out"
log() { printf '%s\n' "$*" >&2; }

mkdir -p "$here/target-linux"

# Compile dans l'image Rust officielle (musl), caches dans des volumes Docker.
build_in() {
    image=$1
    target=$2
    shift 2
    docker run --rm --platform "$platform" -v "$here:/src" -w /src \
        -v moli-storage-bench-cargo:/usr/local/cargo/registry -v "$target:/target" \
        -e CARGO_TARGET_DIR=/target "$image" sh -c "apk add --no-cache musl-dev >/dev/null && $*"
}
log "construction ($platform, musl statique)…"
bins="storage-bench size_none size_sqlite size_turso size_both"
build_in rust:1-alpine moli-storage-bench-target \
    "rustc --version > /src/target-linux/rustc-version && cargo build --release --locked -q \
     && for b in $bins; do cp /target/release/\$b /src/target-linux/; done" 2>"$out/build.log"
# Le moteur seul (`turso_core`, fonctions minimales) : crate à part, pour que
# les fonctions par défaut de `turso` ne s'y ajoutent pas.
build_in rust:1-alpine moli-storage-bench-target-core \
    "cd size-core && cargo build --release --locked -q && cp /target/release/size-core /src/target-linux/" 2>>"$out/build.log"
bins="$bins size-core"
{
    cat "$here/target-linux/rustc-version"
    echo "# binaire, octets : strip=debuginfo (profil de Moli), strip complet, strip complet + gzip -9"
    docker run --rm --platform "$platform" -v "$here/target-linux:/b:ro" alpine:3.22 sh -c \
        "apk add --no-cache binutils >/dev/null && for f in $bins; do strip -o /tmp/s /b/\$f \
         && echo \"\$f \$(wc -c </b/\$f) \$(wc -c </tmp/s) \$(gzip -9c /tmp/s | wc -c)\"; done"
} >"$out/sizes.txt"
# Facultatif (MOLI_IMAGE=1, une vingtaine de minutes) : le binaire de Moli tel
# qu'il est, étapes du Dockerfile racine rejouées sans BuildKit (tableau de
# bord compris), pour situer le delta dans le budget d'image de 25 Mo.
if [ "${MOLI_IMAGE:-0}" = 1 ]; then
    repo=$(cd "$here/../.." && pwd)
    log "binaire actuel de Moli ($platform)…"
    docker run --rm --platform "$platform" -v "$repo:/repo:ro" -v moli-storage-bench-web:/out node:24-alpine sh -c \
        "cp -r /repo/web /web && cp -r /repo/locales /locales && cd /web && rm -rf node_modules dist \
         && npm ci --no-audit --no-fund >/dev/null && npm run build >/dev/null && rm -rf /out/dist && cp -r dist /out/" \
        >>"$out/build.log" 2>&1
    docker run --rm --platform "$platform" -v "$repo:/repo:ro" -v moli-storage-bench-web:/web:ro \
        -v moli-storage-bench-cargo:/usr/local/cargo/registry -v moli-storage-bench-target-moli:/target \
        -v "$here/target-linux:/b" -e CARGO_TARGET_DIR=/target rust:1-alpine sh -c \
        "apk add --no-cache musl-dev >/dev/null && mkdir /src && tar -C /repo --exclude=./target --exclude=./tools \
         --exclude=./web/node_modules --exclude=./web/dist -cf - . | tar -C /src -xf - && cp -r /web/dist /src/web/dist \
         && cd /src && cargo build --release --locked -q -p moli-os && cp /target/release/moli-os /b/" \
        >>"$out/build.log" 2>&1
    docker run --rm --platform "$platform" -v "$here/target-linux:/b:ro" alpine:3.22 sh -c \
        "apk add --no-cache binutils >/dev/null && strip -o /tmp/s /b/moli-os \
         && echo \"moli-os \$(wc -c </b/moli-os) \$(wc -c </tmp/s) \$(gzip -9c /tmp/s | wc -c)\"" >>"$out/sizes.txt"
fi
docker build -q --platform "$platform" -t moli-storage-bench -f "$here/Dockerfile" "$here/target-linux" >/dev/null
docker image inspect moli-storage-bench --format 'image {{.Size}}' >>"$out/sizes.txt"
log "minimum Rust (1.88)…"
if build_in rust:1.88-alpine moli-storage-bench-target-msrv 'cargo check --release --locked' >"$out/msrv.log" 2>&1; then
    echo '{"msrv":"1.88","check":"ok"}' >"$out/msrv.json"
else
    echo '{"msrv":"1.88","check":"failed"}' >"$out/msrv.json"
fi

docker volume create "$volume" >/dev/null
run() {
    docker run --rm --platform "$platform" --cpus 1 --memory 128m -e TURSO_TMPDIR=/bench/tmp \
        -v "$volume:/bench" -v "$shape:/fixtures/shape.json:ro" moli-storage-bench "$@"
}

log "compatibilité…"
run compat /bench/compat >"$out/compat.jsonl"
log "kill -9…"
run crash /bench/crash 30 >"$out/crash.jsonl"
log "fsync par validation…"
for sync in FULL NORMAL OFF; do
    docker run --rm --platform "$platform" --cap-add SYS_PTRACE -v "$volume:/bench" \
        -v "$here/target-linux:/b:ro" alpine:3.22 sh -c "apk add --no-cache strace >/dev/null \
        && strace -f -c -e trace=fsync,fdatasync -o /bench/strace-$sync.txt \
           /b/storage-bench fsync-probe /bench/fsync-$sync.db $sync 200 && cat /bench/strace-$sync.txt >&2" \
        >"$out/fsync-$sync.jsonl" 2>"$out/strace-$sync.txt"
done

i=1
while [ "$i" -le "$repeat" ]; do
    log "charge ×2, tour $i/${repeat}…"
    # Charge de l'hôte (les autres conteneurs comptent) : pour lire la dispersion.
    echo "tour $i : $(docker run --rm alpine:3.22 cat /proc/loadavg)" >>"$out/host-load.txt"
    run load /fixtures/shape.json /bench/hot.db >"$out/load-$i.jsonl"
    run reads /fixtures/shape.json /bench/hot.db >"$out/reads-$i.jsonl"
    run burst /fixtures/shape.json /bench/hot.db 4 >"$out/burst-4-$i.jsonl"
    run burst /fixtures/shape.json /bench/hot.db 974 >"$out/burst-974-$i.jsonl"
    run reopen /fixtures/shape.json /bench/hot.db >"$out/reopen-$i.jsonl"
    run purge /fixtures/shape.json /bench/hot.db >"$out/purge-$i.jsonl"
    i=$((i + 1))
done

python3 -I "$here/summary.py" "$out" | tee "$out/summary.txt"
log "résultats : $out"
