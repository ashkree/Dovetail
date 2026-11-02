#!/usr/bin/env python3
"""Toy stdio MCP server for spike S3. Tools: ping(hold_seconds), who().
Logs every request (incl. _meta) as JSON lines to $SPIKE_LOG."""
import json, os, sys, time, threading

LOG = os.environ.get("SPIKE_LOG", "/tmp/dovetail-spikes/mcp/server.log")
out_lock = threading.Lock()

def log(kind, obj):
    with open(LOG, "a") as f:
        f.write(json.dumps({"t": time.time(), "pid": os.getpid(), "kind": kind, "data": obj}) + "\n")

def send(msg):
    with out_lock:
        sys.stdout.write(json.dumps(msg) + "\n"); sys.stdout.flush()

TOOLS = [
    {"name": "ping", "description": "Wait hold_seconds then return pong.",
     "inputSchema": {"type": "object", "properties": {"hold_seconds": {"type": "number"}, "tag": {"type": "string"}}, "required": ["hold_seconds"]}},
    {"name": "who", "description": "Return immediately; server logs the full request for caller attribution.",
     "inputSchema": {"type": "object", "properties": {"tag": {"type": "string"}}}},
]

def handle_call(req):
    p = req["params"]; name = p["name"]; args = p.get("arguments", {})
    log("call_start", {"id": req["id"], "params": p})
    if name == "ping":
        time.sleep(float(args.get("hold_seconds", 0)))
    log("call_end", {"id": req["id"], "name": name})
    send({"jsonrpc": "2.0", "id": req["id"], "result": {"content": [{"type": "text", "text": f"pong {args.get('tag','')}"}]}})

for line in sys.stdin:
    line = line.strip()
    if not line: continue
    req = json.loads(line)
    m = req.get("method")
    if m != "tools/call": log("req", req)
    if m == "initialize":
        send({"jsonrpc": "2.0", "id": req["id"], "result": {"protocolVersion": req["params"].get("protocolVersion", "2025-06-18"),
              "capabilities": {"tools": {}}, "serverInfo": {"name": "spike", "version": "0"}}})
    elif m == "tools/list":
        send({"jsonrpc": "2.0", "id": req["id"], "result": {"tools": TOOLS}})
    elif m == "tools/call":
        threading.Thread(target=handle_call, args=(req,), daemon=True).start()
    elif "id" in req:
        send({"jsonrpc": "2.0", "id": req["id"], "result": {}})
log("exit", {})
