#!/bin/zsh
# fable51 generic acceptance run — run under the lock (one hold), OUTSIDE the sandbox (osascript proof).
# usage: accept-unit.sh <unit> <newDrives comma-list> <gui script under checks/> <png glob under checks/review-out/> <new test files comma-list, or - when new tests live in existing files> <baseline pass = expected # pass when NEWTESTS is ->
set -u
U=$1; NEW=$2; GUI=$3; PNG=$4; NEWTESTS=$5; BASE=$6
R=$PWD; O=$R/.quad/shared/p6/acceptance; C=$R/.quad/shared/p6/checks; mkdir -p $O; cd $R
fail=0
echo "== tsc"; npx tsc --noEmit -p tsconfig.json && echo tsc-ok || fail=1
echo "== build"; npm run build >/dev/null 2>&1 && echo build-ok || fail=1
echo "== suite"; npm test 2>&1 | tee $O/$U-suite.log | grep -E '^# (tests|pass|fail|skipped)'; grep -q '^# fail 0$' $O/$U-suite.log || fail=1; grep -q '^# skipped 1$' $O/$U-suite.log || fail=1
p=$(grep -E '^# pass ' $O/$U-suite.log | awk '{print $3}'); if [ "$NEWTESTS" = "-" ]; then newt=0; else newt=$(node --test ${(s:,:)NEWTESTS} 2>/dev/null | grep -cE '^ok [0-9]+ - '); fi; echo "pass $p (need >= $((BASE+newt)); new tests $newt)"; [ "$p" -ge $((BASE+newt)) ] || fail=1
echo "== red-before-fix log"; n=$(grep -c '^not ok' $O/$U-red-before.log 2>/dev/null || echo 0); echo "not ok $n"; [ "$n" -ge 1 ] || fail=1
# batch drives need staged folders (contract-batch.md §9.18): a fresh tmp dir with batch-in/ and batch-c/ copies of four fixtures, passed as --out=.
stage() { case $1 in batch*) d=$(mktemp -d /tmp/pdf4all-accept-XXXXXX); mkdir -p $d/batch-in $d/batch-c; cp fixtures/encrypted.pdf fixtures/hello.pdf fixtures/nav.pdf fixtures/rotated.pdf $d/batch-in/; cp fixtures/encrypted.pdf fixtures/hello.pdf fixtures/nav.pdf fixtures/rotated.pdf $d/batch-c/; echo "--out=$d";; *) echo "";; esac; }
red() { node -e 'const r=JSON.parse(require("fs").readFileSync(process.argv[1],"utf8"));const s=r.steps||[];const red=s.filter(x=>x.expect&&x.ok===false).length;console.log(`steps ${s.length} red ${red}`);process.exit(red?1:0)' "$1" 2>/dev/null || { echo "report unparsable or red"; return 1; }; }
for d in ${(s:,:)NEW}; do for i in 1 2 3; do echo "== drive $d #$i"; node scripts/drive.mjs tests/drive/$d.jsonl $(stage $d) > $O/$U-$d-$i.log 2>$O/$U-$d-$i.err; e=$?; echo "exit $e"; [ $e -eq 0 ] || fail=1; red $O/$U-$d-$i.log || fail=1; done; done
for s in direct-edit direct-edit2 direct-edit3 direct-edit4 direct-edit5 direct-edit6 direct-edit7 direct-edit7-save direct-edit8 direct-edit8-cut direct-edit8-save direct-edit9 direct-edit8-chords bates batch compare debt1 object-edit content-edit help-fields dialogs refusals replace-page; do [ -f tests/drive/$s.jsonl ] || continue; echo "== drive $s"; node scripts/drive.mjs tests/drive/$s.jsonl $(stage $s) > $O/$U-$s.log 2>$O/$U-$s.err; e=$?; echo "exit $e"; [ $e -eq 0 ] || fail=1; red $O/$U-$s.log || fail=1; done
echo "== secrets in reports (must be none)"; if grep -l '"password"\|BEGIN PRIVATE KEY\|BEGIN RSA' $O/$U-*.log 2>/dev/null; then fail=1; else echo none; fi
echo "== fixtures.sha256 after drives (9 OK expected)"; shasum -a 256 -c $C/fixtures.sha256 | tee $O/$U-fixtures-check.out | grep -c ': OK$' | grep -qx 9 || fail=1
echo "== hashes ($U-hash-after must equal $U-hash-before, 55 lines)"; find fixtures -type f \( -name '*.pdf' -o -name '*.png' \) | sort | xargs shasum -a 256 > $C/$U-hash-after.txt; cmp $C/$U-hash-before.txt $C/$U-hash-after.txt && echo identical || fail=1
echo "== nothing new under fixtures"; [ $(find fixtures -type d -name 'pdf4all-out' | wc -l) -eq 0 ] && echo none || fail=1
echo "== gui script (real osascript keystrokes)"; node $C/$GUI > $O/$U-shot.log 2>&1; e=$?; echo "exit $e"; tail -5 $O/$U-shot.log; [ $e -eq 0 ] || fail=1
echo "== gui shot"; ls -l $C/review-out/${~PNG} 2>/dev/null || { echo "$PNG missing"; fail=1; }
echo "== fixtures.sha256 after gui (9 OK expected)"; shasum -a 256 -c $C/fixtures.sha256 | grep -c ': OK$' | grep -qx 9 || fail=1
echo "== electron"; if pgrep -fl -- '--drive=' ; then fail=1; else echo none; fi
echo "== package"; npm run package >/dev/null 2>&1 && echo package-ok || fail=1
rm -rf /tmp/pdf4all-accept-* # batch staging dirs from stage()
echo "== result: fail=$fail"; exit $fail
