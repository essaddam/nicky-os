#!/usr/bin/env python3
"""Vérifie hors ligne l'intégrité et les invariants des campagnes v5."""

from collections import Counter, defaultdict
import hashlib
import json
import math
from pathlib import Path
import re
import statistics
import sys
from html.parser import HTMLParser
from urllib.parse import unquote, urlsplit
from zipfile import BadZipFile, ZipFile


ROOT = Path(__file__).resolve().parents[2]
INDEX = ROOT / "tools/moteur-suite-banc/sorties/campagnes-v5.json"
EXPECTED_ASSERTIONS = [
    "drafts_inert",
    "human_publication_required",
    "real_computed_chain",
    "readonly_and_type_validation",
    "guard_holds_setter_and_action",
    "approval_executes",
    "standalone_action_has_no_state",
    "commands_audited",
    "driver_error_propagates",
    "driver_timeout_without_late_effect",
    "monotonic_expiry_without_input",
    "equal_observation_refresh_without_raw_event",
    "distinct_equal_occurrences",
    "approved_state_trigger_to_setter",
    "lossless_bus",
    "all_final_values_match_oracle",
]
ENGINE_METRICS = (
    "raw_observations_per_second",
    "adapter_payload_and_handles_bytes",
    "adapter_live_requested_bytes",
    "hub_setup_net_requested_bytes",
    "populated_live_requested_bytes",
    "final_live_requested_bytes",
    "workload_peak_requested_bytes",
    "workload_allocated_bytes",
    "loaded_rss_bytes",
    "rss_bytes",
    "process_peak_rss_bytes",
    "loaded_anonymous_bytes",
    "steady_anonymous_bytes",
    "sampled_peak_anonymous_bytes",
    "batch_ack_latency_ns.p50",
    "batch_ack_latency_ns.p95",
    "propagation_and_automation_latency_ns.p50",
    "propagation_and_automation_latency_ns.p95",
    "setter_round_trip_latency_ns.p50",
    "setter_round_trip_latency_ns.p95",
    "action_round_trip_latency_ns.p50",
    "action_round_trip_latency_ns.p95",
)
ENGINE_ALLOCATIONS = (
    "baseline_live_requested_bytes",
    "populated_live_requested_bytes",
    "hub_setup_net_requested_bytes",
    "final_live_requested_bytes",
    "workload_peak_requested_bytes",
    "workload_allocated_bytes",
    "adapter_live_requested_bytes",
)
TRANSFORM_INDEX_BASE = "action_api_json_lookup_validate_encode"
TRANSFORM_INDEXED = "experimental_action_api_json_index_validate_encode"
TRANSFORM_TIMING = "sample_mean_ns_per_operation"
TEMP_TIMING = {
    "ingest_ns_per_event_median",
    "query_4windows_ns_median",
    "ingest_ns_rounds",
    "query_ns_rounds",
}
PRODUCTION_TIMING = {
    "hourly_86400_changes_24h_ns_rounds",
    "hourly_86400_changes_24h_ns_median",
}
PRIVATE_PATHS = (
    re.compile(r'''(?:^|[\s"'=(\[{])/(?:Users|home)/[^/\s"'<>]+'''),
    re.compile(r'''(?:^|[\s"'=(\[{])[A-Za-z]:\\Users\\[^\\\s"'<>]+'''),
)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def read_json(path):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ValueError(f"JSON absent ou invalide : {rel(path)} ({exc})") from exc


def read_jsonl(path):
    try:
        return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    except (OSError, json.JSONDecodeError) as exc:
        raise ValueError(f"JSONL absent ou invalide : {rel(path)} ({exc})") from exc


def rel(path):
    try:
        return path.resolve().relative_to(ROOT).as_posix()
    except ValueError:
        return path.name


def digest(data):
    return hashlib.sha256(data).hexdigest()


def close(a, b, *, rel_tol=2e-10, abs_tol=2e-10):
    if a is None or b is None:
        return a is None and b is None
    return math.isclose(float(a), float(b), rel_tol=rel_tol, abs_tol=abs_tol)


def nested(row, name):
    value = row
    for part in name.split("."):
        value = value[part]
    return value


def check_source_archive(root_campaign, campaign_directory):
    archive_path = campaign_directory / "sources.zip"
    require(archive_path.is_file(), "sources.zip absent")
    require(
        digest(archive_path.read_bytes()) == root_campaign["source_archive_sha256"],
        "empreinte de sources.zip différente du manifeste",
    )
    try:
        archive = ZipFile(archive_path)
    except (OSError, BadZipFile) as exc:
        raise ValueError(f"sources.zip illisible ({exc})") from exc
    with archive:
        names = set(archive.namelist())
        sources = root_campaign["sources"]
        require(len(names) == len(sources), "nombre de fichiers de l'archive différent du manifeste")
        require(names == set(sources), "liste des fichiers de l'archive différente du manifeste")
        checked = 0
        for name, expected in sources.items():
            archived = archive.read(name)
            require(digest(archived) == expected, f"empreinte archivée différente : {name}")
            if name.endswith(".rs"):
                current = ROOT / name
                require(current.is_file(), f"source Rust actuelle absente : {name}")
                require(digest(current.read_bytes()) == expected, f"source Rust modifiée depuis la campagne : {name}")
                checked += 1
    return checked, len(sources)


def check_campaign_sources(campaign, archive_sources, current_prefix=""):
    """Check all recorded hashes against the consolidated archive and workspace."""
    source_map = campaign.get("source_sha256")
    if source_map is None and "source_sha256_before" in campaign:
        require(campaign["source_sha256_before"] == campaign["source_sha256_after"], "sources temporelles modifiées pendant la campagne")
        source_map = campaign["source_sha256_before"]
    require(isinstance(source_map, dict) and source_map, "empreintes sources absentes")
    for name, expected in source_map.items():
        archive_name = f"{current_prefix}{name}"
        require(archive_sources.get(archive_name) == expected, f"empreinte campagne/archive différente : {archive_name}")
        current = ROOT / archive_name
        require(current.is_file(), f"source actuelle absente : {archive_name}")
        require(digest(current.read_bytes()) == expected, f"source actuelle différente de la campagne : {archive_name}")
    return len(source_map)


def check_workspace_check():
    output = ROOT / "tools/moteur-suite-banc/sorties"
    run = read_json(output / "workspace-check.json")
    require(run.get("exit_code") == 0, "scripts/check.sh n'a pas terminé avec exit 0")
    require(run.get("source_tree_sha256"), "empreinte du workspace absente")
    log = (output / "workspace-check.log").read_text(encoding="utf-8")
    results = re.findall(r"test result: ok\. (\d+) passed; (\d+) failed", log)
    require(results, "résultats des tests workspace absents")
    passed = sum(int(ok) for ok, _ in results)
    failed = sum(int(bad) for _, bad in results)
    require(passed == 1590 and failed == 0, f"résultats workspace inattendus : {passed} passés, {failed} échoués")
    return passed, len(results)


def check_engine(paths):
    directory = ROOT / paths["moteur-integre-banc"]
    campaign = read_json(directory / "campagne.json")
    raw = read_jsonl(directory / "brut.jsonl")
    summary_doc = read_json(directory / "resultats.json")
    expected_sizes = [206, 290, 496, 2060, 2900, 4960]
    layouts = {"uniform96", "compact64", "columns"}
    modes = {"counted", "timing_plain"}
    require(campaign["sizes"] == expected_sizes and campaign["rounds"] == 10 and campaign["repetitions"] == 3, "paramètres campagne moteur inattendus")
    require(len(raw) == 108 and summary_doc["runs"] == 108 and len(summary_doc["groups"]) == 36, "nombre de processus/groupes moteur incorrect")
    require(campaign["source_sha256"], "empreintes sources moteur absentes")
    engine_source = (ROOT / "tools/moteur-integre-banc/src/main.rs").read_text(encoding="utf-8")
    require(re.search(r"effects\.load\(Ordering::Relaxed\)\s*==\s*round\s*\+\s*2", engine_source), "compteur d'effets par ronde absent du banc")
    require(re.search(r"effects\.load\(Ordering::Relaxed\)\s*==\s*rounds\s*\+\s*1", engine_source), "compteur final d'effets absent du banc")
    grouped = defaultdict(list)
    counts = Counter()
    for row in raw:
        n = row["source_devices"]
        key = (n, row["replay_layout"], row["measurement_mode"])
        require(n in expected_sizes and key[1] in layouts and key[2] in modes, "groupe moteur inconnu")
        grouped[key].append(row)
        counts[(n, key[1], key[2], row["repetition"])] += 1
        require(row["engine"] == "production-moli-runtime+production-moli-automation/synthetic-transport", "binaire moteur inattendu")
        require(row["rounds"] == 10 and row["devices"] == n + 3 and row["points"] == n * 4 + 3, "dimensions moteur incorrectes")
        require(row["raw_observations"] == n * 10 and row["automation_runs"] == 10 and row["bus_occurrences"] == 2, "compteurs métier moteur incorrects")
        require(row["assertions"] == EXPECTED_ASSERTIONS, "liste des 16 assertions moteur modifiée")
        expected_checksum = n * 10_000_000 + n * (n - 1) // 2
        require(row["final_value_checksum"] == expected_checksum, "checksum final moteur incorrect")
        require(row["elapsed_ns"] > 0 and row["raw_observations_per_second"] > 0, "durée/débit moteur invalides")
        rate = n * 10 / (row["elapsed_ns"] / 1e9)
        require(close(row["raw_observations_per_second"], rate, rel_tol=1e-12, abs_tol=1e-9), "débit moteur incohérent avec le chronomètre")
        for metric in ("batch_ack_latency_ns", "propagation_and_automation_latency_ns", "setter_round_trip_latency_ns", "action_round_trip_latency_ns"):
            value = row[metric]
            require(value["samples"] == 10 and value["p50"] <= value["p95"] <= value["max"], f"latence moteur invalide : {metric}")
        if row["measurement_mode"] == "timing_plain":
            require(all(row[field] == 0 for field in ENGINE_ALLOCATIONS), "compteur d'allocation non nul en timing_plain")
        else:
            require(all(row[field] > 0 for field in ENGINE_ALLOCATIONS), "compteur d'allocation compté invalide")
    require(len(grouped) == 36 and all(len(rows) == 3 for rows in grouped.values()), "chaque groupe moteur doit contenir trois processus")
    require(len(counts) == 108 and all(value == 1 for value in counts.values()), "répétitions moteur manquantes ou dupliquées")
    for n in expected_sizes:
        parity = {
            (row["final_value_checksum"], row["automation_runs"], row["bus_occurrences"], tuple(row["assertions"]))
            for row in raw if row["source_devices"] == n
        }
        require(len(parity) == 1, f"parité layouts/modes moteur échouée à {n} appareils")
    for group in summary_doc["groups"]:
        key = (group["source_devices"], group["layout"], group["measurement_mode"])
        rows = grouped[key]
        require(group["processes"] == 3, "effectif groupe moteur incorrect")
        for field in ENGINE_METRICS:
            values = [nested(row, field) for row in rows]
            expected = {"median": statistics.median(values), "min": min(values), "max": max(values)}
            require(group[field] == expected, f"médiane/min/max moteur incorrects : {key} {field}")
    return len(raw), len(grouped)


def check_transformations(paths):
    directory = ROOT / paths["transformations-banc"]
    metadata = read_json(directory / "environnement.json")
    require(metadata.get("production_source_sha256"), "empreinte de production transformations absente")
    measurement_runs = []
    memory_runs = []
    for repeat in range(1, 4):
        rows = read_jsonl(directory / f"mesures-{repeat}.jsonl")
        require(len(rows) == 24 and rows[0].get("type") == "metadata", f"mesures transformation {repeat}: 23 cas requis")
        measured = rows[1:]
        require(all(row.get("type") == "measurement" for row in measured), "ligne transformation non mesurée")
        require(all(row["samples"] == 7 and len(row[TRANSFORM_TIMING]) == 7 for row in measured), "chaque mesure transformation doit contenir sept passages")
        by_key = {(row["name"], row["devices"]): row for row in measured}
        require(len(by_key) == 23, "cas de mesure transformation dupliqué")
        expected_keys = {
            (name, devices)
            for devices in (1, 100, 1000)
            for name in (
                "mqtt_bytes_route_decode",
                TRANSFORM_INDEX_BASE,
                TRANSFORM_INDEXED,
                "experimental_action_index_rebuild",
                "action_codec_only",
            )
        } | {
            (name, 1)
            for name in (
                "profile_json_parse_and_compiled_read",
                "profile_compiled_read_only",
                "profile_recompile_each_message_naive_baseline",
                "profile_action_api_json_validate_render",
                "profile_action_validate_render_only",
                "core_unit_ratio_lookup",
                "experimental_affine_inbound",
                "experimental_affine_outbound",
            )
        }
        require(set(by_key) == expected_keys, "ensemble des 23 mesures transformation inattendu")
        for row in measured:
            values = row[TRANSFORM_TIMING]
            ordered = sorted(values)
            require(row["median_sample_mean_ns_per_operation"] == ordered[3], f"médiane transformation incorrecte : {row['name']}")
            require(row["best_sample_mean_ns_per_operation"] == ordered[0] and row["worst_sample_mean_ns_per_operation"] == ordered[-1], f"min/max transformation incorrects : {row['name']}")
        for devices in (1, 100, 1000):
            base = by_key[(TRANSFORM_INDEX_BASE, devices)]
            indexed = by_key[(TRANSFORM_INDEXED, devices)]
            require(base["checksum"] == indexed["checksum"], f"checksum lookup/index différent pour {devices} appareils")
        measurement_runs.append(by_key)
        memory = read_jsonl(directory / f"memoire-index-{repeat}.jsonl")
        require(len(memory) == 3 and all(row["type"] == "memory" for row in memory), "trois mesures mémoire index requises")
        require([row["devices"] for row in memory] == [1, 100, 1000], "tailles mémoire index inattendues")
        for row in memory:
            require(row["entries"] == row["devices"] and row["requested_live_heap_bytes"] > 0, "mesure mémoire index invalide")
            require(row["requested_heap_bytes_after_drop_delta"] == 0, "l'index ne libère pas toutes ses allocations demandées")
        memory_runs.append(memory)
    # Checksums and index memory are deterministic across independent processes.
    for repeat_rows in measurement_runs[1:]:
        for key, row in measurement_runs[0].items():
            require(repeat_rows[key]["checksum"] == row["checksum"], f"checksum transformation variable : {key}")
    for repeat_memory in memory_runs[1:]:
        require(repeat_memory == memory_runs[0], "résultats mémoire index différents entre processus")
    total_index_heap = sum(row["requested_live_heap_bytes"] for row in memory_runs[0])
    return 69, total_index_heap


def stable_fields(row, timing_fields):
    return {key: value for key, value in row.items() if key not in timing_fields and key != "process_repeat"}


def check_temporal(paths):
    directory = ROOT / paths["fenetres-temporelles-banc"]
    campaign = read_json(directory / "campagne.json")
    require(campaign["structure_processes"] == 36 and campaign["structure_summary_cases"] == 12, "dimensions campagne temporelle incorrectes")
    require(campaign["production_processes"] == 3 and campaign["process_repetitions"] == 3, "répétitions production temporelle incorrectes")
    require(campaign["deterministic_parity"] == "passed_all_non_timing_fields", "parité déterministe temporelle non validée")
    require(campaign["source_sha256_before"] == campaign["source_sha256_after"], "sources temporelles instables")
    require(all(row["matches_initial"] for row in campaign["source_checks"]), "contrôle de stabilité source temporelle en échec")
    raw = read_jsonl(directory / "resultats-bruts.jsonl")
    summaries = read_json(directory / "resultats.json")
    require(len(raw) == 36 and len(summaries) == 12, "36 runs bruts / 12 groupes temporels requis")
    cases = defaultdict(list)
    for row in raw:
        require(row["status"] == "prototype_not_product" and row["rounds"] == 7, "métadonnées prototype temporel incorrectes")
        key = (row["backend"], row["workload"])
        cases[key].append(row)
        require(len(row["windows"]) == 4 and len(row["ingest_ns_rounds"]) == 7 and len(row["query_ns_rounds"]) == 7, "mesures temporelles incomplètes")
        require(close(row["ingest_ns_per_event_median"], sorted(row["ingest_ns_rounds"])[3], rel_tol=0.0, abs_tol=0.00051), "médiane ingestion temporelle incorrecte")
        require(close(row["query_4windows_ns_median"], sorted(row["query_ns_rounds"])[3], rel_tol=0.0, abs_tol=0.00051), "médiane requête temporelle incorrecte")
        for window in row["windows"]:
            require(0 <= window["covered_ms"] <= window["window_ms"], "couverture temporelle hors bornes")
            require(0 <= window["oracle_covered_ms"] <= window["window_ms"], "couverture oracle hors bornes")
            expected_unknown = min(window["window_ms"], row["span_ms"]) - window["covered_ms"]
            require(window["unknown_ms"] == expected_unknown, "durée inconnue temporelle incohérente")
            for value_key, oracle_key, error_key in (
                ("sample_mean", "oracle_sample_mean", "sample_abs_error"),
                ("time_mean", "oracle_time_mean", "time_abs_error"),
            ):
                expected_error = None if window[value_key] is None or window[oracle_key] is None else abs(window[value_key] - window[oracle_key])
                require(close(window[error_key], expected_error, rel_tol=1e-10, abs_tol=1e-10), f"erreur oracle temporelle incohérente : {error_key}")
            if row["backend"] in {"deque", "ring"} and not window["limited"]:
                require(close(window["sample_mean"], window["oracle_sample_mean"]), "moyenne échantillons exacte différente de l'oracle")
                require(close(window["time_mean"], window["oracle_time_mean"]), "moyenne durée exacte différente de l'oracle")
                require(window["covered_ms"] == window["oracle_covered_ms"], "couverture exacte différente de l'oracle")
    require(len(cases) == 12 and all(len(rows) == 3 for rows in cases.values()), "chaque cas temporel doit avoir trois processus")
    for rows in cases.values():
        reference = stable_fields(rows[0], TEMP_TIMING)
        for row in rows[1:]:
            require(stable_fields(row, TEMP_TIMING) == reference, "résultat temporel non chronométrique variable entre processus")
    summary_by_key = {(row["backend"], row["workload"]): row for row in summaries}
    require(len(summary_by_key) == 12, "groupe temporel résumé dupliqué")
    for key, rows in cases.items():
        summary = summary_by_key[key]
        require(summary["process_repetitions"] == 3, "nombre de répétitions résumé temporel incorrect")
        require(summary["aggregation"] == "median_of_three_process_medians_each_of_seven_rounds", "agrégation temporelle inattendue")
        for field in ("ingest_ns_per_event_median", "query_4windows_ns_median"):
            values = [row[field] for row in rows]
            require(summary[field + "_by_process"] == values, f"chronos temporels bruts incohérents : {field}")
            require(summary[field] == statistics.median(values), f"médiane inter-processus temporelle incorrecte : {field}")
        require(stable_fields(summary, TEMP_TIMING | {"process_repetitions", "aggregation", "ingest_ns_per_event_median_by_process", "query_4windows_ns_median_by_process"}) == stable_fields(rows[0], TEMP_TIMING), "résumé sémantique temporel différent du brut")

    production_raw = read_jsonl(directory / "production-brut.jsonl")
    production_summary = read_json(directory / "production.json")
    require(len(production_raw) == 3, "trois répétitions API temporelles requises")
    baseline = stable_fields(production_raw[0], PRODUCTION_TIMING)
    for row in production_raw[1:]:
        require(stable_fields(row, PRODUCTION_TIMING) == baseline, "preuve sémantique API temporelle variable")
    timings = [row["hourly_86400_changes_24h_ns_median"] for row in production_raw]
    require(production_summary["hourly_86400_changes_24h_ns_median_by_process"] == timings, "chronos production temporels bruts incohérents")
    require(production_summary["hourly_86400_changes_24h_ns_median"] == statistics.median(timings), "médiane production temporelle incorrecte")
    require(production_summary["process_repetitions"] == 3, "répétitions du résumé API temporel incorrectes")
    require(stable_fields(production_summary, PRODUCTION_TIMING | {"process_repetitions", "aggregation", "hourly_86400_changes_24h_ns_median_by_process"}) == baseline, "résumé API temporel différent des résultats bruts")
    return len(raw), len(cases), len(production_raw)


def check_private_paths(index_path, campaign_dirs):
    files = [index_path]
    suffixes = {".json", ".jsonl", ".txt", ".log", ".md", ".html", ".zip"}
    for directory in campaign_dirs:
        files.extend(path for path in directory.rglob("*") if path.is_file() and path.suffix.lower() in suffixes)
    checked = set()
    for path in files:
        if path in checked:
            continue
        checked.add(path)
        content = path.read_bytes().decode("utf-8", errors="replace")
        require(not any(pattern.search(content) for pattern in PRIVATE_PATHS), f"chemin utilisateur absolu dans un résultat : {rel(path)}")
        if path.suffix.lower() == ".zip":
            try:
                with ZipFile(path) as archive:
                    for name in archive.namelist():
                        text = archive.read(name).decode("utf-8", errors="replace")
                        require(not any(pattern.search(text) for pattern in PRIVATE_PATHS), f"chemin utilisateur absolu dans l'archive : {name}")
            except (OSError, BadZipFile) as exc:
                raise ValueError(f"archive source illisible : {rel(path)}") from exc
    return len(checked)


def check_html():
    class Links(HTMLParser):
        def __init__(self):
            super().__init__()
            self.ids = []
            self.hrefs = []

        def handle_starttag(self, tag, attrs):
            attrs = dict(attrs)
            if "id" in attrs:
                self.ids.append(attrs["id"])
            if tag == "a" and "href" in attrs:
                self.hrefs.append(attrs["href"])

    path = ROOT / "docs/proposition/analyse/catalogue-memoire-v5.html"
    parser = Links()
    parser.feed(path.read_text())
    require(len(parser.ids) == len(set(parser.ids)), "ID HTML dupliqué")
    for href in parser.hrefs:
        url = urlsplit(href)
        if url.scheme or url.netloc:
            continue
        if not url.path:
            require(url.fragment in parser.ids, "Ancre HTML absente")
        else:
            target = (path.parent / unquote(url.path)).resolve()
            require(target.is_relative_to(ROOT) and target.is_file(), "Lien HTML local absent")
    require("th{text-transform:none}" in path.read_text(), "Les unités µs doivent conserver leur casse")
    return len(parser.hrefs)


def check_final_reproduction(campaign_path):
    directory = campaign_path / "reproduction-final"
    manifest = directory / "reproduction.json"
    if not manifest.exists():
        return "attente de la capture finale"
    info = read_json(manifest)
    archive_path = directory / "sources-et-entrees.zip"
    require(digest(archive_path.read_bytes()) == info["archive_sha256"], "Archive finale modifiée")
    with ZipFile(archive_path) as archive:
        for name, expected in info["source_sha256"].items():
            require(digest(archive.read(name)) == expected, "Source de reproduction modifiée")
            require(digest((ROOT / name).read_bytes()) == expected, "Source actuelle différente de la capture finale")
    for name, expected in info["control_sha256"].items():
        require(digest((directory / "controles" / name).read_bytes()) == expected, "Contrôle archivé modifié")
    return len(info["source_sha256"])


def main():
    index_path = INDEX
    index = read_json(index_path)
    campaign_path = ROOT / index["campaign"]
    root_campaign = read_json(campaign_path / "campagne.json")
    require(root_campaign["production_source_sha256"] == read_json(ROOT / "tools/moteur-suite-banc/sorties/workspace-check.json")["source_tree_sha256"], "empreinte production différente du garde-fou workspace")
    require(root_campaign["source_archive_sha256"] == digest((campaign_path / "sources.zip").read_bytes()), "empreinte archive globale incorrecte")
    rust_files, archived_files = check_source_archive(root_campaign, campaign_path)
    archive_sources = root_campaign["sources"]
    engine_sources = check_campaign_sources(read_json(ROOT / index["experiments"]["moteur-integre-banc"] / "campagne.json"), archive_sources)
    transform_sources = sum(
        name in archive_sources and digest((ROOT / name).read_bytes()) == archive_sources[name]
        for name in (
            "tools/transformations-banc/Cargo.toml",
            "tools/transformations-banc/Cargo.lock",
            "tools/transformations-banc/README.md",
            "tools/transformations-banc/src/lib.rs",
            "tools/transformations-banc/src/main.rs",
            "tools/transformations-banc/src/index_memory.rs",
            "tools/transformations-banc/executer.py",
            "tools/transformations-banc/fixtures/profile.toml",
        )
    )
    require(transform_sources == 8, "sources transformations actuelles différentes de l'archive")
    temporal_sources = check_campaign_sources(read_json(ROOT / index["experiments"]["fenetres-temporelles-banc"] / "campagne.json"), archive_sources, "tools/fenetres-temporelles-banc/")
    workspace_tests, workspace_suites = check_workspace_check()
    engine_runs, engine_groups = check_engine(index["experiments"])
    transform_measurements, index_heap = check_transformations(index["experiments"])
    temporal_runs, temporal_groups, temporal_production_runs = check_temporal(index["experiments"])
    html_links = check_html()
    reproduction_sources = check_final_reproduction(campaign_path)
    campaign_dirs = [ROOT / index["campaign"]] + [ROOT / path for path in index["experiments"].values()]
    private_scan_files = check_private_paths(index_path, campaign_dirs)
    print(
        "PASS "
        f"sources_rust={rust_files} sources_archive={archived_files} "
        f"engine={engine_runs}/{engine_groups}_groupes/{engine_sources}_sources "
        f"transformations={transform_measurements}_mesures/3_processus index_heap_sum={index_heap} "
        f"fenetres={temporal_runs}/{temporal_groups}_groupes/{temporal_production_runs}_API_production "
        f"workspace={workspace_tests}_tests/{workspace_suites}_suites action_effects=asserted "
        f"private_path_scan={private_scan_files}_fichiers "
        f"html_links={html_links} reproduction={reproduction_sources}"
    )


if __name__ == "__main__":
    try:
        main()
    except (KeyError, TypeError, ValueError, OSError) as exc:
        print(f"FAIL {exc}", file=sys.stderr)
        sys.exit(1)
