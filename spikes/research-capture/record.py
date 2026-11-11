#!/usr/bin/env python3
"""S7 recorder. Usage: record.py <foreground|background|search|mcp> <model> <out.ndjson>

Launches claude as Dovetail does (-p stream-json both ways, --verbose,
--include-partial-messages, --permission-mode auto, --permission-prompt-tool stdio),
sends one prompt, records every stdout line as {"t": ms, "dir": "out"|"in", "msg": ...}
until the turn(s) end, then closes stdin. Scratch repo only: /tmp/dovetail-spikes/research/repo.
"""
import json, subprocess, sys, time, select, pathlib, uuid

kind, model, out = sys.argv[1:4]
base = pathlib.Path("/tmp/dovetail-spikes/research")
repo = base / "repo"
repo.mkdir(parents=True, exist_ok=True)
if not (repo / ".git").exists():
    subprocess.run(["git", "init", "-q", "-b", "main"], cwd=repo, check=True)
    (repo / "notes.txt").write_text(
        "The launch codeword is MARIGOLD-42.\nSecond line: the rollout is staged in three waves.\n")
    subprocess.run(["git", "add", "."], cwd=repo, check=True)
    subprocess.run(["git", "-c", "user.name=spike", "-c", "user.email=spike@example.invalid",
                    "commit", "-qm", "init"], cwd=repo, check=True)
mcp = base / "mcp.json"
mcp.write_text(json.dumps({"mcpServers": {"dovetail": {
    "type": "stdio", "command": "python3",
    "args": [str(pathlib.Path(__file__).resolve().parents[1] / "mcp" / "server.py")],
    "env": {"SPIKE_LOG": str(base / "mcp.log")}}}}))

SUB = ("fetch https://example.com with WebFetch, read the local file {f} with Read, "
       "and then report in 3 short lines: the page title, the codeword, and the second line of the file. ")
f_notes = str(repo / "notes.txt")
if kind == "foreground":
    prompt = ("Use the Agent tool (foreground, general-purpose) once with this prompt: '"
              + SUB.format(f=f_notes) + "' When it returns, reply 'done' and its report verbatim.")
elif kind == "background":
    prompt = ("Use the Agent tool with run_in_background: true, general-purpose, once with this prompt: '"
              + SUB.format(f=f_notes) + "' Do not wait: reply 'launched' and end your turn. "
              "When the completion notification arrives, reply 'done' and the result.")
elif kind == "search":
    prompt = ("Use the Agent tool (foreground, general-purpose) once with this prompt: 'Use WebSearch for "
              "\"example.com IANA reserved domain\" and report the first 2 result titles and URLs.' "
              "When it returns, reply 'done' and its report verbatim.")
else:
    prompt = ("Use the Agent tool (foreground, general-purpose) once with this prompt: 'Call the tool "
              "mcp__dovetail__who with no arguments, then report its output in one line.' "
              "When it returns, reply 'done' and its report verbatim.")

args = ["claude", "-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose",
        "--include-partial-messages", "--permission-mode", "auto", "--permission-prompt-tool", "stdio",
        "--session-id", str(uuid.uuid4()), "--model", model, "--mcp-config", str(mcp),
        "--allowedTools", "WebFetch,WebSearch,Read,Agent,mcp__dovetail__who,mcp__dovetail__ping"]
p = subprocess.Popen(args, cwd=repo, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
t0 = time.time()
f = open(out, "w")


def rec(d, msg):
    f.write(json.dumps({"t": int((time.time() - t0) * 1000), "dir": d, "msg": msg}) + "\n")
    f.flush()


um = {"type": "user", "message": {"role": "user", "content": prompt}, "parent_tool_use_id": None}
rec("in", um)
p.stdin.write((json.dumps(um) + "\n").encode())
p.stdin.flush()
results = 0
want = 2 if kind == "background" else 1
deadline = t0 + 240
try:
    while time.time() < deadline and p.poll() is None:
        r, _, _ = select.select([p.stdout], [], [], 1)
        if not r:
            continue
        line = p.stdout.readline()
        if not line:
            break
        try:
            m = json.loads(line)
        except Exception:
            continue
        rec("out", m)
        if m.get("type") == "result":
            results += 1
        if results >= want:
            break
finally:
    try:
        p.stdin.close()
    except Exception:
        pass
    try:
        p.wait(timeout=20)
    except Exception:
        p.kill()
    f.close()
print("results", results, "exit", p.returncode, "secs", int(time.time() - t0))
