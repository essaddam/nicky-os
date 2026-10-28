#!/usr/bin/env python3
"""Campagne post-décodage indépendante, sans MQTT ni appareil réel."""
import hashlib,json,pathlib,statistics,subprocess,tempfile,time
HERE=pathlib.Path(__file__).resolve().parent;OUT=HERE/'sorties';OUT.mkdir(exist_ok=True)
source=(HERE/'main.rs').read_bytes();image='moli-os-dev:latest'
def run(args):
 p=subprocess.run(args,text=True,capture_output=True)
 if p.returncode:print(p.stdout,p.stderr,flush=True);p.check_returncode()
 return p
jobs=[(206,1),(290,1),(496,1),(496,10),(496,100)];variants=['allocating_lookup','borrowed_lookup','cached_binding']
meta=dict(source_sha256=hashlib.sha256(source).hexdigest(),image_id=run(['docker','image','inspect',image,'--format','{{.Id}}']).stdout.strip(),compile_flags='edition2024 -Dwarnings opt-level=s',cpus=1,memory_limit='512m',network='none',process_trials=3,measurement_modes=['counted','timing_plain'],jobs=jobs,variants=variants,timestamp_utc=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),scope='apres decodage, routage preassocie pour cache ; pas un benchmark complet MQTT/JSON')
raw=[]
with tempfile.TemporaryDirectory(prefix='.build-',dir=HERE) as tmp:
 (pathlib.Path(tmp)/'main.rs').write_bytes(source)
 base=['docker','run','--rm','--network','none','--cpus','1','--memory','512m','-v',f'{tmp}:/build',image]
 p=run(base+['sh','-c','rustc --edition=2024 -D warnings -C opt-level=s /build/main.rs -o /build/banc-counted && rustc --edition=2024 -D warnings -C opt-level=s --cfg timing_plain /build/main.rs -o /build/banc-timing_plain && rustc --edition=2024 -D warnings --test /build/main.rs -o /build/tests && /build/tests && rustc --edition=2024 -D warnings --test --cfg timing_plain /build/main.rs -o /build/tests-plain && /build/tests-plain'])
 (OUT/'tests.txt').write_text(p.stdout+p.stderr)
 for devices,scale in jobs:
  for variant in variants:
   for mode in ['counted','timing_plain']:
    for trial in range(1,4):
     r=json.loads(run(base+['/build/banc-'+mode,'--bench',variant,str(devices),str(scale)]).stdout)
     assert r['measurement_mode']==mode
     r['trial']=trial;raw.append(r)
    print(f'{devices} appareils ×{scale} {variant} {mode}: OK',flush=True)
    (OUT/'brut.jsonl').write_text(''.join(json.dumps(x,separators=(',',':'))+'\n' for x in raw))
summary=[]
for devices,scale in jobs:
 for variant in variants:
  for mode in ['counted','timing_plain']:
   group=[r for r in raw if (r['devices'],r['scale'],r['variant'],r['measurement_mode'])==(devices,scale,variant,mode)];assert len(group)==3
   row={k:statistics.median(r[k] for r in group) for k,v in group[0].items() if k!='trial' and isinstance(v,(int,float)) and not isinstance(v,bool)}
   row.update({k:v for k,v in group[0].items() if isinstance(v,(str,bool))});row['trials']=3;summary.append(row)
(OUT/'resultats.json').write_text(json.dumps(dict(meta=meta,results=summary),ensure_ascii=False,indent=2)+'\n')
print(f'{len(raw)} essais ; {len(summary)} médianes.',flush=True)
