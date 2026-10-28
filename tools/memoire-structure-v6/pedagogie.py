"""Parcours visuel local de V6 : explication et simulation, sans moteur ni réseau.

Les tailles proviennent de layouts.json. Le scénario est fictif et ses choix de
publication/conversion restent proposés : ce module n'est pas une preuve Rust.
"""
import html
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent


def esc(value):
    return html.escape(str(value))


def table(headers, rows):
    return '<div class="scroll p-scroll"><table><thead><tr>' + ''.join(
        '<th scope="col">' + esc(x) + '</th>' for x in headers
    ) + '</tr></thead><tbody>' + ''.join(
        '<tr>' + ''.join('<td>' + esc(x) + '</td>' for x in row) + '</tr>' for row in rows
    ) + '</tbody></table></div>'


REGIONS = [
    ('Z1', 'Les fiches canoniques', 'Lieux, équipements, points, résultat de calcul, contrat, binding. Une fiche : Row96. Les valeurs et calculs ont un état ; les lieux et actions n’ont pas de mesure propre.'),
    ('Z2', 'Les raccourcis vers une fiche', 'Quatre alias : chacun possède un ID et une source, aucune valeur ni date propres.'),
    ('Z3', 'Le répertoire pour retrouver une fiche', 'ID → slot → position. Vingt GlobalSlot16 ; index ID à ajouter.'),
    ('Z4', 'Les étiquettes et règles partagées', 'Vingt Meta32 ; schémas, unités et transformations partagés.'),
    ('Z5', 'Les liens entre les fiches', 'Parents, membres de zone, sources de calcul, source → alias. Index variables.'),
    ('Z6', 'Les textes et gros contenus', 'Noms, clés natives, textes et composites : répertoires + octets, hors budget de base.'),
    ('Z7', 'Les recettes et le travail en cours', 'Définition du calcul, routes et plans ; buffers temporaires de messages séparés.'),
]


def append_sections(sec, layouts):
    """Append at most five chapters before the existing technical reference."""
    sizes = {s['name']: s['size'] for s in layouts['structures']}
    fixture = json.loads((HERE / 'exemple-pedagogique.json').read_text())
    n_canonical, n_aliases = len(fixture['canonical']), len(fixture['aliases'])
    n_slots = n_canonical + n_aliases
    canonical = sizes['Row96'] + sizes['GlobalSlot16'] + sizes['Meta32']
    alias = sizes['Alias32'] + sizes['GlobalSlot16'] + sizes['Meta32']
    base = n_canonical * canonical + n_aliases * alias
    extra_counts = fixture['extras']
    extras = sum(extra_counts[key] * sizes[name] for key, name in [('schemas', 'Schema64'), ('transforms', 'Transform64'), ('calculations', 'Calc48'), ('routes', 'Route32')])
    region_cards = ''.join(f'<div class="p-region" data-p-region="{key}"><b>{key} · {esc(title)}</b><p>{esc(body)}</p><span class="p-activity">Au repos</span></div>' for key, title, body in REGIONS)
    sec('p-maison', '1. Partons d’une petite maison fictive.', '''
<p class="lead">Un capteur connaît une température. La pièce et l’étage peuvent afficher cette même température : on range <strong>une mesure</strong> et on crée <strong>deux raccourcis</strong>. Une moyenne, elle, produit un nouveau résultat et possède donc sa propre fiche.</p>
<div class="p-banner"><strong>Simulation pédagogique — aucun ordre réel.</strong><span>Ce parcours explique une proposition de stockage. Il ne se connecte à aucune maison.</span></div>
<div class="p-tree" aria-label="Organisation de l’exemple"><span>Famille 1000</span><i>→</i><span>Maison 1001</span><i>→</i><span>Étage 1002</span><i>→</i><span>Pièce 1003</span></div>
<p class="p-caption">Zone 1004, attachée à la maison, référence la pièce et ses équipements. Un lien de zone ne crée aucune nouvelle mesure.</p>
<div class="p-world">
 <div class="p-world-source"><div class="p-kicker">Les propriétaires physiques</div>
  <article class="p-object"><span>Capteur A · D10</span><strong>P101 · 21 °C</strong><small>La température est stockée ici, une seule fois.</small></article>
  <article class="p-object"><span>Capteur B · D20</span><strong>P201 · 23 °C</strong><small>Une seconde source indépendante.</small></article>
  <article class="p-object"><span>Lampe · D30</span><strong>P301 · 50 %</strong><small>Dernier état observé dans cet exemple proposé.</small></article>
 </div>
 <div class="p-world-links" aria-label="Liens"><span>les vues lisent<br>← ces sources</span><span>101 + 201<br>→ calcul</span><span>un ordre revient<br>← au propriétaire</span></div>
 <div class="p-world-views"><div class="p-kicker">Deux vues virtuelles</div>
  <article class="p-object p-view"><span>Vue pièce · V900</span><strong>901 → P101 · 21 °C</strong><small>902 → P201 · 23 °C<br>903 → P301 · 50 %</small></article>
  <article class="p-object p-view"><span>Vue étage · V910</span><strong>911 → P101 · 21 °C</strong><small>Le même 21 ; aucun champ valeur dans l’alias.</small></article>
  <article class="p-object p-calculation"><span>Calcul C904 · propriété de V900</span><strong>(21 + 23) / 2 = 22 °C</strong><small>Nouveau résultat : une fiche et un état propres.</small></article>
 </div>
</div>
<p><strong>Quand P101 passe à 21,5 °C :</strong> 901 et 911 affichent aussitôt 21,5 en relisant P101. Aucune valeur n’est recopiée dans ces deux alias. C904 doit en revanche être recalculé : (21,5 + 23) / 2 = 22,25 °C.</p>
<details><summary>Voir tous les objets et chaque lien de l’exemple</summary>''' + table(['Objet', 'Parent / propriétaire', 'État ou lien'], [
        ('Famille 1000', 'Racine', 'Contient la maison ; pas une mesure.'),
        ('Maison 1001', '1000', 'Contient étage et zone.'),
        ('Étage 1002', '1001', 'Contient la pièce 1003.'),
        ('Pièce 1003', '1002', 'Contient D10, D20, D30 et vue pièce V900.'),
        ('Zone 1004', '1001', 'Référence pièce 1003 et équipements ; liens, sans état copié.'),
        ('D10 / D20 / D30', '1003', 'Trois équipements physiques.'),
        ('V900 / V910', '1003 / 1001', 'Deux équipements virtuels ; la vue étage est rattachée à la maison.'),
        ('P101 / P201 / P301', 'D10 / D20 / D30', '21 °C / 23 °C / 50 % : trois états propres.'),
        ('Alias 901 / 902 / 903', 'V900', 'Sources 101 / 201 / 301 ; aucun état propre.'),
        ('Alias 911', 'V910', 'Source 101 ; aucun état propre.'),
        ('Calcul 904', 'V900', 'Sources 101 et 201 ; résultat propre à 22 °C.'),
        ('Contrat 8001 « Identifier »', '1001', 'Signature partagée de portée maison ; capacité fictive non qualifiée.'),
        ('Binding 906', 'D30', 'Référence le contrat 8001 et cible la lampe D30.'),
        ('V900 → binding 906', 'Lien de présentation COMMAND_TARGET', 'Expose l’action depuis la vue, sans Alias32 ni valeur persistante.'),
    ]) + '<h3>Les liens de zone, de calcul et d’action</h3>' + table(['Départ', 'Relation proposée', 'Arrivée'], [(edge['from'], edge['relation'], edge['to']) for edge in fixture['edges']]) + '<p class="p-caption">Le parent de chaque canonique reste dans <code>canonical[].parent</code> : cette liste d’arêtes ne le remplace pas. Les sources d’alias restent dans <code>aliases[].source_id</code>. Les index de parcours parents, dépendances et source → alias sont des représentations de navigation supplémentaires à construire. Aucun coût d’arêtes ou d’index n’est inclus dans les 2 624 octets de base ni dans les 624 octets d’extras fixes.</p></details><p class="p-links">Suite : <a href="#p-rangements">où ces objets sont rangés</a> · <a href="#p-parcours">suivre une mesure ou un ordre</a> · <a href="#p-couts">comprendre le coût</a>.</p>')

    type_rows = [(kind, count, f'{canonical} o', f'{count * canonical} o', note) for kind, count, note in [
        ('Famille', 1, 'Organisation, pas de mesure.'), ('Maison', 1, 'Organisation.'), ('Étage', 1, 'Organisation.'),
        ('Pièce', 1, 'Organisation.'), ('Zone', 1, 'Ses membres sont des liens en plus.'),
        ('Équipement physique', 3, 'Capteurs A/B et lampe.'), ('Équipement virtuel', 2, 'Vue pièce et vue étage.'),
        ('Valeur physique', 3, 'Températures et luminosité.'), ('Résultat de calcul', 1, f'+ {sizes["Calc48"]} o pour sa définition, dans les extras.'),
        ('Contrat d’action', 1, 'Signature partagée, sans valeur.'), ('Binding d’action', 1, 'Cible D30, sans valeur.'),
    ]]
    type_rows += [('Alias', 4, f'{alias} o', f'{4 * alias} o', 'Aucune observation propre ; liens vers trois sources.'), ('Base disjointe', '16 + 4', '20 slots', f'{base} o', 'Sans index, liens, pools, plans, textes ni capacité libre.')]
    row_fields = [
        ('Identité + parent', 16, '#1f604f'), ('Courant', 8, '#a4bf9d'), ('Précédent', 8, '#9ebba8'), ('2 dates', 16, '#c7b56e'),
        ('Schéma, références, version, flags et types', 16, '#487b83'), ('Contraintes numériques', 32, '#bb8753'),
    ]
    row_strip = ''.join(f'<span style="flex:{size};background:{color}"><span>{size} o</span><small>{esc(name)}</small></span>' for name, size, color in row_fields)
    sec('p-rangements', '2. Sept rangements ; dix-huit formats de pièces.', '''
<p class="lead">Les <strong>7 régions</strong> sont sept responsabilités de stockage. Les <strong>18 layouts</strong> sont les formats de petites structures que l’on peut y utiliser. Ce ne sont ni 18 objets par appareil, ni 7 copies de chaque valeur.</p>
<div class="p-regions p-static-regions">''' + region_cards + '''</div>
<p>Ces régions sont un <strong>plan de classement logique</strong> : des tables allouées par Rust, pas sept barrettes de RAM ni sept pages physiques fixes. Exemple : une valeur utilise une Row96 en Z1, un slot en Z3, une Meta32 en Z4 et des références vers un schéma, des liens et des textes. Ces morceaux s’additionnent ; ils ne sont pas sept versions de la température.</p>
<h3>Une fiche Row96 vue comme une bande de 96 octets</h3>
<div class="p-byte-strip" role="img" aria-label="96 octets : 16 identité et parent, 8 courant, 8 précédent, 16 deux dates, 16 schéma références version flags types, 32 contraintes numériques">''' + row_strip + '''</div>
<p class="p-caption">Largeurs proportionnelles. 1 octet = 8 bits ; le mot de valeur fait 8 octets = 64 bits. Une valeur booléenne n’économise pas cette réserve. Pour une organisation ou une action, la valeur absente et les contraintes inutilisées restent allouées dans Row96.</p>
<h3>Courant, précédent distinct et deux dates dans la même fiche</h3>
''' + table(['Dans Row96', 'Taille', 'Ce que cela signifie'], [
        ('payload', '8 octets', 'Valeur courante, ou handle vers un contenu texte/composite dans un pool.'),
        ('previous_payload', '8 octets', 'État distinct remplacé au dernier changement, ou handle vers son contenu variable. Absent avant tout changement.'),
        ('received_ms', '8 octets', 'Dernière réception locale, même si la valeur est identique.'),
        ('changed_ms', '8 octets', 'Date source transitoire du dernier changement de type ou valeur ; inchangée sur un relevé identique.'),
        ('flags', 'Bits déjà réservés', 'Présence du courant : 0x10 ; du précédent : 0x40 ; type précédent : masque 0xF000. Aucun octet supplémentaire.'),
        ('Historique complet et dates précédentes', 'Hors Row96', 'Une seule valeur précédente distincte, sans sa date : ni archive complète, ni buffer de fenêtre temporelle.'),
    ]) + '''<p><strong>21 → 22 → 22 → 23 :</strong> au départ, courant 21 et précédent absent. Au premier changement, courant 22 et précédent 21. Le paquet identique renouvelle seulement réception et révision : précédent 21 et date du changement sont conservés. Au changement suivant, courant 23 et précédent 22. Un changement de type compte aussi, même si l’affichage numérique paraît identique. Les alias lisent courant, précédent et dates dans cette même source ; ils ne stockent aucun état.</p>
<p>Le premier relevé d’un point encore jamais renseigné initialise son courant sans créer de précédent. Dans la simulation, les quatre valeurs initiales ont déjà été relevées : leur premier changement conserve donc 21, 23, 50 ou 22 selon le point. Une organisation, un contrat et une action restent sans état de mesure.</p>
<p>Exemple : 20 °C puis encore 20 °C reçus à 08:05:01 ; seule la réception et la révision avancent. Si la valeur devient 21 °C à 08:06 et arrive à 08:06:01, le précédent devient 20, le dernier changement prend 08:06 et la réception 08:06:01. La date source arrive avec le paquet pour cette transition ; elle n’est pas conservée comme troisième horodatage dans Row96 ni dans Hot32.</p>
<p>Les 8 octets du précédent ne contiennent pas tout un texte ou un composite : ils portent un handle vers un pool, dont la durée de vie doit préserver les deux états. La place des octets variables et leur rétention restent en plus ; le pool composite append-only de démonstration n’est pas un historique métier. Le précédent n’inclut aucune date précédente. Les sources sans date, relevés retardés et horloges décalées demandent encore un contrat explicite.</p>
<p>Pour une fenêtre temporelle, il faut les échantillons et leurs dates sources dans un historique séparé. Calc48 peut suivre la réception la plus ancienne des sources et la qualité du résultat ; cela ne remplace ni les dates sources de l’historique, ni la couverture temporelle. La moyenne des dernières valeurs, comme (21,5 + 23) / 2 ici, est différente d’une moyenne pondérée par les durées.</p>
<p><a href="#layout-Row96">Row96 : tous les offsets</a> · <a href="#layout-Alias32">Alias32 : ses 32 octets sans valeur</a> · <a href="#layout-GlobalSlot16">Slot16</a> · <a href="#layout-Meta32">Meta32</a> · <a href="#layouts">explorer les 18 formats</a></p>
<h3>Le coût par type, dans cette petite maison</h3>
<p><strong>Une fiche canonique = ''' + f'{sizes["Row96"]} + {sizes["GlobalSlot16"]} + {sizes["Meta32"]} = {canonical} o.</strong> Un alias = {sizes["Alias32"]} + {sizes["GlobalSlot16"]} + {sizes["Meta32"]} = {alias} o.' + '</p>' + table(['Type', 'Nombre', 'Coût unitaire de base', 'Sous-total', 'Ce qui s’ajoute'], type_rows) +
    table(['Rangement de base', 'Calcul', 'Octets'], [
        ('Z1 · fiches canoniques', f'{n_canonical} × {sizes["Row96"]}', n_canonical * sizes['Row96']),
        ('Z2 · alias', f'{n_aliases} × {sizes["Alias32"]}', n_aliases * sizes['Alias32']),
        ('Z3 · slots', f'{n_slots} × {sizes["GlobalSlot16"]}', n_slots * sizes['GlobalSlot16']),
        ('Z4 · métadonnées', f'{n_slots} × {sizes["Meta32"]}', n_slots * sizes['Meta32']),
        ('Base', 'Somme des quatre lignes précédentes', base),
    ]) + '<h3>Les extras fixes de cet exemple, une seule fois</h3>' + table(['Pièce partagée', 'Nombre × layout', 'Octets'], [
        ('Schémas', f'{extra_counts["schemas"]} × Schema64 ({sizes["Schema64"]})', extra_counts['schemas'] * sizes['Schema64']),
        ('Transformations', f'{extra_counts["transforms"]} × Transform64 ({sizes["Transform64"]})', extra_counts['transforms'] * sizes['Transform64']),
        ('Définition du calcul', f'{extra_counts["calculations"]} × Calc48 ({sizes["Calc48"]})', extra_counts['calculations'] * sizes['Calc48']),
        ('Routes', f'{extra_counts["routes"]} × Route32 ({sizes["Route32"]})', extra_counts['routes'] * sizes['Route32']),
        ('Extras', 'Somme de ces quatre lignes', extras),
    ]) + f'<div class="p-total"><strong>{base} + {extras} = {base + extras} octets décrits</strong><span>Base + extras fixes. Index ID, parents, zones, sources, publications, textes, pools, capacités, conteneurs et buffers ne sont toujours pas comptés. Ce total n’est pas la RAM du processus (RSS).</span></div>' + '<h3>Pourquoi existe-t-il aussi des formats plus petits ?</h3>' + table(['Nature de fiche', 'A · Row96', 'B · Row64 + annexe / C · Hot32 + Cold32'], [
        ('Non numérique : lieux, équipements, contrats, bindings', f'{canonical} o', f'{sizes["Row64"]} + 4 référence + {sizes["GlobalSlot16"]} + {sizes["Meta32"]} = {sizes["Row64"] + 4 + sizes["GlobalSlot16"] + sizes["Meta32"]} o'),
        ('Numérique : trois valeurs et le calcul', f'{canonical} o', f'{sizes["Row64"]} + {sizes["Numeric32"]} annexe + 4 référence + {sizes["GlobalSlot16"]} + {sizes["Meta32"]} = {sizes["Row64"] + sizes["Numeric32"] + 4 + sizes["GlobalSlot16"] + sizes["Meta32"]} o'),
        ('Alias sans état', f'{alias} o', f'{alias} o'),
        ('Cet exemple : 12 non numériques + 4 numériques + 4 alias', f'{base} o', f'{12 * (sizes["Row64"] + 4 + sizes["GlobalSlot16"] + sizes["Meta32"]) + 4 * (sizes["Row64"] + sizes["Numeric32"] + 4 + sizes["GlobalSlot16"] + sizes["Meta32"]) + n_aliases * alias} o'),
    ]) + '<p class="p-caption">B et C paient 4 octets de référence par canonique : 148 o sur un numérique, contre 144 o pour A ; ils économisent sur les autres fiches, à 116 o. Ici 2 624 o contre 2 304 o pour la seule base. Aucun index, en-tête, capacité libre ou pool dans cette comparaison. C sépare le contenu chaud de 32 o du contenu froid de 32 o ; les deux font ensemble 64 o.</p>')

    sec('p-parcours', '3. Faites circuler une donnée, étape par étape.', '''
<div class="p-banner"><strong>Simulation pédagogique — aucun ordre réel.</strong><span>Chaque changement ci-dessous reste dans cette page. Suivant avance d’une étape ; Précédent reconstruit exactement l’état précédent.</span></div>
<div class="p-sim-controls"><div><label for="p-case">Parcours à suivre</label><select id="p-case"><option value="mqtt">Une mesure arrive par MQTT</option><option value="api">Une mesure arrive par API constructeur</option><option value="command">Commander 80 % depuis la vue pièce</option><option value="blocked">Un ordre refusé par la validation</option><option value="same">La même mesure arrive à nouveau</option><option value="action">Deux actions « Identifier » identiques</option></select></div><div><label for="p-notification-mode">Publication MQTT virtuelle — choix proposé</label><select id="p-notification-mode"><option value="source_unique">Un message par source modifiée</option><option value="alias_topics">Un message pour chaque alias aussi</option></select></div></div>
<div class="p-step-controls"><button type="button" id="p-reset">Début</button><button type="button" id="p-previous">← Précédent</button><button type="button" id="p-next">Suivant →</button><span id="p-position"></span></div>
<div id="p-current" class="p-current" aria-live="polite" aria-atomic="true"></div>
<h3>Le mouvement entre les fiches</h3><div id="p-memory-flow"></div><p id="p-step-work" class="p-caption"></p>
<div class="p-legend"><span class="p-read">R · lecture</span><span class="p-write">W · écriture permanente</span><span class="p-temp">T · travail / copie temporaire</span><span>Le texte et la lettre doublent la couleur.</span></div>
<div id="p-sim-regions" class="p-regions">''' + region_cards + '''</div>
<h3>Avant → après cette étape</h3><div id="p-state"></div>
<h3>Travail cumulé depuis le début du parcours</h3><p class="p-caption">Comptage d’opérations logiques de ce modèle : ni instructions CPU, ni appels de l’allocateur Rust, ni octets réellement transférés dans les caches.</p><div id="p-counters"></div>
<details open><summary>Journal des étapes déjà parcourues</summary><ol id="p-log" class="p-log"></ol></details>
<h3>Messages et occurrences simulés</h3><div id="p-messages"></div>
<noscript><p>Activez JavaScript pour parcourir la simulation. Les cinq chapitres, tableaux et références restent lisibles sans JavaScript.</p></noscript>
<p class="p-caption">Chaque scénario repart du même état : 21 °C, 23 °C, 50 %, moyenne 22 °C. Les petites dates 1000/1900/2000 sont des millisecondes fictives de démonstration. Les tests Rust structurels V6 ne valident pas cette simulation JavaScript.</p>''')

    sec('p-transformations', '4. Un message brut ne devient pas directement un état.', '''
<div class="p-pipeline" aria-label="Chemin d’une mesure"><div><b>1 · Entrée brute</b><code>{"temperature_deci_c":215}</code><small>JSON MQTT ou réponse HTTP : buffer temporaire.</small></div><i>→</i><div><b>2 · Recette de lecture</b><code>215 × 0,1</code><small>Cet émetteur fictif déclare des déci-°C.</small></div><i>→</i><div><b>3 · État canonique</b><code>P101 = 21,5 °C</code><small>Une écriture source ; 901 et 911 pointent encore vers 101.</small></div></div>
<p>Ce décodage appartient à l’émetteur fictif <code>fixtures/temp-a/state</code> ou à sa réponse <code>GET /devices/fixture-temp-a/status</code>. Il ne faut pas diviser automatiquement toute température MQTT ou Zigbee2MQTT par dix.</p>
<p>Le calcul dépendant lit P101 et P201 puis écrit son résultat C904 = 22,25 °C. L’index inverse proposé <code>101 → [901, 911]</code> sert à retrouver les vues à notifier ; le lien de dépendance <code>101 → 904</code> sert à déclencher la moyenne. Ils ne contiennent pas de copies de la température.</p>
<h3>Le chemin d’un ordre est différent</h3>
<pre>POST /api/command
{"point":"demo:virtual-room/brightness","value":80}

903 → P301 → propriétaire D30 → contrat % + garde
  → plan d’écriture : round(80 × 254 / 100) = 203
  → zigbee2mqtt/fixture-light/set {"brightness":203}, non retenu
  → ACK 200 {"ok":true} : accepté dans la file locale du pilote
     pas de confirmation du broker MQTT ni de mesure physique
  → retour observé : {"brightness":203}
  → plan de lecture : 203 × 100 / 254 = 79,921259… %
  → P301 mis à jour ; alias 903 affiche 79,9 %</pre>
<p><strong>La demande à 80 % ne force jamais la fiche observée à 80.</strong> L’état reste à 50 % jusqu’au retour de l’appareil. Zigbee2MQTT répond au succès de <code>try_publish</code> : l’acceptation dans sa file locale ne prouve ni la livraison au broker, ni l’exécution physique. Un type ou une plage refusés arrêtent le parcours avant mise en file. Si la garde retient l’ordre, il devient une demande d’approbation 202 : aucune mise en file avant décision humaine. La protection s’applique à D30, même si l’ordre vient d’une vue virtuelle.</p>
<p class="p-caption">Une commande scalaire de luminosité n’accepte pas de <code>request_id</code>. Le <code>correlation_id</code> de la simulation reste une étiquette interne à sa trace ; il n’est pas envoyé dans le corps REST.</p>
<div class="notice"><strong>Statut de cet exemple.</strong> La route REST et la forme du JSON existent aujourd’hui. Les IDs <code>demo:…</code>, les vues virtuelles, le contrat canonique en %, les recettes 0..100 ↔ 0..254 et la publication MQTT ci-dessous sont proposés. Dans le produit actuel, la luminosité Zigbee2MQTT reste dans son domaine natif 0..254.</div>
<h3>Notifier les vues : deux choix à décider</h3>''' + table(['Choix proposé', 'Après la mesure 21 → 21,5', 'Ce que le client doit savoir'], [
        ('Source unique', '2 messages : source 101 et résultat 904.', 'Le client possède le mapping 901/911 → 101 et rafraîchit les deux vues.'),
        ('Un topic par alias', '4 messages : 101, 901, 911 et 904.', 'Deux sérialisations / messages supplémentaires, toujours aucune écriture de valeur dans les alias.'),
        ('Même valeur 21 → 21', '0 message de changement numérique dans cette leçon.', 'Réception et révision renouvelées ; précédent et date du changement conservés.'),
    ]) + '''<pre>Topic proposé : moli/v1/example/points/901/state
{"id":"901","sourceId":"101","value":21.5,"sourceRevision":2}</pre>
<p>Ces IDs sont des chaînes dans les messages. <code>sourceRevision</code> aide le client à ignorer un doublon et à éviter un écho entre vues. Un message d’état peut éventuellement être retenu par MQTT ; un ordre ou une occurrence « Identifier » ne doit pas être retenu ni rejoué au prochain abonnement. Le mode « source unique » suppose une découverte fiable du mapping.</p>
<p>La sérialisation fabrique des octets et des enveloppes temporaires, même avec zéro copie d’état dans les alias. Le choix du transport, de la fiabilité, des files bornées, de la rétention et de la reprise reste à concevoir et mesurer. Un alias ne rend pas la publication gratuite.</p>
<h3>Une action sans valeur : « Identifier » deux fois</h3>
<p>La vue V900 présente l’action via un lien <code>COMMAND_TARGET → 906</code>. Le binding 906, rattaché à D30, référence le contrat fictif 8001 de portée maison. Ce lien d’action n’est pas un Alias32 de valeur. Deux appels de même contenu avec <code>request_id = aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:1</code> puis <code>aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:2</code> produisent deux occurrences, deux mises en file locales et deux entrées de journal. Le format 32 hexadécimaux + séquence positive identifie une occurrence : un réessai conserve exactement la même identité pour l’anti-doublon et la corrélation ; une nouvelle intention incrémente la séquence. Ce champ est réservé aux boutons, dont la valeur REST est <code>null</code>.</p>
<p>Une illustration native <code>{"identify":true}</code> n’est pas une capacité Zigbee2MQTT qualifiée ici. Une mise en file ne confirme ni livraison au broker ni effet matériel. La luminosité reste à 50 % ; on ne crée aucune fausse valeur persistante « pressé = true ».</p>''')

    fanout_rows = [(n, 2, n + 2, 0, f'{n * alias} o + index') for n in [1, 10, 100, 1000]]
    sec('p-couts', '5. Ce qui coûte de la mémoire, et du travail CPU.', '''
<p class="lead">Partager un état évite des écritures permanentes. Il reste à parser le message, suivre les liens, calculer, sérialiser et transmettre. Le coût dépend du chemin parcouru et du nombre de destinataires.</p>''' + table(['Opération', 'Lectures / écritures permanentes', 'Mémoire temporaire', 'Travail à mesurer sur le vrai moteur'], [
        ('Recevoir + parser', 'Lire la route et le contrat.', 'Octets entrants et représentation JSON bornée.', 'Taille du JSON, champs, validation et codec.'),
        ('Résoudre un alias', 'Lire répertoire, Alias32 et source ; aucune écriture de valeur alias.', 'Petit résultat local ; aucune allocation de fiche nécessaire.', 'Index, générations, dépendance au cache CPU.'),
        ('Mettre à jour une source', '1 fiche source modifiée ; 0 fiche alias modifiée.', 'Paquet stable à copier avant de libérer le verrou.', 'Comparer type/valeur et renouveler les dates.'),
        ('Recalculer une moyenne', 'Lire 2 états, écrire 1 résultat propre.', 'Résultat intermédiaire.', 'Dépendances, fraîcheur et nombre de calculs.'),
        ('Notifier A alias', 'Parcourir les liens et lire les sources ; 0 état alias modifié.', 'Sérialisation et enveloppes pour les messages.', 'O(A) si l’on parcourt ou publie chaque alias.'),
        ('Émettre une commande', 'Lire contrat, cible, garde et plan ; ne pas écrire l’état demandé.', 'Enveloppe native et entrée de journal.', 'Validation, encodage, I/O, garde et confirmation.'),
        ('Libérer le travail', 'Les fiches et alias restent présents.', 'Libérer ou réutiliser buffers et enveloppes.', 'Allocateur, files, backpressure et pics transitoires.'),
    ]) + '<h3>Une source, une moyenne dépendante, A alias</h3>' + table(['Nombre d’alias A', 'Messages source unique', 'Messages avec tous les alias', 'Valeurs d’alias écrites', 'Base des alias'], fanout_rows) + '''
<p class="p-caption">Scénario de changement numérique, sans autres abonnés ni calculs. 2 = une source + un calcul ; A + 2 ajoute les alias. 80A inclut Alias32 + slot16 + Meta32 ; les index de liens et de publication restent en plus. Ce tableau calcule des comptes, pas du temps CPU ni toute la RAM.</p>
<h3>À quel moment le verrou est-il tenu ?</h3>
<div class="p-lock"><span>Préparer / valider</span><i>→</i><strong>Verrou d’écriture : modifier 1 source, copier un paquet stable</strong><i>→</i><span>Libérer le verrou</span><i>→</i><span>Calcul / sérialisation / I/O selon versions vérifiées</span></div>
<p>La proposition ne conserve pas un <code>RwLock</code> pendant une attente MQTT, une réponse du pilote ou une approbation humaine. Le paquet publié doit rester cohérent après libération : copie temporaire explicite ou snapshot versionné à choisir. Les calculs dépendants doivent vérifier les versions ; cette leçon séquentielle ne prouve aucune atomicité concurrente.</p>
<h3>Trois niveaux de preuve à garder séparés</h3>''' + table(['Niveau', 'Ce qui existe', 'Ce qu’il permet de conclure'], [
        ('Livré dans Moli', 'Décodage Z2M, encodage natif, REST, garde, journal, SSE.', 'Les chemins actuels sont documentés ; ils ne migrent pas implicitement vers V6.'),
        ('Prototype V6 isolé', '18 layouts, slots, Alias32, résolutions et tests structurels Rust.', 'Tailles et invariants locaux vérifiés ; aucun profil CPU du futur Hub.'),
        ('Proposé / simulation locale', 'Canonique en %, index source → alias, MQTT virtuel, parcours et compteurs de cette page.', 'Explique les décisions et mouvements ; aucun temps en nanosecondes ni mémoire RSS mesurés.'),
    ]) + '''<p>Avant tout benchmark, il faut fixer le parser, les sérialisations, les index, les files, les garanties de publication et les durées de verrou. Puis mesurer débit, latence, contention, allocations et pics sur le moteur intégré. <a href="#mesures">Les mesures historiques V2–V5</a> et <a href="#budgets">les budgets calculés V6</a> restent plus bas, avec leur périmètre propre.</p>
<h3>Les recherches sont rapides grâce aux index, pas au seul nombre 96</h3>
''' + table(['Recherche', 'Accès prévu', 'État de la preuve V6'], [
        ('ID connu → valeur', 'Index hash ID → slot → ligne ; coût constant attendu, pas une garantie de latence.', 'Index et accès présents dans Bank ; latence du futur moteur non mesurée.'),
        ('Alias → valeur réelle', 'Nombre borné de liens et contrôles ; les chaînes d’alias sont aplaties.', 'Résolution testée, sans copie d’état.'),
        ('Membres directs d’une zone', 'Index de liens : parcourir les seuls membres, coût proportionnel à leur nombre.', 'CSR testé ; expansion et déduplication des pièces à développer.'),
        ('Nom, type, unité, plage de valeurs, dernier changement', 'Index dédié ou parcours des lignes ; sans index adapté, coût proportionnel au catalogue.', 'Ces index de requête ne sont pas tous implémentés dans Bank.'),
        ('Précédent distinct', 'Lire previous_payload et ses bits de présence/type sur la source.', 'Inclus dans Row96 / Hot32, lu aussi depuis les alias ; aucune date précédente.'),
        ('Historique complet ou fenêtres temporelles', 'Index historique et échantillons datés ; lire les données pertinentes, hors Row96.', 'Banc temporel V5 distinct, intégration V6 à faire.'),
    ]) + '''<p>Une taille fixe facilite l’accès à la position <code>base + indice × 96</code> et le rangement contigu. Elle n’identifie pas à elle seule la bonne ligne et ne transforme pas une recherche arbitraire en accès direct. Une Row96 n’est pas une ligne de cache CPU garantie ; certains scans des anciens bancs sont plus rapides avec les structures plus petites. Les tests Rust établissent les tailles et invariants, pas la performance de tous les parcours d’un moteur migré.</p>
<p class="p-links"><a href="#p-parcours">Rejouer les six parcours</a> · <a href="#regions">Référence des sept régions</a> · <a href="#layouts">Référence des dix-huit layouts</a> · <a href="../../../tools/memoire-structure-v6/exemple-pedagogique.json">Fixture fictive de cette leçon</a>.</p>''')


CSS = r'''
/* Lesson uses the report's cream/green visual language, scoped to p- classes. */
[id^="p-"]{scroll-margin-top:24px;min-width:0}[id^="p-"] h3{margin-top:34px}.p-banner{display:grid;gap:5px;background:#1b5945;color:#fffdf3;padding:20px 24px;margin:24px 0;border-radius:6px}.p-banner strong{font-size:18px}.p-banner span{font-size:14px;line-height:1.6}.p-tree{display:flex;align-items:center;gap:10px;flex-wrap:wrap;margin-top:28px}.p-tree span{background:#e6eddf;border:1px solid #b9cbb8;border-radius:4px;padding:10px 14px;font-weight:600}.p-tree i,.p-pipeline>i,.p-lock>i{font:26px Georgia,serif;color:#7f8c69;font-style:normal}.p-caption{font-size:13px;color:#506457;line-height:1.65}.p-world{display:grid;grid-template-columns:minmax(0,1fr) 110px minmax(0,1fr);gap:14px;background:#ebeede;padding:24px;border:1px solid #d3ddc8;margin:28px 0}.p-world-source,.p-world-views{display:grid;gap:12px;align-content:start;min-width:0}.p-kicker{font-size:11px;font-weight:700;text-transform:uppercase;letter-spacing:.08em;color:#526f5a;padding-bottom:2px}.p-object{display:grid;gap:8px;background:#fffdf5;border:1px solid #b4c7b8;border-left:5px solid #276651;padding:18px;min-width:0}.p-object>span{font-size:12px;color:#536c5c}.p-object strong{font:22px Georgia,serif;color:#1d523f;line-height:1.4}.p-object small{font-size:13px;line-height:1.65;color:#496052}.p-view{border-left-color:#6287a1}.p-calculation{border-left-color:#b48445;background:#f7efd9}.p-world-links{display:flex;flex-direction:column;justify-content:space-around;gap:25px;text-align:center;font-size:12px;line-height:1.7;color:#4d6755}.p-world-links span{border-top:1px solid #9bad94;border-bottom:1px solid #9bad94;padding:12px 0}.p-links a{display:inline-block;margin:6px 10px 6px 0}.p-regions{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:10px;margin:22px 0}.p-region{background:#f8f9ef;border:1px solid #c8d3bf;border-radius:4px;padding:16px;min-width:0}.p-region b{font-size:14px;line-height:1.6;display:block;color:#294b38}.p-region p{font-size:12px;margin:7px 0;line-height:1.7;color:#526655}.p-activity{display:block;font-size:12px;border-top:1px solid #d4dec8;padding-top:8px;color:#596e5c}.p-static-regions .p-activity{display:none}.p-region.p-read{background:#e7eff4;border-color:#7292a8}.p-region.p-write{background:#f9ead7;border-color:#bd8746}.p-region.p-temp{background:#eaece8;border-color:#90998f}.p-byte-strip{display:flex;min-height:110px;margin:24px 0 14px;border:1px solid #69826e;overflow:hidden;border-radius:4px}.p-byte-strip>span{min-width:0;color:#fff;display:flex;flex-direction:column;padding:10px 7px;justify-content:space-between;gap:12px}.p-byte-strip>span>span{font:21px Georgia,serif;white-space:nowrap}.p-byte-strip small{font-size:10px;line-height:1.5;overflow-wrap:anywhere}.p-byte-strip>span:nth-child(2),.p-byte-strip>span:nth-child(3){color:#153d2b}.p-total{display:grid;gap:10px;background:#e5eddf;border:1px solid #a6bf9e;padding:24px;margin:20px 0}.p-total strong{font:26px Georgia,serif;color:#1a563e}.p-total span{font-size:14px;line-height:1.8}.p-sim-controls{display:grid;grid-template-columns:1.1fr 1fr;gap:20px;margin:24px 0 0}.p-sim-controls>div{display:flex;flex-direction:column;min-width:0}.p-sim-controls select{width:100%;font-size:14px;white-space:normal}.p-step-controls{display:flex;gap:8px;align-items:center;flex-wrap:wrap;padding:8px 0 20px}.p-step-controls button{font:600 13px "Avenir Next",Verdana,sans-serif;background:#fffdf5;color:#20523c;border:1px solid #7d977f;border-radius:4px;min-height:44px;padding:10px 14px;cursor:pointer}.p-step-controls button:last-of-type{background:#215840;color:white}.p-step-controls button:disabled{cursor:default;background:#edf0e6;color:#697565;border-color:#c0ccba}.p-step-controls span{font-size:12px;margin-left:auto;color:#536854}.p-current{background:#f2f0df;border-top:3px solid #38684c;padding:20px 24px;min-height:165px}.p-current h3{margin:0 0 10px!important;font:25px Georgia,serif;color:#21533d}.p-current p{margin:8px 0;line-height:1.8}.p-current code{overflow-wrap:anywhere;white-space:normal}.p-step-note{font-size:13px;color:#62684e}.p-legend{display:flex;gap:8px;flex-wrap:wrap;margin:20px 0;font-size:12px}.p-legend span{padding:7px 9px;background:#f4f5eb}.p-legend .p-read{background:#e7eff4;color:#244d68}.p-legend .p-write{background:#f9ead7;color:#7b4515}.p-legend .p-temp{background:#eaece8;color:#3b4a3b}.p-log{padding:4px 28px 16px 45px}.p-log li{font-size:13px;line-height:1.8;padding:5px}.p-log li:last-child{font-weight:600;color:#1c573d}.p-changed{font-weight:700;background:#f9ead7}.p-message{background:#f8f9f0;border:1px solid #ccd7c1;padding:14px 18px;margin:10px 0;min-width:0}.p-message b{font-size:12px;color:#526550}.p-message code{display:block;margin-top:8px;font-size:12px;overflow-wrap:anywhere;white-space:pre-wrap}.p-message small{display:block;margin-top:8px;font-size:12px;color:#5b6e59}.p-pipeline{display:grid;grid-template-columns:1fr 22px 1fr 22px 1fr;gap:12px;align-items:center;margin:26px 0}.p-pipeline>div{display:grid;gap:12px;padding:20px;background:#eff2e5;border:1px solid #c0cfb6;height:100%;box-sizing:border-box;min-width:0}.p-pipeline b{font-size:14px;color:#31513a}.p-pipeline code{font-size:15px;overflow-wrap:anywhere;white-space:normal}.p-pipeline small{font-size:12px;line-height:1.7;color:#596c54}.p-lock{display:flex;align-items:center;gap:12px;flex-wrap:wrap;padding:20px;background:#eff2e5;font-size:13px;line-height:1.7}.p-lock strong{padding:14px;background:#f2dfbe;max-width:260px}.p-scroll table{font-size:13px}.p-scroll td{vertical-align:top;line-height:1.7}.p-scroll{max-width:100%}[id^="p-"] pre{max-width:100%;box-sizing:border-box;overflow:auto;font-size:12px;line-height:1.9}[id^="p-"] code{overflow-wrap:anywhere}
@media(max-width:760px){.p-world{grid-template-columns:minmax(0,1fr) 72px minmax(0,1fr);padding:14px;gap:8px}.p-object{padding:12px}.p-object strong{font-size:19px}.p-world-links{font-size:10px}.p-sim-controls{grid-template-columns:1fr;gap:0}.p-pipeline{grid-template-columns:1fr}.p-pipeline>i{transform:rotate(90deg);justify-self:center}.p-pipeline>div{width:100%}}
@media(max-width:540px){.p-world{grid-template-columns:1fr}.p-world-links{flex-direction:row;gap:8px;font-size:11px}.p-world-links span{flex:1}.p-regions{grid-template-columns:1fr}.p-current{padding:18px;min-height:210px}.p-current h3{font-size:23px}.p-banner{padding:18px}.p-byte-strip{min-height:135px}.p-byte-strip>span{padding:8px 3px}.p-byte-strip>span>span{font-size:15px}.p-byte-strip small{font-size:9px}.p-total{padding:18px}.p-total strong{font-size:23px}.p-step-controls span{width:100%;margin:5px 0 0}.p-step-controls button{flex:1;padding:10px 7px}.p-tree{gap:6px}.p-tree span{padding:8px;font-size:12px}.p-scroll td{min-width:100px}}
@media print{.p-sim-controls,.p-step-controls{display:none}.p-world,.p-regions,.p-current{break-inside:avoid}.p-current{min-height:0}}
.p-memory-flow{display:grid;grid-template-columns:minmax(0,1fr) 28px minmax(0,1fr);gap:12px;align-items:stretch;margin-top:18px}.p-memory-flow article{background:#f7f9f0;border:1px solid #bfceb7;padding:16px;min-width:0;display:grid;gap:9px}.p-memory-flow b{font-size:13px;color:#34573d}.p-memory-flow strong{font:24px Georgia,serif;color:#1d573e}.p-memory-flow small{font-size:12px;line-height:1.7;color:#51674e}.p-memory-flow i{align-self:center;font:27px Georgia,serif;color:#5c7d66;font-style:normal}.p-memory-flow .p-row-written{background:#f9ead7;border-color:#bd8746}.p-memory-result{grid-column:1/-1}.p-memory-flow .p-memory-result strong{font-size:20px}@media(max-width:540px){.p-memory-flow{grid-template-columns:1fr;gap:7px}.p-memory-flow i{text-align:center;transform:rotate(90deg)}.p-memory-flow strong{font-size:22px}}
'''


def script():
    """Embed fixture safely; no external transport and no collision with DATA."""
    fixture = json.loads((HERE / 'exemple-pedagogique.json').read_text())
    payload = json.dumps(fixture, ensure_ascii=False, separators=(',', ':')).replace('</', '<\\/')
    return SCRIPT.replace('__LESSON_FIXTURE__', payload)


SCRIPT = r'''
(() => {
 'use strict';
 const fixture = __LESSON_FIXTURE__;
 const clone = x => JSON.parse(JSON.stringify(x));
 const safe = s => String(s).replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
 const names = {mqtt:'Mesure MQTT',api:'Mesure API constructeur',command:'Commande virtuelle à 80 %',blocked:'Commande refusée',same:'Mesure identique',action:'Deux actions identiques'};
 const countersEmpty = () => ({parses:0,linkReads:0,rowReads:0,sourceWrites:0,calculationWrites:0,aliasWrites:0,calculations:0,temporaryCreated:0,temporaryFreed:0,publications:0,commands:0,occurrences:0});
 function initial(){const points=Object.fromEntries(Object.entries(fixture.initial_values).map(([id,value])=>[id,{value,kind:'number',previous:null,previousKind:null,hasObservation:true,received:1000,changed:1000,revision:1}]));const aliases=Object.fromEntries(fixture.aliases.map(x=>[x.id,x.source_id]));return {points,aliases,calculationMetadata:{oldestSourceReceived:1000,quality:'fixture_only'},commands:[],messages:[],occurrences:[],journal:[],temporaryLive:0,counters:countersEmpty(),permanentDeltaBytes:0,ack:null,pending:null};}
 function transform(value,plan){const scaled=value*plan.mul+(plan.add||0);return plan.rounding==='nearest_native_integer'?Math.round(scaled):scaled;}
 function mean(s){const ids=fixture.calculation.sources;return ids.reduce((sum,id)=>sum+s.points[id].value,0)/ids.length;}
 const step = (title,text,regions,ops={},apply=null,note='') => ({title,text,regions,ops,apply,note});
 const increment = (s,ops) => {for(const [k,v] of Object.entries(ops))s.counters[k]+=v;s.temporaryLive += (ops.temporaryCreated||0)-(ops.temporaryFreed||0);};
 // Transition pure des valeurs scalaires de la fixture ; aucun pool texte/composite simulé.
 function transitionPoint(point,value,{kind=typeof value,sourceTime=1900,received=2000}={}){
  const next=clone(point),wasPresent=point.hasObservation===true;
  const changed=!wasPresent||point.kind!==kind||!Object.is(point.value,value);
  next.received=received;next.revision=point.revision+1;
  if(changed){next.previous=wasPresent?clone(point.value):null;next.previousKind=wasPresent?point.kind:null;next.value=value;next.kind=kind;next.changed=sourceTime;}
  next.hasObservation=true;return next;
 }
 const update = (s,id,value) => {s.points[id]=transitionPoint(s.points[id],value,{kind:'number'});};
 const publish = (s,id) => {const sourceId=s.aliases[id]||id,p=s.points[sourceId];s.messages.push({topic:fixture.notification.prefix+'/'+id+'/state',payload:{id:String(id),sourceId:String(sourceId),value:p.value,sourceRevision:p.revision},retain:'Possible pour un état ; choix proposé',kind:'state'});};
 function notificationStep(ids,extraText=''){
  return step('Sérialiser puis notifier',`Les identités ${ids.join(', ')} reçoivent un message d’état. ${extraText} Une enveloppe logique par message est créée ; les valeurs des alias ne sont jamais écrites.`,{Z1:'R',Z2:'R',Z5:'R',Z7:'T'},{rowReads:ids.length,linkReads:ids.filter(x=>fixture.aliases.some(a=>a.id===x)).length,temporaryCreated:ids.length,publications:ids.length},s=>ids.forEach(id=>publish(s,id)),'Les octets JSON sont des copies temporaires. Ces compteurs n’estiment ni leur taille ni le nombre exact d’allocations.');
 }
 function cleanup(){return step('Libérer les temporaires ; conserver les états','Les buffers entrants, commandes et messages sérialisés sont libérés ou réutilisés. Les fiches, dates et liens restent. La liste de messages ci-dessous est une trace de la simulation, pas une file réseau vivante.',{Z7:'T'},{},s=>{s.counters.temporaryFreed+=s.temporaryLive;s.temporaryLive=0;});}
 function measurement(kind,mode){
  const identical=kind==='same', api=kind==='api',raw=identical?fixture.samples.unchanged_native:fixture.samples.changed_native,canonical=transform(raw,fixture.transforms.temperature_read),source=fixture.samples.field,calculation=fixture.calculation.id;
  const ids=mode==='alias_topics'?[source,...(fixture.reverse_aliases[source]||[]),calculation]:[source,calculation];
  const list=[step('État initial : une seule mesure partagée','P101 = 21 °C ; P201 = 23 °C ; C904 = 22 °C. Les alias 901 et 911 ne contiennent aucune valeur : ils lisent P101.',{},{}),
   step(api?'Recevoir la réponse de l’API constructeur':'Recevoir le message MQTT',`${api?fixture.sensor_input.api_path:fixture.sensor_input.mqtt_topic} : ${JSON.stringify({[fixture.samples.key]:raw})}. Cet émetteur fictif déclare des déci-°C. Les octets sont placés dans un buffer temporaire borné.`,{Z7:'T'},{temporaryCreated:1}),
   step('Parser le JSON et retrouver le point',`Lire ${fixture.samples.key} ; la route propose le point ${source}. Le répertoire identifie sa fiche et son propriétaire D${fixture.source_owners[source]}. Le chemin API et le chemin MQTT rejoignent ici la même représentation.`,{Z3:'R',Z4:'R',Z7:'T'},{parses:1,linkReads:2,temporaryCreated:1}),
   step('Appliquer le plan de lecture',`${raw} × ${fixture.transforms.temperature_read.mul} = ${canonical} °C. Vérifier type, unité et limites ; le résultat n’est pas encore publié. Cette échelle appartient à cet émetteur, pas à toute température MQTT.`,{Z4:'R',Z7:'R'},{linkReads:1}),
   step(identical?'Rafraîchir les dates, valeur inchangée':'Écrire une seule fiche source',identical?'P101 reste à 21 °C. received passe à 2000 et la révision à 2 ; changed reste à 1000 et le précédent reste absent. Les alias lisent ces mêmes champs sur la source.':`P101 passe de 21 à ${canonical} °C ; précédent = 21, changed = 1900, received = 2000, révision = 2. Aucune écriture dans les alias.`,{Z1:'W'},{rowReads:1,sourceWrites:1},s=>update(s,source,canonical),'Comme Bank::observe en Rust, changed prend la date source du changement. La date source est transitoire ; un paquet identique conserve changed et le précédent.')];
  if(identical){list.push(step('Signaler la fraîcheur sans faux changement numérique','Le relevé confirme la fraîcheur de 101. Aucun nouveau changement de valeur : ni quatre messages numériques, ni recalcul de la moyenne nécessaire ici. Une notification de fraîcheur distincte reste un choix de contrat.',{Z5:'R',Z7:'R'},{linkReads:1}));}
  else {list.push(step('Suivre les liens, sans copier les valeurs',`L’index inverse proposé trouve ${source} → [${(fixture.reverse_aliases[source]||[]).join(', ')}]. L’index des dépendances trouve ${source} → [${(fixture.dependencies[source]||[]).join(', ')}]. Les deux alias affichent déjà ${canonical} °C en lisant leur source.`,{Z2:'R',Z5:'R'},{linkReads:(fixture.reverse_aliases[source]||[]).length+(fixture.dependencies[source]||[]).length}));
   list.push(step('Recalculer le résultat qui lui appartient','Lire P101 = 21,5 et P201 = 23 ; (21,5 + 23) / 2 = 22,25 °C. Écrire C904 : courant 22,25 et précédent 22. La plus ancienne réception des sources (1000) et la qualité sont suivies dans les métadonnées du calcul, séparément des deux dates de la fiche. Il s’agit de la moyenne des dernières valeurs, pas d’une moyenne temporelle. Les alias 901 et 911 restent sans état.',{Z1:'W',Z5:'R',Z7:'R'},{rowReads:fixture.calculation.sources.length,calculationWrites:1,calculations:1},s=>{update(s,calculation,mean(s));s.calculationMetadata.oldestSourceReceived=Math.min(...fixture.calculation.sources.map(id=>s.points[id].received));}));
   list.push(notificationStep(ids,mode==='alias_topics'?'Choix « par alias » : 4 messages au total.':'Choix « source unique » : 2 messages ; le client connaît le mapping des alias.'));
  }
  list.push(cleanup());return list;
 }
 function commandSteps(mode,refused){
  const command=fixture.command,target=command.target,native=transform(command.canonical_value,fixture.transforms.brightness_write),observed=transform(command.observed_native,fixture.transforms.brightness_read);
  const ids=mode==='alias_topics'?[target,...(fixture.reverse_aliases[target]||[])]:[target];
  const list=[step('État initial : la lampe observée est à 50 %','Le point virtuel 903 est un raccourci vers P301, propriété de la lampe D30. Envoyer un souhait ne modifie pas l’état observé.',{},{}),
   step('Lire la demande REST',`POST ${command.path} ${JSON.stringify({...command.body,value:refused?180:command.canonical_value})}. La forme REST existe ; cette identité virtuelle et le contrat % sont proposés.`,{Z7:'T'},{parses:1,temporaryCreated:2}),
   step('Résoudre le virtuel jusqu’au propriétaire physique',`Répertoire → alias ${command.alias} → source ${target} → propriétaire D${command.physical_owner}. Lire contrat, politique et route : une vue ne déplace pas la protection de l’appareil physique.`,{Z2:'R',Z3:'R',Z4:'R',Z7:'R'},{linkReads:4,rowReads:1})];
  if(refused){list.push(step('Refuser avant toute mise en file','180 % est hors contrat 0..100 : demande refusée, journalisée. Aucun plan d’écriture exécuté, aucun paquet MQTT mis en file, P301 reste à 50 %. La garde peut aussi retenir une demande valide (202), sans mise en file avant approbation.',{Z4:'R',Z7:'T'},{},s=>s.journal.push({correlation_id:command.correlation_id,status:'refused',reason:'value outside 0..100'})));list.push(cleanup());return list;}
  list.push(step('Valider le contrat et appliquer la garde','80 est un nombre dans 0..100. Le scénario suppose une cible neutre et une autorisation obtenue. En maison réelle, la garde peut retenir un agent ; il faut conserver le propriétaire D30 pour la décider.',{Z4:'R',Z7:'R'},{linkReads:2}));
  list.push(step('Encoder le souhait vers le domaine natif',`Plan d’écriture proposé : round(${command.canonical_value} × ${fixture.transforms.brightness_write.mul}) = ${native}. Créer le paquet ${command.native_topic} ${JSON.stringify({brightness:native})}. P301 vaut toujours 50 %. Le paquet n’est pas retenu. La corrélation ${command.correlation_id} est interne à cette trace, absente du corps REST.`,{Z4:'R',Z7:'T'},{temporaryCreated:1},s=>{s.pending={correlation_id:command.correlation_id,target,requested:command.canonical_value};s.commands.push({topic:command.native_topic,payload:{brightness:native},correlation_id:command.correlation_id,retain:fixture.notification.command_retention,emitted:false,emitted_scope:'local_queue',locally_queued:false,status:'prepared',broker_acknowledged:false,physical_confirmed:false});}));
  list.push(step('Mettre en file locale et accuser la demande','ACK REST 200 {"ok":true} suit le succès de try_publish : le pilote accepte l’ordre dans sa file locale. Aucune confirmation de livraison au broker MQTT ni d’effet physique. La demande est journalisée ; l’état observé reste à 50 %.', {Z7:'T'},{commands:1},s=>{Object.assign(s.commands[0],{emitted:true,locally_queued:true,status:'queued_locally'});s.pending={...s.pending,status:'awaiting_observation'};s.ack={status:200,ok:true,means:'accepted_local_queue',broker_acknowledged:false,physical_confirmed:false};s.journal.push({correlation_id:command.correlation_id,status:'queued_locally'});}));
  list.push(step('Recevoir la mesure physique et décoder',`Retour fictif : ${JSON.stringify({brightness:command.observed_native})}. Le plan de lecture proposé calcule ${command.observed_native} × 100 / 254 = ${observed} %. Le retour, lui seul, permet de mettre à jour P301.`,{Z4:'R',Z7:'T'},{parses:1,linkReads:1,temporaryCreated:2}));
  list.push(step('Écrire l’état observé et relire l’alias','P301 reçoit 79,921259… %. L’affichage de 903 devient 79,9 % ; aucune valeur 80 n’a été imposée dans la source et aucune valeur n’est stockée dans l’alias.',{Z1:'W',Z2:'R'},{sourceWrites:1,rowReads:1,linkReads:1},s=>{update(s,target,observed);s.pending=null;}));
  list.push(notificationStep(ids,mode==='alias_topics'?'Une source + un alias = 2 messages.':'Un message source ; le client résout 903 → 301.'));
  list.push(cleanup());return list;
 }
 function actionSteps(){const action=fixture.action;const send=(s,request_id)=>{s.commands.push({topic:fixture.command.native_topic,payload:clone(action.native_payload),request_id,retain:fixture.notification.command_retention,emitted:true,emitted_scope:'local_queue',locally_queued:true,status:'queued_locally',broker_acknowledged:false,physical_confirmed:false});s.occurrences.push({binding:action.binding,contract:action.contract,target:action.owner,request_id});s.journal.push({request_id,status:'queued_locally'});};return [step('État initial : une action n’est pas une valeur',`V${action.virtual_owner} présente COMMAND_TARGET → binding ${action.binding} → contrat fictif ${action.contract} « Identifier » → cible D${action.owner}. Ce lien n’est pas un Alias32 d’état. P301 reste à 50 %. La commande native identify est uniquement une illustration non qualifiée.`,{},{}),
  step('Parser le premier appel',`Appel de bouton avec value = null, request_id = ${action.request_ids[0]}. Format : espace client de 32 hexadécimaux + séquence positive. L’identité corrèle cette occurrence et empêche un doublon lors d’un réessai conservant exactement la même identité. Aucun état « bouton pressé » n’est créé.`,{Z7:'T'},{parses:1,temporaryCreated:2}),
  step('Résoudre, valider et mettre en file la première occurrence',`Lire binding, contrat, cible et garde. Accepter ${JSON.stringify(action.native_payload)} dans la file locale, sans rétention ; journaliser cette occurrence. Ni livraison au broker ni effet matériel ne sont confirmés.`,{Z1:'R',Z3:'R',Z4:'R',Z7:'T'},{linkReads:4,rowReads:1,temporaryCreated:1,commands:1,occurrences:1},s=>send(s,action.request_ids[0])),
  step('Parser le second appel de même contenu',`request_id = ${action.request_ids[1]} : nouvelle séquence, nouvelle intention. Un réessai de la première conserverait ${action.request_ids[0]}. Un booléen persistant aurait perdu cette deuxième occurrence.`,{Z7:'T'},{parses:1,temporaryCreated:2}),
  step('Mettre en file la deuxième occurrence','Même binding et même cible, nouvelle occurrence et nouvelle entrée de journal. Deux commandes acceptées dans la file locale au total ; la luminosité reste à 50 %. Aucune livraison au broker ni exécution physique ne sont confirmées. Les occurrences ne sont pas des messages d’état retenus.',{Z1:'R',Z3:'R',Z4:'R',Z7:'T'},{linkReads:4,rowReads:1,temporaryCreated:1,commands:1,occurrences:1},s=>send(s,action.request_ids[1])),cleanup()];}
 let selected='mqtt', mode=fixture.notification.modes[0], index=0, state=initial(), before=initial(), steps=[];
 function rebuild(){steps=selected==='action'?actionSteps():selected==='command'||selected==='blocked'?commandSteps(mode,selected==='blocked'):measurement(selected,mode);index=Math.max(0,Math.min(index,steps.length-1));state=initial();before=initial();for(let i=0;i<=index;i++){if(i===index)before=clone(state);increment(state,steps[i].ops);if(steps[i].apply)steps[i].apply(state);}render();return snapshot();}
 function values(s){return Object.fromEntries(Object.entries(s.points).map(([id,p])=>[id,p.value]));}
 function snapshot(){const c=state.counters;return clone({case:selected,mode,step:index,index,total:steps.length,title:steps[index].title,values:values(state),previous_values:Object.fromEntries(Object.entries(state.points).map(([id,p])=>[id,p.previous])),aliases:fixture.aliases.map(({id,source_id,owner})=>({id,source_id,owner})),resolved_alias_values:Object.fromEntries(fixture.aliases.map(a=>[a.id,state.points[a.source_id].value])),resolved_alias_previous_values:Object.fromEntries(fixture.aliases.map(a=>[a.id,state.points[a.source_id].previous])),pending:state.pending,notifications:state.messages,outgoing:state.commands.filter(x=>x.emitted),occurrences:state.occurrences,timestamps:Object.fromEntries(Object.entries(state.points).map(([id,p])=>[id,{received:p.received,changed:p.changed}])),stats:{source_writes:c.sourceWrites,alias_writes:c.aliasWrites,calculation_writes:c.calculationWrites,logical_reads:c.rowReads,link_reads:c.linkReads,parses:c.parses,calculations:c.calculations,temporary_created:c.temporaryCreated,temporary_freed:c.temporaryFreed,temporary_live:state.temporaryLive,publications:c.publications,commands:c.commands,occurrences:c.occurrences,permanent_delta_bytes:0},state,before,after:state,counters:state.counters,regions:steps[index].regions,history:steps.slice(0,index+1).map(x=>({title:x.title,text:x.text,regions:x.regions})),fixture});}
 function choose(value){const aliases={'measurement-mqtt':'mqtt','measure-mqtt':'mqtt','measurement-api':'api','api-manufacturer':'api','virtual-command':'command','command-80':'command','refused':'blocked','command-refused':'blocked','identical':'same','same-value':'same','actions':'action','two-actions':'action'};const chosen=aliases[value]||value;if(!Object.hasOwn(names,chosen))throw new Error('Parcours inconnu : '+value);selected=chosen;index=0;return rebuild();}
 function next(){index++;return rebuild();}function previous(){index--;return rebuild();}function reset(){index=0;return rebuild();}
 function setMode(value){const aliases={source:'source_unique',aliases:'alias_topics'};value=aliases[value]||value;if(!fixture.notification.modes.includes(value))throw new Error('Mode inconnu');mode=value;return rebuild();}
 function tab(headers,rows){return '<div class="scroll p-scroll"><table><thead><tr>'+headers.map(x=>'<th scope="col">'+safe(x)+'</th>').join('')+'</tr></thead><tbody>'+rows.map(row=>'<tr>'+row.map(x=>'<td>'+safe(x)+'</td>').join('')+'</tr>').join('')+'</tbody></table></div>';}
 const number = (n,digits=5) => Number(n).toLocaleString('fr-FR',{maximumFractionDigits:digits});
 function visibleRows(s){const out=[];for(const id of Object.keys(fixture.initial_values)){const p=s.points[id];out.push([`${id===fixture.calculation.id?'C':'P'}${id} · valeur`,`${number(p.value)} ${fixture.units[id]}`]);out.push([`${id} · précédent distinct`,p.previousKind===null?'Absent':`${number(p.previous)} ${fixture.units[id]}`]);out.push([`${id} · received / changed`,`${p.received} / ${p.changed}`]);out.push([`${id} · révision`,String(p.revision)]);}for(const alias of fixture.aliases){const id=alias.id,source=s.aliases[id];out.push([`Alias ${id} → ${source}`,`${number(s.points[source].value,source===fixture.command.target?1:5)} ${fixture.units[source]} ; précédent : ${s.points[source].previousKind===null?'absent':number(s.points[source].previous)} (lus à la source ; aucun état dans l’alias)`]);}out.push(['Ordres acceptés dans la file locale',String(s.counters.commands)],['Occurrences d’action',String(s.counters.occurrences)],['Temporaires logiques encore vivants',String(s.temporaryLive)],['Nouvelles fiches permanentes allouées','0']);return out;}
 function render(){
  if(typeof document==='undefined'||!document.getElementById('p-current'))return;
  const current=steps[index];document.getElementById('p-case').value=selected;document.getElementById('p-notification-mode').value=mode;document.getElementById('p-position').textContent=`Étape ${index+1} / ${steps.length} · ${names[selected]}`;document.getElementById('p-previous').disabled=index===0;document.getElementById('p-reset').disabled=index===0;document.getElementById('p-next').disabled=index===steps.length-1;
  document.getElementById('p-current').innerHTML='<h3>'+safe(current.title)+'</h3><p>'+safe(current.text)+'</p>'+(current.note?'<p class="p-step-note">'+safe(current.note)+'</p>':'');
  const pointId=['command','blocked','action'].includes(selected)?fixture.command.target:fixture.samples.field,point=state.points[pointId],oldPoint=before.points[pointId],aliases=fixture.aliases.filter(a=>a.source_id===pointId),didWrite=point.revision!==oldPoint.revision;
  const result=state.points[fixture.calculation.id],oldResult=before.points[fixture.calculation.id];
  document.getElementById('p-memory-flow').innerHTML='<div class="p-memory-flow"><article'+(didWrite?' class="p-row-written"':'')+'><b>Z1 · Source P'+safe(pointId)+' · '+(didWrite?'fiche modifiée':'fiche conservée')+'</b><strong>'+safe(number(oldPoint.value))+' → '+safe(number(point.value))+' '+safe(fixture.units[pointId])+'</strong><small>Avant → après cette étape. Précédent distinct : '+safe(point.previousKind===null?'absent':number(point.previous))+' ; deux dates dans cette seule fiche.</small></article><i aria-label="Les vues lisent la source">←</i><article><b>Z2 · Raccourcis, aucune valeur stockée</b><strong>'+aliases.map(a=>safe(a.id)+' → '+safe(pointId)).join('<br>')+'</strong><small>Lecture actuelle : '+safe(number(point.value,pointId===fixture.command.target?1:5))+' '+safe(fixture.units[pointId])+' ; précédent lu : '+safe(point.previousKind===null?'absent':number(point.previous))+'. Les liens restent identiques ; 0 écriture d’état alias.</small></article><article class="p-memory-result'+(result.revision!==oldResult.revision?' p-row-written':'')+'">'+(selected==='action'?'<b>Action : un chemin vers une cible</b><strong>V900 → binding 906 → contrat 8001 / D30</strong><small>'+state.occurrences.length+' occurrence(s) ; aucune nouvelle valeur persistante.</small>':'<b>Z1 · Résultat C904 · sa propre fiche</b><strong>'+safe(number(oldResult.value))+' → '+safe(number(result.value))+' °C</strong><small>Précédent distinct : '+safe(result.previousKind===null?'absent':number(result.previous))+'. Lire P101 et P201 puis écrire C904 à l’étape de calcul.</small>')+'</article></div>';
  const delta=k=>state.counters[k]-before.counters[k];document.getElementById('p-step-work').textContent='Cette étape : '+delta('rowReads')+' lecture(s) logique(s) de fiche, '+delta('linkReads')+' lien(s) suivi(s), '+delta('sourceWrites')+' écriture(s) source, '+delta('calculationWrites')+' écriture(s) de résultat, 0 écriture alias, '+delta('temporaryCreated')+' temporaire(s) créé(s), '+delta('temporaryFreed')+' libéré(s). Comptes logiques, pas des instructions CPU.';
  const meanings={R:'Lecture des structures existantes',W:'Écriture de fiche permanente (lectures préalables possibles)',T:'Travail / copies temporaires'};
  for(const card of document.querySelectorAll('#p-sim-regions [data-p-region]')){const activity=current.regions[card.dataset.pRegion];card.classList.remove('p-read','p-write','p-temp');if(activity)card.classList.add({R:'p-read',W:'p-write',T:'p-temp'}[activity]);card.querySelector('.p-activity').textContent=activity?activity+' · '+meanings[activity]:'Au repos pendant cette étape';}
  const old=visibleRows(before),now=visibleRows(state);document.getElementById('p-state').innerHTML=tab(['Champ / lecture', 'Avant cette étape', 'Après cette étape'],now.map((row,i)=>[row[0],old[i][1],row[1]+(old[i][1]!==row[1]?' ← changé':'')]));
  const labels={parses:'Messages JSON parsés',linkReads:'Liens / références suivis',rowReads:'Lectures logiques de fiches',sourceWrites:'Fiches source mises à jour',calculationWrites:'Fiches résultat écrites',aliasWrites:'États d’alias écrits',calculations:'Calculs exécutés',temporaryCreated:'Unités temporaires créées',temporaryFreed:'Unités temporaires libérées',publications:'Messages d’état proposés',commands:'Ordres natifs en file locale',occurrences:'Occurrences d’action'};
  document.getElementById('p-counters').innerHTML=tab(['Opération logique','Nombre cumulé'],Object.entries(labels).map(([key,label])=>[label,state.counters[key]]));
  document.getElementById('p-log').innerHTML=steps.slice(0,index+1).map(x=>'<li>'+safe(x.title)+'</li>').join('');
  const entries=state.commands.map(x=>({label:'Commande native simulée'+(x.locally_queued?' · acceptée dans la file locale':' · préparée, pas encore mise en file'),topic:x.topic,payload:x.payload,note:(x.request_id?'request_id '+x.request_id:'correlation_id interne '+x.correlation_id)+' · retain = false · livraison au broker et effet physique non confirmés'})).concat(state.messages.map(x=>({label:'Publication d’état proposée',topic:x.topic,payload:x.payload,note:x.retain}))).concat(state.occurrences.map(x=>({label:'Occurrence (journal, sans état booléen)',topic:'Contrat '+x.contract+' → cible D'+x.target,payload:x,note:'Ne pas retenir / rejouer comme un état.'})));
  document.getElementById('p-messages').innerHTML=entries.length?entries.map(x=>'<div class="p-message"><b>'+safe(x.label)+'</b><code>'+safe(x.topic)+'\n'+safe(JSON.stringify(x.payload))+'</code><small>'+safe(x.note)+'</small></div>').join(''):'<p class="p-caption">Aucun message ni occurrence à cette étape.</p>';
 }
 window.memoryLesson={choose,next,previous,reset,snapshot,setMode,transitionPoint,readAliasPrevious:id=>state.points[state.aliases[String(id)]].previous,readAlias:id=>state.points[state.aliases[String(id)]].value};
 if(typeof document!=='undefined'&&document.getElementById('p-case')){document.getElementById('p-case').addEventListener('change',e=>choose(e.target.value));document.getElementById('p-notification-mode').addEventListener('change',e=>setMode(e.target.value));document.getElementById('p-next').addEventListener('click',next);document.getElementById('p-previous').addEventListener('click',previous);document.getElementById('p-reset').addEventListener('click',reset);}
 rebuild();
})();
'''
