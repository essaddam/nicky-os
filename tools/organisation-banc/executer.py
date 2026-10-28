#!/usr/bin/env python3
"""Campagne indépendante d'organisation : données synthétiques, compteurs publics."""
import hashlib,json,pathlib,statistics,subprocess,tempfile,time
HERE=pathlib.Path(__file__).resolve().parent;OUT=HERE/'sorties';OUT.mkdir(exist_ok=True)
mp=HERE.parent/'catalogue-banc/charges/manifest.json';manifest=json.loads(mp.read_text());bylabel={d['label']:d for d in manifest['datasets']}
source=(HERE/'main.rs').read_bytes();image='moli-os-dev:latest'
def run(args):
 p=subprocess.run(args,text=True,capture_output=True)
 if p.returncode:print(p.stdout,p.stderr,flush=True);p.check_returncode()
 return p
jobs=[('a-principal',1),('b-principal',1),('somme-principal',1),('somme-principal',10),('somme-principal',100),('somme-detail',1),('somme-detail',10),('somme-detail',100)]
variants=['nested_ids','flat_cached','csr_cached']
meta=dict(source_sha256=hashlib.sha256(source).hexdigest(),manifest_sha256=hashlib.sha256(mp.read_bytes()).hexdigest(),image_id=run(['docker','image','inspect',image,'--format','{{.Id}}']).stdout.strip(),compile_flags='edition2024 -Dwarnings opt-level=s',cpus=1,memory_limit='512m',network='none',process_trials=3,jobs=jobs,variants=variants,timestamp_utc=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()))
raw=[]
with tempfile.TemporaryDirectory(prefix='.build-',dir=HERE) as tmp:
 (pathlib.Path(tmp)/'main.rs').write_bytes(source)
 base=['docker','run','--rm','--network','none','--cpus','1','--memory','512m','-v',f'{tmp}:/build',image]
 p=run(base+['sh','-c','rustc --edition=2024 -D warnings -C opt-level=s /build/main.rs -o /build/banc && rustc --edition=2024 -D warnings --test /build/main.rs -o /build/tests && /build/tests'])
 (OUT/'tests.txt').write_text(p.stdout+p.stderr)
 for label,scale in jobs:
  d=bylabel[label]
  for variant in variants:
   for trial in range(1,4):
    r=json.loads(run(base+['/build/banc','--bench',variant,str(d['devices']),str(d['values']),str(d['actions']),str(scale)]).stdout)
    r.update(dataset=label,trial=trial);raw.append(r)
   print(f'{label} ×{scale} {variant}: OK',flush=True)
   (OUT/'brut.jsonl').write_text(''.join(json.dumps(x,separators=(',',':'))+'\n' for x in raw))
summary=[]
for label,scale in jobs:
 for variant in variants:
  group=[r for r in raw if (r['dataset'],r['scale'],r['variant'])==(label,scale,variant)];assert len(group)==3
  row={k:statistics.median(r[k] for r in group) for k,v in group[0].items() if k!='trial' and isinstance(v,(int,float)) and not isinstance(v,bool)}
  row.update({k:v for k,v in group[0].items() if isinstance(v,(str,bool))});row['trials']=3;summary.append(row)
(OUT/'resultats.json').write_text(json.dumps(dict(meta=meta,results=summary),ensure_ascii=False,indent=2)+'\n')
print(f'{len(raw)} essais ; {len(summary)} médianes.',flush=True)
