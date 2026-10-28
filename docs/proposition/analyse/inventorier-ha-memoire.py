#!/usr/bin/env python3
"""Analyse hors ligne : seuls agrégats anonymes et empreintes sortent du dossier privé.
Usage: python3 inventorier-ha-memoire.py <racine-export-prive> [sortie-publique]
Aucun appel réseau, aucun service exécuté, aucun identifiant exporté.
"""
import collections, hashlib, html, json, math, pathlib, re, sys
ROOT=pathlib.Path(__file__).resolve().parents[3]
OUT=pathlib.Path(sys.argv[2]) if len(sys.argv)>2 else pathlib.Path(__file__).resolve().parent
PRIVATE=pathlib.Path(sys.argv[1])
STANDARD={'sensor','binary_sensor','light','switch','climate','cover','media_player','number','button','select','input_boolean','input_number','input_button','input_select','automation','script','device_tracker','person','lock','vacuum','alarm_control_panel','timer','sun','weather','fan','humidifier','water_heater','update','camera','event','remote','notify','siren','scene','text','input_text','date','time','datetime','calendar','zone','image'}
BINARY={'binary_sensor','light','switch','input_boolean','automation','fan','siren','remote'}
ENUM={'climate','cover','media_player','person','device_tracker','lock','vacuum','alarm_control_panel','timer','sun','weather','update','select','input_select','humidifier','water_heater'}
def packed(d):return json.dumps(d,ensure_ascii=False,separators=(',',':')).encode()
def byt(n):return f'{n:,} o · {n/1048576:.3f} Mio'.replace(',',' ')
def table(h,rows):
 return '<div class="scroll"><table><thead><tr>'+''.join('<th>'+html.escape(x)+'</th>' for x in h)+'</tr></thead><tbody>'+''.join('<tr>'+''.join('<td>'+html.escape(str(x))+'</td>' for x in r)+'</tr>' for r in rows)+'</tbody></table></div>'
def state_kind(state,domain):
 if state in ('unknown','unavailable') or state is None:return 'unknown'
 if domain in BINARY and state in ('on','off'):return 'boolean'
 if isinstance(state,bool):return 'boolean'
 if isinstance(state,(int,float)) or isinstance(state,str) and re.fullmatch(r'[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?',state):
  try:
   if not math.isfinite(float(state)):return 'text'
   if isinstance(state,int) or isinstance(state,str) and re.fullmatch(r'[+-]?\d+',state):return 'integer' if -(2**63)<=int(state)<2**63 else 'out_of_i64'
   return 'float'
  except (ValueError,OverflowError):return 'text'
 if domain in ENUM:return 'enum_candidate'
 return 'text'
def analyse(folder,label):
 hashes={}
 def load(group,file):
  raw=(folder/'sanitized'/group/(file+'.json')).read_bytes();hashes[file]=hashlib.sha256(raw).hexdigest();return json.loads(raw)
 states=load('runtime','states'); services=load('runtime','services')
 devices=load('registries','core.device_registry')['devices'];entities=load('registries','core.entity_registry')['entities']
 reg={x['entity_id']:x for x in entities}
 # Uniquement des catégories et des longueurs sortent de cette fonction.
 domain_counts=collections.Counter(); kinds=collections.Counter(); nodes=collections.Counter(); keys=set(); enum_codes=set(); strings=set();value_payload=0;all_payload=0;edges=0
 percent_entities=0;bounds=0;numeric=0;typed_attribute_numeric=0
 def walk(v):
  nonlocal edges,all_payload,typed_attribute_numeric
  if isinstance(v,dict):
   nodes['object']+=1;edges+=len(v)
   for key,item in v.items():keys.add(key);walk(item)
  elif isinstance(v,list):
   nodes['list']+=1;edges+=len(v)
   for item in v:walk(item)
  elif v is None:nodes['null']+=1
  elif isinstance(v,bool):nodes['boolean']+=1
  elif isinstance(v,int):nodes['integer']+=1;typed_attribute_numeric+=1
  elif isinstance(v,float):nodes['float']+=1;typed_attribute_numeric+=1
  elif isinstance(v,str):nodes['text']+=1;all_payload+=len(v.encode());strings.add(v)
  else:raise TypeError('non JSON')
 for row in states:
  domain=row['entity_id'].split('.')[0];domain_counts[domain if domain in STANDARD else 'other_domains']+=1
  kind=state_kind(row['state'],domain);kinds[kind]+=1
  if kind in ('integer','float'):numeric+=1
  if kind in ('text','out_of_i64'):value_payload+=len(str(row['state']).encode());strings.add(str(row['state']))
  if kind=='enum_candidate':enum_codes.add((domain,str(row['state'])))
  attrs=row.get('attributes',{});walk(attrs)
  if attrs.get('unit_of_measurement')=='%':percent_entities+=1
  if any(k in attrs for k in ['min','max','min_temp','max_temp','min_value','max_value']):bounds+=1
 service_count=0;valid=0;targeted=0;field_count=0;no_fields=0;schema_bytes=0;fields_bytes=0;service_labels=0;service_by_domain=collections.Counter();field_selector_counts=collections.Counter()
 for group in services:
  domain=group['domain']; bucket=domain if domain in STANDARD or domain in {'homeassistant','recorder','history','logbook','system_log','persistent_notification','frontend','config','backup','conversation','tts','stt','wake_word','mqtt','hassio'} else 'other_domains'
  for name,spec in group['services'].items():
   service_count+=1;service_by_domain[bucket]+=1;service_labels+=len((domain+'.'+name).encode())
   if not isinstance(spec,dict):continue
   valid+=1;targeted+='target' in spec
   fields=spec.get('fields',{})
   if isinstance(fields,dict):
    field_count+=len(fields);no_fields+=not fields;fields_bytes+=len(packed(fields))
    for field in fields.values():
     if isinstance(field,dict):
      selectors=field.get('selector',{})
      if isinstance(selectors,dict):
       for selector in selectors:field_selector_counts[selector if selector in {'number','text','boolean','select','entity','device','area','target','object','action','color_rgb','color_temp','duration','date','time','datetime','template','file','media','location'} else 'other_selectors']+=1
   schema_bytes+=len(packed(spec))
 identity_bytes=sum(len(x['entity_id'].encode()) for x in states)+sum(len(str(x['id']).encode()) for x in devices)+service_labels
 labels=sum(len(str(x.get('attributes',{}).get('friendly_name','')).encode()) for x in states)+sum(len(str(x.get('name_by_user') or x.get('name') or '').encode()) for x in devices)
 container_count=nodes['object']+nodes['list']
 structured_headers=16*container_count+8*edges
 enum_bytes=sum(len(v.encode()) for _,v in enum_codes)
 key_bytes=sum(len(k.encode()) for k in keys)
 v=len(states);a=service_count;e=len(devices);n=v+a+e
 # Modèle de charge déterministe. Ce n'est ni un RSS ni une représentation Rust complète.
 base_aux=identity_bytes+labels+12*n+schema_bytes+32*numeric+32*percent_entities+enum_bytes+value_payload
 full_nodes=sum(nodes.values());full_n=n+full_nodes
 full_aux=base_aux+all_payload+key_bytes+structured_headers+12*full_nodes+32*typed_attribute_numeric
 return dict(label=label,source_hashes=hashes,active_entities=v,registered_entities=len(entities),inactive_or_absent_from_states=sum(x['entity_id'] not in {s['entity_id'] for s in states} for x in entities),disabled_registry_entities=sum(x.get('disabled_by') is not None for x in entities),hidden_registry_entities=sum(x.get('hidden_by') is not None for x in entities),registered_devices=e,devices_with_active_entity=len({reg[s['entity_id']].get('device_id') for s in states if s['entity_id'] in reg and reg[s['entity_id']].get('device_id')}),active_without_registry_device=sum(not reg.get(s['entity_id'],{}).get('device_id') for s in states),services=a,services_with_readable_schema=valid,services_redacted_or_invalid=a-valid,services_with_target=targeted,services_without_explicit_target=valid-targeted,services_without_declared_fields=no_fields,top_level_service_fields=field_count,service_field_selector_counts=dict(sorted(field_selector_counts.items())),state_kinds=dict(sorted(kinds.items())),entities_by_domain=dict(sorted(domain_counts.items())),services_by_domain=dict(sorted(service_by_domain.items())),native_percent_entities=percent_entities,entities_exposing_any_bound=bounds,numeric_known_states=numeric,attribute_nodes=dict(sorted(nodes.items())),attribute_nodes_total=full_nodes,attribute_edges=edges,full_value_rows=v+full_nodes,base_rows=n,full_rows=full_n,bytes=dict(base64=64*n,base96=96*n,full64=64*full_n,full96=96*full_n,base64_plus_model=64*n+base_aux,base96_plus_model=96*n+base_aux-32*numeric,full64_plus_model=64*full_n+full_aux,full96_plus_model=96*full_n+full_aux-32*(numeric+typed_attribute_numeric)),payload_model=dict(identity_utf8=identity_bytes,labels_utf8=labels,observed_enum_labels_utf8=enum_bytes,state_text_utf8=value_payload,attribute_text_utf8=all_payload,attribute_keys_utf8=key_bytes,composite_headers_and_edges=structured_headers,service_contract_json_utf8=schema_bytes,top_level_field_json_utf8=fields_bytes,reference_directory_per_row=12,known_attribute_numbers=typed_attribute_numeric))
folders=sorted(p for p in PRIVATE.iterdir() if (p/'sanitized/runtime/states.json').exists() and (p/'sanitized/runtime/services.json').exists())
if len(folders)!=2:raise SystemExit('Il faut exactement deux exports HA identifiables.')
instances=[analyse(folder,'Instance '+chr(65+n)) for n,folder in enumerate(folders)]
result=dict(version=1,scope='instantanés privés hors ligne ; aucune interrogation en direct',instances=instances,notes=['Les noms, adresses, identifiants et valeurs ne sont pas exportés.','Les services sont des définitions partagées ; le nombre d’actions concrètes par équipement n’est pas connu.','La classification des états porte sur l’observation et le domaine ; elle ne prouve pas le type ou les capacités futurs.','Les mesures UTF-8 portent sur les fichiers déjà expurgés et peuvent sous-estimer les chaînes originales.'])
OUT.mkdir(parents=True,exist_ok=True)
(OUT/'inventaire-ha-memoire-v1.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
html_parts=[]
def add(s):html_parts.append(s)
add('<p class="lead">Les deux instantanés Home Assistant ont été analysés : états, attributs, registre des entités, registre des équipements et catalogue des services. Les chiffres ci-dessous remplacent l’hypothèse précédente « une action par valeur » pour ces exports.</p>')
add('<div class="notice"><b>Données exportées, pas état en direct.</b> Les exports privés retrouvés appartiennent au dossier d’étude constitué en octobre 2026. Ils sont déjà expurgés. Aucun identifiant, nom de personne, lieu, appareil, URL ou valeur n’est publié ; seules les statistiques et empreintes des fichiers sont conservées. Les chaînes mesurées peuvent être plus courtes que les originales.</div>')
add('<h3>1. Ce que contiennent réellement les deux exports</h3>')
rows=[]
metrics=[('États d’entités actifs','active_entities'),('Entités au registre, actives ou non','registered_entities'),('Registre sans état dans cet export','inactive_or_absent_from_states'),('Entités désactivées au registre','disabled_registry_entities'),('Équipements enregistrés','registered_devices'),('Équipements avec au moins une entité active liée','devices_with_active_entity'),('Entités actives sans équipement lié','active_without_registry_device'),('Définitions de services/actions','services'),('Schémas de service lisibles','services_with_readable_schema'),('Schémas expurgés/non objets : non analysables','services_redacted_or_invalid'),('Services avec cible déclarée','services_with_target'),('Champs de premier niveau dans les services','top_level_service_fields'),('Entités affichant l’unité %','native_percent_entities'),('Entités exposant au moins une borne reconnue','entities_exposing_any_bound')]
for label,key in metrics:rows.append([label,*[i[key] for i in instances],sum(i[key] for i in instances)])
add(table(['Mesure','Instance A','Instance B','Somme'],rows))
add('<p>Les 496 équipements précédemment utilisés par le banc sont retrouvés par la somme des deux registres. Une entité peut être virtuelle ou sans équipement lié : elle ne crée pas automatiquement un équipement fictif. Les entités sans état ne sont pas toutes désactivées ; registre, disponibilité et activité sont des notions distinctes.</p>')
add('<h3>2. Types des valeurs observées</h3>')
keys=sorted(set().union(*(i['state_kinds'] for i in instances)))
add(table(['Type observé','Instance A','Instance B','Somme'],[[k,*[i['state_kinds'].get(k,0) for i in instances],sum(i['state_kinds'].get(k,0) for i in instances)] for k in keys]))
add('<p>Les nombres sont reconnus strictement ; integer respecte i64 et float reste fini. boolean est une projection on/off seulement pour les domaines explicitement retenus. enum_candidate signifie état textuel d’un domaine connu : ce n’est pas la liste complète des valeurs admises. unknown regroupe unknown/unavailable ; un état inconnu ne prouve ni lecture seule, ni absence de commande, ni absence de type numérique.</p>')
add('<h3>3. Les actions : compter les contrats sans inventer les liaisons</h3>')
add('<p>Une entrée du catalogue de services devient ici <b>une définition d’action partagée</b>. light.turn_on n’est pas dupliquée pour chaque lumière : ses paramètres et sélecteurs appartiennent au contrat. Le catalogue seul ne dit pas combien d’actions concrètes chaque appareil accepte ; les supported_features, modes, cibles et schémas partiellement expurgés exigent une résolution complémentaire. Aucun service n’a été exécuté pour vérifier une capacité.</p>')
add(table(['Indicateur','A','B'],[[label,*[i[key] for i in instances]] for label,key in [('Sans cible explicite','services_without_explicit_target'),('Sans champ déclaré (cible éventuelle en plus)','services_without_declared_fields'),('Schémas illisibles réservés quand même en mémoire','services_redacted_or_invalid')]]))
add('<p>Sans cible déclarée ne veut pas dire sans effet ni sans paramètre : un service peut être global ou décrire une cible dans ses champs. Les services incluent des gestes, des réglages, de la lecture et des opérations d’administration ; les classer tous comme set serait faux. Les entités button et les scripts sont comptées dans les états existants, les services press/run dans les contrats ; aucun nouveau bouton fictif n’est ajouté.</p>')
domains=sorted(set().union(*(i['services_by_domain'] for i in instances)))
add(table(['Domaine public / autres regroupés','États A','Services A','États B','Services B'],[[d,instances[0]['entities_by_domain'].get(d,0),instances[0]['services_by_domain'].get(d,0),instances[1]['entities_by_domain'].get(d,0),instances[1]['services_by_domain'].get(d,0)] for d in domains]))
add('<h3>4. Coût des lignes : deux représentations clairement séparées</h3>')
add('<p><b>Vue principale :</b> V = un état par entité active, E = tous les équipements du registre, A = toutes les définitions de services, y compris les contrats illisibles réservés. N=V+E+A. Un service est partagé à l’intérieur d’une installation ; les contrats des deux installations ne sont pas dédupliqués entre eux.</p>')
add('<p><b>Vue développée :</b> on ajoute une fiche par nœud d’attribut JSON, y compris les objets/listes et leurs feuilles. Le record racine des attributs de chaque entité est compté. Cette représentation conserve toute la structure ; elle majore un modèle où les métadonnées seraient stockées uniquement dans des descripteurs partagés. Chaque feuille et conteneur n’est compté qu’une fois.</p>')
rows=[]
for i in instances:rows.append([i['label'],i['base_rows'],byt(i['bytes']['base64']),byt(i['bytes']['base96']),i['full_rows'],byt(i['bytes']['full64']),byt(i['bytes']['full96'])])
for factor in [1,10,100]:
 rows.append(['Somme ×'+str(factor),sum(i['base_rows'] for i in instances)*factor,byt(sum(i['bytes']['base64'] for i in instances)*factor),byt(sum(i['bytes']['base96'] for i in instances)*factor),sum(i['full_rows'] for i in instances)*factor,byt(sum(i['bytes']['full64'] for i in instances)*factor),byt(sum(i['bytes']['full96'] for i in instances)*factor)])
add(table(['Scénario','N principal','64 o principal','96 o principal','N développé','64 o développé','96 o développé'],rows))
add('<p>Ces produits sont exacts pour les nombres de fiches et layouts retenus. Ils ne sont ni le RSS des Home Assistant ni celui de Moli. Lire un ancien état ne donne pas le nombre de changements par seconde, le maximum de chaînes ni le nombre d’invocations simultanées.</p>')
add('<p>Vérification complémentaire sous Linux ARM64 : des tableaux de 3 678 puis 12 243 fiches ont été alloués par le banc Rust. Leurs contenus demandés correspondent exactement aux totaux 64/96 o ci-dessus. <a href="layout-deux-ha-linux.jsonl">Résultats bruts</a>. Il s’agit des octets de contenu des tableaux ; pas du RSS ni des payloads externes.</p>')
add('<h3>5. Que pèsent les attributs et les données annexes ?</h3>')
node_keys=sorted(set().union(*(i['attribute_nodes'] for i in instances)))
add(table(['Nœuds d’attributs','A','B','Somme'],[[k,*[i['attribute_nodes'].get(k,0) for i in instances],sum(i['attribute_nodes'].get(k,0) for i in instances)] for k in node_keys]))
add('<p>Une estimation supplémentaire réutilise les longueurs UTF-8 mesurées sur ces exports expurgés : identités et libellés, textes observés, codes enum candidats, clés d’attributs uniques par installation, contrats de service sous forme JSON compacte ; plus 12 o de répertoire par fiche et 16 o par en-tête composite + 8 o par lien enfant. En format 64, Numeric32 est ajouté pour chaque nombre effectivement observé ; les pourcentages ont une réserve de 32 o. En format 96, Numeric32 est déjà inline. Un état unknown d’un capteur numérique ne permet pas d’en déduire le descripteur : les extensions manquantes restent à inventorier.</p>')
rows=[]
for label,group,factor in [('Instance A',[instances[0]],1),('Instance B',[instances[1]],1),('Somme ×1',instances,1),('Somme ×10',instances,10)]:
 rows.append([label,*[byt(sum(i['bytes'][k] for i in group)*factor) for k in ['base64_plus_model','base96_plus_model','full64_plus_model','full96_plus_model']]])
add(table(['Scénario','64 o principal + modèle','96 o principal + modèle','64 o développé + modèle','96 o développé + modèle'],rows))
add('<p><b>Modèle partiel, non borne maximale :</b> pas de table de hachage réelle mesurée, pas de padding/allocateur, pas de réserve de capacité, pas d’historique, d’outbox, de fichiers/images, de moteur de workflows ni de file d’invocations. Les attributs ne sont pas inclus dans la vue principale ; ils le sont dans la vue développée. Les contrats JSON servent ici de budget reproductible, pas de promesse de taille d’un futur schéma compilé. Un contrat absent/expurgé est réservé en ligne mais sa description complète est inconnue.</p>')
add('<h3>6. Ce qui manque pour dimensionner exactement les actions par équipement</h3><ul><li>Résoudre chaque service vers les cibles et supported_features réels, sans exécution ; conserver une liste explicite des correspondances non prouvées.</li><li>Choisir : un contrat partagé + liens compacts par cible, ou une fiche de 64/96 o par action concrète. La seconde représentation peut coûter bien davantage que le nombre de services.</li><li>Pour K liaisons supplémentaires : ajouter 4K o pour des références u32 seules (index et portée en plus), ou 64K / 96K o si chaque liaison devient une fiche autonome. K n’est pas inventé ici.</li><li>Ajouter les entités désactivées si elles doivent être préchargées, les types autorisés mais non observés, les maxima des champs texte et les quotas de composites.</li><li>Comparer sous Linux les allocations de la représentation complète, le RSS anonyme, les lectures et mutations à ×1/×10. Le banc de layout déjà exécuté prouve 64/96 o, pas le coût complet de ces exports.</li></ul>')
add('<p>Reproduction : <a href="inventorier-ha-memoire.py">analyseur hors ligne</a> ; <a href="inventaire-ha-memoire-v1.json">agrégats et empreintes des entrées</a>. La racine des exports reste privée et est fournie en argument ; elle n’est pas inscrite dans le rapport.</p>')
(OUT/'deux-ha-memoire-v1.html').write_text('\n'.join(html_parts)+'\n')
print(json.dumps([dict(instance=i['label'],entities=i['active_entities'],devices=i['registered_devices'],services=i['services'],base_rows=i['base_rows'],full_rows=i['full_rows'],base96=i['bytes']['base96'],full96=i['bytes']['full96']) for i in instances],ensure_ascii=False))
