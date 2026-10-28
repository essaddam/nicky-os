#!/usr/bin/env python3
"""Build and run the two-house process bench using private HA export shapes."""
import argparse,datetime,hashlib,json,os,subprocess,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'tools/mqtt-live-probe'))
from importlib.util import spec_from_file_location,module_from_spec
spec=spec_from_file_location('load_runner',ROOT/'tools/mqtt-live-probe/run-load.py');helper=module_from_spec(spec);spec.loader.exec_module(helper)
def main():
 p=argparse.ArgumentParser();p.add_argument('--serve',action='store_true');p.add_argument('--network-loss',action='store_true');p.add_argument('--skip-check',action='store_true');p.add_argument('--fixture',action='store_true');p.add_argument('--port',type=int,default=1416);a=p.parse_args()
 env=dict(os.environ,RUSTUP_TOOLCHAIN='1.99.1',CARGO_BUILD_JOBS='2',CARGO_TERM_COLOR='never');env.pop('MQTT_PASSWORD',None);env.pop('MOLI_MASTER_KEY',None);env.pop('MOLI_ENUM_CATALOGUE_DIR',None)
 probeenv=dict(env,CARGO_TARGET_DIR=str(ROOT/'target/mqtt-live-probe'));haenv=dict(env,CARGO_TARGET_DIR=str(ROOT/'target/ha-multihome-poc'))
 if not a.fixture:
  print('Compilation de l’export anonyme du pont ; aucune commande physique.',flush=True)
  subprocess.run(['cargo','build','--offline','--manifest-path','tools/mqtt-live-probe/Cargo.toml','--bin','mqtt-live-probe','-j2'],cwd=ROOT,env=probeenv,check=True)
 print('Compilation et contrôles du POC HA.',flush=True)
 manifest=['--manifest-path','tools/ha-multihome-poc/Cargo.toml']
 for command in ([] if a.skip_check else [['cargo','fmt',*manifest,'--','--check'],['cargo','clippy','--offline',*manifest,'--all-targets','-j2','--','-D','warnings'],['cargo','test','--offline',*manifest,'-j2'],['cargo','build','--offline',*manifest,'-j2']]):subprocess.run(command,cwd=ROOT,env=haenv,check=True)
 payload=b'';fingerprint=None
 if not a.fixture:
  exports=helper.sources(None);fingerprint,_=helper.input_fingerprint(exports)
  result=subprocess.run([str(ROOT/'target/mqtt-live-probe/debug/mqtt-live-probe'),'--ha-seed',*[str(x) for x in exports]],cwd=ROOT,env=probeenv,capture_output=True,check=False)
  if result.returncode or len(result.stdout)>20*1024*1024:raise RuntimeError('Export HA refusé ; aucun contenu privé affiché.')
  payload=result.stdout
 binary=ROOT/'target/ha-multihome-poc/debug/moli-ha-multihome-poc'
 if a.serve:
  proc=subprocess.Popen([str(binary),'--serve','--port',str(a.port)],cwd=ROOT,env=haenv,stdin=subprocess.PIPE,start_new_session=True)
  try:proc.stdin.write(payload);proc.stdin.close();payload=b'';raise SystemExit(proc.wait())
  except KeyboardInterrupt:
   proc.terminate()
   try:proc.wait(timeout=15)
   except subprocess.TimeoutExpired:proc.kill();proc.wait(timeout=5)
   raise SystemExit(0)
 else:
  print('Test de coupure TCP et de silence réseau sur les répliques.' if a.network_loss else 'Campagne réelle de processus : copies, coupures WAN, pannes et reprises simulées.',flush=True)
  run=subprocess.run([str(binary),'--tcp-loss' if a.network_loss else '--bench'],input=payload,cwd=ROOT,env=haenv,capture_output=True,timeout=180)
  if run.returncode:print(run.stderr.decode()[-5000:],file=sys.stderr);raise RuntimeError('Banc HA en échec')
  prefix='TCP_LOSS_REPORT ' if a.network_loss else 'HA_REPORT '
  report=json.loads(next(line[len(prefix):] for line in run.stdout.decode().splitlines() if line.startswith(prefix)))
  report.update(date_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),profile='debug',platform=sys.platform,inputs_sha256=fingerprint,binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest())
  dest=ROOT/('docs/proposition/analyse/v6/ha-multimaison-tcp.json' if a.network_loss else 'docs/proposition/analyse/v6/ha-multimaison-resultats.json');dest.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
  print('HA_SUMMARY '+json.dumps({'cases':[(x['case'],x['status']) for x in report['cases']],'physical_commands':0}),flush=True)
if __name__=='__main__':main()
