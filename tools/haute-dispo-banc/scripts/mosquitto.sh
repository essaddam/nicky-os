#!/bin/sh
# Le pont de Mosquitto 2, pour comparer avec le pont à reprise : session persistante
# (`cleansession false`), QoS 1, file en mémoire (`persistence false`). Deux courtiers sur un réseau
# Docker ; le secours tire `moli/v1/#` de l'actif. Un éditeur publie des numéros à 200 messages/s
# pendant 30 s ; le lien du secours est coupé de la 10e à la 20e seconde ; un abonné du secours note
# ce qui arrive. Trois cas : la file par défaut (1 000 messages par client), une file de 100 000, et
# cette grande file quand le courtier de l'actif redémarre pendant la coupure.
# Usage : sh tools/haute-dispo-banc/scripts/mosquitto.sh   (sortie : sorties/linux/mosquitto.txt)
set -eu
P=$(cd "$(dirname "$0")/.." && pwd)
O="$P/sorties/linux/mosquitto.txt"
IMG=eclipse-mosquitto:2
NET=hd-mosq
nettoyer() { docker rm -f hd-a hd-b >/dev/null 2>&1 || true; docker network rm $NET >/dev/null 2>&1 || true; }

actif() { # actif <file> : le courtier de l'actif, file par client bornée à <file> messages
  docker run -d --name hd-a --network $NET $IMG sh -c "printf 'listener 1883\nallow_anonymous true\npersistence false\nmax_queued_messages $1\n' > /tmp/a.conf && exec mosquitto -c /tmp/a.conf" >/dev/null
}
secours() { # le courtier du secours et son pont vers l'actif
  docker run -d --name hd-b --network $NET $IMG sh -c "printf 'listener 1883\nallow_anonymous true\npersistence false\nconnection pont\naddress hd-a:1883\ntopic moli/v1/# in 1\ncleansession false\nremote_clientid pont-secours\nkeepalive_interval 5\nrestart_timeout 1\n' > /tmp/b.conf && exec mosquitto -c /tmp/b.conf" >/dev/null
}
publier() { # publier <de> <à> : 200 messages par seconde, QoS 1, sur l'actif
  docker exec hd-a sh -c "i=$1; while [ \$i -le $2 ]; do echo \$i; [ \$((i % 20)) -eq 0 ] && sleep 0.1; i=\$((i+1)); done | mosquitto_pub -h localhost -q 1 -t moli/v1/val/compteur -l"
}

cas() { # cas <nom> <file> <redémarrer : 0|1>
  nettoyer
  docker network create $NET >/dev/null
  actif "$2"; secours; sleep 3
  docker exec -d hd-b sh -c "mosquitto_sub -h localhost -q 1 -t 'moli/v1/#' > /tmp/recu.txt"; sleep 1
  publier 1 2000
  docker network disconnect $NET hd-b
  publier 2001 4000
  if [ "$3" = 1 ]; then docker restart hd-a >/dev/null; sleep 2; fi
  docker network connect $NET hd-b
  sleep 8
  publier 4001 6000
  sleep 15
  TOTAL=$(docker exec hd-b sh -c 'wc -l < /tmp/recu.txt' | tr -d ' ')
  UNIQ=$(docker exec hd-b sh -c 'sort -n -u /tmp/recu.txt | wc -l' | tr -d ' ')
  JETES=$(docker logs hd-a 2>&1 | grep -c "dropped" || true)
  echo "RESULTAT|mosquitto|1|$1_file|$2|messages"
  echo "RESULTAT|mosquitto|1|$1_publies|6000|messages"
  echo "RESULTAT|mosquitto|1|$1_recus|$UNIQ|messages"
  echo "RESULTAT|mosquitto|1|$1_perdus|$((6000 - UNIQ))|messages"
  echo "RESULTAT|mosquitto|1|$1_doublons|$((TOTAL - UNIQ))|messages"
  echo "RESULTAT|mosquitto|1|$1_alertes_courtier|$JETES|lignes"
}

{
  echo "mosquitto : $(docker run --rm $IMG sh -c 'mosquitto -h 2>&1 | grep -o "version [0-9.]*" | head -1')"
  cas file_defaut 1000 0
  cas grande_file 100000 0
  cas redemarrage 100000 1
} > "$O" 2>&1
nettoyer
cat "$O"
