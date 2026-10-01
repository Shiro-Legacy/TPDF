#!/bin/zsh
# fable51 acceptance run for P6 U5 — run under the lock (one hold). Contract §6.3 as amended by §9.35.
set -u
R=$PWD; O=$R/.quad/shared/p6/acceptance; C=$R/.quad/shared/p6/checks; mkdir -p $O; cd $R
echo "== tsc"; npx tsc --noEmit -p tsconfig.json && echo tsc-ok
echo "== build"; npm run build >/dev/null 2>&1 && echo build-ok
echo "== suite"; npm test 2>&1 | tee $O/u5-suite.log | grep -E '^# (tests|pass|fail|skipped)'
# U4 §9.21: exit 0 is the gate; the red count parses the JSON report (steps with an expect and ok:false) and is informational
red() { node -e 'const r=JSON.parse(require("fs").readFileSync(process.argv[1],"utf8"));const s=r.steps||[];console.log(`steps ${s.length} red ${s.filter(x=>x.expect&&x.ok===false).length}`)' "$1" 2>/dev/null || echo "report unparsable"; }
for i in 1 2 3; do echo "== drive direct-edit6 #$i"; node scripts/drive.mjs tests/drive/direct-edit6.jsonl > $O/u5-direct-edit6-$i.log 2>$O/u5-direct-edit6-$i.err; echo "exit $?"; red $O/u5-direct-edit6-$i.log; done
# §9.35: the three drives that exercise the changed driveDirectClick/driveDirectGesture path (direct-edit, direct-edit2, debt1) join the U3–U5 and panel drives
for s in direct-edit direct-edit2 direct-edit3 direct-edit4 direct-edit5 debt1 object-edit content-edit help-fields; do echo "== drive $s"; node scripts/drive.mjs tests/drive/$s.jsonl > $O/u5-$s.log 2>$O/u5-$s.err; echo "exit $?"; red $O/u5-$s.log; done
echo "== secrets in reports (must be none)"; grep -l '"password"\|BEGIN PRIVATE KEY\|BEGIN RSA' $O/u5-*.log 2>/dev/null || echo none
echo "== fixtures.sha256 after drives (8 OK expected)"; shasum -a 256 -c $C/fixtures.sha256
echo "== hashes"; find fixtures -type f \( -name '*.pdf' -o -name '*.png' \) | sort | xargs shasum -a 256 > $C/u5-hash-after.txt; echo "-- changed or missing baseline hashes (must be empty):"; diff $C/u5-hash-before.txt $C/u5-hash-after.txt | grep '^<' || echo none; echo "-- added (D2: must be empty):"; diff $C/u5-hash-before.txt $C/u5-hash-after.txt | grep '^>' || echo none
echo "== gui shots (view each with Read)"; ls -l $C/review-out/u5-*.png 2>/dev/null || echo "none (run node $C/shot-direct6.mjs first)"
echo "== electron"; pgrep -fl -- '--drive=' || echo none
