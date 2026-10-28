#!/usr/bin/env python3
"""Vérifications isolées V4 : compilation Linux sans réseau, aucune maison contactée."""
import hashlib,json,pathlib,subprocess,tempfile,time
HERE=pathlib.Path(__file__).resolve().parent;OUT=HERE/'sorties';OUT.mkdir(exist_ok=True)
source=(HERE/'main.rs').read_bytes();image='moli-os-dev:latest'
def run(args):
 p=subprocess.run(args,text=True,capture_output=True)
 if p.returncode:print(p.stdout,p.stderr,flush=True);p.check_returncode()
 return p
image_id=run(['docker','image','inspect',image,'--format','{{.Id}}']).stdout.strip()
with tempfile.TemporaryDirectory(prefix='.build-',dir=HERE) as tmp:
 (pathlib.Path(tmp)/'main.rs').write_bytes(source)
 args=['docker','run','--rm','--network','none','--cpus','1','--memory','128m','-v',f'{tmp}:/build',image]
 test=run(args+['sh','-c','rustc --edition=2024 -D warnings -C opt-level=s --test /build/main.rs -o /build/tests && /build/tests'])
 (OUT/'tests.txt').write_text(test.stdout+test.stderr)
 r=run(args+['sh','-c','rustc --edition=2024 -D warnings -C opt-level=s /build/main.rs -o /build/proof && /build/proof'])
 measured=json.loads(r.stdout)
 # Trames d'exemple minimales : longueur applicative, pas une mesure réseau.
 payload=b'x'*20;topic=b'etat'
 mqtt311=bytes([0x30,2+len(topic)+len(payload)])+len(topic).to_bytes(2,'big')+topic+payload
 mqtt5=bytes([0x30,2+len(topic)+1+len(payload)])+len(topic).to_bytes(2,'big')+topic+bytes([0])+payload
 server_ws=bytes([0x82,len(payload)])+payload
 mask=bytes([1,2,3,4]);masked=bytes(v^mask[i%4] for i,v in enumerate(payload))
 client_ws=bytes([0x82,0x80|len(payload)])+mask+masked
 example={'payload_bytes':20,'topic_bytes':4,'ws_server':len(server_ws),'ws_client':len(client_ws),'mqtt311_qos0':len(mqtt311),'mqtt5_qos0_no_properties':len(mqtt5),'json_literal_utf8':len(b'{"temperature":21.5,"humidity":48,"battery":91,"linkquality":96}')}
 assert [example[k] for k in ['ws_server','ws_client','mqtt311_qos0','mqtt5_qos0_no_properties','json_literal_utf8']]==[22,26,28,29,64]
 measured['protocol_examples']=example
 meta=dict(source_sha256=hashlib.sha256(source).hexdigest(),image_id=image_id,network='none',flags='edition2024 -Dwarnings opt-level=s',timestamp_utc=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()))
 (OUT/'resultats.json').write_text(json.dumps(dict(meta=meta,measured=measured),ensure_ascii=False,indent=2)+'\n')
 print(json.dumps(measured,ensure_ascii=False))
