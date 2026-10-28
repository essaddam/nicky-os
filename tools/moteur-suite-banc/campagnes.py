#!/usr/bin/env python3
"""Mesures séquentielles de binaires gelés ; sources et résultats archivés."""
import datetime
import hashlib
import importlib.util
import json
from pathlib import Path
import shlex
import shutil
import subprocess
import zipfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
IMAGE = 'moli-os-dev:latest'
PREFIX = ['docker', 'run', '--rm', '--network', 'none', '--cpus', '1', '--memory',
          '512m', '-v', 'moli-target:/target:ro', IMAGE]
BINARIES = ['/target/v5/moteur-counted', '/target/v5/moteur-plain',
            '/target/release/transformations-banc', '/target/release/index-memory',
            '/target/release/structures', '/target/release/production']


def run(command, **kwargs):
    return subprocess.run(command, check=True, text=True, capture_output=True, **kwargs)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')
    output = HERE / 'sorties/campagnes' / stamp
    output.mkdir(parents=True, exist_ok=False)
    image = run(['docker', 'image', 'inspect', IMAGE, '--format', '{{.Id}}']).stdout.strip()
    toolchain = run(PREFIX + ['sh', '-c', 'rustc --version --verbose && uname -m']).stdout
    binaries_before = run(PREFIX + ['sha256sum', *BINARIES]).stdout
    module_spec = importlib.util.spec_from_file_location('suite_executer', HERE / 'executer.py')
    module = importlib.util.module_from_spec(module_spec)
    module_spec.loader.exec_module(module)
    production = module.fingerprint(module.production_files())
    metadata = dict(
        timestamp_utc=stamp, image_id=image, toolchain=toolchain,
        runtime_environment='Linux aarch64, Docker, réseau none, 1 CPU, plafond 512 Mio',
        production_source_sha256=production, binary_sha256_before=binaries_before,
        profile='release opt-level=s lto=fat codegen-units=1',
        scope='Chemins réels et modèles proposés distincts ; aucun appareil physique.',
    )
    (output / 'environnement.json').write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + '\n')
    paths = {}
    for name in module.EXPERIMENTS:
        directory = ROOT / 'tools' / name / 'sorties/campagnes' / stamp
        paths[name] = directory
        if name != 'fenetres-temporelles-banc':
            directory.mkdir(parents=True, exist_ok=False)
    print('Campagne ' + stamp, flush=True)
    engine = paths['moteur-integre-banc']
    engine_command = ['python3', 'tools/moteur-integre-banc/executer.py',
                      '--counted-command', shlex.join(PREFIX + [BINARIES[0]]),
                      '--timing-command', shlex.join(PREFIX + [BINARIES[1]]),
                      '--output', str(engine)]
    with (output / 'moteur.log').open('w') as log:
        subprocess.run(engine_command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True)
    print('108 processus du moteur : terminés.', flush=True)
    transforms = paths['transformations-banc']
    for repetition in range(1, 4):
        for filename, binary, args in [
            ('mesures', BINARIES[2], ['20000', '7']),
            ('memoire-index', BINARIES[3], []),
        ]:
            result = run(PREFIX + [binary, *args])
            (transforms / f'{filename}-{repetition}.jsonl').write_text(result.stdout)
            (transforms / f'{filename}-{repetition}.stderr.txt').write_text(result.stderr)
            for line in result.stdout.splitlines():
                json.loads(line)
    print('Transformations : trois processus neufs par chemin terminés.', flush=True)
    windows = paths['fenetres-temporelles-banc']
    window_command = ['python3', 'tools/fenetres-temporelles-banc/executer.py',
                      '--structures', shlex.join(PREFIX + [BINARIES[4]]),
                      '--production', shlex.join(PREFIX + [BINARIES[5]]),
                      '--metadata', str(output / 'environnement.json'), '--output', str(windows)]
    with (output / 'fenetres.log').open('w') as log:
        subprocess.run(window_command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True)
    print('36 processus de fenêtres + 3 preuves production : terminés.', flush=True)
    binaries_after = run(PREFIX + ['sha256sum', *BINARIES]).stdout
    assert binaries_before == binaries_after, 'Un binaire a changé pendant la campagne'
    assert production == module.fingerprint(module.production_files()), 'Sources de production modifiées'
    source_files = module.production_files()
    for name in module.EXPERIMENTS:
        folder = ROOT / 'tools' / name
        source_files += [p for p in folder.rglob('*') if p.is_file()
                         and 'sorties' not in p.relative_to(folder).parts
                         and '__pycache__' not in p.parts and 'target' not in p.parts
                         and not any(part.startswith('.build-') for part in p.parts)]
    source_files += list(HERE.glob('*.py')) + [HERE / 'README.md']
    source_files = sorted(set(source_files))
    manifest = {str(p.relative_to(ROOT)): sha(p) for p in source_files}
    with zipfile.ZipFile(output / 'sources.zip', 'w', zipfile.ZIP_DEFLATED) as archive:
        for path in source_files:
            archive.write(path, str(path.relative_to(ROOT)))
    metadata.update(binary_sha256_after=binaries_after, sources=manifest,
                    source_archive_sha256=sha(output / 'sources.zip'),
                    paths={key: str(path.relative_to(ROOT)) for key, path in paths.items()})
    (output / 'campagne.json').write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + '\n')
    for name, directory in paths.items():
        shutil.copyfile(output / 'environnement.json', directory / 'environnement.json')
    index = dict(campaign=str(output.relative_to(ROOT)),
                 experiments={key: str(path.relative_to(ROOT)) for key, path in paths.items()})
    (HERE / 'sorties/campagnes-v5.json').write_text(json.dumps(index, indent=2) + '\n')
    print(json.dumps(index), flush=True)


if __name__ == '__main__':
    main()
