# P6 U7 objections on §10 (D4 widening) — engine lens (Opus 5.5, 2026-09-24)
Scope: §10.1–§10.6, §10.8, §10.12 and their fit with §1–§9. Read-only on src/tests/fixtures. Probes (pure node against build/, no lock) in `.quad/scratch/p6u7/objections-d4/engine/`: `paste-text.mjs` → `paste-text.out` (scanner record → scratch graft → §10.4 block as a new `/Contents` stream → rescan), `view-rule.mjs` → `view-rule.out` (same on direct3 p1 `/Rotate 90`), `fields-marks.mjs` → `fields-marks.out` (field-defs facts, field-add limits, mark/link/field rects on p1), `empty-op.mjs` → `empty-op.out`. `shasum -c checks/fixtures.sha256`: 9 OK.

**E10-1 (blocking) §10.5, §10.10** — a text paste onto a page that already shows the same text as a whole line can never pass the §10.5 proof. `editMatchRuns` keeps whole-line duplicates ambiguous (content-edit.ts:1425-1427, :1470). paste-text.out: "Move me" pasted on direct3 p0 at d (0,0) and at (0,−50) scans `unsupported / ambiguous-text-association`. After a paste at (10,−10) the SOURCE run turns unsupported too (both `c0` and `c1`). Pastes on p2, on a fresh page, and after a `garbage=yes` reopen are candidates with bounds + d exact. Cut keeps runs (§10.8), so Copy/Cut → Paste of a run on its own page always abandons with `unsupported-transform`. That is the most common text paste, and §10.10 never tests it.
Amendment: "10.5 (text) — a pasted run is proven by the rescan's structured text, not by candidacy. Exactly one stext occurrence of the source text must have its first char `origin` at the target origin ±0.5 view pt (the S1 origin rule of `editMatchRuns`); else `unsupported-transform`. `targetIds` lists the pasted runs that scan as candidates. A run that is ambiguous because its text already stands as a whole line on that page is accepted, and Help's ceilings gain: 'A pasted run whose text already appears as a whole line on the page cannot be edited until one of them changes.' §10.10 adds a same-page case: paste 'Move me' on direct3 p0 with no `at` → ok, one journal op, the stext line at origin + (10,+10 view)."

**E10-2 (blocking) §10.1, §10.3, §10.5, §10.10** — "all in one page-point y-up space" is false for four of the six kinds:
- text `bounds`, `origin` and `textLinear` are in MuPDF's view space: `T = page.getTransform()`, y down, with `/Rotate` and the MediaBox origin folded in (content-edit.ts:1854, :2336, :2407);
- `FieldItem.rect` is y down (engine.ts:604), and so is `AnnotItem.rect` (:612);
- `LinkEntry.rect` is `link.getBounds()` (:2569), and redact rects are `getRect()` (redact.ts:55); both are view space;
- only objects' `placement.visible` is PDF y-up.

"Source bounds + d" is therefore undefined across pages. On direct3 p1 (`T=[0,1,1,0,0,0]`) the PDF-space rule puts "Move me" sideways at view [395.8,72,415.1,129.6] (paste-text.out). The view rule lands it upright at the source's view box [72,376.95,129.57,396.19] and scans as a candidate (view-rule.out). field-add, link-add and redact-add all take view rects as given, also on p1 (fields-marks.out `P1 …`).
Amendment: "10.3 — the union, the occupancy walk and d are in view space: each page's `T = page.getTransform()`, y down from the displayed top-left. Objects enter as `editViewBox(visible, T_src)` (content-edit.ts:335); text as `bounds`; fields, links and marks as their rects. C = `editViewBox(editPageClipBox, T_tgt)`. D2's step is +10 x, +10 y (view), −10 x / −10 y at the edge, and `at` is mapped once through `T_tgt`. Writes:
- fields, links, marks: `rect + d` as is;
- objects: `inv(T_tgt)` of `view + d` → the PDF box for §1.2;
- text: `Tm = [L, o]`, with `L = textLinear × inv(T_tgt.linear)` (`editMultiply`/`editInvert`) and `o = inv(T_tgt)(origin + d)`.

§10.5 and §10.10 read 'view bounds + d ±0.5'. The clipboard keeps view rects only, and `bounds` in §10.1 is view space."

**E10-3 (blocking) §10.2, §10.4, §10.10 radio** — `FieldItem.on` is `fieldOn`, the first non-`Off` `/AP /N` key (engine.ts:1846-1853). field-defs "color" kids both carry states A, B and Off (make-field-defs.py:64-70), so both report `on:"A"` (fields-marks.out). field-add refuses duplicate kid names ("kid export names must be unique", engine.ts:2258), so the §10.10 radio green cannot pass.
Amendment: "10.1 — a radio kid's `on` is its `/AS` when that is not `Off`, else `fieldOn`. The group's value is the `on` of the kid whose `value` is true. If the `on` names are still not unique, or the group has fewer than 2 kids on this page, or `/Kids` has a kid on another page, the copy refuses `not-candidate {widgetId}`."

**E10-4 (blocking) §10.4 one mutate** — field-add (engine.ts:3508), link-add (:4206) and redact-add (:3949) each call `mutate` themselves, and a nested `beginOperation` throws (:1737). "ONE mutate … the field-add code path" leaves the builder to guess.
Amendment: "10.4 — extract `addField(doc, page, spec, write)` = the field-add body from `detect` (:3398) to the end. Keep its refusal order, and replace its `mutate(…)` with `write(work)`: the case passes `work => mutate(session, doc, "add field", work)`, clip-paste passes `work => work()`. Links and marks are inlined in paste: before `work()`, `checkLinkRect(pageBox)` + `createLink` (:4186, :4209) and `redactRect` + `addRedactPlans` (:3947-3949); no dispatch and no `session.batch`. Checks run before `beginOperation`: field names, kid names, rects and the mark cap, with each field's name picked against the pre-paste `fieldTreeNames` plus the names picked earlier in the same paste."

**E10-5 (should) §10.4 field value** — writing `/V` "through the helper `field-set` uses" does not work, for three reasons:
- field-set's permission is `form` (engine-core.ts:245), but paste checks `edit+annotate`;
- it refuses read-only fields (engine.ts:3356);
- it has no helper, only inline closures (:3363-3383).

Separately, `FieldItem.label` falls back to the name (:1897), so copying `label` writes `/TU = name` onto fields that had none.
Amendment: "10.4 — `addField` takes an optional `value` that sets the created `/V`/`/AS` (text and combo: string; checkbox: `FIELD_CHECK_ON`/`Off`; radio: `radioOn`) independently of `default` (`/DV`). It needs no field-set call and no `form` bit, and works for read-only sources. The copied label is `FieldItem.tu` (absent → no label)."

**E10-6 (should) §10.2 field gaps** (fields-marks.out):
- field-add refuses `kind:'list'`, a name containing `.` (`invalid-name`, :2080), a name over 128 chars (:172), a rect under 4 pt (:2221) and a 1-kid radio (:2249);
- `listFields` names are fully qualified (`getName()`, :1892);
- checkbox `on` is always rewritten to `FIELD_CHECK_ON`.

Amendment: "10.2 — the copy refuses `not-candidate {widgetId}` for a `list` field, a name containing `.`, and a `/Parent` that is not `isStructuralRadioParent` (field-delete's rule, :3743-3746). At paste, a picked name over 128 chars, or a rect + d under 4 pt, refuses with the field-add token plus `{index, kind:'field'}`. §10.12 gains 'a checkbox's on-state name is not kept; list boxes cannot be copied.'"

**E10-7 (should) §10.1, §10.4 tfState** — the contract's text-state fields do not match what the scanner stores:
- `ts` does not exist: nothing parses `Ts`, and `EditTextState` has no rise (content-edit.ts:273-300);
- `Tz` is stored as a factor (`value / 100`, :2722);
- `leading` may be `null` (:285);
- `fontSize` is set only for CID runs (:2425-2428); `tfSize` is always set;
- the render mode is not on the record;
- `origin` is null unless the run is first on its line (:2335).

Amendment: "10.1 — `tfState = {tc: characterSpacing, tw: wordSpacing, tz: horizontalScale*100, tl: leading ?? 0}`, `size = tfSize`. The block writes `0 Ts 0 Tr 0 g` as probed: `q BT /PF1 14 Tf 0 Tc 0 Tw 100 Tz 0 TL 0 Ts 0 Tr 0 g 1 0 0 1 72 400 Tm <4d6f7665206d65> Tj ET Q` (numbers via `editNumber`). A run with `origin == null` or `tfSize == null` refuses `not-candidate`. A risen source fails §10.5 at paste (accepted). Help ceiling: 'A run that continues a line after another run cannot be copied.'"

**E10-8 (should) §10.3, §10.4 /Rotate** — the scanner sets `geometryBlocked` on `/Rotate`≠0 or `/UserUnit`≠1 (content-edit.ts:1852). A pasted `PA<n>` on direct3 p1 is then `unsupported-transform` (:2532, :2563), so §9.6's proof abandons the whole mixed paste. Text works there (view-rule.out), unlike text-add, which refuses (engine.ts:4584; fields-marks.out). The scanner reads `/Rotate` non-inheritably (:1850); text-add uses `getInheritable`.
Amendment: "10.4 — before any write, a clipboard with an image or form refuses `unsupported-transform {index, kind}` on a target page the scan marks `geometryBlocked`. Text, fields, links and marks paste on rotated pages by the view rule of 10.3."

**E10-9 (should) §10.6 all skipped** — MuPDF journals nothing for an empty operation (empty-op.out: 0 steps), but `mutate` still does `revision += 1` (engine.ts:1752). Amendment: "10.6 — when every item is skipped, no `mutate` runs. The reply is `{ok:true, count:0, skipped, …editState}` with `revision` unchanged (a no-op: nothing dirties, no journal op)."

**E10-10 (should) §10.8 Cut** — §1.3 refuses a batch of fewer than two items (`invalid-selection`), and the single `link-delete` takes `linkId`, not `annotId` (:4219). field-delete removes one radio kid (:3740-3752), while Copy takes the whole group, so Cut + Paste would duplicate the other kids.
Amendment: "10.8 — one deletable member uses its single op (`field-delete {widgetId}`, `annot-delete {annotId}`, `link-delete {linkId}` = the `listLinks` index whose `annotId` matches, `edit-object delete`). Two or more use `edit-batch`, or `edit-objects` when all are objects. A radio member contributes one `field-delete` per kid of its group."

**E10-11 (should) §10.2, §10.4, §10.10 marks** — a search mark's rect is its quad union (redact.ts:50-53), so pasting it as an area mark covers more than the source did, including the text between lines. `addRedactPlans` takes quads (redact.ts:157). The cap check must count every pasted mark. `MAX_REDACT_MARKS` is a module const of 2000 (redact.ts:6), so "a stubbed count" cannot be built.
Amendment: "10.1 — a mark item keeps `quads` (translated by d) when `search` is true. The pasted mark is created with `addRedactPlans([{page, rect + d, quads + d}])` without the search tag. The cap is `redactMarks(doc).length + marks > MAX_REDACT_MARKS` → `too-many` (as redact-search, engine.ts:3984). §10.10's refusal builds, in the test, a hello copy holding 2000 Redact annotations (mupdf `createAnnotation` loop, saved and reopened) and pastes one mark → `too-many`."

**E10-12 (should) §10.10 field value** — field-defs "nom" has value `""` and default "défaut" (fields-marks.out). A created field already has value `""`, so the value write is never exercised. Amendment: "the text-field green first runs `field-set nom 'Zed'`, then copy → paste expects `<nom copy>` with value 'Zed' and default 'défaut'. The radio green expects the pasted group's value 'B'."

**E10-13 (nit) §10.2, §9.9, §10.12** — the Type3 clause only restates the scanner's `type3` refusal (content-edit.ts:2343). The cap is `MAX_IMAGE_BYTES` = 32 MiB (engine.ts:139): a 10 MB CJK font copies fine, only a scratch over 32 MiB refuses `too-large` at copy. Each paste grafts the font again (one graft map per paste), and `garbage=yes` (:163) does not deduplicate. Amendment: "§10.12 gains: 'each paste of a run adds its own copy of the font to the file.'"

**E10-14 (nit) §10.2, §10.4 links** — a link with `annotId 0` (a direct dict, engine.ts:646-648) cannot be named by `targets` → `not-candidate`. A page link is re-written with `fitDest` (:4190), so an XYZ zoom is lost. Amendment: "§10.12 gains 'a pasted page link opens its page at Fit'."

## checkedFine
- Hex-string `Tj` with the raw `payloadBytes` round-trips (paste-text.out):
  - simple font: direct3 "Move me" `F1` graft → scratch → `/PF1`, on p2, on a fresh page, and after a `garbage=yes` reopen: candidate, same text, view bounds + d exact;
  - Identity-H: `fixtures/p0b/edit-subset.pdf` "日本語テスト" (`cidKey 4`) pasted into direct3 p2 is a candidate with `cidWrite` (cross-document; the CID fixture is p0b, not p3).
- The record fields used exist, set at content-edit.ts:2386-2413:
  - `origin` (view space) and `textLinear = tm × lin × pageLinear`;
  - `payloadBytes`, `tfName`, `tfSize`, `characterSpacing`, `wordSpacing`, `horizontalScale`, `leading`, `depth`, `ocDepth`.
- field-add accepts "a copy" (spaces inside a name are fine). Link-add at rect + 10 lands exactly. A redact area mark round-trips its rect exactly (+10, no border growth; fields-marks.out). On p1 a view-space link, field and mark rect is stored and reported unchanged.
- Delete tokens for Cut exist:
  - `field-delete {widgetId}` (:3728);
  - `annot-delete` accepts `Redact` (:3932);
  - batch `link-delete {annotId}` per §9.13.
- `editState` = `journalState` + `pageState` (engine.ts:1522), so the §10.5 reply shape composes. Permissions (engine-core.ts:245-250): text-add `edit`, field-add `edit+annotate`, link-add and redact-add `annotate`.
- `uri` links from links.pdf p0 (`annotId 10`); the page link targets page 2, which a 1-page target skips (§10.6 holds).

## buildRisks
- Order inside `work()`: objects+text (one new stream) → the text/object rescan proof → fields → links → marks. Run the proof before the widgets are added. Whether `editStructuredRuns` sees widget appearance text is UNCHECKED; a pasted field showing the same text could otherwise turn the pasted run ambiguous.
- Grafting the font into the same document it came from duplicates the font dict. That is harmless, but `PF<n>` must be picked against the fresh `/Font` copy (§9.5), never the inherited dict.
- A leftover top-level `cm` before the new stream shifts the pasted text as it does objects (§9.6). The §10.5 proof catches it; do not try to undo the CTM.
- A radio paste reads the group's kids through `listFields` of the source page only; the kid check of E10-3 must run at copy, not at paste.
