# Debt sweep 2 map (Explore/Opus, 2026-09-24) — file:line for every nit

## N1 edit-batch forwards extra item keys
- engine.ts:477-485 EditBatchItem union: edit-text {targetId,move}; field-props {widgetId,rect}; field-delete {widgetId}; link-set {annotId,rect}; link-delete {annotId}; redact-set {annotId,rect}; annot-delete {annotId}; edit-object {targetId,action,box?}.
- engine.ts:3797-3809 shaped() checks required keys only; only extra-key check is edit-text `!("text" in item) && !("size" in item)`.
- engine.ts:3894 leak: `dispatch({ id: request.id, page, ...item } as SessionRequest, session)` — item's own id/page override, other keys reach the single op.
- Inline (not dispatched): edit-text :3850-3858, edit-object :3859-3872, link-delete :3880-3887/:3927-3931.
- Dispatched single ops read: field-props :4348 (type :338: page, widgetId, name, rect, maxLen, multiline, options, label, readOnly, required, default, edit — `name` renames!); field-delete :4510 (:342 page, widgetId); annot-delete :4708 (:301); redact-set :4734 (:304 page, annotId, rect); link-set :5015 (:363 page, annotId, rect?, target? — extra `target` retargets). Single link-delete :4998 takes linkId (:361).
- Fix: shaped() rejects keys outside a per-op allow-list, or build the dispatch request from those keys only at :3894.

## N2 pasted text drops rise (Ts) and render mode (Tr)
- engine.ts:3591 block: `q BT /F size Tf tc Tc tw Tw tz Tz tl TL 0 Ts 0 Tr 0 g … Tm <hex> Tj ET Q`; engine.ts:3398 tfState {tc,tw,tz,tl} (type :812).
- content-edit.ts:280 scan state has renderMode (set :2714-2717 case "Tr"); :2324 candidate only if renderMode 0|1|2; record (:2398-2406, fields :188-191) copies horizontalScale/characterSpacing/wordSpacing but NOT renderMode. No "Ts"/rise tracking anywhere.
- Help: index.html:569 "Pasted text keeps its font and size but is black; kerning is dropped."; pin tests/p2-shell.test.mjs:213.
- Fix options: refuse copy of a run with Tr≠0 (and track rise), or Help + pin: "rise and outline mode are dropped".

## N3 p6-objects7 refusals without nothingWritten
- :401 test "U7 §1.3 refusals…": :410-415 `restrict(f); const member = batch(f, 0, [...])` not wrapped. Fix: `still(f, () => batch(...))`.
- :638 "D4 copy-denied": :647-651 `const member = paste(s, 2)` not wrapped. Fix: `nothingWritten(s, 2, () => paste(s, 2))` (helper :81-89 checks the given page; still :158-165).

## T-4 no Meta+c / Meta+x drive line
- direct-edit8.jsonl:60 and direct-edit8-cut.jsonl:15,32,45 use Meta+v only. design-agent-drive.md:58 advertises `key Meta+c|Meta+x|Meta+v`. Handler renderer.ts:1407-1411 DRIVE_CLIPBOARD → ClipboardEvent on activeElement.
- Fix: new drive lines (append-only to direct-edit8-cut.jsonl or a new JSONL) with Meta+c and Meta+x; p6-drive8 pins move.

## T-5 p6-drive8 S45
- direct-edit8.jsonl:60 = command #45 (Meta+v "Move me" count 1); p6-drive8.test.mjs:132 asserts `/\.6c51e7e2$/` for S43 only; :134 `has(runs(S(45)), MOVE_ME, …)` no suffix assert. Also :168 mark 2 not rechecked after undo ×2.
- Fix: add find + assert.match for S45; add `has(markRects(S(96)), [300,600,400,630], …)`.

## T-6 shot-direct8 saves OS clipboard as text
- :274 `savedClipboard = execFileSync('pbpaste',…)`; :337 restore with pbcopy; header :10.
- Fix: document the text-only ceiling in the header (lazy) or save all flavours.

## T-7 "1 text run stay"
- renderer.ts:3062 `Cut ${plural(rest.length,"item")}; ${plural(runs,"text run")} stay (they cannot be deleted)`; pins direct-edit8-cut.jsonl:31 and checks/review-u7-renderer-probe.mjs:304.
- Fix: `${runs===1?"stays":"stay"} (${runs===1?"it cannot":"they cannot"} be deleted)`; update both pins.

## U6-N1 im3 — already fixed (p6-objects6:349-358). U6-TS2 done (shot-direct7:276-277). TS4 done (one 'before' log :196).
## U6-N2 p6-objects6:232 `transform(s, sc, 'Im1', 'spin')` sends no box. Fix: valid box e.g. [90,0,110,200] so only the transform fails.
## U6-TS3 shot-direct7.mjs:3 header cites §9.19/§9.30/§9.33 — add §9.34.
## U6 clip ceilings → Help: the Direct editing <ul> at index.html:569 (21 li, id-less); `#help-ceilings` (:571-579) is the Form designer list (7 li). No li-count pin; strings pinned at p2-shell:205-222 via includes. Fix: add two li to :569 + strings to the :205 array. Ceilings text (progress §10 U6 N3/N4): a clip set in top-level state before a block's q is not seen, so that block's box is the full rect; once `full` leaves the page a clip cannot be removed (already partly in Help: "Once a picture's full box leaves the page…").
## U6 Inspector float rounding: renderer.ts:4662-4665 openContentObjectForm writes String(box[0]) etc.; round2 at :5885 (used by field form :5969-5972). Fix: round2() on the four values.
