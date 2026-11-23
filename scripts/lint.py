#!/usr/bin/env python3
"""Structural lints for Ferro (part of scripts/ferro-check, run in CI).

1. glossary  — public type names may not use a word the glossary forbids
               (docs/foundation/05-naming-conventions.md, "Don't use") or
               the banned suffixes (Manager, Utils, Helper, Entity, Object).
2. layers    — inside ferro-core a layer imports only layers below it:
               math → geom → topo → algo → model → ops (D-19).
3. deps      — ferro-core dependencies are an allow-list; a new one needs an ADR.
4. tasks     — docs/tasks/TREE.md: markers valid, claimed / done items have an
               owner, a done parent has only done children, ids are unique.
5. api       — docs/api/TREE.md is up to date (scripts/api_tree.py --check).
6. python    — python/ferro/_tools.py matches python/ferro/tools.json
               (scripts/gen_python.py --check, ADR 0005).

Exit code 1 and one line per problem when something is wrong.
"""

import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CORE = os.path.join(ROOT, "crates", "ferro-core")
LAYERS = ["math", "geom", "topo", "algo", "model", "ops"]
CORE_DEPS = {"serde", "serde_json", "robust", "earcutr", "imbl"}
BANNED_PARTS = ["Manager", "Utils", "Util", "Helper", "Entity", "Object"]

problems = []


def rel(p):
    return os.path.relpath(p, ROOT)


def rust_files(base):
    for d, _, files in os.walk(base):
        if "target" in d.split(os.sep):
            continue
        for f in files:
            if f.endswith(".rs"):
                yield os.path.join(d, f)


def glossary_banned():
    path = os.path.join(ROOT, "docs", "foundation", "05-naming-conventions.md")
    banned = set()
    with open(path, encoding="utf-8") as f:
        for line in f:
            cells = [c.strip() for c in line.strip().strip("|").split("|")]
            if len(cells) != 3 or cells[0] in ("Use", "---"):
                continue
            for word in cells[2].split(","):
                # "Floor (as a datum)": banned only in that sense, not checkable.
                if "(" in word:
                    continue
                w = word.strip().strip("`")
                # Multi-word phrases become PascalCase identifiers.
                ident = "".join(p[:1].upper() + p[1:] for p in w.split())
                if ident and ident.isidentifier():
                    banned.add(ident)
    return banned


def check_glossary():
    banned = glossary_banned()
    decl = re.compile(r"^\s*pub(?:\([^)]*\))?\s+(?:struct|enum|trait|type)\s+([A-Za-z_]\w*)")
    for path in rust_files(os.path.join(ROOT, "crates")):
        with open(path, encoding="utf-8") as f:
            for n, line in enumerate(f, 1):
                m = decl.match(line)
                if not m:
                    continue
                name = m.group(1)
                if name in banned:
                    problems.append(f"{rel(path)}:{n}: glossary forbids the name `{name}`")
                for part in BANNED_PARTS:
                    if re.search(rf"(^|[a-z]){part}($|[A-Z])", name):
                        problems.append(f"{rel(path)}:{n}: `{name}` uses banned word `{part}`")


def check_layers():
    src = os.path.join(CORE, "src")
    use = re.compile(r"\bcrate::(math|geom|topo|algo|model|ops)\b")
    for path in rust_files(src):
        parts = os.path.relpath(path, src).split(os.sep)
        if parts[0] not in LAYERS:
            continue
        own = LAYERS.index(parts[0])
        with open(path, encoding="utf-8") as f:
            for n, line in enumerate(f, 1):
                if line.lstrip().startswith("//"):
                    continue
                for m in use.finditer(line):
                    if LAYERS.index(m.group(1)) > own:
                        problems.append(
                            f"{rel(path)}:{n}: layer `{parts[0]}` imports `{m.group(1)}` (above it)"
                        )


def check_deps():
    path = os.path.join(CORE, "Cargo.toml")
    section = None
    with open(path, encoding="utf-8") as f:
        for line in f:
            s = line.strip()
            if s.startswith("["):
                section = s
                continue
            if section == "[dependencies]" and "=" in s and not s.startswith("#"):
                name = s.split("=")[0].strip().split(".")[0]
                if name not in CORE_DEPS:
                    problems.append(
                        f"crates/ferro-core/Cargo.toml: new dependency `{name}` needs an ADR (allow-list in scripts/lint.py)"
                    )


def check_tasks():
    path = os.path.join(ROOT, "docs", "tasks", "TREE.md")
    item = re.compile(r"^(\s*)- \[(.)\]\s+(.*)$")
    ident = re.compile(r"\b(M\d+(?:-\d+(?:\.\d+)*)?)\b")
    nodes = []
    seen = {}
    with open(path, encoding="utf-8") as f:
        for n, line in enumerate(f, 1):
            m = item.match(line)
            if not m:
                continue
            indent, mark, text = len(m.group(1)), m.group(2), m.group(3)
            if mark not in " ~x":
                problems.append(f"docs/tasks/TREE.md:{n}: unknown marker [{mark}]")
            if mark in "~x" and "@" not in text:
                problems.append(f"docs/tasks/TREE.md:{n}: claimed / done item has no @owner")
            i = ident.search(text.replace("*", ""))
            if i:
                key = i.group(1)
                if key in seen:
                    problems.append(f"docs/tasks/TREE.md:{n}: id {key} also on line {seen[key]}")
                seen[key] = n
            nodes.append((n, indent, mark))
    for k, (n, indent, mark) in enumerate(nodes):
        if mark != "x":
            continue
        for n2, indent2, mark2 in nodes[k + 1:]:
            if indent2 <= indent:
                break
            if mark2 != "x":
                problems.append(f"docs/tasks/TREE.md:{n}: done but child on line {n2} is not")
                break


def check_api():
    r = subprocess.run([sys.executable, os.path.join(ROOT, "scripts", "api_tree.py"), "--check"],
                       capture_output=True, text=True)
    if r.returncode != 0:
        problems.append(r.stdout.strip() or "docs/api/TREE.md is out of date")


def check_python():
    r = subprocess.run([sys.executable, os.path.join(ROOT, "scripts", "gen_python.py"), "--check"],
                       capture_output=True, text=True)
    if r.returncode != 0:
        problems.append(r.stdout.strip() or "python/ferro/_tools.py is out of date")


def main():
    check_glossary()
    check_layers()
    check_deps()
    check_tasks()
    check_api()
    check_python()
    for p in problems:
        print(p)
    if problems:
        print(f"lint: {len(problems)} problem(s)")
        return 1
    print("lint: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
