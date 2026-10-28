#!/usr/bin/env python3
"""Run supplied commands without a shell; preserve process repeats and source hashes."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import shlex
import statistics
import subprocess


STRUCTURE_TIMINGS = {
    'ingest_ns_per_event_median', 'query_4windows_ns_median',
    'ingest_ns_rounds', 'query_ns_rounds',
}
PRODUCTION_TIMINGS = {
    'hourly_86400_changes_24h_ns_rounds',
    'hourly_86400_changes_24h_ns_median',
}
SOURCE_NAMES = (
    'main.rs', 'production.rs', 'Cargo.toml', 'Cargo.lock',
    'executer.py', 'README.md',
)


def utc_now():
    return datetime.now(timezone.utc).isoformat()


def deterministic(row, timing_fields):
    return {key: value for key, value in row.items() if key not in timing_fields}


def write_json(path, data):
    path.write_text(json.dumps(data, indent=2, ensure_ascii=False) + '\n')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--structures', required=True, help='Complete command, parsed by shlex.split; no shell')
    parser.add_argument('--production', required=True, help='Complete command, parsed by shlex.split; no shell')
    parser.add_argument('--metadata', required=True, type=Path, help='Compilation/measurement environment JSON supplied by campaign owner')
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    structure_command = shlex.split(args.structures)
    production_command = shlex.split(args.production)
    if not structure_command or not production_command:
        parser.error('Commands must not be empty')
    metadata_bytes = args.metadata.read_bytes()
    metadata = json.loads(metadata_bytes)
    root = Path(__file__).resolve().parent

    def source_bytes():
        return {name: (root / name).read_bytes() if (root / name).exists() else None for name in SOURCE_NAMES}

    def hashes(contents):
        return {name: hashlib.sha256(value).hexdigest() if value is not None else None for name, value in contents.items()}

    initial_bytes = source_bytes()
    before = hashes(initial_bytes)
    started = utc_now()
    args.output.mkdir(parents=True, exist_ok=False)
    snapshot = args.output / 'sources'
    snapshot.mkdir()
    for name, contents in initial_bytes.items():
        if contents is not None:
            (snapshot / name).write_bytes(contents)
    (args.output / 'compilation-metadata.json').write_bytes(metadata_bytes)
    commands = []
    checks = []

    def assert_sources_stable(phase):
        current = hashes(source_bytes())
        checks.append({'phase': phase, 'utc': utc_now(), 'matches_initial': current == before})
        if current != before:
            write_json(args.output / 'source-race.json', {'phase': phase, 'before': before, 'observed': current, 'checks': checks})
            raise RuntimeError(f'Sources changed during campaign: {phase}; discard partial timing results')
        return current

    def run(command, label):
        assert_sources_stable('before_' + label)
        start = utc_now()
        result = subprocess.run(command, check=True, text=True, capture_output=True, shell=False)
        assert_sources_stable('after_' + label)
        if result.stderr:
            raise RuntimeError(result.stderr)
        commands.append({'label': label, 'argv': command, 'started_utc': start, 'completed_utc': utc_now()})
        return json.loads(result.stdout)

    rows = []
    raw_path = args.output / 'resultats-bruts.jsonl'
    with raw_path.open('w') as raw:
        for workload in ('sparse', 'irregular', 'burst'):
            for backend in ('deque', 'ring', 'ring_small', 'buckets_10s'):
                repeats = []
                for repeat in range(3):
                    row = run(structure_command + [backend, workload], f'{backend}_{workload}_{repeat + 1}')
                    if row['backend'] != backend or row['workload'] != workload:
                        raise RuntimeError('Command returned a different backend/workload')
                    if repeats and deterministic(row, STRUCTURE_TIMINGS) != deterministic(repeats[0], STRUCTURE_TIMINGS):
                        raise RuntimeError(f'Non-deterministic values/coverage/errors/memory for {backend}/{workload}')
                    repeats.append(row)
                    raw.write(json.dumps({'process_repeat': repeat + 1, **row}, ensure_ascii=False) + '\n')
                    raw.flush()
                summary = deterministic(repeats[0], STRUCTURE_TIMINGS)
                summary['process_repetitions'] = 3
                summary['aggregation'] = 'median_of_three_process_medians_each_of_seven_rounds'
                for field in ('ingest_ns_per_event_median', 'query_4windows_ns_median'):
                    values = [row[field] for row in repeats]
                    summary[field] = statistics.median(values)
                    summary[field + '_by_process'] = values
                rows.append(summary)

    production_rows = []
    with (args.output / 'production-brut.jsonl').open('w') as raw:
        for repeat in range(3):
            row = run(production_command, f'production_{repeat + 1}')
            if production_rows and deterministic(row, PRODUCTION_TIMINGS) != deterministic(production_rows[0], PRODUCTION_TIMINGS):
                raise RuntimeError('Non-deterministic production semantic evidence')
            production_rows.append(row)
            raw.write(json.dumps({'process_repeat': repeat + 1, **row}, ensure_ascii=False) + '\n')
            raw.flush()
    production = deterministic(production_rows[0], PRODUCTION_TIMINGS)
    field = 'hourly_86400_changes_24h_ns_median'
    production[field + '_by_process'] = [row[field] for row in production_rows]
    production[field] = statistics.median(production[field + '_by_process'])
    production['process_repetitions'] = 3
    production['aggregation'] = 'median_of_three_process_medians_each_of_seven_rounds'
    after = assert_sources_stable('campaign_complete')
    manifest = {
        'schema': 2,
        'started_utc': started,
        'completed_utc': utc_now(),
        'status': 'synthetic_local_prototype_and_production_api_evidence',
        'structure_processes': 36,
        'structure_summary_cases': 12,
        'production_processes': 3,
        'process_repetitions': 3,
        'deterministic_parity': 'passed_all_non_timing_fields',
        'source_sha256_before': before,
        'source_sha256_after': after,
        'source_checks': checks,
        'commands': commands,
        'compilation_metadata_sha256': hashlib.sha256(metadata_bytes).hexdigest(),
        'compilation_metadata': metadata,
        'environment_policy': 'measurement_environment_is_supplied_metadata_not_python_host_platform',
    }
    for name, data in (('resultats.json', rows), ('production.json', production), ('campagne.json', manifest)):
        write_json(args.output / name, data)
    print(json.dumps({'raw_structure_cases': 36, 'summary_cases': len(rows), 'production_processes': 3, 'source_hashes_stable': before == after}))


if __name__ == '__main__':
    main()
