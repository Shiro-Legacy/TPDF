# Bates numbering objections — engine lens (Opus 5.5, 2026-09-24)
Scope: contract-bates.md D1–D4, §0, §1. Read-only on src/tests/fixtures. Probes in `.quad/scratch/bates/objections/engine/`: `fixtures.mjs` (→ `fixtures.out`: page count/labels/boxes/Rotate of nav, hello, rotated; mupdf `Font("Helvetica")` API; exact vs 0.556-em widths), `rightalign.mjs` (→ `rightalign.out`: engine.ts:3122-3127 `place()` replicated with the D3 vx on 400×200 R=0/90/180/270 and 612×792, stext line bbox). No build, no Electron. `shasum -a 256 -c checks/fixtures.sha256`: 9 OK.

**E-1 (blocking) §1.5(d)** — `fixtures/rotated.pdf` has no `/Rotate`: one 612×792 page, rotation 0, only rotated *text runs* (`strings`; fixtures.out `rotate:0`; provenance `pages: 1`). The :164 test does not use it; it builds `rotatedPagePdf(rotate)` 400×200 in-test (tests/p2-decorate.test.mjs:53-59, :164-170). So "(d) … on rotated.pdf (every rotation …)" checks R=0 twice and never checks 90/180/270.
Amendment: "9.x §1.5(d) runs the right-aligned footer `ACME-{bates}` (start 100, pad 6) on `rotatedPagePdf(0|90|180|270)` (p2-decorate.test.mjs:53) and on `fixtures/nav.pdf` p0. It asserts the stext line bbox x1 is within 10 pt of `page.getBounds()` width − 36 (400−36 for R=0/180, 200−36 for R=90/270, 612−36 for nav) and its y1 is > height − 60. `rotated.pdf` is not used."

**E-2 (should) §1.4, §1.5(c)** — the refusal mechanism is left for the builder to find. Stamp refusals today are thrown Errors that `dispatch`'s catch turns into `{ok:false, error: message, reason?, detail?, ...journalState}` (engine.ts:5980-5996). There are two flavours: kind/text are plain `throw new Error("stamp: …")` with no `reason` (engine.ts:3195, :3047), and pages use `failReason("invalid", …)` → `reason:"invalid"` (engine.ts:2843, :2847). Tests assert with `assert.match(reply.error, /…/)` only (p2-decorate.test.mjs:294-313). A bare `/invalid-bates/` would also match `bates-start-required`-free messages that merely mention it, so the test would be weak.
Amendment: "9.x Each new refusal is `failReason("invalid", "stamp: <token>: <detail>")` (engine-core.ts:159), with token ∈ {`bates-start-required`, `invalid-bates`, `invalid-align`}. §1.5(c) asserts `reply.ok === false`, `reply.reason === "invalid"` and `assert.match(reply.error, /^stamp: <token>:/)`, plus `session.revision` unchanged and `op undo` → ok false."

**E-3 (should) §1.2, D1, §1.5(f)** — the range caps the start and not the end. With start 10^15−1 on all 3 nav pages, `to` = 10^15+1, so pages 1–2 get 16 digits (`String(1e15+1).length` = 16, probe). That contradicts "pad 15 … is the ceiling", and the renderer's `batesNext` then exceeds the refusable range. (f) does not say which pages it uses.
Amendment: "9.x `invalid-bates` also when `batesStart + pages.length − 1 > 10^15 − 1`. §1.5(f) stamps `pages:[0]` with start 10^15−1 pad 15 (→ `999999999999999`, 15 digits), and start 10^15−2 on pages [0,1,2] refuses `invalid-bates`."

**E-4 (should) §1.2** — the text does not say when the fields are validated or what types they take. Is `batesPad: 16` refused when the text has no `{bates}`? Is `align:"centre"` refused on a watermark? Is `batesStart:"100"` (a string) accepted by the engine?
Amendment: "9.x `align`, `batesStart` and `batesPad` are validated whenever present, for every kind and whether or not the text has `{bates}`. The engine accepts only JS numbers (`Number.isInteger`); numeric strings are the renderer's job (§2.3). `pad = batesPad ?? 6`. When the text has no `{bates}`, the reply has no `bates` key even if `batesStart` was sent. A start with more digits than pad is not truncated (`padStart`)."

**E-5 (should) §1.3, (6)** — the width uses the wrong length. `stampPage` computes the text as `winAnsiBytes(stampTextFor(...))` (engine.ts:3117), and the watermark estimate uses `template.length` (engine.ts:3133). For D3, `chars` must be the expanded *string* length before `winAnsiBytes`, which adds a `\` byte per `(`, `)` or `\`. The watermark should use the same expanded length. This is safe: no test pins watermark Tm (grep `Tm` in tests/: only the header regexes at p2-decorate.test.mjs:166-169).
Amendment: "9.x `stampPage` keeps `const expanded = stampTextFor(template, page, pageIndex, count, bates)` and computes the width from `expanded.length` for both the right-aligned header/footer and the watermark (`textWidth = expanded.length × 48 × 0.5`), then `winAnsiBytes(expanded)`."

**E-6 (should) D3** — 0.556 em fits digits and the test string, but caps prefixes run past the margin. The AFM widths (A 667, C 722, M 833, E 667, - 333, digits 556) give 65.58 pt for `ACME-000100` at 10 pt against the 61.16 pt estimate. The run ends 4.42 pt into the right margin; `rightalign.out` shows delta 4.42 on every rotation, inside the 10 pt tolerance. `CONFIDENTIAL ACME CORP 000100` ends 11.58 pt past, and `WWWWWWWWWW-000001` 36.57 pt past, which is off a page with a 36 pt margin (fixtures.out). The exact width is free: `stampPage` already creates `new mupdf.Font("Helvetica")` (engine.ts:3163-3169), and mupdf.js exposes `encodeCharacter`/`advanceGlyph` (fixtures.out: Font methods). This is not a width table.
Amendment (take or reject; the lazier D3 stays buildable): "9.x Right alignment measures `textWidth = size × Σ font.advanceGlyph(font.encodeCharacter(cp), 0)` over the expanded string, using one `mupdf.Font("Helvetica")` per call (destroyed in `finally`), so the run ends at visualWidth − 36 ± 0.5 pt. §1.5(d) tolerance 1 pt."

**E-7 (nit) §0** — "`checkStampText` stays the engine-core version" is false. `stampPages` calls the local closure copy at engine.ts:3045 (2 args), and engine-core's `checkStampText` (engine-core.ts:210) is not imported (engine.ts:8-38). `winAnsiBytes` is imported (:34) but shadowed by a local copy at :3059.
Amendment: "9.x `stampPages` keeps calling the local `checkStampText` (engine.ts:3045) and local `winAnsiBytes` (:3059) unchanged. No import is swapped and no copy is added."

**E-8 (nit) §1 header, §4** — "`engine-core.ts` for `BATES_MAX_PAD`" is not in §4 ownership.
Amendment: "9.x No engine-core.ts change. The Bates limits are literals or constants next to `MAX_STAMP_CHARS` (engine.ts:142)."

**E-9 (nit) §1.2** — `stampTextFor` replaces `{label}` last (engine.ts:3073-3077). If `{bates}` is appended after it, a page label that literally contains `{bates}` gets expanded.
Amendment: "9.x `stampTextFor` replaces `{bates}` first (digits cannot form a token), then `{page}`, `{pages}`, `{label}`."

## checkedFine
- The loop counter: `pages` is deduped and sorted before `mutate` (engine.ts:3203-3205), so `pages.entries()` gives the stamped-page index. (b) `[2,0]` → p0 000100, p2 000101 holds. With no `pages`, all pages are used.
- `stampTextFor(template, page, index, count)` (engine.ts:3072) has one caller (:3117) and `stampPage` has one caller (:3207), so adding a `bates?: string` param and an options object is local.
- `place()` handles a right-aligned vx on all four rotations. It uses visual width = `getBounds()` width (400/200/400/200 on 400×200 R=0/90/180/270). The stext bbox x1 lands at target+4.42 on each (rightalign.out), and stext is in visual space. The left default keeps vx 36, so the Tm regexes at p2-decorate.test.mjs:166-169 do not flip.
- The reply is `{id, ok:true, ...editState}` (engine.ts:3209). No test deepEquals stamp reply keys (grep `Object.keys` and stamp in tests/), so adding `bates` is safe. Stamp appears only in p2-decorate, p3-drive and dialogs.jsonl.
- MAX_STAMP_CHARS 200 is checked on the template (engine.ts:3046). Expansion is bounded at about 28×15 digits: the stream is an append-only literal and `winAnsiBytes` maps digits 1:1 (engine.ts:3059-3069). No cap is involved, and the integers stay safe (`Number.isSafeInteger(1e15+2000)` true).
- Undo: one `mutate(session, doc, "add footer", …)` covers every page (engine.ts:3206-3208), so one undo removes all of them (the pattern of p2-decorate.test.mjs:~150).
- `{bates}` in a watermark: `stampPage`'s watermark branch uses the same expanded text (:3117). Only the width (E-5) needs changing.
- Fixtures: nav.pdf is 3 pages, 612×792, labels 1,2,3, no Rotate or CropBox. hello.pdf is 1 page, 612×792. rotated.pdf is 1 page, R=0 (E-1).
- Refusals come before `mutate` today, so a refusal never dirties (p2-decorate.test.mjs:309-311).

## buildRisks
- Put every new check between `checkPages` (engine.ts:3205) and `mutate` (:3206). A check inside `work()` would journal-abandon instead of refusing.
- Keep `requireMutable` (engine.ts:5777) first: signed/read-only docs must still reply `reason:"read-only"` (p2-decorate.test.mjs:320-335).
- `chars` and the watermark width come from the expanded string, not from `winAnsiBytes` output or `template.length` (E-5).
- The renderer's `addStamp` (renderer.ts:7623) only `showError`s the message, and `decorateLast` gets no detail (renderer.ts:1162-1180). The engine token is visible in STATE only via the top-level `error`.
- Churn: debt-sweep-2 edits engine.ts (edit-batch/clip-paste around :5772-5775). The stamp lines above were read as-is; re-grep before editing.
