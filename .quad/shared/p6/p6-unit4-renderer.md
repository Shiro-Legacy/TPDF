# P6 U4 renderer evidence (Opus 5.5 renderer seat, 2026-09-24)
Files: src/renderer/renderer.ts, src/renderer/index.html, src/renderer/styles.css (appended block), tests/p6-direct.test.mjs (additive tabOrder asserts). preload.cts / main.ts unchanged.

## Shipped (contract §1.4–§1.8, §2.2–§2.3, §3.1–§3.4, §4, §9.5–§9.16, §9.24)
- Links on the page: `paintLinkTarget` (button.hit-target.link-target[data-annot-id][data-link-id], role=link) painted before fields in Read/Annotate only when `linksPage === page === wantedPage`; blocked dotted; Read click → `followLink` (nav.last link-follow page/uri/blocked, blocked → announceNav "This link's scheme (ftp:) is not one this app opens").
- Annotate: click focus (`directFocusedLink`, `syncLinkCurrent` → Links row aria-current), grip/body/8 handles via startRectGesture min 4 → `commitLinkRect` → ONE `link-set {rect}` (1e4 round, noop writes nothing, refusal snaps back + #error), plain/⌥ arrows nudge (Shift 10), Delete/Backspace → `deleteLink` (focus drops), Enter/dblclick → inline editor `input.inline-editor.link-editor` (digits → page, else uri; refusal keeps draft; Escape closes; blur/Tab commit; undo closes), `directActive.kind 'link'` in every switch.
- `refreshLinks` records `linksPage`, repaints; catch/stale per §9.5; `resetEditState` resets linksPage/navLinkDraft/directFocusedLink/fieldsTabs; dialog close nulls the draft.
- Draw link: `button#link-draw` (Links tab, aria-pressed), `select#link-kind` in #link-dialog, `startAreaDrag` (window listeners, no capture, 3 px, 0.01 round, addBlocked, onEnd disarm, dashed draft box), < 4 pt → "Draw a bigger link"; `driveArm` link; `driveLink` uses the draft; `link {action:'set'}` (§9.10, no-such-link + #error).
- `tabOrder` exported (R/C/S/A/W/fallback, radios once, signatures out); focusNextField + both activateField indexes use it; STATE fields.tabs/tabsOrder/order.
- Field keydown §9.12 order through runGestureWork; object/mark plain arrows; ⌘/Ctrl-arrow without ⌥ never consumed.
- `#edit-outlines` in #context-edit (localStorage `edit-outlines`, default on), `#hit-layer.outlined` CSS per §9.13; STATE direct.outlines; drive `outlines {on}`.
- Redact: armed + unarmed drags on `startAreaDrag` (`redactDrag` removed everywhere, setPointerCapture gone).
- driveDirectGesture §9.15 (wait fields+links, scroll, 2 rAF; drag on empty/armed → pageRegion pointerdown + window move/up).
- STATE: top-level `links[]`, direct.links, direct.outlines, direct.active kind 'link', nav.linkDraft, nav.last widened. Help: Direct editing cells + ceilings <ul> only (rows stay 11).

## Proof (all under the lock)
- `npx tsc --noEmit` clean; `npm run build && node --test tests/p6-direct.test.mjs tests/p2-shell.test.mjs tests/p3-drive.test.mjs` → 27 pass, 0 fail.
- `node scripts/drive.mjs` exit 0: direct-edit4 (65 steps, 0 red), direct-edit5 (27, 0 red), direct-edit, direct-edit2, direct-edit3, links-bookmarks, field-design, help-fields. Reports under .quad/scratch/u4r/.
- direct-edit4 geometry seen: L1 grip +30/+40 → [102,112,230,132]; se +20/+10 → [72,72,220,102]; ArrowRight +1, Shift+ArrowDown +10; drawn [300,400,420,420]; rotated L4 grip +10 → [710,72,730,200].
- `shasum -a 256 -c checks/fixtures.sha256` 6/6 OK, `u4-hash-before.txt` all OK; no --drive= stragglers.

## Not done / notes
- No GUI (CDP) run from this seat (shot-direct4 is the tests seat's). dropDirect of a link editor on a mode/page change shows a refusal in #error rather than silently (commit path shared with Enter).

## Fixes after review

- R1 (§9.6) reproduced (probe b20: #error + STATE.error "link-set: ftp: ..." after a mode change). Fix: `linkSet(..., silent)` and `commitLinkEditor(active, silent)` skip showError when silent (nav.last.detail still set); dropDirect passes `true`. Probe b20 now OK.
- R2 (§1.5) reproduced (probe b26: toolbar Undo click committed link-set, rev 1). Root cause is two paths: an enabled Undo's mousedown blurs the editor (blur -> finishDirect), and a disabled Undo (fresh doc, the reviewer's case) gets no mousedown/click at all but the press still blurs. Fix at the listener block: undo/redo `mousedown` preventDefault while directActive; window capture `pointerdown` on a disabled #undo/#redo closes the editor with no request. Probe b26 OK; scratch probe .quad/scratch/r2-enabled-probe.mjs (enabled Undo: first click closes with no request, second click undoes) OK.
- Verified under lock: tsc clean; build; p6-direct + p2-shell 6/6; drive direct-edit4/5/3 and links-bookmarks exit 0; fixture hashes unchanged (only links.pdf, tabs.pdf new). Probe h1 (Help rows) was already BAD before this change and is outside R1/R2.
