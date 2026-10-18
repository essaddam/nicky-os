#!/usr/bin/env python3
"""Crash/replay transport fixture; the client only accesses authenticated HTTP."""
import argparse, gzip, json
from load import Client, encoded

parser=argparse.ArgumentParser()
parser.add_argument("--url",required=True)
parser.add_argument("--phase",choices=("send","verify"),required=True)
parser.add_argument("--first",type=int)
args=parser.parse_args()
client=Client(args.url)
first=client.status()["durable_upto"]+1 if args.phase=="send" else args.first
assert first and first>0
last=first+1799
body=b"".join(encoded({"type":"sample","seq":first+i,"point":"fixture.interrupted","ts":i,"kind":"t","num":None,"txt":"x"*4096})+b"\n" for i in range(1800))
assert len(body)<=8*1024*1024
if args.phase=="send":
    ack,_=client.request("POST",f"batches/{first}-{last}",gzip.compress(body,compresslevel=1,mtime=0),True)
    assert ack["durable_upto"]==last and ack["published_upto"]<last,ack
    print(json.dumps({"phase":"durable_before_kill","first":first,"last":last,"ack":ack}),flush=True)
else:
    client.wait(last)
    for start in range(0,1800,100):
        reply,_=client.query("samples",["fixture.interrupted"],start,start+100,"raw",100)
        rows=reply["series"][0]["values"]
        assert len(rows)==100
        assert [r["seq"] for r in rows]==list(range(first+start,first+start+100))
        assert all(r["txt"]=="x"*4096 for r in rows)
    # Same decompressed NDJSON, deliberately different gzip header.
    ack,_=client.request("POST",f"batches/{first}-{last}",gzip.compress(body,compresslevel=1,mtime=1),True)
    assert ack["published_upto"]>=last
    print(json.dumps({"phase":"crash_replay_verified","rows":1800,"first":first,"last":last}),flush=True)
