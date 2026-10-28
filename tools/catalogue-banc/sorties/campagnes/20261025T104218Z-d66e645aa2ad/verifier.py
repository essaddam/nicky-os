#!/usr/bin/env python3
"""Vérifie les artefacts publiés, leurs sources et l'égalité des périmètres."""
import ast,collections,hashlib,json,pathlib,re
HERE=pathlib.Path(__file__).resolve().parent
ROOT=HERE.parents[1]
for p in HERE.glob('*.py'):ast.parse(p.read_text(),filename=str(p))
data=json.loads((HERE/'sorties/resultats.json').read_text())
manifest_path=HERE/'charges/manifest.json';manifest=json.loads(manifest_path.read_text())
assert data['meta']['source_sha256']==hashlib.sha256((HERE/'main.rs').read_bytes()).hexdigest(),'source différente du banc exécuté'
assert data['meta']['manifest_sha256']==hashlib.sha256(manifest_path.read_bytes()).hexdigest(),'charge différente du banc exécuté'
bylabel={x['label']:x for x in manifest['datasets']}
for label,d in bylabel.items():
 path=HERE/'charges'/(label+'.tsv');assert hashlib.sha256(path.read_bytes()).hexdigest()==d['input_sha256']
 rows=[list(map(int,s.split('\t'))) for s in path.read_text().splitlines() if s and not s.startswith('#')]
 assert len(rows)==d['rows'];assert sum(bool(r[3]&4) for r in rows)==d['numeric_rows']
 for n,r in enumerate(rows):
  assert len(r)==12
  assert r[1]==2**32-1 or r[1]<n
  assert tuple(r[5:8]) in [(0,0,0),(1000,1001,1000)],'dates non synthétiques'
  if r[0]==2:assert r[2:8]==[0,0,0,0,0,0]
raw=[json.loads(x) for x in (HERE/'sorties/brut.jsonl').read_text().splitlines()]
assert len(raw)==108;assert len(data['results'])==36
sets=collections.defaultdict(list)
for row in raw:
 assert row['parity'] is True
 d=bylabel[row['dataset']]
 assert row['n']==d['rows']*row['scale'];assert row['numeric_rows']==d['numeric_rows']*row['scale']
 assert row['payload_pool_bytes']==d['payload_pool_bytes']*row['scale']
 assert row['allocations_requested_bytes']==sum(row[k] for k in ['fixed_bytes','numeric_bytes','index_bytes','adjacency_bytes','payload_pool_bytes'])
 for key in ['scan_ns_per_row','scalar_ns_per_op','numeric_ns_per_op','lookup_ns_per_op','equal_update_ns_per_op','changed_update_ns_per_op']:assert row[key]>0
 sets[(row['dataset'],row['scale'])].append(row)
for key,rows in sets.items():
 assert len(rows)==9;assert len({r['checksum'] for r in rows})==1
 for layout in ['uniform96','uniform64','split']:
  group=[r for r in rows if r['layout']==layout];assert len(group)==3
  assert len({r['allocations_requested_bytes'] for r in group})==1
 from96=next(r for r in rows if r['layout']=='uniform96');to64=next(r for r in rows if r['layout']=='uniform64')
 assert from96['allocations_requested_bytes']-to64['allocations_requested_bytes']==32*(from96['n']-from96['numeric_rows'])
test=(HERE/'sorties/tests.txt').read_text();assert re.search(r'test result: ok\. \d+ passed; 0 failed;',test)
from html.parser import HTMLParser
class Page(HTMLParser):
 def __init__(self):super().__init__();self.ids=[];self.links=[]
 def handle_starttag(self,t,attrs):
  a=dict(attrs)
  if 'id' in a:self.ids.append(a['id'])
  if t=='a':self.links.append(a.get('href',''))
for p in [ROOT/'docs/proposition/analyse/catalogue-memoire-v2.html',ROOT/'docs/proposition/analyse/consolidation-entites-valeurs-actions-v1.html']:
 page=Page();page.feed(p.read_text());assert len(page.ids)==len(set(page.ids))
 for h in page.links:
  if h.startswith('#'):assert h[1:] in page.ids,h
  else:assert (p.parent/h.split('#')[0]).exists(),h
print('PASS — sources, 7 charges synthétiques, 108 essais, 36 médianes, parité, décomposition mémoire, tests et liens HTML.')
