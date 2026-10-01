#!/bin/zsh
# fable51 acceptance run for P6 U6 — run under the lock (one hold). Contract §6.3.
set -u
R=$PWD; O=$R/.quad/shared/p6/acceptance; C=$R/.quad/shared/p6/checks; mkdir -p $O; cd $R
echo "== tsc"; npx tsc --noEmit -p tsconfig.json && echo tsc-ok
echo "== build"; npm run build >/dev/null 2>&1 && echo build-ok
echo "== suite"; npm test 2>&1 | tee $O/u6-suite.log | grep -E '^# (tests|pass|fail|skipped)'
# exit 0 is the gate; the red count parses the JSON report (steps with an expect and ok:false) and is informational
red() { node -e 'const r=JSON.parse(require("fs").readFileSync(process.argv[1],"utf8"));const s=r.steps||[];console.log(`steps ${s.length} red ${s.filter(x=>x.expect&&x.ok===false).length}`)' "$1" 2>/dev/null || echo "report unparsable"; }
for i in 1 2 3; do echo "== drive direct-edit7 #$i"; node scripts/drive.mjs tests/drive/direct-edit7.jsonl > $O/u6-direct-edit7-$i.log 2>$O/u6-direct-edit7-$i.err; echo "exit $?"; red $O/u6-direct-edit7-$i.log; done
for s in direct-edit direct-edit2 direct-edit3 direct-edit4 direct-edit5 direct-edit6 debt1 object-edit content-edit help-fields; do echo "== drive $s"; node scripts/drive.mjs tests/drive/$s.jsonl > $O/u6-$s.log 2>$O/u6-$s.err; echo "exit $?"; red $O/u6-$s.log; done
echo "== secrets in reports (must be none)"; grep -l '"password"\|BEGIN PRIVATE KEY\|BEGIN RSA' $O/u6-*.log 2>/dev/null || echo none
echo "== fixtures.sha256 after drives (9 OK expected)"; shasum -a 256 -c $C/fixtures.sha256
echo "== hashes"; find fixtures -type f \( -name '*.pdf' -o -name '*.png' \) | sort | xargs shasum -a 256 > $C/u6-hash-after.txt; echo "-- changed or missing baseline hashes (must be empty):"; diff $C/u6-hash-before.txt $C/u6-hash-after.txt | grep '^<' || echo none; echo "-- added (D3: exactly fixtures/p6/objects6.pdf):"; diff $C/u6-hash-before.txt $C/u6-hash-after.txt | grep '^>' || echo none
echo "== gui shots (view each with Read)"; ls -l $C/review-out/u6-*.png 2>/dev/null || echo "none (run node $C/shot-direct7.mjs first)"
echo "== electron"; pgrep -fl -- '--drive=' || echo none
