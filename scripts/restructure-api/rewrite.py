"""Rewrites callers of dovetail-api to the by-feature layout.

usage: rewrite.py PREFIX API_SRC FILE...
  PREFIX is `dovetail_api` (other crates) or `crate` (inside dovetail-api).
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(__file__))
from spec import CMD, EVT, LIBRARY_ITEMS  # noqa

PREFIX, API_SRC, FILES = sys.argv[1], sys.argv[2], sys.argv[3:]


# ---------- name table from the new tree ----------
def build_names():
    names = {}
    for f in sorted(os.listdir(API_SRC)):
        d = f"{API_SRC}/{f}"
        if not os.path.isdir(d):
            continue
        for sub in ("mod", "command", "event", "attachment"):
            p = f"{d}/{sub}.rs"
            if not os.path.exists(p):
                continue
            for m in re.finditer(r"^pub (?:struct|enum|fn|const|type|mod)\s+(\w+)", open(p).read(), re.M):
                names[m.group(1)] = f
    # snapshot.rs constants (tail caps) are public in snapshot but the module is private: expose
    # via re-export? They live in `snapshot`; callers use them as dovetail_api::snapshot::X.
    return names


NAMES = build_names()
# items that were never at the old crate root (they lived in a `pub mod`)
NOT_ROOT_OLD = set()
for old in ("map", "research", "mcp_x"):
    pass
OLD_NONROOT_MODS = {"map", "research", "library"}
ROOT_KEEP = {"Command", "Event", "Snapshot", "ServerMsg", "CoreLink", "PROTOCOL_VERSION", "apply",
             "ApplyResult"}
SNAPSHOT_NAMES = {"ACTIVITY_TAIL", "MESSAGE_TAIL", "CONVERSATION_TAIL", "CONVERSATION_KEEP"}
SUBENUMS = {}
for old, (f, new) in CMD.items():
    SUBENUMS[f"{f.capitalize()}Command"] = f
for old, (f, new) in EVT.items():
    SUBENUMS[f"{f.capitalize()}Event"] = f


def new_path(old_segments):
    """old path under the crate root (list of segments) -> new list, or None to keep."""
    head = old_segments[0]
    if head in ROOT_KEEP:
        return None
    if head in SNAPSHOT_NAMES:
        return ["snapshot"] + old_segments if False else [head]  # handled by re-export in lib
    if head in ("research", "map") and len(old_segments) >= 2:
        if head == "research" and old_segments[1] in LIBRARY_ITEMS:
            return ["library"] + old_segments[1:]
        return None
    if head in ("tools", "socket", "geometry"):
        if head == "geometry":
            return ["canvas", "geometry"] + old_segments[1:]
        return None
    if head in NAMES and NAMES[head] in ("map", "research") and len(old_segments) == 1:
        return None
    if head in NAMES:
        return [NAMES[head]] + old_segments
    return None


# ---------- lexer mask ----------
def mask(text):
    """kind per char: 'c' code, 'm' comment, 's' string/char."""
    n, k, i = len(text), ["c"] * len(text), 0
    while i < n:
        ch = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            k[i:j] = "m" * (j - i)
            i = j
        elif text.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if text.startswith("/*", j):
                    depth += 1
                    j += 2
                elif text.startswith("*/", j):
                    depth -= 1
                    j += 2
                else:
                    j += 1
            k[i:j] = "m" * (j - i)
            i = j
        elif ch == "r" and re.match(r'r#*"', text[i:i + 8]) and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
            m = re.match(r'r(#*)"', text[i:])
            end = '"' + m.group(1)
            j = text.find(end, i + len(m.group(0)))
            j = n if j < 0 else j + len(end)
            k[i:j] = "s" * (j - i)
            i = j
        elif ch == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            j = min(j + 1, n)
            k[i:j] = "s" * (j - i)
            i = j
        elif ch == "'":
            if i + 1 < n and text[i + 1] == "\\":
                j = text.find("'", i + 2)
                j = n if j < 0 else j + 1
                k[i:j] = "s" * (j - i)
                i = j
            elif i + 2 < n and text[i + 2] == "'":
                k[i:i + 3] = "sss"
                i += 3
            else:
                i += 1
        else:
            i += 1
    return k


def find_close(text, k, i):
    """text[i] is an opener in code; index of its matching closer."""
    pairs = {"(": ")", "{": "}", "[": "]"}
    depth, j = 0, i
    while j < len(text):
        if k[j] == "c":
            c = text[j]
            if c in "({[":
                depth += 1
            elif c in ")}]":
                depth -= 1
                if depth == 0:
                    return j
        j += 1
    raise ValueError("unbalanced")


# ---------- use-tree parsing ----------
def parse_tree(s, i=0):
    """Parse a use tree starting at s[i]; returns (list of (segments, alias)), next index)."""
    out = []
    while True:
        while i < len(s) and s[i] in " \n\t,":
            i += 1
        if i >= len(s) or s[i] == "}":
            return out, i
        if s[i] == "{":
            sub, i = parse_tree(s, i + 1)
            out.extend((seg, al) for seg, al in sub)
            i += 1  # }
            continue
        m = re.match(r"[\w*]+", s[i:])
        seg = m.group(0)
        i += len(seg)
        while i < len(s) and s[i] in " \n\t":
            i += 1
        if s.startswith("::", i):
            i += 2
            while i < len(s) and s[i] in " \n\t":
                i += 1
            if s[i] == "{":
                sub, i = parse_tree(s, i + 1)
                i += 1
                out.extend(([seg] + sg, al) for sg, al in sub)
            else:
                sub, i = parse_tree_one(s, i)
                out.extend(([seg] + sg, al) for sg, al in sub)
        else:
            alias = None
            m2 = re.match(r"as\s+(\w+)", s[i:])
            if m2:
                alias = m2.group(1)
                i += m2.end()
            out.append(([seg], alias))
    return out, i


def parse_tree_one(s, i):
    m = re.match(r"([\w*]+)", s[i:])
    seg = m.group(0)
    i += len(seg)
    if s.startswith("::", i):
        i += 2
        if s[i] == "{":
            sub, i = parse_tree(s, i + 1)
            i += 1
            return [([seg] + sg, al) for sg, al in sub], i
        sub, i = parse_tree_one(s, i)
        return [([seg] + sg, al) for sg, al in sub], i
    alias = None
    m2 = re.match(r"\s*as\s+(\w+)", s[i:])
    if m2:
        alias = m2.group(1)
        i += m2.end()
    return [([seg], alias)], i


def render_tree(leaves):
    tree = {}
    for seg, alias in leaves:
        node = tree
        for s_ in seg[:-1]:
            node = node.setdefault(s_, {})
        leaf = seg[-1] + (f" as {alias}" if alias else "")
        node.setdefault(leaf, None)

    def r(node):
        parts = []
        for key in sorted(node):
            v = node[key]
            if v is None:
                parts.append(key)
            elif len(v) == 1 and list(v.values())[0] is None:
                parts.append(f"{key}::{list(v)[0]}")
            else:
                parts.append(f"{key}::{{{r(v)}}}")
        return ", ".join(parts)

    return r(tree)


def render_use(indent, leaves):
    inner = render_tree(leaves)
    if "," not in inner and "{" not in inner:
        return f"{indent}use {PREFIX}::{inner};"
    return f"{indent}use {PREFIX}::{{{inner}}};"


USE_RE = re.compile(rf"^([ \t]*)use {re.escape(PREFIX)}::", re.M)


def used_tokens(text, k):
    code = "".join(c if kk == "c" else " " for c, kk in zip(text, k))
    return set(re.findall(r"\b[A-Za-z_]\w*\b", code))


def process(path):
    text = open(path).read()
    orig = text
    k = mask(text)
    # ---- 1. variant rewrites (collect edits) ----
    edits = []  # (pos, delete_len, insert)
    needed = []  # (pos, enumname)
    var_re = re.compile(rf"(?<![\w:])(?:{re.escape(PREFIX)}::)?(Command|Event)::(\w+)")
    for m in var_re.finditer(text):
        if k[m.start()] != "c":
            continue
        kind, old = m.group(1), m.group(2)
        table = CMD if kind == "Command" else EVT
        if old not in table:
            continue
        f, new = table[old]
        enum = f"{f.capitalize()}{kind}"
        # keep the `PREFIX::` qualifier if there was one
        qual = f"{PREFIX}::" if m.group(0).startswith(PREFIX) else ""
        j = m.end()
        while j < len(text) and text[j] in " \t":
            j += 1
        if j < len(text) and text[j] in "({" and k[j] == "c":
            close = find_close(text, k, j)
        else:
            close = m.end() - 1
        edits.append((m.start(), m.end() - m.start(), f"{qual}{kind}::{f.capitalize()}({enum}::{new}"))
        edits.append((close + 1, 0, ")"))
        needed.append((m.start(), enum))
    # comment mentions
    for m in re.finditer(r"\b(Command|Event)::(\w+)", text):
        if k[m.start()] != "m":
            continue
        table = CMD if m.group(1) == "Command" else EVT
        if m.group(2) in table:
            f, new = table[m.group(2)]
            edits.append((m.start(), m.end() - m.start(), f"{f.capitalize()}{m.group(1)}::{new}"))
    # ---- 2. qualified paths PREFIX::Name ----
    qual_re = re.compile(rf"(?<![\w:]){re.escape(PREFIX)}::(\w+)(?!\s*!)")
    for m in qual_re.finditer(text):
        if k[m.start()] not in "cm":
            continue
        name = m.group(1)
        # skip the ones already edited (variants handled above start with PREFIX::Command::)
        if name in ("Command", "Event"):
            continue
        np = new_path([name])
        if np:
            edits.append((m.start(), m.end() - m.start(), f"{PREFIX}::{'::'.join(np)}"))
        # `dovetail_api::research::DocEntry` and friends
    qual2 = re.compile(rf"(?<![\w:]){re.escape(PREFIX)}::(research|map)::(\w+)")
    for m in qual2.finditer(text):
        if k[m.start()] not in "cm":
            continue
        np = new_path([m.group(1), m.group(2)])
        if np:
            edits.append((m.start(), m.end() - m.start(), f"{PREFIX}::{'::'.join(np)}"))
    # ---- 3. use statements ----
    stmts = []  # (start, end, indent, leaves)
    for m in USE_RE.finditer(text):
        if k[m.start() + len(m.group(1))] != "c":
            continue
        s = m.end()
        e = text.index(";", s)
        stmts.append((m.start(), e + 1, m.group(1), s, e))
    # scopes: col-0 `mod name {` blocks
    scopes = []
    for m in re.finditer(r"^(?:pub )?mod (\w+) \{$", text, re.M):
        close = find_close(text, k, m.end() - 1)
        scopes.append((m.end(), close))

    def scope_of(pos):
        best = None
        for a, b in scopes:
            if a <= pos < b and (best is None or a > best[0]):
                best = (a, b)
        return best

    need_by_scope = {}
    for pos, enum in needed:
        need_by_scope.setdefault(scope_of(pos), set()).add(enum)
    # the stmts rewrite
    new_stmts = []  # (start, end, replacement)
    used_in_scope = {}
    for start, end, indent, s, e in stmts:
        body = text[s:e]
        # tree after prefix
        leaves, _ = parse_tree(body)
        sc = scope_of(start)
        out_leaves = []
        for seg, alias in leaves:
            if seg == ["*"]:
                # glob: every name the scope uses
                a, b = sc if sc else (0, len(text))
                toks = {t for t in used_tokens(text[a:b], k[a:b])}
                for t in sorted(toks):
                    if t in ROOT_KEEP and t not in ("Snapshot", "ServerMsg", "CoreLink", "PROTOCOL_VERSION", "apply", "ApplyResult", "Command", "Event"):
                        continue
                    if t in ("Command", "Event", "Snapshot", "ServerMsg", "CoreLink", "PROTOCOL_VERSION", "apply", "ApplyResult"):
                        out_leaves.append(([t], None))
                    elif t in NAMES and NAMES[t] not in ("map", "research") and t not in LIBRARY_ITEMS:
                        out_leaves.append(([NAMES[t], t], None))
                    elif t in LIBRARY_ITEMS:
                        pass
                    elif t in SUBENUMS:
                        pass
                    elif t in SNAPSHOT_NAMES:
                        out_leaves.append(([t], None))
                continue
            np = new_path(seg)
            out_leaves.append(((np or seg), alias))
        sc_needed = need_by_scope.pop(sc, set())
        for enum in sorted(sc_needed):
            out_leaves.append(([SUBENUMS[enum], enum], None))
        used_in_scope[sc] = True
        new_stmts.append((start, end, render_use(indent, out_leaves)[len(indent) * 0:]))
    # scopes needing imports but with no existing use statement
    inserts = []
    for sc, enums in need_by_scope.items():
        leaves = [([SUBENUMS[e], e], None) for e in sorted(enums)]
        if sc is None:
            # after the last col-0 use statement, else at the first blank line after the header
            ms = list(re.finditer(r"^use [^;]*;$", text, re.M))
            ms = [m for m in ms if k[m.start()] == "c" and not scope_of(m.start())]
            if ms:
                pos = ms[-1].end()
                inserts.append((pos, "\n" + render_use("", leaves)))
            else:
                # after leading //! doc lines
                i = 0
                lines = text.split("\n")
                n = 0
                while n < len(lines) and (lines[n].startswith("//!") or lines[n] == ""):
                    i += len(lines[n]) + 1
                    n += 1
                inserts.append((i, render_use("", leaves) + "\n\n"))
        else:
            a, b = sc
            m = re.search(r"\n    use super::\*;", text[a:b])
            pos = a + m.end() if m else a
            inserts.append((pos, "\n" + render_use("    ", leaves)))
    # ---- apply all edits (descending) ----
    allw = []
    for pos, dl, ins in edits:
        allw.append((pos, dl, ins))
    for start, end, rep in new_stmts:
        # keep indentation: the replaced region starts at the line start (indent included in rep)
        allw.append((start, end - start, rep))
    for pos, ins in inserts:
        allw.append((pos, 0, ins))
    allw.sort(key=lambda t: (t[0], -t[1]), reverse=True)
    out = text
    last = None
    for pos, dl, ins in allw:
        out = out[:pos] + ins + out[pos + dl:]
    if out != orig:
        open(path, "w").write(out)
        return True
    return False


changed = 0
for p in FILES:
    try:
        if process(p):
            changed += 1
    except Exception as ex:  # noqa
        print("FAILED", p, repr(ex))
print("changed", changed, "of", len(FILES))
