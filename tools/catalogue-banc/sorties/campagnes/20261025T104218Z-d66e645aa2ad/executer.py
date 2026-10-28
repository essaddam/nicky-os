#!/usr/bin/env python3
"""Mesures séquentielles : un conteneur neuf par essai, médiane de trois essais."""
import hashlib,json,pathlib,statistics,subprocess,tempfile,time
HERE=pathlib.Path(__file__).resolve().parent
OUT=HERE/'sorties';OUT.mkdir(exist_ok=True)
manifest=json.loads((HERE/'charges/manifest.json').read_text())
bylabel={x['label']:x for x in manifest['datasets']}
image='moli-os-dev:latest'
source_snapshot=(HERE/'main.rs').read_bytes()
def run(cmd):
 result=subprocess.run(cmd,text=True,capture_output=True)
 if result.returncode:
  print(result.stdout,result.stderr,flush=True)
  result.check_returncode()
 return result
info=run(['docker','image','inspect',image,'--format','{{.Id}}']).stdout.strip()
meta=dict(image=image,image_id=info,cpus=1,memory_limit='512m',process_trials=3,internal_rounds=7,compile_flags='rustc --edition=2024 -D warnings -C opt-level=s -C codegen-units=1',source_sha256=hashlib.sha256(source_snapshot).hexdigest(),manifest_sha256=hashlib.sha256((HERE/'charges/manifest.json').read_bytes()).hexdigest(),timestamp_utc=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()))
with tempfile.TemporaryDirectory(prefix='.build-', dir=HERE) as temp:
 (pathlib.Path(temp)/'main.rs').write_bytes(source_snapshot)
 mount=f'{HERE}:/study:ro';build=f'{temp}:/build'
 base=['docker','run','--rm','--cpus','1','--memory','512m','-v',mount,'-v',build,image]
 commands='rustc --edition=2024 -D warnings -C opt-level=s -C codegen-units=1 /build/main.rs -o /build/banc && rustc --edition=2024 -D warnings --test /build/main.rs -o /build/tests && /build/tests'
 test=run(base+['sh','-c',commands])
 (OUT/'tests.txt').write_text(test.stdout+test.stderr)
 jobs=[('a-principal',1),('b-principal',1),('a-detail',1),('b-detail',1),('somme-principal',1),('somme-principal',10),('somme-principal',100),('somme-detail',1),('somme-detail',10),('somme-detail',100),('somme-detail-liens4',1),('somme-detail-liens4',10)]
 raw=[]
 for label,scale in jobs:
  for layout in ['uniform96','uniform64','split']:
   for trial in range(3):
    data=json.loads(run(base+['/build/banc','--bench',f'/study/charges/{label}.tsv',layout,str(scale),str(bylabel[label]['payload_pool_bytes'])]).stdout)
    data.update(dataset=label,trial=trial+1)
    assert data['n']==bylabel[label]['rows']*scale
    assert data['parity'] is True
    raw.append(data)
   print(f'{label} ×{scale} {layout} : OK',flush=True)
   (OUT/'brut.jsonl').write_text(''.join(json.dumps(x,separators=(',',':'))+'\n' for x in raw))
 summary=[]
 for label,scale in jobs:
  checks=set()
  for layout in ['uniform96','uniform64','split']:
   group=[x for x in raw if x['dataset']==label and x['scale']==scale and x['layout']==layout]
   assert len(group)==3
   checks.update(x['checksum'] for x in group)
   result={k:statistics.median(x[k] for x in group) for k,v in group[0].items() if isinstance(v,(int,float)) and not isinstance(v,bool) and all(isinstance(x.get(k),(int,float)) for x in group)}
   for k in ['dataset','layout','checksum','parity']:result[k]=group[0][k]
   result['trial_count']=3
   summary.append(result)
  assert len(checks)==1, (label,scale,'layout mismatch')
 (OUT/'resultats.json').write_text(json.dumps(dict(meta=meta,results=summary),ensure_ascii=False,indent=2)+'\n')
print('Tous les essais et les comparaisons de contenu sont terminés.',flush=True)
