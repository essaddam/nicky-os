#!/usr/bin/env python3
"""Read-only audit of the saved V6 structural proof and its report."""

from __future__ import annotations

import hashlib
import importlib.util
import html.parser
import json
import math
import re
import statistics
import sys
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import unquote, urlsplit


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
OUT = HERE / "sorties"
REPORT = ROOT / "docs/proposition/analyse/catalogue-memoire-v6.html"

EXPECTED_LAYOUTS = {
    "Row64": (64, 8, {"id": 0, "parent": 8, "payload": 16, "previous_payload": 24, "received_ms": 32, "changed_ms": 40, "schema": 48, "aux": 52, "revision": 56, "flags": 60, "row_kind": 62, "value_kind": 63}),
    "Numeric32": (32, 8, {"min": 0, "max": 8, "step": 16, "anchor": 24}),
    "Row96": (96, 8, {"base": 0, "numeric": 64}),
    "Hot32": (32, 8, {"payload": 0, "previous_payload": 8, "received_ms": 16, "changed_ms": 24}),
    "Cold32": (32, 8, {"id": 0, "parent": 8, "schema": 16, "aux": 20, "revision": 24, "flags": 28, "row_kind": 30, "value_kind": 31}),
    "Alias32": (32, 8, {"alias_id": 0, "owner_id": 8, "source_id": 16, "source_slot": 24, "source_generation": 28}),
    "GlobalSlot16": (16, 4, {"physical_index": 0, "generation": 4, "meta_index": 8, "arena": 12, "flags": 13, "reserved": 14}),
    "Handle8": (8, 4, {"slot": 0, "generation": 4}),
    "Meta32": (32, 4, {"label": 0, "native_key": 4, "location": 8, "policy": 12, "read_plan": 16, "write_plan": 20, "executable": 24, "extra": 28}),
    "Schema64": (64, 8, {"unit": 0, "semantic": 4, "numeric_repr": 6, "decimals": 7, "prefix_exp10": 8, "value_kind": 9, "flags": 10, "max_bytes": 12, "max_items": 16, "composite_schema": 20, "native_min": 24, "native_max": 32, "scale_num": 40, "scale_den": 48, "reserved": 56}),
    "Transform64": (64, 8, {"mul": 0, "add": 8, "in_min": 16, "in_max": 24, "out_min": 32, "out_max": 40, "revision": 48, "opcode": 52, "flags": 54, "input_kind": 56, "output_kind": 57, "rounding": 58, "overflow": 59, "reserved": 60}),
    "Edge24": (24, 8, {"from": 0, "to": 8, "relation": 16, "flags": 18, "revision": 20}),
    "Target12": (12, 4, {"to": 0, "relation": 4, "flags": 6, "revision": 8}),
    "PoolEntry16": (16, 4, {"offset": 0, "len": 4, "generation": 8, "kind": 12, "flags": 14}),
    "Slice8": (8, 4, {"offset": 0, "len": 4}),
    "Component32": (32, 4, {"a": 0, "b": 4, "c": 8, "d": 12, "e": 16, "f": 20, "g": 24, "h": 28}),
    "Calc48": (48, 8, {"definition_id": 0, "deadline_tick": 8, "source_ts": 16, "version": 24, "first_edge": 28, "edge_count": 32, "operation": 34, "flags": 35, "history_ref": 36, "unit_ref": 40, "reserved": 44}),
    "Route32": (32, 8, {"point_id": 0, "handle": 8, "read_plan": 16, "write_plan": 20, "native_key": 24, "flags": 28, "codec": 30, "access": 31}),
}
VARIANTS = ("row96", "row64", "split")
EXEMPT_VARIANT_ZONES = {"rows", "numeric", "numeric_refs", "headers"}
FORBIDDEN_PATH = re.compile(r"(?:/Users/[^/\s]+|/home/[^/\s]+|/private/var/[^\s]+|/Volumes/[^/\s]+)")


class AuditError(Exception):
    pass


def require(ok: bool, message: str) -> None:
    if not ok:
        raise AuditError(message)


def read_json(path: Path):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise AuditError(f"cannot read JSON {path.relative_to(ROOT)}: {exc}") from exc


def index_formula(n: int, probes: dict[str, dict]) -> dict:
    if str(n) in probes:
        probe = probes[str(n)]
        return {"bytes": probe["requested_bytes"], "capacity": probe["capacity"], "method": "measured_reservation"}
    bucket = max(8, 1 << (((8 * n + 6) // 7 - 1).bit_length()))
    return {"bytes": 17 * bucket + 8, "capacity": bucket * 7 // 8, "method": "extrapolated_same_toolchain"}


def dataset_counts(dataset: dict, scale: int) -> dict:
    coverage = dataset["coverage"]
    values = dataset["values"] * scale
    devices = dataset["devices"] * scale
    org = 39 * scale
    bindings = 2 * dataset["devices"] * scale
    virtual = math.ceil(coverage["source_entities"] / 4) * scale
    calcs = 32 * scale
    rows = dataset["rows"] * scale
    canonical = rows + org + virtual + calcs + bindings
    aliases = coverage["source_entities"] * scale
    slots = canonical + aliases
    parent_edges = slots - scale
    member_edges = 2 * (24 * scale + devices + virtual)
    calc_edges = 2 * calcs
    numeric = dataset["numeric_rows"] * scale + calcs
    variable_entries = (dataset["value_kinds"].get("text", 0) + dataset["value_kinds"].get("record", 0)) * scale
    routes = values + bindings
    record_containers = dataset["value_kinds"].get("record", 0) * scale
    return {
        "base_rows": rows,
        "org": org,
        "devices": devices,
        "values": values,
        "num": dataset["numeric_rows"] * scale,
        "contracts": dataset["actions"] * scale,
        "bindings": bindings,
        "virtual": virtual,
        "calcs": calcs,
        "canonical": canonical,
        "aliases": aliases,
        "slots": slots,
        "edges_parent": parent_edges,
        "edges_members": member_edges,
        "edges_calc": calc_edges,
        "edges": parent_edges + member_edges + calc_edges,
        "numeric": numeric,
        "variable_entries": variable_entries,
        "routes": routes,
        "record_containers": record_containers,
    }


def zone_formula(dataset: dict, counts: dict, assumptions: dict, probes: dict, variant: str, scale: int) -> dict:
    slots = counts["slots"]
    canonical = counts["canonical"]
    aliases = counts["aliases"]
    edges = counts["edges"]
    strings = slots + counts["values"] + counts["devices"]
    common = {
        "aliases": 32 * aliases,
        "slots": 16 * slots,
        "metadata": 32 * slots,
        "id_index": index_formula(slots, probes)["bytes"],
        "edges": 24 * edges,
        "csr": 16 * edges + 8 * (slots + 1),
        "alias_reverse": 4 * (slots + 1) + 4 * aliases,
        "typed_index": 4 * slots + 68,
        "schemas": 64 * assumptions["schemas_shared"],
        "transforms": 64 * assumptions["transforms_shared"],
        "components": 32 * (counts["org"] + counts["devices"] + counts["virtual"] + counts["bindings"] + counts["contracts"] + counts["record_containers"]),
        "calculations": 48 * counts["calcs"],
        "freshness": 32 * counts["numeric"],
        "routes": 32 * counts["routes"],
        "route_index": index_formula(counts["routes"], probes)["bytes"],
        "pool_entries": 16 * counts["variable_entries"],
        "pool_bytes": dataset["payload_pool_bytes"] * scale,
        "string_bytes": assumptions["label_bytes"] * slots + assumptions["key_bytes"] * counts["values"] + assumptions["native_id_bytes"] * counts["devices"],
        "string_entries": 8 * strings,
        "string_search": 16 * (counts["values"] + counts["devices"]),
        "headers": 24 * assumptions["vector_headers"] + 48 * 2 + 64,
        "runtime_buffers": assumptions["decode_workers"] * assumptions["decode_buffer_bytes"] + (assumptions["event_slots"] + assumptions["command_slots"]) * assumptions["queue_envelope_bytes"],
    }
    if variant == "row96":
        common.update(rows=96 * canonical, numeric=0, numeric_refs=0)
    elif variant == "row64":
        common.update(rows=64 * canonical, numeric=32 * counts["numeric"], numeric_refs=4 * canonical, headers=common["headers"] + 48)
    elif variant == "split":
        common.update(rows=64 * canonical, numeric=32 * counts["numeric"], numeric_refs=4 * canonical, headers=common["headers"] + 72)
    else:
        raise AuditError(f"unknown variant {variant}")
    return common


class LinksAndAnchors(html.parser.HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.anchors: list[str] = []
        self.links: list[str] = []

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if attrs.get("id"):
            self.anchors.append(attrs["id"])
        if tag == "a":
            if attrs.get("name"):
                self.anchors.append(attrs["name"])
            if attrs.get("href"):
                self.links.append(attrs["href"])


def check_report() -> list[str]:
    require(REPORT.is_file(), "V6 HTML report is missing")
    raw = REPORT.read_text(encoding="utf-8")
    parser = LinksAndAnchors()
    parser.feed(raw)
    duplicates = sorted({a for a in parser.anchors if parser.anchors.count(a) > 1})
    require(not duplicates, f"duplicate HTML anchors: {duplicates}")
    anchor_set = set(parser.anchors)
    for link in parser.links:
        parsed = urlsplit(link)
        if parsed.scheme or parsed.netloc or link.startswith("mailto:"):
            continue
        target = REPORT.parent / unquote(parsed.path) if parsed.path else REPORT
        require(target.resolve().is_file(), f"missing local report source: {link}")
        if parsed.fragment and target.resolve() == REPORT.resolve():
            require(unquote(parsed.fragment) in anchor_set, f"broken report fragment: {link}")
        elif parsed.fragment and target.suffix.lower() in {".html", ".htm"}:
            linked = LinksAndAnchors()
            linked.feed(target.read_text(encoding="utf-8"))
            require(unquote(parsed.fragment) in set(linked.anchors), f"broken linked HTML fragment: {link}")
    require("CALCULÉ" in raw and "sonde ou extrapolation" in raw, "budget labels do not distinguish calculation from index probe/extrapolation")
    require("calculated_not_rss" in raw and "allocation ni RSS" in raw, "report omits calculated-not-RSS scope")
    return ["HTML anchors unique", "local report sources and fragments resolve", "calculated versus probed labels and RSS scope present"]


def check_privacy(paths: list[Path]) -> int:
    scanned = 0
    for path in paths:
        if not path.exists():
            continue
        items = [path] if path.is_file() else path.rglob("*")
        for item in items:
            if not item.is_file() or item.suffix.lower() not in {".json", ".jsonl", ".txt", ".html", ".md", ".rs", ".py", ".js"}:
                continue
            try:
                content = item.read_text(encoding="utf-8")
            except UnicodeDecodeError:
                continue
            scanned += 1
            match = FORBIDDEN_PATH.search(content)
            require(match is None, f"personal absolute path in output: {item.relative_to(ROOT)}")
    return scanned


def check_operations(metadata: dict, datasets: list[dict]) -> list[str]:
    folder = HERE / "operations-banc"
    result = read_json(folder / "sorties/resultats.json")
    meta = result["metadata"]
    require(meta["image_id"] == metadata["image_id"], "operation bank and structure image differ")
    require(meta["cpus"] == 1 and meta["memory"] == "512m" and meta["network"] == "none", "operation bank run limits differ")
    require(meta["process_repetitions"] == 3 and meta["rounds_per_process"] == 3, "operation bank lacks three independent processes and rounds")
    require(meta["flags"] == metadata["flags"], "operation bank compilation flags differ")
    expected_sources = {"layouts.rs", "model.rs", "operations-banc/main.rs", "operations-banc/executer.py", "operations-banc/README.md"}
    require(set(meta["source_sha256"]) == expected_sources, "operation bank source snapshot differs")
    for name, wanted in meta["source_sha256"].items():
        require(hashlib.sha256((HERE / name).read_bytes()).hexdigest() == wanted, f"stale operation source {name}")
    require(meta["manifest_sha256"] == hashlib.sha256((ROOT / "tools/catalogue-banc/charges/manifest.json").read_bytes()).hexdigest(), "operation bank manifest changed")
    semantic = result["semantic_tests"]
    require(semantic["passed"] is True and len(semantic["cases"]) == 7, "operation bank semantic parity proof absent")
    by_dataset = {item["label"]: item for item in datasets}
    scenarios = {item["scenario"]: item for item in result["scenarios"]}
    require(len(scenarios) == 8, "operation bank must preserve all eight dimensions")
    expected_dimensions = {(label, 1) for label in by_dataset} | {("somme-detail", 10), ("somme-detail", 100)}
    require({(item["dataset"], item["scale"]) for item in scenarios.values()} == expected_dimensions, "operation dimensions differ from public source projections")
    for item in scenarios.values():
        counts = dataset_counts(by_dataset[item["dataset"]], item["scale"])
        require(item["canonical_rows"] == counts["canonical"] and item["physical_nodes"] == counts["devices"] and item["projected_aliases"] == counts["aliases"], "operation projection count formula differs")
        require(item["materialized_aliases"] == 2048 and item["instrumented_sources"] == 32, "operation materialized alias scope differs")
        vectors = item["memory"]["vectors"]
        require(vectors["rows"]["len"] == counts["canonical"] and vectors["rows"]["element_bytes"] == 96, "operation Row96 arena differs")
        for vector in vectors.values():
            require(vector["used_bytes"] == vector["len"] * vector["element_bytes"] and vector["capacity_bytes"] == vector["capacity"] * vector["element_bytes"] and vector["capacity"] >= vector["len"], "operation Vec used/reserved bytes inconsistent")
    expected_groups = {("lookup", "linear_scan", None), ("lookup", "bank_read_at", None), ("observe", "identical", None), ("observe", "changed_previous", None), ("zone_members", "edge24_scan", None), ("zone_members", "csr_targets", None)}
    expected_groups |= {(op, variant, fanout) for fanout in (1, 4, 16, 64) for op, variant in (("fanout_update", "canonical_alias"), ("fanout_update", "materialized_row96_views"), ("alias_read", "bank_read_at"))}
    rows = result["measurements"]
    require(len(rows) == 144, "operation bank must contain 144 measured groups")
    for scenario in scenarios:
        selected = [row for row in rows if row["scenario"] == scenario]
        require(len(selected) == 18 and {(row["operation"], row["variant"], row["fanout"]) for row in selected} == expected_groups, "operation metric set differs")
    samples = 0
    for row in rows:
        points = row["samples"]
        require(len(points) == 9 and {(point["process"], point["round"]) for point in points} == {(process, round_) for process in range(3) for round_ in range(3)}, "operation group lacks nine independent process/round pairs")
        values = [point["ns_per_op"] for point in points]
        require(row["unit"] == "ns/op" and all(math.isfinite(value) and value > 0 for value in values), "operation timing invalid")
        require(row["median_ns"] == statistics.median(values) and row["min_ns"] == min(values) and row["max_ns"] == max(values), "operation median/range differs from samples")
        for point in points:
            require(point["iterations"] > 0 and math.isclose(point["ns_per_op"], point["elapsed_ns"] / point["iterations"]), "operation timing does not match elapsed time/iterations")
        samples += len(points)
    require(samples == 1296, "operation sample count differs")
    require(len((folder / "sorties/echantillons.jsonl").read_text().splitlines()) >= samples, "raw operation evidence absent")
    return ["operation bank: eight dimensions, current source hashes, seven semantic invariants, 144 groups and 1296 elapsed-time samples verified"]


def run() -> dict:
    metadata = read_json(OUT / "metadata.json")
    layouts = read_json(HERE / "layouts.json")
    layouts_out = read_json(OUT / "layouts.json")
    demo = read_json(HERE / "demo.json")
    demo_out = read_json(OUT / "demo.json")
    budgets = read_json(OUT / "budgets.json")
    probes = read_json(OUT / "index-probe.json")
    manifest_path = ROOT / "tools/catalogue-banc/charges/manifest.json"
    manifest = read_json(manifest_path)
    checks: list[str] = []

    current_sources = {path.name: hashlib.sha256(path.read_bytes()).hexdigest() for path in sorted(HERE.glob("*.rs"))}
    require(current_sources == metadata.get("source_sha256"), "metadata source hashes differ from current Rust files")
    require(len(current_sources) == 6, f"expected six measured Rust sources, got {len(current_sources)}")
    require(metadata.get("flags") == "--edition=2024 -D warnings -C opt-level=s -C codegen-units=1", "unexpected rustc flags in metadata")
    require(metadata.get("image_id", "").startswith("sha256:") and len(metadata["image_id"]) == 71, "missing pinned build image ID")
    require("aarch64-unknown-linux-musl" in metadata.get("toolchain", "") and "aarch64" in metadata["toolchain"], "metadata toolchain does not match measured target")
    runner = (HERE / "executer.py").read_text(encoding="utf-8")
    require("image_id=run(['docker','image','inspect',image" in runner and "probe_index.rs" in runner and "base=['docker','run','--rm','--network','none','--cpus','1','--memory','512m','-v',str(snap)+':/build',image]" in runner, "layout and index probe are not recorded under the same image/run configuration")
    probe_source = (HERE / "probe_index.rs").read_text(encoding="utf-8")
    require("drop(map);" in probe_source and "assert_eq!(before, after, \"map allocation must be released\")" in probe_source, "HashMap probe does not prove allocation deallocation")
    require(metadata.get("index_reservations") == len(probes) == 23, "metadata/index probe count mismatch")
    checks.append("current Rust source hashes and recorded build environment match")

    require(layouts == layouts_out, "root and sortie layout JSON differ")
    require(layouts.get("pointer_bytes") == 8 and layouts.get("endianness") == "little", "layout target differs from 64-bit little-endian target")
    observed = {entry["name"]: entry for entry in layouts.get("structures", [])}
    require(set(observed) == set(EXPECTED_LAYOUTS) and len(observed) == 18, "expected exactly 18 unique layout structures")
    for name, (size, alignment, offsets) in EXPECTED_LAYOUTS.items():
        entry = observed[name]
        require((entry.get("size"), entry.get("align"), entry.get("offsets")) == (size, alignment, offsets), f"layout mismatch for {name}")
    checks.append("18 layouts, sizes, alignments, and all offsets match the expected ABI")

    test_source = (HERE / "tests.rs").read_text(encoding="utf-8")
    test_output = (OUT / "tests.txt").read_text(encoding="utf-8")
    require("alternative_arenas_preserve_every_field_and_explicit_numeric_refs" in test_source and "test tests::alternative_arenas_preserve_every_field_and_explicit_numeric_refs ... ok" in test_output, "alternative arena field/reference conservation lacks recorded test evidence")
    test_count = metadata.get("test_count", 0)
    require(metadata.get("row_contract") == "two_dates_and_previous_distinct_value", "old three-date row contract recorded")
    require(budgets.get("row_contract") == metadata["row_contract"], "budget and measured row contracts differ")
    require(test_count >= 14 and f"test result: ok. {test_count} passed; 0 failed" in test_output, "saved Rust test count/proof differs from metadata")
    for name in ("first_null_observation_is_present_even_with_zero_timestamps", "first_integer_zero_has_no_previous_and_sets_changed_at", "previous_tracks_distinct_values_and_is_shared_by_aliases", "type_changes_keep_null_and_composite_previous_types"):
        require(f"test tests::{name} ... ok" in test_output, f"previous-value contract lacks passing test {name}")
    checks.append(f"saved {test_count}-test proof includes two-date previous-value contract and alternative field conservation")

    require(demo == demo_out, "root and sortie demo JSON differ")
    require(demo.get("epoch") == 1 and "single-writer" in demo.get("scope", ""), "demo epoch/scope mismatch")
    nodes = {int(node["id"]): node for node in demo.get("nodes", [])}
    require(len(nodes) == len(demo.get("nodes", [])), "duplicate demo IDs")
    for alias_id, source_id in ((200, 100), (201, 100), (202, 101)):
        alias = nodes.get(alias_id)
        source = nodes.get(source_id)
        require(alias is not None and source is not None, f"missing demo alias/source {alias_id}->{source_id}")
        require(alias["arena"] == 2 and alias["resolved_id"] == source_id, f"alias {alias_id} resolves incorrectly")
        require(alias["generation"] == 1 and source["generation"] == 1 and alias["slot"] != source["slot"], f"alias {alias_id} slot/generation mismatch")
        require(alias["payload"] == source["payload"] and alias["value_kind"] == source["value_kind"], f"alias {alias_id} does not share source state")
        require(alias.get("previous_payload") == source.get("previous_payload") and alias.get("previous_kind") == source.get("previous_kind"), f"alias {alias_id} does not share source previous value")
    for node in nodes.values():
        require("observed_ms" not in node and "received_ms" in node and "changed_ms" in node, "demo must retain exactly two timestamps")
        require("previous_payload" in node and "previous_kind" in node, "demo lacks nullable previous value/type")
        require((node["previous_payload"] is None) == (node["previous_kind"] is None), "previous absence/type mismatch")
    require("generation_and_epoch_never_wrap" in test_output and "removal_reuse_stale_generation_and_epoch" in test_output, "generation/epoch behavior lacks passing test evidence")
    checks.append("demo aliases 200/201→100 and 202→101 resolve with slot/generation checks")

    datasets = [item for item in manifest["datasets"] if not item["label"].endswith("liens4")]
    for dataset in datasets:
        input_path = manifest_path.parent / f"{dataset['label']}.tsv"
        require(input_path.is_file(), f"missing manifest side input {dataset['label']}.tsv")
        digest = hashlib.sha256(input_path.read_bytes()).hexdigest()
        require(digest == dataset["input_sha256"], f"manifest/input SHA mismatch for {dataset['label']}")
    checks.append("manifest dataset SHA-256 values match all side inputs")
    checks.extend(check_operations(metadata, datasets))
    spec = importlib.util.spec_from_file_location('alignment_validation', HERE / 'alignement-banc/verifier.py')
    alignment_validator = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(alignment_validator)
    alignment = alignment_validator.validate()
    checks.append(f"alignment bank: {alignment['groups']} groups, {alignment['samples']} timing samples, 144 memory captures and 540 cross-layout parity groups")

    expected_measurements = {}
    for key, folder in (("catalogue", "catalogue-banc"), ("aliases", "alias-banc"), ("organisation", "organisation-banc")):
        expected_measurements[key] = read_json(ROOT / f"tools/{folder}/sorties/resultats.json")["results"]
    engine_index = read_json(ROOT / "tools/moteur-suite-banc/sorties/campagnes-v5.json")
    engine_path = Path(engine_index["experiments"]["moteur-integre-banc"])
    if not engine_path.is_absolute():
        engine_path = ROOT / engine_path
    expected_measurements["engine"] = read_json(engine_path / "resultats.json")["groups"]
    require(budgets.get("measurements") == expected_measurements, "legacy measurement payload/keys differ from the saved source results")
    require((ROOT / "tools/catalogue-banc/verifier.py").is_file() and (ROOT / "tools/alias-banc/verifier.py").is_file(), "previous catalogue/alias validators unavailable")
    require((ROOT / "tools/organisation-banc/sorties/tests.txt").is_file() and (engine_path / "resultats.json").is_file(), "previous organization/engine evidence unavailable")
    checks.append("legacy measured payloads are unchanged; prior validator/evidence files are present (not rerun)")

    require(len(budgets.get("budgets", [])) == 12, "expected 12 budget scenarios")
    source_datasets = {item["label"]: item for item in datasets}
    scales_by_dataset = {name: [1] for name in source_datasets}
    for name in ("somme-principal", "somme-detail"):
        scales_by_dataset[name] = [1, 10, 100, 1000]
    expected_pairs = {(name, scale) for name, scales in scales_by_dataset.items() for scale in scales}
    got_pairs = {(entry["dataset"], entry["scale"]) for entry in budgets["budgets"]}
    require(got_pairs == expected_pairs, "budget scenario dataset/scale set differs from requested 12 scenarios")
    assumptions = budgets["assumptions"]
    require(assumptions == {
        "alias_fanout": 1, "values_per_virtual": 4, "bindings_per_device": 2,
        "calcs_per_world": 32, "schemas_shared": 96, "transforms_shared": 16,
        "label_bytes": 24, "key_bytes": 16, "native_id_bytes": 32,
        "decode_workers": 4, "decode_buffer_bytes": 65536, "event_slots": 1024,
        "command_slots": 256, "queue_envelope_bytes": 64, "vector_headers": 32,
    }, "budget assumptions changed unexpectedly")
    for entry in budgets["budgets"]:
        dataset = source_datasets[entry["dataset"]]
        scale = entry["scale"]
        counts = dataset_counts(dataset, scale)
        require(entry["counts"] == counts, f"count formula mismatch for {entry['dataset']} ×{scale}")
        require(len(entry["variants"]) == 3 and set(entry["variants"]) == set(VARIANTS), f"variant set mismatch for {entry['dataset']} ×{scale}")
        for variant in VARIANTS:
            report = entry["variants"][variant]
            zones = report["zones"]
            require(zones == zone_formula(dataset, counts, assumptions, probes, variant, scale), f"zone formula mismatch for {entry['dataset']} ×{scale} {variant}")
            require(sum(zones.values()) == report["total"], f"zone sum does not equal total for {entry['dataset']} ×{scale} {variant}")
        common_keys = set(entry["variants"]["row96"]["zones"])
        for variant in ("row64", "split"):
            require(set(entry["variants"][variant]["zones"]) == common_keys, f"zone set differs for {variant}")
            for zone in common_keys - EXEMPT_VARIANT_ZONES:
                require(entry["variants"][variant]["zones"][zone] == entry["variants"]["row96"]["zones"][zone], f"unfair shared zone {zone} in {variant}")
        base = entry["variants"]["row96"]["total"]
        row64_gain = base - entry["variants"]["row64"]["total"]
        expected_row64_gain = 28 * counts["canonical"] - 32 * counts["numeric"] - 48
        require(row64_gain == expected_row64_gain, f"Row96-Row64 gain formula mismatch for {entry['dataset']} ×{scale}")
        split_gain = base - entry["variants"]["split"]["total"]
        require(split_gain == expected_row64_gain - 24, f"split header delta mismatch for {entry['dataset']} ×{scale}")
        for label, n, item in (("ID", counts["slots"], entry["index_probe"]), ("route", counts["routes"], entry["route_index_probe"])):
            expected = index_formula(n, probes)
            require(item == expected, f"{label} index probe/extrapolation mismatch for {entry['dataset']} ×{scale}")
    checks.append("12 scenarios: count formulas, all zone totals, variant fairness and savings formula pass")

    probe_max = 0
    for n_text, record in probes.items():
        n = int(n_text)
        probe_max = max(probe_max, n)
        require(n <= 2_000_000 and n == record["entries_requested"], f"probe request bound/key mismatch for {n_text}")
        require(record["capacity"] >= n and record["requested_bytes"] > 16 * n, f"probe reservation/capacity inconsistency for {n_text}")
        require(record["scope"] == "reserved HashMap allocation; no RSS claim" and record["entry_layout_bytes"] == 16 and record["header_bytes"] == 48, f"probe scope/layout mismatch for {n_text}")
        if record["capacity"] >= 7:
            require(record["requested_bytes"] == 17 * (record["capacity"] * 8 // 7) + 8, f"measured bucket allocation formula mismatch for {n_text}")
    require(all((entry["index_probe"]["method"] == "measured_reservation") == (str(entry["counts"]["slots"]) in probes) and (entry["route_index_probe"]["method"] == "measured_reservation") == (str(entry["counts"]["routes"]) in probes) for entry in budgets["budgets"]), "index probe method labels do not follow exact measured requests")
    require(all(entry["index_probe"]["method"] == "extrapolated_same_toolchain" and entry["route_index_probe"]["method"] == "extrapolated_same_toolchain" for entry in budgets["budgets"] if entry["counts"]["slots"] > probe_max), "large index reservations are not labeled extrapolated")
    checks.append("23 bounded index probes exceed key/value payload bytes; deallocation and same-image method are evidenced")

    memory = demo.get("memory", {})
    require("Vec element storage only" in memory.get("scope", "") and "HashMap buckets/control bytes" in memory.get("scope", ""), "demo memory scope omits Vec-only/HashMap exclusions")
    vectors = memory.get("vectors", {})
    require(vectors, "demo has no vector memory accounting")
    for name, vector in vectors.items():
        require(vector["len"] <= vector["capacity"], f"Vec {name} length exceeds capacity")
        require(vector["used_bytes"] == vector["len"] * vector["element_bytes"], f"Vec {name} used bytes mismatch")
        require(vector["capacity_bytes"] == vector["capacity"] * vector["element_bytes"], f"Vec {name} capacity bytes mismatch")
    index = memory.get("index", {})
    require(index.get("allocation_bytes") is None and index["key_value_payload_bytes"] == 16 * index["len"], "demo presents HashMap payload bytes as total allocation")
    checks.append("demo Vec used/capacity bytes are consistent; HashMap total allocation is not claimed")

    html_checks = check_report()
    checks.extend(html_checks)
    lesson = read_json(OUT / "parcours-validation.json")
    require(lesson.get("status") == "pass" and lesson.get("actual_report") is True, "lesson browser proof absent or failing")
    require(lesson.get("cases") == 12 and lesson.get("assertions", 0) > 1800, "lesson browser proof lacks all 12 trajectories")
    require(lesson.get("transition_checks") == 8, "lesson browser proof lacks previous-value transitions")
    require(lesson.get("controls") == "pass" and lesson.get("advanced") == {"layouts": 18, "budgets": 36}, "lesson controls/technical selectors lack browser proof")
    require(lesson.get("console_errors") == [] and lesson.get("external_requests") == [] and lesson.get("body_horizontal_overflow") is False, "lesson browser errors, external traffic, or overflow")
    require(len(lesson.get("source_sha256", {})) == 8, "lesson proof must hash fixture, source, tests, generators, measured operation/alignment results and report")
    require(lesson.get("alignment_views") == {"serial": 48, "parallel": 6, "geometry": 2}, "alignment interactive views lack browser proof")
    for path, wanted in lesson["source_sha256"].items():
        require(hashlib.sha256((ROOT / path).read_bytes()).hexdigest() == wanted, f"stale lesson browser proof for {path}")
    checks.append("current report browser proof: 12 trajectories, step invariants, back/reset, UI controls, 18 layouts and 36 budget views")
    scanned = check_privacy([OUT, HERE / "operations-banc", HERE / "alignement-banc", REPORT])
    checks.append(f"privacy scan found no workstation/home absolute paths ({scanned} text artifacts scanned)")

    return {
        "validated_utc": datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
        "status": "pass",
        "checks": checks,
        "structures": len(EXPECTED_LAYOUTS),
        "rust_tests": test_count,
        "budget_scenarios": len(budgets["budgets"]),
        "variants_per_scenario": len(VARIANTS),
        "index_probes": len(probes),
        "max_probe_request": probe_max,
        "dataset_inputs_verified": len(datasets),
        "text_artifacts_scanned": scanned,
        "report": REPORT.relative_to(ROOT).as_posix(),
    }


def main() -> int:
    try:
        result = run()
    except (AuditError, KeyError, TypeError, ValueError) as exc:
        print(f"V6 structure validation failed: {exc}", file=sys.stderr)
        return 1
    target = OUT / "validation.json"
    target.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({key: value for key, value in result.items() if key not in {"checks"}}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
