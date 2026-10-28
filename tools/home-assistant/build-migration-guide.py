#!/usr/bin/env python3
"""Build the offline explanatory migration guide from dated project evidence."""
from pathlib import Path
import hashlib
import html
import json

ROOT = Path(__file__).resolve().parents[2]
BASE = ROOT / 'docs/proposition/analyse/v6'
OUT = ROOT / 'docs/proposition/migration-home-assistant.html'

COMPONENTS = [
    ('tahoma', '01', 'Volets et TaHoma', 'action', 'Priorité du premier lot',
     'Pilote Overkiz natif et chemin MQTT→Hub→Overkiz testés en simulation. L’observateur de neutralisation HA reste fermé.',
     'L’appareil et tous les chemins qui peuvent le commander : intégration, groupes, scènes, scripts et commandes directes de la box. La portée peut être toute une box, pas un seul volet.',
     ['Relever ouvrir / fermer / stop ; ajouter le favori seulement s’il est déclaré.', 'Identifier les groupes ou scénarios qui atteignent le même moteur.', 'Distinguer le dernier ordre d’une position réellement mesurée.'],
     'Préparer le pilote Moli fermé en écriture. Neutraliser et vérifier le périmètre HA complet avec un adaptateur qualifié ; accorder ensuite une nouvelle génération Moli. Les automatismes HA utilisent alors le proxy prévu et validé.',
     'Fermer d’abord le droit Moli et ses ordres encore en attente, observer l’arrêt, puis réaccorder HA avec une génération neuve. Si une commande est peut-être partie, réconcilier avant de rejouer quoi que ce soit.',
     'HA garde sa mémoire tant qu’il héberge les automatismes ou l’interface. La copie des états et descriptions Moli s’ajoute pendant cette phase.',
     'Les proxies ont de nouveaux identifiants. Les références HA ne sont pas réécrites automatiquement. RTS : état ouvert/fermé et pourcentage restent inconnus sans retour.'),
    ('sensors', '02', 'Capteurs, Zigbee et compteurs', 'read', 'Commencer par observer',
     'Import HA multicanal typé et pilote Zigbee2MQTT existants. La bascule physique des réseaux radio n’est pas qualifiée par le premier lot du pont.',
     'Pour MQTT : abonnement et chemins de commande. Pour une radio : coordinateur et réseau d’appairage ; ne pas changer de coordinateur comme une simple adresse de broker.',
     ['Compter température, humidité, batterie, contact et diagnostic séparément.', 'Conserver les unités, les états inconnus et les instants de dernière observation.', 'Garder le rang, le total et le reset des compteurs d’énergie ; éviter un double comptage.'],
     'Observer en parallèle sans effet. Si le même Zigbee2MQTT reste en place, conserver la radio et changer uniquement le propriétaire des commandes nécessaires ; une reconfiguration de radio est un autre lot.',
     'Rétablir le propriétaire précédent sur le même réseau conservé. Si la radio ou l’appairage a changé, prévoir sa restauration spécifique ; un snapshot MQTT ne recrée pas les appairages.',
     'Un capteur température/humidité reste un appareil à deux valeurs. Dupliquer l’observation ajoute un état, pas nécessairement un nouveau pilote radio. Les historiques restent un poste séparé.',
     'Recâbler les conditions et génériques vers les IDs des canaux ; un inconnu ne devient jamais une porte fermée, une présence absente ou une mesure à zéro.'),
    ('lights', '03', 'Lumières, prises et multiprises', 'action', 'Plan de migration à qualifier',
     'Des pilotes natifs existent notamment pour Hue, Tapo, Tuya et Zigbee. Les commandes du nouveau pont HA sont limitées aux volets dans ce premier lot.',
     'Chaque sortie réelle, avec ses groupes et son luminaire. Une multiprise conserve une sortie et un état par canal ; les groupes ne deviennent pas des appareils physiques supplémentaires.',
     ['Vérifier marche/arrêt, variation, couleur et température de couleur avec leurs plages.', 'Identifier les relais d’alimentation des luminaires et les groupes HA qui les recouvrent.', 'Séparer éclairage, prises d’équipements et mesures de puissance.'],
     'Préparer un lot simple puis qualifier les adaptateurs manquants. Transférer les sorties et leurs groupes cohérents après arrêt vérifié de l’ancien chemin. Ne pas déplacer automatiquement toutes les prises d’une pièce avec ses lumières.',
     'Fermer les écritures du nouveau pilote avant de rétablir l’ancien. Reconstituer groupes et références d’origine ; l’état électrique après une action réelle nécessite une observation, pas une restauration de JSON.',
     'Descriptions de groupes et membres peuvent être partagées ; états par sortie restent distincts. Les agrégats ne doivent pas recopier toutes les mesures des membres en permanence.',
     'Scènes, commandes de pièce et temporisations dépendent des mêmes membres. Migrer les références et vérifier que l’on n’additionne pas un groupe dérivé avec ses propres enfants.'),
    ('heat', '04', 'Chauffage et chauffe-eau', 'action', 'Ariston / Voltalis : contrat à comparer',
     'Les pilotes Ariston et Voltalis existent. Le premier lot HA ne qualifie pas leurs commandes ni une migration complète de thermostats climate.',
     'L’appareil, les programmations et les services cloud du compte concerné. Laisser une seule autorité pour les changements de mode et la rotation des sessions quand elle est exclusive.',
     ['Comparer mode, consigne, température mesurée et activité réelle de chauffe.', 'Conserver les listes operation_list / hvac_modes comme catalogues, leurs sélections comme codes.', 'Relever horaires, fuseau, plages et programmes déjà stockés dans l’appareil.'],
     'Observer un cycle complet avant toute reprise d’action. Qualifier les modes et paramètres sur le modèle exact, puis transférer le plan avec maintien des programmes attendus ; ne pas traiter « mode heat » comme une preuve de chauffe active.',
     'Stopper la nouvelle planification avant de rendre l’autorité. Restaurer les programmes et consignes validés puis constater l’état ; un retour logiciel n’annule pas l’eau déjà chauffée ou l’énergie consommée.',
     'Le catalogue de modes est froid ; le mode courant est un code. Les programmes, calendriers, sessions et statistiques ont une mémoire propre. HA reste nécessaire tant qu’il exécute leur programmation.',
     'Réconcilier les créneaux en cours et délais restants. Aucun programme HA et Moli ne doit réappliquer alternativement deux consignes opposées.'),
    ('weather', '05', 'Météo, soleil et aides de maison', 'read', 'Stabiliser les dépendances d’abord',
     'Météo et calculs horaires existent dans plusieurs briques ; l’équivalence des informations exactes doit être vérifiée pour chaque workflow.',
     'Source de données, position de référence, calendrier, fuseau et helpers persistants ; les minuteries et compteurs ne sont pas de simples capteurs.',
     ['Comparer météo actuelle, prévisions, lever/coucher et aube/crépuscule séparément.', 'Vérifier fuseau, changement d’heure et décalages avant/après le soleil.', 'Inventorier input_boolean, sélecteurs, compteurs, minuteries et leurs valeurs de démarrage.'],
     'Comparer les sources en lecture. Transférer ensuite les helpers avec leur état métier et leurs échéances, puis les workflows qui en dépendent. Garder un helper HA partagé tant que ses appelants n’ont pas tous changé.',
     'Rétablir la source et la bonne valeur du helper, sans réémettre un ancien événement. Pour un minuteur, définir explicitement annuler ou reprendre l’échéance observée.',
     'Prévisions et calendriers sont des tables froides bornées ; états courants et minuteurs sont différents. Une copie retained ne remplace pas leur fichier métier ni leurs conditions de reprise.',
     'Migrer ces dépendances avant un scénario de lever du soleil. Un événement horaire déjà consommé ne doit pas être redéclenché après bascule.'),
    ('media', '06', 'Apple TV, applications et musique', 'media', 'Conserver HA tant que l’équivalent manque',
     'Catalogues et codes d’applications sont étudiés ; des POC musique et des pilotes média existent. Aucune parité complète Apple TV / Music Assistant ni migration média réelle n’est acquise.',
     'Lecteur, session du fournisseur, file musicale, groupes et autorité de lecture ; la diffusion audio n’est pas un simple booléen marche/arrêt.',
     ['Comparer app_id / libellé d’application, volume, source et état du lecteur.', 'Relever morceau, artiste, durée, progression et queue sans supposer qu’un titre est une enum.', 'Identifier Cast, AirPlay et autres sorties réellement compatibles, ainsi que les sessions autorisées.'],
     'Conserver l’exécution HA ou Music Assistant pour les chemins non prouvés. N’exposer que les capacités vérifiées ; migrer lecture, groupes et commandes seulement après une comparaison fonctionnelle sur les lecteurs exacts.',
     'Rendre la main à l’ancien moteur après arrêt des nouveaux ordres. La reprise d’une queue, d’un compte ou d’une progression nécessite un format compatible ; ne pas promettre de reprendre un flux au même échantillon.',
     'Applications et sources : table froide + code. Titres libres : stockage texte borné à dimensionner. Buffers audio, décodage et caches sont un module séparé, hors du petit état courant.',
     'Les annonces et scènes média doivent garder une destination stable. Un accusé réseau ne prouve pas qu’une sortie diffuse effectivement du son.'),
    ('security', '07', 'Serrures, alarmes et accès', 'sensitive', 'Derniers lots, qualification dédiée',
     'Des briques de sécurité existent avec des états de livraison distincts. Le premier lot du pont ne transporte pas de commandes d’alarme, code ou PIN.',
     'Autorité d’accès, profils, approbations, calendrier et matériel ; ce périmètre ne se réduit pas au nom d’une entité.',
     ['Vérifier les gardes, heures calmes et pièces protégées.', 'Conserver les secrets dans les coffres autorisés, jamais dans un catalogue ou ce guide.', 'Distinguer armement, désarmement, déverrouillage et simple état de porte.'],
     'Garder l’ancien système actif pour les fonctions non qualifiées. Préparer un plan et un essai spécifique validés ; ne pas utiliser une serrure ou une alarme comme premier test du pont.',
     'Fermer la nouvelle autorité avant le retour. Réconcilier accès et sessions réellement valides ; ne jamais rejouer un déverrouillage ou un désarmement incertain.',
     'Permissions, fenêtres d’accès et reçus d’action ont un coût distinct des états. Un secours distant ne doit pas recevoir par défaut les secrets locaux ou la clé maître.',
     'Une approbation de migration n’approuve pas toutes les futures actions. Les identités HA externes ne deviennent pas automatiquement des utilisateurs humains Moli.'),
    ('data', '08', 'Historiques, énergie et interfaces', 'read', 'Préserver la continuité des références',
     'Historique, énergie et interfaces ont leurs propres formats. Les états MQTT et le registre de migration ne constituent pas un import historique complet.',
     'Bases, rangs, unités, identités historiques, dashboards, widgets, assistants vocaux et clients mobiles.',
     ['Relever totaux, remises à zéro et période couverte ; contrôler les unités.', 'Vérifier toutes les références entity_id, device_id, scènes, raccourcis et notifications.', 'Prévoir une archive lisible pendant la coexistence et une restauration d’essai.'],
     'Conserver l’archive HA, valider le format d’import et la liaison vers les IDs Moli. Basculer les vues et clients par lot après les commandes ; le nouveau proxy HA n’a pas automatiquement l’ancien entity_id.',
     'Revenir aux vues et références d’origine sans fusion aveugle de deux bases divergentes. Restaurer une base peut effacer les nouveaux enregistrements ; prévoir export et rapprochement.',
     'Retirer un affichage ne libère pas la base HA. Stockage et caches disque restent présents jusqu’à une décision explicite d’archivage et de purge.',
     'Une scène ou un dashboard encore lié à un ancien ID est une dépendance active. Une simple opération de renommage ne traite pas les références dynamiques.'),
]


def esc(value):
    return html.escape(str(value))


def component_cards():
    cards=[]
    for identity, index, title, category, badge, current, scope, checks, transfer, rollback, memory, workflows in COMPONENTS:
        cards.append(f'''<details class="component" id="composant-{identity}" data-family="{category}">
<summary><span class="component-index">{index}</span><span><strong>{esc(title)}</strong><span class="component-caption">{esc(badge)}</span></span><span class="plus" aria-hidden="true">+</span></summary>
<div class="component-body"><p class="component-state"><b>État actuel.</b> {esc(current)}</p>
<div class="component-grid"><div><h4>Définir le lot</h4><p>{esc(scope)}</p><h4>Comparer avant</h4><ul>{''.join('<li>'+esc(c)+'</li>' for c in checks)}</ul></div>
<div><h4>Parcours cible</h4><p>{esc(transfer)}</p><h4>Retour arrière</h4><p>{esc(rollback)}</p></div></div>
<div class="impact-row"><div><span>Mémoire</span><p>{esc(memory)}</p></div><div><span>Workflows et références</span><p>{esc(workflows)}</p></div></div></div></details>''')
    return '\n'.join(cards)


def main():
    measurements=json.loads((BASE/'mesures.json').read_text())
    local=json.loads((BASE/'ha-multimaison-pont-local.json').read_text())
    bridge=json.loads((BASE/'pont-ha-moli-tests.json').read_text())
    if bridge['status']!='PASS' or local['status']!='PASS':
        raise SystemExit('Les preuves de référence doivent être réussies.')
    references=['docs/proposition/analyse/v6/mesures.json',
                'docs/proposition/analyse/v6/ha-multimaison-pont-local.json',
                'docs/proposition/analyse/v6/pont-ha-moli-tests.json',
                'crates/moli-migration/src/model.rs', 'crates/moli-os/src/backup.rs',
                'crates/moli-automation/src/model.rs', 'crates/moli-assistant/src/import.rs']
    # This guide is explanatory. The receipt binds its dated source material;
    # it does not claim a new hardware or memory benchmark.
    recorded={p:hashlib.sha256((ROOT/p).read_bytes()).hexdigest() for p in references}
    memory_rows=[]
    for node,label in [('node-a','Instance A'),('node-b','Instance B')]:
        metrics=local['resources'][node]
        current=local['cases'][-1]['recovered'][node]
        memory_rows.append({'label':label,'live_mib':current['allocator_live_bytes']/1048576,
                            'rss_peak_mib':metrics['rss_peak_sampled_bytes']/1048576,
                            'cpu':metrics['cpu_percent_one_core']})
    payload={'references':recorded,'memory':memory_rows,
             'enum_bytes':measurements['enumerations']['measured_requested_bytes'],
             'rust_tests':sum(x['passed'] for x in bridge['rust_packages'].values()),
             'ha_framework':bridge['home_assistant_version']}
    template=(ROOT/'tools/home-assistant/migration-guide.template.html').read_text()
    template=template.replace('@@COMPONENTS@@',component_cards())
    template=template.replace('@@GUIDE_DATA@@',json.dumps(payload,ensure_ascii=False).replace('</','<\\/'))
    OUT.write_text(template)
    print('Guide HTML de migration rendu :',OUT.relative_to(ROOT))


if __name__=='__main__':
    main()
