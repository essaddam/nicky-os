#!/usr/bin/env python3
"""Banc V3 : source96 unique, miroirs et liens, un processus neuf par essai."""
import hashlib,json,pathlib,statistics,subprocess,tempfile,time
HERE=pathlib.Path(__file__).resolve().parent;TOOLS=HERE.parent
OUT=HERE/'sorties';OUT.mkdir(exist_ok=True)
manifest_path=TOOLS/'catalogue-banc/charges/manifest.json'
manifest=json.loads(manifest_path.read_text());bylabel={d['label']:d for d in manifest['datasets']}
source=(HERE/'main.rs').read_bytes();image='moli-os-dev:latest'
def run(argv):
 r=subprocess.run(argv,text=True,capture_output=True)
 if r.returncode:print(r.stdout,r.stderr,flush=True);r.check_returncode()
 return r
image_id=run(['docker','image','inspect',image,'--format','{{.Id}}']).stdout.strip()
jobs=[('a-principal',1,1),('b-principal',1,1),('somme-principal',1,1),('somme-detail',1,1),('somme-detail',1,4),('somme-detail',1,16),('somme-detail',10,1),('somme-detail',10,4),('somme-detail',100,1),('somme-principal',10,4)]
variants=['mirror','row_alias','id_lookup','handle','pointer']
meta=dict(source_sha256=hashlib.sha256(source).hexdigest(),manifest_sha256=hashlib.sha256(manifest_path.read_bytes()).hexdigest(),image=image,image_id=image_id,cpus=1,memory_limit='512m',process_trials=3,internal_rounds=7,compile_flags='rustc --edition=2024 -D warnings -C opt-level=s -C codegen-units=1',timestamp_utc=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),jobs=jobs,variants=variants)
with tempfile.TemporaryDirectory(prefix='.build-',dir=HERE) as tmp:
 (pathlib.Path(tmp)/'main.rs').write_bytes(source)
 docker=['docker','run','--rm','--cpus','1','--memory','512m','-v',f'{TOOLS}:/study:ro','-v',f'{tmp}:/build',image]
 build='rustc --edition=2024 -D warnings -C opt-level=s -C codegen-units=1 /build/main.rs -o /build/banc && rustc --edition=2024 -D warnings --test /build/main.rs -o /build/tests && /build/tests'
 r=run(docker+['sh','-c',build]);(OUT/'tests.txt').write_text(r.stdout+r.stderr)
 raw=[]
 for label,scale,fanout in jobs:
  for variant in variants:
   for trial in range(3):
    args=['/build/banc','--bench',f'/study/catalogue-banc/charges/{label}.tsv',variant,str(scale),str(fanout),str(bylabel[label]['payload_pool_bytes'])]
    row=json.loads(run(docker+args).stdout)
    row.update(dataset=label,trial=trial+1)
    raw.append(row)
   print(f'{label} ×{scale} fanout={fanout} {variant} : OK',flush=True)
   (OUT/'brut.jsonl').write_text(''.join(json.dumps(r,separators=(',',':'))+'\n' for r in raw))
 summary=[]
 for label,scale,fanout in jobs:
  for variant in variants:
   # Programme doit exposer ces discriminants avec ces noms.
   group=[r for r in raw if r['dataset']==label and r['scale']==scale and r['fanout']==fanout and r['variant']==variant]
   assert len(group)==3
   row={k:statistics.median(x[k] for x in group) for k,v in group[0].items() if isinstance(v,(int,float)) and not isinstance(v,bool) and all(isinstance(x.get(k),(int,float)) for x in group)}
   for k,v in group[0].items():
    if isinstance(v,(str,bool)):row[k]=v
   row['trial_count']=3;summary.append(row)
 (OUT/'resultats.json').write_text(json.dumps(dict(meta=meta,results=summary),ensure_ascii=False,indent=2)+'\n')
print(f'{len(raw)} essais terminés ; {len(summary)} médianes publiées.',flush=True)
