#!/usr/bin/env python3
"""Capture privée du worker synthétique ; garde RSS échantillonnée, sans cgroup."""
import argparse
import concurrent.futures
import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import selectors
import signal
import subprocess
import sys
import tempfile
import time

BUDGET = 1_073_741_824
PERIOD = 0.5
CAPTURE_TIMEOUT = 90
WATCHDOG = 240


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def command_output(arguments, timeout=3):
    return subprocess.run(arguments, capture_output=True, text=True,
                          timeout=timeout, check=False)


def find_worker(supervisor_pid):
    """Inspect only the direct children returned for our own supervisor."""
    children = command_output(["/usr/bin/pgrep", "-P", str(supervisor_pid)])
    if children.returncode not in (0, 1):
        raise RuntimeError("worker_discovery_failed")
    found = []
    for value in children.stdout.split():
        pid = int(value)
        detail = command_output(["/bin/ps", "-ww", "-o", "ppid=,uid=,command=", "-p", str(pid)])
        fields = detail.stdout.strip().split(maxsplit=2)
        if (detail.returncode == 0 and len(fields) == 3
                and int(fields[0]) == supervisor_pid and int(fields[1]) == os.getuid()
                and "--synthetic-load-worker" in fields[2].split()):
            found.append(pid)
    if len(found) > 1:
        raise RuntimeError("multiple_workers")
    return found[0] if found else None


def worker_rss(pid, supervisor_pid):
    result = command_output(["/bin/ps", "-ww", "-o", "ppid=,uid=,rss=,command=", "-p", str(pid)])
    fields = result.stdout.strip().split(maxsplit=3)
    if result.returncode != 0 or not fields:
        return None
    if (len(fields) != 4 or int(fields[0]) != supervisor_pid
            or int(fields[1]) != os.getuid()
            or "--synthetic-load-worker" not in fields[3].split()):
        raise RuntimeError("worker_identity_changed")
    return int(fields[2]) * 1024


def terminate_group(process, include_orphans=False):
    """The process was launched with start_new_session=True: its group is ours."""
    if process is None or (process.poll() is not None and not include_orphans):
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    try:
        process.wait(timeout=3)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait(timeout=3)
    if include_orphans:
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            try:
                os.killpg(process.pid, 0)
            except ProcessLookupError:
                return
            time.sleep(0.05)
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def private_tool(arguments, stdout, stderr, timeout):
    """Keep raw output private and reap the tool, including on timeout."""
    started = time.monotonic()
    result = {"started_utc": now(), "tool": Path(arguments[0]).name,
              "stdout": stdout.name, "stderr": stderr.name}
    process = None
    try:
        with stdout.open("wb") as out, stderr.open("wb") as err:
            process = subprocess.Popen(arguments, stdout=out, stderr=err,
                                       start_new_session=True)
            try:
                result["exit_code"] = process.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                result["error"] = "timeout"
                terminate_group(process)
                result["exit_code"] = process.returncode
    except OSError as error:
        result["error"] = "tool_os_error"
        stderr.write_text(str(error) + "\n")
    finally:
        terminate_group(process)
    result.update(finished_utc=now(), seconds=time.monotonic() - started,
                  stdout_bytes=stdout.stat().st_size if stdout.exists() else 0,
                  stderr_bytes=stderr.stat().st_size if stderr.exists() else 0)
    return result


def capture(directory, label, worker_pid):
    graph = directory / f"{label}.memgraph"
    result = private_tool(
        ["/usr/bin/leaks", "--noContent", "--nosources",
         f"--outputGraph={graph}", str(worker_pid)],
        directory / f"{label}-leaks.stdout", directory / f"{label}-leaks.stderr",
        CAPTURE_TIMEOUT)
    size = graph.stat().st_size if graph.exists() else 0
    result.update(label=label, graph=graph.name, graph_bytes=size)
    result["success"] = (result.get("exit_code") in (0, 1)
                         and "error" not in result and size > 0
                         and result["stdout_bytes"] > 0)
    return result


def analyse(directory, capture_result):
    label = capture_result["label"]
    graph = directory / capture_result["graph"]
    tools = [
        ("heap", ["/usr/bin/heap", "-s", "--showSizes", "--noContent", str(graph)]),
        ("vmmap", ["/usr/bin/vmmap", "-summary", str(graph)]),
        ("malloc_history", ["/usr/bin/malloc_history", str(graph), "-callTree",
                            "-invert", "-ignoreThreads", "-consolidateAllBySymbol",
                            "-noContent"]),
    ]
    results = []
    for name, arguments in tools:
        result = private_tool(arguments, directory / f"{label}-{name}.stdout",
                              directory / f"{label}-{name}.stderr", 90)
        result["success"] = (result.get("exit_code") == 0
                             and result["stdout_bytes"] > 0 and "error" not in result)
        results.append(result)
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exports", type=Path)
    args = parser.parse_args()
    spec = importlib.util.spec_from_file_location("moli_load_campaign",
                                                 Path(__file__).with_name("run-load.py"))
    campaign = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(campaign)
    root = campaign.ROOT
    receipt_path = root / "docs/proposition/analyse/v6/heap-snapshot-recu.json"
    directory = Path(tempfile.mkdtemp(prefix="moli-load-snapshot-"))
    directory.chmod(0o700)
    # All subsequently created raw files are private too, not only their parent.
    os.umask(0o077)
    print(f"Captures privées : {directory}", flush=True)
    receipt = {
        "schema": 1, "started_utc": now(), "status": "initializing",
        "configuration": {"houses": 20, "copies_per_source": 10, "seconds": 120,
                          "rate_per_house": 8, "layout": "per-house", "storage": "memory"},
        "guard": {"budget_rss_bytes": BUDGET, "sample_period_seconds": PERIOD,
                  "scope": "worker_only", "kind": "sampled_rss_watchdog_not_cgroup",
                  "exceeded": False, "rss_peak_sampled_bytes": 0, "samples": 0},
        "artifacts": {"location": "private_temporary_directory",
                      "supervisor_stdout": "supervisor.stdout",
                      "supervisor_stderr": "supervisor.stderr"},
        "captures": [], "analysis": [], "physical_commands": 0,
        "interpretation": "raw_capture_only_no_memory_attribution_conclusion",
    }
    supervisor = None
    executor = concurrent.futures.ThreadPoolExecutor(max_workers=1)
    future = None
    captures_started = 0
    worker_pid = None
    measured_at = None
    latest_rss = None
    failure = None
    def interrupted(_signum, _frame):
        raise RuntimeError("controller_interrupted")
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    try:
        exports = campaign.sources(args.exports)
        binary = root / "target/mqtt-live-probe/release/mqtt-live-probe"
        receipt["binary_sha256"] = hashlib.sha256(binary.read_bytes()).hexdigest()
        env = dict(os.environ, MallocStackLogging="lite")
        env.pop("MQTT_PASSWORD", None)
        env.pop("MOLI_ENUM_CATALOGUE_DIR", None)
        env.pop("MallocStackLoggingNoCompact", None)
        env.pop("MallocStackLoggingDirectory", None)
        arguments = [str(binary), "--synthetic-load", "--copies", "10", "--seconds", "120",
                     "--rate", "8", "--layout", "per-house", "--storage", "memory"]
        for export in exports:
            arguments.extend(["--source", str(export)])
        started = time.monotonic()
        with (directory / "supervisor.stdout").open("wb") as raw_out, \
                (directory / "supervisor.stderr").open("wb") as raw_err:
            supervisor = subprocess.Popen(arguments, cwd=root, env=env, stdout=subprocess.PIPE,
                                          stderr=raw_err, start_new_session=True)
            selector = selectors.DefaultSelector()
            selector.register(supervisor.stdout, selectors.EVENT_READ)
            buffered = b""
            next_sample = started
            eof = False
            while not eof or supervisor.poll() is None:
                current = time.monotonic()
                if current - started >= WATCHDOG:
                    raise RuntimeError("supervisor_watchdog")
                if current >= next_sample:
                    next_sample = current + PERIOD
                    if worker_pid is None and supervisor.poll() is None:
                        worker_pid = find_worker(supervisor.pid)
                    if worker_pid is not None:
                        latest_rss = worker_rss(worker_pid, supervisor.pid)
                        if latest_rss is not None:
                            guard = receipt["guard"]
                            guard["samples"] += 1
                            guard["rss_peak_sampled_bytes"] = max(guard["rss_peak_sampled_bytes"], latest_rss)
                            if latest_rss > BUDGET:
                                guard.update(exceeded=True, exceeded_utc=now(), exceeded_rss_bytes=latest_rss)
                                raise RuntimeError("worker_rss_budget_exceeded")
                if future is not None and future.done():
                    receipt["captures"].append(future.result())
                    future = None
                if (measured_at is not None and worker_pid is not None and latest_rss is not None
                        and future is None and captures_started < 2
                        and (captures_started == 0 or current - measured_at >= 30)):
                    label = "start" if captures_started == 0 else "end"
                    future = executor.submit(capture, directory, label, worker_pid)
                    captures_started += 1
                    print(f"Capture {label} lancée ; RSS observé {latest_rss} octets.", flush=True)
                for key, _ in selector.select(timeout=max(0, min(PERIOD, next_sample - time.monotonic()))):
                    chunk = os.read(key.fileobj.fileno(), 65536)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        eof = True
                        continue
                    raw_out.write(chunk)
                    raw_out.flush()
                    buffered += chunk
                    if len(buffered) > 16 * 1024 * 1024:
                        raise RuntimeError("supervisor_line_bound_exceeded")
                    while b"\n" in buffered:
                        line, buffered = buffered.split(b"\n", 1)
                        if line.startswith(b"LOAD_PHASE ") and measured_at is None:
                            measured_at = time.monotonic()
                            receipt["measured_started_utc"] = now()
                            receipt["guard"]["rss_at_measured_start_bytes"] = latest_rss
                            print(f"Mesure démarrée ; RSS {latest_rss}, budget {BUDGET} octets.", flush=True)
                        elif line.startswith(b"LOAD_REPORT "):
                            report = json.loads(line[len(b"LOAD_REPORT "):])
                            receipt["load_result"] = {
                                key: report.get(key) for key in
                                ("workflow_parity", "workflow_expected_per_kind", "seconds", "profile")}
            selector.close()
            receipt["supervisor_exit_code"] = supervisor.wait(timeout=3)
            if supervisor.returncode != 0:
                raise RuntimeError("supervisor_failed")
            if measured_at is None:
                raise RuntimeError("no_measured_phase")
    except Exception as error:
        # Exception text can contain private source paths: keep it out of the receipt.
        (directory / "controller-error.txt").write_text(repr(error) + "\n")
        known = {"worker_rss_budget_exceeded", "supervisor_watchdog", "supervisor_failed",
                 "no_measured_phase", "worker_discovery_failed", "multiple_workers",
                 "worker_identity_changed", "supervisor_line_bound_exceeded", "controller_interrupted"}
        failure = str(error) if str(error) in known else type(error).__name__
    finally:
        terminate_group(supervisor, include_orphans=True)
        if supervisor is not None:
            receipt["supervisor_exit_code"] = supervisor.returncode
        if future is not None:
            receipt["captures"].append(future.result())
        executor.shutdown(wait=True)
        for captured in receipt["captures"]:
            if captured["success"]:
                receipt["analysis"].append({"label": captured["label"],
                                            "tools": analyse(directory, captured)})
        captures_ok = (len(receipt["captures"]) == 2
                       and all(item["success"] for item in receipt["captures"]))
        analyses_ok = (len(receipt["analysis"]) == 2
                       and all(tool["success"] for item in receipt["analysis"] for tool in item["tools"]))
        receipt["status"] = "captured" if failure is None and captures_ok and analyses_ok else "incomplete"
        if failure is not None:
            receipt["error"] = failure
        receipt["finished_utc"] = now()
        receipt_path.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n")
        print(f"Reçu public : {receipt_path.relative_to(root)} ; état {receipt['status']}.", flush=True)
    return 0 if receipt["status"] == "captured" else 1


if __name__ == "__main__":
    sys.exit(main())
