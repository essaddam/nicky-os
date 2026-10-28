#!/usr/bin/env python3
"""Validate and package the opt-in two-host qualification mode."""
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tarfile


def main():
    root = Path(__file__).resolve().parents[2]
    output = root / 'target/ha-wan'
    output.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, RUSTUP_TOOLCHAIN='1.99.1', CARGO_BUILD_JOBS='2',
               CARGO_TARGET_DIR=str(root / 'target/ha-multihome-poc'),
               CARGO_TERM_COLOR='never')
    manifest = ['--manifest-path', 'tools/ha-multihome-poc/Cargo.toml']
    commands = [
        ['cargo', 'fmt', *manifest, '--', '--check'],
        ['cargo', 'clippy', '--offline', *manifest, '--all-targets', '-j2', '--', '-D', 'warnings'],
        ['cargo', 'test', '--offline', *manifest, '-j2'],
        ['cargo', 'build', '--offline', *manifest, '-j2'],
    ]
    for command in commands:
        subprocess.run(command, cwd=root, env=env, check=True)
    sources = sorted((root / 'tools/ha-multihome-poc/src').glob('*.rs'))
    sources.extend(root / path for path in [
        'tools/ha-multihome-poc/Cargo.toml', 'tools/ha-multihome-poc/Cargo.lock',
        'tools/moteur-integre-banc/src/memory.rs',
        'tools/mqtt-live-probe/src/gateway_runtime/broker.rs',
    ])
    with tarfile.open(output / 'source.tgz', 'w:gz') as archive:
        for path in sources:
            archive.add(path, arcname=str(path.relative_to(root)))
    sha = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
    receipt = {
        'date_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
        'checks': ['fmt', 'clippy -D warnings', 'Rust unit tests', 'native debug build'],
        'source_sha256': {str(path.relative_to(root)): sha(path) for path in sources},
        'binary_sha256': sha(root / 'target/ha-multihome-poc/debug/moli-ha-multihome-poc'),
        'archive_sha256': sha(output / 'source.tgz'),
        'scope': 'local checks only; no WAN outage is claimed by this receipt',
    }
    (output / 'native-check.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print('SITE_CHECKS_PASS; Linux source archive ready; WAN behavior not yet qualified.', flush=True)


if __name__ == '__main__':
    main()
