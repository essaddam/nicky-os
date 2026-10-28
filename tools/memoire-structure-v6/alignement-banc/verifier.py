#!/usr/bin/env python3
"""Audit recorded measurements, source identity, semantic parity and memory math."""
import collections
import hashlib
import json
import math
from pathlib import Path
import statistics

HERE=Path(__file__).resolve().parent
PARENT=HERE.parent
ROOT=HERE.parents[2]
VARIANTS={'compact96','base128_stride96','pad128_align64','pad128_align128','inline_meta128','split32'}
OPS={'scan_state','scan_full','random_state','id_state','state_and_meta','update_previous'}

def validate():
    d=json.loads((HERE/'sorties/resultats.json').read_text());meta=d['metadata']
    assert meta['process_repetitions']==meta['rounds_per_process']==3
    assert (meta['cpus'],meta['cpuset'],meta['memory'],meta['network'])==(4,'0-3','768m','none')
    assert len(meta['source_sha256'])==5
    for name,wanted in meta['source_sha256'].items():assert hashlib.sha256((PARENT/name).read_bytes()).hexdigest()==wanted,name
    path=ROOT/'tools/catalogue-banc/charges/manifest.json';assert hashlib.sha256(path.read_bytes()).hexdigest()==meta['manifest_sha256']
    manifest=json.loads(path.read_text());expected=set()
    for item in manifest['datasets']:
        if item['label'].endswith('liens4'):continue
        for scale in ((1,10,100) if item['label']=='somme-detail' else (1,)):
            n=(item['rows']+39+math.ceil(item['coverage']['source_entities']/4)+32+2*item['devices'])*scale
            expected.add((item['label'],scale,n))
    assert {(x['dataset'],x['scale'],x['n']) for x in d['dimensions']}==expected
    assert d['tests']['passed'] is True and d['tests']['parity_against_bank'] is True
    assert (d['tests']['layout_variants'],d['tests']['atomic_variants'])==(6,5)
    assert len(d['measurements'])==336 and len(d['memory'])==144
    groups=collections.defaultdict(set);parity=collections.defaultdict(set);samples=0
    for row in d['measurements']:
        points=row['samples'];assert len(points)==9
        assert {(s['process'],s['round']) for s in points}=={(p,r) for p in range(3) for r in range(3)}
        values=[s['ns_per_op'] for s in points]
        assert all(math.isfinite(v) and v>0 for v in values)
        assert (row['median_ns'],row['min_ns'],row['max_ns'])==(statistics.median(values),min(values),max(values))
        for s in points:
            assert s['iterations']>0 and math.isclose(s['ns_per_op'],s['elapsed_ns']/s['iterations'])
            key=(row['kind'],row['operation'],row['dataset'],row['scale'],row['threads'],row['pattern'],s['process'],s['round'])
            parity[key].add(s['checksum'])
            if row['kind']=='parallel':
                assert len(s['worker_cpus'])==row['threads']
                for cpu in s['worker_cpus']:assert cpu['requested']==cpu['start']==cpu['end']
                assert s['checksum']==256*row['threads']*(1025 if row['operation']=='all_words' else 1023)
            samples+=1
        groups[(row['kind'],row['dataset'],row['scale'],row['threads'],row['pattern'])].add((row['variant'],row['operation']))
    assert samples==3024 and len(parity)==540 and all(len(v)==1 for v in parity.values())
    assert d['tests']['cross_layout_checksum_groups']==len(parity)
    for dataset,scale,n in expected:
        assert groups[('sample',dataset,scale,None,None)]=={(v,o) for v in VARIANTS for o in OPS}
    for threads in (1,2,4):
        for pattern in ('interleaved','blocks'):
            expected_atomic={(v,'state_words') for v in ('compact96','pad128_align64','pad128_align128','hot32_packed','hot128_isolated')}|{(v,'all_words') for v in ('compact96','pad128_align64','pad128_align128')}
            assert groups[('parallel',None,None,threads,pattern)]==expected_atomic
    memory_keys=set()
    for row in d['memory']:
        n=row['n'];variant=row['variant'];memory_keys.add((row['dataset'],row['scale'],variant,row['process']))
        arrays=row['arrays'];reserved=sum(a['reserved_bytes'] for a in arrays)
        assert reserved==row['requested_rows_meta_bytes']
        for a in arrays:
            assert a['len']<=a['capacity'] and a['used_bytes']==a['len']*a['element_bytes']
            assert a['reserved_bytes']==a['capacity']*a['element_bytes']
        amount=((n+3)//4)*384+n*32 if variant=='base128_stride96' else n*(160 if variant.startswith('pad') else 128)
        assert reserved==amount and row['slots_bytes']==16*n
        if variant in ('inline_meta128','pad128_align128','base128_stride96'):assert arrays[0]['base_mod128']==0
        if variant=='pad128_align64':assert arrays[0]['base_mod64']==0
        assert row['process_bytes']['VmHWM']>=row['process_bytes']['VmRSS']>=row['process_bytes']['RssAnon']>0
    assert len(memory_keys)==144
    raw=[json.loads(line) for line in (HERE/'sorties/echantillons.jsonl').read_text().splitlines()]
    assert len(raw)==3168
    assert sum(e['kind']=='parallel' for e in raw)==432
    result={'status':'pass','groups':336,'samples':samples,'memory_captures':144,'checksum_parity_groups':len(parity),'source_files':5,'scope':'recorded layout kernels and atomic sharing probe; no whole-engine performance claim'}
    (HERE/'sorties/validation.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
    return result

if __name__=='__main__':print(json.dumps(validate()))
