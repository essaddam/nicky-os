#!/usr/bin/env python3
"""Rejouer les contrôles réels dans Docker, sans installation Rust locale."""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import subprocess
import shutil
import tempfile
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
OUT = HERE / 'sorties'
EXPERIMENTS = ('moteur-integre-banc', 'transformations-banc', 'fenetres-temporelles-banc')


def production_files():
    return sorted(set([
        *ROOT.glob('crates/**/*.rs'), *ROOT.glob('crates/**/Cargo.toml'),
        ROOT / 'Cargo.toml', ROOT / 'Cargo.lock', ROOT / 'scripts/check.sh',
    ]))


def fingerprint(paths):
    digest = hashlib.sha256()
    for path in paths:
        digest.update(str(path.relative_to(ROOT)).encode() + b'\0' + path.read_bytes() + b'\0')
    return digest.hexdigest()


def container(command, cpus=2, memory='3g'):
    return [
        'docker', 'run', '--rm', '--network', 'none', '--cpus', str(cpus),
        '--memory', memory, '--tmpfs', '/src/.agentsroom:ro', '-v', str(ROOT) + ':/src',
        '-v', 'moli-cargo:/usr/local/cargo/registry', '-v', 'moli-target:/target',
        '-w', '/src', 'moli-os-dev:latest', *command,
    ]


def record(name, command):
    OUT.mkdir(parents=True, exist_ok=True)
    sources = production_files()
    before = fingerprint(sources)
    started = datetime.datetime.now(datetime.timezone.utc).isoformat()
    image = subprocess.check_output([
        'docker', 'image', 'inspect', 'moli-os-dev:latest', '--format', '{{.Id}}',
    ], text=True).strip()
    begin = time.monotonic()
    print(name + ' : lancement', flush=True)
    with (OUT / (name + '.log')).open('w') as log:
        result = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT)
    if fingerprint(production_files()) != before:
        raise RuntimeError('Les sources de production ont changé pendant le contrôle')
    meta = dict(
        command=[arg.replace(str(ROOT), '${REPO}') for arg in command], started_utc=started,
        elapsed_seconds=round(time.monotonic() - begin, 3), exit_code=result.returncode,
        image_id=image, source_tree_sha256=before, source_files=len(sources),
        scope='Dépôt réel ; données synthétiques ; aucun appareil physique.',
    )
    (OUT / (name + '.json')).write_text(json.dumps(meta, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps(meta, ensure_ascii=False), flush=True)
    if result.returncode:
        print('\n'.join((OUT / (name + '.log')).read_text().splitlines()[-60:]))
        raise SystemExit(result.returncode)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('phase', choices=['workspace', 'build', 'runtime-modes'])
    parser.add_argument('--experiment', choices=EXPERIMENTS)
    args = parser.parse_args()
    if args.phase == 'runtime-modes':
        manifest = 'tools/moteur-integre-banc/Cargo.toml'
        record('moteur-integre-banc-counted-build', container([
            'cargo', 'build', '--offline', '--locked', '--release', '--manifest-path', manifest,
        ]))
        record('moteur-integre-banc-preserve-counted', container([
            'sh', '-c', 'mkdir -p /target/v5 && cp /target/release/moteur-integre-banc /target/v5/moteur-counted',
        ]))
        for phase in ('clippy', 'test', 'build'):
            argv = ['cargo', phase, '--offline', '--locked', '--manifest-path', manifest,
                    '--features', 'plain-timing']
            if phase == 'build':
                argv.append('--release')
            elif phase == 'clippy':
                argv += ['--all-targets', '--', '-D', 'warnings']
            else:
                argv += ['--', '--test-threads=1']
            record('moteur-integre-banc-plain-' + phase, container(argv))
        record('moteur-integre-banc-preserve-plain', container([
            'cp', '/target/release/moteur-integre-banc', '/target/v5/moteur-plain',
        ]))
        return
    if args.phase == 'workspace':
        # The dev image has no git: its fallback leak scan cannot see ignore rules.
        # Feed it the exact public, committable tree, excluding ignored attachments,
        # local binaries and databases. Rust/Cargo files keep their original bytes.
        names = subprocess.check_output([
            'git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z',
        ], cwd=ROOT).decode().split('\0')
        with tempfile.TemporaryDirectory(prefix='.build-public-', dir=HERE) as temporary:
            snapshot = Path(temporary)
            for name in names:
                if not name:
                    continue
                source = ROOT / name
                if not source.is_file() or not source.resolve().is_relative_to(ROOT):
                    continue
                destination = snapshot / name
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(source, destination)
            command = container(['sh', 'scripts/check.sh'], memory='6g')
            command[command.index('-w'):command.index('-w')] = ['-e', 'CARGO_BUILD_JOBS=1']
            command[command.index(str(ROOT) + ':/src')] = str(snapshot) + ':/src'
            record('workspace-check', command)
        return
    for name in ([args.experiment] if args.experiment else EXPERIMENTS):
        manifest = ROOT / 'tools' / name / 'Cargo.toml'
        if not manifest.exists():
            raise SystemExit('Manifest absent : ' + str(manifest))
        lock = manifest.with_name('Cargo.lock')
        if not lock.exists():
            lock.write_bytes((ROOT / 'Cargo.lock').read_bytes())
        path = str(manifest.relative_to(ROOT))
        record(name + '-resolve', container(['cargo', 'check', '--offline', '--manifest-path', path]))
        record(name + '-fmt', container(['cargo', 'fmt', '--manifest-path', path, '--', '--check']))
        record(name + '-clippy', container([
            'cargo', 'clippy', '--offline', '--locked', '--manifest-path', path,
            '--all-targets', '--', '-D', 'warnings',
        ]))
        record(name + '-tests', container([
            'cargo', 'test', '--offline', '--locked', '--manifest-path', path,
            '--', '--test-threads=1',
        ]))
        record(name + '-release', container([
            'cargo', 'build', '--offline', '--locked', '--release', '--manifest-path', path,
        ]))


if __name__ == '__main__':
    main()
