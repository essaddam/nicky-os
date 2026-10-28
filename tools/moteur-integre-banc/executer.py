#!/usr/bin/env python3
"""Run independent, paired processes against production runtime crates.

Commands are tokenized with shlex, never executed through a shell. They may be
native binaries or complete `docker run ... /target/binary` launch commands.
"""
import argparse
import hashlib
import json
import platform
import shlex
import statistics
import subprocess
import sys
import time
from pathlib import Path


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def metric(row, name):
    current = row
    for part in name.split("."):
        current = current[part]
    return current


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--counted-command", required=True)
    parser.add_argument("--timing-command", required=True)
    parser.add_argument("--sizes", default="206,290,496,2060,2900,4960")
    parser.add_argument("--rounds", type=int, default=10)
    parser.add_argument("--repetitions", type=int, default=3)
    parser.add_argument("--output", type=Path, default=Path(__file__).parent / "sorties")
    args = parser.parse_args()
    sizes = [int(size) for size in args.sizes.split(",")]
    if not (1 <= args.repetitions <= 100 and 1 <= args.rounds <= 20):
        parser.error("repetitions: 1..100; rounds: 1..20")
    if any(size < 4 or size > 100_000 for size in sizes):
        parser.error("sizes: 4..100000")
    args.output.mkdir(parents=True, exist_ok=True)
    raw_path = args.output / "brut.jsonl"
    if raw_path.exists():
        parser.error("output already contains brut.jsonl; use a fresh directory")
    root = Path(__file__).resolve().parents[2]
    sources = []
    for folder in ("moli-core", "moli-runtime", "moli-automation", "moli-i18n"):
        sources.extend(sorted((root / "crates" / folder / "src").rglob("*.rs")))
        sources.append(root / "crates" / folder / "Cargo.toml")
    sources.extend(sorted(Path(__file__).parent.glob("src/*.rs")))
    sources.extend([Path(__file__), Path(__file__).parent / "Cargo.toml", Path(__file__).parent / "Cargo.lock"])
    source_hashes = {str(path.relative_to(root)): digest(path) for path in sources}
    manifest = {
        "schema": 1,
        "host_platform": platform.platform(),
        "counted_command": shlex.split(args.counted_command),
        "timing_command": shlex.split(args.timing_command),
        "sizes": sizes, "rounds": args.rounds, "repetitions": args.repetitions,
        "source_sha256": source_hashes,
        "scope": "production Hub + computed + Automations; synthetic transport; layouts are adapters only",
    }
    (args.output / "campagne.json").write_text(json.dumps(manifest, indent=2) + "\n")
    rows = []
    with raw_path.open("w") as raw:
        for size in sizes:
            for repetition in range(args.repetitions):
                # Rotate layout order across processes to reduce fixed-order bias.
                layouts = ["uniform96", "compact64", "columns"]
                offset = repetition % len(layouts)
                layouts = layouts[offset:] + layouts[:offset]
                for layout in layouts:
                    for mode, command in (("counted", args.counted_command), ("timing_plain", args.timing_command)):
                        argv = shlex.split(command) + ["--devices", str(size), "--rounds", str(args.rounds), "--layout", layout]
                        started = time.perf_counter_ns()
                        completed = subprocess.run(argv, text=True, capture_output=True, timeout=120, check=False)
                        name = f"{size}-{layout}-{mode}-{repetition}"
                        (args.output / f"{name}.stderr.txt").write_text(completed.stderr)
                        if completed.returncode:
                            raise RuntimeError(f"{name} failed ({completed.returncode}): {completed.stderr}")
                        row = json.loads(completed.stdout)
                        if row["measurement_mode"] != mode or row["replay_layout"] != layout:
                            raise RuntimeError(f"binary/mode mismatch in {name}")
                        row.update({"repetition": repetition, "source_devices": size, "process_wall_ns": time.perf_counter_ns() - started})
                        raw.write(json.dumps(row, separators=(",", ":")) + "\n")
                        raw.flush()
                        rows.append(row)
                        print(f"{name}: assertions passed", file=sys.stderr)
    summaries = []
    fields = (
        "raw_observations_per_second", "adapter_payload_and_handles_bytes",
        "adapter_live_requested_bytes", "hub_setup_net_requested_bytes",
        "populated_live_requested_bytes", "final_live_requested_bytes",
        "workload_peak_requested_bytes", "workload_allocated_bytes", "loaded_rss_bytes",
        "rss_bytes", "process_peak_rss_bytes", "loaded_anonymous_bytes",
        "steady_anonymous_bytes", "sampled_peak_anonymous_bytes",
        "batch_ack_latency_ns.p50", "batch_ack_latency_ns.p95",
        "propagation_and_automation_latency_ns.p50", "propagation_and_automation_latency_ns.p95",
        "setter_round_trip_latency_ns.p50", "setter_round_trip_latency_ns.p95",
        "action_round_trip_latency_ns.p50", "action_round_trip_latency_ns.p95",
    )
    for size in sizes:
        same_size = [row for row in rows if row["source_devices"] == size]
        # Every process already checks all final values; compare cross-layout outputs too.
        oracles = {(row["final_value_checksum"], row["automation_runs"], row["bus_occurrences"], tuple(row["assertions"])) for row in same_size}
        if len(oracles) != 1:
            raise RuntimeError(f"cross-layout/mode correctness mismatch for {size}")
        for layout in ("uniform96", "compact64", "columns"):
            for mode in ("counted", "timing_plain"):
                group = [row for row in same_size if row["replay_layout"] == layout and row["measurement_mode"] == mode]
                summary = {"source_devices": size, "layout": layout, "measurement_mode": mode, "processes": len(group)}
                for field in fields:
                    values = [metric(row, field) for row in group if metric(row, field) is not None]
                    summary[field] = {"median": statistics.median(values), "min": min(values), "max": max(values)} if values else None
                summaries.append(summary)
    (args.output / "resultats.json").write_text(json.dumps({"schema": 1, "runs": len(rows), "groups": summaries}, indent=2) + "\n")
    print(json.dumps({"runs": len(rows), "raw": str(raw_path), "summary": str(args.output / "resultats.json")}))


if __name__ == "__main__":
    main()
