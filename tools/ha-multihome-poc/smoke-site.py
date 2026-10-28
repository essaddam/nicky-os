#!/usr/bin/env python3
"""Run one autonomous site locally; this does not qualify intersite recovery."""
import argparse
import datetime
import ipaddress
import json
from pathlib import Path
import subprocess
import time
import urllib.request


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--config', required=True)
    args = parser.parse_args()
    config = json.loads(Path(args.config).read_text())
    assert config.get('loopback_fixture') is True
    assert all(ipaddress.ip_address(config[key]).is_loopback
               for key in ['bind_ip', 'peer_ip', 'observer_ip'])
    root = Path(__file__).resolve().parents[2]
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    base = f'http://{config["bind_ip"]}:{config["control_port"]}'

    def request(path, payload=None):
        data = json.dumps(payload).encode() if payload is not None else None
        headers = {'Content-Type': 'application/json'} if data is not None else {}
        req = urllib.request.Request(base + path, data=data, headers=headers,
                                     method='POST' if path == '/stop' or data is not None else 'GET')
        with opener.open(req, timeout=2) as response:
            return json.load(response)

    child = subprocess.Popen([str(root / 'target/ha-multihome-poc/debug/moli-ha-multihome-poc'),
                              '--site-bench'], stdin=subprocess.PIPE,
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
    try:
        child.stdin.write(json.dumps(config).encode())
        child.stdin.close()
        deadline = time.monotonic() + 5
        while True:
            try:
                before = request('/status')
                break
            except OSError as error:
                if child.poll() is not None or time.monotonic() > deadline:
                    if child.poll() is not None:
                        details = child.stderr.read().decode(errors='replace')[-2000:]
                    else:
                        details = str(error)
                    raise RuntimeError('local site failed to start: ' + details) from error
                time.sleep(0.1)
        request('/fault', {'mode': 'blackhole'})
        time.sleep(4.2)
        during = request('/status')
        assert during['own_sequence'] > before['own_sequence']
        assert during['effects'] > before['effects'] and during['own_lease_valid']
        assert not during['remote_connected'] and during['physical_commands'] == 0
        request('/freeze', {'enabled': True})
        frozen = request('/status')
        time.sleep(3.3)
        still = request('/status')
        assert still['own_sequence'] == frozen['own_sequence'] and still['own_lease_valid']
        request('/fault', {'mode': 'up'})
        request('/stop')
        assert child.wait(timeout=4) == 0
        receipt = {'status': 'PASS', 'date_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
                   'scope': 'one local macOS loopback fixture; synthetic tiny document; no remote site',
                   'before_sequence': before['own_sequence'], 'after_sequence': during['own_sequence'],
                   'effects': during['effects'], 'lease_valid_during_fault_and_freeze': True,
                   'remote_connected': False, 'physical_commands': 0, 'clean_shutdown': True,
                   'wan_outage_qualified': False}
        (root / 'target/ha-wan/site-smoke.json').write_text(json.dumps(receipt, indent=2) + '\n')
        print('LOCAL_SITE_SMOKE ' + json.dumps(receipt), flush=True)
    finally:
        if child.poll() is None:
            child.terminate()
            try:
                child.wait(timeout=3)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=3)


if __name__ == '__main__':
    main()
