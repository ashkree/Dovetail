#!/usr/bin/env python3
"""Third probe: reconnect and enable/disable on a claude.ai connector (the first two probes only
tried local servers). Toggles one connected connector off and on again and always restores it
(finally block), then prints each answer. No model call. Needs the same scratch repo as probe.py.
Usage: python3 -I probe3.py ["connector name"]  (default: the first connected claudeai one)."""
import json, subprocess, sys, threading, time, uuid

p = subprocess.Popen(
    ["claude", "-p", "--input-format", "stream-json", "--output-format", "stream-json",
     "--verbose", "--permission-prompt-tool", "stdio", "--session-id", str(uuid.uuid4()),
     "--model", "haiku"],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, cwd="/tmp/dt-mcp-spike")
lines = []
threading.Thread(target=lambda: [lines.append(l) for l in p.stdout], daemon=True).start()
n = [0]


def req(sub, wait=30, **kw):
    n[0] += 1
    rid = f"p{n[0]}"
    p.stdin.write(json.dumps({"type": "control_request", "request_id": rid,
                              "request": {"subtype": sub, **kw}}) + "\n")
    p.stdin.flush()
    print(f"\n>>> {sub} {json.dumps(kw)}")
    t = time.time()
    while time.time() - t < wait:
        for l in lines:
            if f'"request_id":"{rid}"' in l and '"control_response"' in l:
                r = json.loads(l)["response"]
                print(json.dumps(r)[:400])
                return r
        time.sleep(0.2)
    print("(no answer)")


def status(name):
    r = req("mcp_status")
    for s in r["response"]["mcpServers"]:
        if s["name"] == name:
            print(f"    {name}: {s['status']}, {len(s.get('tools', []))} tools")
            return s["status"]


req("initialize")
time.sleep(6)
servers = req("mcp_status")["response"]["mcpServers"]
for s in servers:
    print("   ", s["name"], s["status"], s.get("transport"))
want = sys.argv[1] if len(sys.argv) > 1 else next(
    s["name"] for s in servers if s["status"] == "connected" and s.get("source") == "claudeai")
print("using:", want)
was_disabled = False
try:
    req("mcp_reconnect", serverName=want)
    status(want)
    req("mcp_toggle", serverName=want, enabled=False)
    was_disabled = True
    status(want)
finally:
    req("mcp_toggle", serverName=want, enabled=True)
    final = status(want)
    print("RESTORED" if final == "connected" else f"NOT RESTORED: {final}")
p.stdin.close()
p.wait(timeout=20)
