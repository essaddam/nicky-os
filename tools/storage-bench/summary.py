"""Résume un dossier de résultats du banc : verdicts, puis dispersion (min,
médiane, max sur les répétitions) des chiffres que l'ADR D13 cite."""

import json
import pathlib
import statistics
import sys

out: pathlib.Path


def lines(pattern):
    for path in sorted(out.glob(pattern)):
        for line in path.read_text().splitlines():
            line = line.strip()
            if line.startswith("{"):
                yield path.name, json.loads(line)


def spread(values):
    values = [v for v in values if v is not None]
    if not values:
        return "—"
    if len(values) == 1:
        return f"{values[0]:.2f}"
    return f"{min(values):.2f} / {statistics.median(values):.2f} / {max(values):.2f}"


def validate_results(directory, repeat):
    """Une sortie lisible ne suffit pas : exiger les preuves complètes et vraies."""
    errors = []

    def read(name):
        try:
            data = [json.loads(line) for line in (directory / name).read_text().splitlines() if line.strip()]
            if not data or any(not isinstance(row, dict) for row in data):
                raise ValueError("aucune preuve JSON objet")
        except (OSError, ValueError) as error:
            errors.append(f"{name}: {error}")
            return []
        for row in data:
            if "test" in row and row.get("pass") is not True:
                errors.append(f"{name}: verdict faux ou absent pour {row['test']}")
        return data

    def require(condition, message):
        if not condition:
            errors.append(message)

    def tests(data, expected, name):
        actual = [row["test"] for row in data if "test" in row]
        require(set(expected) <= set(actual), f"{name}: essais obligatoires absents")
        require(len(actual) == len(set(actual)), f"{name}: essais dupliqués")

    compat = read("compat.jsonl")
    tests(compat, "rowid_index_dml_upsert_returning differential_vs_sqlite_20k_ops transactions_commit_rollback temp_directory_requirements readers_during_write_snapshot mvcc_begin_concurrent durability_pragmas_checkpoint sqlite_file_opened_by_turso turso_file_opened_by_sqlite_both_ways migration_and_rollback_round_trip sequence_never_reused agent_sql_without_authorizer second_process_is_refused backup_vacuum_into io_backend_under_default_seccomp archive_export_retry_immutable archive_restore_replay_journal migration_batch_boundaries".split(), "compatibilité")
    require(bool(compat) and compat[-1].get("phase") == "compat_done" and compat[-1].get("failed") == 0,
            "compatibilité: fin réussie absente")
    crash = read("crash.jsonl")
    tests(crash, ["crash_recovery_read_by_sqlite", "turso_reopens_after_sqlite_recovery", "kill9_during_writes"], "crash")
    require(any(row.get("test") == "kill9_during_writes" and row.get("kills", 0) >= 31 for row in crash),
            "crash: moins de 31 arrêts brutaux prouvés")
    msrv = read("msrv.json")
    require(len(msrv) == 1 and msrv[0].get("check") == "ok" and msrv[0].get("msrv") == "1.88", "MSRV 1.88 non validé")
    for sync in ("FULL", "NORMAL", "OFF"):
        data = read(f"fsync-{sync}.jsonl")
        require(len(data) == 1 and data[0].get("commits") == 200 and data[0].get("synchronous") == sync,
                f"fsync {sync}: sonde incomplète")
        try:
            trace = (directory / f"strace-{sync}.txt").read_text()
            calls = sum(int(parts[3]) for line in trace.splitlines()
                        if len(parts := line.split()) >= 5 and parts[-1] in ("fsync", "fdatasync"))
            require(calls >= (200 if sync == "FULL" else 1), f"fsync {sync}: appels absents")
        except (OSError, ValueError) as error:
            errors.append(f"strace {sync}: {error}")
    expected_reads = "plug_24h_stable plug_24h_median plug_24h_busy plug_7d_busy plug_24h_worst_point security_720h_events dashboard_20_sparklines energy_24h_hourly energy_168h_hourly energy_30d_daily_from_hours energy_30d_daily_from_totals energy_12m_monthly_from_totals energy_summary_two_month_window energy_summary_from_totals export_samples_batch_5000_ops export_energy_batch_5000_ops export_mixed_batch_5000_ops".split()
    for i in range(1, repeat + 1):
        load = read(f"load-{i}.jsonl")
        require([row.get("phase") for row in load] == ["load_samples", "load_energy_window", "load_done"],
                f"tour {i}: chargement incomplet")
        reads = read(f"reads-{i}.jsonl")
        names = [row["name"] for row in reads if "name" in row]
        require(set(names) == set(expected_reads) and len(names) == len(expected_reads), f"tour {i}: lectures incomplètes")
        tests(reads, ["summary_window_equals_totals", "export_batches_cover_samples_and_energy"], f"lectures {i}")
        require(bool(reads) and reads[-1].get("phase") == "reads_done", f"tour {i}: fin des lectures absente")
        for rate in (4, 974):
            data = read(f"burst-{rate}-{i}.jsonl")
            require(len(data) == 1 and data[0].get("iterations") == 500 and data[0].get("writes_committed", 0) > 0,
                    f"tour {i}: rafale {rate} incomplète")
        reopen = read(f"reopen-{i}.jsonl")
        require(len(reopen) == 1 and reopen[0].get("integrity") == "ok", f"tour {i}: intégrité non validée")
        purge = read(f"purge-{i}.jsonl")
        require(len(purge) == 1 and purge[0].get("anchors", {}).get("pass") is True,
                f"tour {i}: ancrages non validés")
        require(len(purge) == 1 and purge[0].get("journal_retention", {}).get("pass") is True,
                f"tour {i}: rétention du journal non validée")
    for name in ("sizes.txt", "host-load.txt"):
        require((directory / name).is_file() and (directory / name).stat().st_size > 0, f"{name}: preuve absente")
    return errors


def main():
    global out
    out = pathlib.Path(sys.argv[1])
    repeat = int(sys.argv[2]) if len(sys.argv) > 2 else 3
    if repeat < 1:
        raise SystemExit("Le banc exige au moins un tour.")
    errors = validate_results(out, repeat)
    if errors:
        for error in errors:
            print(f"ÉCHEC: {error}", file=sys.stderr)
        raise SystemExit(1)

    print("== essais fonctionnels (pass attendu partout)")
    for name, row in list(lines("compat.jsonl")) + list(lines("crash.jsonl")) + list(lines("reads-*.jsonl")):
        if "test" in row:
            flag = "PASS" if row.get("pass") else "FAIL"
            if row.get("informational"):
                flag = "INFO"
            print(f"  {flag}  {row['test']}")
    for name in ("msrv.json",):
        p = out / name
        if p.exists():
            print("  msrv", p.read_text().strip())

    print("\n== tailles (octets)")
    p = out / "sizes.txt"
    if p.exists():
        text = p.read_text().strip()
        print("  " + text.replace("\n", "\n  "))
        sizes = {f[0]: [int(x) for x in f[1:]] for f in (l.split() for l in text.splitlines()) if len(f) == 4 and f[1].isdigit()}

        def delta(a, b):
            if a in sizes and b in sizes:
                print(f"  {a} − {b} : " + " / ".join(f"{(x - y) / 1e6:+.2f} Mo" for x, y in zip(sizes[a], sizes[b])))

        print("  écarts (strip=debuginfo / strip complet / gzip)")
        delta("size_turso", "size_sqlite")
        delta("size_both", "size_sqlite")
        delta("size-core", "size_none")
        delta("size_turso", "size_none")
        if "moli-os" in sizes:
            d = [t - s for t, s in zip(sizes["size_turso"], sizes["size_sqlite"])]
            print("  Moli avec Turso au lieu de SQLite (projection) : "
                  + " / ".join(f"{(m + x) / 1e6:.2f} Mo" for m, x in zip(sizes["moli-os"], d)))

    print("\n== fsync pour 200 validations")
    for sync in ("FULL", "NORMAL", "OFF"):
        p = out / f"strace-{sync}.txt"
        if p.exists():
            calls = 0
            for line in p.read_text().splitlines():
                parts = line.split()
                if parts and parts[-1] in ("fsync", "fdatasync"):
                    calls += int(parts[3]) if len(parts) >= 5 else int(parts[-2])
            print(f"  {sync:6} {calls}")

    print("\n== charge ×2 (min / médiane / max sur les répétitions)")
    load = [r for _, r in lines("load-*.jsonl")]
    samples = [r for r in load if r.get("phase") == "load_samples"]
    done = [r for r in load if r.get("phase") == "load_done"]
    if samples:
        print(f"  relevés chargés          {samples[0]['rows']} sur {samples[0]['points']} points")
        print(f"  durée (s)                {spread([r['elapsed_ms'] / 1000 for r in samples])}")
        print(f"  op/s                     {spread([r['ops_per_s'] for r in samples])}")
        print(f"  transaction p95 (ms)     {spread([r['commit']['p95_ms'] for r in samples])}")
        energy = [r for r in load if r.get("phase") == "load_energy_window"]
        print(f"  heures d'énergie (s)     {spread([r['elapsed_ms'] / 1000 for r in energy])}  ({energy[0]['rows'] if energy else '—'} heures)")
        print(f"  pic RSS (Mio)            {spread([r['memory']['peak_rss_kib'] / 1024 for r in done])}")
        print(f"  fichier (Mo)             {spread([r['db_bytes'] / 1e6 for r in done])}")

    p = out / "host-load.txt"
    if p.exists():
        print("  charge (1, 5, 15 min) de la VM Docker et de l'hôte, au début de chaque tour et à la fin :")
        print("  " + p.read_text().strip().replace("\n", "\n  "))

    print("\n== lectures, p95 en ms (min / médiane / max)")
    by_name = {}
    for _, r in lines("reads-*.jsonl"):
        if "name" in r:
            by_name.setdefault(r["name"], []).append(r)
    for name, rs in by_name.items():
        print(f"  {name:36} p95 {spread([r['p95_ms'] for r in rs]):>24}   lignes {rs[0]['rows_scanned']:>7}   JSON {rs[0]['json_bytes']:>8} o")
    mem = [r["memory"]["peak_rss_kib"] / 1024 for _, r in lines("reads-*.jsonl") if r.get("phase") == "reads_done"]
    print(f"  pic RSS (Mio)                        {spread(mem)}")

    print("\n== cinq lecteurs + rédacteur durable")
    for rate in ("4", "974"):
        rs = [r for _, r in lines(f"burst-{rate}-*.jsonl") if "name" in r]
        if rs:
            print(f"  {rate:>4} op/s visés : atteint {spread([r['achieved_writes_per_s'] for r in rs])} op/s ; "
                  f"lecture p95 {spread([r['p95_ms'] for r in rs])} ms, p99 {spread([r['p99_ms'] for r in rs])} ms ; "
                  f"transaction p95 {spread([r['commit']['p95_ms'] for r in rs])} ms ; pic RSS {spread([r['memory']['peak_rss_kib'] / 1024 for r in rs])} Mio")

    print("\n== réouverture et purge")
    ro = [r for _, r in lines("reopen-*.jsonl")]
    if ro:
        print(f"  ouverture (ms)           {spread([r['open_ms'] for r in ro])}")
        print(f"  première courbe (ms)     {spread([r['first_curve_ms'] for r in ro])}")
        print(f"  integrity_check (s)      {spread([r['integrity_ms'] / 1000 for r in ro])}  {ro[0]['integrity']}")
    pu = [r for _, r in lines("purge-*.jsonl")]
    if pu:
        print(f"  relevés purgés           {pu[0]['samples_deleted']} ({pu[0]['points']} points)")
        print(f"  purge totale (s)         {spread([r['samples_ms'] / 1000 for r in pu])}")
        print(f"  transaction par point p95 / max (ms) {spread([r['per_point_tx']['p95_ms'] for r in pu])} | {spread([r['per_point_tx']['max_ms'] for r in pu])}")
        print(f"  ancrages conservés       {all(r['anchors']['pass'] for r in pu)}")
        print(f"  accusé + outbox vidée (s) {spread([r['outbox_ms'] / 1000 for r in pu])}")
        print(f"  journal après 7 j (s)    {spread([r['journal_ms'] / 1000 for r in pu])}")
        print(f"  fichier avant / après (Mo) {spread([r['db_bytes_before'] / 1e6 for r in pu])} | {spread([r['db_bytes_after'] / 1e6 for r in pu])}")


if __name__ == "__main__":
    main()
