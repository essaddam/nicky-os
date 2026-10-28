#!/usr/bin/env python3
"""Preuve Docker des structures et réservations d'index, sans toucher au moteur."""
import hashlib,importlib.util,json,pathlib,re,shutil,subprocess,tempfile,time
HERE=pathlib.Path(__file__).resolve().parent; ROOT=HERE.parents[1]; OUT=HERE/'sorties'
OUT.mkdir(exist_ok=True)
spec=importlib.util.spec_from_file_location('v6_budget',HERE/'budget.py')
budget=importlib.util.module_from_spec(spec);spec.loader.exec_module(budget)
source={p.name:p.read_bytes() for p in HERE.glob('*.rs')}
source_hash={n:hashlib.sha256(b).hexdigest() for n,b in source.items()}
sizes={1,14,1567}
for d in budget.DATASETS:
 if d['label'].endswith('liens4'):continue
 for scale in ((1,10,100) if d['label'].startswith('somme') else (1,)):
  c=budget.counts(d,scale);sizes.update((c['slots'],c['routes']))
assert max(sizes)<=2_000_000
image='moli-os-dev:latest'
def run(args):
 p=subprocess.run(args,text=True,capture_output=True)
 if p.returncode:raise RuntimeError(p.stdout+p.stderr)
 return p.stdout
image_id=run(['docker','image','inspect',image,'--format','{{.Id}}']).strip()
with tempfile.TemporaryDirectory(prefix='.build-',dir=HERE) as temp:
 snap=pathlib.Path(temp)
 for name,raw in source.items():(snap/name).write_bytes(raw)
 base=['docker','run','--rm','--network','none','--cpus','1','--memory','512m','-v',str(snap)+':/build',image]
 flags='--edition=2024 -D warnings -C opt-level=s -C codegen-units=1'
 tests=run(base+['sh','-c',f'rustc {flags} --test /build/main.rs -o /build/tests && /build/tests && rustc {flags} /build/main.rs -o /build/banc && rustc {flags} /build/probe_index.rs -o /build/probe'])
 layouts=json.loads(run(base+['/build/banc','--layouts']))
 demo=json.loads(run(base+['/build/banc','--demo']))
 probes=[json.loads(l) for l in run(base+['/build/probe',*map(str,sorted(sizes))]).splitlines()]
 toolchain=run(base+['sh','-c','rustc --version --verbose && uname -m'])
assert source_hash=={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in HERE.glob('*.rs')},'Sources modifiées pendant la preuve'
(OUT/'tests.txt').write_text(tests)
(OUT/'layouts.json').write_text(json.dumps(layouts,ensure_ascii=False,indent=2)+'\n')
(OUT/'demo.json').write_text(json.dumps(demo,ensure_ascii=False,indent=2)+'\n')
(HERE/'layouts.json').write_text(json.dumps(layouts,ensure_ascii=False,indent=2)+'\n')
(HERE/'demo.json').write_text(json.dumps(demo,ensure_ascii=False,indent=2)+'\n')
(OUT/'index-probe.json').write_text(json.dumps({str(p['entries_requested']):p for p in probes},indent=2)+'\n')
test_count=int(re.search(r'test result: ok\. (\d+) passed; 0 failed',tests).group(1))
metadata=dict(timestamp_utc=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),image_id=image_id,toolchain=toolchain,flags=flags,cpus=1,memory='512m',network='none',source_sha256=source_hash,index_reservations=len(probes),test_count=test_count,row_contract='two_dates_and_previous_distinct_value',scope='18 layouts et modèle structurel synthétique ; pas moteur livré/RSS/latence')
(OUT/'metadata.json').write_text(json.dumps(metadata,ensure_ascii=False,indent=2)+'\n')
result=budget.build({str(p['entries_requested']):p for p in probes})
(OUT/'budgets.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
print(json.dumps(dict(structures=len(layouts['structures']),tests=test_count,index_reservations=len(probes),budget_scenarios=len(result['budgets']),scope='calculated_not_rss')))
