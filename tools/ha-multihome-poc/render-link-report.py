#!/usr/bin/env python3
"""Render the two-instance Mac qualification from recorded measurements."""
from pathlib import Path
import html
import json

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / 'docs/proposition/analyse/v6'
result = json.loads((OUT / 'ha-multimaison-pont-local.json').read_text())
if not (result.get('status') == 'PASS' and result.get('qualification_120s')
        and result.get('native_checks_match_binary') and not result.get('quick')
        and not result.get('fixture')):
    raise SystemExit('Le rapport final exige la campagne complète et son reçu natif vérifié.')


def esc(value):
    return html.escape(str(value))


def number(value, digits=2):
    return f'{value:.{digits}f}'.replace('.', ',')


def mib(value):
    return number(value / 1048576) + ' Mio'


def table(headers, rows):
    return '<div class="table-wrap"><table><thead><tr>' + ''.join(
        '<th>' + esc(value) + '</th>' for value in headers) + '</tr></thead><tbody>' + ''.join(
            '<tr>' + ''.join('<td>' + esc(value) + '</td>' for value in row) + '</tr>'
            for row in rows) + '</tbody></table></div>'


names = {'bilateral_cut_8s': 'Fermeture TCP des deux ponts',
         'bilateral_blackhole_3s': 'Silence bref, mêmes connexions',
         'one_way_cut_8s': 'Arrêt du seul flux B→A',
         'bilateral_blackhole_120s': 'Silence prolongé des deux ponts'}
cases = []
for case in result['cases']:
    gains = case.get('checks', {})
    snapshots = sum(case.get('recovered', {}).get(node, {}).get('snapshot_recoveries', 0)
                    - case.get('before', {}).get(node, {}).get('snapshot_recoveries', 0)
                    for node in ('node-a', 'node-b'))
    cases.append([names.get(case['case'], case['case']), case['status'],
                  number(case.get('fault_duration_ms', 0) / 1000, 3) + ' s',
                  ' / '.join(str(gains.get(node, {}).get('local_sequence_gain', '—'))
                             for node in ('node-a', 'node-b')),
                  number(case.get('recovery_ms', 0) / 1000, 3) + ' s',
                  str(snapshots) + ' instantanés' if snapshots else 'Deltas'])

last = result['cases'][-1]
memory = []
for node, label in [('node-a', 'Instance A'), ('node-b', 'Instance B')]:
    metrics = result['resources'][node]
    final = last['resources'][node]
    memory.append([label, mib(final['allocator_live_bytes']), mib(final['allocator_peak_bytes']),
                   mib(final['process_sample']['rss_bytes']), mib(metrics['rss_peak_sampled_bytes']),
                   number(metrics['cpu_percent_one_core']) + ' %',
                   number(metrics['sampled_seconds']) + ' s'])
runner = result.get('runner_resources')
if runner:
    memory.append(['Contrôleur Python', '—', '—', '—',
                   mib(runner['rss_peak_sampled_bytes']),
                   number(runner['cpu_percent_one_core']) + ' %',
                   number(runner['sampled_seconds']) + ' s'])

sources = table(['Jeu initial', 'Appareils projetés', 'Points', 'État initial'], [
    [source['house'], source.get('devices', '—'), source.get('points', '—'),
     mib(source.get('snapshot_bytes', 0))] for source in result['sources']])
css = (OUT / 'index.html').read_text().split('<style>', 1)[1].split('</style>', 1)[0]
status = 'Quatre scénarios réussis' if result['status'] == 'PASS' else 'Campagne en échec'
page = '''<!doctype html><html lang="fr"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>V6 · Pannes du pont sur deux instances Mac</title><style>''' + css + '''</style></head><body><div class="wrap"><header class="hero"><p class="eyebrow">Moli · Qualification locale · 27 octobre 2026</p><h1>Deux instances autonomes.<br>Le pont peut tomber.</h1><p class="intro">''' + esc(status) + ''' sur le Mac : les deux maisons continuent localement pendant la perte de réplication, puis leurs copies retrouvent le même rang et la même empreinte.</p></header><p class="note"><a href="ha-multimaison.html">Rapport HA principal</a> · <a href="ha-multimaison-pont-local.json">Mesures enregistrées</a> · <a href="ha-multimaison-pont-local-preuves.json">Preuves et empreintes</a> · <a href="ha-multimaison-pont-local-protocole.md">Protocole</a></p><main class="main">
<section class="section"><h2>Périmètre demandé et exécuté</h2><p>Le propriétaire a précisé : faire tomber le pont entre deux instances sur le Mac. Chaque processus Rust possède son courtier MQTT, le primaire de sa maison et une copie passive de l’autre. Chaque primaire produit ses changements toutes les2s et renouvelle son bail toutes les200ms. Le contrôleur Python déclenche les défauts et lit les statuts ; il ne produit aucun changement d’état pour les maisons.</p>''' + table(['Instance', 'Maison A', 'Maison B'], [['A', 'Active', 'Passive'], ['B', 'Passive', 'Active']]) + '''<p>Les ports sont distincts et toutes les adresses sont loopback. Les entrées sont les deux structures HA exportées, anonymisées puis projetées par le pont existant. Les changements pendant le test sont synthétiques ; aucun connecteur vers les appareils réels n’est chargé.</p>''' + sources + '''<p>La campagne vise une perte de pont locale. Elle ne revendique pas une panne du Mac entier, un déploiement dans deux maisons, un consensus distribué ou une reprise de radios et de secrets.</p></section>
<section class="section"><h2>Résultats des quatre pannes</h2>''' + table(['Défaut', 'Résultat', 'Durée mesurée', 'Rangs gagnés A / B', 'Convergence après rétablissement', 'Reprise observée'], cases) + '''<p>La durée mesurée inclut les contrôles de mise en défaut et de rétablissement ; chaque entrée interrompue a été maintenue en défaut au moins pendant la durée visée. Le compteur d’octets transmis reste constant après stabilisation sur les entrées interrompues. Les copies visées gardent leur rang et leur empreinte ; les deux primaires continuent et leurs effets factices sont admis. Les baux restent valides, les rôles restent actifs chez soi et passifs pour l’autre, à l’époque1.</p><p>Le silence de3s reprend les mêmes connexions : compteurs d’acceptation, générations et connexions RPC inchangés. Le cas B→A laisse progresser la copie de A chez B. Après120s, chaque primaire a gagné60rangs et60effets factices ; les deux copies ont repris chacune par un instantané.</p><p>La convergence est contrôlée sans gel : source avant lecture, copie distante, source après lecture doivent présenter le même rang et le même SHA-256. Le retour des deux maisons est vérifié sous une échéance de30s. Ces temps sont des mesures loopback en debug, pas des RTO entre deux sites.</p></section>
<section class="section"><h2>Mémoire et CPU mesurés</h2>''' + table(['Processus', 'Allocations vivantes après reprise longue', 'Pic d’allocations', 'RSS après reprise longue', 'Maximum RSS échantillonné', 'CPU moyen, un cœur', 'Fenêtre CPU'], memory) + '''<p>Les deux processus incluent chacun deux documents de maison, leur courtier, le primaire, le passif et les tâches de réplication. RSS et CPU sont relevés par PID avec <code>ps</code> sur macOS, toutes les2s et aux fins de cas. Les allocations proviennent de l’allocateur instrumenté. Le contrôleur Python est mesuré séparément et inclut la préparation des jeux d’entrée.</p><p>Les allocations vivantes ne s’ajoutent pas au RSS. Les maximums sont échantillonnés ; une somme de RSS ne mesure pas des pages physiques uniques. Le pic d’initialisation et de reconstruction des documents JSON diffère du volume encore alloué après reprise. Ce banc ne remplace pas un snapshot de tas pour attribuer chaque page, ni une mesure du cœur compact en release.</p><p>À chaque contrôle, la file reste sous256deltas/1Mio et les connexions de pair sous16. La suspension n’ajoute aucun tampon applicatif. Les files MQTT, les instantanés en construction et les buffers du noyau conservent leur propre coût. Le tampon stdin initial est libéré après son décodage dans le mode site.</p></section>
<section class="section"><h2>Preuves, reproduction et limites</h2><p><strong>67tests Rust, formatage et Clippy stricts réussis.</strong> La campagne utilise le binaire correspondant au reçu natif et aux empreintes de sources vérifiées. Les résultats bruts conservent les statuts avant, pendant et après chaque défaut, les compteurs de connexions, les reprises, les empreintes et les échantillons de processus. Le script exécuté est archivé à son empreinte exacte dans <a href="bancs/pont-local-runner-execute.py">les sources de preuve du banc</a>.</p><pre><code>python3 tools/ha-multihome-poc/run-link-faults.py</code></pre><p>La variante <code>--skip-check</code> réutilise un reçu natif vérifié. Les variantes <code>--fixture</code> et <code>--quick</code> servent aux diagnostics et ne qualifient pas la campagne complète de120s sur les deux exports.</p><p>Les faux workflows prouvent l’admission déterministe des effets, pas une livraison à un appareil. L’égalité finale prouve la convergence de l’état, pas la livraison de chaque état intermédiaire. Aucune promotion étrangère n’est exécutée par ce mode. La durée de vie est bornée pour rester sous le plafond de256reçus, sans les effacer. Le service interactif1416 peut atteindre ce plafond après une session prolongée ; ce constat antérieur reste documenté.</p><p>Les deux instances temporaires ont accepté l’arrêt HTTP puis ont été récoltées avec un code0, sans terminaison forcée. Les services1414 et1416 sont restés en place. Zéro commande physique, aucune modification VPN ou HA. La tentative interhôtes antérieure reste un <a href="ha-multimaison-wan.html">constat séparé et historique</a> ; elle n’est pas un prérequis à cette simulation locale.</p></section></main></div></body></html>'''
(OUT / 'ha-multimaison-pont-local.html').write_text(page)
print('Rapport des pannes du pont local rendu depuis les mesures.')
