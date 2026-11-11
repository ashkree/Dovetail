#!/usr/bin/env python3
"""Usage: sanitise.py <raw.ndjson> <fixture.ndjson>

Replaces the home directory, the thinking signatures (opaque blobs) and the /tmp/claude-<uid>
path; fails if an email address or key-like string remains. Everything else is kept as recorded.
"""
import json, re, sys

src, dst = sys.argv[1:3]
text = open(src).read()
text = text.replace("/home/mave", "/home/user").replace("/tmp/claude-1000", "/tmp/claude-UID")
text = re.sub(r'("signature":\s*")[A-Za-z0-9+/=]{20,}(")', r"\1SIGNATURE\2", text)
for line in text.splitlines():
    json.loads(line)
bad = re.findall(r"[\w.+-]+@[\w-]+\.[\w.]+|sk-[A-Za-z0-9-]{8,}|mave", text)
bad = [b for b in bad if not b.endswith("@builtin") and "example.invalid" not in b]
if bad:
    sys.exit("sensitive strings left: %s" % bad[:5])
open(dst, "w").write(text)
print(dst, len(text), "bytes")
