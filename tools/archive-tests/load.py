#!/usr/bin/env python3
"""Synthetic domestic load only; never run against an inhabited installation."""
import argparse, gzip, http.client, json, math, statistics, sys, time
from urllib.parse import urlsplit

TOKEN = "fixture-archive-token-for-tests"
INSTALL = "00000000000000000000000000000002"
HOUR = 3_600_000
DAY = 24 * HOUR
BASE = 1_767_225_600_000  # 2026-01-01 UTC, synthetic calendar.


def encoded(value):
    return json.dumps(value, separators=(",", ":"), allow_nan=False).encode()


class Client:
    def __init__(self, url):
        target = urlsplit(url)
        assert target.scheme == "http" and target.hostname.startswith("moli-archive-c1-"), "isolated fixture only"
        self.host, self.port = target.hostname, target.port
        self.calls = 0
        self.ack_ms = []
        self.publish_ms = []

    def request(self, method, path, body=b"", compression=False):
        started = time.monotonic()
        while True:
            conn = http.client.HTTPConnection(self.host, self.port, timeout=5)
            try:
                conn.connect()
                conn.sock.settimeout(30)
                headers = {"Authorization": "Bearer " + TOKEN, "Content-Type": "application/json"}
                if compression:
                    headers["Content-Encoding"] = "gzip"
                begin = time.perf_counter()
                conn.request(method, f"/v1/installations/{INSTALL}/{path}", body, headers)
                reply = conn.getresponse()
                raw = reply.read(1_048_577)
                elapsed = (time.perf_counter() - begin) * 1000
                self.calls += 1
                assert len(raw) <= 1_048_576
                value = json.loads(raw)
                if reply.status == 503 and time.monotonic() - started < 30:
                    time.sleep(0.05)
                    continue
                assert reply.status == 200, (reply.status, value)
                return value, elapsed
            except (OSError, http.client.HTTPException):
                if time.monotonic() - started >= 30:
                    raise
                time.sleep(0.1)
            finally:
                conn.close()

    def status(self):
        return self.request("GET", "status")[0]

    def wait(self, last):
        started = time.monotonic()
        while self.status()["published_upto"] < last:
            assert time.monotonic() - started < 120, "publication did not complete"
            time.sleep(0.02)
        self.publish_ms.append((time.monotonic() - started) * 1000)

    def send(self, operations):
        first, last = operations[0]["seq"], operations[-1]["seq"]
        body = b"".join(encoded(op) + b"\n" for op in operations)
        assert len(body) <= 8 * 1024 * 1024
        packed = gzip.compress(body, compresslevel=1, mtime=0)
        assert len(packed) <= 1024 * 1024
        ack, elapsed = self.request("POST", f"batches/{first}-{last}", packed, True)
        assert ack["durable_upto"] >= last
        self.ack_ms.append(elapsed)
        self.wait(last)

    def query(self, source, points, start, end, resolution, maximum=5000):
        body = encoded({"source": source, "points": points, "from": start, "to": end,
                        "resolution": resolution, "max_points": maximum})
        return self.request("POST", "query", body)


def shape_catalog(points, meters):
    entries = [{"id": f"fixture.point.{i:04}", "family": "power", "unit": "W",
                "aggregation": "time_weighted", "freshness_ms": None, "origin": "recorded"}
               for i in range(points)]
    entries.append({"id": "fixture.stable", "family": "power", "unit": "W",
                    "aggregation": "time_weighted", "freshness_ms": None, "origin": "recorded"})
    return {"timezone": "UTC", "points": entries,
            "meters": [{"id": f"fixture.meter.{i:03}", "origin": "recorded"} for i in range(meters)]}


def operations(points, samples, meters, energy_hours, days):
    seq = 0
    for point in range(points):
        count = samples // points + (point < samples % points)
        for index in range(count):
            seq += 1
            yield {"type": "sample", "seq": seq, "point": f"fixture.point.{point:04}",
                   "ts": BASE + index * days * DAY // count, "kind": "f",
                   "num": float(100 + index % 100), "txt": None}
    for meter in range(meters):
        count = energy_hours // meters + (meter < energy_hours % meters)
        for hour in range(count):
            seq += 1
            kwh = 0.02 + (meter % 11) * 0.003 + (hour % 24) / 10000
            cost = None if hour % 17 == 0 else kwh * (0.16 if hour % 24 < 8 else 0.25)
            yield {"type": "energy_hour", "seq": seq, "meter": f"fixture.meter.{meter:03}",
                   "hour": BASE + hour * HOUR, "kwh": kwh, "cost": cost}


def quantiles(values):
    ordered = sorted(values)
    def at(p):
        return ordered[min(len(ordered) - 1, max(0, math.ceil(len(ordered) * p) - 1))]
    return {"count": len(ordered), "p50_ms": at(.5), "p95_ms": at(.95),
            "max_ms": ordered[-1]} if ordered else {"count": 0}


def expected_first_hour(samples, points, days):
    count = samples // points + (samples % points > 0)
    total = 0.0
    for index in range(count):
        a = index * days * DAY // count
        b = (index + 1) * days * DAY // count
        held = max(0, min(HOUR, b) - a)
        total += (100 + index % 100) * held
    return total / HOUR


def bench(client, name, action, repetitions=20):
    times = []
    reply = None
    for _ in range(repetitions):
        reply, elapsed = action()
        times.append(elapsed)
    result = {"phase": "query", "name": name, **quantiles(times)}
    print(json.dumps(result), flush=True)
    return reply, result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--url", required=True)
    parser.add_argument("--points", type=int, default=3000)
    parser.add_argument("--samples", type=int, default=2_000_000)
    parser.add_argument("--meters", type=int, default=128)
    parser.add_argument("--energy-hours", type=int, default=128_000)
    parser.add_argument("--days", type=int, default=7)
    parser.add_argument("--stress-only", action="store_true")
    args = parser.parse_args()
    assert 1 <= args.points <= 4095 and args.samples >= args.points and args.meters > 0
    client = Client(args.url)
    initial = client.status()
    catalog = shape_catalog(args.points, args.meters)
    current, _ = client.request("GET", "catalog")
    if current["catalog"] != catalog:
        client.request("PUT", "catalog", encoded({"expected_revision": current["revision"], "catalog": catalog}))
    started = time.monotonic()
    if not args.stress_only:
        durable = client.status()["durable_upto"]
        batch = []
        for op in operations(args.points, args.samples, args.meters, args.energy_hours, args.days):
            if op["seq"] <= durable:
                continue
            batch.append(op)
            if len(batch) == 5000:
                client.send(batch)
                if batch[-1]["seq"] % 100_000 == 0:
                    print(json.dumps({"phase": "ingest", "published": batch[-1]["seq"],
                                      "seconds": time.monotonic() - started}), flush=True)
                batch = []
        if batch:
            client.send(batch)
        final = client.status()
        assert final["published_upto"] >= args.samples + args.energy_hours
        print(json.dumps({"phase": "fixture_complete", "samples": args.samples, "energy_hours": args.energy_hours,
                          "points": args.points, "seconds": time.monotonic() - started,
                          "ack": quantiles(client.ack_ms), "publication": quantiles(client.publish_ms)}), flush=True)
    point = "fixture.point.0000"
    reply, _ = bench(client, "plug_seven_days_hourly", lambda: client.query("samples", [point], BASE, BASE + args.days * DAY, "hour", 200))
    assert abs(reply["series"][0]["values"][0]["stat"]["aggregate"]["mean"] - expected_first_hour(args.samples, args.points, args.days)) < 1e-8
    bench(client, "dashboard_twenty_daily_curves", lambda: client.query("samples", [f"fixture.point.{i:04}" for i in range(min(20, args.points))], BASE, BASE + args.days * DAY, "day", 200))
    meter = "fixture.meter.000"
    reply, _ = bench(client, "energy_two_months", lambda: client.query("energy", [meter], BASE, BASE + 59 * DAY, "month", 200))
    hours = args.energy_hours // args.meters + (args.energy_hours % args.meters > 0)
    expected = sum(0.02 + (hour % 24) / 10000 for hour in range(hours))
    observed = sum(row["stat"]["aggregate"]["total"] for row in reply["series"][0]["values"])
    assert abs(expected - observed) < 1e-7, (expected, observed)
    bench(client, "energy_house_daily", lambda: client.query("energy", [f"fixture.meter.{i:03}" for i in range(args.meters)], BASE, BASE + 7 * DAY, "day", 5000))
    if args.stress_only:
        seq = client.status()["durable_upto"]
        for stamp, value in [(1_451_606_400_000, 10.0), (BASE, 20.0)]:
            seq += 1
            client.send([{"type": "sample", "seq": seq, "point": "fixture.stable", "ts": stamp,
                          "kind": "f", "num": value, "txt": None}])
        text = []
        for index in range(1800):
            seq += 1
            text.append({"type": "sample", "seq": seq, "point": "fixture.text", "ts": BASE + index,
                         "kind": "t", "num": None, "txt": "x" * 4096})
        client.send(text)
        reply, _ = client.query("samples", ["fixture.stable"], 1_483_228_800_000, 1_483_232_400_000, "hour", 200)
        assert reply["series"][0]["values"][0]["stat"]["aggregate"]["mean"] == 10.0
        print(json.dumps({"phase": "stress_complete", "last_sequence": seq}), flush=True)
    assert client.status()["generation"] == initial["generation"]
    print(json.dumps({"phase": "complete", "status": client.status(), "http_calls": client.calls}), flush=True)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(json.dumps({"phase": "failed", "error": str(error)}), flush=True)
        raise
