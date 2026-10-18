#!/bin/bash
# check-handoff.sh OUT_DIR [--done WS...] [--baseline REV]
#
# Fails when a hand-off tree (make-handoff.sh) leaks what the clean room keeps
# out:
#   - text naming the oracle or its internals: mupdf (any case), artifex,
#     fz_*, pdf_<word>_*, the .quad agent directory, the repository path,
#     other worktrees (tpdf-wt/, .codex/);
#   - a byte-identical copy of a class A file (workstreams.txt), as of the
#     baseline commit (before any rewrite) or of HEAD for workstreams not
#     --done.
# Hits listed in check-allowlist.txt (path glob | line regex | justification)
# pass. It also warns about long lines of the old class A code (lines its
# stubs removed that occur nowhere else in the baseline engine) found anywhere
# in the tree, for the reviewer to judge.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=$(git -C "$here" rev-parse --show-toplevel)
[ $# -ge 1 ] || { echo "usage: $0 OUT_DIR [--done WS...] [--baseline REV]" >&2; exit 2; }
out=$(cd "$1" && pwd)
shift
done_list=""
# The last commit before any clean-room rewrite landed.
baseline=af83162
while [ $# -gt 0 ]; do
  case $1 in
    --done) shift
      while [ $# -gt 0 ] && [ "${1#--}" = "$1" ]; do done_list="$done_list $(echo "$1" | tr ',' ' ')"; shift; done ;;
    --baseline) baseline=$2; shift 2 ;;
    *) echo "unknown argument $1" >&2; exit 2 ;;
  esac
done

# Old class A files: baseline versions, and HEAD's for workstreams not done.
old=$(mktemp -d)
trap 'rm -rf "$old"' EXIT
i=0
while read -r w path action; do
  [ "$action" = new ] && continue
  for rev in "$baseline" HEAD; do
    if [ "$rev" = HEAD ]; then
      for d in $done_list; do [ "$d" = "$w" ] && continue 2; done
    fi
    i=$((i + 1))
    if git -C "$repo" show "$rev:engine/$path" > "$old/$i.rs" 2>/dev/null; then
      printf '%s\t%s\t%s\n' "$i.rs" "$rev:engine/$path" "$here/stubs/$path" >> "$old/index"
    fi
  done
done < <(grep -E '^0[1-8] ' "$here/workstreams.txt")

# The baseline engine, to tell distinctive class A lines from common ones.
mkdir -p "$old/base"
git -C "$repo" archive --format=tar "$baseline" -- engine/src engine/tests engine/examples engine/bridge/src engine/vendor \
  | tar -x -C "$old/base"
grep -E '^0[1-8] ' "$here/workstreams.txt" | awk '{ print "engine/" $2 }' > "$old/apaths"

python3 -I - "$out" "$old" "$here/check-allowlist.txt" <<'PY'
import fnmatch, hashlib, os, re, sys

out, old, allow_path = sys.argv[1:4]
patterns = [
    ("mupdf", re.compile(r"mupdf", re.I)),
    ("artifex", re.compile(r"artifex", re.I)),
    ("fz_", re.compile(r"(?<![A-Za-z0-9_])fz_")),
    ("pdf_*_", re.compile(r"(?<![A-Za-z0-9_])pdf_[a-z]+_")),
    (".quad", re.compile(r"(?<![A-Za-z0-9_)\]])\.quad(?![A-Za-z0-9_])")),
    ("repo path", re.compile(r"/Users/owner/Documents/TPDF")),
    ("worktree", re.compile(r"tpdf-wt/|\.codex/")),
]
rules = []
with open(allow_path) as f:
    for n, line in enumerate(f, 1):
        line = line.rstrip("\n")
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        parts = [p.strip() for p in line.split(" | ", 2)]
        if len(parts) != 3 or not parts[2]:
            sys.exit(f"check-allowlist.txt:{n}: need 'path glob | line regex | justification'")
        rules.append((parts[0], re.compile(parts[1]), n))
used = set()

def allowed(rel, text):
    for glob, rx, n in rules:
        if fnmatch.fnmatch(rel, glob) and rx.search(text):
            used.add(n)
            return True
    return False

files = []
for root, dirs, names in os.walk(out):
    dirs[:] = [d for d in dirs if not (root == out and d == ".git")]
    for name in names:
        path = os.path.join(root, name)
        files.append((os.path.relpath(path, out), path))
files.sort()

failures = []
for rel, path in files:
    data = open(path, "rb").read()
    binary = b"\0" in data[:8192]
    text = data.decode("utf-8", "replace")
    for n, line in enumerate(text.splitlines(), 1):
        for label, rx in patterns:
            if rx.search(line) and not allowed(rel, line):
                shown = "(binary)" if binary else line.strip()[:160]
                failures.append(f"{rel}:{n}: [{label}] {shown}")

# Byte-identical copies of old class A files.
index = [l.split("\t") for l in open(os.path.join(old, "index")).read().splitlines()]
digest = {}
removed = set()
kept = set()
for name, origin, stub in index:
    data = open(os.path.join(old, name), "rb").read()
    digest.setdefault(hashlib.sha256(data).hexdigest(), origin)
    if os.path.exists(stub):
        kept |= {l.strip() for l in open(stub, encoding="utf-8", errors="replace")}
    for l in data.decode("utf-8", "replace").splitlines():
        if len(l.strip()) >= 50:
            removed.add(l.strip())
removed -= kept
for rel, path in files:
    h = hashlib.sha256(open(path, "rb").read()).hexdigest()
    if h in digest:
        failures.append(f"{rel}: [identical] byte-identical to {digest[h]}")

# Lines that also occur outside class A at the baseline are not distinctive.
apaths = set(open(os.path.join(old, "apaths")).read().split())
base = os.path.join(old, "base")
for root, _, names in os.walk(base):
    for name in names:
        path = os.path.join(root, name)
        if os.path.relpath(path, base) in apaths:
            continue
        for l in open(path, encoding="utf-8", errors="replace"):
            removed.discard(l.strip())

warnings = {}
for rel, path in files:
    if not rel.endswith(".rs"):
        continue
    for l in open(path, encoding="utf-8", errors="replace"):
        if l.strip() in removed:
            warnings[rel] = warnings.get(rel, 0) + 1

for f in failures:
    print("LEAK", f)
for rel, count in sorted(warnings.items()):
    print(f"warn {rel}: {count} long line(s) equal to old class A code")
unused = [n for _, _, n in rules if n not in used]
if unused:
    print("note: allowlist lines not needed for this tree:", ", ".join(map(str, unused)))
print(f"check-handoff: {len(files)} files, {len(failures)} leak(s), {len(warnings)} file(s) with old lines")
sys.exit(1 if failures else 0)
PY
