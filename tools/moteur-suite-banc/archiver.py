#!/usr/bin/env python3
"""Compléter la campagne : entrées publiques de compilation et preuves finales."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import zipfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
index = json.loads((HERE / 'sorties/campagnes-v5.json').read_text())
campaign = ROOT / index['campaign']
target = campaign / 'reproduction-final'
target.mkdir(exist_ok=True)
sources = set()
names = subprocess.check_output([
    'git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z',
], cwd=ROOT).decode().split('\0')
for name in names:
    path = ROOT / name
    if not name or not path.is_file() or not path.resolve().is_relative_to(ROOT):
        continue
    if name.startswith(('crates/', 'locales/', 'integrations/', 'docs/api/')):
        sources.add(path)
    if name in ('Cargo.toml', 'Cargo.lock', 'scripts/check.sh', 'scripts/leak-check.sh'):
        sources.add(path)
for experiment in (*index['experiments'], 'moteur-suite-banc'):
    folder = ROOT / 'tools' / experiment
    for path in folder.rglob('*'):
        if not path.is_file():
            continue
        parts = path.relative_to(folder).parts
        if any(p in ('sorties', 'target', '__pycache__') or p.startswith('.build-') for p in parts):
            continue
        sources.add(path)
checks = target / 'controles'
checks.mkdir(exist_ok=True)
for path in (HERE / 'sorties').glob('*'):
    if path.is_file() and path.suffix in ('.json', '.log') and path.name != 'campagnes-v5.json':
        destination = checks / path.name
        if destination.exists():
            assert destination.read_bytes() == path.read_bytes(), 'Contrôle archivé différent'
        else:
            shutil.copyfile(path, destination)
archive_path = target / 'sources-et-entrees.zip'
if not archive_path.exists():
    with zipfile.ZipFile(archive_path, 'w', zipfile.ZIP_DEFLATED) as archive:
        for path in sorted(sources):
            archive.write(path, str(path.relative_to(ROOT)))
with zipfile.ZipFile(archive_path) as archive:
    for path in sources:
        assert archive.read(str(path.relative_to(ROOT))) == path.read_bytes(), 'Source archivée différente'
digest = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
record = dict(
    git_commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
    archive_sha256=digest(archive_path),
    source_sha256={str(path.relative_to(ROOT)): digest(path) for path in sorted(sources)},
    control_sha256={path.name: digest(path) for path in sorted(checks.iterdir())},
    replay='Extraire à la racine d’une copie du dépôt au commit indiqué ; utiliser README et locks archivés.',
    scope='Sources et entrées publiques, aucun export HA brut ni conversation privée.',
)
manifest = target / 'reproduction.json'
encoded = json.dumps(record, ensure_ascii=False, indent=2) + '\n'
if manifest.exists():
    assert manifest.read_text() == encoded
else:
    manifest.write_text(encoded)
print(json.dumps(dict(public_sources=len(sources), controls=len(record['control_sha256']), archive=str(archive_path.relative_to(ROOT)))))
