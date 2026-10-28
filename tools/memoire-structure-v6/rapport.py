#!/usr/bin/env python3
"""Rapport V6 autonome, déterministe, à partir des artefacts locaux publiables."""
import html
import json
import re
from pathlib import Path
import pedagogie
import alignement

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
OUT = ROOT / 'docs/proposition/analyse/catalogue-memoire-v6.html'


def esc(value):
    return html.escape(str(value))


def table(headers, rows):
    return '<div class="scroll"><table><thead><tr>' + ''.join('<th scope="col">' + esc(x) + '</th>' for x in headers) + '</tr></thead><tbody>' + ''.join('<tr>' + ''.join('<td>' + esc(x) + '</td>' for x in row) + '</tr>' for row in rows) + '</tbody></table></div>'


def link(path, label):
    return '<a href="../../../' + esc(path) + '">' + esc(label) + '</a>'


def fmt(value):
    return f'{value:,.0f}'.replace(',', ' ')


def mib(value):
    return f'{value / 1048576:.3f}'


def bars(title, unit, series):
    """Each chart has its own zero-based, linear axis; no mixed-unit overlay."""
    if not series:
        return '<p>Mesure absente de cette campagne.</p>'
    description = '; '.join(f'{name} : {value:.3f} {unit}' for name, value in series)
    width, height, left, plot = 850, 65 + len(series) * 40, 220, 470
    maximum = max(value for _, value in series) or 1
    parts = [f'<svg class="chart" role="img" aria-label="{esc(title)}" viewBox="0 0 {width} {height}"><title>{esc(title)}</title><desc>{esc(description)}</desc><text x="{left}" y="20" class="axis">0 → {maximum:.3g} {esc(unit)} · axe linéaire</text>']
    for index, (name, value) in enumerate(series):
        y = 40 + index * 40
        parts.append(f'<text x="8" y="{y+17}">{esc(name)}</text><rect x="{left}" y="{y}" width="{value/maximum*plot:.2f}" height="25" rx="3" fill="{["#1b6250", "#6e9472", "#c18a40", "#385f7c", "#8d697b"][index%5]}"/><text x="{left+plot+15}" y="{y+17}">{value:.3f} {esc(unit)}</text>')
    return '<figure>' + ''.join(parts) + '</svg><figcaption>' + esc(title) + '</figcaption></figure>'


def alias_diagram(demo):
    """One source rectangle per canonical ID; positions come from the Rust demo."""
    nodes = {n['id']: n for n in demo['nodes']}
    parts = ['<div class="diagram"><svg role="img" aria-label="Deux alias partagent un seul slot et un seul état canonique" viewBox="0 0 1120 405"><title>Des alias distincts convergent sur une source unique</title><desc>IDs 200 et 201 : slots de répertoire distincts, puis Alias32 distincts ; leur cache cible le même slot source 8 génération 1 et la même ligne 100 à 21. Alias 202 cible le calcul 101 à 42, avec son état propre.</desc><defs><marker id="alias-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0 0L10 5L0 10z" fill="#536f5c"/></marker></defs>']
    for x, width, label, color in [(5,210,'Z3 · répertoire ID/slot','#f0e7d2'),(235,240,'Z2 · Vec&lt;Alias32&gt;','#dfeada'),(505,245,'Z3 · slots sources','#f0e7d2'),(780,330,'Z1 · Vec&lt;Row96&gt;','#dfebe9')]:
        parts.append(f'<rect x="{x}" y="5" width="{width}" height="365" rx="6" fill="{color}"/><text x="{x+12}" y="30">{label}</text>')
    for alias_id, y in [(200,65),(201,155),(202,265)]:
        alias, source = nodes[alias_id], nodes[nodes[alias_id]['resolved_id']]
        parts.append(f'<rect x="17" y="{y}" width="185" height="65" rx="4" fill="#fffdf6"/><text x="29" y="{y+23}">ID {alias_id} → slot {alias["slot"]}</text><text x="29" y="{y+46}">gén. {alias["generation"]} → Alias index {alias["physical_index"]}</text>')
        parts.append(f'<rect x="247" y="{y}" width="216" height="65" rx="4" fill="#fffdf6"/><text x="260" y="{y+23}">Alias32 [{alias["physical_index"]}] · ID {alias_id}</text><text x="260" y="{y+46}">source {source["id"]} · cache {source["slot"]}, {source["generation"]}</text>')
        center=y+33
        target_y={200:120,201:160,202:305}[alias_id]
        parts.append(f'<path d="M202 {center}H247M463 {center}C485 {center} 488 {target_y} 517 {target_y}" fill="none" stroke="#536f5c" stroke-width="2" marker-end="url(#alias-arrow)"/>')
    for source_id,y in [(100,90),(101,255)]:
        source=nodes[source_id]
        title='Valeur physique' if source_id==100 else 'Calcul · résultat propre'
        parts.append(f'<rect x="517" y="{y}" width="221" height="100" rx="4" fill="#fffdf6"/><text x="530" y="{y+26}">GlobalSlot16 [{source["slot"]}]</text><text x="530" y="{y+50}">génération {source["generation"]} · arène Row96</text><text x="530" y="{y+76}">physical_index = {source["physical_index"]}</text>')
        parts.append(f'<rect x="795" y="{y}" width="300" height="100" rx="4" fill="#fffdf6"/><text x="809" y="{y+26}">{title}</text><text x="809" y="{y+50}">Row96 [{source["physical_index"]}] · ID {source_id}</text><text x="809" y="{y+76}">payload = {source["payload"]} · un seul état</text><path d="M738 {y+50}H795" fill="none" stroke="#536f5c" stroke-width="2" marker-end="url(#alias-arrow)"/>')
    parts.append('<text x="17" y="395">Indices et générations de demo.json après compaction ; aucune adresse mémoire absolue.</text></svg></div>')
    return ''.join(parts)



def observation_band():
    """A fixed-size line keeps two payloads and two dates; variable contents live outside."""
    fields = [(0, 16, 'ID + parent'), (16, 8, 'payload'), (24, 8, 'previous_payload'),
              (32, 8, 'received_ms'), (40, 8, 'changed_ms'), (48, 16, 'contrat + flags'),
              (64, 32, 'Numeric32')]
    parts = ['<div class="diagram"><svg role="img" aria-label="Row96 : deux payloads, deux dates et 96 octets" viewBox="0 0 1120 155"><title>La précédente valeur remplace un mot de date ; taille fixe inchangée</title><desc>ID et parent de 0 à 16 ; payload de 16 à 24 ; previous_payload de 24 à 32 ; received_ms de 32 à 40 ; changed_ms de 40 à 48 ; schéma, auxiliaire, version, flags et types de 48 à 64 ; Numeric32 de 64 à 96.</desc>']
    for index, (offset, size, label) in enumerate(fields):
        x, width = 20 + offset * 11.25, size * 11.25
        color = ['#dfeada', '#c4dbcc', '#e9d5af', '#d4e3ed', '#c7d9e6', '#e2e4d4', '#ddd8bd'][index]
        parts.append(f'<rect x="{x}" y="30" width="{width-1}" height="38" fill="{color}"/><text x="{x+4}" y="24">{offset}</text><text x="{x+4}" y="94" style="font-size:10px">{esc(label)}</text><text x="{x+4}" y="116">{size} octets</text>')
    parts.append('<text x="1084" y="24">96</text><text x="20" y="145">Row64 [0..64[ + Numeric32 [64..96[ ; Hot32 = payload + previous_payload + received_ms + changed_ms.</text></svg></div>')
    return ''.join(parts)



def operation_measurements(path):
    """Explain measured kernels without turning local timings into whole-engine claims."""
    if not path.exists():
        raise SystemExit('Attente de operations-banc/sorties/resultats.json : aucune mesure d’opération n’est inventée.')
    results = json.loads(path.read_text())
    metadata = results['metadata']
    measurements = results['measurements']
    scenarios = results['scenarios']
    if results['semantic_tests']['passed'] is not True:
        raise ValueError('Les vérifications sémantiques du banc doivent réussir avant le rapport.')
    required_operations = {'lookup', 'observe', 'fanout_update', 'alias_read', 'zone_members'}
    if not required_operations.issubset({row['operation'] for row in measurements}):
        raise ValueError('Le banc des opérations ne contient pas toutes les comparaisons annoncées.')
    if not measurements or not scenarios:
        raise ValueError('Le banc des opérations doit publier ses scénarios et ses mesures avant le rapport.')
    for row in measurements:
        if row['unit'] != 'ns/op' or row['median_ns'] < 0:
            raise ValueError('Unité ou durée inattendue dans le banc des opérations.')
    # Prefer the detailed combined inventory at its base scale; all measured sizes stay visible below.
    selected = next((scenario for scenario in scenarios
                     if scenario.get('dataset', scenario['scenario']) == 'somme-detail' and scenario['scale'] == 1),
                    max(scenarios, key=lambda scenario: scenario['canonical_rows']))
    selected_rows = [row for row in measurements
                     if (row['scenario'], row['scale']) == (selected['scenario'], selected['scale'])]
    scenario_by_id = {scenario['scenario']: scenario for scenario in scenarios}

    def dimension_label(scenario):
        return f"{scenario.get('dataset', scenario['scenario'])} ×{scenario['scale']}"

    selected_label = dimension_label(selected)
    variants = {
        'linear_scan': 'Scan linéaire', 'bank_read_at': 'Index + résolution Bank',
        'identical': 'Relevé identique', 'changed_previous': 'Changement + précédente',
        'canonical_alias': 'Source unique', 'materialized_row96_views': 'Miroirs Row96',
        'edge24_scan': 'Scan des Edge24', 'csr_targets': 'Tranche CSR',
    }

    def measured(operation, fanout=None):
        rows = [row for row in selected_rows if row['operation'] == operation
                and (fanout is None or row['fanout'] == fanout)]
        return ' ; '.join(f"{variants[row['variant']]} : {row['median_ns']:.3f} ns/op" for row in rows)

    body = '<p class="lead">On gagne du temps en retrouvant directement une fiche et en évitant de recopier le même état dans chaque vue. Ces gains ont un coût : index, références, vérifications et parfois une lecture supplémentaire. Voici les opérations réellement chronométrées avec la valeur précédente et les deux dates.</p>'
    body += '<div class="notice"><b>MESURÉ — Bank isolé, aucune commande physique.</b> Ce banc importe le modèle structurel V6 ; il ne migre pas le Hub et ne mesure ni MQTT, ni REST, ni garde, ni journal, ni notifications. Les tailles A/B/somme proviennent de formes anonymisées projetées dans une topologie synthétique ; ce ne sont pas des relectures de maisons réelles.</div>'
    body += f'<p>Les graphiques montrent <b>{esc(selected_label)}</b> : {fmt(selected["canonical_rows"])} lignes canoniques. Les autres dimensions mesurées sont conservées plus bas. Une nanoseconde (ns) vaut un milliardième de seconde ; les résultats sont des temps par opération dans ce processus, pas une latence de bout en bout.</p>'
    body += '<details><summary>Relier chaque opération aux structures et au coût mémoire</summary>' + table(['Opération', 'Travail évité et structures parcourues', 'Prix en mémoire', 'Résultat mesuré sur la charge affichée', 'Limite'], [
        ('Retrouver un ID', 'Le scan visite des Row96 jusqu’à trouver l’ID. L’index retrouve Handle8 puis vérifie slot, génération et ligne.', 'HashMap + GlobalSlot16 ; construction et réservations hors chronomètre.', measured('lookup'), 'Même valeur recherchée ; les vérifications Bank s’ajoutent à son chemin. Le gain dépend de N, des requêtes et du cache matériel.'),
        ('Recevoir une valeur identique', 'Comparer type/bits ; garder courante, précédente et changed_ms ; mettre à jour réception et revision.', 'Même ligne fixe. Pour ces entiers, aucun nouveau contenu de pool.', measured('observe'), 'Deux travaux différents, sans ratio de gain. Le temps comprend observe, relecture Bank et digest ; Bank republie Row64, sans compter les octets des caches CPU.'),
        ('Mettre à jour une source vue 64 fois', 'Source partagée : une observation canonique. Témoin : même observation puis copie de Row96 dans un buffer de snapshots.', 'Le banc alloue le même Bank avec 2 048 alias et le même buffer de 2 048 snapshots pour les deux variantes ; ce n’est pas une comparaison de RAM entre catalogues.', measured('fanout_update', 64), 'Copies gardant l’ID source, sans catalogue distinct. Ni lecture d’alias ni notification dans cette boucle ; ces coûts restent à ajouter.'),
        ('Lire par un alias', 'Retrouver la vue, vérifier epoch/source/générations puis lire la ligne source courante et précédente.', 'La vue garde une référence, sans état recopié.', measured('alias_read'), 'Partage économise les copies ; la lecture comporte des contrôles et des indirections, elle n’est pas gratuite.'),
        ('Lister une zone', 'Le scan filtre toutes les Edge24 ; le CSR lit la tranche d’adjacence de la zone.', 'Target12 + offsets et index inverse en plus des Edge24 ; buffers de construction exclus du temps de parcours.', measured('zone_members'), 'Parcours déjà construit, sans expansion récursive ni déduplication métier des équipements.'),
        ('Ranger des scalaires typés', 'Payloads, deux dates et contraintes vivent dans des buffers typés ; textes/composites passent par des handles.', '96 octets fixes + pools variables ; pas de JSON permanent par scalaire.', 'Layout vérifié ; gain face à un arbre JSON non chronométré dans ce banc.', '96 octets n’est pas une taille universelle de ligne de cache ; aucun gain CPU n’est déduit du seul layout.'),
    ]) + '</details>'
    graphs = [
        ('lookup', '1. Retrouver une ligne par son ID'),
        ('observe', '2. Recevoir un entier : deux contrats de travail'),
        ('fanout_update', '3. Mettre à jour la source ou ses miroirs'),
        ('alias_read', '4. Lire l’état partagé par une vue'),
        ('zone_members', '5. Parcourir les membres d’une zone'),
    ]
    for operation, title in graphs:
        rows = [row for row in selected_rows if row['operation'] == operation]
        series = [(variants[row['variant']] + (f" · {row['fanout']} vues" if row['fanout'] is not None else ''), row['median_ns']) for row in rows]
        body += '<h3>' + esc(title) + '</h3>'
        body += bars(f'{selected_label} · {title}', 'ns/op', series)
        if operation == 'observe':
            body += '<p>Les temps identique/changement sont affichés comme deux mesures absolues du parcours local, incluant observe, relecture de la ligne par Bank.read_at et calcul d’un digest. Le résultat métier diffère : on ne transforme pas leur rapport en accélération à résultat égal.</p>'
        elif operation == 'fanout_update':
            body += '<p>La source partagée reste unique quand le nombre de vues augmente. Le témoin ajoute la copie de Row96 dans un buffer de snapshots gardant le même ID source. Ce ne sont pas des points réinsérés dans un second catalogue avec leurs propres IDs, slots, métadonnées et index. La même fixture avec 2 048 alias et le même buffer de 2 048 copies sont alloués dans chaque variante. Ces boucles ne lisent pas les alias, ne notifient aucun abonné et n’exécutent aucune entrée/sortie ; une courbe plate ne promet donc pas un coût constant de livraison à toutes les vues.</p>'
    body += '<h3>Protocole, dimensions et mémoire conservée</h3>'
    body += '<p>Les contrôles sémantiques de la capture réussissent avant chronométrage : identité des lignes, égalité des lectures indexées et linéaires, résolution des alias, snapshots complets avec précédente/types/dates, paquet identique, Null précédent, membres de zone et refus d’observer un alias. Les cas exacts restent dans le JSON brut.</p>'
    body += table(['Paramètre', 'Valeur de la capture'], [
        ('Processus indépendants', metadata['process_repetitions']),
        ('Rondes par processus', metadata['rounds_per_process']),
        ('CPU / mémoire conteneur / réseau', f"{metadata['cpus']} / {metadata['memory']} / {metadata['network']}"),
        ('Compilation', metadata['flags']),
        ('Image de développement', metadata['image_id']),
    ])
    body += '<p>Le tableau de détail conserve médiane, minimum et maximum des échantillons. Un p95 sur ces quelques rondes est un résumé d’échantillons, jamais le p95 d’une API en charge. La chauffe, la création des Bank, index et CSR, les fixtures et les vérifications sémantiques sont hors des boucles mesurées. Le scan de zone et le CSR parcourent la même liste logique de membres et additionnent leurs slots sans allouer de résultat ; les durées sont par requête, sans les diviser par le nombre de membres. Pour le fanout, une opération met à jour une source avec ses vues ; pour la lecture, une opération lit une valeur. Les réservations de mémoire et les buffers nécessaires à leur construction restent à budgéter même si leur construction n’entre pas dans ces chronomètres. La construction massive de la fixture utilise un remplissage dédié qui évite les recherches de slots libres de insert ; elle ne qualifie donc pas les performances de création massive par cette API.</p>'
    dimension_rows = []
    for scenario in scenarios:
        memory = scenario['memory']
        vectors = memory['vectors']
        used = sum(vector['used_bytes'] for vector in vectors.values())
        reserved = sum(vector['capacity_bytes'] for vector in vectors.values())
        dimension_rows.append((dimension_label(scenario), fmt(scenario['canonical_rows']), fmt(scenario['physical_nodes']), fmt(scenario['projected_aliases']), fmt(vectors['aliases']['len']), fmt(used), fmt(reserved), fmt(scenario['requested_bank_allocation_bytes']), fmt(scenario['materialized_views']['capacity_bytes'])))
    body += table(['Projection de formes', 'Canoniques', 'Nœuds physiques', 'Alias projetés', 'Alias du Bank mesuré', 'Vec utiles, octets', 'Vec réservés, octets', 'Bank alloué avant CSR, octets', 'Buffer snapshots réservé, octets'], dimension_rows)
    body += '<p>Les deux colonnes Vec viennent des structures effectivement construites, CSR compris. La colonne Bank alloué compte les demandes à l’allocateur avant le CSR (Vec et HashMap) ; elle recouvre une partie des colonnes Vec et ne doit pas leur être ajoutée. Le buffer de snapshots est séparé. Les colonnes Vec excluent les octets des buckets/contrôles du HashMap ; la colonne Bank alloué inclut ses allocations réservées. Aucune de ces colonnes n’inclut le surcoût d’allocateur, les piles et les temporaires de construction : aucune n’est un total mémoire du moteur ou un RSS. Les tailles projetées des alias ne sont pas toutes matérialisées : le Bank instrumente 32 sources avec 64 alias chacune, soit 2 048 vues, pour mesurer les fanouts 1/4/16/64. Les canoniques suivent la dimension annoncée ; le JSON donne la construction exacte et distingue les alias projetés de ces alias instrumentés. Le buffer de miroirs réserve aussi 2 048 × 96 = 196 608 octets d’éléments utiles, à côté du Bank, dans toutes les variantes. Alias32 + slot + métadonnée demande 80 octets utiles par vue dans le modèle, tandis qu’un snapshot Row96 ne représente que ses 96 octets d’éléments : ces périmètres différents ne prouvent aucun gain de mémoire entre deux catalogues complets.</p>'
    detail_rows = []
    for row in measurements:
        detail_rows.append((dimension_label(scenario_by_id[row['scenario']]), row['operation'], variants[row['variant']], '—' if row['fanout'] is None else row['fanout'], f"{row['median_ns']:.3f}", f"{row['min_ns']:.3f}–{row['max_ns']:.3f}", f"{row['allocations_per_op']:.3g}", f"{row['allocated_bytes_per_op']:.3g}"))
    body += '<details><summary>Toutes les mesures d’opérations et compteurs d’allocation</summary>' + table(['Projection', 'Opération', 'Variante', 'Vues', 'Médiane, ns/op', 'Min–max, ns/op', 'Allocations/op', 'Octets demandés/op'], detail_rows) + '</details>'
    body += '<p>Les compteurs d’allocation concernent uniquement les opérations délimitées par le banc ; zéro allocation dans une boucle ne signifie pas zéro mémoire réservée, ni zéro copie, ni zéro travail dans le futur Hub.</p>'
    body += '<h3>Reproduire et vérifier le périmètre</h3><ul>' + ''.join('<li>' + link('tools/memoire-structure-v6/operations-banc/' + path, label) + '</li>' for path, label in [('README.md', 'Protocole et commande de reproduction'), ('main.rs', 'Microkernels Rust, contrôles et frontières des chronomètres'), ('executer.py', 'Lanceur : processus indépendants et agrégation'), ('sorties/resultats.json', 'Résultats bruts, échantillons et empreintes des sources')]) + '</ul>'
    body += '<h3>Ce que ces chronomètres ne démontrent pas</h3><ul>' + ''.join('<li>' + esc(limit) + '</li>' for limit in [
        'Le jeu de lookup contient 16 à 128 IDs canoniques existants, répétés par le chemin indexé. Les 32 sources et leurs vues restent petites et chaudes même quand l’arène grandit ; aucune fréquence de défauts de cache en charge aléatoire complète n’est mesurée.',
        'La zone contient au plus 256 membres directs. Ses IDs synthétiques denses permettent au scan de convertir ID−1 en slot, sans recherche HashMap supplémentaire ; des IDs durables arbitraires demandent une stratégie de correspondance.',
        'Instant mesure du temps écoulé, sans compter les cycles CPU. Le quota d’un CPU et les neuf rondes n’isolent ni les caches, ni l’ordonnancement de la VM et de l’hôte.',
        'Le compteur GlobalAlloc mesure les appels réussis et les octets demandés. Son instrumentation atomique peut perturber les chemins qui allouent ; elle ne mesure pas la fragmentation ou la résidence des pages.',
        'Une arène exécutée à ×100 qualifie ce microbanc de taille ; elle ne relève aucune limite du produit et ne prouve ni les notifications, ni les transports, ni la latence d’une maison complète.',
    ]) + '</ul><p>Le protocole complet et les limites exactes de la capture sont conservés dans les fichiers liés ci-dessus.</p>'
    body += '<p>Ces nouvelles mesures utilisent le contrat V6 avec précédente valeur. Les mesures V2–V5 restent des campagnes historiques de leurs propres implémentations. Les calculs de budget A/B/somme restent distincts de ces durées : aucune ancienne courbe ne mesure automatiquement ce nouveau contrat.</p>'
    return body


def main():
    budget_path = HERE / 'sorties/budgets.json'
    if not budget_path.exists():
        raise SystemExit('Attente de sorties/budgets.json : aucun nombre de budget n’est inventé.')
    data = json.loads(budget_path.read_text())
    layouts = json.loads((HERE / 'layouts.json').read_text())
    demo = json.loads((HERE / 'demo.json').read_text())
    measurements = data['measurements']
    metadata_path = HERE / 'sorties/metadata.json'
    metadata = json.loads(metadata_path.read_text()) if metadata_path.exists() else {}
    test_count = metadata.get('test_count')
    test_evidence = (f'ses {test_count} tests structurels ciblés'
                     if isinstance(test_count, int) and not isinstance(test_count, bool) and test_count > 0
                     else 'ses tests structurels ciblés (nombre et résultat dans le journal de tests de la capture)')
    sections = []

    def sec(key, title, body):
        sections.append((key, title, f'<section id="{key}"><div class="eyebrow">Catalogue mémoire / V6</div><h2>{title}</h2>{body}</section>'))

    pedagogie.append_sections(sec, layouts)
    lesson_key, lesson_title, lesson_body = sections[-1]
    sections[-1] = (lesson_key, lesson_title, lesson_body.replace('</section>', '<p><a href="#optimisations">Voir ce que ces choix accélèrent, avec les mesures →</a></p></section>'))
    sec('optimisations', 'Ce que l’organisation mémoire accélère, mesures à l’appui.',
        operation_measurements(HERE / 'operations-banc/sorties/resultats.json'))
    sec('alignement', '96 ou 128 octets : alignement, mémoire et vitesse.', alignement.body())

    sec('reponse', 'Une donnée, une identité, plusieurs vues.', '''<p class="lead">La proposition range les objets dans des arènes typées, retrouve chaque objet par son ID durable et partage les états au travers d’alias. La ligne de 96 octets reste le point de départ recommandé ; slots, métadonnées, graphes et textes ont leur propre budget.</p><div class="recommend"><b>Choix de référence : Row96 + Alias32 + slots générationnels.</b> Un canonique occupe déjà <strong>144 octets</strong> avec son slot et sa métadonnée ; un alias <strong>80 octets</strong>. Index, relations, chaînes, schémas et exécution s’ajoutent. Ces nombres ne sont pas des totaux RSS.</div><p>Dans les fiches d’organisation et d’action, les champs d’observation et la réserve numérique peuvent être absents mais restent alloués avec A. Ils ne signifient pas une mesure zéro. Une action autonome ne crée pas une valeur observée « bouton pressé » : ses exécutions et résultats vivent dans le journal et les enveloppes d’exécution. Un alias lit la valeur courante, la précédente et les deux dates de sa source, sans copie.</p>''' + table(['Aujourd’hui dans Moli', 'V6 proposée et éprouvée isolément', 'Ce qui reste à livrer'], [
        ('Hub typé : appareils, spécifications et échantillons dans ses collections Rust ; labels séparés ; accès sous verrou.', 'Vec<Row96>, Vec<Alias32>, Vec<GlobalSlot16>, Vec<Meta32> et HashMap<ID, Handle8>.', 'Migration du Hub, persistance, transactions, concurrence et mesure du moteur réellement migré.'),
        ('Profils déjà compilés, validation, garde, journal et pilotes existants.', 'Références compactes vers contrats et plans partagés ; intentions de commandes sans effet physique.', 'Raccorder ces références au Hub sans contourner ses validations.'),
        ('V2–V4 : microbancs ; V5 : adaptateurs devant le vrai Hub.', 'V6 : layouts réels, modèle structurel sûr et budgets explicites.', 'Les anciennes mesures ne deviennent pas des performances mesurées du stockage V6.')]))

    tree = '''<div class="diagram"><svg role="img" aria-label="Arborescence et deux zones transverses" viewBox="0 0 940 350"><title>Organisation : la zone référence, elle ne copie pas</title><g stroke="#6c8274" stroke-width="2" fill="none"><path d="M90 62V280M90 122H240V280M240 180H410V280M410 240H605"/><path d="M90 280H240M240 280H410M410 280H605"/><path d="M800 105C740 105 740 180 480 180M800 145C750 145 740 240 660 240" stroke="#b67c30" stroke-dasharray="6 5"/><path d="M800 295C740 295 740 180 480 180" stroke="#4e7887" stroke-dasharray="6 5"/></g><g fill="#e0e9da" stroke="#9daf9d"><rect x="25" y="22" width="160" height="44" rx="6"/><rect x="155" y="100" width="170" height="44" rx="6"/><rect x="320" y="158" width="170" height="44" rx="6"/><rect x="510" y="218" width="210" height="44" rx="6"/><rect x="510" y="273" width="210" height="44" rx="6"/><rect x="25" y="290" width="180" height="44" rx="6"/></g><g fill="#213f34" font-size="16"><text x="42" y="50">Famille → Maison</text><text x="173" y="128">Étage facultatif</text><text x="343" y="186">Pièce</text><text x="526" y="246">Physique → valeur/action</text><text x="526" y="301">Virtuel → alias/calcul</text><text x="40" y="318">Pièce sans étage</text><text x="780" y="95">Zone A</text><text x="780" y="138">Zone B</text><text x="766" y="287">Géozone ≠ zone</text></g></svg></div>'''
    sec('organisation', 'La hiérarchie contient ; les zones relient.', tree + '''<p>Famille → maison → étage facultatif → pièce organise les objets. Un équipement physique ou virtuel peut aussi être rattaché directement à sa maison. Valeurs, actions et calculs appartiennent à un équipement. La « famille » est ici un regroupement d’organisation, sans déduire de droits humains.</p><p>Une zone fonctionnelle porte une maison et référence des pièces ou équipements, avec chevauchement autorisé et sans copie d’état. Les liens entre maisons sont refusés. Une géozone de présence porte coordonnées/rayon en ressource froide distincte. Un groupe ou une zone Hue provient d’une intégration : sa sémantique et la restriction actuelle de commande restent à conserver, sans l’assimiler automatiquement à notre zone.</p><p>Le modèle rejette parent absent, cycle, mauvais type de parent et déplacement invalidant une zone. La topologie de 39 nœuds par monde utilisée dans les budgets est synthétique : elle n’a pas été découverte dans les inventaires HA.</p>''')

    regions = [('Z1', 'Canoniques', 'Row96 / Row64 / Hot+Cold'), ('Z2', 'Vues', 'Alias32 sans état'), ('Z3', 'Localisation', 'GlobalSlot16 + index ID'), ('Z4', 'Données froides', 'Meta32, schémas, contrats'), ('Z5', 'Graphes', 'Edge24 + CSR avant/inverse'), ('Z6', 'Variables', 'PoolEntry16 + octets'), ('Z7', 'Exécution', 'Plans, calculs, routes, temporaires')]
    region_svg = '<svg role="img" aria-label="Sept régions de stockage" viewBox="0 0 940 340"><title>Chaque région possède ses propres allocations</title>'
    for i, (number, title, subtitle) in enumerate(regions):
        x, y = (i % 2) * 470, (i // 2) * 83
        region_svg += f'<rect x="{x+3}" y="{y+3}" width="455" height="72" rx="5" fill="{["#dbe7d8", "#efe0c5"][i%2]}"/><text x="{x+18}" y="{y+30}" font-size="18">{number} · {title}</text><text x="{x+18}" y="{y+55}" font-size="13">{esc(subtitle)}</text>'
    region_svg += '</svg>'
    sec('regions', 'Sept régions, des responsabilités explicites.', '<div class="diagram">' + region_svg + '</div>' + table(['Entité', 'Où elle est rangée', 'Ce qu’elle possède / référence'], [
        ('Famille, maison, étage, pièce, zone, équipement', 'Z1 canonique + Z3 slot + Z4 méta', 'Identité, parent et type ; label, pièce, politique et extension par références.'),
        ('Valeur scalaire', 'Z1 ; schéma partagé en Z4', 'Valeur courante et précédente typées, deux dates, version ; unité et contraintes ne sont pas recopiées dans chaque alias.'),
        ('Alias de virtuel', 'Z2 + Z3 + Z4', 'Son ID, son propriétaire et le lien vers une source canonique ; aucune observation propre.'),
        ('Calcul', 'Résultat propre Z1 ; définition Calc48 Z7', 'Sources, opération, fraîcheur et historique référencés ; les alias peuvent lire son résultat.'),
        ('Action / contrat / cible', 'Contrat ET binding : chacun sa Row96 en Z1 ; signature Z4 ; Route32 à côté en Z7', 'Le budget compte c.contracts et c.bindings parmi les canoniques. binding.parent = équipement cible ; signature et paramètres partagés via schéma, cible/clé/plans dans le binding et la route. Un set intégré à une valeur ne crée pas automatiquement une Row96 d’action supplémentaire.'),
        ('Paramètres composés, texte, binaire', 'Z6 : répertoire PoolEntry16 + octets', 'Longueurs, type, génération et version ; gros contenus hors des lignes fixes.'),
        ('Liens parent, zones et dépendances', 'Z5 : sources Edge24 + index CSR', 'IDs durables aux sources, indices compacts dans les caches reconstruits.'),
        ('Plans, composants et conversions', 'Z4/Z7 : Schema64, Transform64, Component32', 'Tables typées partagées ; Meta32 ne contient que leurs références.'),
        ('JSON/MQTT, files, snapshots', 'Z7 temporaire, budget distinct', 'Décodage, lots, sérialisation et duplication temporaire ne sont pas inclus gratuitement dans le catalogue.')]))

    key, title, body = sections[-1]
    assert body.endswith('</section>')
    body = body[:-len('</section>')]
    body += '<h3>Attributs des lieux : références, pas duplication de la géométrie</h3>' + table(['Objet', 'Parent dans la ligne', 'Données froides référencées'], [
        ('Famille', 'Racine, parent = 0', 'Libellé et métadonnées ; ses maisons sont retrouvées par le CSR, pas copiées dans une liste de valeurs.'),
        ('Maison', 'ID famille', 'Fuseau, préférences et ressource de plan ; les droits sont un contrat séparé.'),
        ('Étage', 'ID maison', 'Référence de géométrie/plan avec altitude, dimensions et fond de plan ; pas ces gros contenus dans chaque ligne.'),
        ('Pièce', 'ID étage ou maison', 'Contour du plan et pont vers la pièce du modèle actuel ; une modification doit conserver la protection physique.'),
        ('Zone fonctionnelle', 'ID maison de portée', 'Membres dans Edge24/CSR ; aucun polygone ou état recopié implicitement.'),
        ('Géozone', 'Portée maison, type distinct', 'Latitude/longitude/rayon dans une ressource typée de présence, indépendante des pièces et zones fonctionnelles.'),
        ('Équipement physique/virtuel', 'ID pièce ou maison', 'Identité native/driver pour le physique ; liaisons/alias/calculs pour le virtuel. Ces objets ne portent aucune observation fictive ; une éventuelle valeur de santé doit être modélisée explicitement.')])
    body += '<p>Component32 est encore un squelette de huit mots u32 : il peut référencer ces ressources mais ne les contient pas. Leur schéma métier, leurs octets et leur politique de vie restent à définir ; leurs contenus variables ne sont pas compris dans le seul coût de 32 octets. Aucune transmutation de ces mots en coordonnées flottantes n’est proposée.</p>'
    sections[-1] = (key, title, body + '</section>')

    names = [s['name'] for s in layouts['structures']]
    options = ''.join(f'<option value="{esc(n)}"'+ (' selected' if n == 'Row96' else '') + f'>{esc(n)}</option>' for n in names)
    all_layouts = ''
    for shape in layouts['structures']:
        offsets = list(shape['offsets'].items())
        all_layouts += f'<details id="layout-{shape["name"]}"'+ (' open' if shape['name'] == 'Row96' else '') + f'><summary>{shape["name"]} · {shape["size"]} octets · alignement {shape["align"]}</summary>'
        all_layouts += table(['Champ', 'Offset en octets', 'Étendue jusqu’au champ suivant / fin'], [(field, offset, (offsets[i+1][1] if i+1 < len(offsets) else shape['size'])-offset) for i, (field, offset) in enumerate(offsets)]) + '</details>'
    field_explanations = '<h3>Les douze champs de Row64</h3>' + table(['Champ', 'Type Rust', 'Rôle'], [
        ('id', 'u64', 'ID local indépendant du nom et de la position physique ; jamais une adresse mémoire.'),
        ('parent', 'u64', 'ID du conteneur/propriétaire canonique ; 0 pour la racine.'),
        ('payload', 'u64', 'Bits de scalaire ou référence de pool selon value_kind ; aucun type déduit du contenu.'),
        ('previous_payload', 'u64', 'Bits ou handle de la dernière valeur distincte avant changement ; présence et type précédent dans flags.'),
        ('received_ms', 'u64', 'Date à laquelle le cœur reçoit le relevé, même si sa valeur reste identique.'),
        ('changed_ms', 'u64', 'Date du dernier changement effectif de type ou de valeur.'),
        ('schema', 'u32', 'Référence de contrat partagé : type, unité, sémantique et représentation.'),
        ('aux', 'u32', 'Référence auxiliaire typée par le contrat du row_kind ; sa convention finale reste à fixer.'),
        ('revision', 'u32', 'Version de la valeur/ligne ; incrément contrôlé, épuisement refusé.'),
        ('flags', 'u16', 'Accès WRITE/ACTIVE_GET, présences HAS_OBSERVATION/HAS_PREVIOUS et type précédent sur quatre bits hauts.'),
        ('row_kind', 'u8', 'Nature de l’objet : maison, équipement, valeur, action ou calcul, etc.'),
        ('value_kind', 'u8', 'Discriminant du payload courant : Null, entier ou composite dans Bank ; HAS_OBSERVATION distingue Null reçu et absence.')])
    field_explanations += '<p>Numeric32 contient quatre mots u64 : min, max, step et anchor. Une numérique peut être encore Null tout en ayant ses contraintes. La proposition fixe le masque de présence ci-dessous ; le Bank met en œuvre WRITE, ACTIVE_GET, HAS_OBSERVATION, HAS_PREVIOUS et le type précédent. La classification numérique vient du schéma, même si la dernière valeur est Null. Ne pas détourner NaN en marqueur d’absence ; un futur masque explicite doit distinguer limite absente et limite zéro, et définir quand anchor est pertinent pour le pas.</p>'
    field_explanations += '<h3>Masque flags proposé pour V6</h3>' + table(['Masque', 'Sens proposé', 'Preuve actuelle'], [
        ('0x0001', 'WRITE : un contrat de set existe.', 'Vérifié par intent ; autorisation finale au Hub.'),
        ('0x0002', 'ACTIVE_GET : contrat explicite de rafraîchissement.', 'Vérifié par intent, indépendant de WRITE.'),
        ('0x0004', 'READ : valeur exposable à la lecture du cache.', 'Proposition ; droits finaux séparés.'),
        ('0x0008', 'AVAILABLE : source actuellement disponible.', 'Proposition ; santé propre de la source.'),
        ('0x0010', 'HAS_OBSERVATION : observation effectivement reçue, même à t=0.', 'Implémenté ; distingue un Null reçu d’une absence d’observation.'),
        ('0x0020', 'ACCESS_UNRESOLVED : découverte à confirmer, aucune écriture accordée.', 'Proposition ; les candidats HA ne suffisent pas.'),
        ('0x0100 / 0x0200', 'MIN_PRESENT / MAX_PRESENT.', 'Proposition : zéro reste une vraie borne.'),
        ('0x0400 / 0x0800', 'STEP_PRESENT / ANCHOR_PRESENT.', 'Proposition : step positif ; ancre pertinente pour la grille.'),
        ('0x0040', 'HAS_PREVIOUS : une précédente valeur distincte existe.', 'Implémenté ; absent à la première observation, conservé sur paquet identique.'),
        ('0x0080', 'Bit réservé, initialisé à zéro.', 'Aucune sémantique inventée lors du décodage.'),
        ('0xF000, décalage 12', 'Type de previous_payload sur quatre bits.', 'Implémenté ; un Null précédent présent reste distinct d’une précédente absente.')])
    field_explanations += '<p>Les flags des anciens bancs V2 et ceux de cette proposition ont des espaces distincts : ne pas recopier leurs bits sans adaptation. Les schémas doivent refuser min supérieur à max, pas nul/négatif, dénominateur zéro et valeurs non finies ; ces validations de schéma restent à raccorder, elles ne sont pas prouvées par les seuls tests de layout.</p>'
    sec('layouts', 'Voir chaque octet, retrouver chaque champ.', f'''<p>Les 18 layouts sont produits par <code>size_of</code>, <code>align_of</code> et <code>offset_of!</code> du binaire Rust linux/aarch64. Aucun <code>packed</code>, pointeur brut ou transmutation n’est nécessaire. Une taille de 96 octets et un alignement de 8 ne prouvent ni une ligne de cache matérielle de 96 octets ni un alignement matériel optimal. Les étendues incluent un éventuel padding ; ce ne sont pas des tailles de type déduites d’un nom.</p><label for="layout-choice">Structure à explorer</label><select id="layout-choice">{options}</select><div id="layout-map" class="diagram" aria-live="polite"></div><div id="layout-fields"></div><p>Row96 = Row64 [0..64[ + Numeric32 [64..96[. Les deux payloads et les deux dates sont dans Row64 ; Hot32 contient ces quatre mots de 8 octets. Alias32 garde alias_id, owner_id, source_id, source_slot et source_generation. La métadonnée 32 et le slot 16 s’ajoutent aux deux variantes : 96 + 16 + 32 = 144 ; 32 + 16 + 32 = 80, avant le reste.</p><p>{link('tools/memoire-structure-v6/layouts.json', 'JSON de tous les offsets')} · {link('tools/memoire-structure-v6/layouts.rs', 'Déclarations Rust exactes')}</p>''' + observation_band() + field_explanations + all_layouts)

    selected_nodes = [node for node in demo['nodes'] if node['id'] in (100, 101, 200, 201, 202)]
    sec('identites', 'De l’ID durable à la ligne physique.', '''<pre>Identité publique proposée = installation UUID + ID local u64 durable
  → HashMap&lt;u64, Handle8&gt; → (slot global u32, génération u32)
  → GlobalSlot16 : arène, indice physique, référence Meta32
  → Row96 / Alias32 → source canonique vérifiée</pre><p>Le prototype reçoit des IDs synthétiques de l’appelant ; son allocateur fourni ne garantit pas la non-réutilisation durable. L’allocateur monotone/non réutilisable et la persistance de l’UUID d’installation restent à développer. Nom, pièce, topic, adresse et ordre dans un Vec ne constituent jamais l’identité. Un renommage change Meta32 ; une compaction change l’indice physique dans le slot global. Aucun pointeur de Vec ne survit à une réallocation.</p><p>Les IDs u64 restent internes ; sur une API JSON, les transmettre comme chaînes opaques avec leur espace d’installation pour préserver les valeurs au-delà de 2⁵³ dans les clients JavaScript. Les petits nombres du schéma sont des IDs de démonstration, pas un nouveau format public livré.</p><p>Lecture de 200 ou 201 : vérifier epoch du lecteur, génération du slot de l’alias, puis génération et ID de la source ; tous deux lisent 100. L’alias 201 a été créé depuis 200 et aplati immédiatement. 202 lit le calcul 101, qui possède son état 42 indépendamment de la source 100 à 21. Boucles, mauvaise source et propriétaire non virtuel sont refusés.</p>''' + alias_diagram(demo) + table(['ID demandé', 'Slot', 'Génération', 'Indice physique après compaction', 'ID résolu', 'Valeur'], [(n['id'], n['slot'], n['generation'], n['physical_index'], n['resolved_id'], n['payload']) for n in selected_nodes]) + '''<p>Retrait et réemploi incrémentent la génération ; l’épuisement est refusé, sans rebouclage. Un ancien cache d’alias devient invalide. L’epoch sépare les incarnations du catalogue et est vérifiée à la lecture ; elle ne remplace pas une génération. Bank initialise epoch à 1 : cela ne prouve aucune sûreté après redémarrage. Il faut encore une boot-epoch durable ou un nonce d’incarnation associé à l’UUID d’installation. Dans Bank, les mutations exigent <code>&amp;mut Bank</code> et le CSR emprunte Bank immuablement. Un <code>RwLock</code> ou des snapshots versionnés seraient un choix d’intégration à éprouver ; aucune concurrence multi-lecteur du futur moteur n’est prouvée ici.</p><p>Écrire par 200 renvoie une intention vers propriétaire 50 / point 100. L’observation reste 21 jusqu’au retour du pilote. Le vrai Hub doit encore appliquer type, lecture seule, garde, journal, délai et confirmation : une vue virtuelle ne déplace jamais la protection physique.</p><h3>Créer, renommer, déplacer, retirer, recharger</h3>''' + table(['Opération', 'Transition et garanties à conserver'], [
        ('Créer', 'Allouer un ID durable avant son premier enregistrement ; créer ligne, slot et méta, puis publier les index et liens cohérents.'),
        ('Renommer', 'Modifier le label référencé dans Meta32 ; conserver ID, slot, propriétaire et source des alias.'),
        ('Déplacer', 'Vérifier la nouvelle topologie et toutes les appartenances de zones, puis changer parent et reconstruire les index concernés. Conserver les protections physiques.'),
        ('Supprimer', 'Aucune cascade implicite : Bank refuse un propriétaire avec enfants. Les alias restants deviennent invalides si leur source disparaît ; un futur calcul doit passer explicitement à dégradé/inconnu, pas zéro.'),
        ('Recharger', 'Vérifier l’incarnation, reconstruire slots, liens et caches depuis les IDs persistés ; les anciens handles/générations/epochs ne doivent pas être acceptés.'),
        ('Persister', 'Écrire un format explicite, versionné et contrôlé, avec IDs, types et longueurs. Ne jamais sérialiser la RAM repr(C), ses HashMap ou pointeurs comme un fichier portable.')]) + '''<p>Dans l’intégration proposée, le service vérifie droits/garde avant l’accès mutable, prend le verrou d’écriture puis obtient <code>&amp;mut Bank</code> ; il revérifie les versions pertinentes sous verrou. Une approbation humaine ne garde pas ce verrou pendant l’attente. Bank seul ne fournit ni ce verrou, ni la garde, ni la transaction durable.</p>''')

    sec('types', 'Unités, valeurs et plans ont des rôles différents.', '''<p>Le type distingue Null, booléen, entier signé, float, texte et composite. Le payload u64 stocke des bits ou une référence selon <code>value_kind</code> ; « zéro » n’est pas « absent ». Bank teste Null/Integer/Composite ; les autres lignes du tableau décrivent le contrat cible, pas des codecs déjà livrés. Les chiffres de température et de pourcentage ne forcent pas tous les entiers à devenir des f64. Une somme conserve les unités compatibles et une politique explicite d’overflow ; un calcul peut nécessiter un résultat float.</p>''' + table(['Type cible', 'Contenu des 8 octets du payload'], [
        ('Null', 'Absence donnée par le discriminant ; aucun NaN sentinelle, aucun zéro interprété comme absent.'),
        ('Bool', 'Une information de 1 bit dans un mot de 8 octets ; valeurs valides à restreindre à 0 ou 1.'),
        ('Int', '64 bits de complément à deux, interprétés comme entier signé selon le type.'),
        ('Float', '64 bits IEEE-754 réinterprétés explicitement en f64 ; proposition : rejeter NaN et les infinis en entrée, sans détourner NaN en limite absente.'),
        ('Text', 'Référence de pool typée ; longueur et génération contrôlées avant lecture UTF-8.'),
        ('Octets binaires futurs', 'Référence de pool vers des octets arbitraires, avec taille/type/contrat explicites.'),
        ('Composite', 'Handle encodé slot u32 + génération u32 ; accès au blob entier versionné.')]) + '''<p>Le <code>Kind::Binary</code> actuel désigne un booléen de l’API métier ; il ne signifie pas un tableau d’octets. Un futur Value d’octets binaires est une extension distincte, dont le pool peut illustrer le rangement sans la livrer au produit.</p>''' + table(['Donnée', 'Emplacement et rôle'], [
        ('Numeric32 : min, max, step, anchor', 'Contraintes canoniques propres au point ; représentation des bits donnée par le schéma. Le pas s’applique à partir de l’ancre.'),
        ('Schema64 : unit, semantic, numeric_repr, decimals, prefix_exp10, value_kind', 'Contrat partagé : unité, sens, représentation, précision et préfixe. Décimales d’affichage ≠ préfixe physique ×10ⁿ.'),
        ('Schema64 : native_min/max, scale_num/den', 'Bornes natives et échelle rationnelle ; les limites max_bytes/max_items bornent les données variables. Dénominateur nul à refuser lors d’une future compilation de schéma.'),
        ('Transform64', 'Plan déterministe avec échelle/décalage, bornes d’entrée/sortie, arrondi et overflow ; versions distinctes des plans de lecture et d’écriture.'),
        ('Meta32, Route32 : read_plan / write_plan', 'Liens vers le bon codec et contrat, propriétaire et clé native. Un flag ne prouve ni permission ni validation physique.')]) + '''<pre>Lecture : natif 127 / domaine 0..254 → Row Float canonique 50.0 %
Écriture : Row Float 50.0 % × 254 / 100 → 127 natif, via plan write
Réponse API : statut de commande / code du pilote ≠ valeur observée confirmée</pre><p>Le préfixe k signifie ×10³, M ×10⁶ ; Ki signifie ×2¹⁰ et Mi ×2²⁰. prefix_exp10 encode seulement des puissances décimales, pas les préfixes binaires ; ceux-ci demandent une unité ou une échelle explicite.</p><p>Une plage 0..100 avec pas 1 et ancre 0 ne décrit pas à elle seule une inversion, un gamma ou le sens de 100 %. Garder les deux domaines et les deux plans. Ce code de conversion n’est pas raccordé au Bank : les layouts en matérialisent le coût. Lire le cache, demander un get actif et faire un set sont trois opérations distinctes. Le modèle fixe WRITE = 1 et ACTIVE_GET = 2, contrôlés séparément ; lire le cache ne déclenche jamais un get actif. Une valeur en lecture seule peut autoriser le get actif sans autoriser le set. Les présences d’observation et de précédente ainsi que le type précédent sont aussi implémentés ; READ, AVAILABLE, ACCESS_UNRESOLVED et les présences numériques restent des propositions ci-dessus. Les droits finaux et la garde sont toujours ceux du Hub, pas une déduction de ces deux bits.</p>''')

    sec('variables', 'Le JSON entre et sort ; la RAM reste typée.', '''<p>JSON est une frontière d’API ou de transport. Décoder un message MQTT/JSON borné, retrouver clé native et plan compilé, puis mettre à jour une ligne ne signifie pas conserver un objet JSON par point en RAM. Les actions suivent le sens inverse avec validation et codec d’écriture. Les allocations temporaires du parseur se mesurent à côté de l’état conservé.</p><p>Un composite peut rester atomique dans un pool ou exposer des enfants typés si le contrat en a besoin. Les deux inventaires HA « détail » servent une expansion de forme ; ce n’est pas une obligation de transformer chaque attribut JSON en point permanent.</p><pre>Format composite du prototype, little endian explicite :
version:u32 | count:u16 | réservé:u16 = 0
puis count × (clé:u32 | bits de valeur:u64)
8 + 12 × count octets ; count ≤ 64 ; clés uniques</pre><p>Ce format wire little endian est explicite ; il ne découle pas de repr(C) et ne sert pas de format persistant du catalogue. Il prouve le parsing borné, pas toutes les formes futures. PoolEntry16 porte offset, longueur, génération et type ; les octets peuvent aussi contenir du binaire sans encodage JSON. Bornes : 4 Kio par entrée, 1 Mio d’octets et 65 536 descripteurs, y compris les valeurs vides. Une référence hors bornes, mauvaise génération, mauvais type, longueur malformée ou version concurrente est refusée.</p><p>La nouvelle valeur composite est validée entièrement avant publication du descripteur et de la version sous <code>&amp;mut Bank</code>. Le pool append-only conserve les anciennes versions ; collecte, compaction, grosses images et persistance ne sont pas implémentées. Les limites du produit devront dépendre du contrat et de son budget.</p>''' + table(['Date conservée', 'Sens', 'Paquet identique'], [('received_ms', 'Dernière réception locale par le cœur, sans prétendre dater la mesure physique.', 'Mis à jour à chaque réception acceptée.'), ('changed_ms', 'Date du dernier changement effectif de type ou de valeur, fournie à observe par l’appelant.', 'Conserve la date du dernier changement.')]) + '''<p>La date source est un argument transitoire d’observation ; sa dernière occurrence n’est plus conservée dans la ligne. Le choix d’une date en son absence reste un contrat explicite à définir par adaptateur. Un relevé identique renouvelle received_ms et peut renouveler la fraîcheur sans déplacer changed_ms ni écraser previous_payload. Les échéances demandent une horloge monotone distincte. Les dates source et qualités utiles aux fenêtres demandent une ressource temporelle externe proposée, avec son propre budget ; les deux dates du catalogue ne suffisent pas à reconstruire un historique.</p><h3>Une seule précédente valeur, à chaque changement</h3>''' + table(['Valeur reçue', 'Courante', 'Précédente distincte', 'received_ms', 'changed_ms'], [('20 à t=10', '20', 'Absente', '10', '10'), ('21 à t=20', '21', '20', '20', '20'), ('21 à t=30', '21', '20, conservée', '30', '20'), ('22 à t=40', '22', '21', '40', '40')]) + '''<p>Dans cet exemple 20 → 21 → 21 → 22, les dates source et de réception coïncident par convention illustrative. Un changement de type compte aussi, y compris vers ou depuis Null. Les scalaires précédents gardent leurs bits ; texte, binaire et composite gardent le handle du pool. Le pool append-only conserve les blobs nécessaires mais ne sait pas encore les récupérer. La ligne reste à 96 octets ; conserver les contenus variables précédents peut augmenter les octets du pool et doit être budgété séparément. Cette précédente valeur ne porte pas sa propre date et ne fournit aucun historique complet.</p>''')

    sec('index', 'Des parcours directs, au prix de vrais index.', '''<p>Le parent est déjà dans Row64. Le prototype le duplique volontairement en Edge24 pour unifier les parcours avec les zones et simplifier les preuves : c’est une redondance conservatrice, chiffrée. Une optimisation dérivant les enfants directement des parents et séparant les arêtes de zones reste <b>non testée</b>.</p><pre>Edge24 durable en RAM : from ID, to ID, relation, flags, version
  + CSR avant : offsets u32[S+1] + Target12[E]
  + CSR inverse : offsets u32[S+1] + indices u32[E]
  = 24E + 12E + 4E + 8(S+1) = 40E + 8(S+1) octets</pre><p>E comprend parents, appartenances et dépendances de calcul dans le budget ; le Bank minimal ne matérialise que parents et zones. « Durable » désigne ici la source stable des caches en RAM, sans journal disque. Les IDs survivent au réordonnancement ; les CSR se reconstruisent aux changements de topologie.</p><p>Un index source → alias évite de chercher tous les alias lors d’une notification. Il est budgété séparément mais <b>non implémenté dans Bank</b>. Lire un alias n’exige pas de recopier l’état ; notifier ses abonnés a malgré tout un coût. Les index typés, tables de fraîcheur, Calc48 et Route32 sont budgétés et leurs layouts testés : aucun ordonnanceur ni moteur de routage complet V6 n’en découle automatiquement.</p><h3>Quatre accès, pas à pas</h3>''' + table(['Besoin', 'Chemin et travail réellement nécessaire'], [
        ('Valeur physique', 'ID → index → Handle8 → contrôle génération → GlobalSlot16 → Row96 → type/payload. Pour libellé ou unité : Meta32 puis schéma/pool référencé.'),
        ('Alias', 'ID de la vue → Alias32 → contrôle epoch/source ID/génération → slot canonique → même Row96. La commande conserve le propriétaire physique et passe par le Hub.'),
        ('Zone', 'ID de zone → slot → tranche du CSR avant → membres ; développer les pièces via enfants, sélectionner les types et lire les canoniques. Si une pièce et son appareil sont tous deux membres, dédupliquer les IDs avant somme/moyenne/commande. Cette expansion/déduplication métier reste à développer.'),
        ('Notification et calcul', 'Observation canonique → index source→alias et dépendants → notifications/dirty set → calcul ordonné et vérifié → résultat propre, version et fraîcheur. Ces index et l’ordonnanceur V6 sont budgétés, pas implémentés par le seul layout Calc48.')]) + '''<p>Reconstruire un CSR à partir de N slots et E liens coûte O(N + E), avec recherches hash attendues constantes ; recharger l’index ID et ses caches demande aussi de parcourir les objets. Une permutation ou un rechargement doit publier un ensemble cohérent. Pendant une reconstruction sans blocage prolongé, l’ancien index, le nouveau et les buffers temporaires peuvent coexister : leur pic mémoire s’ajoute au budget permanent affiché.</p>''')

    temporal_path = ROOT / 'tools/fenetres-temporelles-banc/sorties/campagnes/20261025T140712Z/resultats.json'
    temporal = json.loads(temporal_path.read_text()) if temporal_path.exists() else []
    sec('temps', 'Quatre fenêtres partagent un historique.', '''<p>Moyenne entre appareils, moyenne des relevés, moyenne des changements enregistrés et moyenne pondérée par durée sont quatre contrats différents. Null/périmé signifie couverture inconnue ; zéro contribue réellement. Une journée civile peut durer 23 ou 25 heures ; 24 h glissantes restent 24 heures écoulées.</p><p>Pour 1 min, 5 min, 1 h et 24 h : un historique partagé par source, quatre résultats canoniques. Une ligne ne garde qu’une précédente valeur distincte ; elle ne remplace pas cet historique temporel. Les seules lignes Row96 coûtent 4 × 96 = <b>384 octets</b>, auxquels s’ajoutent slots, métadonnées, définitions, index et surtout historique. Ne pas réserver quatre historiques identiques, ni un gros anneau sur chaque point.</p>''' + table(['V5, charge sparse', 'Buffer par source', '100 sources, Mio calculés', '1 000 sources, Mio calculés'], [(r['backend'], fmt(r['heap_capacity_bytes']) + ' octets', mib(r['heap_capacity_bytes']*100), mib(r['heap_capacity_bytes']*1000)) for r in temporal if r['workload']=='sparse']) + '''<p>Ce tableau reprend les capacités mesurées des prototypes V5 ; ses multiplications sont des extrapolations distinctes du catalogue V6. Anneau tronqué : limited=true et couverture visibles. Seaux : erreur de bord possible, sans garantie générale tirée d’un seul jeu de données. Fenêtres actives, ordonnanceur et stockage partagé restent à intégrer et mesurer ensemble.</p>''')

    sec('options', 'Trois options, un choix explicite.', table(['Option', 'Structure des canoniques', 'Avantages', 'Coûts et limites', 'Quand choisir'], [
        ('A · Row96', '96N ; numérique inclus pour chaque ligne.', 'Lecture simple, limites proches de la valeur, invariant unique, préférence conservée.', '32 octets numériques inutiles pour les lignes non numériques ; méta et graphes restent à côté.', 'Référence V6 recommandée pour avancer et mesurer le vrai moteur.'),
        ('B · Row64 + annexe', '64N + 32M + 4N références numériques.', 'Réserve les contraintes numériques aux M lignes concernées, y compris une numérique encore Null.', 'Indirection supplémentaire et cohérence de deux arènes. Le Vec<u32> par canonique est obligatoire dans ce modèle.', 'Si le budget RAM réel justifie la complexité.'),
        ('C · Hot32 + Cold32', '32N + 32N + 32M + 4N références.', 'Scans des seuls payloads et dates sur des buffers plus denses.', 'Même volume utile que B ; accès complet à plusieurs buffers et mutations plus complexes.', 'Après profilage montrant des scans chauds dominants ; aucun gain CPU garanti.')]) + '''<p>N désigne toutes les lignes canoniques, M les contrats numériques. Les alias restent à part. B économise sur A lorsque M &lt; 0,875N dans ces seules arènes ; la somme globale dépend aussi des index, pools et chaînes. Les alternatives matérialisées reconstruisent chaque champ en tests ; leur temps d’exécution V6 n’a pas été mesuré. Les anciens résultats V2 peuvent orienter une expérience, pas prédire toutes les charges.</p>''')

    datasets = data['datasets']
    sec('donnees', 'Deux inventaires de formes, leur somme et leurs détails.', table(['Charge', 'Appareils', 'Valeurs', 'Contrats/actions de la fixture', 'Lignes de départ', 'Numériques', 'Pool synthétique, octets'], [(d['label'], d['devices'], d['values'], d['actions'], d['rows'], d['numeric_rows'], fmt(d['payload_pool_bytes'])) for d in datasets]) + '''<p>A principal : 206 appareils et 986 entités ; B principal : 290 et 1 278 ; somme : 496 et 2 264. Les charges détail déplient des attributs selon leur forme et ajoutent des composites. somme-detail-liens4 ajoute des liens synthétiques. Contrats partagés et bindings cibles sont distingués dans les budgets V6 ; les anciennes lignes « actions » ne deviennent pas toutes des copies de contrat.</p><p>Valeurs, bornes, noms, identifiants et dates ont été remplacés. Min/max/pas 0..100/1 sont synthétiques ; les accès restent non résolus. 147 entités de la somme n’ont pas de parent appareil direct et 13 actives ne figurent pas dans le registre ; les conventions de fixture ne prouvent aucune permission. Les alias, virtuels, zones, textes et plans de V6 viennent d’hypothèses explicites, pas d’une découverte HA.</p>''')

    catalogue = [r for r in measurements['catalogue'] if r['dataset']=='somme-principal' and r['scale']==1]
    alias = [r for r in measurements['aliases'] if r['dataset']=='somme-detail' and r['scale']==1 and r['fanout']==1]
    organisation = [r for r in measurements['organisation'] if r['dataset']=='somme-principal' and r['scale']==1]
    measured_body = '<div class="notice"><b>Mesures historiques conservées.</b> Chaque graphe porte sa propre métrique et son périmètre. Les campagnes V2–V5 sont figées : elles ne mesuraient pas la conservation de la précédente valeur et ne sont pas comparables sur cette fonction nouvelle. Aucun ancien total n’est additionné aux nouvelles zones V6.</div>'
    measured_body += bars('V2 · somme principal ×1 · allocations demandées par le microbanc', 'Mio', [(r['layout'],r['allocations_requested_bytes']/1048576) for r in catalogue])
    measured_body += bars('V2 · même charge · scan séquentiel par ligne', 'ns', [(r['layout'],r['scan_ns_per_row']) for r in catalogue])
    measured_body += bars('V2 · même charge · recherche ID', 'ns', [(r['layout'],r['lookup_ns_per_op']) for r in catalogue])
    large_v2 = [r for r in measurements['catalogue'] if r['dataset']=='somme-detail' and r['scale']==100]
    measured_body += '<h3>Grand cas réellement mesuré : V2 somme détail ×100</h3><p>1 224 300 lignes de fixture, dans trois layouts du microbanc V2. Ce graphe est une mesure conservée ; il ne qualifie pas les nouveaux slots, métadonnées et quotas V6. Le ×1000 du budget plus bas reste calculé.</p>'
    measured_body += bars('V2 · somme détail ×100 · 1 224 300 lignes · allocations demandées', 'Mio', [(r['layout'],r['allocations_requested_bytes']/1048576) for r in large_v2])
    measured_body += table(['Layout V2 mesuré','Lignes','Allocations demandées, Mio','Scan, ns/ligne','Lookup, ns/op'],[(r['layout'],fmt(r['n']),mib(r['allocations_requested_bytes']),f"{r['scan_ns_per_row']:.3f}",f"{r['lookup_ns_per_op']:.3f}") for r in large_v2])
    measured_body += bars('V3 · somme détail ×1 · allocations demandées du microbanc alias', 'Mio', [(r['variant'],r['allocations_requested_bytes']/1048576) for r in alias])
    measured_body += bars('V3 · somme détail ×1 · lecture alias scalaire', 'ns', [(r['variant'],r['scalar_ns_per_op']) for r in alias if r['scalar_timing_available']])
    measured_body += bars('V3 · même charge · source mise à jour et notification', 'ns', [(r['variant'],r['source_update_notify_ns_per_op']) for r in alias])
    measured_body += bars('V4 · somme principal ×1 · parcours avant par arête visitée', 'ns', [(r['variant'],r['forward_median_ns']/r['forward_visited_edges']) for r in organisation])
    engine = [r for r in measurements['engine'] if r['source_devices']==496 and r['measurement_mode']=='counted']
    measured_body += table(['V5 · 496 appareils', 'Tas demandé final, Mio', 'Pages anonymes, Mio', 'Pic RSS, Mio'], [(r['layout'],mib(r['final_live_requested_bytes']['median']),mib(r['steady_anonymous_bytes']['median']),mib(r['process_peak_rss_bytes']['median'])) for r in engine])
    measured_rows = []
    for dataset in datasets:
        current = {r['layout']: r for r in measurements['catalogue'] if r['dataset']==dataset['label'] and r['scale']==1}
        if all(k in current for k in ('uniform96','uniform64','split')):
            measured_rows.append([dataset['label'],*[mib(current[k]['allocations_requested_bytes']) for k in ('uniform96','uniform64','split')],f"{current['uniform96']['lookup_ns_per_op']:.3f}"])
    measured_body += '<h3>Mesures V2 sur A, B et leur somme, ×1</h3>' + table(['Charge de formes','Row96 : demandé, Mio','Row64 : demandé, Mio','Split : demandé, Mio','Row96 : lookup ID, ns'], measured_rows)
    measured_body += '''<p>V2 : médianes de trois processus, sept rondes internes, un CPU, 512 Mio ; ×10 et ×100 existent aussi dans les données conservées. V3 compare notamment miroir, alias en ligne, lookup ID et handle. La variante pointer est un témoin du microbanc, pas la direction de sûreté retenue. V4 mesure une organisation synthétique de ces inventaires. V5 garde le vrai Hub et ajoute trois appareils calculés, quatre points synthétiques par appareil source : les 1 590 tests réussis appartiennent à l’ancienne campagne V5, pas à une nouvelle suite moteur V6.</p><p>Les p95, débit et limites moteur/transports sont détaillés dans V5. Les intervalles se recouvrent et les parcours diffèrent : aucun classement CPU universel de A/B/C n’est démontré. Les allocations demandées, pages anonymes et RSS sont trois mesures distinctes.</p>'''
    sec('mesures', 'Les graphes de mesures restent séparés du calcul.', measured_body)

    assumptions = data['assumptions']
    budget_options = ''.join(f'<option value="{esc(d["label"])}"'+ (' selected' if d['label']=='somme-principal' else '') + f'>{esc(d["label"])}</option>' for d in datasets)
    sec('budgets', 'Le budget de base se décompose, les contenus précédents s’ajoutent.', '''<div class="notice"><b>CALCULÉ — budget de base, hors contenu variable précédent ; catalogue hypothétique, pas RSS ni moteur complet.</b> Les allocations de l’index sont sondées séparément ; les autres zones sont des multiplications de layouts et d’hypothèses. Les réservations libres des Vec, en-têtes, fragmentation, processus, files, snapshots, historique et exécution temporaire ne sont pas tous couverts.</div>''' + f'''<div class="controls"><div><label for="dataset-choice">Inventaire de formes</label><select id="dataset-choice">{budget_options}</select></div><div><label for="scale-choice">Multiplication synthétique</label><select id="scale-choice"><option>1</option><option>10</option><option>100</option><option>1000</option></select></div><div><label for="variant-choice">Stockage canonique</label><select id="variant-choice"><option value="row96">A · Row96</option><option value="row64">B · Row64 + annexe</option><option value="split">C · Hot32 + Cold32</option></select></div></div><div id="budget-result" aria-live="polite"></div><div id="budget-chart" class="diagram"></div><div id="budget-zones"></div><h3>Extrapolations du même scénario</h3><div id="extrapolation"></div><h3>Hypothèses communes</h3>''' + table(['Paramètre', 'Valeur explicite'], assumptions.items()) + '<h3>Limites du dimensionnement</h3><ul>' + ''.join('<li>' + esc(limit) + '</li>' for limit in data['limits']) + '</ul>' + '''<p>Par monde synthétique : 39 nœuds d’organisation et 32 calculs. Les alias référencent les valeurs principales, fanout 1 ; un virtuel pour quatre valeurs principales (arrondi par monde). Les bindings sont comptés à part des contrats ; la cible est hypothétique. Les 96 schémas et 16 transformations sont partagés globalement, donc ne sont pas multipliés par l’échelle.</p><p>Les textes supposent 24 octets par label, 16 par clé et 32 par identifiant natif d’appareil, plus des répertoires de 8 octets ; le pool synthétique existant s’ajoute. Le probe HashMap mesure les octets réservés demandés à l’allocateur pour <code>HashMap&lt;u64, Handle8&gt;</code>, avec sa capacité réelle : ce n’est pas son RSS. Ne pas le confondre avec le seul payload 16 octets par paire dans demo.json.</p><p>La réserve d’exécution ajoute quatre buffers de parsing de 64 Kio et 1 280 enveloppes de 64 octets pour événements/commandes ; ce sont des hypothèses, pas les limites actuelles de production. Les en-têtes de conteneurs sont aussi comptés. Chaque total additionne seulement ses zones disjointes. Les trois variantes partagent les mêmes hypothèses ; un écart ne vaut pas un gain produit mesuré. L’échelle ×1000 est une extrapolation, même lorsqu’un microbanc ancien a réellement mesuré ×100. Les budgets de pool ci-dessus conservent leur hypothèse de contenu : ils ne mesurent pas le surcoût de rétention des contenus variables précédents et n’en font pas un coût nul. Le pool théorique des charges somme dépasse dès ×10 la borne de 1 Mio du Bank de démonstration ; les 32 calculs par monde deviennent 320 à ×10 et 3 200 à ×100, au-delà de la limite livrée de 128 calculs. Ces scénarios sont du calcul de dimensionnement, aucune exécution validée sous ces quotas. Changer quotas, pools, chiffrement/persistance et configurations futures nécessitera ses propres contrats et mesures. Le budget produit de 32 Mio anonymes concerne toutes les briques et devra être vérifié sur un moteur intégré.</p>''')

    coverage = [
        ('Stockage 96 octets et alternatives', '18 layouts ; aller-retour des alternatives ; tableau de choix.', 'Modèle testé, moteur non migré.'),
        ('Familles, maisons, étages, pièces, zones', 'Validation topologique, chevauchement, géozone distincte ; CSR.', 'Topologie synthétique, droits hors modèle.'),
        ('Alias sans copie et calculs propres', '200/201 → 100 ; 202 → 101 ; stale génération/epoch et compaction.', 'Notifications et scheduler V6 non implémentés.'),
        ('Actions, get, set et effets physiques', 'Flags séparés, binding cible et intention vers le propriétaire.', 'Garde du Hub à conserver ; aucun ordre physique dans ce travail.'),
        ('Unités, précision, échelle et conversions', 'Schema64, Numeric32, Transform64 ; exemple 127 ↔ 50 %.','Layouts et contrat proposé ; conversions réelles étudiées en V5.'),
        ('Composites, binaire et timestamps', 'Pool borné, parser little endian, publication versionnée, précédente typée et deux dates.', 'Format prototype ; collecte/persistance à développer.'),
        ('Moyennes 1 min / 5 min / 1 h / 24 h', 'Historique partagé, couverture, quatre résultats et budget séparé.', 'Prototypes V5 distincts du futur catalogue.'),
        ('HA A/B/somme et extrapolation', 'Mesures V2–V5 conservées ; budgets ×1/10/100/1000 explicités.', 'Formes anonymisées et hypothèses nouvelles, pas données brutes.')]
    proof_links = [
        ('tools/memoire-structure-v6/README.md','Périmètre et reproduction V6'),('tools/memoire-structure-v6/sorties/budgets.json','Budgets et mesures importées JSON'),('tools/memoire-structure-v6/demo.json','IDs, résolutions et capacités des Vec'),('tools/memoire-structure-v6/tests.rs','Tests structurels Rust'),('tools/catalogue-banc/sorties/campagnes/20261025T104218Z-d66e645aa2ad/resultats.json','Mesures V2'),('tools/alias-banc/sorties/campagnes/20261025T113014Z-602c1a8b466f/resultats.json','Mesures V3 alias'),('tools/organisation-banc/sorties/campagnes/20261025T125921Z-d031adc6cd1b/resultats.json','Mesures V4 organisation'),('tools/moteur-suite-banc/sorties/campagnes-v5.json','Index des campagnes V5'),('tools/catalogue-banc/charges/manifest.json','Manifest des formes HA anonymisées')]
    sec('preuves', 'Chaque besoin a sa preuve et sa limite.', '''<p>V4 a relu l’export fourni : cinq conversations, 43 messages balisés dont 23 utilisateur. Une sixième conversation est exclue par l’archive et le début de C04 manque. Cette couverture partielle est conservée ; V6 ne prétend pas avoir relu des échanges privés absents. La présente session complète le besoin de structures détaillées, conversions, commandes, fenêtres et coûts complets.</p>''' + table(['Besoin retenu', 'Où le retrouver', 'Niveau de preuve'],coverage) + '<p><a href="catalogue-memoire-v4.html">Analyse V4 et sources des intentions</a> · <a href="orientations-v4/corpus.json">Couverture de l’export</a> · <a href="catalogue-memoire-v5.html">V5 : vrai moteur et fenêtres</a> · <a href="catalogue-memoire-v2.html">V2 : catalogue</a> · <a href="catalogue-memoire-v3.html">V3 : alias</a></p><ul>'+''.join('<li>'+link(path,label)+'</li>' for path,label in proof_links)+'</ul><p>Les fichiers publiés n’incluent ni conversations brutes, ni données HA privées, ni secrets. Aucune migration de production, aucun déploiement et aucune commande physique ne sont réalisés. La suite complète du dépôt exécutée en V5 reste une preuve de cette campagne ; V6 revendique ' + esc(test_evidence) + ', ses layouts et ses calculs, sans rebaptiser cette ancienne exécution en validation d’un moteur V6.</p>')

    css = re.search(r'<style>(.*?)</style>', (OUT.parent/'catalogue-memoire-v4.html').read_text(),re.S).group(1)
    css += '''
th{text-transform:none;letter-spacing:0;font-size:12px}td{min-width:90px}select{font:inherit;background:#fffdf6;color:#193e37;border:1px solid #738a78;border-radius:4px;padding:10px;max-width:100%;margin:8px 0 18px}label{font-size:13px;font-weight:600}button:focus-visible,select:focus-visible,summary:focus-visible{outline:3px solid #ad682d;outline-offset:3px}.controls{display:flex;gap:24px;flex-wrap:wrap}.controls>div{display:flex;flex-direction:column}figure{margin:24px 0;background:#fcfcf7;border:1px solid #d7dfd0;padding:16px;border-radius:5px}.chart{width:100%;height:auto;display:block;min-width:580px}.chart text,.diagram svg text{font-family:"Avenir Next",Verdana,sans-serif;fill:#213f34;font-size:13px}.chart .axis{font-size:11px;fill:#657768}figcaption{font-size:12px;color:#5e7166;margin-top:10px}.diagram{overflow:auto;background:#fcfcf7;padding:15px;border:1px solid #d5decd;border-radius:5px;margin:20px 0}.diagram svg{display:block;width:100%;min-width:620px;height:auto}figure{overflow:auto}details{margin:14px 0;border:1px solid #c6d3bf;background:#fbfcf5;border-radius:4px}summary{padding:13px 16px;cursor:pointer;font-weight:600}details .scroll{margin:0}.budget-big{font:42px Georgia,serif;color:#1b5945}.muted{font-size:13px;color:#576f62}.skip{position:absolute;left:-9999px}.skip:focus{left:10px;top:10px;background:white;padding:12px;z-index:20}nav a{font-size:12px}.legend{display:flex;gap:10px;flex-wrap:wrap;font-size:12px}.legend span{padding:5px 8px;background:#e6ecdf}.layout-note{margin:0;color:#5a7164;font-size:12px}@media(max-width:600px){main{padding:25px 16px}h2{font-size:30px}.controls{display:block}select{width:100%}.lead{font-size:19px}th,td{padding:10px}figure{padding:8px}}@media(prefers-reduced-motion:reduce){html{scroll-behavior:auto}}@media print{details{display:block}details>*{display:block!important}.controls{display:none}.diagram svg,.chart{min-width:0}.chart{max-height:220px}}
'''
    payload = json.dumps({'budget':data,'layouts':layouts},ensure_ascii=False,separators=(',',':')).replace('</','<\\/')
    css += pedagogie.CSS + '''
aside details{margin-top:24px;background:transparent;border-color:#5e7f6b}aside summary{font-size:12px;color:#f4f5e9;padding:12px 8px}aside details nav{padding-bottom:12px}aside .p-nav-lesson a{font-size:13px}.p-hero-links{display:flex;gap:12px;flex-wrap:wrap;margin-top:26px}.p-hero-links a{display:inline-block;padding:12px 16px;border:1px solid #52765b;border-radius:4px;color:#1d573e;font-weight:600;font-size:14px}.p-hero-links a:first-child{background:#215840;color:white}
'''
    js = SCRIPT.replace('__DATA__',payload) + '\n' + pedagogie.script() + '\n' + alignement.script()
    nav = ''.join(f'<a href="#{key}">{esc(title)}</a>' for key,title,_ in sections if key.startswith('p-'))
    technical_nav = ''.join(f'<a href="#{key}">{esc(title)}</a>' for key,title,_ in sections if not key.startswith('p-'))
    page = '<!doctype html><html lang="fr"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Moli — V6 : comprendre les mouvements en mémoire</title><style>'+css+'</style></head><body><a class="skip" href="#p-maison">Aller au contenu</a><aside><b>Moli / V6</b><p>Comprendre avec un exemple<br>Puis vérifier les détails</p><nav class="p-nav-lesson" aria-label="Explications visuelles">'+nav+'</nav><details><summary>Référence technique et tests</summary><nav aria-label="Référence technique">'+technical_nav+'</nav></details></aside><main><header><span class="badge">PROPOSITION · EXEMPLES FICTIFS · SIMULATION LOCALE</span><h1>Une valeur réelle.<br>Plusieurs vues, sans copie.</h1><p class="lead">Une petite maison, des fiches et des raccourcis : suivez une mesure, un ordre ou une action, et voyez ce qui est lu, écrit, calculé et envoyé à chaque étape.</p><div class="p-hero-links"><a href="#p-parcours">Suivre une opération →</a><a href="#p-rangements">Voir les tailles en mémoire</a><a href="#optimisations">Voir les opérations mesurées</a><a href="#budgets">Comparer les budgets HA</a></div></header>'+''.join(body for _,_,body in sections)+'<footer>V6 · prototype isolé · aucune commande physique · <a href="catalogue-memoire-v5.html">V5</a> · <a href="catalogue-memoire-v4.html">V4</a></footer></main><script>'+js+'</script></body></html>'
    page = page.replace('<a href="#budgets">Comparer les budgets HA</a>', '<a href="#alignement">96 ou 128 : comparer</a><a href="#budgets">Comparer les budgets HA</a>')
    OUT.write_text(page)
    print(json.dumps({'sections':len(sections),'layouts':len(names),'budgets':len(data['budgets']),'html_bytes':len(page.encode()),'output':str(OUT.relative_to(ROOT))},ensure_ascii=False))


SCRIPT = r'''
'use strict';
const DATA=__DATA__;
const labels={rows:'Z1 · lignes canoniques',numeric:'Z1 · annexe Numeric32',numeric_refs:'Z1 · références numériques u32',aliases:'Z2 · Alias32',slots:'Z3 · slots globaux',metadata:'Z4 · Meta32',id_index:'Z3 · index ID sondé',edges:'Z5 · Edge24 sources',csr:'Z5 · CSR avant/inverse',alias_reverse:'Z5 · source → alias',typed_index:'Z5 · index typés',schemas:'Z4 · Schema64 partagés',transforms:'Z4 · Transform64 partagées',components:'Z4 · composants froids',calculations:'Z7 · Calc48',freshness:'Z7 · fraîcheur',routes:'Z7 · Route32',pool_entries:'Z6 · descripteurs des valeurs',pool_bytes:'Z6 · octets des valeurs',string_bytes:'Z6 · octets des chaînes',string_entries:'Z6 · répertoires des chaînes',route_index:'Z7 · index des routes',string_search:'Z6 · recherche clés/natif',headers:'En-têtes et enveloppe Bank',runtime_buffers:'Z7 · parsing et files proposés'};
const colors=['#1f604f','#8baf8a','#c7b56e','#be8341','#385f7c','#8e6e80','#687d4c','#4e8c88','#a15a43','#738bab','#6b956b','#adad68','#796449','#c6966b','#538672','#c3a5a7','#808749','#6777a0','#9e7960','#a3bfb5','#685c75'];
const money=n=>Number(n).toLocaleString('fr-FR');
const mib=n=>(n/1048576).toFixed(3);
const safe=s=>String(s).replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
function tab(head,rows){return '<div class="scroll"><table><thead><tr>'+head.map(x=>'<th scope="col">'+safe(x)+'</th>').join('')+'</tr></thead><tbody>'+rows.map(row=>'<tr>'+row.map(x=>'<td>'+safe(x)+'</td>').join('')+'</tr>').join('')+'</tbody></table></div>';}
function renderLayout(){
 const s=DATA.layouts.structures.find(x=>x.name===document.getElementById('layout-choice').value);
 const fields=Object.entries(s.offsets); let x=0;
 const parts=fields.map(([name,offset],i)=>{const end=i+1<fields.length?fields[i+1][1]:s.size;const width=(end-offset)/s.size*850;const fragment='<rect x="'+(20+x)+'" y="35" width="'+Math.max(0,width-1)+'" height="45" fill="'+colors[i%colors.length]+'"><title>'+safe(name)+' : offset '+offset+', étendue '+(end-offset)+' octets</title></rect><text x="'+(20+x)+'" y="102">'+offset+'</text>';x+=width;return fragment;});
 document.getElementById('layout-map').innerHTML='<svg role="img" aria-label="'+safe(s.name)+' de '+s.size+' octets" viewBox="0 0 900 125"><title>'+safe(s.name)+' : champs proportionnels à leur étendue</title><text x="20" y="20">'+safe(s.name)+' · '+s.size+' octets · alignement '+s.align+'</text>'+parts.join('')+'<text x="870" y="102">'+s.size+'</text></svg><p class="layout-note">Les couleurs distinguent les champs ; leurs noms et offsets restent dans le tableau. Les tables exhaustives sont aussi accessibles ci-dessous.</p>';
 document.getElementById('layout-fields').innerHTML=tab(['Champ','Offset','Étendue, octets'],fields.map(([name,offset],i)=>[name,offset,(i+1<fields.length?fields[i+1][1]:s.size)-offset]));
}
function renderBudget(){
 const dataset=document.getElementById('dataset-choice').value,variant=document.getElementById('variant-choice').value;
 const available=DATA.budget.budgets.filter(x=>x.dataset===dataset).map(x=>x.scale);
 const scaleControl=document.getElementById('scale-choice');
 for(const option of scaleControl.options)option.disabled=!available.includes(Number(option.value));
 if(!available.includes(Number(scaleControl.value)))scaleControl.value=String(available[0]);
 const scale=Number(scaleControl.value);
 const record=DATA.budget.budgets.find(x=>x.dataset===dataset&&x.scale===scale);
 if(!record){document.getElementById('budget-result').textContent='Scénario absent des artefacts.';return;}
 const chosen=record.variants[variant],zones=Object.entries(chosen.zones),counts=record.counts;
 document.getElementById('budget-result').innerHTML='<p><span class="budget-big">'+mib(chosen.total)+' Mio</span><br><b>CALCULÉ · '+safe(dataset)+' ×'+scale+' · '+safe(variant)+'</b></p><p class="muted">'+money(counts.canonical)+' canoniques · '+money(counts.aliases)+' alias · '+money(counts.slots)+' slots · '+money(counts.edges)+' arêtes. Budget de base, hors contenu variable précédent ; sans garantie RSS.</p>'+tab(['Index','Octets réservés : sonde ou extrapolation','Capacité','Méthode'],[['IDs',money(record.index_probe.bytes),money(record.index_probe.capacity),record.index_probe.method],['Routes',money(record.route_index_probe.bytes),money(record.route_index_probe.capacity),record.route_index_probe.method]]);
 const regions=[['Z1','Canoniques',['rows','numeric','numeric_refs']],['Z2','Alias',['aliases']],['Z3','Localisation',['slots','id_index','headers']],['Z4','Données froides',['metadata','schemas','transforms','components']],['Z5','Graphes',['edges','csr','alias_reverse','typed_index']],['Z6','Variables',['pool_entries','pool_bytes','string_bytes','string_entries','string_search']],['Z7','Exécution',['calculations','freshness','routes','route_index','runtime_buffers']]];
 const grouped=regions.map(([key,label,keys])=>[key+' · '+label,keys.reduce((n,k)=>n+chosen.zones[k],0)]);
 let cursor=20;
 const rects=grouped.map(([name,value],i)=>{const width=value/chosen.total*850;const rect='<rect x="'+cursor+'" y="35" width="'+width+'" height="65" fill="'+colors[i%colors.length]+'"><title>'+safe(labels[name]||name)+' : '+money(value)+' octets ('+(value/chosen.total*100).toFixed(2)+' %)</title></rect>';cursor+=width;return rect;});
 document.getElementById('budget-chart').innerHTML='<svg role="img" aria-label="Décomposition du budget calculé" viewBox="0 0 900 140"><title>Zones mémoire : '+mib(chosen.total)+' Mio calculés</title><text x="20" y="20">0 → '+mib(chosen.total)+' Mio · zones additives, largeur proportionnelle</text>'+rects.join('')+'<text x="20" y="125">Survol : région ; le tableau détaille chaque poste.</text></svg><div class="legend">'+grouped.map(([name,value],i)=>'<span style="border-left:10px solid '+colors[i]+'">'+safe(name)+' : '+mib(value)+' Mio</span>').join('')+'</div>';
 document.getElementById('budget-zones').innerHTML=tab(['Zone disjointe','Octets calculés / réservés pour index','Mio','Part'],zones.map(([name,value])=>[labels[name]||name,money(value),mib(value),(value/chosen.total*100).toFixed(2)+' %']).concat([['Total de base, hors contenu variable précédent',money(chosen.total),mib(chosen.total),'100 %']]));
 const records=DATA.budget.budgets.filter(x=>x.dataset===dataset).sort((a,b)=>a.scale-b.scale);
 document.getElementById('extrapolation').innerHTML=tab(['Échelle synthétique','Canoniques','Alias','A · base Row96, Mio','B · base Row64 + annexe, Mio','C · base Hot/Cold, Mio'],records.map(x=>['×'+x.scale,money(x.counts.canonical),money(x.counts.aliases),mib(x.variants.row96.total),mib(x.variants.row64.total),mib(x.variants.split.total)]));
}
document.getElementById('layout-choice').addEventListener('change',renderLayout);
for(const id of ['dataset-choice','scale-choice','variant-choice'])document.getElementById(id).addEventListener('change',renderBudget);
renderLayout();renderBudget();
'''

if __name__ == '__main__':
    main()
