#!/bin/zsh
# fable51 acceptance run for P6 U4 — run under the lock (one hold)
set -u
R=$PWD; O=$R/.quad/shared/p6/acceptance; mkdir -p $O; cd $R
echo "== tsc"; npx tsc --noEmit -p tsconfig.json && echo tsc-ok
echo "== build"; npm run build >/dev/null 2>&1 && echo build-ok
echo "== suite"; npm test 2>&1 | tee $O/u4-suite.log | grep -E '^# (tests|pass|fail|skipped)'
# §9.21: exit 0 is the gate; the red count parses the JSON report (steps with an expect and ok:false) and is informational
red() { node -e 'const r=JSON.parse(require("fs").readFileSync(process.argv[1],"utf8"));const s=r.steps||[];console.log(`steps ${s.length} red ${s.filter(x=>x.expect&&x.ok===false).length}`)' "$1" 2>/dev/null || echo "report unparsable"; }
for s in direct-edit4 direct-edit5; do for i in 1 2 3; do echo "== drive $s #$i"; node scripts/drive.mjs tests/drive/$s.jsonl > $O/u4-$s-$i.log 2>$O/u4-$s-$i.err; echo "exit $?"; red $O/u4-$s-$i.log; done; done
for s in direct-edit direct-edit2 direct-edit3 links-bookmarks field-design help-fields; do echo "== drive $s"; node scripts/drive.mjs tests/drive/$s.jsonl > $O/u4-$s.log 2>$O/u4-$s.err; echo "exit $?"; red $O/u4-$s.log; done
echo "== fixtures.sha256 after drives (§9.21)"; shasum -a 256 -c $R/.quad/shared/p6/checks/fixtures.sha256
echo "== hashes"; find fixtures -type f \( -name '*.pdf' -o -name '*.png' \) | sort | xargs shasum -a 256 > $R/.quad/shared/p6/checks/u4-hash-after.txt; echo "-- changed or missing baseline hashes (must be empty):"; diff $R/.quad/shared/p6/checks/u4-hash-before.txt $R/.quad/shared/p6/checks/u4-hash-after.txt | grep '^<' || echo none; echo "-- added (expected: fixtures/p6/links.pdf, fixtures/p6/tabs.pdf only):"; diff $R/.quad/shared/p6/checks/u4-hash-before.txt $R/.quad/shared/p6/checks/u4-hash-after.txt | grep '^>' || echo none
echo "== electron"; pgrep -fl -- '--drive=' || echo none
