#!/usr/bin/env python3
"""Exercise module topologies and the minimal master without real devices."""
import hashlib
import json
import os
from pathlib import Path
import re
from datetime import datetime, timezone
import signal
import shutil
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
BASE = ROOT / "tools/module-host-poc"
OUTPUT = ROOT / "docs/proposition/analyse/v6/modules-resultats.json"
ENV = dict(os.environ, RUSTUP_TOOLCHAIN="1.99.1", CARGO_BUILD_JOBS="2",
           CARGO_TARGET_DIR=str(ROOT / "target/module-host-poc"))
for name in ("MQTT_PASSWORD", "MOLI_MASTER_KEY", "MOLI_MASTER_KEY_FILE"):
    ENV.pop(name, None)


def get(port, path="/status", method="GET", data=None):
    request = urllib.request.Request(f"http://127.0.0.1:{port}{path}", data=data, method=method)
    try:
        with urllib.request.urlopen(request, timeout=2) as response:
            return response.status, json.load(response)
    except urllib.error.HTTPError as error:
        return error.code, json.load(error)


def wait_for(port, predicate, timeout=6):
    started = time.monotonic()
    latest = None
    while time.monotonic() - started < timeout:
        _, latest = get(port)
        if predicate(latest):
            return latest
        time.sleep(.1)
    raise AssertionError(f"Statut attendu absent : {latest}")


def no_process(pid):
    result = subprocess.run(["ps", "-o", "pid=", "-p", str(pid)], capture_output=True)
    return result.returncode != 0 or not result.stdout.strip()


def master_case(binary, config, kill_group=None):
    process = subprocess.Popen([str(binary), "--config", str(config), "--seconds", "12"],
                               cwd=ROOT, env=ENV, stdout=subprocess.PIPE, text=True)
    try:
        line = process.stdout.readline()
        assert line.startswith("MASTER_LISTENING "), line
        listening = json.loads(line[len("MASTER_LISTENING "):])
        port = listening["port"]
        code, first = get(port, "/health")
        assert code == 200 and first["alive"]
        status = wait_for(port, lambda s: s["ready"])
        assert status["startup_order"][0] == "http_bound"
        assert get(port, "/status", "POST", b"x")[0] == 405
        assert get(port, "/missing")[0] == 404
        assert get(port, "/status", "GET", b"x")[0] == 413
        time.sleep(1.1)
        status = get(port)[1]
        owned = [g["pid"] for g in status["groups"] if g["pid"]]
        before = status
        if kill_group:
            victim = next(g for g in status["groups"] if g["id"] == kill_group)
            parent = subprocess.check_output(["ps", "-o", "ppid=", "-p", str(victim["pid"])], text=True).strip()
            assert int(parent) == process.pid
            os.kill(victim["pid"], signal.SIGTERM)
            status = wait_for(port, lambda s: any(g["id"] == kill_group and g["phase"] == "failed" for g in s["groups"]))
            assert any(g["id"] != kill_group and g["phase"] == "running" for g in status["groups"])
            assert get(port, "/health")[1]["alive"]
        started = time.monotonic()
        process.terminate()
        rest, _ = process.communicate(timeout=4)
        stop_seconds = time.monotonic() - started
        assert process.returncode == 0
        last = next(line for line in rest.splitlines() if line.startswith("MASTER_STOPPED "))
        final = json.loads(last[len("MASTER_STOPPED "):])
        assert not final["ready"] and final["phase"] == "stopped"
        assert all(no_process(pid) for pid in owned), "Un enfant est resté actif"
        for snapshot in (before, status, final):
            for group in snapshot["groups"]:
                group.pop("pid", None)
                group.pop("broker_port", None)
        return {"status": "PASS", "first_health": first, "before": before,
                "after_fault": status if kill_group else None, "final": final,
                "stop_seconds": stop_seconds, "owned_children_gone": True}
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=4)


def invalid_master_case(binary):
    with tempfile.TemporaryDirectory(prefix="moli-invalid-layout-") as scratch:
        process = subprocess.Popen([str(binary), "--config", str(Path(scratch)/"absent.toml"), "--seconds", "5"],
                                   cwd=ROOT, env=ENV, stdout=subprocess.PIPE, text=True)
        try:
            listening = json.loads(process.stdout.readline().split(" ", 1)[1])
            status = wait_for(listening["port"], lambda s: s["phase"] == "error")
            assert not status["ready"] and get(listening["port"], "/health")[1]["alive"]
            process.terminate(); process.communicate(timeout=4)
            assert process.returncode == 0
            return {"status":"PASS", "health_survives_invalid_configuration":True}
        finally:
            if process.poll() is None:
                process.terminate(); process.wait(timeout=4)


def adversarial_master_case(binary, kind, cancel_early=False):
    """Only a copied test master and its own deliberately faulty fixture host."""
    with tempfile.TemporaryDirectory(prefix="moli-master-fault-") as scratch:
        copied=Path(scratch)/"moli-master-poc"
        shutil.copy2(binary,copied)
        host=Path(scratch)/"moli-module-host-poc"
        source='''#!/usr/bin/env python3
import json,os,sys,time
if "--plan" in sys.argv:
    if KIND == "closed-stdout-planner":
        os.close(1)
        time.sleep(30)
    else:
        print(json.dumps({"mode":"grouped","groups":[{"id":"demo","modules":["fixture"]}]}),flush=True)
elif "--broker" in sys.argv:
    while True:
        print("HOST_HEARTBEAT {}",flush=True)
        time.sleep(.05)
'''.replace('import json,os,sys,time','import json,os,sys,time\nKIND='+repr(kind))
        host.write_text(source);host.chmod(0o700)
        process=subprocess.Popen([str(copied),"--config","fixture","--seconds","12"],env=ENV,stdout=subprocess.PIPE,text=True)
        try:
            port=json.loads(process.stdout.readline().split(" ",1)[1])["port"]
            assert get(port,"/health")[1]["alive"]
            time.sleep(.2)
            child_pids=subprocess.run(["pgrep","-P",str(process.pid)],capture_output=True,text=True).stdout.split()
            if cancel_early:
                status=get(port)[1]
            else:
                status=wait_for(port,lambda s:s["phase"]=="error",timeout=7)
                assert get(port,"/health")[1]["alive"]
            started=time.monotonic();process.terminate();rest,_=process.communicate(timeout=3)
            assert process.returncode==0
            stop=json.loads(next(line.split(" ",1)[1] for line in rest.splitlines() if line.startswith("MASTER_STOPPED ")))
            assert not stop["ready"] and stop["phase"]=="stopped"
            assert all(no_process(int(pid)) for pid in child_pids)
            return {"status":"PASS","fixture":kind,"cancel_during_startup":cancel_early,
                    "health_stays_available":True,"owned_children_gone":True,
                    "stop_seconds":time.monotonic()-started,"error_reported":status["error"]}
        finally:
            if process.poll() is None:
                process.terminate();process.wait(timeout=3)


def main():
    checks = {}
    for name, arguments in [
        ("fmt", ["fmt", "--manifest-path", "tools/module-host-poc/Cargo.toml", "--", "--check"]),
        ("clippy", ["clippy", "--offline", "--manifest-path", "tools/module-host-poc/Cargo.toml", "--all-targets", "-j2", "--", "-D", "warnings"]),
        ("module_tests", ["test", "--offline", "--manifest-path", "tools/module-host-poc/Cargo.toml", "-j2"]),
        ("native_model_tests", ["test", "--offline", "--manifest-path", "Cargo.toml", "-p", "moli-ariston", "-p", "moli-overkiz", "--lib", "-j2"]),
    ]:
        print(f"Contrôle {name}", flush=True)
        checked = subprocess.run(["cargo", *arguments], cwd=ROOT, env=ENV, capture_output=True, text=True)
        assert checked.returncode == 0, checked.stdout + checked.stderr
        checks[name] = {"status":"PASS", "command":["cargo", *arguments],
                        "tests_passed":sum(map(int,re.findall(r"test result: ok\. (\d+) passed", checked.stdout)))}
    subprocess.run(["cargo", "build", "--offline", "--release", "--bins", "--manifest-path",
                    "tools/module-host-poc/Cargo.toml", "-j2"], cwd=ROOT, env=ENV, check=True)
    binaries = ROOT / "target/module-host-poc/release"
    host, master = binaries/"moli-module-host-poc", binaries/"moli-master-poc"
    result = {"schema":1, "profile":"release", "physical_commands":0, "topologies":[], "master_tests":{},
              "binaries":{name:{"bytes":path.stat().st_size,"sha256":hashlib.sha256(path.read_bytes()).hexdigest()}
                          for name,path in [("host",host),("master",master)]}}
    if OUTPUT.exists():
        previous=json.loads(OUTPUT.read_text())
        if previous.get("binaries")==result["binaries"]:
            result=previous
    result["checks"] = checks
    def save():
        OUTPUT.write_text(json.dumps(result, ensure_ascii=False, indent=2)+"\n")
    digest = None
    for name in ("merged", "grouped", "isolated", "disabled", "with-music"):
        existing=next((r for r in result["topologies"] if r.get("case")==name),None)
        if existing:
            if name not in ("disabled", "with-music"):digest=existing["initial_semantic_digest"]
            continue
        print(f"Topologie {name} : adaptateurs d’essai, aucun appareil physique.", flush=True)
        child = subprocess.run([str(host), "--config", str(BASE/f"examples/{name}.toml")],
                               cwd=ROOT, env=ENV, capture_output=True, text=True, timeout=30)
        assert child.returncode == 0, child.stderr
        report = json.loads(next(line[len("MODULE_REPORT "):] for line in child.stdout.splitlines()
                                 if line.startswith("MODULE_REPORT ")))
        if name == "disabled":
            assert report["active_modules"] == 0 and not report["broker_started"]
        else:
            assert report["active_modules"] == (5 if name == "with-music" else 4)
            assert report["simulated_commands"] == report["expected_test_commands"] == (4 if name == "with-music" else 3)
            assert report["all_modules_offline_after_shutdown"]
            assert not report["failed_groups"]
            if name == "with-music":
                music = next(m for g in report["groups"] for m in g["modules"] if m["kind"] == "music")
                assert music["adapter"]["rendered_frames"] > 0
                assert music["adapter"]["samples"] == 2 * music["adapter"]["rendered_frames"]
                assert music["adapter"]["physical_outputs"] == 0 and not music["adapter"]["pcm_on_mqtt"]
            elif digest is None:
                digest = report["initial_semantic_digest"]
            if name != "with-music":
                assert report["initial_semantic_digest"] == digest, "Identités/valeurs modifiées par regroupement"
        report["status"]="PASS"
        report["case"]=name
        result["topologies"].append(report);save()
    for name, mode, victim in [("merged", "merged", None), ("grouped", "grouped", None),
                               ("isolated-fault", "isolated", "apple-tv"), ("disabled", "disabled", None),
                               ("with-music", "with-music", None)]:
        if name in result["master_tests"]:continue
        print(f"Master {name} : HTTP en premier, surface read-only, arrêt et nettoyage.", flush=True)
        result["master_tests"][name]=master_case(master, BASE/f"examples/{mode}.toml", victim);save()
    result["master_tests"]["invalid-config"]=invalid_master_case(master);save()
    for kind,early in [("closed-stdout-planner",False),("heartbeat-without-ready",False),("closed-stdout-planner",True)]:
        key=kind+("-cancel" if early else "")
        if key in result["master_tests"]:continue
        print(f"Master défaillant simulé : {key}",flush=True)
        result["master_tests"][key]=adversarial_master_case(master,kind,early);save()
    symbols = subprocess.check_output(["nm", str(master)], text=True)
    forbidden = [name for name in ("moli_ariston", "moli_overkiz", "rumqttc", "apple_tv", "cpal", "symphonia") if name in symbols]
    assert not forbidden, forbidden
    result["master_business_symbols_absent"]=True
    result["semantic_parity"]=True
    result["validated_at"] = datetime.now(timezone.utc).isoformat()
    save()
    print("Validation terminée : modules-resultats.json", flush=True)


if __name__ == "__main__":
    main()
