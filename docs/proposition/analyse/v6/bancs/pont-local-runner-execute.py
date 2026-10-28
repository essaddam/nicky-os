#!/usr/bin/env python3
"""Exercise real bridge faults between two autonomous loopback site processes."""
import argparse
import datetime
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import secrets
import socket
import subprocess
import sys
import threading
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
OUTPUT = ROOT / 'docs/proposition/analyse/v6/ha-multimaison-pont-local.json'
WORK = ROOT / 'target/ha-local-link'
BINARY = ROOT / 'target/ha-multihome-poc/debug/moli-ha-multihome-poc'
PROBE = ROOT / 'target/mqtt-live-probe/debug/mqtt-live-probe'
NODES = ('node-a', 'node-b')
HOUSES = {'node-a': 'house-a', 'node-b': 'house-b'}
PEERS = {'node-a': 'node-b', 'node-b': 'node-a'}
CASES = (
    ('bilateral_cut_8s', 'cut', NODES, 8),
    ('bilateral_blackhole_3s', 'blackhole', NODES, 3),
    ('one_way_cut_8s', 'cut', ('node-a',), 8),
    ('bilateral_blackhole_120s', 'blackhole', NODES, 120),
)


class Failure(Exception):
    def __init__(self, code):
        self.code = code
        super().__init__(code)


def require(condition, code):
    if not condition:
        raise Failure(code)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()


def save(report):
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    temporary = OUTPUT.with_suffix('.json.tmp')
    temporary.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    temporary.replace(OUTPUT)


def private_log(name):
    WORK.mkdir(parents=True, exist_ok=True, mode=0o700)
    os.chmod(WORK, 0o700)
    descriptor = os.open(WORK / name, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    return os.fdopen(descriptor, 'wb')


def environment(target):
    env = dict(os.environ, RUSTUP_TOOLCHAIN='1.99.1', CARGO_BUILD_JOBS='2',
               CARGO_TARGET_DIR=str(ROOT / 'target' / target), CARGO_TERM_COLOR='never')
    for name in ('MQTT_PASSWORD', 'MOLI_MASTER_KEY', 'MOLI_ENUM_CATALOGUE_DIR'):
        env.pop(name, None)
    return env


def load_helper():
    spec = importlib.util.spec_from_file_location(
        'ha_link_load_helpers', ROOT / 'tools/mqtt-live-probe/run-load.py')
    helper = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helper)
    return helper


def prepare(args, report, run_id):
    if not args.skip_check:
        print('STATUS Contrôles du prototype local.', flush=True)
        with private_log(run_id + '-checks.log') as log:
            result = subprocess.run([sys.executable, str(ROOT / 'tools/ha-multihome-poc/check-sites.py')],
                                    cwd=ROOT, env=environment('ha-multihome-poc'),
                                    stdout=log, stderr=subprocess.STDOUT, check=False)
        require(result.returncode == 0, 'local_checks_failed')
    require(BINARY.is_file(), 'site_binary_missing')
    if args.fixture:
        bundle = {'v': 1, 'kind': 'synthetic_fixture', 'houses': [
            {'id': house, 'document': {'temperature': {'value': 21, 'unit': 'fixture'},
                                      'humidity': {'value': 45, 'unit': 'fixture'}},
             'summary': {'house': house, 'kind': 'synthetic_fixture',
                         'entities': 1, 'devices': 1, 'points': 2}}
            for house in HOUSES.values()]}
        report['inputs_sha256'] = hashlib.sha256(encoded(bundle)).hexdigest()
    else:
        helper = load_helper()
        try:
            exports = helper.sources(args.exports)
            report['inputs_sha256'], _ = helper.input_fingerprint(exports)
        except Exception:
            raise Failure('private_export_discovery_failed') from None
        probe_inputs = list((ROOT / 'tools/mqtt-live-probe/src').rglob('*.rs'))
        probe_inputs += [ROOT / 'tools/mqtt-live-probe/Cargo.toml',
                         ROOT / 'tools/mqtt-live-probe/Cargo.lock']
        rebuild = (not PROBE.exists() or any(path.exists() and path.stat().st_mtime > PROBE.stat().st_mtime
                                           for path in probe_inputs))
        if rebuild:
            print('STATUS Préparation de l’export normalisé.', flush=True)
            with private_log(run_id + '-probe-build.log') as log:
                result = subprocess.run(['cargo', 'build', '--offline', '--manifest-path',
                                         'tools/mqtt-live-probe/Cargo.toml', '--bin',
                                         'mqtt-live-probe', '-j2'], cwd=ROOT,
                                        env=environment('mqtt-live-probe'), stdout=log,
                                        stderr=subprocess.STDOUT, check=False)
            require(result.returncode == 0, 'probe_build_failed')
        with private_log(run_id + '-seed-error.log') as log:
            result = subprocess.run([str(PROBE), '--ha-seed', *map(str, exports)], cwd=ROOT,
                                    env=environment('mqtt-live-probe'), stdout=subprocess.PIPE,
                                    stderr=log, timeout=60, check=False)
        require(result.returncode == 0 and len(result.stdout) <= 20 * 1024 * 1024,
                'normalized_export_failed')
        try:
            bundle = json.loads(result.stdout)
        except (ValueError, UnicodeError):
            raise Failure('normalized_export_invalid') from None
        report['probe_binary_sha256'] = sha(PROBE)
    require(bundle.get('v') == 1 and [house['id'] for house in bundle['houses']]
            == ['house-a', 'house-b'], 'seed_identity_mismatch')
    report['bundle_sha256'] = hashlib.sha256(encoded(bundle)).hexdigest()
    allowed = ('house', 'kind', 'entities', 'devices', 'points', 'nodes', 'observed_points',
               'description_bytes', 'snapshot_bytes', 'physical_commands', 'synthetic_identities')
    report['sources'] = [{key: house['summary'][key] for key in allowed if key in house['summary']}
                         for house in bundle['houses']]
    report['binary_sha256'] = sha(BINARY)
    paths = sorted((ROOT / 'tools/ha-multihome-poc/src').glob('*.rs'))
    paths += [ROOT / name for name in (
        'tools/ha-multihome-poc/Cargo.toml', 'tools/ha-multihome-poc/Cargo.lock',
        'tools/moteur-integre-banc/src/memory.rs',
        'tools/mqtt-live-probe/src/gateway_runtime/broker.rs',
        'tools/mqtt-live-probe/src/ha_seed.rs')]
    report['source_sha256'] = {str(path.relative_to(ROOT)): sha(path) for path in paths}
    receipt = ROOT / 'target/ha-wan/native-check.json'
    if receipt.exists():
        checked = json.loads(receipt.read_text())
        report['native_checks'] = {key: checked[key] for key in (
            'date_utc', 'checks', 'source_sha256', 'binary_sha256', 'archive_sha256', 'scope')
                                   if key in checked}
        report['native_checks_sha256'] = sha(receipt)
        report['native_checks_match_binary'] = checked.get('binary_sha256') == report['binary_sha256']
        require(report['native_checks_match_binary'], 'checked_binary_mismatch')
        require(all(sha(ROOT / path) == digest
                    for path, digest in checked.get('source_sha256', {}).items()),
                'checked_sources_mismatch')
    return bundle


class Meter:
    """Bounded ps sampling of one autonomous site, excluding the Python runner."""
    def __init__(self, pid):
        self.pid = pid
        self.started = time.monotonic()
        self.samples = []
        self.lock = threading.Lock()
        self.done = threading.Event()
        self.thread = threading.Thread(target=self._loop, daemon=True)
        self.thread.start()

    def sample(self):
        with self.lock:
            try:
                result = subprocess.run(['ps', '-o', 'rss=,time=', '-p', str(self.pid)],
                                        capture_output=True, text=True, timeout=2, check=False)
                fields = result.stdout.split()
                if result.returncode or len(fields) != 2:
                    return None
                cpu = 0.0
                for part in fields[1].split(':'):
                    cpu = cpu * 60 + float(part)
                sample = {'seconds': round(time.monotonic() - self.started, 3),
                          'rss_bytes': int(fields[0]) * 1024, 'cpu_seconds': cpu}
                self.samples.append(sample)
                del self.samples[:-256]
                return dict(sample)
            except (OSError, ValueError, subprocess.TimeoutExpired):
                return None

    def _loop(self):
        while not self.done.is_set():
            self.sample()
            self.done.wait(2)

    def finish(self):
        self.done.set()
        self.thread.join(timeout=3)
        with self.lock:
            samples = [dict(sample) for sample in self.samples]
        result = {'sampling_period_seconds': 2, 'max_samples': 256, 'samples': samples,
                  'scope': 'one autonomous site process; ticker, broker and RPC included; runner excluded'}
        if samples:
            result['rss_peak_sampled_bytes'] = max(sample['rss_bytes'] for sample in samples)
        if len(samples) >= 2:
            elapsed = max(0.001, samples[-1]['seconds'] - samples[0]['seconds'])
            result['cpu_percent_one_core'] = max(0, samples[-1]['cpu_seconds'] - samples[0]['cpu_seconds']) / elapsed * 100
            result['sampled_seconds'] = elapsed
        return result


class Site:
    def __init__(self, node, config, run_id):
        self.node = node
        self.base = f'http://127.0.0.1:{config["control_port"]}'
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        self.logs = [private_log(f'{run_id}-{node}-{kind}.log') for kind in ('stdout', 'stderr')]
        self.child = None
        self.meter = None
        try:
            self.child = subprocess.Popen([str(BINARY), '--site-bench'], cwd=ROOT,
                                          env=environment('ha-multihome-poc'), stdin=subprocess.PIPE,
                                          stdout=self.logs[0], stderr=self.logs[1], start_new_session=True)
            self.meter = Meter(self.child.pid)
            self.child.stdin.write(encoded(config))
            self.child.stdin.close()
        except BaseException:
            if self.child is not None:
                self.child.kill()
                self.child.wait(timeout=5)
            if self.meter is not None:
                self.meter.finish()
            for log in self.logs:
                log.close()
            raise

    def request(self, path, body=None, timeout=3):
        data = encoded(body) if body is not None else None
        request = urllib.request.Request(self.base + path, data=data,
                                        headers={'Content-Type': 'application/json'} if data is not None else {},
                                        method='POST' if path == '/stop' or body is not None else 'GET')
        try:
            with self.opener.open(request, timeout=timeout) as response:
                payload = response.read(64 * 1024 + 1)
            require(len(payload) <= 64 * 1024, 'status_byte_limit')
            return json.loads(payload)
        except Failure:
            raise
        except Exception:
            raise Failure('local_control_unavailable_' + self.node) from None

    def close(self):
        outcome = {'stop_requested': False, 'reaped': False, 'forced_termination': False}
        try:
            if self.child.poll() is None:
                try:
                    self.request('/stop')
                    outcome['stop_requested'] = True
                except Failure:
                    pass
                try:
                    self.child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    outcome['forced_termination'] = True
                    self.child.terminate()
                    try:
                        self.child.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        self.child.kill()
                        self.child.wait(timeout=3)
            else:
                self.child.wait(timeout=1)
            outcome['exit_code'] = self.child.returncode
            outcome['reaped'] = self.child.poll() is not None
        finally:
            if self.child.poll() is None:
                outcome['forced_termination'] = True
                self.child.kill()
                self.child.wait(timeout=3)
                outcome.update(reaped=True, exit_code=self.child.returncode)
            self.resources = self.meter.finish()
            for log in self.logs:
                log.close()
        return outcome


def ports():
    held = []
    try:
        for _ in range(4):
            listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            listener.bind(('127.0.0.1', 0))
            held.append(listener)
        values = [listener.getsockname()[1] for listener in held]
        require(len(set(values)) == 4 and not set(values) & {1414, 1415, 1416, 1883, 1884, 8790},
                'reserved_local_port')
        return values
    finally:
        for listener in held:
            listener.close()


def copy(status, house):
    rows = [row for row in status.get('copies', []) if row.get('house') == house]
    require(len(rows) == 1, 'copy_identity_mismatch')
    return rows[0]


def health(status, node):
    require(status.get('node') == node and status.get('house') == HOUSES[node], 'site_identity_mismatch')
    require(status.get('network_scope') == 'loopback_fixture', 'unexpected_network_scope')
    require(status.get('physical_commands') == 0 and status.get('frozen') is False, 'unexpected_executor_or_freeze')
    require(status.get('own_lease_valid') is True, 'local_lease_invalid_' + node)
    require(status.get('effects') == status.get('own_sequence'), 'effect_admission_sequence_mismatch')
    require(status.get('ring_len', 257) <= 256 and status.get('ring_bytes', 1048577) <= 1048576,
            'history_bound_exceeded')
    gate = status.get('fault', {})
    require(gate.get('active_connections', 17) <= 16 and gate.get('max_active_seen', 17) <= 16,
            'peer_connection_bound_exceeded')
    require(gate.get('gate_buffer_bytes_per_direction') == 0, 'gate_buffer_bound_exceeded')
    for house in HOUSES.values():
        row = copy(status, house)
        require(row.get('epoch') == 1 and row.get('records', 0) > 0, 'invalid_house_epoch_or_empty_copy')
        require(row.get('role') == ('active' if house == HOUSES[node] else 'passive'), 'house_role_changed')
    own = copy(status, HOUSES[node])
    require(own['sequence'] == status['own_sequence'] and own['sha256'] == status['own_sha256'],
            'primary_summary_mismatch')


def statuses(sites):
    result = {node: sites[node].request('/status') for node in NODES}
    for node, status in result.items():
        health(status, node)
    return result


def wait_ready(sites):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        require(all(site.child.poll() is None for site in sites.values()), 'site_exited_during_startup')
        try:
            return statuses(sites)
        except Failure:
            time.sleep(0.2)
    raise Failure('site_startup_deadline')


def mark(status, house=None):
    row = copy(status, house) if house is not None else {
        'sequence': status['own_sequence'], 'sha256': status['own_sha256']}
    return {'sequence': row['sequence'], 'sha256': row['sha256']}


def converge(sites):
    deadline = time.monotonic() + 30
    def fetch(node):
        remaining = deadline - time.monotonic()
        require(remaining > 0, 'recovery_deadline_30s')
        value = sites[node].request('/status', timeout=min(3, remaining))
        require(time.monotonic() < deadline, 'recovery_deadline_30s')
        return value
    while time.monotonic() < deadline:
        proofs = {}
        for source in NODES:
            peer, house = PEERS[source], HOUSES[source]
            first = fetch(source)
            middle = fetch(peer)
            last = fetch(source)
            for node, status in ((source, first), (peer, middle), (source, last)):
                health(status, node)
            points = [mark(first), mark(middle, house), mark(last)]
            if points[0] == points[1] == points[2]:
                proofs[house] = {'source': source, 'replica_node': peer,
                                 'source_before': points[0], 'replica': points[1],
                                 'source_after': points[2]}
        if len(proofs) == 2:
            snapshots = {node: fetch(node) for node in NODES}
            for node, status in snapshots.items():
                health(status, node)
            return snapshots, proofs
        time.sleep(0.25)
    raise Failure('recovery_deadline_30s')


def resources(sites, snapshots):
    return {node: {'process_sample': site.meter.sample(),
                   'allocator_live_bytes': snapshots[node]['allocator_live_bytes'],
                   'allocator_peak_bytes': snapshots[node]['allocator_peak_bytes']}
            for node, site in sites.items()}


def exercise(sites, descriptor, quick):
    name, mode, faulted, requested = descriptor
    duration = 12 if quick and requested == 120 else requested
    case = {'case': name, 'status': 'FAIL', 'fault_mode': mode, 'faulted_entries': list(faulted),
            'requested_fault_seconds': duration, 'quick': quick, 'physical_commands': 0}
    print('CASE_BEGIN ' + name, flush=True)
    before, _ = converge(sites)
    case['before'] = before
    begin = time.monotonic()
    next_progress = begin + 20
    started = {}
    restored = {}
    try:
        for node in faulted:
            started[node] = time.monotonic()
            sites[node].request('/fault', {'mode': mode})
        hold_started = time.monotonic()
        time.sleep(0.2)
        settled = statuses(sites)
        case['settled'] = settled
        poll_count = 0
        while time.monotonic() < hold_started + duration:
            current = statuses(sites)
            for node in faulted:
                require(current[node]['fault']['mode'] == mode, 'fault_mode_changed')
                require(current[node]['fault']['forwarded_bytes'] == settled[node]['fault']['forwarded_bytes'],
                        'fault_forwarded_bytes')
                foreign = HOUSES[PEERS[node]]
                require(mark(current[node], foreign) == mark(settled[node], foreign), 'faulted_passive_progressed')
            poll_count += 1
            if time.monotonic() >= next_progress:
                print('CASE_PROGRESS ' + json.dumps({'case': name, 'elapsed_seconds': round(time.monotonic() - begin)}), flush=True)
                next_progress += 20
            time.sleep(min(0.25, max(0, hold_started + duration - time.monotonic())))
        during = statuses(sites)
        case['during'] = during
        case['fault_poll_count'] = poll_count
        minimum = max(1, math.floor(duration / 2) - 1)
        checks = {}
        for node in NODES:
            gain = during[node]['own_sequence'] - before[node]['own_sequence']
            effect_gain = during[node]['effects'] - before[node]['effects']
            require(gain >= minimum and effect_gain >= minimum, 'local_progress_insufficient_' + node)
            foreign = HOUSES[PEERS[node]]
            frozen = mark(during[node], foreign) == mark(settled[node], foreign)
            if node in faulted:
                require(frozen, 'faulted_passive_changed_at_end')
                require(during[node]['fault']['forwarded_bytes'] == settled[node]['fault']['forwarded_bytes'],
                        'fault_forwarded_at_end')
                if mode == 'cut':
                    require(during[node]['fault']['active_connections'] == 0, 'fault_socket_state_wrong')
                elif requested == 3:
                    require(during[node]['fault']['active_connections'] >= 1, 'fault_socket_state_wrong')
            else:
                require(during[node]['fault']['mode'] == 'up', 'healthy_entry_faulted')
                require(during[node]['copies'] and copy(during[node], foreign)['sequence']
                        > copy(settled[node], foreign)['sequence'], 'unaffected_passive_stopped')
            checks[node] = {'local_sequence_gain': gain, 'effect_admission_gain': effect_gain,
                            'minimum_local_gain': minimum, 'passive_frozen': frozen,
                            'lease_valid': during[node]['own_lease_valid']}
        case['checks'] = checks
        for node in faulted:
            sites[node].request('/fault', {'mode': 'up'})
            restored[node] = time.monotonic()
        case['fault_duration_ms'] = round((max(restored.values()) - begin) * 1000)
        case['fault_duration_by_entry_ms'] = {node: round((restored[node] - started[node]) * 1000) for node in faulted}
        recovery_started = time.monotonic()
        recovered, proof = converge(sites)
        case['recovery_ms'] = round((time.monotonic() - recovery_started) * 1000)
        case['recovered'] = recovered
        case['convergence'] = proof
        for target in faulted:
            source = PEERS[target]
            if mode == 'cut':
                require(recovered[target]['fault']['generation'] > before[target]['fault']['generation'],
                        'cut_generation_not_advanced')
                require(recovered[target]['fault']['accepted_connections'] > before[target]['fault']['accepted_connections'],
                        'cut_connection_not_replaced')
                require(recovered[source]['replica_rpc']['connections'] > before[source]['replica_rpc']['connections'],
                        'cut_rpc_not_reconnected')
            if requested == 120 and not quick:
                gap = during[source]['own_sequence'] - copy(during[target], HOUSES[source])['sequence']
                require(gap > 16, 'long_gap_did_not_cross_snapshot_threshold')
                require(recovered[source]['snapshot_recoveries'] > before[source]['snapshot_recoveries'],
                        'long_recovery_did_not_use_snapshot')
            else:
                require(recovered[source]['delta_recoveries'] > before[source]['delta_recoveries'],
                        'short_recovery_did_not_use_deltas')
        same = requested == 3 and mode == 'blackhole'
        case['same_socket_check'] = {'required': same, 'passed': None}
        if same:
            for node in NODES:
                for sample in (settled[node], during[node], recovered[node]):
                    require(sample['fault']['accepted_connections'] == before[node]['fault']['accepted_connections']
                            and sample['fault']['generation'] == before[node]['fault']['generation']
                            and sample['replica_rpc']['connections'] == before[node]['replica_rpc']['connections'],
                            'short_blackhole_reconnected')
            case['same_socket_check']['passed'] = True
        case['resources'] = resources(sites, recovered)
        case['status'] = 'PASS'
        print('CASE_PASS ' + name, flush=True)
    except Exception as error:
        case['failure'] = {'code': error.code if isinstance(error, Failure) else type(error).__name__}
        try:
            case['failure_status'] = {node: sites[node].request('/status') for node in NODES}
        except Exception:
            pass
        print('CASE_FAIL ' + name, flush=True)
    finally:
        for node in faulted:
            if node not in restored:
                try:
                    sites[node].request('/fault', {'mode': 'up'})
                except Exception:
                    pass
        if 'fault_duration_ms' not in case:
            case['fault_duration_ms'] = round((time.monotonic() - begin) * 1000)
    return case


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--skip-check', action='store_true')
    parser.add_argument('--fixture', action='store_true')
    parser.add_argument('--quick', action='store_true', help='12-second debug long case; never qualifies 120 seconds')
    parser.add_argument('--exports', type=Path)
    parser.add_argument('--measure-runner', action='store_true')
    args = parser.parse_args()
    run_id = secrets.token_hex(8)
    report = {'schema': 1, 'date_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'scope': 'two autonomous instances and two brokers on one Mac; loopback bridge faults; no WAN or VPN claim',
              'profile': 'debug', 'platform': sys.platform, 'quick': args.quick, 'fixture': args.fixture,
              'qualification_120s': False, 'physical_commands': 0, 'checks_skipped': args.skip_check,
              'runner_sha256': sha(Path(__file__)), 'cases': [], 'resources': {}, 'cleanup': {},
              'sources': [], 'status': 'FAIL', 'effects_admission_only': True,
              'intermediate_delivery_proven': False,
              'limitations': ['one_host_not_two_failure_domains', 'replayed_exports_not_live_ha',
                              'no_physical_executor', 'effect_admission_not_device_delivery',
                              'no_intermediate_state_delivery_claim', 'quick_never_qualifies_120s']}
    sites = {}
    runner_meter = Meter(os.getpid()) if args.measure_runner else None
    stage = 'prepare'
    try:
        bundle = prepare(args, report, run_id)
        mqtt_a, http_a, mqtt_b, http_b = ports()
        report['ports'] = {'node-a': {'mqtt': mqtt_a, 'control': http_a},
                           'node-b': {'mqtt': mqtt_b, 'control': http_b}}
        namespace = 'moli-probe/ha-bench/' + run_id
        stage = 'start'
        for node, mqtt, peer_mqtt, control in (
                ('node-a', mqtt_a, mqtt_b, http_a), ('node-b', mqtt_b, mqtt_a, http_b)):
            config = {'node': node, 'house': HOUSES[node], 'bind_ip': '127.0.0.1',
                      'peer_ip': '127.0.0.1', 'observer_ip': '127.0.0.1', 'loopback_fixture': True,
                      'mqtt_port': mqtt, 'peer_mqtt_port': peer_mqtt, 'control_port': control,
                      'prefix': namespace, 'bundle': bundle, 'ttl_seconds': 420}
            sites[node] = Site(node, config, run_id)
        wait_ready(sites)
        stage = 'cases'
        for descriptor in CASES:
            case = exercise(sites, descriptor, args.quick)
            report['cases'].append(case)
            save(report)
            if case['status'] != 'PASS':
                raise Failure('case_failed_' + case['case'])
    except BaseException as error:
        report['failure'] = {'stage': stage, 'code': error.code if isinstance(error, Failure) else type(error).__name__}
        print('STATUS Échec enregistré ; nettoyage des instances.', flush=True)
    finally:
        for node, site in sites.items():
            try:
                report['cleanup'][node] = site.close()
                report['resources'][node] = site.resources
            except Exception:
                report['cleanup'][node] = {'reaped': site.child.poll() is not None, 'cleanup_error': True}
        if runner_meter is not None:
            report['runner_resources'] = runner_meter.finish()
            report['runner_resources']['scope'] = 'Python coordinator only; autonomous site processes excluded'
        completed = {case['case'] for case in report['cases']}
        for name, mode, faulted, seconds in CASES:
            if name not in completed:
                report['cases'].append({'case': name, 'status': 'FAIL', 'fault_mode': mode,
                                        'faulted_entries': list(faulted),
                                        'requested_fault_seconds': 12 if args.quick and seconds == 120 else seconds,
                                        'failure': {'code': 'not_executed_after_failure'}, 'physical_commands': 0})
        clean = (set(report['cleanup']) == set(NODES)
                 and all(row.get('reaped') and row.get('exit_code') == 0
                         and not row.get('forced_termination') for row in report['cleanup'].values()))
        report['cleanup_complete'] = clean
        passed = (all(case['status'] == 'PASS' for case in report['cases'])
                  and len(report['cases']) == 4 and 'failure' not in report)
        report['status'] = 'PASS' if passed and clean else 'FAIL'
        report['qualification_120s'] = report['status'] == 'PASS' and not args.quick and not args.fixture
        report['finished_utc'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        save(report)
    print('LINK_FAULTS_RESULT ' + json.dumps({'status': report['status'], 'cases': [
        {'case': case['case'], 'status': case['status']} for case in report['cases']],
        'qualification_120s': report['qualification_120s'], 'cleanup_complete': report['cleanup_complete'],
        'physical_commands': 0}), flush=True)
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
