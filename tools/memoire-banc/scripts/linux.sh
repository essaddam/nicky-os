#!/bin/sh
# Rejoue toute la série, comme la production : compilation en Linux musl (rust:1-alpine, binaire
# statique, hors ligne, registre cargo du poste en lecture seule, volume de compilation à part
# `moli-banc-memoire-target`), tests du scénario dans le conteneur de compilation, puis chaque mesure
# dans un conteneur neuf limité à 2 CPU et 128 Mio, sans échange, trois fois : la série retient la
# médiane des trois essais, qu'une charge passagère de la machine ne déplace pas, et pour un pire
# cas (`_max`), le pire des trois (scripts/mediane.py). Sorties dans sorties/linux/ (environ 20 min).
#
# Colima ne monte que le dossier personnel : les sources passent par `docker cp`.
# Usage : sh tools/memoire-banc/scripts/linux.sh [section …]
# Avec des sections, seules celles-ci sont rejouées, à leurs échelles habituelles (scenario : 1, 100
# et 400 copies ; precision : ×1 et ×10 ; les autres : ×1, ×3 et ×10) ; un document dit alors quelles
# mesures ont été rejouées seules.
set -eu
P=$(cd "$(dirname "$0")/.." && pwd)
O="$P/sorties/linux"
rm -rf "$P/linux-bins"; mkdir -p "$O"
ID=$(docker create -v "$HOME/.cargo/registry:/usr/local/cargo/registry:ro" -v moli-banc-memoire-target:/target \
  -w /src rust:1-alpine sh -c '
  set -e
  mkdir -p /out
  CARGO_TARGET_DIR=/target cargo test --release --offline 2>&1 | grep -E "^test |test result|^error" || true
  CARGO_TARGET_DIR=/target cargo build --release --offline 2>&1 | grep -E "^error|Finished" || true
  CARGO_TARGET_DIR=/target cargo build --profile vitesse --offline 2>&1 | grep -E "^error|Finished" || true
  cp /target/release/banc-memoire /out/banc-memoire
  cp /target/vitesse/banc-memoire /out/banc-memoire-vitesse
  rustc --version')
docker cp "$P/Cargo.toml" "$ID:/src/Cargo.toml" >/dev/null
docker cp "$P/Cargo.lock" "$ID:/src/Cargo.lock" >/dev/null
docker cp "$P/src" "$ID:/src/src" >/dev/null
docker start -a "$ID" | tee "$O/tests-scenario.txt"
docker cp "$ID:/out" "$P/linux-bins" >/dev/null
docker rm "$ID" >/dev/null

lancer() { # lancer <binaire> <section> [échelle] : trois conteneurs neufs, la médiane des trois
  B=$1; shift
  for k in 1 2 3; do
    C=$(docker create --cpuset-cpus=0,1 --memory=128m --memory-swap=128m alpine:3 /bin/banc "$@")
    docker cp "$P/linux-bins/$B" "$C:/bin/banc" >/dev/null
    docker start -a "$C" > "$P/linux-bins/essai-$k.txt" 2>&1 || true
    docker inspect -f 'CONTENEUR|{{.State.OOMKilled}}|{{.State.ExitCode}}' "$C" >> "$P/linux-bins/essai-$k.txt"
    docker rm "$C" >/dev/null
  done
  python3 -I "$P/scripts/mediane.py" "$P/linux-bins/essai-1.txt" "$P/linux-bins/essai-2.txt" "$P/linux-bins/essai-3.txt"
}

if [ $# -gt 0 ]; then
  for s in "$@"; do
    case $s in scenario) E="1 100 400" ;; precision) E="1 10" ;; *) E="1 3 10" ;; esac
    for e in $E; do lancer banc-memoire "$s" "$e" > "$O/$s-x$e.txt" 2>&1; done
  done
  grep -h CONTENEUR "$O"/*.txt | sort | uniq -c
  echo FINI
  exit 0
fi

for s in valeurs echeances workflows conditions decisions; do
  for e in 1 3 10; do
    lancer banc-memoire "$s" "$e" > "$O/$s-x$e.txt" 2>&1
  done
done
lancer banc-memoire valeurs 100 > "$O/valeurs-x100.txt" 2>&1
lancer banc-memoire echeances 100 > "$O/echeances-x100.txt" 2>&1
lancer banc-memoire passage > "$O/passage.txt" 2>&1
lancer banc-memoire-vitesse passage > "$O/passage-vitesse.txt" 2>&1
lancer banc-memoire-vitesse echeances 1 > "$O/echeances-vitesse-x1.txt" 2>&1
for c in 1 100 400; do lancer banc-memoire scenario "$c" > "$O/scenario-x$c.txt" 2>&1; done
for e in 1 10; do lancer banc-memoire precision "$e" > "$O/precision-x$e.txt" 2>&1; done
grep -h CONTENEUR "$O"/*.txt | sort | uniq -c
echo FINI
