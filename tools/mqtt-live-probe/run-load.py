#!/usr/bin/env python3
"""Run the isolated MQTT/Hub load campaign; only aggregate results are saved."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import threading
import time

ROOT = Path(__file__).resolve().parents[2]


def input_fingerprint(exports):
    digest = hashlib.sha256()
    latest = 0.0
    for root in exports:
        base = root / "sanitized"
        files = [base / "runtime/states.json"]
        files += [base / f"registries/core.{kind}_registry.json"
                  for kind in ("device", "entity", "area", "floor")]
        for path in files:
            digest.update(path.name.encode())
            digest.update(path.read_bytes())
            latest = max(latest, path.stat().st_mtime)
    return digest.hexdigest(), latest


class GeneratorMeter:
    """The Rust supervisor/generator PID only, excluding its tested worker."""
    def __init__(self, pid):
        self.pid = pid
        self.samples = []
        self.closed = threading.Event()
        self.started = time.monotonic()
        self.thread = threading.Thread(target=self._loop, daemon=True)
        self.thread.start()

    def _loop(self):
        while not self.closed.is_set():
            output = subprocess.run(["ps", "-o", "rss=,time=", "-p", str(self.pid)],
                                    capture_output=True, text=True, check=False)
            if output.returncode == 0:
                fields = output.stdout.split()
                if len(fields) == 2:
                    cpu = 0.0
                    for value in fields[1].split(":"):
                        cpu = cpu * 60 + float(value)
                    self.samples.append({"seconds": time.monotonic() - self.started,
                                         "rss_bytes": int(fields[0]) * 1024, "cpu_seconds": cpu})
            self.closed.wait(1)

    def finish(self):
        self.closed.set()
        self.thread.join(timeout=3)
        if len(self.samples) < 2:
            return {"samples": self.samples, "scope": "fenêtre trop courte"}
        first, last = self.samples[0], self.samples[-1]
        duration = max(0.001, last["seconds"] - first["seconds"])
        return {"samples": self.samples, "seconds": duration,
                "cpu_percent_one_core": (last["cpu_seconds"] - first["cpu_seconds"]) / duration * 100,
                "rss_peak_sampled_bytes": max(s["rss_bytes"] for s in self.samples),
                "scope": "générateur et abonnés de contrôle seuls ; démarrage et vidage inclus ; worker exclu"}


def sources(export_root):
    candidates = [export_root] if export_root else sorted(ROOT.parent.glob("ha-offline-*"))
    candidates = [p for p in candidates if len(list(p.glob("*/sanitized/runtime/states.json"))) == 2]
    if len(candidates) != 1:
        raise ValueError("Indiquer un unique répertoire d’exports avec --exports.")
    inputs = list(candidates[0].glob("*/sanitized/runtime/states.json"))
    counts = []
    for path in inputs:
        if path.stat().st_size > 16 * 1024 * 1024:
            raise ValueError("Export trop volumineux.")
        data = json.loads(path.read_text())
        if not isinstance(data, list):
            raise ValueError("Format d’état non pris en charge.")
        counts.append((len(data), path.parents[2]))
    # Stable anonymised A/B ordering by shape, never by household name.
    return [path for _, path in sorted(counts, key=lambda row: -row[0])]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exports", type=Path)
    parser.add_argument("--profile", choices=("debug", "release"), default="release")
    parser.add_argument("--suite", choices=("all", "smoke"), default="all")
    args = parser.parse_args()
    exports = sources(args.exports)
    inputs_sha, latest_input_write = input_fingerprint(exports)
    env = dict(os.environ, RUSTUP_TOOLCHAIN="1.99.1", CARGO_BUILD_JOBS="2",
               CARGO_TARGET_DIR=str(ROOT / "target/mqtt-live-probe"))
    env.pop("MQTT_PASSWORD", None)
    env.pop("MOLI_ENUM_CATALOGUE_DIR", None)
    build = ["cargo", "build", "--offline", "--manifest-path",
             "tools/mqtt-live-probe/Cargo.toml", "--bin", "mqtt-live-probe", "-j2"]
    if args.profile == "release":
        build.append("--release")
    print("Compilation du banc isolé ; aucun service de maison n’est redémarré.", flush=True)
    subprocess.run(build, cwd=ROOT, env=env, check=True)
    binary = ROOT / "target/mqtt-live-probe" / args.profile / "mqtt-live-probe"
    cases = [("smoke", 1, 7, 8, "per-house", "memory")]
    if args.suite == "all":
        cases += [("two-shared-memory", 1, 25, 8, "shared", "memory"),
                  ("twenty-shared-memory", 10, 31, 8, "shared", "memory"),
                  ("twenty-partitioned-memory", 10, 31, 8, "per-house", "memory"),
                  ("twenty-partitioned-disk", 10, 31, 8, "per-house", "disk"),
                  ("twenty-burst-disk", 10, 31, 80, "per-house", "disk"),
                  ("twenty-burst-memory", 10, 31, 80, "per-house", "memory"),
                  ("twenty-soak-memory", 10, 120, 8, "per-house", "memory"),
                  ("twenty-accounting-memory", 10, 31, 8, "per-house", "memory")]
    output = ROOT / "docs/proposition/analyse/v6/charge-resultats.json"
    result = {"schema": 1, "when_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "profile": args.profile, "platform": sys.platform,
              "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
              "inputs_sha256": inputs_sha,
              "physical_commands": 0, "cases": []}
    if output.exists():
        previous = json.loads(output.read_text())
        # First campaign predates the fingerprint field. Its inputs are immutable
        # offline exports; retain it only when every input mtime predates that run.
        prior_time = datetime.datetime.fromisoformat(previous["when_utc"]).timestamp()
        unchanged_inputs = (previous.get("inputs_sha256") == inputs_sha or
                            ("inputs_sha256" not in previous and latest_input_write < prior_time))
        if (previous.get("binary_sha256") == result["binary_sha256"] and
                previous.get("profile") == args.profile and unchanged_inputs):
            result = previous
            if "inputs_sha256" not in result:
                result["input_fingerprint_scope"] = "empreinte ajoutée à la reprise ; tous les exports hors ligne précèdent la première campagne"
                result["inputs_sha256"] = inputs_sha
            result["resumed_at_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
            print("Même binaire : les scénarios déjà mesurés sont conservés, échecs compris.", flush=True)
    completed = {case["name"] for case in result["cases"] if case.get("exit_code") == 0}
    for name, copies, seconds, rate, layout, storage in cases:
        if name in completed:
            continue
        print(f"CHARGE : {name} — {copies * 2} maisons, {rate} messages/s/maison visés.", flush=True)
        command = [str(binary), "--synthetic-load", "--copies", str(copies), "--seconds", str(seconds),
                   "--rate", str(rate), "--layout", layout, "--storage", storage]
        for path in exports:
            command += ["--source", str(path)]
        report = None
        with subprocess.Popen(command, cwd=ROOT, env=env, stdout=subprocess.PIPE,
                              text=True, encoding="utf-8") as child:
            meter = GeneratorMeter(child.pid)
            for line in child.stdout:
                if line.startswith("LOAD_REPORT "):
                    report = json.loads(line[len("LOAD_REPORT "):])
                elif line.startswith(("LOAD_PHASE ", "LOAD_SAMPLE ")):
                    print(line.rstrip(), flush=True)
            code = child.wait()
            generator_resources = meter.finish()
        if report is None:
            result["cases"].append({"name": name, "exit_code": code, "status": "execution_failed"})
        else:
            report["name"] = name
            report["exit_code"] = code
            report["profile"] = args.profile
            report["generator_resources"] = generator_resources
            report["recorded_at_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
            result["cases"].append(report)
            print(f"RÉSULTAT : {name}, statut={report['outcome']}, workflows concordants={report['workflow_parity']}, "
                  f"CPU={report['resources']['cpu_percent_one_core']:.2f} % d’un cœur.", flush=True)
        output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
        if code or (name == "smoke" and (report is None or not report["healthy"])):
            raise RuntimeError("Le contrôle initial a échoué ; résultats agrégés conservés.")
    print("Campagne terminée : docs/proposition/analyse/v6/charge-resultats.json", flush=True)


if __name__ == "__main__":
    main()
