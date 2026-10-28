#!/bin/sh
# Rejoue la série du banc « les ordres des appareils » comme la production : compilation en Linux
# musl (rust:1-alpine, hors ligne, registre cargo du poste en lecture seule, volume de compilation à
# part `moli-banc-ordres-target`), tests dans le conteneur de compilation, puis chaque mesure dans
# trois conteneurs neufs limités à 2 CPU et 128 Mio, sans échange. La série retient la médiane des
# trois essais (tools/memoire-banc/scripts/mediane.py, repris tel quel). Sorties dans sorties/linux/.
#
# Le banc inclut des fichiers du banc mémoire (outils, maison, valeurs) et du banc du nettoyage (le
# registre) : les trois arbres sont copiés dans le conteneur, à leur place relative. Colima ne monte
# que le dossier personnel : les sources passent par `docker cp`.
# Usage : sh tools/ordres-banc/scripts/linux.sh [section …]
set -eu
P=$(cd "$(dirname "$0")/.." && pwd)
T=$(cd "$P/../.." && pwd)
O="$P/sorties/linux"
rm -rf "$P/linux-bins"; mkdir -p "$O"
ID=$(docker create -v "$HOME/.cargo/registry:/usr/local/cargo/registry:ro" -v moli-banc-ordres-target:/target \
  -w /w/tools/ordres-banc rust:1-alpine sh -c '
  set -e
  mkdir -p /out
  CARGO_TARGET_DIR=/target cargo test --release --offline 2>&1 | grep -E "^test |test result|^error" || true
  CARGO_TARGET_DIR=/target cargo build --release --offline 2>&1 | grep -E "^error|Finished" || true
  cp /target/release/banc-ordres /out/banc-ordres
  rustc --version')
COPYFILE_DISABLE=1 tar --no-xattrs --no-mac-metadata -C "$T" -s ',^tools,w/tools,' -cf - tools/ordres-banc/Cargo.toml tools/ordres-banc/Cargo.lock \
  tools/ordres-banc/src tools/memoire-banc/src tools/nettoyage-banc/src | docker cp - "$ID:/"
docker start -a "$ID" | tee "$O/tests.txt"
docker cp "$ID:/out" "$P/linux-bins" >/dev/null
docker rm "$ID" >/dev/null

lancer() { # lancer <section> [échelle] : trois conteneurs neufs, la médiane des trois
  for k in 1 2 3; do
    C=$(docker create --cpuset-cpus=0,1 --memory=128m --memory-swap=128m alpine:3 /bin/banc "$@")
    docker cp "$P/linux-bins/banc-ordres" "$C:/bin/banc" >/dev/null
    docker start -a "$C" > "$P/linux-bins/essai-$k.txt" 2>&1 || true
    docker inspect -f 'CONTENEUR|{{.State.OOMKilled}}|{{.State.ExitCode}}' "$C" >> "$P/linux-bins/essai-$k.txt"
    docker rm "$C" >/dev/null
  done
  python3 -I "$T/tools/memoire-banc/scripts/mediane.py" "$P/linux-bins/essai-1.txt" "$P/linux-bins/essai-2.txt" "$P/linux-bins/essai-3.txt"
}

SECTIONS=${*:-"soiree catalogue demandes clients"}
for s in $SECTIONS; do
  case $s in catalogue|demandes|clients) E="1 10 100" ;; *) E="1" ;; esac
  for e in $E; do lancer "$s" "$e" > "$O/$s-x$e.txt" 2>&1; done
done
{
  echo "machine : $(sysctl -n machdep.cpu.brand_string 2>/dev/null || uname -m)"
  echo "docker : $(docker info --format '{{.ServerVersion}} · {{.NCPU}} CPU · {{.MemTotal}} o')"
  echo "conteneurs : 2 CPU, 128 Mio sans échange, trois essais par mesure (médiane)"
  echo "charge à la fin de la série : $(uptime | sed 's/.*load averages*: //')"
} > "$O/machine.txt" 2>&1
grep -h CONTENEUR "$O"/*.txt | sort | uniq -c
echo FINI
