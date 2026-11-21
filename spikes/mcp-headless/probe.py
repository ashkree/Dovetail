#!/usr/bin/env python3
"""Probe MCP control requests on a headless session started like the driver starts one.

Usage: mkdir -p /tmp/dt-mcp-spike && git -C /tmp/dt-mcp-spike init
       python3 -I spikes/mcp-headless/probe.py
Only localhost is used. One tiny model call is made, for `/mcp` sent as a user message.
"""
import json, os, subprocess, sys, threading, time, uuid

HERE = os.path.dirname(os.path.abspath(__file__))
SERVER = os.path.join(HERE, "..", "mcp", "server.py")
PORT = 18765
cfg = "/tmp/dt-mcp-spike-mcp.json"
json.dump({"mcpServers": {
    "good": {"command": "python3", "args": [SERVER], "env": {"SPIKE_LOG": "/tmp/dt-mcp-spike-server.log"}},
    "broken": {"command": "/nonexistent/binary"},
    "needsauth": {"type": "http", "url": f"http://127.0.0.1:{PORT}/mcp"},
}}, open(cfg, "w"))
auth = subprocess.Popen([sys.executable, "-I", os.path.join(HERE, "auth_server.py"), str(PORT)])
time.sleep(0.5)
p = subprocess.Popen(
    ["claude", "-p", "--input-format", "stream-json", "--output-format", "stream-json",
     "--verbose", "--include-partial-messages", "--permission-prompt-tool", "stdio",
     "--replay-user-messages", "--permission-mode", "default", "--session-id", str(uuid.uuid4()),
     "--model", "haiku", "--mcp-config", cfg],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, cwd="/tmp/dt-mcp-spike")
lines = []


def rd():
    for l in p.stdout:
        lines.append(l)


threading.Thread(target=rd, daemon=True).start()
n = [0]


def req(sub, **kw):
    n[0] += 1
    rid = f"r{n[0]}_{sub}"
    p.stdin.write(json.dumps({"type": "control_request", "request_id": rid,
                              "request": {"subtype": sub, **kw}}) + "\n")
    p.stdin.flush()
    print(f"\n>>> {sub} {json.dumps(kw)}")
    return rid


def show(rid, wait=8):
    t = time.time()
    while time.time() - t < wait:
        for l in lines:
            if f'"request_id":"{rid}"' in l and '"control_response"' in l:
                print(l[:3000].rstrip())
                return
        time.sleep(0.2)
    print(f"(no answer in {wait}s)")


req("initialize")
time.sleep(6)
for l in lines:
    try:
        o = json.loads(l)
    except ValueError:
        continue
    if o.get("subtype") == "init":
        print("system/init mcp_servers:", json.dumps(o.get("mcp_servers")))
steps = [
    ("mcp_status", {}),
    ("mcp_reconnect", {"serverName": "broken"}), ("mcp_status", {}),
    ("mcp_toggle", {"serverName": "good", "enabled": False}), ("mcp_status", {}),
    ("mcp_toggle", {"serverName": "good", "enabled": True}), ("mcp_status", {}),
    ("mcp_reconnect", {"serverName": "good"}),
    ("mcp_reconnect", {"serverName": "nope"}),
    ("mcp_authenticate", {"serverName": "needsauth"}),
    ("mcp_authenticate", {"serverName": "needsauth", "redirectUriOnly": True,
                          "redirectUri": "http://localhost:1/cb"}),
    ("mcp_authenticate", {"serverName": "good"}),
    ("mcp_clear_auth", {"serverName": "needsauth"}),
    ("mcp_oauth_callback_url", {"serverName": "needsauth",
                                "callbackUrl": "http://localhost:1/cb?code=x&state=y"}),
    ("mcp_status", {}),
]
for sub, kw in steps:
    show(req(sub, **kw), 20 if sub == "mcp_authenticate" else 8)
print("\n>>> user message: /mcp")
p.stdin.write(json.dumps({"type": "user", "message": {"role": "user", "content": "/mcp"}}) + "\n")
p.stdin.flush()
t = time.time()
while time.time() - t < 30 and not any('"type":"result"' in l for l in lines):
    time.sleep(0.3)
for l in lines[-12:]:
    print(l[:600].rstrip())
p.stdin.close()
p.wait(timeout=20)
open("/tmp/dt-mcp-spike-raw.ndjson", "w").write("".join(lines))
auth.terminate()
