#!/bin/zsh
# fable51 acceptance run for P5 U3 — run under the lock (one hold). Contract §6.3 + §9.31.
set -u
R=$PWD; O=$R/.quad/shared/p5/acceptance; C=$R/.quad/shared/p5/checks; mkdir -p $O; cd $R
PY=$R/.venv-qa/bin/python
echo "== tsc"; npx tsc --noEmit -p tsconfig.json && echo tsc-ok
echo "== build"; npm run build >/dev/null 2>&1 && echo build-ok
echo "== suite"; npm test 2>&1 | tee $O/u3-suite.log | grep -E '^# (tests|pass|fail|skipped)'
# U4 §9.21: exit 0 is the gate; the red count parses the JSON report (steps with an expect and ok:false) and is informational
red() { node -e 'const r=JSON.parse(require("fs").readFileSync(process.argv[1],"utf8"));const s=r.steps||[];console.log(`steps ${s.length} red ${s.filter(x=>x.expect&&x.ok===false).length}`)' "$1" 2>/dev/null || echo "report unparsable"; }
for i in 1 2 3; do echo "== drive sign-field #$i"; node scripts/drive.mjs tests/drive/sign-field.jsonl --out=$O/u3-out-$i > $O/u3-sign-field-$i.log 2>$O/u3-sign-field-$i.err; echo "exit $?"; red $O/u3-sign-field-$i.log; done
for s in sign verify field-design direct-edit direct-edit5 help-fields; do echo "== drive $s"; node scripts/drive.mjs tests/drive/$s.jsonl > $O/u3-$s.log 2>$O/u3-$s.err; echo "exit $?"; red $O/u3-$s.log; done
echo "== secrets in reports (must be none)"; grep -l '"pdf4all"\|"password"' $O/u3-*.log || echo none
echo "== pyHanko on run 3 outputs"; for f in field-signed.pdf:1 field-signed2.pdf:2 field-signed3.pdf:3 hello-field.pdf:1; do p=${f%%:*}; n=${f##*:}; echo "-- $p (expect $n)"; $PY scripts/verify-sign.py $O/u3-out-3/$p --ca fixtures/p0b/.ca/ca.pem --expect-signatures $n; echo "exit $?"; done
echo "== fixtures.sha256 after drives"; shasum -a 256 -c $R/.quad/shared/p6/checks/fixtures.sha256
echo "== hashes"; find fixtures -type f \( -name '*.pdf' -o -name '*.png' \) | sort | xargs shasum -a 256 > $C/u3-hash-after.txt; echo "-- changed or missing baseline hashes (must be empty):"; diff $C/u3-hash-before.txt $C/u3-hash-after.txt | grep '^<' || echo none; echo "-- added (expected: fixtures/p5/empty-sigfield.pdf only):"; diff $C/u3-hash-before.txt $C/u3-hash-after.txt | grep '^>' || echo none
echo "== by-value pins"; shasum -a 256 fixtures/hello.pdf fixtures/p5/empty-sigfield.pdf
echo "== trust store"; ls -l "$HOME/Library/Application Support/pdf4all-p0/trusted-certs.pem" 2>/dev/null || echo absent; find $C/review-out -name trusted-certs.pem 2>/dev/null || true
echo "== electron"; pgrep -fl -- '--drive=' || echo none
