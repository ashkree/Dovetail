#!/usr/bin/env python3
"""Probe `agy acp` for subagent lifecycle events (invoke_subagent).

Usage:
    python3 subagents.py [name] [scenario]

Scenarios:
    wait (default): instructs agy to invoke a subagent and wait for its completion.
    nowait:         instructs agy to invoke a subagent, reply immediately, and then
                    subsequently check status via manage_subagents.

Logs raw ACP JSON-RPC messages to spikes/agy-headless/out/<name>.ndjson.
"""

import json
import os
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
CWD = "/tmp/agy-subagent-spike"
MODEL = os.environ.get("ACP_MODEL", "gemini-3.8-flash-medium")
NAME = sys.argv[1] if len(sys.argv) > 1 else "subagent-live"
SCENARIO = sys.argv[2] if len(sys.argv) > 2 else "wait"

os.makedirs(os.path.join(HERE, "out"), exist_ok=True)
os.makedirs(CWD, exist_ok=True)

test_file = os.path.join(CWD, "lines.txt")
if not os.path.exists(test_file):
    with open(test_file, "w") as f:
        f.write("apple\nbanana\ncherry\n")

log_path = os.path.join(HERE, "out", f"{NAME}.ndjson")
err_path = os.path.join(HERE, "out", f"{NAME}.err")

log = open(log_path, "w")
err = open(err_path, "w")

start = time.time()
proc = subprocess.Popen(
    ["agy", *json.loads(os.environ.get("ACP_PRE", "[]")), "acp"],
    cwd=CWD,
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=err,
    text=True,
    bufsize=1,
)

lock = threading.Lock()
responses = {}
cond = threading.Condition(lock)
next_id = [0]
last_event_time = [time.time()]


def record(direction, msg):
    now = time.time()
    last_event_time[0] = now
    t_ms = int((now - start) * 1000)
    log.write(json.dumps({"t": t_ms, "dir": direction, "msg": msg}) + "\n")
    log.flush()


def send(msg):
    record("in", msg)
    proc.stdin.write(json.dumps(msg) + "\n")
    proc.stdin.flush()


def request(method, params, timeout=180):
    with lock:
        next_id[0] += 1
        rid = next_id[0]
    send({"jsonrpc": "2.0", "id": rid, "method": method, "params": params})
    deadline = time.time() + timeout
    with cond:
        while rid not in responses:
            left = deadline - time.time()
            if left <= 0:
                print(f"!! timeout waiting for {method} (id={rid})")
                return None
            cond.wait(left)
        return responses.pop(rid)


def short(v, n=160):
    s = json.dumps(v)
    return s if len(s) <= n else s[:n] + "…"


def reader():
    for line in proc.stdout:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except ValueError:
            record("out", {"raw": line})
            continue
        record("out", msg)
        if "method" in msg and "id" in msg:
            on_agent_request(msg)
        elif "method" in msg:
            upd = msg.get("params", {}).get("update", {})
            kind = upd.get("sessionUpdate", msg["method"])
            if kind == "tool_call":
                print(f"<< [TOOL_CALL] {upd.get('name')} id={upd.get('toolCallId')} title={upd.get('title')}")
                print(f"    rawInput: {short(upd.get('rawInput', {}), 300)}")
            elif kind == "tool_call_update":
                print(f"<< [TOOL_CALL_UPDATE] id={upd.get('toolCallId')} status={upd.get('status')}")
                if "rawOutput" in upd:
                    print(f"    rawOutput: {short(upd.get('rawOutput', {}), 300)}")
            else:
                print(f"<< [UPDATE] {kind}: {short(upd, 200)}")
        else:
            print(f"<< [RESP] id={msg.get('id')}: {short(msg.get('result', msg.get('error')), 300)}")
            with cond:
                responses[msg.get("id")] = msg
                cond.notify_all()


def on_agent_request(msg):
    method = msg["method"]
    print("<< REQ", method, short(msg.get("params"), 600))
    if method == "session/request_permission":
        options = msg["params"].get("options", [])
        allow = next((o for o in options if o.get("kind") == "allow_once"), options[0] if options else None)
        result = {"outcome": {"outcome": "selected", "optionId": allow["optionId"]}} if allow else {"outcome": {"outcome": "cancelled"}}
        send({"jsonrpc": "2.0", "id": msg["id"], "result": result})
    else:
        send({"jsonrpc": "2.0", "id": msg["id"], "error": {"code": -32601, "message": "not supported"}})


reader_thread = threading.Thread(target=reader, daemon=True)
reader_thread.start()

print("1. Initializing...")
request("initialize", {
    "protocolVersion": 1,
    "clientCapabilities": {"fs": {"readTextFile": False, "writeTextFile": False}, "terminal": False},
    "clientInfo": {"name": "dovetail-subagent-spike", "version": "0"},
})

print("2. Starting session...")
new_session = request("session/new", {
    "cwd": CWD,
    "mcpServers": [],
})
sid = (new_session or {}).get("result", {}).get("sessionId")
print(f"Session started: {sid}")

if MODEL:
    print(f"Setting model: {MODEL}")
    request("session/set_model", {"sessionId": sid, "modelId": MODEL})

if SCENARIO == "wait":
    prompt_text = (
        "You MUST use the invoke_subagent tool to count the lines in lines.txt. "
        "Do NOT read the file yourself or run terminal commands yourself. "
        "Call invoke_subagent with TypeName: 'research', Role: 'Line Counter', and "
        "Prompt: 'Read lines.txt in the current directory and count the lines. Reply with just the count.' "
        "Wait for the subagent to finish."
    )
    print(f"3. Sending prompt:\n{prompt_text}\n")
    prompt_res = request("session/prompt", {
        "sessionId": sid,
        "prompt": [{"type": "text", "text": prompt_text}],
    }, timeout=300)
    print(f"Prompt result: {short(prompt_res, 300)}")

    print("4. Waiting for background activity...")
    wait_until = time.time() + 60
    while time.time() < wait_until:
        time.sleep(2)
        if (time.time() - last_event_time[0]) > 15 and (time.time() - start) > 30:
            break
else:
    prompt_text = (
        "Launch a subagent using invoke_subagent with TypeName: 'research', Role: 'Line Counter', and "
        "Prompt: 'Read lines.txt and count lines. Reply with just the count.' "
        "Do NOT wait for the subagent. Finish your reply immediately with the single word: LAUNCHED."
    )
    print(f"3. Sending prompt (nowait):\n{prompt_text}\n")
    p1 = request("session/prompt", {
        "sessionId": sid,
        "prompt": [{"type": "text", "text": prompt_text}],
    }, timeout=120)
    print(f"Turn 1 result: {short(p1, 300)}")
    time.sleep(20)
    p2 = request("session/prompt", {
        "sessionId": sid,
        "prompt": [{"type": "text", "text": "What is the status of the subagent?"}],
    }, timeout=120)
    print(f"Turn 2 result: {short(p2, 300)}")

print("Done. Closing session...")
try:
    proc.stdin.close()
    proc.wait(5)
except Exception:
    proc.kill()

print(f"Finished. Exit code: {proc.returncode}")
print(f"Log: {log_path}")
