#!/usr/bin/env python3
"""Rapport V5 déterministe, à partir des seules campagnes conservées."""
import html,json,re,statistics
from pathlib import Path
HERE=Path(__file__).resolve().parent; ROOT=HERE.parents[1]; OUT=ROOT/'docs/proposition/analyse'
I=json.loads((HERE/'sorties/campagnes-v5.json').read_text())
D={k:ROOT/v for k,v in I['experiments'].items()}
E=json.loads((D['moteur-integre-banc']/'resultats.json').read_text())
W=json.loads((D['fenetres-temporelles-banc']/'resultats.json').read_text())
P=json.loads((D['fenetres-temporelles-banc']/'production.json').read_text())
C=json.loads((HERE/'sorties/workspace-check.json').read_text())
assert C['exit_code']==0,'Le garde-fou complet doit avoir terminé avec succès'
log=(HERE/'sorties/workspace-check.log').read_text()
tests=sum(map(int,re.findall(r'test result: ok\. (\d+) passed;',log)))
assert tests and 'leak-check : propre' in log
def esc(x):return html.escape(str(x))
def link(p,label):return '<a href="../../../'+esc(p)+'">'+esc(label)+'</a>'
def table(head,rows):
 return '<div class="scroll"><table><thead><tr>'+''.join('<th>'+esc(v)+'</th>' for v in head)+'</tr></thead><tbody>'+''.join('<tr>'+''.join('<td>'+esc(v)+'</td>' for v in row)+'</tr>' for row in rows)+'</tbody></table></div>'
def mib(n):return f'{n/1048576:.3f}'
def engine(n,l,m):return next(r for r in E['groups'] if (r['source_devices'],r['layout'],r['measurement_mode'])==(n,l,m))
M={};B={}
for repeat in (1,2,3):
 for r in map(json.loads,(D['transformations-banc']/f'mesures-{repeat}.jsonl').read_text().splitlines()):
  if r['type']=='measurement':M.setdefault((r['name'],r['devices']),[]).append(r)
 for r in map(json.loads,(D['transformations-banc']/f'memoire-index-{repeat}.jsonl').read_text().splitlines()):B[r['devices']]=r
def ns(name,n=1):return statistics.median(r['median_sample_mean_ns_per_operation'] for r in M[(name,n)])
S=[]
def sec(key,title,body):S.append((key,title,'<section id="'+key+'"><div class="eyebrow">Moteur et mémoire / V5</div><h2>'+title+'</h2>'+body+'</section>'))

sec('portee','Le vrai moteur, avec un périmètre vérifiable.',f'''<p class="lead">Cette V5 ajoute le hub de production, ses calculs, ses automatismes, ses commandes et les codecs réels aux microbancs V2–V4. Données synthétiques et pilote en mémoire : aucune commande physique.</p><div class="stats"><div><b>{tests}</b><span>tests du dépôt réussis</span></div><div><b>{E['runs']}</b><span>processus moteur contrôlés</span></div><div><b>36 + 3</b><span>fenêtres proposées + API temporelles</span></div><div><b>3 × {len(M)}</b><span>mesures de transformations</span></div></div><p><b>Les anciens essais ne couvraient pas le moteur complet ni les fenêtres glissantes.</b> La suite complète du dépôt est maintenant exécutée et plusieurs chemins réels sont mesurés. Cela ne qualifie pas tous les matériels ni une future réécriture du stockage.</p>'''+table(['Surface','Exercice actuel','Limite'],[
('Dépôt complet','Format, Clippy strict, tests de toutes les crates, catalogue 44 paquets, contrôle de fuites.','Six avertissements de fixtures restent au catalogue ; qualification de tous les appareils non prouvée.'),
('Hub, bus et calculs','Chaîne mean→max, fraîcheur monotone, péremption sans entrée, observation identique renouvelée.','3 calculs et 1 automatisme ; pas toutes les topologies jusqu’aux limites du registre.'),
('Commandes','Brouillon inactif, approbation, garde setter/action, lecture seule, types, journal, erreur, délai sans effet tardif, occurrences distinctes.','Pilote en mémoire. HTTP/MCP sont testés par la suite du dépôt ; leurs sockets ne sont pas chronométrées ici.'),
('Transformations','Vrais Catalog/Entry Z2M, profils compilés et validateurs ; parsing JSON inclus dans les chemins complets.','Broker MQTT et commande physique exclus.'),
('Temps','Vrais History::series, rollup::hourly/group, AggregateSpec, DurationLedger.','Contrats de fonctions ; reprise du ledger sans crash ni preuve fsync.'),
('1 min / 5 min / 1 h / 24 h','12 tests sémantiques et 36 processus de comparaison.','Prototypes distincts des points calculés actuellement raccordés au hub.'),
]))

rows=[]
for n in (206,290,496,2060,2900,4960):
 for l in ('uniform96','compact64','columns'):
  r=engine(n,l,'counted')
  rows.append([n,l,mib(r['adapter_payload_and_handles_bytes']['median']),mib(r['final_live_requested_bytes']['median']),mib(r['steady_anonymous_bytes']['median']),mib(r['process_peak_rss_bytes']['median'])])
sec('memoire','Mémoire : adaptateur, tas et pages du vrai processus.',table(['Appareils sources','Adaptateur','Buffers adaptateur, Mio','Tas vivant demandé, Mio','Pages anonymes, Mio','Pic RSS, Mio'],rows)+'''<p>Quatre points par appareil synthétique, plus trois appareils calculés et leurs trois points. Les nombres issus des deux inventaires dimensionnent la fixture ; leurs descriptions, états et attributs complets ne sont pas rejoués ici.</p><p><b>Les trois layouts sont des adaptateurs devant le même Hub.</b> Le hub garde encore son modèle typé réel : le tableau ne prouve pas les performances d’un futur stockage interne en 64 ou 96 octets. Les champs numériques occupent un quart des lignes de l’adaptateur.</p><p>Le tas demandé exclut la fragmentation et les en-têtes internes de l’allocateur. Les pages anonymes et le RSS sont lus dans Linux ; RSS inclut aussi code/fichiers. Le pic RSS vient de VmHWM depuis le démarrage ; le pic anonyme est seulement échantillonné aux fins de lots. Plafond conteneur : 512 Mio. Le budget produit de 32 Mio pour toutes les briques n’est pas qualifié par cette fixture.</p>''')

rows=[]
for n in (496,4960):
 for l in ('uniform96','compact64','columns'):
  r=engine(n,l,'timing_plain');t=r['raw_observations_per_second']
  rows.append([n,l,f"{t['median']/1000:.1f}",f"{t['min']/1000:.1f}–{t['max']/1000:.1f}",f"{r['propagation_and_automation_latency_ns.p95']['median']/1e6:.3f}",f"{r['setter_round_trip_latency_ns.p95']['median']/1000:.2f}",f"{r['action_round_trip_latency_ns.p95']['median']/1000:.2f}"])
sec('latences','Les intervalles empêchent un classement CPU hâtif.',table(['Appareils','Adaptateur','Milliers de maj numériques/s','Min–max des 3 processus','Propagation + automate p95, ms','Setter p95, µs','Action p95, µs'],rows)+'''<p>Temps du binaire timing_plain, sans compteurs atomiques d’allocation ; mémoire du binaire counted de même source. Les zéros des compteurs en timing_plain signifient « non mesurés ». Chaque p95 affiché est la médiane de trois p95 empiriques de dix lots/commandes, pas une garantie de latence en production.</p><p>Le débit compte seulement les observations numériques vérifiées, appareils × dix rondes. La ronde contient aussi un état déclencheur, puis les calculs, l’automatisme et les commandes ; ils participent au temps sans gonfler le compteur numérique. La propagation est attendue par événements, sans polling dans la boucle mesurée.</p><p><b>La dispersion est forte et les intervalles se recouvrent.</b> Ces courtes boucles ne prouvent pas une supériorité CPU universelle de 96, 64 ou des colonnes. Les différences de buffers sont déterministes ; le classement des temps reste sensible à l’hôte et à l’ordonnancement.</p>''')

rows=[]
for n in (1,100,1000):
 a=ns('action_api_json_lookup_validate_encode',n);b=ns('experimental_action_api_json_index_validate_encode',n)
 rows.append([n,f"{ns('mqtt_bytes_route_decode',n)/1000:.3f}",f'{a/1000:.3f}',f'{b/1000:.3f}',f'{a/b:.2f}×',f"{B[n]['requested_live_heap_bytes']/1024:.2f}",f"{ns('experimental_action_index_rebuild',n)/1000:.2f}"])
sec('transformations','Valeurs et actions : le routage compte aussi.',table(['Appareils','Entrée JSON + 8 valeurs, µs','Action JSON/recherche actuelle/codec, µs','Même action avec index proposé, µs','Rapport local','Index ajouté, Kio','Construction + destruction index, µs'],rows)+f'''<p>Les profils sont <b>déjà compilés</b> en CompiledPoint/CompiledWrite. Parse JSON + cinq lectures : {ns('profile_json_parse_and_compiled_read')/1000:.3f} µs ; demande JSON + validation + rendu d’action : {ns('profile_action_api_json_validate_render')/1000:.3f} µs sur cette fixture. Recompiler le TOML à chaque message est une référence volontairement naïve, pas le produit actuel.</p><p>Le vrai Catalog::by_id parcourt les appareils. Le banc compare ce chemin à un index emprunté ID→Entry, en conservant les mêmes validations et le vrai encodage ; tests et checksums vérifient les résultats et erreurs. Le gain mesuré ne représente pas une latence physique ni un changement produit livré. L’index devra être reconstruit avec le catalogue. Sa mémoire est comptée dans un binaire séparé, sans biaiser les temps.</p><pre>Entrée MQTT / réponse API
  → décodage borné → identité native → liaison compilée → codec de lecture
  → valeur canonique : type, unité, dates, qualité

Setter ou action autonome
  → validation du contrat → hub / garde → codec d’écriture → payload natif
  → retour observé pour confirmer, distinct de la demande</pre><p>Lecture et écriture sont deux contrats. Garder domaines natif/canonique, min/max, unité, échelle, inversion, pas/arrondi et politique hors bornes. Une unité seule ne définit ni l’inversion ni le sens de 100 %. Le prototype affine pourcentage↔0..254 vérifie ces choix ; il n’est pas ajouté implicitement au pilote.</p><p>Proposition : des plans déterministes partagés dans une table versionnée, avec référence compacte depuis valeur/action. Identités durables distinctes des noms/topics et caches ; nouveau schéma ou codec → invalidation. Aucun GPT/LLM dans le chemin de contrôle. La dictée est comprise provisoirement comme « MQTT et API ».</p>''')

sec('moyennes','Trois sens de moyenne, testés séparément.',table(['Question','Résultat','Sens'],[
('Moyenne entre sources','AggregateSpec::Mean, fraîcheur et conversions d’unités.','Le max_age_s retire les observations périmées ; ce n’est pas une fenêtre d’historique.'),
('Moyenne des changements enregistrés',P['recorded_change_mean'],'Deux changements 0 puis 100 : History AVG porte sur les lignes.'),
('Moyenne pondérée par la durée',P['duration_weighted_mean'],'0 pendant 54 min, 100 pendant 6 min : vraie fonction hourly.'),
('Moyenne de tous les relevés','33,33 pour [0, 0, 100]','Exemple arithmétique ; l’historique construit ne contient que les changements. Impossible d’en déduire tous les polls.'),
('24 h glissantes','24 heures écoulées.','Prototype : 1 min, 5 min, 1 h, 24 h.'),
('Jour civil local','23 h / 25 h vérifiées.','Vrai hourly/group en Europe/Paris, pondéré par les durées.'),
])+'''<p><b>Les fenêtres glissantes proposées ne sont pas livrées comme points calculés actifs.</b> Rollup exige actuellement des heures UTC entières ; il ne fournit pas à lui seul une fenêtre arbitraire de cinq minutes. Le prototype et les fonctions de production sont distingués.</p><p>Le prototype emploie des fenêtres demi-ouvertes et remplace un doublon au même instant. History::series a une borne haute inclusive ; rollup compte chaque ligne et la dernière valeur tient jusqu’à la suivante. Ces différences sont testées. Une donnée absente ou périmée laisse du temps inconnu ; zéro est une vraie contribution.</p><p>Une moyenne de zone sur une période doit préciser le poids : appareil, relevé ou durée couverte. Moyenner deux moyennes sans leur couverture peut changer la réponse. Familles, maisons, étages, pièces et zones restent dans la V4 ; la fenêtre temporelle est une dimension distincte.</p>''')

rows=[]
for r in W:
 errors=[w['time_abs_error'] for w in r['windows'] if w['time_abs_error'] is not None]
 rows.append([r['workload'],r['backend'],f"{r['heap_capacity_bytes']/1024:.1f}",f"{r['ingest_ns_per_event_median']:.2f}",f"{r['query_4windows_ns_median']/1000:.2f}",f'{max(errors):.6g}',max(abs(w['covered_ms']-w['oracle_covered_ms']) for w in r['windows']),'oui' if any(w['limited'] for w in r['windows']) else 'non'])
sec('fenetres','Mémoire bornée, perte et approximation sont visibles.',table(['Charge','Structure','Buffers Kio/point','ns/entrée','µs/requête 4 fenêtres','Erreur max moyenne temporelle','Écart couverture, ms','Historique tronqué'],rows)+'''<p>Deque exacte, anneau de 65 536 échantillons, anneau de 2 048 et seaux de 10 s. Trois processus par cas, chacun avec chauffe et sept passages. Un seul historique partagé par les quatre fenêtres. Requêtes chaudes après ingestion, sans écrivain concurrent.</p><p>Mémoire = capacité du buffer × layout, sans en-têtes de pile ni fragmentation/RSS du moteur complet. Sparse couvre un jour à 30 s ; irregular couvre un jour à intervalles variables ; burst contient 100 000 événements sur environ 2 h 47, pas un jour complet de rafale. Le temps simulé part de zéro : une fenêtre plus ancienne est tronquée à cette époque et la durée inconnue est comptée sur ce domaine effectif.</p><p>Une moyenne de l’anneau tronqué ne doit pas être présentée comme complète : limited=true le signale. Les seaux peuvent changer moyenne et couverture aux bords. L’erreur observée n’est pas une borne maximale garantie ; un test ciblé de frontière fournit un contre-exemple même quand l’erreur du tableau est presque nulle.</p>''')

rows=[]
for backend in ('deque','ring','ring_small','buckets_10s'):
 r=next(r for r in W if r['backend']==backend and r['workload']=='sparse');n=r['heap_capacity_bytes']
 rows.append([backend,f'{n/1024:.1f}',mib(n*100),mib(n*1000),'dépend du trafic' if backend=='deque' else 'réserve fixe du modèle'])
sec('budget','Les historiques dépassent vite les lignes canoniques.',table(['Fixture sparse','Kio par point','100 points, Mio','1 000 points, Mio','Nature'],rows)+'''<p>Extrapolations arithmétiques de buffers sur des nombres hypothétiques de points activés, pas RAM mesurée d’une maison. Quatre résultats canoniques de 96 octets coûtent 384 octets de lignes ; historique et index coûtent à côté. Ne pas multiplier l’historique par quatre ni activer une grosse réserve sur chaque valeur.</p><p>Direction proposée : fenêtres sur les points nécessaires, historique partagé par source, persistance/résumés existants ou prévus par D13, résultats compacts en RAM. Un mélange résumés + bords exacts est une expérience suivante, pas un algorithme qualifié ici. Le raccordement doit réutiliser l’ordonnanceur borné et la validation existants.</p>''')

body='<p>'+link('tools/moteur-suite-banc/README.md','Procédure de reproduction')+' · '+link(I['campaign']+'/campagne.json','Empreintes, environnement et archive des sources')+' · '+link('tools/moteur-suite-banc/sorties/workspace-check.log','Journal du garde-fou complet')+'</p><ul>'
for name in D:body+='<li>'+link('tools/'+name+'/README.md',name+' : contrat')+' · '+link(I['experiments'][name]+'/resultats.json' if name!='transformations-banc' else I['experiments'][name]+'/mesures-1.jsonl','résultats')+'</li>'
body+='''</ul><p>Sources, lock, commandes et sorties sont conservés en campagnes séparées ; les précédentes restent intactes. Contrôle complet sur copie des fichiers publiables : les pièces jointes ignorées et binaires locaux en sont exclus. Une liaison de compilation a dépassé 3 Gio ; le contrôle final emploie 6 Gio et un job, indépendamment des essais moteur à 512 Mio.</p><p>Les tests du dépôt et les scénarios réels passent. L’index des actions est une piste quantifiée, les profils sont déjà compilés, et les fenêtres exigent un contrat de couverture et un budget propre. Le stockage interne futur et les nouvelles fenêtres actives restent à intégrer avant de mesurer leur implémentation complète.</p>'''
sec('preuves','Des preuves séparées, conservées pour évoluer.',body)
css=re.search(r'<style>(.*?)</style>',(OUT/'catalogue-memoire-v4.html').read_text(),re.S).group(1)+'\nth{text-transform:none}\n'
nav=''.join('<a href="#'+key+'">'+esc(title)+'</a>' for key,title,_ in S)
page='<!doctype html><html lang="fr"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Moli — V5 : moteur réel, transformations et moyennes temporelles</title><style>'+css+'</style></head><body><aside><b>Moli / V5</b><p>Moteur réel<br>Transformations et temps</p><nav aria-label="Sections">'+nav+'</nav></aside><main><header><span class="badge">MESURES — PROPOSITIONS IDENTIFIÉES</span><h1>Le moteur à l’épreuve.<br>La mémoire avec ses coûts.</h1><p class="lead">Valeurs, actions, conversions et quatre fenêtres temporelles, avec des campagnes reproductibles.</p></header>'+''.join(body for _,_,body in S)+'<footer>V5 · données synthétiques · aucune commande physique · <a href="catalogue-memoire-v4.html">V4</a></footer></main></body></html>'
(OUT/'catalogue-memoire-v5.html').write_text(page)
print(json.dumps(dict(workspace_tests=tests,engine_runs=E['runs'],transformation_cases=len(M),temporal_cases=len(W),sections=len(S)),ensure_ascii=False))
