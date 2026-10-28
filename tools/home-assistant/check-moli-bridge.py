#!/usr/bin/env python3
"""Reproduce the first HA/Moli bridge qualification without reaching a house."""
import datetime
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / 'target/ha-bridge-validation'
PACKAGES = ['moli-runtime', 'moli-net', 'moli-overkiz', 'moli-migration',
            'moli-home-assistant', 'moli-api', 'moli-os']
OUTPUT = ROOT / 'docs/proposition/analyse/v6/pont-ha-moli-tests.json'


def main():
    WORK.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, RUSTUP_TOOLCHAIN='1.99.1', CARGO_BUILD_JOBS='2',
               CARGO_TARGET_DIR=str(ROOT / 'target/mqtt-live-probe'), CARGO_TERM_COLOR='never')
    for key in ['MOLI_MASTER_KEY', 'MOLI_MASTER_KEY_FILE', 'MQTT_PASSWORD', 'MOLI_CONFIG']:
        env.pop(key, None)
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')
    report = {'schema': 1, 'date_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'scope': 'synthetic bridge qualification; local MQTT and local HA framework; no real house',
              'physical_commands': 0, 'status': 'FAIL', 'checks': [], 'rust_packages': {}}

    def run(name, command, timeout=600):
        start = time.monotonic()
        completed = subprocess.run(command, cwd=ROOT, env=env, text=True, encoding='utf-8',
                                   errors='replace', stdout=subprocess.PIPE,
                                   stderr=subprocess.STDOUT, timeout=timeout, check=False)
        (WORK / f'{stamp}-{name}.log').write_text(completed.stdout)
        row = {'name': name, 'status': 'PASS' if completed.returncode == 0 else 'FAIL',
               'exit_code': completed.returncode, 'seconds': round(time.monotonic() - start, 3)}
        report['checks'].append(row)
        print(name + ' ' + row['status'], flush=True)
        if completed.returncode:
            print('Échec ; journal privé dans target/ha-bridge-validation.', flush=True)
            raise RuntimeError(name)
        return completed.stdout

    selected = [argument for package in PACKAGES for argument in ['-p', package]]
    try:
        report['rustc'] = run('toolchain', ['rustc', '--version']).strip()
        run('fmt', ['cargo', 'fmt', *selected, '--', '--check'])
        run('clippy', ['cargo', 'clippy', '--offline', *selected,
                       '--all-targets', '-j2', '--', '-D', 'warnings'])
        for package in PACKAGES:
            result = run(package, ['cargo', 'test', '--offline', '-p', package, '-j2', '--quiet'])
            summaries = re.findall(r'test result: ok.*?([0-9]+) passed; ([0-9]+) failed; ([0-9]+) ignored', result)
            if not summaries:
                raise RuntimeError('missing_test_summary')
            report['rust_packages'][package] = {
                'passed': sum(int(row[0]) for row in summaries),
                'failed': sum(int(row[1]) for row in summaries),
                'ignored': sum(int(row[2]) for row in summaries),
                'test_targets': len(summaries),
            }
        run('catalogue', ['cargo', 'run', '--offline', '--quiet', '-p', 'moli-os',
                          '--', 'catalogue', 'check', 'integrations'])
        run('python_protocol', [sys.executable, '-m', 'unittest', 'discover', '-s',
                               'tools/home-assistant/tests_moli_companion', '-p', 'test_protocol.py', '-v'])
        python = ROOT / 'target/ha-companion-venv/bin/python'
        if not python.is_file():
            raise RuntimeError('ha_framework_environment_missing')
        report['home_assistant_version'] = run('ha_version', [str(python), '-c',
            'import importlib.metadata; print(importlib.metadata.version("homeassistant"))']).strip()
        framework = run('python_framework', [str(python), '-m', 'unittest', 'discover', '-s',
                                             'tools/home-assistant/tests_moli_companion',
                                             '-p', 'test_framework.py', '-v'])
        if 'skipped' in framework.lower():
            raise RuntimeError('ha_framework_was_skipped')
        run('i18n', ['node', 'scripts/i18n-check.mjs'])
        run('leak_check', ['sh', 'scripts/leak-check.sh'])
        run('diff_check', ['git', 'diff', '--check'])
        report['status'] = 'PASS'
    except (RuntimeError, subprocess.TimeoutExpired) as error:
        report['failure'] = type(error).__name__
    finally:
        report['finished_utc'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        paths = []
        for directory in ['crates/moli-home-assistant', 'crates/moli-migration',
                          'tools/home-assistant/custom_components/moli',
                          'tools/home-assistant/tests_moli_companion', 'integrations/home-assistant']:
            paths += [p for p in (ROOT / directory).rglob('*')
                      if p.is_file() and p.suffix in {'.rs', '.toml', '.py', '.json', '.md'}]
        paths += [ROOT / path for path in ['Cargo.toml', 'Cargo.lock',
            'crates/moli-runtime/src/write_authority.rs', 'crates/moli-runtime/src/hub.rs',
            'crates/moli-runtime/src/lib.rs', 'crates/moli-runtime/src/supervisor.rs',
            'crates/moli-net/src/lib.rs', 'crates/moli-overkiz/src/api.rs',
            'crates/moli-overkiz/src/lib.rs', 'crates/moli-overkiz/src/fake.rs',
            'crates/moli-overkiz/src/tests.rs', 'crates/moli-overkiz/src/bridge_tests.rs',
            'crates/moli-overkiz/test-fixtures/broker.cjs', 'crates/moli-overkiz/Cargo.toml',
            'crates/moli-overkiz/test-fixtures/package.json',
            'crates/moli-overkiz/test-fixtures/package-lock.json', 'docker/dev.Dockerfile',
            'crates/moli-api/src/home_assistant.rs', 'crates/moli-api/src/lib.rs',
            'crates/moli-api/Cargo.toml', 'crates/moli-os/src/main.rs',
            'crates/moli-os/src/backup.rs', 'crates/moli-os/Cargo.toml',
            'locales/fr/home_assistant.json', 'locales/en/home_assistant.json',
            'tools/home-assistant/check-moli-bridge.py']]
        report['source_sha256'] = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
                                   for p in sorted(set(paths))}
        OUTPUT.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    print('HA_BRIDGE_CHECKS ' + json.dumps({'status': report['status'],
          'rust_passed': sum(row['passed'] for row in report['rust_packages'].values()),
          'physical_commands': 0}), flush=True)
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
