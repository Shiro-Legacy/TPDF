# P5 U3 objections — tests / drive / evidence lens (refute-first) — 4 blocking, 8 amend, 4 nit
Read-only review, 2026-09-24. Paths are relative to the repo root. Nothing ran (no Electron, build or test).

**T1** severity: blocking
claim: §2.6/§4.2 "the report never contains the password" cannot pass, because every drive report contains the substring `pdf4all` in paths.
evidence: src/main.ts:1742 `await finish({ out, steps, memory: metrics() }, !failed);` and tests/p6-drive4.test.mjs:20 `mkdtemp(join(tmpdir(), 'pdf4all-drive-test-'))`. Also every expanded `to`/`path` and the repo root `/Users/…/pdf4all`.
amendment: §4.2: "asserts `!stdout.includes('"password"')` and `!stdout.includes('"pdf4all"')` (a quoted JSON value), every `sign` step has `contents === '[redacted]'`, and no step has a `password` key. It never tests the bare substring (paths hold `pdf4all`). The same applies to STATE: the leak check (main.ts:1698) is a substring test, and STATE holds no paths (renderer.ts:6557)."

**T2** severity: blocking
claim: §1.3 lets `field-props`/`field-delete` act on an empty `/Sig` widget. `fixtures/form2.pdf`'s `sig` is empty (obj 18 has no `/V`), so four pinned refusals turn into successes or new messages. Three of those files are also forbidden or unowned.
evidence: tests/drive/field-design.jsonl:25 `"props","name":"sig","rename":"signed"} … "detail":"signature"` with tests/p3-drive.test.mjs:152 `byStep(20).last.detail, 'signature'`. Also tests/p3-forms.test.mjs:416 `remove(form2, sig.id).detail, 'signature'` and tests/p2-forms.test.mjs:308 `[item('sig'), 'nope', 'later version']`.
amendment: §4.2 changes "field-design.jsonl … never edited" to "field-design.jsonl line 25 only: expect `fields.last {action:'rename', name:'signed'}, dirty:true` (tests role, with p3-drive.test.mjs:152 → `deepEqual(byStep(20).last, {action:'rename', name:'signed'})`)".
§5 engine gains ownership of tests/p3-forms.test.mjs:416 (→ `remove(form2, sig.id).ok === true`, then undo) and of the p2-forms.test.mjs:308 regex (→ `'click the signature field'`). Those are replacements, not additive changes.
The signed-branch refusal stays proven by §1.5's scratch `/V` document.

**T3** severity: blocking
claim: `fields {action:'fill'}` on a signature field reports `detail:'invalid'` today, not `'signature'`. The engine throws a plain Error, and `refusalDetail` falls back to `"invalid"`.
evidence: src/renderer/engine.ts:3302 `if (kind === "signature") throw new Error("field-set: signing arrives in a later version");` and renderer.ts:4675 `return typeof reply?.detail === "string" ? reply.detail : "invalid";`.
amendment: §1.3: "`field-set` on any signature field → `failField("signature", "field-set: click the signature field to sign it")` (reply `detail:'signature'`, reason `invalid`)". Drop the words "the plain-Error branch is unchanged in shape". §4.1 keeps `fields.last {action:'fill', name:'S1', detail:'signature'}`.

**T4** severity: blocking
claim: the panel no-name add cannot show `fields.last.name:'Signature1'`. `driveFields` records `name` (here `""`) before the request and never reads `reply.name`, and §2.4 says the add passes through "unchanged".
evidence: renderer.ts:4302 `fieldDesign = { action, name };` then `reply = (await request(payload))`, with nothing after it that reads the name.
amendment: §2.4: "`driveFields` `add`: after an ok reply, `fieldDesign = { action:'add', name: typeof reply.name === 'string' ? reply.name : name }` (every kind; `field-add`'s reply carries `name` for every kind, §1.2). A refusal keeps `{action:'add', name, detail}`."

**T5** severity: amend
claim: §4.1's shorthand `sign {…, to}` is ambiguous. `packDrive` lifts keys only for click/type/key/drag/handle, so `sign`, `arm`, `fields`, `goto`, `mode` and `verify` must carry `contents`, and `to`/`path` must stay at the top level. A flat `sign` line reaches the renderer with `contents === null` and opens the image Sign dialog.
evidence: scripts/drive.mjs:22 `if (!['click', 'type', 'key', 'drag', 'handle'].includes(row.cmd)) return line;` and renderer.ts:1466 `else if (op === "sign") return contents === null ? openSignDialog() : driveSign(contents);`.
amendment: §4.1 adds the line templates:
`{"cmd":"arm","contents":{"tool":"sigfield"}}` · `{"cmd":"drag","page":0,"x":72,"y":300,"dx":220,"dy":60}` · `{"cmd":"click","page":0,"x":182,"y":330}` (Signature4 centre)
`{"cmd":"sign","contents":{"cert":"fixtures/p5/leaf.p12","password":"pdf4all","field":"Signature4","reason":"drive field"},"to":"${OUT}/field-signed.pdf"}` (as tests/drive/sign.jsonl:3)
`{"cmd":"goto","contents":{"page":1}}` · `{"cmd":"mode","contents":{"mode":"forms"}}` · `{"cmd":"open","path":"${OUT}/field-signed.pdf"}`

**T6** severity: amend
claim: the expects written as `signatures[0].status 'signed'` / `[2].status` / "length 3" cannot be written as JSONL. `matches()` compares arrays whole, with exact length and every item. Separately, the enumerated script has 31 steps, not "about 32".
evidence: src/main.ts:1660 `if (Array.isArray(want)) return Array.isArray(got) && want.length === got.length && want.every(…)`.
amendment: §4.1: "every `signatures` expect is a full-length array of per-item subsets, e.g. `[{"field":"Signature1","status":"signed"},{"field":"Signature2","status":"empty"},{"field":"Signature4","status":"signed"}]`. 'length n' means n items of `{field}`. The script has 31 steps (open…open hello-field). §4.2 pins `steps.length === 31`, or the final count the integrator records."

**T7** severity: amend
claim: every drive `sign` step switches the UI to Annotate, because `OP_MODE.sign = "annotate"` (the image-stamp tool). The field path therefore starts in Forms mode (click) and ends in Annotate mode. Nothing in §4.1 asserts `mode`, but an author who adds `mode:'forms'` after a sign step will see it fail.
evidence: renderer.ts:569 `ink: "annotate", stamp: "annotate", sign: "annotate", shapes: "annotate",` and :949 `if (wantedMode && wantedMode !== uiMode) setMode(wantedMode);`.
amendment: §4.1: "no `mode` expect after a `sign` step (OP_MODE.sign is `annotate`; unchanged in this unit)". §4.4 design-agent-drive.md states the same in one line.

**T8** severity: amend
claim: the fixture baseline has 52 files today, not 51, so it becomes 53 with the new fixture. `accept-u4.sh` hard-codes p6 paths and the U4 drive list.
evidence: `find fixtures -type f \( -name '*.pdf' -o -name '*.png' \) | wc -l` → 52. accept-u4.sh: `O=$R/.quad/shared/p6/acceptance`, `u4-hash-before.txt`, `for s in direct-edit4 direct-edit5`.
amendment: §6.3: "…7-line hash list + the 52-file baseline diff (after: 53; only `fixtures/p5/empty-sigfield.pdf` added)".
§5 fable51: "`accept-u3.sh` sets `O=.quad/shared/p5/acceptance` and uses `.quad/shared/p5/checks/u3-hash-{before,after}.txt`. Drives: sign-field ×3; sign, verify, field-design, direct-edit5 once each. Every one gates on exit 0 (U4 §9.21). `shasum -c` stays on the p6 list."

**T9** severity: amend
claim: tests/p5-verify-crypto.test.mjs has no `.venv-qa` skip to copy (it always spawns). Also, verify-sign.py falls back silently to "untrusted" and exits 0 when `--ca` does not resolve, because the default path is relative to the working directory.
evidence: tests/p5-verify-crypto.test.mjs:46 `spawnSync(py, ['scripts/verify-sign.py', path.join(root, rel), '--ca', caPem], { encoding: 'utf8', cwd: root })`; scripts/verify-sign.py:34 `trust = … if ca.exists() else []`.
amendment: §1.5/§4.2: "pyHanko assertions sit in their own `test()` with `{ skip: existsSync(py) ? false : 'no .venv-qa' }`. They spawn with `spawnSync(py, ['scripts/verify-sign.py', abs, '--ca', join(root,'fixtures/p0b/.ca/ca.pem'), '--expect-signatures', String(n)], {cwd: root, encoding:'utf8'})`, assert `status === 0`, and match lines by field name (`Signature4 INTACT ENTIRE_REVISION trusted docmdp-ok`)."

**T10** severity: amend
claim: §4.3's "open `${OUT}/field-signed2.pdf` from a prior drive run" is not reproducible, and a verbatim copy of shot-direct4.mjs breaks in two places. `session()` joins root with the fixture path, which mangles an absolute path. `hashes()` reads `fixtures.sha256` next to the script, which does not exist in p5/checks.
evidence: shot-direct4.mjs:99 `{ cmd: 'open', path: join(root, fixture) }` and :91 `shasum … join(checksDir, 'fixtures.sha256')`.
amendment: §4.3: "The script first runs `execFileSync(process.execPath, ['scripts/drive.mjs','tests/drive/sign-field.jsonl', `--out=${out}/u3-drive-out`], {cwd: root})` and requires exit 0. It then opens `${out}/u3-drive-out/field-signed2.pdf` in its own `session()`, which resolves `isAbsolute(f) ? f : join(root, f)`. `hashes()` reads `join(checksDir, '../../p6/checks/fixtures.sha256')`. udata is `review-out/u3-udata-<tag>`, removed first."

**T11** severity: amend
claim: the GUI "✓ Valid" badge shot cannot appear. Verify without an imported CA returns valid + untrusted, which §2.1 captions "✓ Valid, untrusted". Importing trust needs a native file dialog.
evidence: tests/drive/verify.jsonl:4 `{"status":"valid","trust":"untrusted",…}` for a bare `verify`. src/main.ts:858 opens `showOpenDialog` for trust-add.
amendment: §4.3: "after a real click on `#verify-open`, the shot shows `.sig-target.valid.untrusted` captions '✓ Valid, untrusted' (no trust import in the GUI script)". The trusted-green look stays proven only by §4.1 STATE (`status 'valid'`).

**T12** severity: amend
claim: §4.2 should also prove that the drawn rect survives signing. `pick()` can read `signatures` through the path form (`at()` splits on `.`, so array indices work), whereas `fields.defs` excludes signatures.
evidence: tests/p6-drive4.test.mjs:37 `path.split('.').reduce(…)`; renderer.ts:4602 `if (item.kind === "signature") continue;`.
amendment: §4.2: "`near(pick(report,'drag',{'contents.dx':220}).state.signatures[2].rect, [72,300,292,360], 0.5)`. The `open` step whose `path` ends in `/field-signed.pdf` has `state.signatures[2].rect` within ±0.01 of the drag step's rect."

**T13** severity: nit
claim: `verify` should be written as verify.jsonl writes it (`"contents":{}` → `driveVerify`). The bare form opens the dialog through `openVerifyDialog()`, which is a different path.
evidence: renderer.ts:1460 `return contents === null ? openVerifyDialog() : driveVerify(contents);`.
amendment: §4.1: "`{"cmd":"verify","contents":{}}`".

**T14** severity: nit
claim: `sign.dialog` must live in `stateSummary`'s `sign` object. However, `editSummary` spreads a `sign` key over it whenever an image-signature stamp is on the page, and that erases `sign.last` and `sign.dialog`. The U3 fixtures do not trigger this.
evidence: renderer.ts:6599 `sign: { last: signLast },` versus :6674 `...(sign ? { sign } : {}),`.
amendment: §2.5: "`sign.dialog` sits beside `sign.last` in `stateSummary`; the `editSummary` stamp key is unchanged (its collision is pre-existing and out of scope)".

**T15** severity: nit
claim: the §4.3 trust-store check is vacuous. Under `--drive` (smoke), main builds the store with no file, and the path it names is not the scratch udata anyway.
evidence: src/main.ts:424 `createTrustStore(smoke ? null : join(app.getPath('userData'), 'trusted-certs.pem'))`; main.ts:27 `smoke = … || driveArg !== undefined`.
amendment: §4.3: "trust store: no `trusted-certs.pem` under any `review-out/u3-udata-*`, and the real `~/Library/Application Support/pdf4all-p0/trusted-certs.pem` has the same mtime before and after (the drive path never writes it: main.ts:424)".

**T16** severity: nit
claim: §4.2 should pin hello.pdf's hash by value. tests/fixtures.test.mjs pins no hello hash (only IDs at :16).
evidence: `shasum -a 256 fixtures/hello.pdf` → `229ca9488a139d3f32b864ba24055152fe7f0faf670c9db6491144f23d275b94`.
amendment: §4.2: "`fixtures/hello.pdf` = `229ca948…d275b94` and `fixtures/p5/empty-sigfield.pdf` = `07e46851…0523a206`, before and after".

## Confirmed
- `${OUT}` is expanded across the whole script text before parsing (main.ts:1683). `to` is read from the top level of every command (main.ts:1719), and sign-file writes only to `saveToArg` in drive mode (main.ts:813). sign.jsonl:3/5 already use this pattern.
- No `editable()` guard stands in front of `sign` (runEditRequest 1420-1470). sign.jsonl:5 signs the read-only `signed.pdf` and expects `revisions:2`.
- The drawn field's target is painted before the next `click`. `applyEdit` awaits `refreshFields` (4140), which calls `paintHits` (4409). `driven()` serialises steps, and `goto` awaits `refreshFields` (1000).
- Re-arming after a tiny drag works (direct-edit5.jsonl:39-42). A 220×60 pt drag is far above 3 px and inside the 612×792 page. The drag at (72,300) misses Signature1/2 (y 620-680).
- STATE carries no paths (renderer.ts:6557). `signer` is the CN "P0B Signer", and `sign.last.name` is a basename. Registering `pdf4all` as a secret does not false-fail the STATE leak check.
- Nothing in the script writes `hello.pdf` or `empty-sigfield.pdf`: there is no `save-in-place`, and every `to` goes under `${OUT}`.
- No test pins "kind must be one of text, checkbox, combo, radio". direct-edit5 rightly stays in acceptance: it covers form2's empty `sig` next to the Forms-mode nudge and the Tab order (`order` excludes signatures, renderer.ts:4612).
- The timeouts are enough. There are 31 steps with 5 signing attempts, against U4's 71 steps in 150 s. Each step has its own 60 s timeout (main.ts:1705, step timeout).
