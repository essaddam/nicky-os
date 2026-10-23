"""Ce que pèse chaque forme de l'historique en Parquet : format actuel de l'archive contre
colonnes typées, relevés bruts contre relevés à bande, heures et jours denses contre creux.

Charge : les comptes horaires réels anonymisés de l'étude privée (sept jours) ; les VALEURS sont
synthétiques (natures tirées au sort, bruit et dérives plausibles) : les octets des formes
« bande » dépendent de ce modèle, les autres surtout de la forme. Écrit un rapport daté
`docs/reports/formes-parquet-<série>.md`. Ces chiffres fondent la V3 de
`docs/proposition/valeurs-appareils-stockage-v3.md`.

Usage : python3 -I tools/storage-writes/parquet_forms.py <dashboard-workload-shape.json> [dossier des rapports]
Dépendance : pyarrow (seule exception à « bibliothèque standard seule » dans ce dossier).
"""

import datetime
import hashlib
import json
import os
import pathlib
import random
import sys
import tempfile

sys.dont_write_bytecode = True
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import pyarrow as pa  # noqa: E402
import pyarrow.parquet as pq  # noqa: E402
from measure import ROOT, environment, fr, git, table  # noqa: E402

TOOL = pathlib.Path(__file__).resolve()
HOUR = 3_600_000
H0 = 1_760_000_000_000 - 1_760_000_000_000 % HOUR
HOURS = 168
ENUM_KEYS = ["off", "on", "heat", "auto", "cool", "idle", "playing", "paused", "standby", "eco"]
POINT_NAME = "zigbee2mqtt:appareil_{:04d}_piece/mesure_principale"

# Réglages de l'archive aujourd'hui (crates/moli-archive/src/parquet.rs) et proposés.
V1 = {"compression": "snappy", "use_dictionary": False, "row_group_size": 256, "data_page_size": 64 * 1024}
V3 = {"compression": "zstd", "use_dictionary": True, "row_group_size": 65536}


def natures(n, rng):
    """86,6 % de mesures, 9,1 % d'énumérations, 3,5 % de booléens (série profil) ; parmi les
    mesures, les séries bavardes sont des puissances bruitées, les autres des mesures lentes."""
    out = []
    for _ in range(n):
        r = rng.random()
        out.append("measure" if r < 0.866 else "enum" if r < 0.957 else "bool")
    return out


def generate(series, nat, rng):
    """Relevés bruts des sept jours : (point, ts, valeur entière ou code), par série, triés par série."""
    raws, deadband = {}, {}
    for p, s in enumerate(series):
        total = sum(s["hours"])
        rows = []
        if nat[p] == "measure":
            chatty = total > 7 * 288
            if chatty:  # puissance : watts, bruit ±3 W, sauts de charge
                v, sd, db = rng.randint(50, 800), 3.0, 5
            else:  # température ×100 : bruit ±0,02 °C, dérive lente
                v, sd, db = rng.randint(1500, 2400), 2.0, 5
            deadband[p] = db
            drift = 0.0
            for h in range(HOURS):
                n = s["hours"][h]
                for k in range(n):
                    ts = H0 + h * HOUR + int((k + 0.5) * HOUR / n)
                    if chatty and rng.random() < 0.01:
                        v = rng.randint(30, 2500)  # un appareil s'allume ou s'éteint
                    elif not chatty and rng.random() < 0.05:
                        drift = rng.uniform(-0.6, 0.6)
                    v = max(0, v + drift + rng.gauss(0, sd))
                    rows.append((ts, int(round(v))))
        else:
            codes = len(ENUM_KEYS) if nat[p] == "enum" else 2
            code = rng.randrange(codes)
            for h in range(HOURS):
                n = s["hours"][h]
                for k in range(n):
                    ts = H0 + h * HOUR + int((k + 0.5) * HOUR / n)
                    if rng.random() < 0.7:  # la plupart des rapports d'un état sont des transitions
                        code = (code + rng.randrange(1, codes)) % codes if codes > 1 else code
                    rows.append((ts, code))
        raws[p] = rows
    return raws, deadband


def band(rows, db):
    """Relevé à bande : écrit quand |v − écrite| ≥ bande ; la ligne écrite résume l'intervalle qu'elle
    ferme (moyenne pondérée par le temps, min, max, rapports absorbés)."""
    out = []
    if not rows:
        return out
    t0, v0 = rows[0]
    written, held, lo, hi, area, n, last_t = v0, v0, v0, v0, 0.0, 1, t0
    start = t0
    out.append((t0, v0, None, None, None, None))
    for ts, v in rows[1:]:
        area += held * (ts - last_t)
        if abs(v - written) >= db:
            mean = area / (ts - start) if ts > start else held
            out.append((ts, v, int(round(mean)), lo, hi, n) if n > 1 else (ts, v, None, None, None, None))
            written, held, lo, hi, area, n, start = v, v, v, v, 0.0, 1, ts
        else:
            held, lo, hi, n = v, min(lo, v), max(hi, v), n + 1
        last_t = ts
    return out


def hours_of(rows, kind):
    """Stat par heure (dense) : mesure → moyenne pondérée, min, max ; état → durée par code.
    Rend (dense, creux) où creux ne garde que les heures avec un changement (ou la première)."""
    dense, sparse = [], []
    if not rows:
        return dense, sparse
    i, held, held_from = 0, None, None
    for h in range(HOURS):
        hs, he = H0 + h * HOUR, H0 + (h + 1) * HOUR
        changes, lo, hi, area, covered, durations = 0, None, None, 0.0, 0, {}
        cursor = hs
        while i < len(rows) and rows[i][0] < he:
            ts, v = rows[i]
            if held is not None and ts > cursor:
                seg = ts - cursor
                area += held * seg
                covered += seg
                durations[held] = durations.get(held, 0) + seg
                lo = held if lo is None else min(lo, held)
                hi = held if hi is None else max(hi, held)
            if v != held:
                changes += 1
            held, cursor = v, ts
            i += 1
        if held is not None and he > cursor:
            seg = he - cursor
            area += held * seg
            covered += seg
            durations[held] = durations.get(held, 0) + seg
            lo = held if lo is None else min(lo, held)
            hi = held if hi is None else max(hi, held)
        if held is None:
            continue
        if kind == "measure":
            row = (h, int(round(area / covered)) if covered else held, lo, hi, covered, changes)
            dense.append(row)
            if changes or not sparse:
                sparse.append(row)
        else:
            for code, ms in sorted(durations.items()):
                dense.append((h, code, ms, changes))
            if changes or not sparse:
                for code, ms in sorted(durations.items()):
                    sparse.append((h, code, ms, changes))
    return dense, sparse


def days_of(hours, kind):
    """Jours depuis les heures (UTC, pour la mesure) : fusion pondérée par la couverture."""
    dense, sparse = [], []
    if kind == "measure":
        by_day = {}
        for h, mean, lo, hi, covered, changes in hours:
            d = by_day.setdefault(h // 24, [0.0, None, None, 0, 0])
            d[0] += mean * covered
            d[1] = lo if d[1] is None else min(d[1], lo)
            d[2] = hi if d[2] is None else max(d[2], hi)
            d[3] += covered
            d[4] += changes
        for day, (area, lo, hi, covered, changes) in sorted(by_day.items()):
            row = (day, int(round(area / covered)) if covered else 0, lo, hi, covered, changes)
            dense.append(row)
            if changes or not sparse:
                sparse.append(row)
    else:
        by_day = {}
        for h, code, ms, changes in hours:
            d = by_day.setdefault(h // 24, {})
            d[code] = d.get(code, 0) + ms
            d["_c"] = d.get("_c", 0) + changes
        for day, codes in sorted(by_day.items()):
            changes = codes.pop("_c", 0)
            for code, ms in sorted(codes.items()):
                dense.append((day, code, ms, changes))
                if changes or not sparse:
                    sparse.append((day, code, ms, changes))
    return dense, sparse


def write(path, tab, settings):
    pq.write_table(tab, path, **settings)
    return os.path.getsize(path)


def payload_sample(p, ts, kind, v, seq):
    return json.dumps({"type": "sample", "seq": seq, "point": POINT_NAME.format(p), "ts": ts, "kind": kind,
                       "num": v if kind != "t" else None, "txt": ENUM_KEYS[v] if kind == "t" else None},
                      separators=(",", ":"))


def payload_stat(p, h, mean, lo, hi, covered, changes):
    stat = {"start": H0 + h * HOUR, "end": H0 + (h + 1) * HOUR,
            "aggregate": {"TimeWeighted": {"mean": mean / 100, "min": lo / 100, "max": hi / 100}},
            "covered_ms": covered, "changes": changes, "last": {"value": mean / 100, "ts": H0 + (h + 1) * HOUR - 1},
            "quality": "Complete" if covered == HOUR else "Partial", "origin": "Recorded"}
    return json.dumps({"point": POINT_NAME.format(p), "source": "Samples", "catalog_revision": 3, "stat": stat,
                       "known_cost_sum": 0.0, "cost_hours_known": 0, "hours_observed": 0, "coverage": "measured"},
                      separators=(",", ":"))


def main():
    if len(sys.argv) not in (2, 3):
        sys.exit(__doc__)
    shape_path = pathlib.Path(sys.argv[1])
    directory = pathlib.Path(sys.argv[2]) if len(sys.argv) == 3 else ROOT / "docs" / "reports"
    serie = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H%M%SZ")
    raw_bytes = shape_path.read_bytes()
    series = json.loads(raw_bytes)["series"]
    rng = random.Random(11)
    nat = natures(len(series), rng)
    raws, deadband = generate(series, nat, rng)
    measures = [p for p in range(len(series)) if nat[p] == "measure"]
    states = [p for p in range(len(series)) if nat[p] != "measure"]
    n_raw = sum(len(r) for r in raws.values())

    # Formes
    bands = {p: band(raws[p], deadband[p]) for p in measures}
    hours_m = {p: hours_of(raws[p], "measure") for p in measures}
    hours_s = {p: hours_of(raws[p], "state") for p in states}
    days_m = {p: days_of(hours_m[p][0], "measure") for p in measures}
    days_s = {p: days_of(hours_s[p][0], "state") for p in states}

    results = {}
    with tempfile.TemporaryDirectory(prefix="parquet-forms-") as tmp:
        tmp = pathlib.Path(tmp)
        # 1. archive_v1 : brut, format et réglages actuels, puis réglages seuls changés
        ent, tss, seqs, kinds, payloads, nums, costs, cov = [], [], [], [], [], [], [], []
        seq = 0
        for p in range(len(series)):
            kind = "f" if nat[p] == "measure" else "t" if nat[p] == "enum" else "b"
            for ts, v in raws[p]:
                ent.append(POINT_NAME.format(p)); tss.append(ts); seqs.append(seq); kinds.append("s")
                payloads.append(payload_sample(p, ts, kind, v, seq))
                nums.append(None if kind == "t" else float(v)); costs.append(None); cov.append(0)
                seq += 1
        v1 = pa.table({"entity": ent, "ts": pa.array(tss, pa.int64()), "seq": pa.array(seqs, pa.int64()), "kind": kinds,
                       "payload": payloads, "num": pa.array(nums, pa.float64()), "cost": pa.array(costs, pa.float64()),
                       "covered_ms": pa.array(cov, pa.int64())})
        results["archive_v1 brut, réglages actuels"] = (n_raw, write(tmp / "v1.parquet", v1, V1))
        results["archive_v1 brut, zstd + dictionnaire + grands groupes"] = (n_raw, write(tmp / "v1z.parquet", v1, V3))
        del v1, ent, payloads
        # 2. typé brut
        pts, tss, vals = [], [], []
        for p in measures:
            for ts, v in raws[p]:
                pts.append(p); tss.append(ts); vals.append(v)
        tm = pa.table({"point": pa.array(pts, pa.int32()), "ts": pa.array(tss, pa.int64()), "val": pa.array(vals, pa.int32())})
        pts, tss, vals = [], [], []
        for p in states:
            for ts, v in raws[p]:
                pts.append(p); tss.append(ts); vals.append(v)
        te = pa.table({"point": pa.array(pts, pa.int32()), "ts": pa.array(tss, pa.int64()), "code": pa.array(vals, pa.int16())})
        results["typé brut : mesures + états"] = (n_raw, write(tmp / "tm.parquet", tm, V3) + write(tmp / "te.parquet", te, V3))
        # 3. typé à bande (mesures) + états bruts
        pts, tss, vals, los, his, means, ns = [], [], [], [], [], [], []
        for p in measures:
            for ts, v, mean, lo, hi, n in bands[p]:
                pts.append(p); tss.append(ts); vals.append(v); means.append(mean); los.append(lo); his.append(hi); ns.append(n)
        tb = pa.table({"point": pa.array(pts, pa.int32()), "ts": pa.array(tss, pa.int64()), "val": pa.array(vals, pa.int32()),
                       "mean": pa.array(means, pa.int32()), "lo": pa.array(los, pa.int32()), "hi": pa.array(his, pa.int32()),
                       "n": pa.array(ns, pa.int32())})
        n_band = len(pts) + te.num_rows
        results["typé à bande : mesures à bande + états"] = (n_band, write(tmp / "tb.parquet", tb, V3) + os.path.getsize(tmp / "te.parquet"))
        # 4. heures denses et creuses
        for label, idx in (("heures denses", 0), ("heures creuses", 1)):
            pts, hs, means, los, his, covs, chs = [], [], [], [], [], [], []
            for p in measures:
                for h, mean, lo, hi, covered, changes in hours_m[p][idx]:
                    pts.append(p); hs.append(h); means.append(mean); los.append(lo); his.append(hi); covs.append(covered); chs.append(changes)
            th = pa.table({"point": pa.array(pts, pa.int32()), "hour": pa.array(hs, pa.int32()), "mean": pa.array(means, pa.int32()),
                           "lo": pa.array(los, pa.int32()), "hi": pa.array(his, pa.int32()), "covered_ms": pa.array(covs, pa.int32()),
                           "changes": pa.array(chs, pa.int32())})
            pts, hs, codes, mss, chs = [], [], [], [], []
            for p in states:
                for h, code, ms, changes in hours_s[p][idx]:
                    pts.append(p); hs.append(h); codes.append(code); mss.append(ms); chs.append(changes)
            ts_ = pa.table({"point": pa.array(pts, pa.int32()), "hour": pa.array(hs, pa.int32()), "code": pa.array(codes, pa.int16()),
                            "ms": pa.array(mss, pa.int32()), "changes": pa.array(chs, pa.int32())})
            results[f"typé {label} : mesures + durées par état"] = (th.num_rows + ts_.num_rows, write(tmp / f"h{idx}.parquet", th, V3) + write(tmp / f"s{idx}.parquet", ts_, V3))
            if idx == 0:  # archive_v1 : les mêmes heures denses en JSON, réglages actuels
                ent, tss, seqs, kinds, payloads, nums, costs, cov = [], [], [], [], [], [], [], []
                for p in measures:
                    for h, mean, lo, hi, covered, changes in hours_m[p][0]:
                        ent.append("samples:" + POINT_NAME.format(p)); tss.append(H0 + h * HOUR); seqs.append(3); kinds.append("hour")
                        payloads.append(payload_stat(p, h, mean, lo, hi, covered, changes)); nums.append(mean / 100); costs.append(None); cov.append(covered)
                vs = pa.table({"entity": ent, "ts": pa.array(tss, pa.int64()), "seq": pa.array(seqs, pa.int64()), "kind": kinds,
                               "payload": payloads, "num": pa.array(nums, pa.float64()), "cost": pa.array(costs, pa.float64()),
                               "covered_ms": pa.array(cov, pa.int64())})
                results["archive_v1 heures denses (mesures seulement), réglages actuels"] = (vs.num_rows, write(tmp / "vs.parquet", vs, V1))
                del vs, payloads
        # 5. jours denses et creux
        for label, idx in (("jours denses", 0), ("jours creux", 1)):
            pts, ds, means, los, his, covs, chs = [], [], [], [], [], [], []
            for p in measures:
                for d, mean, lo, hi, covered, changes in days_m[p][idx]:
                    pts.append(p); ds.append(d); means.append(mean); los.append(lo); his.append(hi); covs.append(covered); chs.append(changes)
            td = pa.table({"point": pa.array(pts, pa.int32()), "day": pa.array(ds, pa.int32()), "mean": pa.array(means, pa.int32()),
                           "lo": pa.array(los, pa.int32()), "hi": pa.array(his, pa.int32()), "covered_ms": pa.array(covs, pa.int32()),
                           "changes": pa.array(chs, pa.int32())})
            pts, ds, codes, mss, chs = [], [], [], [], []
            for p in states:
                for d, code, ms, changes in days_s[p][idx]:
                    pts.append(p); ds.append(d); codes.append(code); mss.append(ms); chs.append(changes)
            tsd = pa.table({"point": pa.array(pts, pa.int32()), "day": pa.array(ds, pa.int32()), "code": pa.array(codes, pa.int16()),
                            "ms": pa.array(mss, pa.int32()), "changes": pa.array(chs, pa.int32())})
            results[f"typé {label} : mesures + durées par état"] = (td.num_rows + tsd.num_rows, write(tmp / f"d{idx}.parquet", td, V3) + write(tmp / f"sd{idx}.parquet", tsd, V3))

    band_rows = sum(len(b) for b in bands.values())
    measure_rows = sum(len(raws[p]) for p in measures)
    workload = {"series": len(series), "mesures": len(measures), "etats": len(states), "releves_7j": n_raw,
                "releves_mesures_7j": measure_rows, "lignes_bande_7j": band_rows,
                "reduction_bande": round(measure_rows / band_rows, 1) if band_rows else None}
    meta = {"commit": git("rev-parse", "--short", "HEAD") or "inconnu",
            "outil_etat": "modifié ou non committé" if git("status", "--porcelain", "--", str(TOOL)) else "committé tel quel",
            "outil_sha256": hashlib.sha256(TOOL.read_bytes()).hexdigest(),
            "entree": shape_path.name, "entree_sha256": hashlib.sha256(raw_bytes).hexdigest(),
            "env": environment(), "pyarrow": pa.__version__}
    rows = [(label, fr(n), fr(size / 1e6, 2), fr(size / n, 1) if n else "—", fr(size * 52 / 1e6)) for label, (n, size) in results.items()]
    out = directory / f"formes-parquet-{serie}.md"
    out.write_text(f"""# Formes de l'historique en Parquet : série {serie}

> Rapport généré par [`tools/storage-writes/parquet_forms.py`](../../tools/storage-writes/parquet_forms.py) :
> ne pas le modifier à la main, relancer l'outil. Ses chiffres fondent la
> [V3 de la proposition](../proposition/valeurs-appareils-stockage-v3.md). Rapports voisins :
> [profil de l'historique](profil-historique-2026-10-21T151507Z.md),
> [mesure des écritures](mesure-ecritures-2026-10-21T114647Z.md).

## Série

{table(["Champ", "Valeur"], [
        ("Série", f"`{serie}` : instant UTC du calcul"),
        ("Dépôt", f"`{meta['commit']}` ; outil {meta['outil_etat']}"),
        ("Outil", f"`tools/storage-writes/parquet_forms.py`, SHA-256 `{meta['outil_sha256']}`"),
        ("Entrée", f"`{meta['entree']}` de l'étude privée, hors dépôt : SHA-256 `{meta['entree_sha256']}`"),
        ("Environnement", f"Python {meta['env']['python']}, pyarrow {meta['pyarrow']}, {meta['env']['systeme']}, {meta['env']['processeur'] or 'processeur inconnu'}"),
        ("Commande", "`python3 -I tools/storage-writes/parquet_forms.py <dashboard-workload-shape.json>`"),
    ])}

## Charge et valeurs

- Comptes réels : {fr(workload['series'])} séries, {fr(workload['releves_7j'])} relevés en sept jours
  ({fr(workload['mesures'])} mesures, {fr(workload['etats'])} états ou booléens, natures tirées au sort selon
  le mélange de l'étude).
- **Valeurs synthétiques** (graine fixe) : puissances bruitées (±3 W, sauts d'allumage) pour les séries
  bavardes, mesures lentes (±0,02 °C, dérives) pour les autres, états à transitions fréquentes.
  Les octets des formes à bande dépendent de ce modèle ; ceux des autres formes surtout de la forme.
- Bande : seuil de 5 unités (5 W, 0,05 °C). Elle garde {fr(workload['lignes_bande_7j'])} lignes de mesure sur
  {fr(workload['releves_mesures_7j'])}, soit {fr(workload['reduction_bande'], 1)} fois moins.

## Octets par forme

Sept jours ; « Mo par an » = taille × 52. Réglages actuels de l'archive : Snappy, dictionnaire coupé,
groupes de 256 lignes. Réglages proposés : zstd, dictionnaire, groupes de 65 536 lignes.

{table(["Forme", "Lignes (7 j)", "Mo (7 j)", "Octets par ligne", "Mo par an"], rows)}

## Lecture

- Le format actuel de l'archive coûte surtout par sa forme, pas par ses réglages : la colonne
  `payload` répète l'opération en JSON, `entity` répète le nom du point.
- Une grille dense se compresse bien quand les séries ne changent pas (dictionnaire et répétitions) :
  comparer « denses » et « creuses » en octets, pas seulement en lignes.
- La bande réduit les lignes ; son gain en octets dépend du bruit réel des capteurs, que seules des
  valeurs réelles donneront.

## Données brutes

```json
{json.dumps({"serie": serie, "meta": meta, "charge": workload, "resultats": {k: {"lignes": n, "octets": s} for k, (n, s) in results.items()}}, indent=1, ensure_ascii=False)}
```
""", encoding="utf-8")
    print(out)


if __name__ == "__main__":
    main()
