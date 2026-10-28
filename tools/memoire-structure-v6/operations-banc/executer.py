#!/usr/bin/env python3
"""Frozen-source V6 microkernel campaign; Docker only, no third-party packages."""
import collections
import hashlib
import json
import math
import pathlib
import platform
import statistics
import subprocess
import tempfile
import time

HERE = pathlib.Path(__file__).resolve().parent
PARENT = HERE.parent
ROOT = HERE.parents[2]
OUT = HERE / "sorties"
IMAGE = "moli-os-dev:latest"
FLAGS = "--edition=2024 -D warnings -C opt-level=s -C codegen-units=1"

def run(args):
    result = subprocess.run(args, text=True, capture_output=True, timeout=120)
    if result.returncode:
        raise RuntimeError(f"Command failed ({result.returncode}): {args!r}\n{result.stdout}\n{result.stderr}")
    return result.stdout

def sha(raw):
    return hashlib.sha256(raw).hexdigest()

def main():
    started = time.monotonic()
    manifest_path = ROOT / "tools/catalogue-banc/charges/manifest.json"
    manifest_raw = manifest_path.read_bytes()
    manifest = json.loads(manifest_raw)
    scenarios = []
    for data in manifest["datasets"]:
        if data["label"].endswith("liens4"):
            continue
        scales = (1, 10, 100) if data["label"] == "somme-detail" else (1,)
        for scale in scales:
            # Same dimensional formula as V6 budget.counts; no raw HA state is read.
            entities = data["coverage"]["source_entities"]
            canonical = (data["rows"] + 39 + math.ceil(entities / 4) + 32 + 2 * data["devices"]) * scale
            scenarios.append(dict(
                scenario=f'{data["label"]}-x{scale}', dataset=data["label"], scale=scale,
                canonical_rows=canonical, physical_nodes=data["devices"] * scale,
                projected_aliases=entities * scale,
                manifest_rows=data["rows"] * scale,
                dimensions_source="public synthetic manifest shapes; canonical formula from V6 budget.counts",
            ))
    files = {
        "layouts.rs": (PARENT / "layouts.rs").read_bytes(),
        "model.rs": (PARENT / "model.rs").read_bytes(),
        "operations-banc/main.rs": (HERE / "main.rs").read_bytes(),
        "operations-banc/executer.py": pathlib.Path(__file__).read_bytes(),
        "operations-banc/README.md": (HERE / "README.md").read_bytes(),
    }
    source_hash = {name: sha(raw) for name, raw in files.items()}
    image_id = run(["docker", "image", "inspect", IMAGE, "--format", "{{.Id}}"]).strip()
    raw_lines = []
    with tempfile.TemporaryDirectory(prefix=".build-", dir=HERE) as temp:
        snapshot = pathlib.Path(temp)
        for name, raw in files.items():
            target = snapshot / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(raw)
        base = ["docker", "run", "--rm", "--network", "none", "--cpus", "1", "--memory", "512m", "-v", f"{snapshot}:/build", image_id]
        toolchain = run(base + ["sh", "-c", "rustc --version --verbose && uname -a"])
        run(base + ["sh", "-c", f"rustc {FLAGS} /build/operations-banc/main.rs -o /build/operations"])
        semantic = json.loads(run(base + ["/build/operations", "--self-test"]))
        assert semantic["passed"] is True
        args = [f'{s["scenario"]}:{s["canonical_rows"]}:{s["physical_nodes"]}' for s in scenarios]
        all_samples = []
        memories = {}
        for process in range(3):
            output = run(base + ["/build/operations", *args])
            for line in output.splitlines():
                event = json.loads(line)
                event["process"] = process
                raw_lines.append(event)
                if event["kind"] == "sample":
                    all_samples.append(event)
                elif event["kind"] == "memory":
                    if event["scenario"] not in memories:
                        memories[event["scenario"]] = {k: v for k, v in event.items() if k not in ("kind", "process")}
    for name, raw in files.items():
        assert sha((PARENT / name).read_bytes()) == source_hash[name], f"Source changed during campaign: {name}"
    assert manifest_path.read_bytes() == manifest_raw, "Manifest changed during campaign"
    grouped = collections.defaultdict(list)
    for sample in all_samples:
        grouped[(sample["scenario"], sample["operation"], sample["variant"], sample["fanout"])].append(sample)
    measurements = []
    lookup_scenarios = {s["scenario"]: s for s in scenarios}
    for (scenario, operation, variant, fanout), samples in grouped.items():
        assert len(samples) == 9
        timings = sorted(s["ns_per_op"] for s in samples)
        measurements.append(dict(
            scenario=scenario, scale=lookup_scenarios[scenario]["scale"], operation=operation,
            variant=variant, fanout=fanout or None, iterations=samples[0]["iterations"],
            unit="ns/op", median_ns=statistics.median(timings), min_ns=min(timings), max_ns=max(timings),
            p95_sample_ns=timings[math.ceil(len(timings) * 0.95) - 1],
            allocations_per_op=statistics.median(s["allocations_per_op"] for s in samples),
            allocated_bytes_per_op=statistics.median(s["allocated_bytes_per_op"] for s in samples),
            samples=[{k: v for k, v in s.items() if k not in ("kind", "scenario", "operation", "variant", "fanout")} for s in samples],
        ))
    for scenario in scenarios:
        scenario.update(memories[scenario["scenario"]])
    limits = [
        "Measured isolated microkernels over the actual V6 Row96/Bank, not the complete engine, command latency, device/network notifications, LLM, or a replay of HA exports.",
        "Manifest A/B/sum principal/detail determines canonical arena lengths and physical node counts. Node values, topology, aliases, dates and queries are fully synthetic; projected aliases are not all materialized.",
        "Each fixture materializes 32 scattered canonical sources and 64 aliases per source (2048 aliases), reused for fanout 1/4/16/64. The separate 2048 Row96 views are a materialization baseline, not additional production state.",
        "Bulk setup, identity validation, HashMap construction/reservation, CSR build and query buffers are outside timed regions. Current Bank.insert scans free slots linearly; bulk creation performance is not qualified by this benchmark.",
        "Lookup variants return the same canonical row and digest over exactly the same deterministic query set; linear iterations capped at 128, indexed iterations repeat that set. Results omit aliases from the linear lookup comparison.",
        "Observe identical and changed_previous are different behaviors, reported in absolute elapsed ns/op. Both include a canonical read/digest afterward. Repeated reports retain previous/changed; changing reports publish previous state.",
        "Fanout update measures one real canonical observation plus one canonical read/digest; the view baseline additionally copies the entire Row96 to fanout views. It does not traverse subscribers or emit notifications. Alias reads are measured separately.",
        "Zone variants return the same member slot list/sum. Dense synthetic canonical IDs allow scan to convert target ID to slot by subtracting one. This avoids adding a HashMap lookup only to scan; arbitrary durable IDs need a separate mapping strategy.",
        "Three independent processes, three rounds per process after warmup, Instant elapsed nanoseconds (not CPU cycles/process CPU time). A sample percentile over nine rounds is not a request-tail p95 or a confidence interval.",
        "CountingAllocator reports successful allocation calls and requested bytes per timed op, including realloc via default allocator behavior; atomic instrumentation perturbs allocating kernels. Allocation counts are not RSS or allocator fragmentation.",
        "requested_bank_allocation_bytes covers Bank Vec/HashMap requested backing allocations before CSR. memory.vectors describes actual Vec elements/length/capacity including CSR. Views and query buffers are separate. Headers, stacks, allocator metadata, fragmentation and page residency are excluded.",
        "CPU quota 1 and memory limit 512MiB bound the container; the VM/host scheduling and CPU cache are not isolated. Canonical arena scale x100 is measured capacity, not full HA semantics or a raised product quota.",
    ]
    result = dict(
        metadata=dict(
            timestamp_utc=time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), image=IMAGE, image_id=image_id,
            flags=FLAGS, toolchain=toolchain, host_machine=platform.machine(), cpus=1, memory="512m", network="none",
            source_sha256=source_hash, manifest_sha256=sha(manifest_raw),
            process_repetitions=3, rounds_per_process=3, index_build_outside_timing=True,
            timing="Instant elapsed ns/op", instrumentation="GlobalAlloc counting calls and requested bytes",
            warmup="min(iterations,256) operations per measurement before three timed rounds",
            campaign_wall_seconds=time.monotonic() - started,
            row_contract="96-byte Row96, current/previous distinct values, received_ms/changed_ms",
        ), semantic_tests=semantic, scenarios=scenarios, measurements=measurements, limits=limits,
    )
    OUT.mkdir(exist_ok=True)
    (OUT / "resultats.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    (OUT / "echantillons.jsonl").write_text("".join(json.dumps(event, ensure_ascii=False) + "\n" for event in raw_lines))
    print(json.dumps(dict(scenarios=len(scenarios), measurements=len(measurements), samples=len(all_samples), semantic_tests=semantic["passed"], wall_seconds=result["metadata"]["campaign_wall_seconds"])))

if __name__ == "__main__":
    main()
