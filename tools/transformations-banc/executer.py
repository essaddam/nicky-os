#!/usr/bin/env python3
"""Offline benchmark campaign; stdlib only, no network or physical commands."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--iterations", type=int, default=20000)
    parser.add_argument("--samples", type=int, default=7)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.iterations < 1 or args.samples < 1:
        parser.error("iterations and samples must be positive")
    here = Path(__file__).resolve().parent
    root = here.parent.parent
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    output = args.output or here / "sorties" / stamp
    output.mkdir(parents=True, exist_ok=False)
    manifest = str((here / "Cargo.toml").relative_to(root))
    lock = ["--locked"] if (here / "Cargo.lock").exists() else []
    commands = [
        ("tests.txt", ["cargo", "test", "--release", "--offline", *lock, "--manifest-path", manifest]),
        ("mesures.jsonl", ["cargo", "run", "--release", "--offline", "--locked", "--manifest-path", manifest,
                           "--", str(args.iterations), str(args.samples)]),
        ("memoire.jsonl", ["cargo", "run", "--release", "--offline", "--locked", "--manifest-path", manifest,
                           "--bin", "index-memory"]),
    ]
    for name, command in commands:
        print("Running:", " ".join(command), flush=True)
        with (output / name).open("w") as stdout, (output / (name + ".stderr")).open("w") as stderr:
            subprocess.run(command, cwd=root, stdout=stdout, stderr=stderr, check=True)
    for name in ["mesures.jsonl", "memoire.jsonl"]:
        with (output / name).open() as stream:
            rows = [json.loads(line) for line in stream if line.strip()]
        (output / name.replace(".jsonl", ".json")).write_text(json.dumps(rows, indent=2) + "\n")
    sources = [
        "Cargo.toml", "Cargo.lock", "crates/moli-z2m/src/catalog.rs", "crates/moli-z2m/src/lib.rs",
        "crates/moli-profile/src/profile.rs", "crates/moli-profile/src/lib.rs",
        "crates/moli-core/src/id.rs", "crates/moli-core/src/value.rs", "crates/moli-core/src/spec.rs",
        "crates/moli-core/src/aggregate.rs", "crates/moli-runtime/src/hub.rs",
        "integrations/daikin-brp069/profile.toml",
    ]
    harness_sources = [*here.glob("*.toml"), *here.glob("Cargo.lock"), *here.glob("*.py"),
                       *here.glob("*.md"), *here.glob("src/*.rs"), *here.glob("fixtures/*")]
    sources += [str(path.relative_to(root)) for path in harness_sources]
    hashes = {name: hashlib.sha256((root / name).read_bytes()).hexdigest() for name in sources}
    snapshot = output / "sources-banc"
    for source in harness_sources:
        target = snapshot / source.relative_to(here)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
    metadata = {
        "schema": 1, "utc": stamp,
        "commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(),
        "platform": platform.platform(), "architecture": platform.machine(),
        "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
        "cargo": subprocess.check_output(["cargo", "--version"], text=True).strip(),
        "rustup_toolchain": os.environ.get("RUSTUP_TOOLCHAIN"),
        # Flags can contain a private absolute path; the hash records divergence.
        "rustflags_sha256": hashlib.sha256(os.environ.get("RUSTFLAGS", "").encode()).hexdigest(),
        "iterations": args.iterations, "samples": args.samples,
        "commands": [command for _, command in commands], "sha256": hashes,
        "limits": "offline conversion adapters; no Hub/guard/HTTP server/MQTT broker/device delivery; experimental index and affine separate from production",
    }
    (output / "campagne.json").write_text(json.dumps(metadata, indent=2) + "\n")
    print(output)


if __name__ == "__main__":
    main()
