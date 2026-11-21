#!/usr/bin/env python3
"""Second probe: details beyond mcp_status (mcp_message tools/list, mcp_read_resource),
mcp_set_servers, and the OAuth start for a claude.ai connector that is `needs-auth`.
Uses redirectUriOnly so no browser flow completes. Same setup as probe.py. No model call."""
import json, os, subprocess, threading, time, uuid

HERE = os.path.dirname(os.path.abspath(__file__))
SERVER = os.path.join(HERE, "..", "mcp", "server.py")
cfg = "/tmp/dt-mcp-spike-mcp.json"
json.dump({"mcpServers": {"good": {"command": "python3", "args": [SERVER],
           "env": {"SPIKE_LOG": "/tmp/dt-mcp-spike-server.log"}}}}, open(cfg, "w"))
p = subprocess.Popen(
    ["claude", "-p", "--input-format", "stream-json", "--output-format", "stream-json",
     "--verbose", "--permission-prompt-tool", "stdio", "--session-id", str(uuid.uuid4()),
     "--model", "haiku", "--mcp-config", cfg],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, cwd="/tmp/dt-mcp-spike")
lines = []
threading.Thread(target=lambda: [lines.append(l) for l in p.stdout], daemon=True).start()
n = [0]


def req(sub, wait=10, **kw):
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
                print(l[:1500].rstrip())
                return
        time.sleep(0.2)
    print("(no answer)")


req("initialize")
time.sleep(6)
msg = {"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}
req("mcp_message", server_name="good", message=msg)
req("mcp_read_resource", serverName="good", uri="file:///x")
req("mcp_authenticate", serverName="claude.ai Google Calendar", redirectUriOnly=True,
    redirectUri="http://localhost:1/cb", wait=30)
req("mcp_set_servers", servers={"extra": {"type": "stdio", "command": "python3", "args": [SERVER]}})
req("mcp_status")
p.stdin.close()
p.wait(timeout=20)
