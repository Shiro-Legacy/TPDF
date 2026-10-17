# M10: save failures and the fuzz campaign

October 17, 2026, branch `app-p10-fuzz` from 034eead. Two parts: save failure and recovery tests (the
app's `src/safe-write.ts`), and a campaign with the existing fuzzers on the current head. No new
fuzz or mutation code was written. The campaign tools are copied, unchanged except for paths, into
`engine/tools/fuzz/` (origin and edits in its README). No corpus file, and no document bytes, are
in this note or the repository; every input is synthetic or derived from the repository's `fixtures/`.

## Save failures (app)

`writeCopy` writes an exclusive temp (0600), fsyncs it, rereads and hashes it, checks that the source is
unchanged (in-place Save), renames it over the target and fsyncs the folder; the journal
(`pending-saves.json` in userData) lists the temp while the save runs, and `cleanupOrphans` removes a
listed temp at the next start. The renderer marks the document clean only on `result.ok` and otherwise
shows `SAVE_REASON_TEXT[reason]`.

Fixed:

- **A read-only file was replaced.** `rename(2)` replaces a 0444 target whenever its folder is
  writable, so Save on a read-only file, and Save As over an existing read-only file, silently
  overwrote it. It is the same before and after the engine move (the save path is the app's, not the
  engine's; `safe-write.ts` changed since the first commit only in the Windows port, 14e0630, which
  skips the folder fsync on win32). `writeCopy` now refuses with
  `permission` when the target exists and `access(W_OK)` fails (EACCES, EPERM, EROFS), checked right
  before the rename, so a target made read-only during the write is refused too. Save As can still
  write elsewhere: the user (or another app) marked the file read-only, and Save should not undo that.
- **A temp that could not be created left a journal entry.** In a read-only folder the journal entry
  was added before the create failed, and only `tmp` (unset) was dropped afterwards; the entry stayed
  until the next start. Every journalled name is now dropped.
- **Texts.** `SAVE_REASON_TEXT` is typed `satisfies Record<SaveFail, string>`, so a new reason without
  text fails the build, and has text for `cancelled`. The texts no longer say "the copy" (in-place Save
  shows them too) and say the file on disk was not changed: `permission` reads "The file or its folder
  is read-only, or Tandem PDF is not allowed to write there. The file on disk was not changed; use Save
  As to save elsewhere."
- **In-place Save through a symlink replaced the link** (review round 1, pre-existing): the temp was
  renamed over the link, so the link became a regular file with the save and the real file kept the
  old bytes. In place, `writeCopy` now resolves the target with `realpath`, makes the temp beside the
  real file, checks the real file unchanged and renames over it; the result keeps the path and name
  the window opened. `realpath` resolves Windows symlinks and junctions alike.
- **Review round 2.** The success result carried `inspectRegularFile(dest)`, the identity of whatever
  the link named afterwards: a link retargeted to another file during the write left the save in the
  first file but the second's identity in the window, so the next Save passed the unchanged check and
  overwrote the second. It now carries the identity of the file written (`destPath`) under the path
  opened. Save As and new files onto a symlink were refused as `same-file` whatever the link named;
  they now write through to the file it names (`resolveTarget`), as in-place Save does, and stay
  `same-file` only when that is the open document. A link to nothing (or a loop) is refused with a new
  reason, `broken-link`: "That name is a link to a file that no longer exists, so nothing was
  written. Choose another name." In place, a link to nothing reads as `source-changed` (the file was
  deleted).
- **Exports and signing showed "(unreadable)"** (review round 1): `save-new` and `sign-file` in
  `main.ts` mapped `permission` and `disk-full` to `unreadable`. They pass both through, and the
  renderer shows "The PNG was not saved." or "Signing failed." followed by the reason's text; for
  `permission` that text ends "Choose another folder." (round 2: exports have no Save As).

Tests (`tests/m10-save-failure.test.mjs`, 20; 14 in round 1, 18 in round 2): every `SaveFail` has plain non-empty text and the three
copies of the `SaveResult` union (safe-write, preload, renderer) agree; a read-only file in place and
as a Save As target; a read-only folder (in-place and Save As); the target made read-only, deleted
(in place: `source-changed`, not recreated; Save As: the copy is written) or replaced (in place:
`source-changed`, the other file kept; Save As by a hard link to the open file: `same-file`) inside
the `writeAll` hook; a child node SIGKILLed inside the `rename` hook (the original byte-identical,
one fsynced temp left and listed, `cleanupOrphans` removes it, and so does the app's next start with
that userData); and three driven-window cases (read-only file, read-only folder for Save and Save As,
the file replaced or deleted on disk before Save) that read STATE: `save {ok:false, reason}`,
`dirty: true`, `status: "Not saved"`, `error` equal to the reason's text. Round 2 added in-place Save
through a symlink (the real file, in another folder, gets the save; the link stays a link; a second
Save through it works), a symlink to a read-only file (refused), Save As onto a symlink (written through
to the file in another folder, the link kept; a link to the open document `same-file`; a link to
nothing `broken-link`), a link retargeted inside the `writeAll` and the `rename` hooks (the first file
gets the save, the next Save refuses the second as `source-changed`), and an export into a read-only
folder (the permission text, no "Save As"). `tests/p1-save.test.mjs`'s symlink case now expects the
write-through. The read-only
cases failed before the fix (Save returned `ok: true`). The app gate runs with `CI_NO_ELECTRON`,
which skips the five Electron tests; they pass when run with Electron (`node --test tests/m10-save-failure.test.mjs`).

Not covered: a full disk and EIO from a real device (injected only, `tests/p1-save.test.mjs`), network
and FUSE folders, Windows (ACLs and the read-only attribute; the permission tests skip there and for
root), a power cut between the rename and the folder fsync (`durability-uncertain` is tested only by
injection).

## Campaign

Seeds are new (none used by the earlier runs). Every chunk ran through `tools/slot` (3 GB cap, two at
a time machine-wide) with a 1,200 s alarm (1,800 s for the Rust tools); the machine's load average was
about 7 throughout (other lanes). Run twice: on 034eead plus the save fix ("before"), and on the head
with the engine fixes below ("after"); the Rust tools only once more after the repair fixes (the
`/Contents` fix is in the app's view, which they do not use).

| Tool | Seeds × inputs | Inputs |
|---|---|---|
| `p7-fuzz.mjs` (open, authenticate, security-set, save, validate on mutated fixtures and MuPDF-written encrypted/object-stream files) | 31-38 × 500 | 4,000 |
| `p22-lexfuzz.mjs` (damaged tokens in a 3-page file's dictionaries) | 301-306 × 500 | 3,000 |
| `p25-bytefuzz.mjs` (1-2 byte edits in dictionaries) | 7-14 × 400 | 3,200 |
| `q3-cutfuzz.mjs` (one object cut short, moved) | 4-9 × 300 | 1,800 |
| `p6_fuzz.rs` (render robustness: colour, shading, tiling, image at 0.5 and 2 and text; mesh) | color/shading/tiling/image 6-7 × 600; mesh 4-5 × 200 | 5,200 |
| `mutation-smoke.py` (`pdf-inspect` --document/--fonts/--content/--path) | 2026101701, 2026101702 × 1,500 | 3,000 |
| `r1`…`r6` review probes (fixed sets, no seed) | — | 92 rows |

20,200 generated inputs per run. Reproduce: `engine/tools/fuzz/README.md`, with the seeds and counts
above. `p7-fuzz` is reproducible only up to MuPDF's encryption: three of its seed files are written
by MuPDF with a random file key and IVs on every run, so its counts move by a few percent between
runs (the reviewers' `p7pair` caches those files for that reason).

### Outcomes

| Outcome | Before | After |
|---|---|---|
| Trap (Rust panic, WASM `unreachable`, engine stopped) | 0 | 0 |
| Hang (alarm) | 0 | 0 |
| Memory cap hit | 0 (peak 484 MB, `p6` mesh) | 0 (peak 393 MB) |
| Over 2 s | 5 `p6` mesh cases (2.0-3.4 s; rerun alone 2.1-2.9 s under load), none elsewhere | 4 |
| `mutation-smoke` failures (signal, panic, non-atomic or invalid output, timeout) | 0 / 3,000 (slowest 10 ms) | 0 |
| Save fails or does not validate | 0 | 0 |
| Lossy save: less text than MuPDF reads in the source | 4 (p22: 3, p25: 1) | 1 (p25: a page listed twice) |
| Lossy save: fewer annotations | 3 (p22, with the pages above) | 0 |
| Lossy save: fewer pages than MuPDF counts | p22 27, p25 22, q3 40 | p22 27, p25 22, q3 40 |
| Refusal where MuPDF opens (p7) | 180 / 2,024 MuPDF opens | 201 / 2,054 |
| … of which MuPDF shows text | 1 | 1 |
| Less text than MuPDF in the view | p22 13, p25 20 | p22 9, p25 20 |
| Fewer annotations listed in the view | p22 13 | p22 10 |
| Probe rows where Rust shows less than MuPDF | 0 of 92 | 0 |

The q3 and p25 fewer-pages counts, and p7's page-count and password differences (88 and 33), are the
recorded differences below; none of those inputs loses text (all 121 p7 ones checked).

### Fixed here (each with a test in `engine/tests/m10_fuzz.rs`)

- **A header read through as a damaged value replaced an object** (repair, lossy save; p22 seeds
  304-306). Minimal shape: with the xref broken, `3 0 obj` inside a later dictionary where a value
  belongs: `<< /Type 3 0 obj /Font /Subtype /Type1 ... >>`. MuPDF's dictionary parse reads `3 0 obj` as
  a damaged value (null) and goes on (`/Font /Subtype` is a pair), failing only at `endobj`; the scan
  took the header as a later definition of object 3 (the name /Font), which replaced page 1, and the
  save wrote the loss (page, text and its annotation). With two such headers the file was refused.
- **A whole object read through inside a later object replaced its namesake** (repair; minimized from
  p7 seed 37 i=439): a copy of a whole object (dictionary, then `stream`) inside a later object's
  array, its header mid-line, replaced the earlier object of that number, even with an intact xref.
- The rule for both (`repair.rs`, the second `Err(Some(failed))` arm of `object`, after review round
  1): when the reread of a dictionary cut short reads through exactly one object header
  (`only_header_before`) and fails later, that header is skipped only when it would redefine an object
  the scan already met (`defined`: read or unparsable), and, if its object is whole, only when the
  failure lies past its value (`/A 4 0 obj <<` failing at `<<` is still round 3's glued next object).
  The scan then goes on at the failing token when that token starts a header that is new or whole (the
  next object), and after it otherwise. The only definition of a number is never skipped.
- **Round 1's rule was wrong** (review, major): it skipped every read-through header and resumed after
  the failing token. A dictionary cut short (`20 0 obj << /Title (x) /CS `) glued to
  `21 0 obj /DeviceRGB endobj` failed on the number of the next header (`13 0 obj`, a page at a line
  start, or `6 0 obj`, its content stream), which the scan then skipped: a page or its text lost in
  the view and the save. A cut-short array (`/W [1 2 `) glued mid-line to the only definition of a
  content stream whose data holds a stray `]` lost that stream. 034eead keeps both, and so does the
  rule above; MuPDF keeps the first and loses the stream in the second.
- Tests: `a_header_read_as_a_damaged_value_does_not_replace_an_object` (font /Type, form /Subtype,
  page /Rotate, two headers, the array copy; intact and broken xref, view and reopened save equal
  MuPDF's reading) and `the_rule_keeps_the_next_object_and_sole_definitions` (the review's shapes,
  the one-digit variant, two array openers; equal to 034eead). The second fails on round 1's rule.
- **An object-stream member lost to a read-through header** (review round 2, major, also in 034eead):
  pages 3 and 4 in an object stream, then a font `<< /Type 3 0 obj /Font ... >>`, xref broken. No
  top-level 3 came before the fake header, so the rule kept it, and as a later definition it beat the
  member: one page, "Alpha" lost in the view and the save. A header kept that way whose object is not
  whole is now weak (`Scan::weak`): in `winners` it loses to an object-stream member of its number in
  either order. A whole later redefinition (an update's real object, at a line start or glued
  mid-line) is not weak and still wins over an older member by position. Test:
  `an_object_stream_member_outranks_a_read_through_header_not_an_update` (fake before and after the
  stream; the update both ways); the fake-after case fails before the change.
- Round 2 against 034eead: p22 seeds 301, 304, 306 no category worse (304 and 306 better in six), p25
  seeds 7 and 12 identical, q3 seeds 1, 4 and 9 identical; probes x1-x7 no row worse, x6 4 and x7 2
  rows better (the object-stream case and a fake `3 0 obj` in an update's font).
- Head against 034eead on the same inputs (the review's base package, `review-p10/pkg-base`): p22
  seeds 301-306 no category worse in any seed, lossy saves 0 against 3, fewer-pages saves 24
  against 27; p25 seeds 7-14 identical; q3 seeds 1-9 identical (2,700 inputs, 59 fewer-pages saves
  in both, all pre-existing); p7 seeds 21-23 and 31-38 (5,500 inputs; each build run on both builds'
  907 dumps): the head refuses none that 034eead opens and opens one it refused (a blank page, as
  MuPDF); the review's probes x1-x6: no row where the head shows or saves less than 034eead, x6 4
  rows better.
- **A /Contents entry that is no stream blanked the page in the view** (p22 seed 306 i=38:
  `/Contents [ /Xk2 0.000001 8 0 R ]`). hayro's typed array iterator stops at the first entry that is
  not a stream; MuPDF skips such entries. The view's private update now writes the page without them
  (`app/view_contents.rs`, with the image-filtered streams it already left out). The save kept the
  text already.

### Found, not fixed (cause and proposed fix)

- **A copied object stream header at a line start** (p7 seed 37 i=439, before): a fuzz copy of the
  object stream's header, dictionary and first data bytes landed inside the xref stream's `/W` array,
  its header at a line start. The rule for a dictionary that lost its `>>` (round 3: a line-start
  header whose own object parses whole is the next object) reads the copy as object 10, which replaces
  the real object stream: "PDF has no pages"; MuPDF reads the array through and shows three pages.
  Proposed: when the reread of the outer dictionary succeeds past that header only by reading it as
  array or dictionary values and ends at `endobj` or `stream`, and the header's number already has an
  intact definition earlier, keep the earlier one. Not done: it touches the round 3 rule the open
  lane tuned; one input in 4,000.
- **An object header split by junk** (p7 seed 31 i=424, after): a fuzz insertion between `10` and
  `0 obj` of the object stream's header (with a copied `%%EOF` in it). MuPDF's repair pairs the last
  two integers it lexed before `obj`; the scan needs `N G obj` with only whitespace between. Refused
  ("PDF has no pages"); MuPDF shows three pages. Proposed: none (a token-level header reconstruction
  for one input in 4,000 is not worth the risk of inventing objects).
- **A damaged form /BBox hides the form in the view** (p22, all 9 remaining view text losses):
  `/BBox [ 0 NaN 500 500 ]`, `[ falsey 0 500 500 ]`, `/BBox /Xk8 /` and the like. hayro reads /BBox as
  four numbers or skips the form (`x_object/form.rs`); MuPDF reads a damaged entry as 0 and still
  extracts the form's text (also for `/BBox /Xk8`, a name). The save keeps the form. Proposed: a view
  fix (`app/view_fixes`, beside `tiling`) that writes such a form's /BBox with each non-number as 0,
  and a non-array as an empty box (to be measured against MuPDF), through the copy budget. Not in `walk.rs` or the vendored crates.
- **p25 view text losses (20)**: p25 records no example for view-only differences and keeps no input,
  so they are not classified; its saves keep the text (the one save loss is the duplicate page below).

### Recorded differences (not defects; unchanged)

- A /Kids entry damaged into something that names no page (`5 0 Z`, `5 0+R`, `[3 0 R 8 0 R ...]` with
  8 a content stream, `0 0 R`): MuPDF keeps /Count and counts a blank page; the app drops it
  (`app-corpus-open.md`). All 22 p25 fewer-pages saves are this. Of the 23 p22 ones dumped, 19 are this
  and 4 a page whose dictionary does not parse and swallows the next page's (MuPDF: a blank page, the
  app: none); q3's 40 are all a header glued after a damaged tail with a broken xref, where MuPDF
  counts a blank page ("pre-existing-glued", as before). None loses text MuPDF shows.
- A page listed twice in /Kids (`[4 0 R 4 0 R 5 0 R]`): shown and saved once (`app-corpus-open.md`);
  the one p25 "save loses text" input after the fixes (seed 12 i=155) is this: the duplicate's text.
- Refusals where MuPDF opens, 200 of the 201 after: MuPDF shows blank pages only (no text) or opens
  locked with no pages; the app refuses with "PDF has no pages" (68: every kid named no page), "invalid
  encryption without the file identifier" (94, a destroyed `trailer` keyword), "invalid document
  catalog" (24) or a damaged encryption dictionary (14) (`app-accept-session.md`).
- An annotation whose /Subtype is damaged (`/Subtype -5-`, `3 0 obj`): MuPDF lists it, the app's
  annotation list leaves it out (nothing to draw or edit); the save keeps it.
- A glued whole object that redefines an earlier, intact object of the same number with a broken xref
  (`/A 3 0 obj << /Type /Page ... >> endobj` inside a later dictionary): round 3 reads it as the next
  object and it replaces page 1; MuPDF does not see it. Seen only in the hand minimization, not in the
  campaign.

## Not covered

- Encrypted inputs beyond p7's fixtures and its three MuPDF-written seeds; no fuzzing of the password
  or permission paths beyond `authenticate`/`security-set` with fixed passwords.
- Editing operations (annotations, forms, page edits, redaction) on damaged inputs: only open, view,
  text, annots, page-ids, save and validate are exercised. `app/redact/*`, `view/walk.rs` and the
  vendored crates were not changed (other lanes).
- Large inputs: every input is under 16 KB except `p6` meshes (up to 200 KB); memory and time bounds on
  large files are the corpus runs' and `rust-engine-bounds` tests'.
- Real-world damage: the campaign mutates synthetic files; the external corpus is not used here.
- `p25`'s view-only differences (no examples kept) and `p7`'s byte reproducibility (random encryption).
