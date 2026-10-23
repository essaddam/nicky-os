"""Octets écrits sur disque par jour, selon la disposition des relevés et le rythme des validations.

Rejoue une vraie journée de relevés (comptes horaires réels anonymisés de l'étude privée, la
charge ×1 du banc D13) sur SQLite, dont Turso reprend le format de pages. Écrit un rapport daté
`docs/reports/mesure-ecritures-<série>.md` : la série est l'instant UTC de la mesure. Ces mesures
fondent les § 13.7 à 13.9 de `docs/proposition/valeurs-appareils-stockage.md`.

Usage : python3 -I tools/storage-writes/measure.py <dashboard-workload-shape.json> [dossier des rapports]
"""

import datetime
import hashlib
import json
import pathlib
import platform
import random
import sqlite3
import statistics
import subprocess
import sys
import tempfile
import time

PAGE = 4096
FRAME = 24 + PAGE  # en-tête d'une trame du WAL, puis la page
WAL_LIMIT = 64 * 1024 * 1024  # point de contrôle manuel au-delà
HOUR = 3_600_000
H0 = 1_760_000_000_000 - 1_760_000_000_000 % HOUR  # début d'heure arbitraire, en ms
PREFILL_HOURS = range(0, 120)  # jours 1 à 5 : l'historique déjà présent
DAY_HOURS = range(120, 144)  # jour 6 : la journée rejouée
ROOT = pathlib.Path(__file__).resolve().parents[2]
TOOL = pathlib.Path(__file__).resolve()
PROPOSAL = "../proposition/valeurs-appareils-stockage.md#137-double-écriture--létat-courant-complet-dans-maisondb"

# Schémas : ceux de la proposition (§ 14) et ceux du banc D13 (tools/storage-bench/src/schema.rs).
LEAN_SAMPLES = "CREATE TABLE samples(id INTEGER PRIMARY KEY, seq INTEGER NOT NULL, num ANY, txt TEXT, q INTEGER) STRICT"
OPS = ("CREATE TABLE ops(seq INTEGER PRIMARY KEY, kind INTEGER NOT NULL, point INTEGER NOT NULL,"
       " ts INTEGER NOT NULL, num ANY, txt TEXT, cost ANY) STRICT")
PROGRESS = "CREATE TABLE progress(task INTEGER PRIMARY KEY, upto INTEGER NOT NULL) STRICT"
EXPORT_BATCH = "CREATE TABLE export_batch(first INTEGER PRIMARY KEY, bytes BLOB NOT NULL, published_at INTEGER) STRICT"
POINT_STATE = ("CREATE TABLE point_state(id INTEGER PRIMARY KEY, point TEXT NOT NULL UNIQUE, num ANY, txt TEXT,"
               " observed_at INTEGER, received_at INTEGER NOT NULL, changed_at INTEGER) STRICT")
D13 = """
CREATE TABLE meta(id INTEGER PRIMARY KEY CHECK(id=1), next_seq INTEGER NOT NULL) STRICT;
CREATE TABLE samples(id INTEGER PRIMARY KEY, point_id INTEGER NOT NULL, ts INTEGER NOT NULL, seq INTEGER NOT NULL,
                     kind TEXT NOT NULL, num REAL, txt TEXT) STRICT;
CREATE TABLE outbox(seq INTEGER PRIMARY KEY, kind INTEGER NOT NULL, ref INTEGER NOT NULL) STRICT;
CREATE TABLE export_journal(seq INTEGER PRIMARY KEY, payload TEXT NOT NULL, published_at INTEGER) STRICT;
"""


def events(series, phase, hours):
    """(ts_ms, point) triés : les n relevés d'une heure régulièrement espacés, phase par série."""
    out = []
    for p, s in enumerate(series):
        for h in hours:
            n = s["hours"][h]
            out.extend((H0 + h * HOUR + int((k + phase[p]) * HOUR / n), p) for k in range(n))
    out.sort()
    return out


def wal_pages(path):
    """Pages distinctes présentes dans le WAL : ce que le prochain point de contrôle recopie."""
    pages = set()
    with open(path, "rb") as f:
        f.seek(32)
        while len(header := f.read(24)) == 24:
            pages.add(int.from_bytes(header[:4], "big"))
            f.seek(PAGE, 1)
    return len(pages)


class Meter:
    """Une base WAL dont on compte les octets écrits : trames ajoutées au WAL et pages recopiées."""

    def __init__(self, path):
        self.wal = pathlib.Path(f"{path}-wal")
        self.wal_bytes = self.ckpt_pages = self.commits = self.base = 0
        self.c = sqlite3.connect(path, isolation_level=None)
        # On compte des octets, pas des fsync : en FULL, chaque validation ajoute un fsync.
        for pragma in (f"page_size={PAGE}", "journal_mode=WAL", "synchronous=OFF", "wal_autocheckpoint=0"):
            self.c.execute(f"PRAGMA {pragma}")

    def checkpoint(self, count=True):
        if count and self.wal.exists() and self.wal.stat().st_size > 32:
            self.ckpt_pages += wal_pages(self.wal)
        self.c.execute("PRAGMA wal_checkpoint(TRUNCATE)")
        self.base = 0

    def reset(self):
        """Oublie le pré-remplissage : seule la journée rejouée est mesurée."""
        self.checkpoint(count=False)
        self.wal_bytes = self.ckpt_pages = self.commits = 0

    def commit(self):
        self.c.execute("COMMIT")
        self.commits += 1
        size = self.wal.stat().st_size
        self.wal_bytes += size - self.base
        self.base = size
        if size > WAL_LIMIT:
            self.checkpoint()

    def tx(self, *statements):
        self.c.execute("BEGIN")
        for sql, args in statements:
            self.c.execute(sql, args)
        self.commit()

    def count(self, table):
        return self.c.execute(f"SELECT count(*) FROM {table}").fetchone()[0]

    def close(self):
        """Dernier point de contrôle compté, puis le bilan."""
        self.checkpoint()
        self.c.close()
        return {
            "validations": self.commits,
            "wal_mo": round(self.wal_bytes / 1e6, 1),
            "checkpoint_mo": round(self.ckpt_pages * PAGE / 1e6, 1),
            "total_mo": round((self.wal_bytes + self.ckpt_pages * PAGE) / 1e6, 1),
            "pages_par_validation": round(self.wal_bytes / FRAME / max(self.commits, 1), 2),
        }


def value(r):
    return r.randint(1000, 30000)  # décimal mis à l'échelle de la source : 2 octets


def prefill_lean(c, history):
    c.execute(LEAN_SAMPLES)
    r = random.Random(1)
    c.execute("BEGIN")
    c.executemany("INSERT INTO samples(id, seq, num) VALUES (?, ?, ?)",
                  sorted(((p << 44) + ts, i, value(r)) for i, (ts, p) in enumerate(history)))
    c.execute("COMMIT")


def point_layout(work, history, day, cadence_s):
    """samples rangé par point et écrit relevé par relevé : le rangement de D13, sans l'export."""
    m = Meter(work / f"point-{cadence_s}.db")
    prefill_lean(m.c, history)
    m.reset()
    r, seq0, window = random.Random(7), len(history), None
    for i, (ts, p) in enumerate(day):
        if ts // (cadence_s * 1000) != window:
            if window is not None:
                m.commit()
            m.c.execute("BEGIN")
            window = ts // (cadence_s * 1000)
        m.c.execute("INSERT INTO samples(id, seq, num) VALUES (?, ?, ?)", ((p << 44) + ts, seq0 + i, value(r)))
    m.commit()
    assert m.count("samples") == len(history) + len(day)
    return m.close()


def payload(names, seq, p, ts, v):
    record = {"type": "sample", "seq": seq, "point": names[p], "ts": ts, "kind": "num", "num": v / 100, "txt": None}
    return json.dumps(record, separators=(",", ":"))


def d13_path(work, history, day, names, window_ms):
    """D13 tel que décidé : relevé + outbox + journal NDJSON par opération, meta par transaction,
    accusé par lots de 5 000, journal purgé par préfixes après sept jours."""
    m = Meter(work / f"d13-{window_ms}.db")
    c = m.c
    c.executescript(D13)
    r = random.Random(1)
    c.execute("BEGIN")
    c.executemany("INSERT INTO samples VALUES (?, ?, ?, ?, 'num', ?, NULL)",
                  sorted(((p << 44) + ts, p, ts, i, value(r) / 100) for i, (ts, p) in enumerate(history)))
    c.executemany("INSERT INTO export_journal VALUES (?, ?, 1)",
                  ((i, payload(names, i, p, ts, 1234)) for i, (ts, p) in enumerate(history)))
    c.execute("INSERT INTO meta VALUES (1, ?)", (len(history),))
    c.execute("COMMIT")
    m.reset()
    seq, window, published = len(history), None, len(history)
    r = random.Random(7)
    for ts, p in day:
        if ts // window_ms != window:
            if window is not None:
                c.execute("UPDATE meta SET next_seq = ? WHERE id = 1", (seq,))
                m.commit()
            c.execute("BEGIN")
            window = ts // window_ms
        v = value(r)
        c.execute("INSERT INTO samples VALUES (?, ?, ?, ?, 'num', ?, NULL)", ((p << 44) + ts, p, ts, seq, v / 100))
        c.execute("INSERT INTO outbox VALUES (?, 1, ?)", (seq, (p << 44) + ts))
        c.execute("INSERT INTO export_journal VALUES (?, ?, NULL)", (seq, payload(names, seq, p, ts, v)))
        seq += 1
        if seq - published >= 5000:  # accusé d'un lot, dans sa propre transaction, comme le banc D13
            c.execute("UPDATE meta SET next_seq = ? WHERE id = 1", (seq,))
            m.commit()
            m.tx(("UPDATE export_journal SET published_at = 1 WHERE seq >= ? AND seq < ?", (published, published + 5000)),
                 ("DELETE FROM outbox WHERE seq < ?", (published + 5000,)))
            published += 5000
            c.execute("BEGIN")
    c.execute("UPDATE meta SET next_seq = ? WHERE id = 1", (seq,))
    m.commit()
    first = 0  # purge du journal à sept jours : un jour de lignes, par préfixes de 10 000
    while first < len(day):
        m.tx(("DELETE FROM export_journal WHERE seq >= ? AND seq < ?", (first, first + 10000)))
        first += 10000
    assert m.count("samples") == len(history) + len(day)
    return m.close()


def proposed_path(work, history, day, window_ms, sort):
    """Proposé : une opération compacte en ajout par relevé (export et journal du jour), lot
    d'export figé à sa formation, rangement par point à chaque heure close ou la nuit, opérations
    purgées par préfixes après sept jours. meta.next_seq ne s'écrit pas à chaque lot (§ 14.6)."""
    m = Meter(work / f"prop-{window_ms}-{sort}.db")
    c = m.c
    for ddl in (LEAN_SAMPLES, OPS, EXPORT_BATCH, PROGRESS):
        c.execute(ddl)
    r = random.Random(1)
    c.execute("BEGIN")
    c.executemany("INSERT INTO samples VALUES (?, ?, ?, NULL, NULL)",
                  sorted(((p << 44) + ts, i, value(r)) for i, (ts, p) in enumerate(history)))
    c.executemany("INSERT INTO ops VALUES (?, 1, ?, ?, ?, NULL, NULL)",
                  ((i, p, ts, 1234) for i, (ts, p) in enumerate(history)))
    c.execute("INSERT INTO progress VALUES (1, ?)", (len(history),))
    c.execute("COMMIT")
    m.reset()
    seq = batch_first = moved = len(history)
    window = hour = None
    r = random.Random(7)

    def move(upto):
        nonlocal moved
        m.tx(("INSERT INTO samples SELECT (point << 44) + ts, seq, num, txt, NULL FROM ops"
              " WHERE seq >= ? AND seq < ? ORDER BY point, ts ON CONFLICT DO NOTHING", (moved, upto)),
             ("UPDATE progress SET upto = ? WHERE task = 1", (upto,)))
        moved = upto

    for ts, p in day:
        if ts // window_ms != window:
            if window is not None:
                m.commit()
            if sort == "heure" and hour is not None and ts // HOUR != hour:
                move(seq)
            hour, window = ts // HOUR, ts // window_ms
            c.execute("BEGIN")
        c.execute("INSERT INTO ops VALUES (?, 1, ?, ?, ?, NULL, NULL)", (seq, p, ts, value(r)))
        seq += 1
        if seq - batch_first >= 5000:  # lot figé : octets du NDJSON gzip, ≈ 16 o par opération
            m.commit()
            m.tx(("INSERT INTO export_batch VALUES (?, ?, 1)", (batch_first, bytes(5000 * 16))))
            batch_first += 5000
            c.execute("BEGIN")
    m.commit()
    move(seq)  # la nuit : tout le jour ; à l'heure close : la dernière heure
    assert m.count("samples") == len(history) + len(day)
    first = 0  # purge à sept jours : un jour d'opérations, par préfixes
    while first < len(day):
        m.tx(("DELETE FROM ops WHERE seq >= ? AND seq < ?", (first, first + 10000)))
        first += 10000
    m.tx(("DELETE FROM export_batch WHERE first < ?", (len(day),)))
    return m.close()


def point_state(work, n_points, day, flush_s):
    """maison.db : une ligne par point ; à chaque vidage, les points touchés sont réécrits."""
    m = Meter(work / f"state-{flush_s}.db")
    c = m.c
    c.execute(POINT_STATE)
    c.execute("BEGIN")
    c.executemany("INSERT INTO point_state(id, point, num, received_at, changed_at) VALUES (?, ?, 0, 0, 0)",
                  [(p, f"zigbee2mqtt:appareil_{p:04d}_piece/mesure_principale") for p in range(n_points)])
    c.execute("COMMIT")
    m.reset()
    r, dirty, window = random.Random(3), {}, None

    def flush():
        m.tx(*(("UPDATE point_state SET num = ?, received_at = ?, changed_at = ? WHERE id = ?", (v, t, t, q))
               for q, (t, v) in dirty.items()))
        dirty.clear()

    for ts, p in day:
        if ts // (flush_s * 1000) != window and dirty:
            flush()
        window = ts // (flush_s * 1000)
        dirty[p] = (ts, value(r))
    if dirty:
        flush()
    return m.close()


def state_json_bytes(n_points):
    """Taille d'un state.json de même forme que state_cache.rs : {appareil: {clé: {value, ts}}}."""
    r = random.Random(5)
    keys = ["temperature", "humidity", "battery", "linkquality", "power", "energy", "contact", "illuminance",
            "occupancy", "state", "voltage", "current"]
    rooms = ["salon", "cuisine", "chambre", "garage", "bureau", "entree"]
    kinds = ["capteur", "prise", "lampe", "contact", "thermostat"]
    states = {}
    for d in range(n_points // 4):
        device = f"zigbee2mqtt:{r.choice(rooms)}_{r.choice(kinds)}_{d:03d}"
        states[device] = {}
        for k in r.sample(keys, 4):
            v = r.choice([round(r.uniform(0, 3000), 2), r.randint(0, 255), True, "on"])
            states[device][k] = {"value": v, "ts": 1_760_000_000_000 + r.randint(0, 10**8)}
    return len(json.dumps(states, separators=(",", ":")).encode())


def reads(work, series, day):
    """Courbe des dernières 24 h du point le plus bavard : plage de samples, ou balayage du journal."""
    busiest = max(range(len(series)), key=lambda p: sum(series[p]["hours"][DAY_HOURS.start:DAY_HOURS.stop]))
    t0, t1 = H0 + DAY_HOURS.start * HOUR, H0 + DAY_HOURS.stop * HOUR

    def median_ms(c, sql, args):
        times, rows = [], 0
        for _ in range(31):
            start = time.perf_counter()
            rows = len(c.execute(sql, args).fetchall())
            times.append(time.perf_counter() - start)
        return round(statistics.median(times) * 1000, 2), rows

    out = {}
    c = sqlite3.connect(work / "point-2.db")
    out["plage_samples"] = median_ms(c, "SELECT id, num FROM samples WHERE id BETWEEN ? AND ?",
                                     ((busiest << 44) + t0, (busiest << 44) + t1))
    c.close()
    c = sqlite3.connect(work / "lecture.db")
    c.execute(OPS)
    c.executemany("INSERT INTO ops VALUES (?, 1, ?, ?, 1234, NULL, NULL)", ((i, p, ts) for i, (ts, p) in enumerate(day)))
    c.commit()
    out["journal_du_jour"] = median_ms(c, "SELECT ts, num FROM ops WHERE point = ?", (busiest,))
    c.execute("DELETE FROM ops WHERE ts < ?", (t1 - HOUR,))
    c.commit()
    out["journal_d_une_heure"] = median_ms(c, "SELECT ts, num FROM ops WHERE point = ?", (busiest,))
    c.close()
    return out


def git(*args):
    done = subprocess.run(["git", "-C", str(ROOT), *args], capture_output=True, text=True, check=False)
    return done.stdout.strip()


def environment():
    cpu = ""
    if platform.system() == "Darwin":
        cpu = subprocess.run(["sysctl", "-n", "machdep.cpu.brand_string"],
                             capture_output=True, text=True, check=False).stdout.strip()
    elif pathlib.Path("/proc/cpuinfo").exists():
        lines = pathlib.Path("/proc/cpuinfo").read_text().splitlines()
        cpu = next((line.split(":", 1)[1].strip() for line in lines if line.lower().startswith("model")), "")
    return {"python": platform.python_version(), "sqlite": sqlite3.sqlite_version,
            "systeme": f"{platform.system()} {platform.release()} {platform.machine()}", "processeur": cpu}


def fr(n, digits=0):
    """Nombre à la française, comme dans les documents du dépôt : espace des milliers, virgule."""
    return f"{n:,.{digits}f}".replace(",", " ").replace(".", ",")


def table(header, rows):
    lines = ["| " + " | ".join(header) + " |", "| " + " | ".join("---" for _ in header) + " |"]
    lines += ["| " + " | ".join(str(cell) for cell in row) + " |" for row in rows]
    return "\n".join(lines)


def measured(label, r):
    return [label, fr(r["validations"]), fr(r["wal_mo"], 1), fr(r["checkpoint_mo"], 1), fr(r["total_mo"], 1),
            fr(r["pages_par_validation"], 2)]


HEAD = ["Scénario", "Validations par jour", "WAL (Mo)", "Points de contrôle (Mo)", "Écrit par jour (Mo)",
        "Pages par validation"]
PATH_LABELS = {
    "d13-100ms": "D13 tel que décidé : relevé + outbox + journal NDJSON + `meta`, lots ≤ 100 ms",
    "d13-2s": "D13 avec des lots de 2 s",
    "prop-100ms-nuit": "Proposé : opération compacte en ajout, lots ≤ 100 ms, rangée la nuit",
    "prop-2s-heure": "Proposé : opération compacte en ajout, lots de 2 s, rangée à l'heure close",
    "prop-2s-nuit": "Proposé : lots de 2 s, rangée la nuit",
    "prop-10s-nuit": "Proposé : lots de 10 s, rangée la nuit",
}


def write_report(directory, serie, meta, workload, results):
    w, sj, lr, env = workload, results["etat"]["state_json"], results["lecture"], meta["env"]
    serie_table = table(["Champ", "Valeur"], [
        ("Série", f"`{serie}` : instant UTC de la mesure"),
        ("Dépôt", f"`{meta['commit']}` ; outil {meta['outil_etat']}"),
        ("Outil", f"`tools/storage-writes/measure.py`, SHA-256 `{meta['outil_sha256']}`"),
        ("Entrée", f"`{meta['entree']}` de l'étude privée, hors dépôt : SHA-256 `{meta['entree_sha256']}`, "
                   f"{fr(meta['entree_octets'])} octets"),
        ("Environnement", f"Python {env['python']}, SQLite {env['sqlite']}, {env['systeme']}, "
                          f"{env['processeur'] or 'processeur inconnu'}"),
        ("Durée", f"{fr(meta['duree_s'])} s"),
        ("Commande", "`python3 -I tools/storage-writes/measure.py <dashboard-workload-shape.json>`"),
    ])
    path_table = table(HEAD, [measured(PATH_LABELS[k], v) for k, v in results["chemin"].items()])
    point_table = table(HEAD, [measured(f"`samples` rangé par point, lots de {k.split('-')[1]}", v)
                               for k, v in results["rangement_par_point"].items()])
    state_table = table(HEAD, [measured(f"`point_state`, lots de {k.split('-')[1]}", v)
                               for k, v in results["etat"].items() if k.startswith("point_state")])
    read_table = table(["Lecture", "Lignes rendues", "Médiane (ms)"], [
        ("plage de `samples` rangé par point", fr(lr["plage_samples"][1]), fr(lr["plage_samples"][0], 2)),
        ("balayage du journal du jour entier", fr(lr["journal_du_jour"][1]), fr(lr["journal_du_jour"][0], 2)),
        ("balayage du journal d'une heure", fr(lr["journal_d_une_heure"][1]), fr(lr["journal_d_une_heure"][0], 2)),
    ])
    raw = json.dumps({"serie": serie, "meta": meta, "charge": workload, "resultats": results}, indent=1, ensure_ascii=False)
    text = f"""# Écritures du stockage : série {serie}

> Rapport généré par [`tools/storage-writes/measure.py`](../../tools/storage-writes/measure.py) : ne pas
> le modifier à la main, relancer l'outil. Ses chiffres fondent les § 13.7 à 13.9 de la proposition
> [Valeurs, appareils et stockage]({PROPOSAL}) et la section « Double écriture et nuit » de sa
> [page HTML](../proposition/valeurs-appareils-stockage.html#double-ecriture). Contexte :
> [inventaire de la persistance](inventaire-persistance-2026-10-20T060728Z.md),
> [annexe D13](../annexe-d13-stockage.md).

## Série

{serie_table}

## Charge rejouée

- {fr(w['series'])} séries anonymisées, comptes horaires réels sur {w['heures']} heures : {fr(w['releves_7j'])} relevés
  en sept jours, soit {fr(w['releves_par_s'], 2)} par seconde en moyenne (charge ×1 du banc D13). Les
  10 séries les plus bavardes font {fr(w['part_10'] * 100)} % des relevés, les 50 premières {fr(w['part_50'] * 100)} %.
- Jours 1 à 5 pré-remplis ({fr(w['prerempli'])} relevés), jour 6 rejoué : {fr(w['jour'])} relevés,
  {fr(w['minutes_avec_rapport'])} minutes sur 1 440 avec au moins un rapport.
- Dans une heure, les relevés d'une série sont régulièrement espacés, avec une phase tirée par série
  (graine 42). Les valeurs sont fictives : des entiers de 2 octets, comme un décimal mis à l'échelle.
- Données utiles du jour : ≈ {fr(w['jour'] * 21 / 1e6, 1)} Mo, à ≈ 21 octets par relevé amaigri (*estimation*
  du § 14.1).

## Méthode

- SQLite en WAL, pages de 4 Kio, `synchronous = OFF` : on compte des octets, pas des fsync. En
  `FULL`, comme dans D13, chaque validation ajoute un fsync.
- Point de contrôle automatique coupé ; `TRUNCATE` manuel quand le WAL dépasse 64 Mo.
- Écrit par jour = octets ajoutés au WAL + pages distinctes que chaque point de contrôle recopie dans
  la base. Le pré-remplissage n'est pas compté.
- Schémas : ceux du banc D13 (`tools/storage-bench/src/schema.rs`) et ceux du § 14 de la proposition.

## 1. Chemin d'écriture complet de l'historique

{path_table}

## 2. `samples` rangé par point, seul

Le rangement de D13 sans l'export : chaque point présent dans un lot réécrit sa propre page.

{point_table}

## 3. État courant

{state_table}

`state.json` aujourd'hui (*estimation*) : un fichier de même forme pour {fr(sj['points'])} points pèse
{fr(sj['octets'])} octets. Réécrit à chaque minute qui a reçu un rapport ({fr(sj['reecritures'])} par jour), il
écrit ≈ {fr(sj['mo_par_jour'], 1)} Mo par jour, avec deux fsync par réécriture (fichier et dossier).

## 4. Lectures

Courbe des dernières 24 h du point le plus bavard, médiane de 31 lectures sur la machine de mesure :

{read_table}

## Contrôles

- Chaque scénario de l'historique retrouve tous ses relevés après la journée : pré-remplissage +
  jour dans `samples`, sans doublon.
- Relancer l'outil réécrit un rapport identique pour les octets et les validations (graines fixes,
  même version de SQLite) ; les lectures varient d'une série à l'autre.

## Limites

- SQLite remplace Turso : même format de pages, mais Turso peut différer sur le point de contrôle.
  À confirmer avec `tools/storage-bench`, puis sur le Pi Zero 2 W de D10 et sa carte microSD.
- La charge est celle d'une maison entièrement passée de Home Assistant à Moli, sans bande morte
  ni intervalle minimal ; les horodatages sont réguliers dans l'heure.
- La machine de mesure n'est pas un Pi : comparer les lignes entre elles.

## Données brutes

```json
{raw}
```
"""
    out = directory / f"mesure-ecritures-{serie}.md"
    out.write_text(text, encoding="utf-8")
    return out


def main():
    if len(sys.argv) not in (2, 3):
        sys.exit(__doc__)
    shape_path = pathlib.Path(sys.argv[1])
    directory = pathlib.Path(sys.argv[2]) if len(sys.argv) == 3 else ROOT / "docs" / "reports"
    started = time.monotonic()
    serie = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H%M%SZ")
    raw = shape_path.read_bytes()
    series = json.loads(raw)["series"]
    assert all(len(s["hours"]) == 168 for s in series), "forme attendue : 168 heures par série"
    rng = random.Random(42)
    phase = [rng.random() for _ in series]
    history, day = events(series, phase, PREFILL_HOURS), events(series, phase, DAY_HOURS)
    totals = sorted((sum(s["hours"]) for s in series), reverse=True)
    workload = {
        "series": len(series), "heures": 168, "releves_7j": sum(totals),
        "releves_par_s": round(sum(totals) / (7 * 86_400), 2),
        "part_10": round(sum(totals[:10]) / sum(totals), 3), "part_50": round(sum(totals[:50]) / sum(totals), 3),
        "prerempli": len(history), "jour": len(day), "minutes_avec_rapport": len({ts // 60_000 for ts, _ in day}),
    }
    names = [f"zigbee2mqtt:appareil_{p:04d}_piece/mesure_principale" for p in range(len(series))]
    results = {"chemin": {}, "rangement_par_point": {}, "etat": {}}
    with tempfile.TemporaryDirectory(prefix="storage-writes-") as tmp:
        work = pathlib.Path(tmp)
        for cadence in (2, 10, 60):
            key = f"point-{cadence} s"
            results["rangement_par_point"][key] = point_layout(work, history, day, cadence)
            print(key, results["rangement_par_point"][key], file=sys.stderr)
        results["lecture"] = reads(work, series, day)
        print("lectures", results["lecture"], file=sys.stderr)
        runs = {
            "d13-100ms": lambda: d13_path(work, history, day, names, 100),
            "d13-2s": lambda: d13_path(work, history, day, names, 2000),
            "prop-100ms-nuit": lambda: proposed_path(work, history, day, 100, "nuit"),
            "prop-2s-heure": lambda: proposed_path(work, history, day, 2000, "heure"),
            "prop-2s-nuit": lambda: proposed_path(work, history, day, 2000, "nuit"),
            "prop-10s-nuit": lambda: proposed_path(work, history, day, 10_000, "nuit"),
        }
        for name, run in runs.items():
            results["chemin"][name] = run()
            print(name, results["chemin"][name], file=sys.stderr)
        for flush_s, key in ((2, "point_state-2 s"), (60, "point_state-60 s"), (900, "point_state-15 min")):
            results["etat"][key] = point_state(work, len(series), day, flush_s)
            print(key, results["etat"][key], file=sys.stderr)
    size, minutes = state_json_bytes(len(series)), workload["minutes_avec_rapport"]
    results["etat"]["state_json"] = {"points": len(series) // 4 * 4, "octets": size, "reecritures": minutes,
                                     "mo_par_jour": round(size * minutes / 1e6, 1)}
    meta = {
        "commit": git("rev-parse", "--short", "HEAD") or "inconnu",
        "outil_etat": "modifié ou non committé" if git("status", "--porcelain", "--", str(TOOL)) else "committé tel quel",
        "outil_sha256": hashlib.sha256(TOOL.read_bytes()).hexdigest(),
        "entree": shape_path.name, "entree_sha256": hashlib.sha256(raw).hexdigest(), "entree_octets": len(raw),
        "env": environment(), "duree_s": round(time.monotonic() - started),
    }
    print(write_report(directory, serie, meta, workload, results))


if __name__ == "__main__":
    main()
