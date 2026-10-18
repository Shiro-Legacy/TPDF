#!/bin/bash
# make-handoff.sh WS_LIST OUT_DIR [--done WS...] [--allow-stale]
#
# Builds a clean-room hand-off tree for the workstream(s) in WS_LIST (comma
# separated; 01 and 02 go together as 01,02) from the repository's HEAD, without history:
#   OUT_DIR/engine/     the engine, every class A file or part replaced by its
#                       interface stub (stubs/), except for --done workstreams
#   OUT_DIR/engine/tests/   only the workstreams' acceptance set, read from
#                       docs/clean-room/acceptance/{01-03,04-08}.md at HEAD
#                       (acceptance-files.py; fails when a list is missing)
#   OUT_DIR/fixtures/, OUT_DIR/assets/fonts/   test inputs, when the set needs
#                       them (handoff-manifest.txt)
#   OUT_DIR/specs/      SPEC-RULES.md, OWNER-DECISIONS.md, the workstreams'
#                       specs and the specs they build on (workstreams.txt)
#   OUT_DIR/HANDOFF.md  the implementer's rules, commands and tests
# then makes one git commit in OUT_DIR and runs check-handoff.sh on it.
#
# --done WS...   workstreams already rewritten and merged: HEAD's files are kept.
# --allow-stale  build although a partial stub was made from an older HEAD file.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=$(git -C "$here" rev-parse --show-toplevel)
usage() { echo "usage: $0 WS_LIST OUT_DIR [--done WS...] [--allow-stale]" >&2; exit 2; }

[ $# -ge 2 ] || usage
ws_list=$(echo "$1" | tr ',' ' ')
out=$2
shift 2
done_list=""
allow_stale=0
while [ $# -gt 0 ]; do
  case $1 in
    --done) shift
      while [ $# -gt 0 ] && [ "${1#--}" = "$1" ]; do done_list="$done_list $(echo "$1" | tr ',' ' ')"; shift; done ;;
    --allow-stale) allow_stale=1; shift ;;
    *) usage ;;
  esac
done

valid() { case $1 in 01|02|03|04|05|06|07|08) return 0 ;; esac; return 1; }
member() { local x=$1; shift; for y in "$@"; do [ "$x" = "$y" ] && return 0; done; return 1; }
for w in $ws_list $done_list; do valid "$w" || { echo "unknown workstream: $w (01..08)" >&2; exit 2; }; done
for w in $ws_list; do
  if member "$w" $done_list; then echo "workstream $w is both handed off and --done" >&2; exit 2; fi
done

# Owner decision: workstreams 01 and 02 are handed off as one tree.
for pair in "01 02" "02 01"; do
  set -- $pair
  if member "$1" $ws_list && ! member "$2" $ws_list $done_list; then
    echo "workstreams 01 and 02 go in one tree: use 01,02 (or --done $2)" >&2; exit 2
  fi
done

if [ -e "$out" ] && [ -n "$(ls -A "$out" 2>/dev/null)" ]; then
  echo "$out exists and is not empty" >&2; exit 2
fi

head=$(git -C "$repo" rev-parse HEAD)
map="$here/workstreams.txt"
entries=$(grep -E '^0[1-8] ' "$map")

# Specs: the workstreams' own and their dependencies, transitively.
spec_ws=""
add_spec() {
  member "$1" $spec_ws && return 0
  spec_ws="$spec_ws $1"
  local deps
  deps=$(awk -v w="$1" '$1 == "deps" && $2 == w { for (i = 3; i <= NF; i++) print $i }' "$map")
  for d in $deps; do add_spec "$d"; done
}
for w in $ws_list; do add_spec "$w"; done

# Stubs made from an older HEAD file would carry stale surviving code.
stale=0
while read -r w path action; do
  member "$w" $done_list && continue
  [ "$action" = new ] && continue
  recorded=$(awk -v p="$path" '$2 == p { print $1 }' "$here/stubs.base")
  now=$(git -C "$repo" rev-parse -q --verify "HEAD:engine/$path" 2>/dev/null || echo none)
  if [ "$recorded" != "$now" ]; then
    if [ "$action" = partial ]; then
      echo "STALE: engine/$path changed since its partial stub was made; regenerate stubs/$path and stubs.base" >&2
      stale=1
    else
      echo "note: engine/$path changed since its stub was made (stub replaces the whole file)" >&2
    fi
  fi
done <<< "$entries"
if [ $stale = 1 ] && [ $allow_stale = 0 ]; then exit 1; fi

# The workstreams' acceptance set (docs/clean-room/acceptance/*.md): the test
# files, support modules and data the tree keeps. Fails when a list is missing.
accept_set=$(python3 -I "$here/acceptance-files.py" "$repo" "$head" $ws_list) || exit 1

# Archive HEAD through the manifest.
manifest="$here/handoff-manifest.txt"
includes=() keeps=() keep_excludes=() placeholders=()
while read -r verb path; do
  case $verb in
    include) includes+=("$path") ;;
    exclude) includes+=(":(exclude)$path"); keep_excludes+=(":(exclude)$path") ;;
    exclude-glob) includes+=(":(exclude,glob)$path"); keep_excludes+=(":(exclude,glob)$path") ;;
    keep) keeps+=("$path") ;;
    placeholder) placeholders+=("$path") ;;
    ''|'#'*) ;;
    *) echo "bad manifest line: $verb $path" >&2; exit 2 ;;
  esac
done < <(grep -vE '^\s*(#|$)' "$manifest")

mkdir -p "$out"
out=$(cd "$out" && pwd)
git -C "$repo" archive --format=tar "$head" -- "${includes[@]}" | tar -x -C "$out"
if [ ${#keeps[@]} -gt 0 ]; then
  git -C "$repo" archive --format=tar "$head" -- "${keeps[@]}" ${keep_excludes[@]+"${keep_excludes[@]}"} | tar -x -C "$out"
fi

# Tests: only the acceptance set. Reference data it names comes back from HEAD.
kept() {
  local f=$1 k
  while read -r k; do
    [ -n "$k" ] || continue
    # shellcheck disable=SC2053
    if [ "$f" = "$k" ] || [[ $f == $k ]] || [ "${f#"$k"/}" != "$f" ]; then return 0; fi
  done <<< "$accept_set"
  return 1
}
while read -r f; do
  rel=${f#"$out"/}
  kept "$rel" || rm -f "$f"
done < <(find "$out/engine/tests" -type f)
find "$out/engine/tests" -type d -empty -delete
refs=$(printf '%s\n' "$accept_set" | grep '^engine/reference/' || true)
if [ -n "$refs" ]; then
  # shellcheck disable=SC2086
  git -C "$repo" archive --format=tar "$head" -- $refs | tar -x -C "$out"
fi
kept fixtures || rm -rf "$out/fixtures"
grep -rqs 'assets/' "$out/engine/tests" || rm -rf "$out/assets"
# A placeholder only where a kept file names it.
for p in "${placeholders[@]+"${placeholders[@]}"}"; do
  if grep -rqsF "$(basename "$p")" "$out/engine/tests" "$out/engine/src"; then
    mkdir -p "$out/$(dirname "$p")"; : > "$out/$p"
  fi
done

# Class A files: stub, leave out, or keep HEAD's for --done workstreams.
own_files="" other_files=""
while read -r w path action; do
  member "$w" $done_list && continue
  target="$out/engine/$path"
  case $action in
    stub|partial|new) mkdir -p "$(dirname "$target")"; cp "$here/stubs/$path" "$target" ;;
    delete) rm -f "$target" ;;
  esac
  if member "$w" $ws_list; then own_files="$own_files $w:$path:$action"; else other_files="$other_files $w:$path:$action"; fi
done <<< "$entries"

# engine/README.md: HEAD's when it says nothing of the oracle, else a neutral
# one. .gitignore loses lines that name the oracle.
readme=$(git -C "$repo" show "$head:engine/README.md" 2>/dev/null || true)
if [ -n "$readme" ] && ! printf '%s' "$readme" | grep -qi mupdf; then
  printf '%s\n' "$readme" > "$out/engine/README.md"
else
  cat > "$out/engine/README.md" <<'EOF'
# Tandem PDF engine

A safe Rust PDF engine: syntax, documents, filters, repair, view (rendering
and structured text through the vendored hayro crates) and the app layer
behind the Tandem PDF desktop and mobile apps. `bridge/` is the WASM export.

Build and test from this directory with cargo; see ../HANDOFF.md.
EOF
fi
if [ -f "$out/engine/.gitignore" ]; then
  grep -vi mupdf "$out/engine/.gitignore" > "$out/engine/.gitignore.new" || true
  mv "$out/engine/.gitignore.new" "$out/engine/.gitignore"
fi

# Specs.
mkdir -p "$out/specs"
for f in SPEC-RULES.md OWNER-DECISIONS.md; do
  git -C "$repo" show "$head:docs/clean-room/$f" > "$out/specs/$f"
done
for w in $spec_ws; do
  name=$(git -C "$repo" ls-tree --name-only "$head" docs/clean-room/specs/ | grep "/$w-" || true)
  [ -n "$name" ] || { echo "no spec for workstream $w at HEAD" >&2; exit 1; }
  git -C "$repo" show "$head:$name" > "$out/specs/$(basename "$name")"
done

# Acceptance tests the implementer un-ignores: #[ignore = "..."] reasons that
# start "clean-room: NN", "clean-room: spec NN" or "spec NN".
accept=""
for w in $ws_list; do
  n=${w#0}
  found=$(cd "$out/engine" && grep -rnE --include='*.rs' "ignore *= *\"(clean-room: *(spec *)?|spec *)0?$n([^0-9]|\")" src tests examples 2>/dev/null \
    | while IFS=: read -r file line _; do
        name=$(awk -v l="$line" 'NR > l && /fn [A-Za-z0-9_]+/ { sub(/.*fn /, ""); sub(/[(<].*/, ""); print; exit }' "$file")
        reason=$(sed -n "${line}p" "$file" | sed -E 's/.*ignore *= *"([^"]*)".*/\1/')
        echo "- \`engine/$file\` \`$name\`: $reason"
      done || true)
  accept="$accept$found"$'\n'
done
accept=$(printf '%s\n' "$accept" | sed '/^$/d' | sort -u)
[ -n "$accept" ] || accept="- (none marked yet; ask the reviewer for the acceptance list)"

spec_names=$(cd "$out/specs" && ls | sed 's/^/- `specs\//; s/$/`/')
fmt_files() {
  for e in $1; do
    w=${e%%:*}; rest=${e#*:}; path=${rest%:*}; action=${rest##*:}
    case $action in
      stub) echo "- \`engine/$path\` (workstream $w): the whole file" ;;
      partial) echo "- \`engine/$path\` (workstream $w): the items that \`todo!()\`; the rest of the file stays" ;;
      new) echo "- \`engine/$path\` (workstream $w): a new module" ;;
      delete) echo "- \`engine/$path\` (workstream $w): generated data, absent here; regenerate it from the public source the spec names" ;;
    esac
  done
}
own=$(fmt_files "$own_files")
other=$(fmt_files "$other_files")
[ -n "$other" ] || other="- (none)"
slot=/Users/owner/Documents/tpdf-wt/tools/slot

cat > "$out/HANDOFF.md" <<EOF
# Clean-room hand-off: workstream $(echo $ws_list | sed 's/ /, /g')

You implement the files below from the specs in \`specs/\`, public standards
and public data only.

## Rules

1. Read only this directory, the specs in \`specs/\`, public standards (ISO
   32000-2, TIFF 6.0, PNG, EXIF, Unicode, OpenType, the Adobe technical
   notes) and public data sources the specs name.
2. Never read MuPDF source code, the Tandem PDF repository or any copy of it,
   its git history, other working directories on this machine, or the
   \`~/.codex\` worktrees. Do not search the web for MuPDF code, and do not
   run MuPDF or mupdf.js: comparing with other viewers is the reviewer's job.
3. Record every external source you use (standard, data file, web page,
   crate documentation) in \`SOURCES.md\` at the top of this directory: what,
   where from, and what you used it for.
4. Change only the files listed under "Your files", plus \`Cargo.toml\` when
   a spec allows a new dependency, and \`SOURCES.md\`. If something else must
   change, write it down in \`NOTES.md\` instead.
5. No \`unsafe\`, no panics on any input, the limits each spec sets.
6. Stubs keep the public interface the rest of the engine calls. Keep every
   public name, type and signature unless the spec changes it. Private items
   are yours to design.

## Your files

$own

## Stubs of other workstreams (leave them)

These belong to later workstreams. Their bodies are \`todo!()\`, so tests that
reach them fail at run time; that is expected.

$other

## Specs

$spec_names

\`OWNER-DECISIONS.md\` records the owner's choices on the specs' open
questions; where it differs from a spec's default, it wins.

## Build and test

Run every cargo command through the slot tool (at most two heavy jobs run on
this machine at once):

\`\`\`sh
cd engine
$slot cargo check --all-targets
$slot cargo test --test <file>          # one tests/<file>.rs
$slot cargo test --lib <module>::       # unit tests in src/
$slot cargo clippy --all-targets
\`\`\`

The first build compiles the dependencies from scratch.

## Acceptance tests

\`engine/tests/\` holds only the test files of your workstreams' acceptance
set. Remove the \`#[ignore = "clean-room: ..."]\` attribute from each test below and
make it pass, together with the existing tests the spec's acceptance section
keeps:

$accept

Done means: \`cargo test\` passes for the tests your spec lists, \`cargo clippy
--all-targets\` is clean for your files, and \`SOURCES.md\` lists what you read.
EOF

# One commit, no history.
(
  cd "$out"
  git init -q
  git add -A
  git -c user.name="clean-room hand-off" -c user.email="handoff@invalid" \
    commit -q -m "Hand-off tree for workstream $(echo $ws_list | sed 's/ /, /g')"
)

size=$(du -sh "$out" | cut -f1)
files=$(cd "$out" && git ls-files | wc -l | tr -d ' ')
echo "made $out from $(git -C "$repo" rev-parse --short "$head"): $files files, $size"
done_args=()
[ -n "${done_list// /}" ] && done_args=(--done $done_list)
"$here/check-handoff.sh" "$out" "${done_args[@]+"${done_args[@]}"}"
