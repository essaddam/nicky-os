#!/usr/bin/env python3
"""Publish aggregate heap findings from private Apple-tool output."""
import argparse
import datetime
import hashlib
import html
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[2]
PUBLIC = ROOT / "docs/proposition/analyse/v6"


def apple_bytes(value):
    match = re.fullmatch(r"([0-9.]+)([KMG]?)", value)
    if not match:
        raise ValueError("Format mémoire Apple inconnu")
    return round(float(match[1]) * {"": 1, "K": 1024, "M": 1048576, "G": 1073741824}[match[2]])


def snapshot(folder, label):
    heap_path = folder / f"{label}-heap.stdout"
    text = heap_path.read_text()
    match = re.search(r"All zones: (\d+) nodes \((\d+) bytes\)", text)
    if not match:
        raise ValueError("Totaux heap absents")
    categories = {
        "json_nodes": {"label": "Nœuds BTreeMap de serde_json::Value", "bytes": 0, "blocks": 0, "lines": []},
        "string_clone_symbol": {"label": "Allocations attribuées au symbole String::clone", "bytes": 0, "blocks": 0, "lines": []},
        "house_channels_tree": {"label": "Nœuds BTreeMap des canaux House", "bytes": 0, "blocks": 0, "lines": []},
        "sample_trees": {"label": "Nœuds BTreeMap de Sample", "bytes": 0, "blocks": 0, "lines": []},
    }
    for line_no, line in enumerate(text.splitlines(), 1):
        row = re.match(r"^\s+(\d+)\s+(\d+)\s+[\d.]+\s+(.+?)\s+(?:C|ObjC|Swift)\s+\S+\s*$", line)
        if not row:
            continue
        name = row[3]
        node = "LeafNode" in name or "InternalNode" in name
        key = None
        if node and "serde_json" in name:
            key = "json_nodes"
        elif "6string" in name and "6String" in name and "5Clone5clone" in name and not node:
            key = "string_clone_symbol"
        elif node and "house7Channel" in name:
            key = "house_channels_tree"
        elif node and "6Sample" in name:
            key = "sample_trees"
        if key:
            c = categories[key]
            c["blocks"] += int(row[1])
            c["bytes"] += int(row[2])
            c["lines"].append(line_no)
    total = int(match[2])
    used = sum(c["bytes"] for c in categories.values())
    if used > total or not categories["json_nodes"]["bytes"]:
        raise ValueError("Catégories heap incohérentes")
    categories["other"] = {"label": "Autres allocations", "bytes": total-used,
                           "blocks": int(match[1])-sum(c["blocks"] for c in categories.values()), "lines": []}
    vm_text = (folder / f"{label}-vmmap.stdout").read_text()
    footprint = re.search(r"Physical footprint:\s+([0-9.]+[KMG]?)", vm_text)
    peak = re.search(r"Physical footprint \(peak\):\s+([0-9.]+[KMG]?)", vm_text)
    history_path = folder / f"{label}-malloc_history.stdout"
    history = history_path.read_text()
    paths = {}
    for key, marker, caption in [
        ("broker_read_buffers", "broker11read_packet", "Tampons de lecture du broker"),
        ("mqtt_write_buffers", "Network5write", "Tampons d’écriture MQTT des ponts"),
        ("broker_payload_copies", "7Brokers5start00", "Copies de payloads dans le hook du broker"),
    ]:
        found = []
        for line_no, line in enumerate(history.splitlines(), 1):
            m = re.search(r"(\d+) \(([0-9.]+[KMG]?)\)", line)
            if marker in line and m:
                found.append({"blocks": int(m[1]), "bytes_rounded": apple_bytes(m[2]), "line": line_no})
        if found:
            paths[key] = dict(max(found, key=lambda r: r["bytes_rounded"]), label=caption,
                              source=history_path.name,
                              scope="plus grand groupe observé de cette pile ; arrondi ; vue distincte des classes heap")
    return {"label": label, "heap_blocks": int(match[1]), "heap_bytes_rounded_allocations": total,
            "allocation_classes": categories, "classes_source": heap_path.name,
            "physical_footprint_bytes_rounded": apple_bytes(footprint[1]) if footprint else None,
            "physical_footprint_peak_bytes_rounded": apple_bytes(peak[1]) if peak else None,
            "selected_stack_groups": paths}


def fr(value, digits=2):
    return f"{value:,.{digits}f}".replace(",", "\u202f").replace(".", ",")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("private_directory", type=Path)
    args = parser.parse_args()
    folder = args.private_directory.resolve()
    receipt = json.loads((PUBLIC / "heap-snapshot-recu.json").read_text())
    snapshots = [snapshot(folder, label) for label in ("start", "end")]
    reports = [json.loads(line[12:]) for line in (folder/"supervisor.stdout").read_text().splitlines()
               if line.startswith("LOAD_REPORT ")]
    report = reports[-1]
    leaks = []
    for name in ("end-leak-scan.stdout", "end-leak-conservative.stdout"):
        p = folder/name
        if p.exists():
            m = re.search(r"Process \d+: (\d+) leaks for (\d+) total leaked bytes", p.read_text())
            if m:
                leaks.append({"source": name, "blocks_flagged": int(m[1]), "bytes_flagged": int(m[2]),
                              "interpretation": "signalement de l’outil ; fuite Rust non confirmée"})
    leak_roots = {}
    scan_path = folder / "end-leak-scan.stdout"
    if scan_path.exists() and leaks:
        sections = scan_path.read_text().split("STACK OF ")
        if len(sections) > 1 and "Network5write" in sections[1]:
            sizes = [int(v) for v in re.findall(r"ROOT LEAK:.*?\[(\d+)\]", sections[1])]
            if sizes:
                leak_roots["network_write_buffers"] = {"blocks":len(sizes),"bytes":sum(sizes)}
                leak_roots["other_roots"] = {"blocks":leaks[0]["blocks_flagged"]-len(sizes), "bytes":leaks[0]["bytes_flagged"]-sum(sizes), "remember_all_seen": any("remember_all" in part for part in sections[2:])}
    private_files = [p for p in folder.iterdir() if p.is_file() and p.suffix in (".memgraph", ".stdout", ".stderr")]
    evidence = {
        "schema": 1, "recorded_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "budget": receipt["guard"], "binary_sha256": receipt["binary_sha256"],
        "snapshots": snapshots, "leak_diagnostics": leaks,
        "flagged_root_groups": leak_roots,
        "profile_context": {
            "bootstrap_stable": report["worker"]["bootstrap_stable"],
            "workflow_parity": report["workflow_parity"],
            "hub_points_final": report["worker"]["measured_end"]["hub_points"],
            "retained_topics_final": report["worker"]["measured_end"]["broker"]["retained_topics"],
            "broker_errors": report["worker"]["measured_end"]["broker"]["errors"],
            "physical_commands": 0,
            "qualification": "profilage perturbant l’initialisation ; snapshots non comparables comme deux états stabilisés ; seconde capture au modèle complet",
        },
        "private_artifacts": {p.name: {"bytes": p.stat().st_size, "sha256": hashlib.sha256(p.read_bytes()).hexdigest()}
                              for p in sorted(private_files)},
        "privacy": "captures et sorties brutes hors dépôt ; aucun PID, adresse ou chemin personnel dans ce résumé",
    }
    (PUBLIC/"heap-snapshot-analyse.json").write_text(json.dumps(evidence, ensure_ascii=False, indent=2)+"\n")
    a, b = snapshots
    to_mib = lambda value: value/1048576
    rows = "".join(f"<tr><td>{html.escape(c['label'])}</td><td>{fr(a['allocation_classes'][k]['bytes']/1048576)}</td>"
                   f"<td>{fr(c['bytes']/1048576)}</td><td>{fr(c['blocks'],0)}</td></tr>"
                   for k,c in b["allocation_classes"].items())
    paths = "".join(f"<tr><td>{html.escape(p['label'])}</td><td>≈ {fr(to_mib(p['bytes_rounded']),1)} Mio</td>"
                    f"<td>{p['blocks']}</td><td>{html.escape(p['source'])}:{p['line']}</td></tr>"
                    for p in b["selected_stack_groups"].values())
    flagged = leaks[-1] if leaks else {}
    guard = receipt["guard"]
    peak_mib = to_mib(guard["rss_peak_sampled_bytes"])
    memory_report = f"""<!doctype html><html lang="fr"><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>V6 · Instantanés mémoire sous budget de 1 Gio</title>
<style>*{{box-sizing:border-box}}body{{margin:0;background:#f4f6f8;color:#203044;font:16px/1.65 system-ui,sans-serif}}main{{max-width:1120px;margin:auto;padding:28px 20px 60px}}header,section{{padding:25px;background:white;border:1px solid #dce3ea;border-radius:14px;margin-bottom:20px;min-width:0}}h1{{font-size:clamp(29px,4vw,42px);line-height:1.15;letter-spacing:-.035em}}h2{{font-size:23px}}h3{{font-size:18px}}a{{color:#3157a5}}code{{overflow-wrap:anywhere;font-size:.88em;background:#f1f4f8;padding:2px 4px}}.note{{padding:15px;background:#eef3fa;border-left:3px solid #3157a5}}.warn{{background:#fff5e9;border-color:#a46d35}}.scroll{{overflow:auto}}table{{width:100%;border-collapse:collapse;font-size:14px}}th,td{{text-align:left;vertical-align:top;padding:12px 10px;border-bottom:1px solid #dce3ea}}th{{background:#f5f7fa}}.muted{{font-size:13px;color:#5b6778}}li+li{{margin-top:9px}}@media(max-width:600px){{main{{padding:12px}}header,section{{padding:18px}}table{{font-size:12px}}}}@media print{{body{{background:white}}section,header{{border:0;padding:0}}.scroll{{overflow:visible}}tr{{break-inside:avoid}}}}</style>
<main><header><p class="muted">Moli · Dossier V6 · 27 octobre 2026</p>
<h1>La mémoire part surtout dans les descriptions et les tampons MQTT.</h1>
<p>Deux graphes mémoire réels du worker à vingt maisons ont été capturés et analysés avec les outils Apple.
Le budget surveillé est de <strong>1 Gio</strong> ; le maximum RSS échantillonné atteint <strong>{fr(peak_mib)} Mio</strong>, sans franchissement observé.</p>
<p class="note">La seconde capture contient {fr(b['heap_blocks'],0)} allocations malloc pour {fr(to_mib(b['heap_bytes_rounded_allocations']))} Mio de tailles arrondies.
Les nœuds d’arbres JSON représentent {fr(to_mib(b['allocation_classes']['json_nodes']['bytes']))} Mio, les allocations attribuées à <code>String::clone</code>
{fr(to_mib(b['allocation_classes']['string_clone_symbol']['bytes']))} Mio. Une autre vue des piles montre environ 76 Mio de tampons MQTT.</p>
<p><a href="charge.html">Campagne de charge</a> · <a href="index.html">Dossier V6</a> ·
<a href="heap-snapshot-recu.json">Reçu des captures</a> · <a href="heap-snapshot-analyse.json">Mesures et sources</a></p></header>
<section id="budget"><h2>01 · Budget et méthode</h2>
<p>1 Gio signifie 1 073 741 824 octets. Le contrôleur vérifie le RSS du seul worker toutes les 0,5 seconde et peut arrêter son groupe de processus.
Sur ce poste macOS sans Docker installé, cette garde est une <strong>surveillance échantillonnée</strong>, avec dépassement possible entre mesures, et non un plafond cgroup strict.
Elle n’a pas été déclenchée lors des {guard['samples']} observations.</p>
<p>Le même binaire optimisé est lancé avec <code>MallocStackLogging=lite</code>. Les allocations passent par l’allocateur système.
<code>leaks --noContent --outputGraph</code> enregistre deux memgraphs privés, puis <code>heap</code>, <code>vmmap</code> et <code>malloc_history</code>
les analysent hors ligne. Les symboles du binaire suffisent à rattacher les allocations aux chemins Rust ; les fichiers bruts restent hors dépôt.</p>
<p class="warn">Le profilage a perturbé l’initialisation : incidents MQTT et redémarrages ont été observés. La première capture n’est pas un état stabilisé ;
la seconde correspond au modèle complet de 31 540 points et 31 250 retained. Les deux instants ne forment pas un test de fuite à modèle constant.
La parité des workflows du passage profilé est fausse ; les résultats fonctionnels et CPU de référence viennent des passages non instrumentés.</p></section>
<section id="types"><h2>02 · Vue par allocations : ce qui occupe le tas</h2>
<div class="scroll"><table><thead><tr><th>Origine reconnue dans les piles</th><th>Capture initiale, Mio</th><th>Seconde capture, Mio</th><th>Blocs seconde capture</th></tr></thead><tbody>{rows}</tbody></table></div>
<p>Ces lignes sont une partition des classes dérivées par l’outil, tailles malloc arrondies comprises. Elles ne sont pas des tailles logiques de textes,
et l’optimisation du compilateur peut fusionner des symboles. <code>String::clone</code> ne veut pas dire que tous ces octets sont des titres musicaux :
les identifiants, clés et métadonnées sont aussi des chaînes.</p>
<p>Le calcul structurel des 31 540 Sample de 32 octets donnerait seulement environ 0,96 Mio, hors index et clés.
Le POC garde les canaux et descriptions dans des conteneurs dynamiques, plusieurs représentations JSON et le modèle du Hub :
ce coût est très supérieur au seul stockage des valeurs. Les canaux température et humidité restent indépendants ; les supprimer ne serait pas l’optimisation recherchée.</p>
<p><strong>Priorité :</strong> remplacer les copies profondes de descriptions par une vue canonique immuable ou une lecture froide, et sérialiser des structures typées
plutôt que conserver partout des arbres JSON à clés répétées. La méthode <code>metadata::Bank::read()</code> clone actuellement le JSON en mode RAM ;
<code>House::publications()</code> construit et compare aussi des versions. Les snapshots montrent ces chemins, sans prouver que toutes leurs allocations sont inutiles. Une pile indique le site d’allocation, pas le propriétaire actuel : canonique, publication en attente et temporaire peuvent se mêler.</p></section>
<section id="buffers"><h2>03 · Vue par piles : le coût du chargement initial reste dans les buffers</h2>
<div class="scroll"><table><thead><tr><th>Groupe observé</th><th>Volume arrondi</th><th>Blocs</th><th>Source privée relative</th></tr></thead><tbody>{paths}</tbody></table></div>
<p class="note">Cette vue recoupe la précédente : <strong>ne pas additionner les deux tableaux</strong>. Les piles sont inclusives ;
un parent et son descendant ne représentent pas deux allocations distinctes.</p>
<p>Le broker lit de gros registres et descriptions au démarrage ; ses <code>BytesMut</code> grossissent. Les clients MQTT des ponts conservent eux aussi un tampon d’écriture devenu grand.
La source montre des buffers réutilisés entre paquets ; les mesures trouvent 40 gros tampons de lecture et 20 d’écriture.
Ce sont des capacités à examiner, en plus des payloads retained nécessaires.</p>
<p><strong>Piste concrète :</strong> rendre une grosse capacité de lecture après un paquet froid, lorsque le tampon ne contient plus de fragment en attente,
et mesurer une politique analogue pour l’écriture. Il faut conserver les bornes, l’assemblage des paquets partiels et les données réellement retenues.
Le gain potentiel doit être vérifié avant/après ; environ 76 Mio de buffers observés ne garantissent pas 76 Mio de RSS libérés.</p></section>
<section id="automations"><h2>04 · Une copie de tous les états pour les automatismes</h2><p>Le chemin <code>remember_all</code> du moteur construit une mémoire précédente sur les états du Hub. Les piles lui attribuent environ 8 Mio, dont 4,76 Mio de clés et 3,08 Mio de table. Le banc ne référence pourtant que 60 entrées dans ses quarante graphes.</p><p>Une piste est de partager les identifiants et de limiter cette mémoire précédente aux dépendances nécessaires. Les canaux restent tous conservés dans le Hub. Cette optimisation doit préserver les franchissements de seuil, les transitions, les changements de graphes et la reprise après perte du bus ; elle n’est pas livrée ici.</p></section>
<section id="interpretation"><h2>05 · Occupation, réserve et suspicion de fuite</h2>
<p>Les tailles malloc passent de {fr(to_mib(a['heap_bytes_rounded_allocations']))} à {fr(to_mib(b['heap_bytes_rounded_allocations']))} Mio,
alors que leur nombre diminue de {fr(a['heap_blocks']-b['heap_blocks'],0)}. Le modèle et les files évoluaient encore : ce changement ne démontre pas une fuite.</p>
<p>L’empreinte physique rapportée par vmmap passe de {fr(to_mib(a['physical_footprint_bytes_rounded']),1)} à
{fr(to_mib(b['physical_footprint_bytes_rounded']),1)} Mio. Le RSS, cette empreinte, les tailles malloc arrondies et les octets demandés au Meter
sont des mesures distinctes. Des zones d’allocateur vides ou réservées et des pages partagées expliquent aussi une partie des écarts ; aucune soustraction unique ne les ventile précisément.</p>
<p>Une recherche de fuites sur la seconde capture signale <strong>{fr(flagged.get('blocks_flagged',0),0)} blocs, {fr(to_mib(flagged.get('bytes_flagged',0)))} Mio</strong>.
Le mode conservateur produit le même signalement. Le premier groupe contient {fr(leak_roots.get("network_write_buffers",dict(blocks=0))["blocks"],0)} grands tampons d’écriture <code>rumqttc / BytesMut</code> pour {fr(leak_roots.get("network_write_buffers",dict(bytes=0))["bytes"],0)} octets ; l’autre racine principale vient de <code>remember_all</code>.
Ce résultat demande validation : capacité encore possédée, pointeurs encodés ou intérieurs et vraie mémoire sans propriétaire ne se distinguent pas par le RSS.
La libération après fermeture des connexions et la croissance sur une durée plus longue restent à tester avant de conclure à une fuite Rust.</p>
<ol><li><strong>D’abord :</strong> réduire les copies de descriptions et mesurer le tas vivant par catégories.</li>
<li><strong>Ensuite :</strong> rendre les grandes capacités réseau après initialisation, sans perdre les paquets partiels.</li>
<li><strong>Puis :</strong> suivre un état stabilisé et la fermeture des connexions pour qualifier les blocs signalés par leaks.</li>
<li><strong>Enfin :</strong> comparer release, mêmes données et mêmes clients, avec le RSS, l’empreinte et les compteurs séparés.</li></ol>
<p>La représentation inline des textes courts peut supprimer de petites allocations ; elle n’explique pas à elle seule les dizaines de Mio d’arbres JSON et de buffers.
Aucune optimisation de stockage n’est présentée comme déjà livrée par cette analyse.</p></section>
<section id="preuves"><h2>06 · Preuves conservées</h2>
<p>Les deux memgraphs font {fr(receipt['captures'][0]['graph_bytes']/1048576)} et {fr(receipt['captures'][1]['graph_bytes']/1048576)} Mio.
Les captures et les six analyses natives ont réussi. Les fichiers publics contiennent les totaux, empreintes des artefacts, lignes sources et limites ;
les graphes bruts et les sorties complètes restent dans leur dossier temporaire privé.</p>
<p><a href="heap-snapshot-recu.json">Reçu natif</a> · <a href="heap-snapshot-analyse.json">Agrégats exacts et empreintes</a> ·
<a href="charge-empreintes.json">Code et vérifications</a> ·
<a href="../../../../tools/mqtt-live-probe/snapshot-load.py">Capture reproductible</a> ·
<a href="../../../../tools/mqtt-live-probe/analyze-heap-snapshot.py">Analyse reproductible</a>.</p>
<p class="muted">Documentation primaire : <a href="https://developer.apple.com/library/archive/documentation/Performance/Conceptual/ManagingMemory/Articles/MallocDebug.html">journalisation malloc Apple</a>,
<a href="https://developer.apple.com/documentation/xcode/gathering-information-about-memory-use">graphes mémoire et piles d’allocation</a>.
Les aides des outils installés précisent les options de cette capture. Aucune commande physique n’a été envoyée.</p></section></main></html>"""
    (PUBLIC/"memoire-1g.html").write_text(memory_report)
    print(json.dumps({"analysis": "heap-snapshot-analyse.json", "html": "memoire-1g.html",
                      "heap_end_bytes": b["heap_bytes_rounded_allocations"],
                      "categories": {k: c["bytes"] for k,c in b["allocation_classes"].items()}}, ensure_ascii=False))


if __name__ == "__main__":
    main()
