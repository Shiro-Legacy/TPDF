#!/bin/zsh
# fable51 acceptance run for P6 U7 — run under the lock (one hold). Contract §6.3, §9.51 (fail flag).
set -u
R=$PWD; O=$R/.quad/shared/p6/acceptance; C=$R/.quad/shared/p6/checks; mkdir -p $O; cd $R
fail=0
echo "== tsc"; npx tsc --noEmit -p tsconfig.json && echo tsc-ok || fail=1
echo "== build"; npm run build >/dev/null 2>&1 && echo build-ok || fail=1
echo "== suite"; npm test 2>&1 | tee $O/u7-suite.log | grep -E '^# (tests|pass|fail|skipped)'; grep -q '^# fail 0$' $O/u7-suite.log || fail=1
# exit 0 is the gate; the red count parses the JSON report (steps with an expect and ok:false)
red() { node -e 'const r=JSON.parse(require("fs").readFileSync(process.argv[1],"utf8"));const s=r.steps||[];const red=s.filter(x=>x.expect&&x.ok===false).length;console.log(`steps ${s.length} red ${red}`);process.exit(red?1:0)' "$1" 2>/dev/null || { echo "report unparsable or red"; return 1; }; }
for i in 1 2 3; do echo "== drive direct-edit8 #$i"; node scripts/drive.mjs tests/drive/direct-edit8.jsonl > $O/u7-direct-edit8-$i.log 2>$O/u7-direct-edit8-$i.err; e=$?; echo "exit $e"; [ $e -eq 0 ] || fail=1; red $O/u7-direct-edit8-$i.log || fail=1; done
for s in direct-edit direct-edit2 direct-edit3 direct-edit4 direct-edit5 direct-edit6 direct-edit7 direct-edit7-save debt1 object-edit content-edit help-fields; do echo "== drive $s"; node scripts/drive.mjs tests/drive/$s.jsonl > $O/u7-$s.log 2>$O/u7-$s.err; e=$?; echo "exit $e"; [ $e -eq 0 ] || fail=1; red $O/u7-$s.log || fail=1; done
echo "== secrets in reports (must be none)"; if grep -l '"password"\|BEGIN PRIVATE KEY\|BEGIN RSA' $O/u7-*.log 2>/dev/null; then fail=1; else echo none; fi
echo "== fixtures.sha256 after drives (9 OK expected)"; shasum -a 256 -c $C/fixtures.sha256 | tee $O/u7-fixtures-check.out | grep -c ': OK$' | grep -qx 9 || fail=1
echo "== hashes (u7-hash-after must equal u7-hash-before, 55 lines)"; find fixtures -type f \( -name '*.pdf' -o -name '*.png' \) | sort | xargs shasum -a 256 > $C/u7-hash-after.txt; cmp $C/u7-hash-before.txt $C/u7-hash-after.txt && echo identical || fail=1
echo "== D4 engine tests (>= 12 expected)"; n=$(grep -cE '^ok [0-9]+ - D4 ' $O/u7-suite.log); echo "D4 ok $n"; [ "$n" -ge 12 ] || fail=1
echo "== gui script"; node $C/shot-direct8.mjs > $O/u7-shot.log 2>&1 && echo shot-ok || fail=1
echo "== gui shots (view each with Read)"; for l in a b c e; do ls $C/review-out/u7-$l-*.png >/dev/null 2>&1 && echo "u7-$l ok" || { echo "u7-$l missing"; fail=1; }; done; ls -l $C/review-out/u7-*.png 2>/dev/null
echo "== fixtures.sha256 after gui (9 OK expected)"; shasum -a 256 -c $C/fixtures.sha256 | grep -c ': OK$' | grep -qx 9 || fail=1
echo "== electron"; if pgrep -fl -- '--drive=' ; then fail=1; else echo none; fi
echo "== package"; npm run package >/dev/null 2>&1 && echo package-ok || fail=1
echo "== result: fail=$fail"; exit $fail
