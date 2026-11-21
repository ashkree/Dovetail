#!/usr/bin/env python3
"""Ask an idle headless Claude Code for its usage over control requests; no prompt is sent,
so no model call is made. Usage: claude_usage.py <name> [subtype...]
Logs both directions to out/<name>.ndjson in the fixture format."""

import json
import os
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
name, *subtypes = sys.argv[1:]
subtypes = subtypes or ["get_usage", "get_context_usage", "get_session_cost", "get_status"]
log = open(os.path.join(HERE, "out", f"{name}.ndjson"), "w")
start = time.time()
proc = subprocess.Popen(
    ["claude", "-p", "--input-format", "stream-json", "--output-format", "stream-json",
     "--verbose", "--no-session-persistence"],
    cwd="/tmp/agy-spike/repo", stdin=subprocess.PIPE, stdout=subprocess.PIPE,
    stderr=subprocess.DEVNULL, text=True, bufsize=1,
)
answered = {}
cond = threading.Condition()


def record(direction, msg):
    log.write(json.dumps({"t": int((time.time() - start) * 1000), "dir": direction, "msg": msg}) + "\n")
    log.flush()


def reader():
    for line in proc.stdout:
        try:
            msg = json.loads(line)
        except ValueError:
            continue
        record("out", msg)
        if msg.get("type") == "control_response":
            rid = msg["response"].get("request_id")
            with cond:
                answered[rid] = msg
                cond.notify_all()


threading.Thread(target=reader, daemon=True).start()
for i, subtype in enumerate(["initialize", *subtypes]):
    rid = f"probe_{i}"
    msg = {"type": "control_request", "request_id": rid, "request": {"subtype": subtype}}
    record("in", msg)
    proc.stdin.write(json.dumps(msg) + "\n")
    proc.stdin.flush()
    with cond:
        cond.wait_for(lambda: rid in answered, timeout=30)
    reply = answered.get(rid, {}).get("response", {})
    body = json.dumps(reply.get("response", reply.get("error")))
    print(f"== {subtype}: {reply.get('subtype')} {body[:1500] if subtype != 'initialize' else '(…)'}")
proc.stdin.close()
proc.wait(15)
