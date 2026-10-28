#!/usr/bin/env python3
"""Fige des campagnes publiques : source exacte, scripts, sorties et charges synthétiques."""
import hashlib,json,pathlib,re,subprocess,zipfile
ROOT=pathlib.Path(__file__).resolve().parents[2]
DIRS=['catalogue-banc','alias-banc','memoire-orientations-banc','organisation-banc','mqtt-canonicalisation-banc']
manifest=ROOT/'tools/catalogue-banc/charges/manifest.json'
manifest_sha=hashlib.sha256(manifest.read_bytes()).hexdigest()
fixture_dir=ROOT/'tools/bancs-reproductibilite/charges';fixture_dir.mkdir(exist_ok=True)
fixture=fixture_dir/f'charges-synthetiques-{manifest_sha[:16]}.zip'
if not fixture.exists():
 with zipfile.ZipFile(fixture,'w',compression=zipfile.ZIP_DEFLATED) as z:
  for p in sorted((ROOT/'tools/catalogue-banc/charges').iterdir()):
   if p.suffix in ['.json','.tsv']:z.write(p,p.name)
versions={};runs=[]
def put(path,data):
 if path.exists():
  assert path.read_bytes()==data,f'Archive existante différente : {path.name}'
 else:path.write_bytes(data)
for name in DIRS:
 base=ROOT/'tools'/name
 result=json.loads((base/'sorties/resultats.json').read_text());meta=result['meta'];raw=(base/'main.rs').read_bytes()
 assert hashlib.sha256(raw).hexdigest()==meta['source_sha256'],name
 if 'manifest_sha256' in meta:assert meta['manifest_sha256']==manifest_sha,name
 stamp=re.sub(r'[-:]','',meta['timestamp_utc'])
 folder=base/'sorties/campagnes'/(stamp+'-'+meta['source_sha256'][:12]);folder.mkdir(parents=True,exist_ok=True)
 put(folder/'source-mesuree.rs',raw)
 if (folder/'campagne.json').exists():
  # Les scripts archivés restent ceux de la capture initiale, même si le rendu évolue.
  for filename in ['resultats.json','brut.jsonl','tests.txt']:
   current=base/'sorties'/filename
   if current.exists():assert (folder/filename).read_bytes()==current.read_bytes(),name+' : campagne modifiée sans nouvel identifiant'
  runs.append(dict(experiment=name,path=str(folder.relative_to(ROOT)),source_sha256=meta['source_sha256']))
  continue
 for p in sorted(base.glob('*.py')):put(folder/p.name,p.read_bytes())
 put(folder/'README.md',(base/'README.md').read_bytes())
 for filename in ['resultats.json','brut.jsonl','tests.txt']:
  path=base/'sorties'/filename
  if path.exists():put(folder/filename,path.read_bytes())
 image=meta['image_id']
 if image not in versions:
  out=subprocess.run(['docker','run','--rm','--network','none',image,'sh','-c','rustc --version --verbose && uname -m && cat /etc/alpine-release'],text=True,capture_output=True,check=True)
  versions[image]=out.stdout
 put(folder/'toolchain.txt',versions[image].encode())
 source_info=dict(experiment=name,source_sha256=meta['source_sha256'],timestamp_utc=meta['timestamp_utc'],image_id=image,fixture_archive=str(fixture.relative_to(ROOT)) if 'manifest_sha256' in meta else None,fixture_archive_sha256=hashlib.sha256(fixture.read_bytes()).hexdigest() if 'manifest_sha256' in meta else None,scope='Sources et sorties publiques uniquement ; aucune conversation ni export HA brut.')
 put(folder/'campagne.json',(json.dumps(source_info,ensure_ascii=False,indent=2)+'\n').encode())
 runs.append(dict(experiment=name,path=str(folder.relative_to(ROOT)),source_sha256=meta['source_sha256']))
index=ROOT/'docs/proposition/analyse/orientations-v4/campagnes.json';index.write_text(json.dumps(dict(campaigns=runs,fixture=str(fixture.relative_to(ROOT))),ensure_ascii=False,indent=2)+'\n')
print(json.dumps(dict(campaigns=len(runs),synthetic_fixture_archive=str(fixture.relative_to(ROOT))),ensure_ascii=False))
