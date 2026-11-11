#!/usr/bin/env python3
"""Usage: inspect.py <raw.ndjson>. Prints the task_notification, the output file's shape,
the async-launch result and any WebSearch result bodies."""
import json, os, sys

for l in open(sys.argv[1]):
    m = json.loads(l)["msg"]
    if m.get("subtype") == "task_notification":
        print("NOTIFICATION", {k: v for k, v in m.items() if k not in ("uuid", "session_id")})
        p = m["output_file"]
        print("output_file ->", os.path.realpath(p), os.path.getsize(p), "bytes")
        for x in open(p):
            d = json.loads(x)
            c = d.get("message", {}).get("content") if "message" in d else None
            print("  ", d.get("type"), str(c)[:220] if c else list(d)[:5])
    if m.get("type") == "user" and isinstance(m.get("tool_use_result"), dict) and m["tool_use_result"].get("isAsync"):
        print("ASYNC LAUNCH", json.dumps(m["tool_use_result"])[:700])
    if m.get("type") == "user" and m.get("parent_tool_use_id"):
        for b in m["message"]["content"]:
            if isinstance(b, dict) and b.get("type") == "tool_result" and "Links" in json.dumps(b):
                print("WEBSEARCH", json.dumps(b)[:1800])
                print("  tool_use_result:", json.dumps(m.get("tool_use_result"))[:600])
