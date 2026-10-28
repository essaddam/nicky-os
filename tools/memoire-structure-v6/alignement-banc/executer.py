#!/usr/bin/env python3
"""Layout measurements in a disposable Docker runtime, never against the home."""
import argparse
import collections
import hashlib
import json
import math
import pathlib
import shutil
import statistics
import subprocess
import tempfile
import time

HERE=pathlib.Path(__file__).resolve().parent
PARENT=HERE.parent
ROOT=HERE.parents[2]
OUT=HERE/'sorties'
FLAGS='--edition=2024 -D warnings -C opt-level=3 -C codegen-units=1'
VARIANTS=['compact96','base128_stride96','pad128_align64','pad128_align128','inline_meta128','split32']

def run(cmd):
    p=subprocess.run(cmd,text=True,capture_output=True,timeout=180)
    if p.returncode: raise RuntimeError(p.stdout+'\n'+p.stderr)
    return p.stdout

def digest(p):return hashlib.sha256(p.read_bytes()).hexdigest()

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--docker-context')
    parser.add_argument('--image',default='rust:1-alpine')
    args=parser.parse_args()
    cli=shutil.which('docker')
    if not cli:
        candidate=pathlib.Path('/opt/homebrew/opt/docker/bin/docker')
        if candidate.exists():cli=str(candidate)
    if not cli:raise SystemExit('Docker CLI required; no local compiler fallback.')
    docker=[cli]+(['--context',args.docker_context] if args.docker_context else [])
    image=run(docker+['image','inspect',args.image,'--format','{{.Id}}']).strip()
    sources={PARENT/'layouts.rs',PARENT/'model.rs',HERE/'main.rs',HERE/'executer.py',HERE/'README.md'}
    hashes={str(p.relative_to(PARENT)):digest(p) for p in sorted(sources)}
    manifest_path=ROOT/'tools/catalogue-banc/charges/manifest.json'
    manifest=json.loads(manifest_path.read_text())
    dimensions=[]
    for d in manifest['datasets']:
        if d['label'].endswith('liens4'):continue
        for scale in ((1,10,100) if d['label']=='somme-detail' else (1,)):
            n=(d['rows']+39+math.ceil(d['coverage']['source_entities']/4)+32+2*d['devices'])*scale
            dimensions.append(dict(dataset=d['label'],scale=scale,n=n))
    OUT.mkdir(exist_ok=True)
    events=[];t0=time.monotonic()
    with tempfile.TemporaryDirectory(prefix='.build-',dir=HERE) as temp:
        snap=pathlib.Path(temp)
        for p in sources:
            target=snap/p.relative_to(PARENT);target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(p,target)
        base=docker+['run','--rm','--network','none','--cpus','4','--cpuset-cpus','0-3','--memory','768m','-v',str(snap)+':/build',image]
        toolchain=run(base+['sh','-c','rustc --version --verbose && uname -m'])
        guest=run(base+['sh','-c','cat /sys/devices/system/cpu/online; cat /sys/fs/cgroup/cpu.max; cat /sys/fs/cgroup/cpuset.cpus.effective; for f in /sys/devices/system/cpu/cpu0/cache/index*/coherency_line_size; do test ! -f "$f" || cat "$f"; done'])
        run(base+['sh','-c',f'rustc {FLAGS} /build/alignement-banc/main.rs -o /build/alignement'])
        tests=json.loads(run(base+['/build/alignement','--self-test']))
        assert tests['passed'] is True
        for process in range(3):
            # Alternate variant order to reduce one-direction cache/thermal bias.
            order=VARIANTS if process%2==0 else list(reversed(VARIANTS))
            for d in dimensions:
                for variant in order:
                    raw=run(base+['/build/alignement','--serial',variant,str(d['n'])])
                    for line in raw.splitlines():events.append(dict(json.loads(line),process=process,**d))
            for threads in (1,2,4):
                for pattern in ('interleaved','blocks'):
                    for variant in (['compact96','pad128_align64','pad128_align128','hot32_packed','hot128_isolated'] if process%2==0 else ['hot128_isolated','hot32_packed','pad128_align128','pad128_align64','compact96']):
                        for operation in (['state_words','all_words'] if variant.startswith(('compact','pad')) else ['state_words']):
                            raw=run(base+['/build/alignement','--parallel',variant,str(threads),pattern,operation])
                            for line in raw.splitlines():events.append(dict(json.loads(line),process=process))
            print(json.dumps({'process_complete':process+1,'events':len(events)}),flush=True)
    assert all(digest(PARENT/name)==wanted for name,wanted in hashes.items())
    groups=collections.defaultdict(list);memories=[]
    parity=collections.defaultdict(set)
    for e in events:
        if e['kind']=='memory':memories.append(e);continue
        key=(e['kind'],e['variant'],e['operation'],e.get('dataset'),e.get('scale'),e.get('threads'),e.get('pattern'))
        groups[key].append(e)
        common=(e['kind'],e['operation'],e.get('dataset'),e.get('scale'),e.get('threads'),e.get('pattern'),e['process'],e['round'])
        parity[common].add(e['checksum'])
    assert all(len(values)==1 for values in parity.values()), 'Observable result differs across layouts'
    tests['cross_layout_checksum_groups']=len(parity)
    measured=[]
    for key,samples in groups.items():
        assert len(samples)==9
        values=[s['ns_per_op'] for s in samples]
        kind,variant,operation,dataset,scale,threads,pattern=key
        measured.append(dict(kind=kind,variant=variant,operation=operation,dataset=dataset,scale=scale,threads=threads,pattern=pattern,median_ns=statistics.median(values),min_ns=min(values),max_ns=max(values),samples=samples))
    host={}
    if shutil.which('sysctl'):
        for name in ('hw.cachelinesize','hw.physicalcpu','hw.logicalcpu'):
            p=subprocess.run(['sysctl','-n',name],text=True,capture_output=True)
            if p.returncode==0:host[name]=int(p.stdout.strip())
    result=dict(metadata=dict(timestamp_utc=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),image_id=image,toolchain=toolchain,guest_cpu_observation=guest,host_cpu_observation=host,cpus=4,cpuset='0-3',memory='768m',network='none',flags=FLAGS,source_sha256=hashes,manifest_sha256=digest(manifest_path),process_repetitions=3,rounds_per_process=3,wall_seconds=time.monotonic()-t0,scope='isolated layout kernels and atomic sharing probe; not a migrated Hub'),tests=tests,dimensions=dimensions,measurements=measured,memory=memories)
    (OUT/'resultats.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
    (OUT/'echantillons.jsonl').write_text(''.join(json.dumps(e,ensure_ascii=False)+'\n' for e in events))
    print(json.dumps({'status':'pass','groups':len(measured),'events':len(events),'wall_seconds':result['metadata']['wall_seconds']}))

if __name__=='__main__':main()
