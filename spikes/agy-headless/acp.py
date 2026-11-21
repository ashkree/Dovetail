#!/usr/bin/env python3
"""Probe `agy acp` (Agent Client Protocol, JSON-RPC 2.0 over stdio).

Usage: acp.py <name> <scenario> [extra agy acp flags...]
Logs every line both ways to out/<name>.ndjson as {"t","dir","msg"} (the driver's fixture
format) and prints a short trace. Runs in /tmp/agy-spike/repo.

Scenarios:
  hello      initialize, session/new, one tiny prompt
  tool       a prompt that needs a shell command (permission request); answers allow-once
  cancel     a long prompt, session/cancel after the first chunk
  model      session/set_model (and set_mode, set_config_option) then a prompt
  mcp        session/new with Dovetail's MCP server (DOVETAIL_MCP_CMD) and a prompt to call it
  resume     session/load of the session id in $RESUME_ID, then a prompt
  slash      a prompt of "/quota" and "/usage" (print-mode slash commands)
"""

import json
import os
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
CWD = "/tmp/agy-spike/repo"
MODEL = os.environ.get("ACP_MODEL", "gemini-3.8-flash-low")

name, scenario, *extra = sys.argv[1:]
os.makedirs(os.path.join(HERE, "out"), exist_ok=True)
log = open(os.path.join(HERE, "out", f"{name}.ndjson"), "w")
start = time.time()
proc = subprocess.Popen(
    ["agy", *json.loads(os.environ.get("ACP_PRE", "[]")), "acp", *extra],
    cwd=CWD,
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=open(os.path.join(HERE, "out", f"{name}.err"), "w"),
    text=True,
    bufsize=1,
)

lock = threading.Lock()
responses = {}
cond = threading.Condition(lock)
next_id = [0]
first_chunk = threading.Event()


def record(direction, msg):
    log.write(json.dumps({"t": int((time.time() - start) * 1000), "dir": direction, "msg": msg}) + "\n")
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
                print(f"!! timeout waiting for {method}")
                return None
            cond.wait(left)
        return responses.pop(rid)


def notify(method, params):
    send({"jsonrpc": "2.0", "method": method, "params": params})


def short(v, n=300):
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
            print("<< (non-JSON)", line[:200])
            continue
        record("out", msg)
        if "method" in msg and "id" in msg:
            on_agent_request(msg)
        elif "method" in msg:
            upd = msg.get("params", {}).get("update", {})
            kind = upd.get("sessionUpdate", msg["method"])
            if kind == "agent_message_chunk":
                first_chunk.set()
            print("<< note", kind, short(upd, 240))
        else:
            print("<< resp", msg.get("id"), short(msg.get("result", msg.get("error")), 600))
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
    elif method == "fs/read_text_file":
        path = msg["params"]["path"]
        try:
            content = open(path).read()
            send({"jsonrpc": "2.0", "id": msg["id"], "result": {"content": content}})
        except OSError as e:
            send({"jsonrpc": "2.0", "id": msg["id"], "error": {"code": -32000, "message": str(e)}})
    else:
        send({"jsonrpc": "2.0", "id": msg["id"], "error": {"code": -32601, "message": "not supported by probe"}})


threading.Thread(target=reader, daemon=True).start()


def mcp_servers():
    cmd = os.environ.get("DOVETAIL_MCP_CMD")
    if not cmd:
        return []
    argv = cmd.split()
    env = [{"name": k, "value": v} for k, v in os.environ.items() if k.startswith("DOVETAIL_") and k != "DOVETAIL_MCP_CMD"]
    return [{"name": "dovetail", "command": argv[0], "args": argv[1:], "env": env}]


def prompt(sid, text, timeout=240):
    return request("session/prompt", {"sessionId": sid, "prompt": [{"type": "text", "text": text}]}, timeout)


init = request("initialize", {
    "protocolVersion": 1,
    "clientCapabilities": {"fs": {"readTextFile": False, "writeTextFile": False}, "terminal": False},
    "clientInfo": {"name": "dovetail-spike", "version": "0"},
})

def session_params():
    """session/new and resume params; ROLE_PROMPT and ADD_DIR exercise the role prompt and
    extra folders."""
    params = {"cwd": CWD, "mcpServers": mcp_servers()}
    if os.environ.get("ADD_DIR"):
        params["additionalDirectories"] = [os.environ["ADD_DIR"]]
    if os.environ.get("ROLE_PROMPT"):
        params["_meta"] = {"antigravity": {"systemPrompt": {"append": os.environ["ROLE_PROMPT"]}}}
    return params


if scenario == "resume":
    sid = os.environ["RESUME_ID"]
    method = os.environ.get("RESUME_METHOD", "session/load")
    print(">>", method, sid)
    request(method, {"sessionId": sid, **session_params()})
else:
    new = request("session/new", session_params())
    sid = (new or {}).get("result", {}).get("sessionId")
    print(">> session", sid)

if scenario in ("hello", "resume"):
    if scenario == "hello":
        request("session/set_model", {"sessionId": sid, "modelId": MODEL})
    prompt(sid, os.environ.get("PROMPT", "Reply with the single word: pong"))
elif scenario == "tool":
    request("session/set_model", {"sessionId": sid, "modelId": MODEL})
    prompt(sid, os.environ.get("PROMPT", "Run the shell command `ls -la` in the current directory and tell me how many entries it printed. Use your terminal tool."))
elif scenario == "cancel":
    request("session/set_model", {"sessionId": sid, "modelId": MODEL})
    t = threading.Thread(target=lambda: print(">> prompt done", short(prompt(sid, "Count slowly from 1 to 400, one number per line."))))
    t.start()
    first_chunk.wait(120)
    time.sleep(1)
    print(">> cancel")
    notify("session/cancel", {"sessionId": sid})
    t.join(120)
    prompt(sid, "Reply with the single word: after")
elif scenario == "model":
    for method, params in [
        ("session/set_model", {"sessionId": sid, "modelId": MODEL}),
        ("session/set_mode", {"sessionId": sid, "modeId": "plan"}),
        ("session/set_config_option", {"sessionId": sid, "configId": "effort", "value": "low"}),
    ]:
        print(">>", method)
        request(method, params)
    prompt(sid, "Which model are you? One short line.")
    request("session/set_model", {"sessionId": sid, "modelId": "gemini-3.8-flash-medium"})
    prompt(sid, "Which model are you now? One short line.")
elif scenario == "mcp":
    request("session/set_model", {"sessionId": sid, "modelId": MODEL})
    prompt(sid, os.environ.get("PROMPT", "List the MCP tools you have from the server named dovetail (names only), then call its report_status tool with status 'agy spike probe'."))
elif scenario == "slash":
    request("session/set_model", {"sessionId": sid, "modelId": MODEL})
    for cmd in ("/quota", "/usage", "/credits"):
        print(">> prompt", cmd)
        prompt(sid, cmd, 60)

time.sleep(1)
proc.stdin.close()
try:
    proc.wait(10)
except subprocess.TimeoutExpired:
    proc.kill()
print("exit", proc.returncode)
