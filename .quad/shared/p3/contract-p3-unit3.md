# P3 U3 — bounded subset fonts (existing-code edits on Identity-H CID text)

2026-09-17. Orchestrator **fable51**; reviewer **opus5** (Claude Opus 5, astra's seat); ds2 = engine + UI + `tests/p3-edit.test.mjs`; grok = one fixture only (budget-limited). Parent `contract-p3.md` §2 font ceiling (human option 1: subset edits need a validated existing-character map for that same font; no import/re-embed/substitution), §5 row U3. U2 accepted 236/236, U1b 246/246; current suite 247.

## 0. Facts this contract rests on (fable51 checked 2026-09-17, `fixtures/p0b/edit-subset.pdf`, `fixtures/cjk.pdf`, `fixtures/rtl.pdf`)
- Every CJK/RTL fixture font is `Type0` + `Identity-H` + `CIDFontType2`, `CIDToGIDMap` absent (= Identity), `FontFile2` embedded, `/W` an indirect array, `/DW` absent (default 1000), `/ToUnicode` present.
- The `/ToUnicode` CMap **still maps the whole original font after `subsetFonts()`** (100+ bfrange blocks for a 6-glyph page). ToUnicode membership therefore proves nothing about the subset. Only a code that is **already drawn with that font resource on the page** is proven to have a glyph.
- P0-b: `Font.encodeCharacter` returns 0 on subset CID fonts even for glyphs that render (`review-p0b.md`). U2's stext-handle width measurement cannot be used for CID runs; the PDF advance oracle is `/W`/`/DW` anyway.
- U1 today refuses every `Type0` run: `subset-font` (prefixed BaseFont) or `unsupported-encoding`; `rtl-or-shaping` is a reserved token (`engine.ts:4386-4394`).

## 1. Deliverable
A text run in an embedded `Type0/Identity-H` font (subset or not) becomes an editable candidate when every code in its payload has a ToUnicode mapping to exactly one BMP code point that is not RTL/combining. The Text inspector shows the run's decoded text and the **writable repertoire**: the set of characters proven for that font on this page. Replacement text may only use that repertoire; anything else refuses with the missing characters listed. The write is the U2 write (same operator span, same width rule, same ActualText rule, same one `mutate`), with a hex operand instead of a literal. Simple fonts with a subset prefix get the same repertoire rule (see §2.4). No font program is touched, added or replaced.

## 2. Engine seam (ds2)

### 2.1 Font resolution (`editResolveFont`)
Extend `EditFont` for `Type0`: `cid: { toUnicode: Map<number,string>, widths: Map<number,number>, dw: number } | null`, set only when all of: `/Encoding` is the name `/Identity-H`; one descendant of `/Subtype /CIDFontType2` or `/CIDFontType0`; `/CIDToGIDMap` absent or `/Identity`; descriptor has `FontFile2`/`FontFile3`; `/ToUnicode` is a stream whose CMap parses (`begincodespacerange` all 2-byte; `bfchar` and `bfrange` with 2-byte codes; destination hex of 4 hex digits = one BMP code point, or an array form in `bfrange`; a destination of any other length, i.e. surrogate pairs or ligature sequences, marks that code as **unmappable**); `/W` parses (both `c [w1 w2 …]` and `cfirst clast w` forms, indirect refs resolved), `/DW` number or 1000. Stream reads go through the existing bounded `readStream` path and are charged to the 2 MiB scan budget; a ToUnicode or W over the budget → font unresolved → `unsupported-encoding`. Anything else (`Identity-V`, predefined CMaps, embedded CMap streams, non-Identity CIDToGIDMap, no font file) → `cid: null` and the run keeps `unsupported-encoding`; `WMode 1` keeps `vertical-writing`.
Reply `EditTargetFont` gains nothing new except `repertoire` on the target (below); `subset` stays as is.

### 2.2 Discovery (`edit-targets`)
For a run whose font has `cid`:
- Split payload bytes into 2-byte codes (odd length → `unsupported-encoding`). Decode each through `toUnicode`; any code missing or unmappable → `unsupported-encoding`. Decoded text is reported in `text` (this replaces today's `undefined`).
- If any decoded code point is in `֐-ࣿ`, `יִ-﷿`, `ﹰ-﻿` (Hebrew, Arabic, Syriac, Thaana, NKo, presentation forms) or is a combining mark `̀-ͯ`, `᪰-᫿`, `᷀-᷿`, `⃐-⃿`, `︠-︯` → reason **`rtl-or-shaping`** (new token; add it to `EditTargetReason` and `CONTENT_REASON_TEXT`: "Right-to-left or shaped text is read-only"). Byte order in the file is visual order for such runs; U3 does not decode bidi.
- Otherwise the font gate passes; the existing association (`editMatchRuns`, stext chars vs decoded text), tag, OC and sharing gates run unchanged. `subset-font` is no longer produced for Identity-H fonts; it stays for any other prefixed font the gates cannot prove (keep the token).
- **Repertoire:** during the page walk, per font resource *object* (dictionary identity, not resource name), collect every code seen in any text-show payload of that font, including runs that are refused for other reasons (OC, clipped, tail, shared): a code drawn anywhere on the page proves its glyph. For each candidate CID run, `repertoire: string` = the sorted, de-duplicated decoded characters of that set (cap 512 characters; over the cap → the first 512 by code, note it in evidence). Reverse map for the write = `unicode → code` built from the same set; if two used codes decode to the same character, that character is **ambiguous** and left out of the repertoire.
- Simple fonts with `EDIT_SUBSET_PREFIX` (§2.4) also get `repertoire`. Non-subset simple fonts: no `repertoire` field (unchanged U2 behaviour).

### 2.3 Write (`edit-text`)
- Text validation for a `cid` run: every char of `text` must be in the run's repertoire (recomputed at write time from the fresh `resolveEditTarget` walk, never from the reply); otherwise `unsupported-encoding` with `missing` (max 20, ambiguous chars count as missing). Length cap 4096 unchanged. ASCII-only rule applies only to non-`cid` runs.
- Width: `Σ (widths.get(code) ?? dw) / 1000 × size` for old and new codes (size = the run's `Tf` size as recorded, same as U2). Same tail/overflow rule and `oldWidth/newWidth` report. `unmeasurable-width` when `size` is unknown.
- Operand: write `<hex>` (4 hex digits per code, lowercase); delete writes `<>`. U1 already parses hex payloads so re-discovery finds it.
- ActualText (inline or StructElem, U2 rules): when the new text is not printable ASCII write a UTF-16BE text string with BOM as hex `<FEFF…>`; ASCII keeps the U2 literal. Equality check against the existing ActualText decodes both forms (BOM'd UTF-16BE or PDFDocEncoding-as-Latin1).
- Everything else (COW privatise, journal, undo, revision bump) is the U2/U1b path. No new op, no new IPC.

### 2.4 Simple subset fonts (latent U2 gap, close it here)
A `Type1/TrueType` font with a subset prefix currently passes U2's gate and relies on `encodeCharacter` to refuse missing glyphs. Reviewer's probe (§4) decides: if `encodeCharacter` returns 0 for a WinAnsi char absent from a subset TrueType, keep U2's path and record it. If it returns a gid that renders blank, ds2 applies the repertoire rule to prefixed simple fonts too: repertoire = bytes used on the page with that font decoded as Latin-1, write refuses chars outside it with `missing`. Either way add the test.

## 3. UI seam (ds2) — inside `#content-editor`, no new IDs beyond these
- Row text for CID candidates is the decoded text (CJK visible in the list). Reason text for `rtl-or-shaping` as in §2.2.
- When the selected candidate has `repertoire`, `#content-edit-form` shows `<p id="content-repertoire">` "Only characters already in this font on this page can be used: <repertoire>" (one line, `overflow-wrap:anywhere`). `#content-hint` keeps the U2 width sentence. Apply with a char outside the repertoire → the engine refusal text plus the missing characters in `#edit-status` ("Cannot draw: 試 験 (not in this font)").
- Nothing else changes: same keyboard flow, same announce strings, same `OP_MODE` entry.
- Drive script `tests/drive/subset-edit.jsonl`: open `fixtures/p0b/edit-subset.pdf`; `content` target `日本語テスト` → `テスト日本` (4 existing codes, shorter) expect edited; second `content` `日本語テスト`… n/a after edit, so instead target `隣の行` → `試験` expect `refused` with `unsupported-encoding`; undo/redo; save-to. Add a test in `tests/p3-drive.test.mjs` if the U1b one is the pattern (check `tests/drive/cow-edit.jsonl` and its test first).

## 4. Gate probe (opus5, script only, no `src/`) — `scripts/p3/subset-probe.mjs <out-dir>` + `p3-unit3-probe.md`
Runs before ds2's write lands (ds2 may build §2.1–2.2 discovery in parallel; the write waits for the probe verdict).
1. On `edit-subset.pdf` and `cjk.pdf`: parse `/W` + `/ToUnicode` in JS; compare each used code's `/W` advance × size against the stext quad width of the same char (`toStructuredText().walk` `onChar` quad) → agree within 0.5 pt, else report.
2. Rewrite line 2's hex operand to a permutation of existing codes (`テスト日本`, 4 codes), save `incremental=no,garbage=yes`, then: mupdf + pypdf extraction show the new string and not the old; `scripts/p0b/recover.py <out> <old hex>` raw 0; `qlmanage` render of the edited line is non-blank (count dark pixels in the line's rect ≥ 1% of the rect) and the neighbour line's pixels are unchanged (`scripts/verify-edit-region.py before after --rect …`).
3. Write a code that is in ToUnicode but **not used on the page** (e.g. 試's code from the full font's ToUnicode) and render: expect a blank or notdef box → this is the evidence that ToUnicode membership is not enough. Record the pixel result.
4. Simple subset TrueType: build a one-off PDF in the probe (do not add a fixture) with `addSimpleFont(new Font(DroidSansFallback bytes), SIMPLE_ENCODING_LATIN)` drawing `ALPHA`, `subsetFonts()`, then `Font.encodeCharacter("Z")` on the stext handle and `advanceGlyph`; and rewrite `(ALPHA)`→`(ALPHZ)` and render: does Z draw, draw blank, or fail? Verdict feeds §2.4.
5. `rtl.pdf`: confirm the Hebrew/Arabic runs decode into the §2.2 ranges (so the refusal token is honest) and that the Arabic run's codes map to presentation forms (shaped).
Verdict line: **GATE PASS / FAIL** with the numbers. If FAIL on item 2 the unit stops and the orchestrator reports the ceiling to the human; the refusal stays.

## 5. Fixture (grok) — `fixtures/p3/make-edit3.mjs` → `fixtures/p3/edit-subset2.pdf` + provenance + README row
One page, DroidSansFallback via `fixtures/fetch-font.mjs`, `subsetFonts()`, `garbage:true`: line 1 `/F2` `日本語テスト` at 28 pt; line 2 `/F2` `テスト` (repeated codes, different run); line 3 `/F2` `日本 語` with a `TJ` array `[<…> -500 <…>]` (must stay `unsupported-text-operator`); line 4 a second Type0 resource `/F3` = the **same font bytes added again** (`doc.addFont(new Font(...))` twice) drawing `試験` — proves the repertoire is per font object, not per name or per BaseFont: editing line 1 must not be allowed to use 試. Line 5 `/F1` Helvetica `NEIGHBOR`. Record which codes each line uses in the provenance `notes`. Nothing else; no smoke, no `main.ts`.

## 6. Ownership, slots, evidence
- ds2: `src/renderer/{engine.ts,renderer.ts,index.html,styles.css}`, `tests/p3-edit.test.mjs` §U3, `tests/p3-targets.test.mjs` expectation updates (`subset-font` → candidate on `edit-subset.pdf`/`cjk.pdf`; `rtl.pdf` → `rtl-or-shaping`), `tests/drive/subset-edit.jsonl`, `tests/p3-drive.test.mjs`.
- opus5: `scripts/p3/subset-probe.mjs`, `p3-unit3-probe.md`, then `review-p3-unit3.md` (independent rerun of §7; reviewer fix authority per progress.md §9.3, claim files first).
- grok: §5 only. No build needed.
- **Slot order:** opus5 probe and grok fixture need no build → first. ds2 holds the build slot (focused tests: `node --test tests/p3-edit.test.mjs tests/p3-targets.test.mjs tests/p3-drive.test.mjs`). Then opus5 full `npm test` + review. Claim/done on the bus each time.
- Evidence: `p3-unit3-ds2.md`, `p3-unit3-probe.md`, outputs under `.quad/shared/p3/unit3/`; reviewer `review-p3-unit3.md`; fable51 accepts and updates `progress.md`.

## 7. Acceptance (parent §5 U3 row)
- `edit-subset.pdf`: `日本語テスト` → `テスト日本` succeeds; saved file: mupdf + pypdf extract the new string, `recover.py` old hex raw 0, QL render non-blank in the rect and byte-identical outside (`verify-edit-region.py`); `隣の行` unchanged; undo restores bytes; redo → save → reopen.
- `日本語テスト` → `日本語試験` refuses `unsupported-encoding` with `missing: ["試","験"]`, file unchanged. `edit-subset2.pdf` line 1 → text using 試 refuses even though `/F3` on the same page has it. Line 3 stays `unsupported-text-operator`.
- `cjk.pdf` page 0: both CJK runs are candidates; `中文测试` → `测试` succeeds (shorter).
- `rtl.pdf`: every Hebrew/Arabic run `rtl-or-shaping`; Latin `PDF4All` runs unchanged.
- `edit-embedded.pdf` (full font, non-subset Type0): same rules apply; repertoire is still page-scoped (evidence notes this as the U3 ceiling: full fonts could allow any glyph, deferred).
- Tail/overflow refusals with `/W` numbers; ActualText UTF-16BE rewrite on a tagged CJK case (ds2 may build a synthetic one in the test as U1b did); `encrypted-noedit.pdf` refuses; `npm test` all green; `tests/drive/subset-edit.jsonl` passes via `node scripts/drive.mjs`.
- Ceilings to record, not solve: page-scoped repertoire; no bidi; no `TJ`; ligature/surrogate destinations refused; `Identity-V`/predefined CMaps refused.
