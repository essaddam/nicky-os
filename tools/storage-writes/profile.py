"""Profil de l'historique : ce que la charge réelle contient, et ce que pèserait chaque forme de
conservation à long terme (relevés bruts, heures denses ou creuses, jours).

Lit les comptes horaires anonymisés de l'étude privée et, s'il est donné, son fichier de charge
(mélange des types de valeurs, octets par ligne mesurés en Parquet). Écrit un rapport daté
`docs/reports/profil-historique-<série>.md`. Ces chiffres fondent la V2 de
`docs/proposition/valeurs-appareils-stockage-v2.md` (paliers par type, segments).

Usage : python3 -I tools/storage-writes/profile.py <dashboard-workload-shape.json> [production-load.json] [dossier des rapports]
"""

import datetime
import hashlib
import json
import pathlib
import sys

sys.dont_write_bytecode = True  # aucun __pycache__ dans le dépôt
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from measure import DAY_HOURS, ROOT, environment, fr, git, table  # noqa: E402

TOOL = pathlib.Path(__file__).resolve()
CLASSES = [(0, 0, "silencieuses"), (1, 24, "1 à 24 par jour"), (25, 96, "25 à 96"), (97, 288, "97 à 288"),
           (289, 1440, "289 à 1 440"), (1441, None, "plus de 1 440 (plus d'une par minute)")]


def profile(series):
    day = [s["hours"][DAY_HOURS.start:DAY_HOURS.stop] for s in series]
    per_series = [sum(h) for h in day]
    total = sum(per_series)
    classes = []
    for lo, hi, label in CLASSES:
        members = [n for n in per_series if n >= lo and (hi is None or n <= hi)]
        classes.append({"classe": label, "series": len(members), "releves": sum(members),
                        "part": round(sum(members) / total, 3) if total else 0})
    return {
        "series": len(series), "releves_jour": total,
        "heures_denses": len(series) * 24,
        "heures_creuses": sum(1 for h in day for n in h if n),
        "jours_denses": len(series),
        "jours_creux": sum(1 for n in per_series if n),
        "classes": classes,
    }


def study(path):
    """Agrégats de l'étude : mélange des types de valeurs, octets par ligne en Parquet."""
    data = json.loads(path.read_bytes())
    mix, raw, stats = {}, [], []
    for source in data["sources"]:
        for kind, n in source["payload_classes_for_value_changes"].items():
            mix[kind] = mix.get(kind, 0) + n
        raw.append(source["raw_parquet_real_sample"]["bytes_per_row"])
        stats.append(source["statistics_parquet_real_sample"]["bytes_per_row"])
    total = sum(mix.values())
    return {"melange": {k: round(v / total, 3) for k, v in sorted(mix.items(), key=lambda kv: -kv[1])},
            "changements_7j": total, "octets_brut": [min(raw), max(raw)], "octets_stats": [min(stats), max(stats)]}


def mo(rows, octets):
    return f"{fr(rows * octets[0] / 1e6, 1)} à {fr(rows * octets[1] / 1e6, 1)}"


def write_report(directory, serie, meta, prof, st):
    classes = table(["Séries", "Nombre", "Relevés du jour", "Part des relevés"],
                    [(c["classe"], fr(c["series"]), fr(c["releves"]), f"{fr(c['part'] * 100, 1)} %") for c in prof["classes"]])
    forms = [("Relevés bruts", prof["releves_jour"], "brut"),
             ("Heures denses : chaque point, chaque heure", prof["heures_denses"], "stats"),
             ("Heures creuses : seulement les heures où le point a changé", prof["heures_creuses"], "stats"),
             ("Jours denses : chaque point, chaque jour", prof["jours_denses"], "stats"),
             ("Jours creux : seulement les jours où le point a changé", prof["jours_creux"], "stats")]
    if st:
        rows = [(label, fr(n), fr(n * 365), mo(n * 365, st["octets_" + kind])) for label, n, kind in forms]
        sizes = table(["Forme", "Lignes par jour", "Lignes par an", "Mo par an en Parquet (mesuré)"], rows)
        mix = table(["Type de valeur (étude)", "Part des changements"],
                    [(k, f"{fr(v * 100, 1)} %") for k, v in st["melange"].items()])
        study_text = f"""## Types de valeurs et octets par ligne (étude)

{mix}

Sur {fr(st['changements_7j'])} changements en sept jours. `text_32` : textes courts, surtout des
énumérations (« on », « heat », « playing ») ; `text_256` : textes longs (titres, messages).
En Parquet, sur des valeurs réelles : {fr(st['octets_brut'][0], 1)} à {fr(st['octets_brut'][1], 1)} octets par
relevé brut, {fr(st['octets_stats'][0], 1)} à {fr(st['octets_stats'][1], 1)} octets par ligne de statistiques
(moyenne, minimum, maximum).
"""
    else:
        rows = [(label, fr(n), fr(n * 365), "non calculé") for label, n, _ in forms]
        sizes = table(["Forme", "Lignes par jour", "Lignes par an", "Mo par an en Parquet"], rows)
        study_text = "## Types de valeurs et octets par ligne\n\nFichier de charge de l'étude non fourni.\n"
    text = f"""# Profil de l'historique : série {serie}

> Rapport généré par [`tools/storage-writes/profile.py`](../../tools/storage-writes/profile.py) : ne
> pas le modifier à la main, relancer l'outil. Ses chiffres fondent la
> [V2 de la proposition](../proposition/valeurs-appareils-stockage-v2.md) (paliers par type,
> segments). Mesure voisine des écritures :
> [mesure-ecritures-2026-10-21T114647Z](mesure-ecritures-2026-10-21T114647Z.md).

## Série

{table(["Champ", "Valeur"], [
        ("Série", f"`{serie}` : instant UTC du calcul"),
        ("Dépôt", f"`{meta['commit']}` ; outil {meta['outil_etat']}"),
        ("Outil", f"`tools/storage-writes/profile.py`, SHA-256 `{meta['outil_sha256']}`"),
        ("Entrées", "; ".join(f"`{name}`, SHA-256 `{digest}`" for name, digest in meta["entrees"]) + " (étude privée, hors dépôt)"),
        ("Environnement", f"Python {meta['env']['python']}, {meta['env']['systeme']}"),
        ("Commande", "`python3 -I tools/storage-writes/profile.py <dashboard-workload-shape.json> <production-load.json>`"),
    ])}

## Qui écrit l'historique

Journée rejouée par la mesure des écritures (jour 6) : {fr(prof['releves_jour'])} relevés pour
{fr(prof['series'])} séries. Les comptes comprennent les mises à jour d'attributs seuls : ils majorent
les vrais changements de valeur.

{classes}

{study_text}
## Ce que pèse chaque forme à long terme

Lignes par an pour la charge ×1 ; le poids applique les octets par ligne mesurés en Parquet sur des
valeurs réelles (bas et haut de l'étude). Une heure creuse ou un jour creux non écrit veut dire
« la valeur n'a pas changé » : la disponibilité de l'appareil dit si la source vivait.

{sizes}

## Limites

- Les comptes viennent de Home Assistant, sans bande morte : une forme creuse sur Moli serait plus
  petite encore.
- Les octets par ligne ont été mesurés par l'étude, en colonnes typées et zstd. Le format actuel
  de l'archive (`archive_v1`) en est loin : nom du point et JSON complet de l'opération sur chaque
  ligne, sans dictionnaire, en Snappy, par groupes de 256 lignes. Son coût réel reste à mesurer.
- Les segments à tolérance demandent les valeurs elles-mêmes, que ce profil n'a pas : leur gain se
  mesure sur un export anonymisé avec valeurs.

## Données brutes

```json
{json.dumps({"serie": serie, "meta": meta, "profil": prof, "etude": st}, indent=1, ensure_ascii=False)}
```
"""
    out = directory / f"profil-historique-{serie}.md"
    out.write_text(text, encoding="utf-8")
    return out


def main():
    args = [pathlib.Path(a) for a in sys.argv[1:]]
    if not 1 <= len(args) <= 3:
        sys.exit(__doc__)
    shape_path = args[0]
    load_path = args[1] if len(args) > 1 else None
    directory = args[2] if len(args) > 2 else ROOT / "docs" / "reports"
    serie = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H%M%SZ")
    shape_raw = shape_path.read_bytes()
    series = json.loads(shape_raw)["series"]
    inputs = [(shape_path.name, hashlib.sha256(shape_raw).hexdigest())]
    st = None
    if load_path:
        inputs.append((load_path.name, hashlib.sha256(load_path.read_bytes()).hexdigest()))
        st = study(load_path)
    meta = {
        "commit": git("rev-parse", "--short", "HEAD") or "inconnu",
        "outil_etat": "modifié ou non committé" if git("status", "--porcelain", "--", str(TOOL)) else "committé tel quel",
        "outil_sha256": hashlib.sha256(TOOL.read_bytes()).hexdigest(),
        "entrees": inputs, "env": environment(),
    }
    print(write_report(directory, serie, meta, profile(series), st))


if __name__ == "__main__":
    main()
