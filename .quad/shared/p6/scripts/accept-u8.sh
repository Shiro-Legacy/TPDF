#!/bin/zsh
# fable51 acceptance run for P6 U8 — run under the lock (one hold), OUTSIDE the sandbox (osascript proof). Contract §5 + §9.11–9.12.
set -u
R=$PWD; O=$R/.quad/shared/p6/acceptance; C=$R/.quad/shared/p6/checks; mkdir -p $O; cd $R
fail=0
echo "== tsc"; npx tsc --noEmit -p tsconfig.json && echo tsc-ok || fail=1
echo "== build"; npm run build >/dev/null 2>&1 && echo build-ok || fail=1
echo "== suite"; npm test 2>&1 | tee $O/u8-suite.log | grep -E '^# (tests|pass|fail|skipped)'; grep -q '^# fail 0$' $O/u8-suite.log || fail=1; grep -q '^# skipped 1$' $O/u8-suite.log || fail=1
p=$(grep -E '^# pass ' $O/u8-suite.log | awk '{print $3}'); newt=$(grep -cE '^ok [0-9]+ ' <(node --test tests/p6-text8.test.mjs tests/p6-drive9.test.mjs 2>/dev/null | grep -E '^ok [0-9]+ - ' ) ); echo "pass $p (need >= $((529+newt)), new tests $newt)"; [ "$p" -ge $((529+newt)) ] || fail=1
echo "== red-before-fix log (>= 3 not ok, naming a/b/e)"; n=$(grep -c "^not ok" $O/u8-red-before.log 2>/dev/null || echo 0); echo "not ok $n"; [ "$n" -ge 3 ] || fail=1; for c in "(a)" "(b)" "(e)"; do grep -q "^not ok.*$c" $O/u8-red-before.log || { echo "missing red $c"; fail=1; }; done
red() { node -e 'const r=JSON.parse(require("fs").readFileSync(process.argv[1],"utf8"));const s=r.steps||[];const red=s.filter(x=>x.expect&&x.ok===false).length;console.log(`steps ${s.length} red ${red}`);process.exit(red?1:0)' "$1" 2>/dev/null || { echo "report unparsable or red"; return 1; }; }
for i in 1 2 3; do echo "== drive direct-edit9 #$i"; node scripts/drive.mjs tests/drive/direct-edit9.jsonl > $O/u8-direct-edit9-$i.log 2>$O/u8-direct-edit9-$i.err; e=$?; echo "exit $e"; [ $e -eq 0 ] || fail=1; red $O/u8-direct-edit9-$i.log || fail=1; done
for s in direct-edit direct-edit2 direct-edit3 direct-edit4 direct-edit5 direct-edit6 direct-edit7 direct-edit7-save direct-edit8 direct-edit8-cut direct-edit8-save debt1 object-edit content-edit help-fields replace-page; do [ -f tests/drive/$s.jsonl ] || { echo "== drive $s (absent, skipped)"; continue; }; echo "== drive $s"; node scripts/drive.mjs tests/drive/$s.jsonl > $O/u8-$s.log 2>$O/u8-$s.err; e=$?; echo "exit $e"; [ $e -eq 0 ] || fail=1; red $O/u8-$s.log || fail=1; done
echo "== secrets in reports (must be none)"; if grep -l '"password"\|BEGIN PRIVATE KEY\|BEGIN RSA' $O/u8-*.log 2>/dev/null; then fail=1; else echo none; fi
echo "== fixtures.sha256 after drives (9 OK expected)"; shasum -a 256 -c $C/fixtures.sha256 | tee $O/u8-fixtures-check.out | grep -c ': OK$' | grep -qx 9 || fail=1
echo "== hashes (u8-hash-after must equal u8-hash-before, 55 lines)"; find fixtures -type f \( -name '*.pdf' -o -name '*.png' \) | sort | xargs shasum -a 256 > $C/u8-hash-after.txt; cmp $C/u8-hash-before.txt $C/u8-hash-after.txt && echo identical || fail=1
echo "== gui script (real osascript keystrokes)"; node $C/shot-direct9.mjs > $O/u8-shot.log 2>&1; e=$?; echo "exit $e"; tail -5 $O/u8-shot.log; [ $e -eq 0 ] || fail=1
echo "== gui shot"; ls -l $C/review-out/u8-a-twins.png 2>/dev/null || { echo "u8-a-twins.png missing"; fail=1; }
echo "== fixtures.sha256 after gui (9 OK expected)"; shasum -a 256 -c $C/fixtures.sha256 | grep -c ': OK$' | grep -qx 9 || fail=1
echo "== electron"; if pgrep -fl -- '--drive=' ; then fail=1; else echo none; fi
echo "== package"; npm run package >/dev/null 2>&1 && echo package-ok || fail=1
echo "== result: fail=$fail"; exit $fail
