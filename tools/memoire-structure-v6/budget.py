#!/usr/bin/env python3
"""Budgets V6 calculés : jamais présentés comme RSS du futur moteur."""
import json,math,pathlib
HERE=pathlib.Path(__file__).resolve().parent; ROOT=HERE.parents[1]
ASSUMPTIONS=dict(alias_fanout=1,values_per_virtual=4,bindings_per_device=2,calcs_per_world=32,schemas_shared=96,transforms_shared=16,label_bytes=24,key_bytes=16,native_id_bytes=32,decode_workers=4,decode_buffer_bytes=65536,event_slots=1024,command_slots=256,queue_envelope_bytes=64,vector_headers=32)
DATASETS=json.loads((ROOT/'tools/catalogue-banc/charges/manifest.json').read_text())['datasets']
def counts(d,scale):
 p=d['coverage']['source_entities'];a=p*scale;v=math.ceil(p/4)*scale;k=32*scale;org=39*scale;b=2*d['devices']*scale
 r=d['rows']*scale+org+v+k+b;s=r+a
 ep=s-scale;em=2*(24*scale+d['devices']*scale+v);ec=2*k
 return dict(base_rows=d['rows']*scale,org=org,devices=d['devices']*scale,values=d['values']*scale,num=d['numeric_rows']*scale,contracts=d['actions']*scale,bindings=b,virtual=v,calcs=k,canonical=r,aliases=a,slots=s,edges_parent=ep,edges_members=em,edges_calc=ec,edges=ep+em+ec,numeric=d['numeric_rows']*scale+k,variable_entries=(d['value_kinds'].get('text',0)+d['value_kinds'].get('record',0))*scale,routes=d['values']*scale+b,record_containers=d['value_kinds'].get('record',0)*scale)
def index_size(n,probes):
 if str(n) in probes:return dict(bytes=probes[str(n)]['requested_bytes'],capacity=probes[str(n)]['capacity'],method='measured_reservation')
 # This extrapolation is checked against every larger measured table in this toolchain.
 bucket=max(8,1<<(((8*n+6)//7)-1).bit_length())
 large=[p for p in probes.values() if p['capacity']>=7]
 overheads={p['requested_bytes']-17*(p['capacity']*8//7) for p in large}
 assert len(overheads)==1
 return dict(bytes=17*bucket+next(iter(overheads)),capacity=bucket*7//8,method='extrapolated_same_toolchain')
def build(probes):
 budgets=[]
 for d in DATASETS:
  if d['label'].endswith('liens4'):continue
  scales=(1,10,100,1000) if d['label'].startswith('somme') else (1,)
  for scale in scales:
   c=counts(d,scale);r=c['canonical'];s=c['slots'];a=c['aliases'];e=c['edges'];m=c['numeric'];idx=index_size(s,probes);routeidx=index_size(c['routes'],probes)
   strings=s+c['values']+c['devices']
   common=dict(aliases=32*a,slots=16*s,metadata=32*s,id_index=idx['bytes'],edges=24*e,csr=16*e+8*(s+1),alias_reverse=4*(s+1)+4*a,typed_index=4*s+68,schemas=64*96,transforms=64*16,components=32*(c['org']+c['devices']+c['virtual']+c['bindings']+c['contracts']+c['record_containers']),calculations=48*c['calcs'],freshness=32*m,routes=32*c['routes'],route_index=routeidx['bytes'],pool_entries=16*c['variable_entries'],pool_bytes=d['payload_pool_bytes']*scale,string_bytes=24*s+16*c['values']+32*c['devices'],string_entries=8*strings,string_search=16*(c['values']+c['devices']),headers=24*32+48*2+64,runtime_buffers=4*65536+(1024+256)*64)
   variants={}
   for mode in ('row96','row64','split'):
    zones=dict(common,rows=(96 if mode=='row96' else 64)*r,numeric=0 if mode=='row96' else 32*m,numeric_refs=0 if mode=='row96' else 4*r)
    if mode=='row64':zones['headers']+=48
    if mode=='split':zones['headers']+=72
    variants[mode]=dict(zones=zones,total=sum(zones.values()))
   budgets.append(dict(dataset=d['label'],scale=scale,counts=c,variants=variants,index_probe=idx,route_index_probe=routeidx,status='calculated_not_rss',scale_status='extrapolated' if scale==1000 else 'dimensioned_from_shapes'))
 measurements={}
 for key,folder in [('catalogue','catalogue-banc'),('aliases','alias-banc'),('organisation','organisation-banc')]:
  measurements[key]=json.loads((ROOT/f'tools/{folder}/sorties/resultats.json').read_text())['results']
 index=json.loads((ROOT/'tools/moteur-suite-banc/sorties/campagnes-v5.json').read_text())
 measurements['engine']=json.loads((ROOT/index['experiments']['moteur-integre-banc']/'resultats.json').read_text())['groups']
 return dict(row_contract='two_dates_and_previous_distinct_value',assumptions=ASSUMPTIONS,datasets=[d for d in DATASETS if not d['label'].endswith('liens4')],budgets=budgets,measurements=measurements,limits=[
  'Budget de dimensionnement proposé, pas allocation ni RSS d’un Bank HA complet.',
  'Meta32, Schema64, Transform64, Component32, Calc48 et Route32 ne contiennent pas implicitement les textes, géométries ou instructions qu’ils référencent.',
  '39 conteneurs, 2 actions autonomes par équipement, alias et 32 calculs par réplication sont des hypothèses synthétiques, pas des capacités découvertes dans HA.',
  'Les services HA sont des contrats partagés ; les candidats HAS_SET restent non résolus.',
  'Les exports ne prouvent ni unique_id durable, ni bornes/unités originales complètes, ni last_changed/last_reported ; seules formes et dernières mises à jour sont exploitées.',
  'Les grands scénarios ne sont pas exécutables tels quels dans Bank (pool exemple limité à1Mio) ni dans le moteur actuel (128 définitions calculées maximum) ; ce sont des dimensions proposées, pas une levée des quotas.',
  'Le pool de valeurs reprend les octets synthétiques du manifeste ; les chaînes de métadonnées sont hypothétiques (24/16/32 octets).',
  'Les 8 octets de previous_payload remplacent observed_ms : les tailles fixes ne changent pas. Un précédent variable reste une référence ; son ancien texte/blob et son descripteur peuvent ajouter du pool, non isolable dans le manifeste opaque hérité et non inclus dans ces totaux de base.',
  'La dernière date de mesure source n’est plus conservée dans Row96. Des calculs fondés sur cette date ou sa fraîcheur exigent une donnée transitoire et/ou un historique temporel séparé, hors de ces deux dates.',
  'Le budget conserve les arêtes parent en plus de Row.parent ; dériver ces arêtes à la reconstruction est une optimisation séparée, pas une mesure déjà faite.',
  'Capacités denses hors HashMap, sans tombstones/fragmentation. Géométrie, droits/instructions variables, piles des tâches, protocoles, connexions, journaux/historiques et souscriptions supplémentaires restent à chiffrer selon configuration.',
  'Réserve parsing proposée 4×64 Kio et enveloppes 1024 événements+256 commandes de64 octets ; ce ne sont pas des tailles/limites actuelles de production.',
  'Les schémas et transformations sont partagés globalement (96 et16), y compris entre répliques identiques.',
 ])
if __name__=='__main__':
 probes=json.loads((HERE/'sorties/index-probe.json').read_text())
 output=build(probes)
 (HERE/'sorties/budgets.json').write_text(json.dumps(output,ensure_ascii=False,indent=2)+'\n')
 print(json.dumps(dict(scenarios=len(output['budgets']),alternatives=3,scope='calculated_not_rss')))
