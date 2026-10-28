#!/usr/bin/env python3
"""Preuves V3 : sources figées, volumes, alias identiques avant/après mutations."""
import ast,collections,hashlib,json,pathlib,re,statistics
from html.parser import HTMLParser
HERE=pathlib.Path(__file__).resolve().parent;ROOT=HERE.parents[1]
for p in HERE.glob('*.py'):ast.parse(p.read_text(),filename=str(p))
D=json.loads((HERE/'sorties/resultats.json').read_text());manifest_path=HERE.parent/'catalogue-banc/charges/manifest.json'
M=json.loads(manifest_path.read_text());bylabel={d['label']:d for d in M['datasets']}
assert D['meta']['source_sha256']==hashlib.sha256((HERE/'main.rs').read_bytes()).hexdigest()
assert D['meta']['manifest_sha256']==hashlib.sha256(manifest_path.read_bytes()).hexdigest()
raw=[json.loads(s) for s in (HERE/'sorties/brut.jsonl').read_text().splitlines()]
assert len(raw)==150;assert len(D['results'])==50
bycase=collections.defaultdict(list)
parts=['canonical_bytes','virtual_equipment_bytes','bindings_bytes','generations_bytes','reverse_index_bytes','identity_index_bytes','source_slots_bytes','payload_pool_bytes']
for r in raw:
 d=bylabel[r['dataset']];assert r['parity'] is True
 assert r['canonical_rows']==d['rows']*r['scale']
 assert r['source_rows']==d['coverage']['source_entities']*r['scale']
 assert r['aliases']==r['source_rows']*r['fanout']
 assert r['virtual_equipment_rows']==(r['aliases']+3)//4
 assert r['row_bytes']==96
 assert r['binding_bytes']==(128 if r['variant']=='mirror' else 96 if r['variant']=='row_alias' else 32)
 assert r['canonical_bytes']==96*r['canonical_rows']
 assert r['virtual_equipment_bytes']==96*r['virtual_equipment_rows']
 assert r['bindings_bytes']==r['binding_bytes']*r['aliases']
 assert r['payload_pool_bytes']==d['payload_pool_bytes']*r['scale']
 assert r['allocations_requested_bytes']==sum(r[k] for k in parts)
 assert r['reverse_index_bytes']==4*(r['canonical_rows']+1+r['aliases'])
 for key in ['scalar_ns_per_op','numeric_ns_per_op','source_update_notify_ns_per_op']:assert r[key]>0
 bycase[(r['dataset'],r['scale'],r['fanout'])].append(r)
for key,group in bycase.items():
 assert len(group)==15
 for checksum in ['initial_checksum','final_checksum']:assert len({r[checksum] for r in group})==1
 for variant in ['mirror','row_alias','id_lookup','handle','pointer']:
  trials=[r for r in group if r['variant']==variant];assert len(trials)==3
  assert len({r['allocations_requested_bytes'] for r in trials})==1
  median=next(r for r in D['results'] if (r['dataset'],r['scale'],r['fanout'],r['variant'])==(*key,variant))
  assert median['allocations_requested_bytes']==statistics.median(r['allocations_requested_bytes'] for r in trials)
 fullalias=next(r for r in group if r['variant']=='row_alias')
 mirror=next(r for r in group if r['variant']=='mirror');handle=next(r for r in group if r['variant']=='handle')
 assert mirror['allocations_requested_bytes']-handle['allocations_requested_bytes']==96*mirror['aliases']
 assert fullalias['allocations_requested_bytes']-handle['allocations_requested_bytes']==64*mirror['aliases']
 for variant in ['pointer','id_lookup']:
  shared=next(r for r in group if r['variant']==variant)
  assert shared['allocations_requested_bytes']==handle['allocations_requested_bytes']
tests=(HERE/'sorties/tests.txt').read_text();assert re.search(r'test result: ok\. 17 passed; 0 failed;',tests)
class Page(HTMLParser):
 def __init__(self):super().__init__();self.ids=[];self.links=[]
 def handle_starttag(self,t,attrs):
  a=dict(attrs)
  if 'id' in a:self.ids.append(a['id'])
  if t=='a':self.links.append(a.get('href',''))
for p in [ROOT/'docs/proposition/analyse/catalogue-memoire-v3.html',ROOT/'docs/proposition/analyse/consolidation-entites-valeurs-actions-v1.html']:
 obj=Page();obj.feed(p.read_text());assert len(obj.ids)==len(set(obj.ids))
 for href in obj.links:
  if href.startswith('https://'):continue
  if href.startswith('#'):assert href[1:] in obj.ids
  else:assert (p.parent/href.split('#')[0]).exists(),href
print('PASS — 17 tests ; 150 essais ; 50 médianes ; sources, parité avant/après, allocations et liens vérifiés.')
