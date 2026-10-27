#!/usr/bin/env python3
"""Build the public V6 load report from aggregate evidence only."""
import html
import json
from pathlib import Path
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt

ROOT = Path(__file__).resolve().parents[2]
FOLDER = ROOT / "docs/proposition/analyse/v6"
NAMES = {
    "smoke": "Contrôle initial corrigé · 2 maisons",
    "two-shared-memory": "2 maisons · broker partagé · RAM",
    "twenty-shared-memory": "20 maisons · broker partagé · RAM",
    "twenty-partitioned-memory": "20 maisons · brokers isolés · RAM",
    "twenty-partitioned-disk": "20 maisons · brokers isolés · disque",
    "twenty-burst-disk": "20 maisons · rafale · disque",
    "twenty-burst-memory": "20 maisons · rafale · RAM",
    "twenty-soak-memory": "20 maisons · 120 secondes · RAM",
    "twenty-accounting-memory": "20 maisons · mesure du générateur · RAM",
}


def number(value, digits=2):
    if value is None:
        return "—"
    return f"{value:,.{digits}f}".replace(",", "\u202f").replace(".", ",")


def mib(value):
    return number(value / 1048576 if value is not None else None)


def case_view(case):
    worker = case.get("worker") or {}
    end = worker.get("measured_end") or {}
    resource = case.get("resources") or {}
    flows = worker.get("workflows") or {}
    broker = end.get("broker") or {}
    measured_peak = max(resource.get("rss_measured_peak_sampled_bytes") or 0,
                        (resource.get("start") or {}).get("rss_bytes") or 0,
                        (resource.get("end") or {}).get("rss_bytes") or 0)
    raw = sum(s["received"] for s in end.get("sources", [])) - sum(
        s["received"] for s in (worker.get("bootstrap") or {}).get("sources", []))
    return dict(case=case, worker=worker, end=end, resource=resource, flows=flows, broker=broker,
                peak=measured_peak, raw=raw, rate=raw / max(case.get("seconds", 0), .001))


def main():
    data = json.loads((FOLDER / "charge-resultats.json").read_text())
    cases = [case_view(case) for case in data["cases"] if case.get("worker")]
    lookup = {row["case"]["name"]: row for row in cases}
    selected = [row for row in cases if row["case"]["name"] not in ("smoke", "twenty-accounting-memory")]
    fig, axes = plt.subplots(1, 2, figsize=(12, 6.8), constrained_layout=True)
    labels = [NAMES[row["case"]["name"]].replace(" · ", "\n", 1) for row in selected]
    indexes = list(range(len(selected)))
    colors = ["#36715e" if row["case"].get("healthy") else "#a46d35" for row in selected]
    axes[0].barh(indexes, [row["resource"]["rss_peak_sampled_bytes"] / 1048576 for row in selected],
                 color=colors, alpha=.32, height=.65, label="RSS max échantillonné, démarrage inclus")
    axes[0].barh(indexes, [row["end"]["allocator_live_bytes"] / 1048576 for row in selected],
                 color=colors, height=.32, label="Allocations demandées vivantes à la fin")
    axes[0].set_yticks(indexes, labels)
    axes[0].set_xlabel("Mio · mesures distinctes, non cumulables")
    axes[0].set_title("Mémoire du worker Rust")
    axes[0].legend(loc="lower right", fontsize=7)
    axes[1].barh(indexes, [row["resource"]["cpu_percent_one_core"] for row in selected], color=colors, height=.65)
    axes[1].set_yticks(indexes, labels)
    axes[1].set_xlabel("% d’un cœur logique · moyenne de la phase")
    axes[1].set_title("CPU · brun = scénario non validé")
    for axis in axes:
        axis.invert_yaxis()
        axis.grid(axis="x", alpha=.16)
        axis.set_axisbelow(True)
        axis.spines[["top", "right"]].set_visible(False)
        axis.tick_params(labelsize=8)
    matplotlib.rcParams["svg.hashsalt"] = "moli-load-v6"
    fig.savefig(FOLDER / "charge-ressources.svg", format="svg", metadata={"Date": None})
    plt.close(fig)
    svg = (FOLDER / "charge-ressources.svg").read_text()
    svg = svg[svg.index("<svg"):]
    rows = []
    flow_rows = []
    for row in cases:
        c, w, r, f, b = [row[key] for key in ("case", "worker", "resource", "flows", "broker")]
        label = html.escape(NAMES[c["name"]])
        status = "Validé dans ce scénario" if c.get("healthy") else "Non validé"
        cls = "good" if c.get("healthy") else "bad"
        rows.append(f"<tr><td>{label}<br><span class='{cls}'>{status}</span></td>"
                    f"<td>{number(c['seconds'],1)} s<br>{number(row['rate'],1)} msg/s reçus</td>"
                    f"<td>{mib(r['start']['rss_bytes'])} → {mib(r['end']['rss_bytes'])}</td>"
                    f"<td>{mib(row['peak'])}<br>avec démarrage : {mib(r['rss_peak_sampled_bytes'])}</td>"
                    f"<td>{mib(row['end']['allocator_live_bytes'])}</td>"
                    f"<td>{number(r['cpu_percent_one_core'])} %</td></tr>")
        expected = c["workflow_expected_per_kind"]
        flow_rows.append(f"<tr><td>{label}</td><td>{f.get('automations',0)}</td>"
                         f"<td>{f.get('threshold_runs_done',0)} / {expected}</td>"
                         f"<td>{f.get('opening_runs_done',0)} / {expected}</td>"
                         f"<td>{f.get('synthetic_threshold_commands',0)+f.get('synthetic_opening_commands',0)}</td>"
                         f"<td>{number(f.get('threshold_duration_p95_ms'),0)} / "
                         f"{number(f.get('opening_duration_p95_ms'),0)} ms</td>"
                         f"<td>{f.get('runs_failed',0)}<br>messages perdus par observateurs : "
                         f"{f.get('runs_observer_lagged_messages',0)} / "
                         f"{f.get('notices_observer_lagged_messages',0)}</td></tr>")
    good = lookup["twenty-partitioned-memory"]
    soak = lookup.get("twenty-soak-memory", good)
    burst = lookup.get("twenty-burst-memory", good)
    shared = lookup["twenty-shared-memory"]
    disk = lookup.get("twenty-partitioned-disk", {})
    accounting = lookup.get("twenty-accounting-memory", {})
    gen = accounting.get("case", {}).get("generator_resources") or {}
    base_sources = good["case"]["sources"]
    source_rows = "".join("<tr><td>Source " + chr(65+i) + "</td><td>"
                          + number(s["registry_devices"],0) + "</td><td>"
                          + number(s["registry_entities"],0) + "</td><td>"
                          + number(s["state_entities"],0) + "</td><td>"
                          + number(s["registry_rooms"],0) + " / " + number(s["registry_floors"],0)
                          + "</td><td>" + number(s["initial_messages"],0) + "</td></tr>"
                          for i,s in enumerate(base_sources))
    errors = html.escape(json.dumps(shared["broker"]["errors"], ensure_ascii=False))
    disk_errors = html.escape(json.dumps(disk.get("broker",{}).get("errors",{}), ensure_ascii=False))
    html_page = f"""<!doctype html>
<html lang="fr"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>V6 · Charge de vingt maisons et workflows simulés</title>
<style>
:root{{--ink:#203044;--muted:#5b6778;--line:#dce3ea;--bg:#f4f6f8;--blue:#3157a5}}
*{{box-sizing:border-box}}body{{margin:0;background:var(--bg);color:var(--ink);font:16px/1.65 system-ui,sans-serif}}
main{{max-width:1240px;margin:auto;padding:30px 22px 60px}}header,section{{background:white;border:1px solid var(--line);border-radius:14px;padding:26px;margin-bottom:20px;min-width:0}}
h1{{font-size:clamp(28px,4vw,43px);line-height:1.15;letter-spacing:-.035em;margin:8px 0 20px}}h2{{font-size:23px;line-height:1.3}}h3{{font-size:18px}}a{{color:var(--blue)}}p{{max-width:1050px}}
.eyebrow,.muted{{font-size:13px;color:var(--muted)}}nav{{display:flex;gap:14px;flex-wrap:wrap;font-size:14px}}
.note{{background:#eef3fa;padding:14px 17px;border-left:3px solid var(--blue)}}.warn{{background:#fff5e9;border-color:#a46d35}}
.scroll{{overflow-x:auto;max-width:100%}}table{{width:100%;border-collapse:collapse;font-size:13px}}th,td{{text-align:left;vertical-align:top;padding:12px 10px;border-bottom:1px solid var(--line)}}th{{background:#f5f7fa}}
.good{{color:#22664e;font-weight:700}}.bad{{color:#94531f;font-weight:700}}code{{font-size:.87em;overflow-wrap:anywhere;background:#f1f4f8;padding:2px 4px}}
.chart svg{{width:100%;height:auto;max-width:100%}}ul{{padding-left:23px}}li+li{{margin-top:7px}}pre{{overflow:auto;padding:14px;background:#f1f4f8;font-size:12px}}:target{{scroll-margin-top:20px}}
@media(max-width:600px){{main{{padding:14px 12px}}header,section{{padding:18px}}table{{font-size:12px}}.chart{{overflow-x:auto}}.chart svg{{min-width:780px}}}}
@media print{{body{{background:white;font-size:10pt}}main{{padding:0}}section,header{{border:0;padding:0;margin-bottom:22px}}nav{{display:none}}tr{{break-inside:avoid}}.scroll{{overflow:visible}}}}
</style>
<main><header><p class="eyebrow">Moli · Dossier V6 · 27 octobre 2026 · Rust {html.escape(data['profile'])}</p>
<h1>Vingt maisons simulées : le traitement passe, le broker partagé décroche.</h1>
<p>Les deux exports HA sont rejoués dix fois chacun, avec variations et quarante workflows exécutés par le vrai moteur.
Les entrées passent par MQTT TCP, House puis le vrai Hub. Les sorties vont exclusivement à un pilote simulé.
<strong>Aucune commande physique, aucun message vers une maison réelle.</strong></p>
<p class="note">Sur le scénario RAM isolé par maison, {number(good['end']['hub_points'],0)} points et quarante workflows sont chargés.
Le passage nominal de 31 secondes utilise {number(good['resource']['cpu_percent_one_core'])} % d’un cœur.
Le suivi de 120 secondes et la rafale en RAM sont {"concluants sur leur fenêtre" if soak['case'].get('healthy') and burst['case'].get('healthy') else "à lire avec leurs limites"}.
Les échecs du broker partagé et du stockage disque restent visibles.</p>
<nav><a href="#protocole">Protocole</a><a href="#ressources">Mémoire et CPU</a><a href="#workflows">Workflows</a><a href="#limites">Échecs et suites</a><a href="#preuves">Preuves</a><a href="memoire-1g.html">Snapshots sous budget de 1 Gio</a><a href="index.html">Dossier V6 complet</a></nav></header>
<section id="protocole"><h2>01 · Ce qui est réellement simulé</h2>
<p>Générateur et système testé sont deux processus distincts. Le worker ne reçoit aucun secret ; ses sockets sont limités à la boucle locale et à des ports éphémères.
Le service 1414 courant n’est pas redémarré. Le générateur lit les exports historiques déjà disponibles ; les identités, noms et textes sont synthétisés en mémoire.
Il ne republie rien sur les brokers HA.</p>
<div class="scroll"><table><thead><tr><th>Corpus</th><th>Appareils du registre</th><th>Entités du registre</th><th>États du snapshot</th><th>Pièces / étages</th><th>Messages initiaux par copie</th></tr></thead><tbody>{source_rows}</tbody></table></div>
<p>Les registres comprennent des entrées désactivées ; le nombre de canaux projetés n’est donc pas leur simple total.
Les treize états sans entrée dans les deux registres restent des messages non associés ; aucune entité n’est inventée pour les rendre valides.
D’autres messages exclus ou désactivés peuvent aussi être comptés comme non associés : ce n’est pas une perte réseau.</p>
<p>Chaque maison reçoit en plus un appareil d’entrée à trois canaux pour les workflows et un appareil de sortie simulée à deux points.
Le total du scénario complet est de {number(good['end']['hub_devices'],0)} appareils/virtuels et {number(good['end']['hub_points'],0)} points du Hub,
dont {number(sum(s['projection']['channels'] for s in good['end']['sources']),0)} canaux projetés.
Les types, unités et catalogues observés sont conservés autant que l’export le permet ; les octets ne sont pas une copie exacte d’une maison actuelle.</p>
<p>Le nominal vise 8 messages par seconde et par maison ; la rafale 80, émis en lots chaque seconde.
Cela donne environ {number(soak['rate'],1)} messages/s reçus sur le suivi long, et {number(burst['rate'],1)} sur la rafale.
Le flux comprend aussi les trois états de contrôle toutes les six secondes. Il ne s’agit pas d’une capture temporelle exacte du trafic HA.</p>
<p>Le chargement initial est séparé de la phase mesurée : tous les topics attendus doivent exister, le flux source être stable trois secondes et le broker ne pas avoir signalé d’erreur.
Une erreur d’initialisation marque le scénario non validé ; sa CPU ne devient pas un comparatif de régime établi.</p></section>
<section id="ressources"><h2>02 · Mémoire et CPU mesurés</h2>
<div class="scroll"><table><thead><tr><th>Scénario</th><th>Fenêtre / débit</th><th>RSS début → fin, Mio</th><th>RSS max mesuré, Mio</th><th>Allocations vivantes fin, Mio</th><th>CPU / un cœur</th></tr></thead><tbody>{''.join(rows)}</tbody></table></div>
<p class="muted">RSS lu par ps une fois par seconde environ, avec les bornes de phase. « Avec démarrage » inclut l’initialisation.
La CPU est le delta du temps processeur sur la durée mesurée ; les lectures ont une granularité de 0,01 s. Ce ne sont ni des maxima instantanés ni des mesures cgroup.
Le worker inclut Hub, projection, brokers et moteur ; HTTP, navigateur/PWA, historique persistant et appareils sont exclus.</p>
<div class="chart">{svg}</div>
<h3>Le nombre de valeurs n’explique pas tout le coût</h3>
<p>À raison d’un Sample de 32 octets par point, les {number(good['end']['hub_points'],0)} emplacements représentent environ {mib(good['end']['hub_points']*32)} Mio,
hors clés, index, fiches, catalogues et allocations. Pourtant le scénario RAM utilise {mib(good['end']['allocator_live_bytes'])} Mio d’allocations demandées vivantes.
Le banc conserve plusieurs représentations JSON, les métadonnées de projection, les fiches du Hub, les payloads retained et les buffers.
Il reste loin d’un cœur compact qui ne garderait que ses valeurs.</p>
<p>Les {number(good['broker']['retained_topics'],0)} retained représentent {mib(good['broker']['retained_payload_bytes'])} Mio de payloads logiques,
dont {mib(good['broker']['description_ram_bytes'])} Mio de descriptions.
Le mode disque déplace environ {mib(disk.get('broker',{}).get('description_disk_bytes',0))} Mio de descriptions hors du stockage RAM du broker.
Les métadonnées JSON de House restent en mémoire. Le compteur vivant baisse dans cette mesure ; <strong>aucune baisse globale de RSS ou de CPU n’est démontrée par ces passages disque non validés</strong>.</p>
<p>Le RSS et les allocations demandées ne s’additionnent pas. Ce sont des comptabilités différentes ; certaines pages demandées peuvent ne pas être résidentes,
et le RSS inclut aussi d’autres pages. La forte baisse du RSS au fil d’une fenêtre ne prouve pas que le modèle a supprimé ses données : les compteurs et points restent chargés.</p>
<h3>Générateur compté séparément</h3><p>Le passage dédié mesure le générateur Rust et ses abonnés de contrôle : maximum RSS échantillonné
{mib(gen.get('rss_peak_sampled_bytes'))} Mio, CPU {number(gen.get('cpu_percent_one_core'))} % d’un cœur sur {number(gen.get('seconds'),1)} s.
Cette fenêtre inclut son démarrage et le vidage ; elle n’est pas identique à la phase worker. Ces coûts ne sont pas inclus dans les chiffres du worker.</p>
<h3>Ce qui reste à optimiser</h3><ul>
<li>Mesurer un profil de tas par catégorie avant de ventiler précisément les centaines de Mio.</li>
<li>Réduire les copies de descriptions et de fiches JSON ; conserver une représentation canonique froide au lieu de promettre que toutes les métadonnées sont déjà sorties de RAM.</li>
<li>Limiter chaque abonnement de pont à sa maison pour éviter la diffusion des états des vingt maisons vers tous les ponts ; cette amélioration n’est pas livrée ici.</li>
<li>Conserver les bornes et définir les budgets par maison et globalement avant une architecture à vingt maisons.</li>
<li>Ne pas attribuer ce coût aux seuls textes sélectionnés : les enums sont déjà codées, tandis que les enveloppes et les descriptions dominent encore ce banc.</li></ul>
<p>Les pics observés dépassent 512 Mio dans certains passages. Cela ne valide pas vingt maisons sur une box de 512 Mio.
Les budgets de production en pages anonymes et les fonctions réellement déployées restent à mesurer sur la cible.</p></section>
<section id="workflows"><h2>03 · Workflows réellement exécutés, sorties simulées</h2>
<ol><li>Température passant au-dessus de 22 °C et maintenue une seconde → mode « comfort » → attente d’une seconde → commande du point de sortie synthétique.</li>
<li>Ouverture passant de faux à vrai → notification locale au Hub → commande d’une seconde sortie synthétique.</li></ol>
<p>Chaque copie possède ses graphes propres, approuvés uniquement dans le Hub éphémère du banc. Toutes les entrées viennent du faux statestream par le vrai pont ;
le module de workflows ne les écrit pas directement dans le Hub. Les sorties n’ont aucun pilote physique ni route externe.</p>
<div class="scroll"><table><thead><tr><th>Scénario</th><th>Graphes</th><th>Seuil : terminé / attendu</th><th>Ouverture : terminé / attendu</th><th>Commandes simulées</th><th>p95 seuil / ouverture</th><th>Échecs / pertes des observateurs</th></tr></thead><tbody>{''.join(flow_rows)}</tbody></table></div>
<p>Sur les 120 secondes : {number(soak['flows'].get('threshold_runs_done',0),0)} runs seuil et
{number(soak['flows'].get('opening_runs_done',0),0)} runs ouverture terminés, pour les mêmes effectifs attendus.
L’oracle vérifie aussi les deux compteurs de commandes, les notifications, les pertes des observateurs, le nombre de sessions, les vagues réellement émises et la cadence minimale.
Zéro vague ne peut pas être accepté comme zéro exécution correcte.</p>
<p class="note">Le p95 est celui de la durée des graphes terminés, fenêtre bornée à 512 échantillons.
Le seuil inclut l’attente explicite d’une seconde dans le graphe ; le maintien préalable d’une seconde est avant le début du run.
Ce n’est ni une latence physique ni le délai complet depuis l’émission MQTT. Les échos du broker ne constituent pas une réponse d’appareil.</p>
<p>Le mode de contrôle est du texte. Un test du vrai moteur montre qu’un EnumCode ne satisfait pas un littéral textuel « comfort » :
la résolution catalogue/code des conditions reste à compléter. Le banc ne prétend pas valider tous les nœuds, imports HA, conditions négatives, reprises ou workflows de production.</p></section>
<section id="limites"><h2>04 · Échecs conservés et conditions de suite</h2>
<h3>Broker partagé à vingt maisons : non validé</h3><p>Le passage atteint {number(shared['broker']['retained_topics'],0)} topics retained et rencontre des déconnexions,
dont des demandes de resynchronisation retained. Il ne délivre que {shared['flows'].get('threshold_runs_done',0)} runs seuil et
{shared['flows'].get('opening_runs_done',0)} runs ouverture sur {shared['case']['workflow_expected_per_kind']} attendus de chaque.
Des événements sont également perdus par l’observateur de notices. La charge n’est pas correctement servie.</p>
<pre>{errors}</pre>
<p>Les files de sortie sont bornées à 128 livraisons. Les abonnements actuels des ponts couvrent toutes les maisons :
leur diffusion croisée est un facteur de surcharge plausible, à isoler par une correction puis une nouvelle mesure.
Le test ne prouve pas que le plafond de 16 384 topics est la cause de chaque déconnexion observée.</p>
<p>Indépendamment de ces déconnexions, la cible complète exige {number(good['broker']['retained_topics'],0)} topics et
{mib(good['broker']['retained_payload_bytes'])} Mio de payloads logiques, au-delà des limites globales actuelles de 16 384 topics et 32 Mio.
Les plafonds n’ont pas été relevés pour faire passer le test. Les vingt brokers isolés sont une configuration de comparaison, pas une migration de production décidée.</p>
<h3>Disque : résultats fonctionnels partiels, initialisation fragile</h3><p>Les quarante workflows terminent leurs runs attendus dans les passages disque,
mais le nominal rencontre des déconnexions ({disk_errors}) et la rafale une erreur d’envoi de description.
Ils sont donc marqués non validés malgré leurs comptes de workflows complets.
Il faut qualifier l’initialisation, la contre-pression et les reconnexions avant de les présenter comme aussi robustes que le scénario RAM.</p>
<h3>Portée temporelle et fonctionnelle</h3><p>Le suivi long dure deux minutes, pas plusieurs jours : il montre une mémoire demandée restant du même ordre,
sans prouver l’absence de fuite durable. Les exemples sont des exports historiques expurgés ; ils ne contiennent pas toute la distribution de titres musicaux,
les vidéos/caméras, toutes les métadonnées dynamiques ni la découverte complète de contrats de commandes.
Les canaux d’état sont testés ; les actions HA volet/chauffe-eau/Apple TV ne deviennent pas raccordées par ce banc.</p>
<p>Aucune commande réelle de volet n’a été envoyée. La règle RTS reste inchangée : dernier ordre et état observé sont distincts ; aucune position finale n’est inventée.</p></section>
<section id="preuves"><h2>05 · Preuves et reproduction</h2>
<p><a href="charge-resultats.json">Résultats bruts agrégés de tous les scénarios</a> ·
<a href="charge-initial-echec.json">Premier contrôle en échec, conservé</a> ·
<a href="charge-protocole.md">Protocole</a> · <a href="charge-empreintes.json">Empreintes et contrôles</a> ·
<a href="charge-ressources.svg">Graphique exportable</a>.</p>
<p>Le premier contrôle envoyait 24 °C avec un seuil strictement supérieur à 24 °C : le moteur ne déclenchait pas ce scénario.
Le banc a été corrigé à 22 °C et son test vérifie maintenant que la valeur envoyée dépasse bien le seuil.
L’échec initial n’a pas été effacé. La reprise conserve les résultats du même binaire et des mêmes exports hors ligne.</p>
<pre>python3 tools/mqtt-live-probe/run-load.py
# Sources privées hors dépôt ; aucun secret MQTT.
# RUSTUP_TOOLCHAIN=1.99.1, deux jobs, compilation release.
# --exports &lt;répertoire&gt; permet de choisir les deux exports.
# Les résultats du même binaire/jeu d’entrées sont repris.</pre>
<p>Sources : <a href="../../../../tools/mqtt-live-probe/src/load_test.rs">superviseur et worker</a>,
<a href="../../../../tools/mqtt-live-probe/src/load_fixture.rs">fixtures synthétiques</a>,
<a href="../../../../tools/mqtt-live-probe/src/load_wire.rs">transport source</a>,
<a href="../../../../tools/mqtt-live-probe/src/load_workflows.rs">graphes et sorties</a>,
<a href="../../../../tools/mqtt-live-probe/src/gateway_runtime/broker.rs">broker réel et bornes</a>.
La validation de compilation, les tests ciblés et les contrôles finaux sont consignés dans les empreintes.</p>
<p class="muted">Aucune donnée de maison, identité native, adresse réseau privée ou secret n’est joint. Les résultats sont des observations de ce banc sur le poste macOS ARM64, pas une parité complète avec vingt instances Home Assistant.</p></section></main></html>"""
    (FOLDER / "charge.html").write_text(html_page)
    print(json.dumps({"cases": len(cases), "html": str((FOLDER / "charge.html").relative_to(ROOT)),
                      "svg": "charge-ressources.svg"}, ensure_ascii=False))


if __name__ == "__main__":
    main()
