#!/usr/bin/env python3
"""Exporte seulement une FORME synthétique (types, longueurs, arborescence).
Usage: preparer.py <racine_privee> <sortie_charge>
Aucune valeur, identité, date réelle, pièce ou nom de service n'est copié.
"""
import collections,hashlib,json,math,pathlib,re,struct,sys
PRIVATE=pathlib.Path(sys.argv[1]);OUT=pathlib.Path(sys.argv[2]);OUT.mkdir(parents=True,exist_ok=True)
READ=1;HAS_SET=2;NUMERIC=4;BOUNDS=8;UNRESOLVED=16
KINDS={'null':0,'bool':1,'int':2,'float':3,'text':4,'record':5}
BOOL_DOMAINS={'binary_sensor','light','switch','input_boolean','automation','fan','siren','remote'}
NUMERIC_DOMAINS={'number','input_number'}
SET_SERVICES={'number':('set_value',),'input_number':('set_value',),'select':('select_option',),'input_select':('select_option',),'text':('set_value',),'input_text':('set_value',),'switch':('turn_on','turn_off'),'light':('turn_on','turn_off'),'input_boolean':('turn_on','turn_off')}
def compact(v):return json.dumps(v,ensure_ascii=False,separators=(',',':')).encode()
def bits(v):return struct.unpack('<Q',struct.pack('<d',v))[0]
def classify(state,domain):
 if state in ('unknown','unavailable') or state is None:return 'null'
 if domain in {'text','input_text','date','time','datetime','input_datetime'}:return 'text'
 if domain in BOOL_DOMAINS and state in ('on','off'):return 'bool'
 if isinstance(state,str) and re.fullmatch(r'[+-]?\d+',state):
  try:return 'int' if -(2**63)<=int(state)<2**63 else 'text'
  except ValueError:return 'text'
 if isinstance(state,str):
  try:
   n=float(state)
   if math.isfinite(n):return 'float'
  except ValueError:pass
 return 'text'
def load(p):return json.loads(p.read_text())
def prepare(folders,label,detail):
 rows=[];pools=0;cases=collections.Counter();inferred_set=0;hashes=[];coverage=collections.Counter()
 def row(kind,parent,vkind,flags):
  n=len(rows);payload=(n*17+3) if vkind in ('int','bool') else bits((n%200)/4) if vkind=='float' else n+1 if vkind in ('text','record') else 0
  if vkind=='bool':payload=n%2
  # Horodatages explicitement synthétiques. Zéro signifie absent dans ce banc.
  dates=(1000,1001,1000) if kind in (0,1) and vkind!='null' else (0,0,0)
  numeric=[0,0,0,0]
  if flags&NUMERIC:
   numeric=[bits(0.),bits(100.),bits(1.),bits(0.)] if vkind!='int' else [0,100,1,0]
  rows.append([kind,parent,KINDS[vkind],flags,payload,*dates,*numeric]);cases[vkind]+=1
  return n
 def walk(obj,parent):
  nonlocal pools
  if isinstance(obj,dict):
   n=row(1,parent,'record',READ);pools+=16+8*len(obj)
   for key,value in obj.items():pools+=len(key.encode());walk(value,n)
  elif isinstance(obj,list):
   n=row(1,parent,'record',READ);pools+=16+8*len(obj)
   for value in obj:walk(value,n)
  elif obj is None:row(1,parent,'null',READ)
  elif isinstance(obj,bool):row(1,parent,'bool',READ)
  elif isinstance(obj,int):row(1,parent,'int',READ|NUMERIC|BOUNDS)
  elif isinstance(obj,float):row(1,parent,'float',READ|NUMERIC|BOUNDS)
  elif isinstance(obj,str):row(1,parent,'text',READ);pools+=len(obj.encode())
  else:raise TypeError('format JSON inattendu')
 for folder in folders:
  rt=folder/'sanitized/runtime';rg=folder/'sanitized/registries'
  states=load(rt/'states.json');devices=load(rg/'core.device_registry.json')['devices'];registry=load(rg/'core.entity_registry.json')['entities'];services=load(rt/'services.json')
  for p in [rt/'states.json',rt/'services.json',rg/'core.device_registry.json',rg/'core.entity_registry.json']:hashes.append(hashlib.sha256(p.read_bytes()).hexdigest())
  reg={x['entity_id']:x for x in registry};device_slots={}
  for device in devices:
   n=row(0,2**32-1,'bool',READ);device_slots[device['id']]=n
   pools+=len(str(device['id']).encode())+len(str(device.get('name_by_user') or device.get('name') or '').encode())
  service_names={(g['domain'],name) for g in services for name,spec in g['services'].items() if isinstance(spec,dict)}
  for entity in states:
   domain=entity['entity_id'].split('.')[0];attrs=entity.get('attributes',{});kind=classify(entity['state'],domain)
   flags=READ|UNRESOLVED
   if domain in SET_SERVICES and all((domain,s) in service_names for s in SET_SERVICES[domain]):flags|=HAS_SET;inferred_set+=1
   if kind in ('int','float') or domain in NUMERIC_DOMAINS:flags|=NUMERIC|BOUNDS
   parent=device_slots.get(reg.get(entity['entity_id'],{}).get('device_id'),2**32-1)
   n=row(1,parent,kind,flags)
   coverage['source_entities']+=1;coverage['missing_device_parent']+=parent==2**32-1
   if entity['entity_id'] not in reg:coverage['active_not_in_registry']+=1
   if 'last_updated' in entity:coverage['last_updated_present']+=1
   if 'last_changed' in entity:coverage['last_changed_present']+=1
   if 'last_reported' in entity:coverage['last_reported_present']+=1
   if kind=='text':pools+=len(str(entity['state']).encode())
   pools+=len(entity['entity_id'].encode())+len(str(attrs.get('friendly_name','')).encode())
   stable=reg.get(entity['entity_id'],{}).get('unique_id')
   if stable is not None:pools+=len(str(stable).encode());coverage['unique_id_present']+=1
   if detail:walk(attrs,n)
  for g in services:
   for name,spec in g['services'].items():
    row(2,2**32-1,'null',0)
    pools+=len((g['domain']+'.'+name).encode())
    if isinstance(spec,dict):pools+=len(compact(spec))
    else:coverage['unreadable_service_contract']+=1
 count=collections.Counter(str(r[0]) for r in rows)
 numeric=sum(bool(r[3]&NUMERIC) for r in rows)
 path=OUT/(label+'.tsv')
 header='# kind parent_slot value_kind flags payload observed_ms received_ms changed_ms min max step anchor\n'
 path.write_text(header+'\n'.join('\t'.join(map(str,r)) for r in rows)+'\n')
 # Opaque pools add identity/labels/text/schema bytes, no real strings.
 return dict(label=label,detail=detail,rows=len(rows),devices=count['0'],values=count['1'],actions=count['2'],numeric_rows=numeric,possible_set_rows=inferred_set,unresolved_access_rows=coverage['source_entities'],payload_pool_bytes=pools,row_kinds=dict(count),value_kinds=dict(cases),coverage=dict(coverage),input_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),source_hashes=hashes)
folders=sorted(p for p in PRIVATE.iterdir() if (p/'sanitized/runtime/states.json').exists())
if len(folders)!=2:raise SystemExit('Deux instantanés requis')
sets=[]
for detail in [False,True]:
 suffix='detail' if detail else 'principal'
 for label,parts in [('a',[folders[0]]),('b',[folders[1]]),('somme',folders)]:sets.append(prepare(parts,label+'-'+suffix,detail))
# Stress cardinalité des liaisons : 4 actions ciblées par état principal,
# en plus des contrats partagés. Ce n'est pas une observation HA.
original=next(d for d in sets if d['label']=='somme-detail')
source=(OUT/'somme-detail.tsv').read_text().splitlines()
records=[list(map(int,line.split('\t'))) for line in source if line and not line.startswith('#')]
entity_slots=[n for n,r in enumerate(records) if r[0]==1 and (r[1]==2**32-1 or records[r[1]][0]==0)]
assert len(entity_slots)==original['coverage']['source_entities']
for slot in entity_slots:
 for _ in range(4):records.append([2,slot,0,0,0,0,0,0,0,0,0,0])
stress_path=OUT/'somme-detail-liens4.tsv'
stress_path.write_text(source[0]+'\n'+'\n'.join('\t'.join(map(str,r)) for r in records)+'\n')
stress=dict(original,label='somme-detail-liens4',rows=len(records),actions=original['actions']+4*len(entity_slots),synthetic_target_links=4*len(entity_slots),input_sha256=hashlib.sha256(stress_path.read_bytes()).hexdigest())
sets.append(stress)
result=dict(version=2,source='formes des deux instantanés HA privés, données et horodatages remplacés',set_semantics='HAS_SET est candidat par domaine+services, ACCESS_UNRESOLVED reste présent : aucune permission prouvée',numeric_semantics='grilles synthétiques 0..100 pas1 ; réserves par forme numérique observée ou domaine number ; pas les bornes originales',datasets=sets)
(OUT/'manifest.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
print(json.dumps([{k:d[k] for k in ['label','rows','numeric_rows','possible_set_rows','payload_pool_bytes']} for d in sets]))
