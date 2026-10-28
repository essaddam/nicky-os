#!/usr/bin/env python3
"""Capture immuable : sources V6, entrées publiques et sorties explicites."""
import hashlib,json,pathlib,shutil,subprocess,zipfile
HERE=pathlib.Path(__file__).resolve().parent;ROOT=HERE.parents[1]
meta=json.loads((HERE/'sorties/metadata.json').read_text())
files=set(HERE.glob('*.rs'))|set(HERE.glob('*.py'))|set(HERE.glob('*.js'))|{HERE/'README.md',HERE/'layouts.json',HERE/'demo.json',HERE/'exemple-pedagogique.json'}
files.update(p for p in (HERE/'sorties').iterdir() if p.is_file() and p.name not in ('validation.json','capture.json'))
operations=HERE/'operations-banc'
if operations.exists():
 files.update(p for p in operations.rglob('*') if p.is_file() and p.suffix in ('.rs','.py','.md','.json','.jsonl','.txt') and not any(part.startswith('.build-') for part in p.parts))
alignment=HERE/'alignement-banc'
if alignment.exists():
 files.update(p for p in alignment.rglob('*') if p.is_file() and p.suffix in ('.rs','.py','.md','.json','.jsonl','.txt') and not any(part.startswith('.build-') for part in p.parts))
files.add(ROOT/'docs/proposition/analyse/catalogue-memoire-v6.html')
files.add(ROOT/'docs/proposition/analyse/catalogue-memoire-v4.html')
for folder in ('catalogue-banc','alias-banc','organisation-banc'):
 files.update((ROOT/f'tools/{folder}/main.rs',ROOT/f'tools/{folder}/sorties/resultats.json'))
files.update((ROOT/'tools/catalogue-banc/charges').glob('*'))
index=ROOT/'tools/moteur-suite-banc/sorties/campagnes-v5.json';files.add(index)
v5=json.loads(index.read_text())
files.add(ROOT/v5['experiments']['moteur-integre-banc']/'resultats.json')
files={p for p in files if p.is_file()}
digest=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
hashes={str(p.relative_to(ROOT)):digest(p) for p in sorted(files)}
key=hashlib.sha256(json.dumps(hashes,sort_keys=True).encode()).hexdigest()[:16]
stamp=meta['timestamp_utc'].replace('-','').replace(':','')
folder=HERE/'sorties/campagnes'/f'{stamp}-{key}'
if not (folder/'capture.json').exists():
 folder.mkdir(parents=True,exist_ok=True)
 with zipfile.ZipFile(folder/'sources-et-preuves.zip','w',zipfile.ZIP_DEFLATED) as z:
  for p in sorted(files):z.write(p,str(p.relative_to(ROOT)))
 record=dict(scope='Sources publiques, formes synthétiques et mesures historiques ; aucune conversation ni export HA brut.',source_files=hashes,image_id=meta['image_id'],toolchain=meta['toolchain'],archive_sha256=digest(folder/'sources-et-preuves.zip'),git_commit=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),replay='Extraire dans une copie du dépôt, depuis sa racine ; suivre README.')
 (folder/'capture.json').write_text(json.dumps(record,ensure_ascii=False,indent=2)+'\n')
 if (HERE/'sorties/validation.json').exists():shutil.copyfile(HERE/'sorties/validation.json',folder/'validation.json')
else:
 record=json.loads((folder/'capture.json').read_text())
 assert record['source_files']==hashes
 assert record['archive_sha256']==digest(folder/'sources-et-preuves.zip')
with zipfile.ZipFile(folder/'sources-et-preuves.zip') as z:
 for name,wanted in hashes.items():assert hashlib.sha256(z.read(name)).hexdigest()==wanted
pointer=dict(path=str(folder.relative_to(ROOT)),archive_sha256=record['archive_sha256'],files=len(hashes))
(HERE/'sorties/capture.json').write_text(json.dumps(pointer,indent=2)+'\n')
print(json.dumps(pointer))
