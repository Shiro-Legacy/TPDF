# P6 U4 objections — three Opus 5.5 lenses (engine, renderer/UX, tests-drive), 2026-09-24
Raw structured reports: `objections-p6-unit4-opus55.json` (same directory; per lens: objections, checkedFine, buildRisks, refuter verdicts). Verdicts by fable51 below; accepted items are folded into `contract-p6-unit4.md` §9 (9.1–9.23, plus 9.24 from the critic pass at the end of this file), which overrides §1–§8 where they differ.

**Counts.** 38 objections: engine 9 (E0–E8), renderer/UX 12 (R0–R11), tests-drive 17 (T0–T16). Raised severity: 9 blocking, 18 should, 11 nit. All 9 blocking items were confirmed by an independent refuter (none refuted); the refuters kept 4 as blocking (E1, R3, T0, T1) and downgraded 5 to should (E0, R0, R1, R2, T2). Verdicts: **33 ACCEPT, 5 MODIFY (E1, R3, R7, T8, T15), 0 REJECT.** 23 amendments (24 after the critic pass below). Ids are 0-based, as in the JSON.

## Engine lens
| id | § | sev | verdict | §9 |
|---|---|---|---|---|
| E0 | 1.2 | blocking → should | ACCEPT — `failLink(detail, message)`, reason `invalid`, token in `detail`, as `failRedact` (with E3, T3) | 9.3 |
| E1 | 1.1/1.3 | blocking | MODIFY — the lens rule (direct entries, `/AA`, normalised key rect, cursor stays on a no-match) plus: the one `linkPairs` helper for `listLinks` and `link-set` that the refuter dropped is restored (the lens's own build risk: two code paths can edit a different entry than the panel shows); `/A /S /JavaScript` entries are skipped (mupdf does not list them, lens scratch t.mjs); the helper never throws; object numbers pinned in provenance (with T5) | 9.1 |
| E2 | 1.2 | should | ACCEPT — full check order incl. requireMutable/checkPage and the unpaired-`/Link` case | 9.2 |
| E3 | 1.2/6.2 | should | ACCEPT — link-add's wording with op `link-set` (with E0, T3) | 9.3 |
| E4 | 2.1 | should | ACCEPT — `/K`-only walk, not redact.ts `walkObject` | 9.11 |
| E5 | 5 | should | ACCEPT — no page `/StructParents`; ParentTree 0/1/2 (with T6, E8) | 9.17 |
| E6 | 1.3/6.1/7 | nit | ACCEPT — pin moves to p6-links; engine drops p2-shell | 9.4, 9.22 |
| E7 | 1.3 | nit | ACCEPT — reopen through `openBytes`, never raw `reopen()` | 9.4 |
| E8 | 5 | nit | ACCEPT — `/DR /Helv` + widget `/DA` as make-form.py | 9.17 |

## Renderer/UX lens
| id | § | sev | verdict | §9 |
|---|---|---|---|---|
| R0 | 3.4/1.6/3.3 | blocking → should | ACCEPT as refined by the refuter (canvas box recomputed after the scroll); refuter's correction noted: an armed drag over a field is a silent miss today, not a field move | 9.15 |
| R1 | 1.6/3.3/6.2 | blocking → should | ACCEPT — 0.01 pt rounding in `startAreaDrag`, and no gesture-derived rect in any expect (with T7) | 9.8 |
| R2 | 1.4/1.5 | blocking → should | ACCEPT + refuter's `syncLinkCurrent()` in `dropDirectFocus` (with R4, R5, T2) | 9.6 |
| R3 | 3.1/6.2 | blocking | MODIFY — no new `driveDirectKey` branch: once the field button's keydown runs arrows through `runGestureWork`, the existing fallback (renderer.ts:1290-1292) dispatches to the focused button and awaits `directGestureWork`; the nudge target is reached by Tab (T0), since a click commits the checkbox/radio (an extra journal op) and a text field's click opens its editor; the refuter's size-kid point (169,559) is y-up — y-down is (169,233), and Tab replaces the click anyway | 9.12, 9.16 |
| R4 | 1.5 | should | ACCEPT — explicit `link` branch in every `directActive` switch; class `inline-editor link-editor` (with T2) | 9.6 |
| R5 | 1.8/3.1 | should | ACCEPT — `onTarget` selector gains `.link-target` (with T2) | 9.6 |
| R6 | 3.3 | should | ACCEPT — pointerdown order, `canAnnotate()`, preventDefault on pointerdown, disarm in `cancelPendingTools`, `driveArm` `link` | 9.14 |
| R7 | 1.4 | should | MODIFY — `linksPage` is set on the no-document, catch and success exits, but the stale exits (`seq`/`generation`/`wantedPage`) leave `links` and `linksPage` alone (setting them there would paint another page's links; the newer refresh sets them), and the catch sets them only while its `seq` is current; the wait half goes to 9.15 | 9.5, 9.15 |
| R8 | 3.2 | should | ACCEPT — `:not(:hover, :focus-visible, .selected…)` so focus/selection outlines win | 9.13 |
| R9 | 1.7 | should | ACCEPT — `page` 1-based, sent as `page - 1` | 9.10 |
| R10 | 3.2/7 | nit | ACCEPT — checkbox in `#context-edit` after the hint, `aria-label`; `label[for=link-kind]` | 9.9 |
| R11 | 1.4/1.8 | nit | ACCEPT (with T4); its evidence is wrong — `commitMarkRect` (renderer.ts:2483-2493) does not set `navLast`, so the rule is new, not mirrored | 9.7, 9.8 |

## Tests-drive lens
| id | § | sev | verdict | §9 |
|---|---|---|---|---|
| T0 | 6.2 | blocking | ACCEPT — form2.pdf, Tab to agree/size (with R3, T1) | 9.12 |
| T1 | 3.1/6.2 | blocking | ACCEPT — `runGestureWork` in the field button keydown; text/choice fields ⌥-only (with R3, T0) | 9.12, 9.16 |
| T2 | 1.4/1.5/1.8/3.1 | blocking → should | ACCEPT (with R2, R4, R5) | 9.6 |
| T3 | 1.2/6.2 | should | ACCEPT — exact error string + `nav.last.detail 'scheme'` (with E0, E3) | 9.3 |
| T4 | 1.8/1.4/6.2 | should | ACCEPT — every link-set path sets `navLast` (with R11) | 9.7 |
| T5 | 5/6.2 | should | ACCEPT — provenance + header pin object numbers; drawn links omit `annotId` (with E1) | 9.1 |
| T6 | 5/2.3 | should | ACCEPT — `/Annots` orders that differ from every alternative (with E5) | 9.17 |
| T7 | 6.2 | should | ACCEPT — geometry via `pick()`/`near()` (with R1) | 9.8 |
| T8 | 3.4 | should | MODIFY — the drag-only fallback is adopted, but a `handle` miss stays a silent no-op rather than a new drive error (U3 §9.12 already requires an expect on every gesture step, which goes red); of its two options, scroll-into-view (as `driveDirectClick`) is chosen over dispatching on the canvas, so the real `elementsFromPoint` hit test stays valid | 9.15 |
| T9 | 6.2 | should | ACCEPT — `revision 0`/`dirty false`/`page 0` after reopen, no `wait` | 9.18 |
| T10 | 6.3 | should | ACCEPT — one Electron per fixture (with T11) | 9.19 |
| T11 | 6.3 | nit | ACCEPT — `pagePoint(x, y, widthPt = 612)`, no rotated GUI shot | 9.19 |
| T12 | 6.2 | nit | ACCEPT — `contents`/`ms`/`path` JSONL syntax | 9.18 |
| T13 | 6.2 | nit | ACCEPT — `dx 1 dy 0` below the threshold, applied to the link-draw drag too | 9.18 |
| T14 | 6.4 | nit | ACCEPT — appended section, not rows | 9.20 |
| T15 | 8 | nit | MODIFY — the claim is wrong: accept-u4.sh:8 counts `"ok": false`, not `"red"`, already prints `exit $?`, and :10 already labels links.pdf/tabs.pdf as the expected additions. Real gaps: that count adds one for the top-level `ok` on a red run and misses step timeouts, and nothing re-checks links.pdf/tabs.pdf after the drives (the suite with `PINNED_SHA256` runs before them). Fix: `exit 0` is the gate, and `shasum -a 256 -c checks/fixtures.sha256` runs after the drives — no 52-file snapshot needed. No agent edits the script; fable51 patches it | 9.21 |
| T16 | 5/7 | nit | ACCEPT — hash map covers links, tabs, direct3, form2 | 9.18 |

Rejected: none.

## Refuter verdicts on the blocking items (all confirmed)
- **E0** — confirmed, downgraded to should: `failRedact` sets reason `invalid` and the token in `detail` (engine-core.ts:143-148); `SaveReason` has no link tokens (engine.ts:187-189); §1.7/§1.8 already read `nav.last.detail`; the map's `failLink` lacked `target`.
- **E1** — confirmed, blocking: rerun against mupdf 1.28.1 — the contract rule gives [5,0,0,0,0] (and [0] on the rotated page), the amended rule [5,0,6,7,9] and [13]; `Rect.transform` returns an invalid rect untransformed (mupdf.js:114-115); the refuter dropped the one-helper sentence, restored in 9.1.
- **R0** — confirmed, downgraded to should: `driveDirectGesture` never scrolls (renderer.ts:1228-1250); an armed drag over a field is a silent miss today (`handle = target`, pointer-events:none), not a move; the box must be recomputed after the scroll.
- **R1** — confirmed, downgraded to should: only the renderer-held `nav.linkDraft.rect` is at risk; engine rects come back through float32 (0 misses in 3678 cases); rounding in the helper is harmless.
- **R2** — confirmed, downgraded to should: `clearDirectFocus` (:2104) misses the link, `restoreDirectFocus` (:2346-2376) has no link branch, `target.focus()` (:2374) has no `directActive` guard; adds `syncLinkCurrent()` in `dropDirectFocus`.
- **R3** — confirmed, blocking: a text click opens the editor, a checkbox/radio click nulls `directFocusedField` (:2535), the field keydown's `void nudgeField`/`void moveRadio` race the STATE line, form.pdf has no radio; its size-kid point is y-up (corrected in the R3 row).
- **T0** — confirmed, blocking: make-form.py builds one /Tx widget; Tab reaches agree/size with no editor (`focusNextField` → `activateField(item, false)`); ArrowUp is −1 in y-down; `moveRadio` +1 from S gives `M`.
- **T1** — confirmed, blocking: a text field always opens its editor (renderer.ts:2526-2542); the race is real only after a checkbox/radio click; `runGestureWork` in the field button handler is the fix.
- **T2** — confirmed, downgraded to should: Enter on a focused link is swallowed (selector at :1274, Enter-with-nothing-active return), Tab calls `focusNextField`, `finishDirect`/`dropDirect`/`directUnchanged` fall through to `applyContentEdit`; Delete/arrows are not lost (fallback), and §1.8 already fixes the `direct.active` shape.

## Critic pass on §9 (2026-09-24)
An independent critic reviewed §9 and this file and raised 10 items (0 blocking, 4 should, 6 nit). I checked each one against renderer.ts and the tests it cites. All 10 are applied: 9 as proposed and 1 modified (C7, where the editor line is 2543, not 2549). None was rejected.

| # | sev | where | verdict | §9 |
|---|---|---|---|---|
| C1 | should | `linksPage`, `navLinkDraft` and `directFocusedLink` are not reset on open (`resetEditState` has only `fieldsPage = -1`, renderer.ts:847) | ACCEPT — reset all three there; the `#link-dialog` close listener (6879) also clears the draft | 9.5 |
| C2 | should | a drive `key Escape` does not close the draw dialog (Escape branch, renderer.ts:1287) | ACCEPT — the parenthetical is dropped and `arm {tool:'off'}` is named as the drive cancel | 9.18 |
| C3 | should | a plain-arrow nudge on any field kind contradicts 9.16 (a text button can hold focus without an editor, 2280, 2529-2530) | ACCEPT — plain-arrow nudge applies to checkboxes and radios only | 9.12, 9.16 |
| C4 | should | the progress.md ceiling line has no owner, and it is misnamed "JavaScript-link" | ACCEPT — fable51 owns it at acceptance; renamed "dropped-`/Link`" | 9.16, 9.22 |
| C5 | nit | the `#error` precedent in `link {action:'delete'}` does not exist (1160-1163) | ACCEPT — exact text `link: no link ${index} on this page` on the set path only | 9.10 |
| C6 | nit | whether a sub-threshold armed drag disarms is unstated; `redactDrag` has more references than `layerLive` | ACCEPT — `onEnd` disarms on every pointerup; all references are listed (707, 869, 2113, 3963, 6974-6980, 7000-7007) | 9.8, 9.14 |
| C7 | nit | the `tabOrder` call sites are unnamed | MODIFY — the editor's `directFieldIndex` is at 2543, not 2549; the sites are 2499, 2511, 2527 and 2543, and `fieldsTabs` is reset on open | 9.24 (new) |
| C8 | nit | a plain arrow nudges a field in Read mode (fields are painted there, 2267; `nudgeField` has no mode check, 2143) | ACCEPT — the plain-arrow field nudge applies in Forms mode only, and Help says so | 9.12, 9.16 |
| C9 | nit | the `navLast` type (703) rejects the `link-follow` and `link-draw` shapes | ACCEPT — the type is widened | 9.7 |
| C10 | nit | 9.4 would rewrite the existing p2-nav `reopen()` tests | ACCEPT — only new save → reopen tests go through the engine; the existing ones at 520 and 573 stay | 9.4 |
