# P3 U1b — per-occurrence copy-on-write integration (engine) — QUEUED after U2

2026-09-16 22:55, fable51. Gate evidence: `p3-unit1b-cow-probe.md` (grok) + `review-p3-unit1b-probe.md`. ds2 owns; **starts only after U2 is done** (serial on `engine.ts`). grok: fixtures/QA rows only if asked.

## Deliverable
`edit-text` (U2) accepts targets that U1 marks `shared-or-unproven-sharing` **when the sharing is one of the three proven shapes**, by privatising the path first inside the same journal operation, then editing exactly as U2 does. Anything else stays refused with the same token.

## Engine rules (ds2)
1. Sharing classification at discovery (U1 scanner) gains a `cow: "form-invocation" | "shared-contents" | "inherited-resources" | null` field on the record (not in the reply beyond `support` — the reply may add `cowable:true` so the UI can say "Editable (this placement only)"). Set only when *every* obstacle on the path is one of: (a) the run is a direct descendant of a Form XObject invoked from the page's own `/Contents` (depth 1, not nested deeper), (b) the page's `/Contents` element holding the operator is referenced by >1 page, (c) the page's `/Resources` are inherited. Nested Forms (depth ≥ 2), Forms invoked from other Forms, or resources shared via a Form's own `/Resources` stay `null` → refused.
2. Privatise inside `mutate("edit text")`, before the byte rewrite, in this order: inherited `/Resources` → copy the dict onto the page (`page.put("Resources", copy)`); shared `/Contents` element → `addRawStream(readRawStream(), dictCopy)` and replace that element in the page's `/Contents` (array element or scalar); Form invocation → `newGraftMap().graftObject(form)`, register under a fresh name `<name>c<n>` in the page's own `/Resources/XObject` (never clobber an existing name), rewrite **only the selected `Do` operand** (byte span from the record) to the new name, then edit the copy's stream. All writes are byte-span replacements in streams the page now owns.
3. After privatising, re-resolve the target on the *copy* (recompute the record for the copied stream: same relative offsets, new object number) and apply the U2 write, width gate and tag gate unchanged. One operation, one undo step; undo must remove the copy, the resource entry and the `Do` rename (probe shows the journal covers this).
4. Never touch the original Form/stream. Other placements and other pages must be byte-identical afterwards (test by comparing their decoded streams before/after).
5. Bounds: one privatisation per edit; the copied stream counts toward the 2 MiB scan budget; refuse `too-large` beyond it.

## Tests (`tests/p3-edit.test.mjs` additions, ds2)
`fixtures/p0b/edit-shared.pdf`: edit the first `/Fm1` placement → p0 extracts BRAVO+ALPHA+NEIGHBOR, p1 PAGE2, two distinct Form objects, `Im1` untouched; undo → original bytes, no `/Fm1c`; save → `recover.py` finds ALPHA only in the surviving placement. `fixtures/p3/targets-streams.pdf`: first `/Fm1 Do` editable, nested `Fm2→Fm1` still refused, p2's shared `Fm1` unchanged. A synthetic 2-page shared-`/Contents` PDF (as the probe) and an inherited-resources variant. Refusals: depth-2 descendant, Form-owned shared resources.

## Acceptance
Parent §5 U1b row. Evidence `p3-unit1b-ds2.md`; reviewer `review-p3-unit1b.md`; `npm test` count as measured. Stop on any change to a non-selected placement.
