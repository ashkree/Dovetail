#!/usr/bin/env python3
"""Search the agy binary's printable strings. Usage: strings.py <regex> [context-chars] [max]"""
import mmap
import os
import re
import sys

pattern = re.compile(sys.argv[1].encode())
ctx = int(sys.argv[2]) if len(sys.argv) > 2 else 80
limit = int(sys.argv[3]) if len(sys.argv) > 3 else 20
path = os.path.realpath(os.path.expanduser("~/.local/bin/agy"))
with open(path, "rb") as f:
    data = mmap.mmap(f.fileno(), 0, access=mmap.ACCESS_READ)
    seen = set()
    for m in pattern.finditer(data):
        s = data[max(0, m.start() - ctx): m.end() + ctx]
        s = re.sub(rb"[^\x20-\x7e]+", b" | ", s).decode()
        if s in seen:
            continue
        seen.add(s)
        print(s)
        print("--")
        limit -= 1
        if limit == 0:
            break
