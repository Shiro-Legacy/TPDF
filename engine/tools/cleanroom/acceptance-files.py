#!/usr/bin/env python3
"""acceptance-files.py REPO REV WS...

Prints the repository paths a hand-off tree keeps for the workstreams' tests,
read at REV from the acceptance ledgers:

  docs/clean-room/acceptance/01-03.md, section "## Hand-off tree ...":
    "- NN: `a.rs`, ..."         test files under engine/tests/;
    "Reference data" bullets    files under engine/reference/.
  docs/clean-room/acceptance/04-08.md, section "### Hand-off tree (workstream N)":
    every backticked engine test file or data path before "Reviewer side
    only" (acceptance files and the ones that must stay green); node tests
    are not part of the engine tree.

Then adds what the kept tests use: support modules (`#[path = ...]`,
`mod NAME;`), files they `include!`, files under engine/tests/data/ they name, and fixtures/ when
they read ../fixtures. Exits 1, naming the ledger, when a workstream's list
is missing.
"""
import os
import re
import subprocess
import sys

repo, rev, *wss = sys.argv[1:]
if not wss:
    sys.exit("usage: acceptance-files.py REPO REV WS...")


def show(path):
    done = subprocess.run(["git", "-C", repo, "show", f"{rev}:{path}"],
                          capture_output=True, text=True)
    return done.stdout if done.returncode == 0 else None


def ls(prefix):
    done = subprocess.run(["git", "-C", repo, "ls-tree", "-r", "--name-only", rev, prefix],
                          capture_output=True, text=True, check=True)
    return done.stdout.split()


def bullets(section):
    """Top-level bullets ("- ...") of a section, indented continuations joined."""
    out, open_ = [], False
    for line in section.splitlines():
        if line.startswith("- "):
            out.append(line[2:])
            open_ = True
        elif open_ and line.startswith("  ") and line.strip():
            out[-1] += " " + line.strip()
        else:
            open_ = False
    return out


def ledger_01_03(text, ws, path):
    m = re.search(r"^## Hand-off tree.*?$(.*?)(?=^## |\Z)", text, re.M | re.S)
    if not m:
        sys.exit(f"{path} has no '## Hand-off tree' section (workstream {ws})")
    section = m.group(1)
    items = bullets(section)
    own = [b for b in items if re.match(rf"0?{int(ws)}:", b)]
    if not own:
        sys.exit(f"{path} lists no hand-off test files for workstream {ws} ('- {ws}: ...')")
    keep = set()
    for b in own:
        for token in re.findall(r"`([^`]+)`", b):
            if token.endswith(".rs") and "/" not in token:
                keep.add(f"engine/tests/{token}")
    in_reference = False
    for line in section.splitlines():
        if line.startswith("Reference data"):
            in_reference = True
        elif line.startswith(("Test files", "Shared")):
            in_reference = False
        if in_reference and line.startswith("- "):
            for token in re.findall(r"`([^`]+)`", line):
                if "/" in token and "." in token.rsplit("/", 1)[-1]:
                    keep.add(token if token.startswith("engine/") else f"engine/reference/{token}")
    # "Shared support" (support modules, tests/data, fixtures) is added below
    # only where the kept tests use it.
    return keep


def ledger_04_08(text, ws, path):
    m = re.search(rf"^### Hand-off tree \(workstream 0?{int(ws)}\)\s*$(.*?)(?=^#{{2,3}} |^<!--|\Z)",
                  text, re.M | re.S)
    if not m:
        sys.exit(f"{path} has no '### Hand-off tree (workstream {int(ws)})' section")
    section = m.group(1).split("Reviewer side only")[0]
    keep = set()
    for token in re.findall(r"`([^`]+)`", section):
        if token.startswith("fixtures/"):
            keep.add("fixtures")
        elif token.startswith("engine/tests/"):
            keep.add(token)
        elif "/" not in token and token.endswith(".rs"):
            keep.add(f"engine/tests/{token}")
    if not any(k.endswith(".rs") for k in keep):
        sys.exit(f"{path} lists no engine test files for workstream {ws}")
    return keep


keep = set()
for ws in wss:
    path = "docs/clean-room/acceptance/" + ("01-03.md" if int(ws) <= 3 else "04-08.md")
    text = show(path)
    if text is None:
        sys.exit(f"no acceptance list for workstream {ws}: {path} is not in {rev}")
    keep |= (ledger_01_03 if int(ws) <= 3 else ledger_04_08)(text, ws, path)

# Expand globs (`app_redact*.rs`) against REV; note names that do not exist.
files = set(ls("engine/tests")) | set(ls("engine/reference"))
expanded = set()
for k in keep:
    if "*" in k:
        rx = re.compile("^" + re.escape(k).replace(r"\*", "[^/]*") + "$")
        expanded |= {f for f in files if rx.match(f)}
    elif k == "fixtures" or k in files:
        expanded.add(k)
    else:
        print(f"note: {k} (acceptance ledger) is not in {rev}", file=sys.stderr)
keep = expanded

# What the kept tests use.
data_files = [f[len("engine/tests/data/"):] for f in files if f.startswith("engine/tests/data/")]
changed = True
while changed:
    changed = False
    for t in sorted(k for k in keep if k.endswith(".rs") and k.startswith("engine/tests/")):
        src = show(t) or ""
        here = t.rsplit("/", 1)[0]
        wanted = set()
        for p in re.findall(r'#\[path\s*=\s*"([^"]+)"\]', src):
            wanted.add(f"{here}/{p}")
        for p in re.findall(r'include(?:_str|_bytes)?!\(\s*"([^"]+)"', src):
            wanted.add(os.path.normpath(f"{here}/{p}"))
        for name in re.findall(r"^\s*(?:pub\s+)?mod\s+(\w+)\s*;", src, re.M):
            wanted |= {f"{here}/{name}.rs", f"{here}/{name}/mod.rs"}
        literals = [l for l in re.findall(r'"([^"{}]{4,})"', src)]
        for lit in re.findall(r'"([^"]*tests/data/[^"]*)"', src):
            rel = lit.split("tests/data/", 1)[1]
            rx = re.compile("^" + re.sub(r"\\\{[^}]*\\\}", "[^/]*", re.escape(rel)) + "$")
            for f in data_files:
                # A name built at run time ("{name}") must also appear as a
                # literal piece somewhere in the test.
                if rx.match(f) and ("{" not in rel or any(l in f for l in literals)):
                    wanted.add(f"engine/tests/data/{f}")
        if "../fixtures" in src or '"fixtures/' in src:
            wanted.add("fixtures")
        for w in wanted:
            # engine/reference/ comes only from the ledger.
            if (w == "fixtures" or (w in files and w.startswith("engine/tests/"))) and w not in keep:
                keep.add(w)
                changed = True

print("\n".join(sorted(keep)))
