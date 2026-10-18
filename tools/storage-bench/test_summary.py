"""Régressions du verdict global : chaque preuve obligatoire doit bloquer le banc."""

import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SUMMARY = Path(__file__).with_name('summary.py')
COMPAT = (
    'rowid_index_dml_upsert_returning differential_vs_sqlite_20k_ops '
    'transactions_commit_rollback temp_directory_requirements '
    'readers_during_write_snapshot mvcc_begin_concurrent '
    'durability_pragmas_checkpoint sqlite_file_opened_by_turso '
    'turso_file_opened_by_sqlite_both_ways migration_and_rollback_round_trip '
    'sequence_never_reused agent_sql_without_authorizer second_process_is_refused '
    'backup_vacuum_into io_backend_under_default_seccomp '
    'archive_export_retry_immutable archive_restore_replay_journal migration_batch_boundaries'
).split()
READS = (
    'plug_24h_stable plug_24h_median plug_24h_busy plug_7d_busy '
    'plug_24h_worst_point security_720h_events dashboard_20_sparklines '
    'energy_24h_hourly energy_168h_hourly energy_30d_daily_from_hours '
    'energy_30d_daily_from_totals energy_12m_monthly_from_totals '
    'energy_summary_two_month_window energy_summary_from_totals '
    'export_samples_batch_5000_ops export_energy_batch_5000_ops export_mixed_batch_5000_ops'
).split()


class SummaryExitStatus(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.out = Path(self.temp.name)
        self.write('compat.jsonl', *({'test': name, 'pass': True} for name in COMPAT),
                   {'phase': 'compat_done', 'failed': 0, 'turso': '0.8.3'})
        self.write('crash.jsonl', {'test': 'crash_recovery_read_by_sqlite', 'pass': True},
                   {'test': 'turso_reopens_after_sqlite_recovery', 'pass': True},
                   {'test': 'kill9_during_writes', 'pass': True, 'kills': 31})
        self.write('msrv.json', {'msrv': '1.88', 'check': 'ok'})
        memory = {'peak_rss_kib': 1024}
        quantiles = {'p95_ms': 1, 'max_ms': 2}
        self.write('load-1.jsonl',
                   {'phase': 'load_samples', 'rows': 10, 'points': 1, 'elapsed_ms': 1,
                    'ops_per_s': 10, 'commit': quantiles, 'memory': memory},
                   {'phase': 'load_energy_window', 'rows': 1, 'elapsed_ms': 1},
                   {'phase': 'load_done', 'db_bytes': 4096, 'memory': memory})
        self.write('reads-1.jsonl',
                   *({'name': name, 'iterations': 20, 'p95_ms': 1, 'rows_scanned': 1,
                      'json_bytes': 10} for name in READS),
                   {'test': 'summary_window_equals_totals', 'pass': True},
                   {'test': 'export_batches_cover_samples_and_energy', 'pass': True},
                   {'phase': 'reads_done', 'memory': memory})
        for rate in (4, 974):
            self.write(f'burst-{rate}-1.jsonl',
                       {'name': f'five_readers_with_{rate}_writes_per_s', 'iterations': 500,
                        'writes_committed': 10, 'achieved_writes_per_s': rate,
                        'p95_ms': 1, 'p99_ms': 2, 'commit': quantiles, 'memory': memory})
        self.write('reopen-1.jsonl', {'phase': 'reopen', 'integrity': 'ok', 'open_ms': 1,
                   'first_curve_ms': 1, 'integrity_ms': 1})
        self.write('purge-1.jsonl', {'phase': 'purge', 'samples_deleted': 1, 'points': 1,
                   'samples_ms': 1, 'per_point_tx': quantiles, 'anchors': {'pass': True},
                   'journal_retention': {'pass': True}, 'outbox_ms': 1, 'journal_ms': 1,
                   'db_bytes_before': 4096, 'db_bytes_after': 4096})
        (self.out/'sizes.txt').write_text('size_none 1 1 1\nsize_sqlite 2 2 2\n'
                                        'size_turso 3 3 3\nsize_both 4 4 4\nsize-core 3 3 3\n')
        (self.out/'host-load.txt').write_text('tour 1 : VM 0 0 0 ; hôte 0 0 0\nfin : VM 0 0 0 ; hôte 0 0 0\n')
        for sync, calls in [('FULL', 213), ('NORMAL', 13), ('OFF', 13)]:
            self.write(f'fsync-{sync}.jsonl', {'phase': 'fsync_probe', 'commits': 200,
                                            'synchronous': sync})
            (self.out/f'strace-{sync}.txt').write_text(f'100.00 0.1 1 {calls} fsync\n')

    def write(self, name, *rows):
        (self.out/name).write_text(''.join(json.dumps(row)+'\n' for row in rows))

    def run_summary(self):
        return subprocess.run([sys.executable, '-I', str(SUMMARY), str(self.out), '1'],
                              capture_output=True, text=True)

    def test_complete_evidence_succeeds(self):
        result = self.run_summary()
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_false_compatibility_verdict_fails(self):
        self.write('compat.jsonl', {'test': 'mandatory', 'pass': False},
                   {'phase': 'compat_done', 'failed': 1, 'turso': '0.8.3'})
        self.assertNotEqual(self.run_summary().returncode, 0)

    def test_crash_failure_fails(self):
        self.write('crash.jsonl', {'test': 'kill9_during_writes', 'pass': False, 'kills': 31})
        self.assertNotEqual(self.run_summary().returncode, 0)

    def test_msrv_failure_fails(self):
        self.write('msrv.json', {'msrv': '1.88', 'check': 'failed'})
        self.assertNotEqual(self.run_summary().returncode, 0)

    def test_integrity_failure_fails(self):
        self.write('reopen-1.jsonl', {'phase': 'reopen', 'integrity': 'corrupt',
                   'open_ms': 1, 'first_curve_ms': 1, 'integrity_ms': 1})
        self.assertNotEqual(self.run_summary().returncode, 0)

    def test_missing_stage_fails(self):
        (self.out/'reads-1.jsonl').unlink()
        self.assertNotEqual(self.run_summary().returncode, 0)

    def test_wrong_anchor_fails(self):
        data = json.loads((self.out/'purge-1.jsonl').read_text())
        data['anchors']['pass'] = False
        self.write('purge-1.jsonl', data)
        self.assertNotEqual(self.run_summary().returncode, 0)

    def test_replay_retention_failure_fails(self):
        data = json.loads((self.out/'purge-1.jsonl').read_text())
        data['journal_retention']['pass'] = False
        self.write('purge-1.jsonl', data)
        self.assertNotEqual(self.run_summary().returncode, 0)


if __name__ == '__main__':
    unittest.main()
