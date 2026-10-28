#!/bin/sh
# Rejoue la série du banc « nettoyage des équipements » comme la production : compilation en Linux
# musl (rust:1-alpine, hors ligne, registre cargo du poste en lecture seule, volume de compilation à
# part `moli-banc-nettoyage-target`), tests dans le conteneur de compilation, puis chaque mesure dans
# trois conteneurs neufs limités à 2 CPU et 128 Mio, sans échange. La série retient la médiane des
# trois essais, et pour un pire cas (`_max`), le pire des trois (tools/memoire-banc/scripts/mediane.py,
# repris tel quel). Sorties dans sorties/linux/ (quelques minutes).
#
# Le banc inclut trois fichiers du banc mémoire (outils, maison, valeurs) : les deux arbres sont
# copiés dans le conteneur, à leur place relative. Colima ne monte que le dossier personnel : les
# sources passent par `docker cp`.
# Usage : sh tools/nettoyage-banc/scripts/linux.sh [section …]
set -eu
P=$(cd "$(dirname "$0")/.." && pwd)
T=$(cd "$P/../.." && pwd)
O="$P/sorties/linux"
rm -rf "$P/linux-bins"; mkdir -p "$O"
ID=$(docker create -v "$HOME/.cargo/registry:/usr/local/cargo/registry:ro" -v moli-banc-nettoyage-target:/target \
  -w /w/tools/nettoyage-banc rust:1-alpine sh -c '
  set -e
  mkdir -p /out
  CARGO_TARGET_DIR=/target cargo test --release --offline 2>&1 | grep -E "^test |test result|^error" || true
  CARGO_TARGET_DIR=/target cargo build --release --offline 2>&1 | grep -E "^error|Finished" || true
  cp /target/release/banc-nettoyage /out/banc-nettoyage
  rustc --version')
COPYFILE_DISABLE=1 tar --no-xattrs --no-mac-metadata -C "$T" -s ',^tools,w/tools,' -cf - tools/nettoyage-banc/Cargo.toml tools/nettoyage-banc/Cargo.lock \
  tools/nettoyage-banc/src tools/memoire-banc/src | docker cp - "$ID:/"
docker start -a "$ID" | tee "$O/tests.txt"
docker cp "$ID:/out" "$P/linux-bins" >/dev/null
docker rm "$ID" >/dev/null

lancer() { # lancer <section> [échelle] : trois conteneurs neufs, la médiane des trois
  for k in 1 2 3; do
    C=$(docker create --cpuset-cpus=0,1 --memory=128m --memory-swap=128m alpine:3 /bin/banc "$@")
    docker cp "$P/linux-bins/banc-nettoyage" "$C:/bin/banc" >/dev/null
    docker start -a "$C" > "$P/linux-bins/essai-$k.txt" 2>&1 || true
    docker inspect -f 'CONTENEUR|{{.State.OOMKilled}}|{{.State.ExitCode}}' "$C" >> "$P/linux-bins/essai-$k.txt"
    docker rm "$C" >/dev/null
  done
  python3 -I "$T/tools/memoire-banc/scripts/mediane.py" "$P/linux-bins/essai-1.txt" "$P/linux-bins/essai-2.txt" "$P/linux-bins/essai-3.txt"
}

SECTIONS=${*:-"refonte lecture lot vie"}
for s in $SECTIONS; do
  case $s in lecture|lot) E="1 10 100" ;; vie) E="1 10" ;; *) E="1" ;; esac
  for e in $E; do lancer "$s" "$e" > "$O/$s-x$e.txt" 2>&1; done
done
{
  echo "machine : $(sysctl -n machdep.cpu.brand_string 2>/dev/null || uname -m)"
  echo "docker : $(docker info --format '{{.ServerVersion}} · {{.NCPU}} CPU · {{.MemTotal}} o')"
  echo "conteneurs : 2 CPU, 128 Mio sans échange, trois essais par mesure (médiane ; le pire pour _max)"
  echo "charge à la fin de la série : $(uptime | sed 's/.*load averages*: //')"
} > "$O/machine.txt" 2>&1
grep -h CONTENEUR "$O"/*.txt | sort | uniq -c
echo FINI
