# P3 U1 Gate A — journal / abandonOperation

2026-09-16. grok. **No `src/` edits.** Probe `scripts/p3/journal-probe.mjs`. JSON `.quad/shared/p3/unit1/journal/journal-probe.json`.

Command: `node scripts/p3/journal-probe.mjs .quad/shared/p3/unit1/journal`  
mupdf 1.28.1 wasm, Node v22.23.1, save `incremental=no,garbage=yes`. Fixtures `edit-transformed.pdf`, `edit-tagged.pdf`. **20 pass / 4 fail** (exit 1). The four fails are redo-wipe after written abandon. Tagged stext/ActualText mismatch is a recorded limitation, not a journal fail.

## Verdict

**Content rollback is proven. Redo-history preservation after a written-then-abandoned op is not.** Do not invent snapshots. ds2: do **not** land Gate C until astra/fable51 say the redo-wipe ceiling is accepted.

## Native call sequence (measured)

```
pdf.enableJournal();
pdf.beginOperation(op);
try {
  work();                 // real writeStream / dict put / addStream
  pdf.endOperation();     // exactly one journal step
} catch (e) {
  pdf.abandonOperation(); // closes the op; rolls back writes
  throw e;
}
// never end+abandon; never abandon twice
// never getJournal/canUndo/canRedo while an op is open
```

| Call | Result |
|---|---|
| `abandonOperation` after writes | restores stream bytes + resource dict; closes op |
| `abandonOperation` twice / with no op | throws `Can't abandon a non-existent operation!` |
| `endOperation` after abandon | **no-throw** — do not call; not a second rollback |
| `getJournal` / `canUndo` during op | throws `Can't undo/redo during an operation` |
| begin + abandon, **no writes**, redo available | redo **kept** |
| begin + **any write** + abandon, redo available | streams restored; **redo wiped** (`steps=[]`, `canRedo=false`; `redo()` → `Already at end of history`) |
| `endOperation` after a partial write, then `undo()` | content restored but failed op sits on the redo stack — **not** a substitute for abandon |

## Assertions

Pass: one journal step for contents rewrite + new `/XObject /U1Probe`; undo/redo restore bytes and resource association; save → undo → save → redo → save on the **same** document (no reopen, no `compact`); pypdf independent; `recover.py ALPHA` on edited transformed = 0 raw/hits; qlmanage 0; clean-baseline abandon (first write and later write) restores content **and** journal; tagged stream rewrite (pypdf BRAVO, QL 0).

Fail:

1. `abandon-redo-first-journal` / `abandon-redo-later-journal` — native redo wipe after writes (see above).
2. `abandon-redo-*-redo-still` — `redo()` after that wipe: `Already at the end of history`.
3. `tagged-extract` — mupdf stext still `ALPHA` because `/ActualText` was not updated. **Not a journal failure.** pypdf stream has BRAVO, not ALPHA. Gate A did not rewrite ActualText.

## Independent inspect

| file | pypdf text | stream ALPHA/BRAVO | U1Probe |
|---|---|---|---|
| `transformed-edited.pdf` | BRAVO NEIGHBOR | BRAVO | yes |
| `transformed-undone.pdf` | ALPHA NEIGHBOR | ALPHA | no |
| `transformed-redone.pdf` | BRAVO NEIGHBOR | BRAVO | yes |
| `abandoned-clean-*.pdf` | ALPHA | ALPHA | no; `U1-ABANDON-7731` absent |
| `tagged-edited.pdf` | BRAVO NEIGHBOR | BRAVO | — |

QL: `.quad/shared/p3/unit1/journal/ql/{transformed,tagged}-edited.pdf.png`.

## What ds2 may use vs not

Use: `abandonOperation` in `mutate()` failure path; revision bump only after `endOperation`; do not `end` in `finally`.

Do not assume: a failed mutation after the user has undone something will keep that redo branch. Native drops it as soon as the abandoned op wrote anything. Empty (no-write) abandon keeps it — so keep validation **before** `beginOperation`.

U1 acceptance of the transaction gate is astra/fable51’s call on that ceiling. Read-only discovery (Gate B) is not blocked.
