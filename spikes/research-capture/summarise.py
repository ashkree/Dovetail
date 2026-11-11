#!/usr/bin/env python3
"""Print an event-level summary of a recorded stream (skips stream_event deltas)."""
import json, sys

for line in open(sys.argv[1]):
    r = json.loads(line)
    m = r["msg"]
    if r["dir"] == "in":
        print("IN ", str(m)[:100]); continue
    t = m.get("type"); st = m.get("subtype")
    if t == "stream_event":
        continue
    par = m.get("parent_tool_use_id")
    tag = f"[parent={par}]" if par else ""
    if t in ("assistant", "user"):
        c = m["message"]["content"]
        if isinstance(c, str):
            print(t, tag, "TEXT", c[:120]); continue
        for b in c:
            bt = b.get("type")
            if bt == "tool_use":
                print(t, tag, "tool_use", b["name"], b["id"], json.dumps(b["input"])[:200])
            elif bt == "tool_result":
                cc = b.get("content")
                s = cc if isinstance(cc, str) else json.dumps(cc)
                print(t, tag, "tool_result", b["tool_use_id"], "err=%s" % b.get("is_error"), "len=%d" % len(s), repr(s[:160]))
            elif bt == "text":
                print(t, tag, "text", repr(b["text"][:200]))
            else:
                print(t, tag, bt)
        if "tool_use_result" in m:
            print("    tool_use_result keys:", list(m["tool_use_result"]) if isinstance(m["tool_use_result"], dict) else type(m["tool_use_result"]))
    elif t == "system":
        if st in ("init",):
            print("system init"); continue
        print("system", st, tag, json.dumps({k: v for k, v in m.items() if k not in ("type", "subtype", "uuid", "session_id")})[:300])
    elif t == "result":
        print("RESULT", m.get("subtype"), "cost", m.get("total_cost_usd"), repr(str(m.get("result"))[:200]))
    else:
        print(t, st)
