"""Generates the feature folders of dovetail-api from model.rs, protocol.rs and the whole files."""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(__file__))
from spec import *  # noqa

SRC = sys.argv[1]  # .../crates/dovetail-api/src (old layout, read from git HEAD copy)
OUT = sys.argv[2]  # output dir

FEATURE_DOC = {
    "task": "tasks: the user's own work items, their status, order and links",
    "decision": "decision logs: what was decided on a task, and why",
    "agent": "agents: coordinators, slices, side chats and subagents, and their native to-do lists",
    "conversation": "conversations: each agent's entries, the chat bar's prompts and `@` completion",
    "inbox": "Needs you: everything waiting on the user, and how the user answers",
    "model": "the models a session can run and the reasoning effort it runs them at",
    "mcp": "the MCP menu: a session's servers and the actions on them",
    "journal": "the end-of-day journal and the coordinator's handoffs",
    "tangent": "tangents: questions that do not belong to the open project",
    "terminal": "handing the user's terminal to an editor or to native `claude`",
    "clipboard": "the system clipboard: reading it for a chat bar, copying to it",
    "upgrade": "`:upgrade`: building and swapping in the next binary",
    "canvas": "canvases: an effort's blocks and cards",
    "design": "design previews and variants a slice declares",
    "effort": "efforts: goals with acceptance criteria",
    "worktree": "slice worktrees: git and pull-request state",
    "project": "projects: registered projects, their repos and account",
    "map": "maps: drawn diagrams and their versions",
    "research": "research: rounds, findings, sources and reports of a task",
    "library": "reading mode's library: project docs and other projects' research",
    "app": "the app itself: resync, quit, open a URL, command failures",
    "activity": "the Activity feed and the messages agents send each other",
    "status": "the status bar: totals and warnings",
    "usage": "Claude usage limits",
}
CMD_DOC = {
    "task": "Tasks", "decision": "Decision logs", "agent": "Agents", "conversation": "Conversations",
    "inbox": "Needs you", "model": "Models", "mcp": "The MCP menu", "journal": "The journal",
    "tangent": "Tangents", "terminal": "The terminal", "clipboard": "The clipboard",
    "upgrade": "Upgrades", "canvas": "Canvases", "design": "Design previews and variants",
    "effort": "Efforts", "worktree": "Worktrees", "project": "Projects", "map": "Maps",
    "research": "Research", "library": "The library", "app": "The app",
}


def mod_path(dest):
    p = dest[:-3]
    if p.endswith("/mod"):
        p = p[:-4]
    parts = p.split("/")
    # command.rs, event.rs and attachment.rs are re-exported by their feature's mod.rs.
    if len(parts) > 1 and parts[-1] in ("command", "event", "attachment"):
        parts = parts[:-1]
    return "crate::" + "::".join(parts)


# ---------- parsing ----------
ITEM_RE = re.compile(r"^(pub )?(const|fn|struct|enum|type|impl|mod)\b")


def parse_items(path):
    """Top-level items: (name, kind, text), with leading docs/attrs. impls carry the type's name."""
    lines = open(path).read().split("\n")
    items, pending, i = [], [], 0
    while i < len(lines):
        l = lines[i]
        if l.startswith("///") or l.startswith("#[") or l.startswith("//"):
            pending.append(l)
            i += 1
            continue
        if l == "":
            # a blank line ends a pending run of plain comments only; keep docs attached
            if pending and not any(p.startswith("///") or p.startswith("#[") for p in pending):
                pending = []
            i += 1
            continue
        m = ITEM_RE.match(l)
        if not m:
            pending = []
            i += 1
            continue
        kind = m.group(2)
        start = i
        # find the end
        if l.rstrip().endswith(";") and "{" not in l:
            end = i
        else:
            j = i
            if l.rstrip().endswith("{") or "{" in l and not l.rstrip().endswith(";"):
                while lines[j] != "}":
                    j += 1
            else:
                while not lines[j].rstrip().endswith(";"):
                    j += 1
            end = j
        text = "\n".join(pending + lines[start : end + 1])
        if kind == "impl":
            mm = re.match(r"impl(?:<[^>]*>)?\s+(?:[\w:<>]+\s+for\s+)?(?:\w+::)*(\w+)", l)
            name = mm.group(1)
        else:
            name = re.match(r"(?:pub )?(?:const|fn|struct|enum|type|mod)\s+(\w+)", l).group(1)
        items.append((name, kind, text))
        pending = []
        i = end + 1
    return items


def parse_variants(path, enum):
    lines = open(path).read().split("\n")
    s = lines.index(f"pub enum {enum} {{")
    out, pending, i = [], [], s + 1
    while lines[i] != "}":
        l = lines[i]
        if l.startswith("    ///") or l.startswith("    #[") or l.startswith("    //"):
            pending.append(l)
            i += 1
            continue
        m = re.match(r"    ([A-Z]\w*)", l)
        assert m, (enum, i, l)
        name = m.group(1)
        j = i
        if l.rstrip().endswith("{"):
            while lines[j] != "    },":
                j += 1
        out.append((name, "\n".join(pending + lines[i : j + 1])))
        pending = []
        i = j + 1
    return out


# ---------- name table ----------
NAME_MOD = {}


def register(name, mod):
    assert name not in NAME_MOD or NAME_MOD[name] == mod, (name, mod, NAME_MOD[name])
    NAME_MOD[name] = mod


items_by_file = {}
for fname in ("model.rs", "protocol.rs"):
    for name, kind, text in parse_items(f"{SRC}/{fname}"):
        items_by_file.setdefault(fname, []).append((name, kind, text))

special = {"Event", "Command"}
for fname, its in items_by_file.items():
    for name, kind, text in its:
        if name in special:
            continue
        assert name in ITEM_DEST, f"unmapped {name} in {fname}"
        register(name, mod_path(ITEM_DEST[name]))

whole_items = {}
for old, new in WHOLE.items():
    its = parse_items(f"{SRC}/{old}")
    whole_items[old] = its
    for name, kind, text in its:
        if old == "research.rs" and name in LIBRARY_ITEMS:
            register(name, "crate::library")
        elif kind not in ("impl", "mod"):
            register(name, mod_path(new))
register("geometry", "crate::canvas")
# driver.rs
for name, kind, text in parse_items(f"{SRC}/driver.rs"):
    if kind != "impl":
        register(name, "crate::model")
for f in sorted({v[0] for v in CMD.values()}):
    register(f"{f.capitalize()}Command" if f != "" else "", f"crate::{f}")
for f in sorted({v[0] for v in EVT.values()}):
    register(f"{f.capitalize()}Event", f"crate::{f}")


def cname(f, kind):
    return f"{f.capitalize()}{kind}"


# Names that are also enum variants or common words: only imported when used as types.
EXTERNAL = {
    "PathBuf": "use std::path::PathBuf;",
    "BTreeMap": "use std::collections::BTreeMap;",
    "Receiver": "use std::sync::mpsc::Receiver;",
    "Sender": "use std::sync::mpsc::Sender;",
}


def strip_comments(text):
    return "\n".join(l for l in text.split("\n") if not l.lstrip().startswith("//"))


def rewrite_refs(text, feature):
    """Map `Command::Old`, `Event::Old` (optionally `crate::`-prefixed) to the nested enums."""

    def sub(m):
        kind, old = m.group(2), m.group(3)
        table = CMD if kind == "Command" else EVT
        if old not in table:
            return m.group(0)
        f, new = table[old]
        enum = cname(f, kind)
        if f == feature:
            return f"{enum}::{new}"
        return f"crate::{f}::{enum}::{new}"

    return re.sub(r"(crate::)?\b(Command|Event)::(\w+)", sub, text)


def unqualify(text):
    # crate::design::Preview -> Preview (names are imported); keep crate::apply etc. alone
    def sub(m):
        last = m.group(1)
        if last in NAME_MOD:
            return last
        return m.group(0)

    return re.sub(r"\bcrate::(?:\w+::)*(\w+)", sub, text)


def imports_for(text, defined, own_feature=None):
    code = strip_comments(text)
    # drop string literals
    code = re.sub(r'"(?:[^"\\]|\\.)*"', '""', code)
    toks = set(re.findall(r"\b[A-Za-z_]\w*\b", code))
    by_mod, ext = {}, []
    for t in toks:
        if t in defined:
            continue
        if t in NAME_MOD:
            by_mod.setdefault(NAME_MOD[t], []).append(t)
        elif t in EXTERNAL:
            ext.append(EXTERNAL[t])
    if "Serialize" in toks or "Deserialize" in toks:
        ext.append("use serde::{Deserialize, Serialize};")
    lines = sorted(set(ext))
    for m in sorted(by_mod):
        names = sorted(by_mod[m])
        lines.append(f"use {m}::{{{', '.join(names)}}};" if len(names) > 1 else f"use {m}::{names[0]};")
    return "\n".join(lines)


def defined_names(text):
    return set(re.findall(r"^(?:pub )?(?:const|fn|struct|enum|type)\s+(\w+)", text, re.M))


def write(rel, body):
    p = f"{OUT}/{rel}"
    os.makedirs(os.path.dirname(p), exist_ok=True)
    open(p, "w").write(body.rstrip("\n") + "\n")


# ---------- assemble ----------
files = {}  # rel -> list of chunk texts


def add(rel, text):
    files.setdefault(rel, []).append(text)


for fname, its in items_by_file.items():
    for name, kind, text in its:
        if name in special:
            continue
        add(ITEM_DEST[name], text)

# Command / Event variants
variants = {}
for enum, table, kind in (("Command", CMD, "Command"), ("Event", EVT, "Event")):
    for name, text in parse_variants(f"{SRC}/protocol.rs", enum):
        assert name in table, (enum, name)
        f, new = table[name]
        variants.setdefault((f, kind), []).append((new, re.sub(rf"^(    )({name})\b", rf"\g<1>{new}", text, count=1, flags=re.M) if False else text, name))

for (f, kind), vs in sorted(variants.items()):
    chunks = []
    for new, text, old in vs:
        # rename the variant identifier line (first line at 4-space indent starting with the name)
        lines = text.split("\n")
        for idx, l in enumerate(lines):
            if re.match(rf"    {old}\b", l):
                lines[idx] = re.sub(rf"^    {old}\b", f"    {new}", l)
                break
        chunks.append(rewrite_refs("\n".join(lines), f))
    enum = cname(f, kind)
    doc = (
        f"/// {CMD_DOC[f]}: what the UI asks the core to do."
        if kind == "Command"
        else f"/// A change to the client's snapshot, or an answer, in the feature `{f}`."
    )
    body = (
        f"{doc}\n#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\n"
        f'#[serde(rename_all = "snake_case")]\npub enum {enum} {{\n' + "\n".join(chunks) + "\n}"
    )
    rel = f"{f}/{kind.lower()}.rs"
    files.setdefault(rel, []).insert(0, body)

# whole files
for old, new in WHOLE.items():
    for name, kind, text in whole_items[old]:
        if old == "research.rs" and name in LIBRARY_ITEMS:
            add("library/mod.rs", text)
        else:
            add(new, text)
# impls of library items in research.rs were attached by name; they are handled by LIBRARY_ITEMS above

# driver.rs (DriverKind) into model/mod.rs
for name, kind, text in parse_items(f"{SRC}/driver.rs"):
    add("model/mod.rs", text)

# ---------- emit ----------
feature_dirs = sorted({r.split("/")[0] for r in files if "/" in r})
# make sure every feature that has commands/events/items gets a mod.rs
for f in sorted({v[0] for v in CMD.values()} | {v[0] for v in EVT.values()}):
    files.setdefault(f"{f}/mod.rs", [])

for rel, chunks in sorted(files.items()):
    chunks = [unqualify(rewrite_refs(c, rel.split("/")[0])) for c in chunks]
    tests = [c for c in chunks if re.search(r"^mod tests \{", c, re.M)]
    chunks = [c for c in chunks if c not in tests]
    text = "\n\n".join(chunks)
    own = defined_names(text)
    for t in tests:
        extra = imports_for(t, own)
        extra = "\n".join("    " + l for l in extra.split("\n") if l)
        t = t.replace("    use super::*;\n", "    use super::*;\n" + (extra + "\n" if extra else ""), 1)
        text += "\n\n" + t
    feat = rel.split("/")[0]
    header = ""
    mods = ""
    reexports = ""
    if rel.endswith("/mod.rs"):
        desc = FEATURE_DOC.get(feat, feat)
        header = f"//! The `{feat}` feature: {desc}.\n\n"
    elif rel.endswith("/command.rs"):
        header = f"//! The `{feat}` feature's commands.\n\n"
    elif rel.endswith("/event.rs"):
        header = f"//! The `{feat}` feature's events.\n\n"
    elif rel.endswith("/attachment.rs"):
        header = "//! Images attached to a message, and the limits on them.\n\n"
    defined = defined_names(text)
    imp = imports_for("\n\n".join(chunks), defined)
    write(rel, header + imp + "\n\n" + text if imp else header + text)

print("features:", len(feature_dirs))
