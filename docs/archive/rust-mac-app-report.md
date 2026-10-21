# Tandem PDF for Mac, all in Rust: report (October 19, 2026)

Branch `p4-rust-mac` (worktree `tpdf-wt/rust-mac`), 99 commits on top of local `main` f70b41f. Nothing is merged to
`main` or pushed. This is Phase 4 of `docs/rust-and-ux-plan.md` for macOS: the app's logic, its window and its shell
are Rust; the engine runs natively instead of as WASM.

## In short

- **It works and passes everything.** Every gate passes on head de85001 (36 of 36), including the full app test suite
  run against the release Mac binary: 793 pass, 0 fail, 53 skipped. The 53 are Electron-only tests (Chromium, CDP, the
  TypeScript build). The Electron app and the engine are unchanged and still pass their own suites.
- **It is small.** `Tandem PDF.app` is 38 MB and the disk image 15 MB, against about 250 MB for the Electron app.
- **The UX pass is done for every feature area.** Seven first-time-user walkthroughs found about 90 problems; two fix
  rounds, a code review and a re-walk later, the blocking ones are fixed and the rest are fixed or listed below.
- **Four things wait for you** (end of this report): two licences Tauri brings in, a Developer ID, the GPT reviewer's
  login, and when to merge.

## What was built

| Crate | What it is | Rust lines |
|---|---|---|
| `crates/tandem-protocol` | The drive protocol, `STATE` and the window's view model as serde types | 2,600 |
| `crates/tandem-app` | The app's core: a function-by-function port of the Electron renderer and main process (documents, tools, undo, save journal, autosave, printing, recent files, trust list, drive/MCP harness) plus the window model (`ui.rs`) | 57,900 |
| `crates/tandem-ui` | The window, in Leptos, built to WASM (266 KB) and shown in the system web view | 4,100 |
| `crates/tandem-desktop` | The Tauri 2 shell: menus, native panels and alerts, PDFKit printing, Open Recent, drag and drop, window state, Finder open | 1,900 |
| `crates/tandem-sign` | Signing and verification (Phase 2), used by path | 4,800 |

How it fits together: one core thread owns the document; the window, the menus and the page-pixel protocol send it jobs,
so the window never waits on the engine. The headless binary (`tandem-headless`, or `tandem-pdf --smoke/--drive`)
runs the same core without a window, so the test suites, `scripts/drive.mjs` and the MCP agent run against the Rust app
unchanged (`TANDEM_APP_BIN`). The only `unsafe` code is `crates/tandem-desktop/src/macos.rs` (the native calls).

## Proof

Gates (`tpdf-wt/tools/gates.sh`, logs in `tpdf-wt/gates-out/rust-mac-final/`; 26 minutes cold, 7 warm):

| Gate | Result |
|---|---|
| Engine tests, stable and Rust 1.92 | 1,474 pass, 0 fail (each) |
| Engine fmt, clippy (stable, 1.92), bridge | pass |
| Every crate: fmt, clippy -D warnings (stable, 1.92), tests | pass (tandem-app: 240 unit tests) |
| UI built to WASM, clippy on wasm32 | pass |
| Electron app suite | 675 pass, 0 fail, 171 skipped |
| Rust-engine app suite | 693 pass, 0 fail, 153 skipped |
| **Mac app suite (release binary)** | **793 pass, 0 fail, 53 skipped** |

The packaged app (`crates/target/release/bundle/macos/Tandem PDF.app`) passes a smoke run (open nav.pdf, print to PDF),
verifies with `codesign` (ad-hoc, hardened runtime, no entitlements) and carries every shipped crate's licence text.

## The UX pass

Seven reviewers walked the real window as first-time users, at 1280×820 and 1920×1080, light and dark: opening and
saving, viewing, annotating, editing and forms, redaction, pages and output, signing. Then:

1. **Fix round 1** (three lanes on disjoint files) and **"Make an editable copy"** for signed files (new engine op
   `unsigned-copy`: a copy without signatures, the original untouched).
2. **Code review** of the fixes found six real bugs, all fixed with tests: a document made from pictures could be closed
   without asking to save; multi-page Undo could take back the wrong step; the first drawn signature was not ready to
   place; "From the top" was wrong on rotated pages; the Security dialog called a restricted file password-protected;
   a failed open reset the current document's Undo grouping. A seventh risk (an encrypted signed file's copy dropping
   redaction boxes) was confirmed and fixed.
3. **Re-walk** of all seven areas against the rebuilt app: most findings confirmed fixed in the window; the rest went
   into **fix round 2** (three lanes) and a last leftovers pass.
4. **Re-check walk** of the round-2 fixes: see below.

Highlights of what changed for people: a failed open keeps your document; plain messages with a next step instead of
engine text; Save / Don't Save / Cancel; Untitled documents can be saved; one Undo per multi-page action, with its name;
Compare shows the right pages; the page number follows the view; the tool resets for a new document; annotations,
signatures and redaction boxes can be selected, moved and deleted by mouse and keyboard; the text-box tool works;
signing works end to end (pad, placement, certificate box by dragging, verify with trusted list and per-signature
status); redaction boxes say what they cover; drag and drop shows what will happen; ⇧⌘G is Find Previous; the screen
reader hears what happened ("Added note"); status lines are coloured by kind; a "Show in Finder" on save; the layout
fills the window at 1920.

Screenshots: `docs/ux-audit/rust-mac/<area>/` (first walk) and `docs/ux-audit/rust-mac/after/recheck/` (the final
re-check, 1280×820 light). The re-walk and fix-round sets (about 1,000 shots, 195 MB, at 1280 and 1920 in light and
dark) are kept outside the repository in `tpdf-wt/ux-shots/rust-mac-after/`, so they don't bloat git. Findings:
`tpdf-wt/scratch/rust-mac/ux-findings/` and `rewalk-findings/`.

### Re-check walk (round 2)

Ten checks against the release binary (screenshots `docs/ux-audit/rust-mac/after/recheck/`):

| Check | Result |
|---|---|
| Page number follows the view through zoom, fit, sidebar and panel changes (large.pdf, page 251) | works |
| Fit width with mixed portrait and landscape pages: one scale, nothing overflows | works |
| Another document, or a signed one, starts with Select | works |
| Toolbar Save on Untitled | works |
| Annotations: select, Delete, Undo; text box tool places a box with its editor | partly: shapes cannot be moved yet |
| A page the engine cannot edit: no leftover targets, a plain notice, text boxes still work | works |
| Fields: list boxes offer their options, sensible text size, no handles in Fill & Sign | works (read-only fields: see below) |
| Signing: pad shows the saved signature and chosen picture, resize follows the pointer, certificate box placement says where it landed, no stray outline | works |
| Redaction: rows in whole words, page menu, "Added 8 boxes", over-redaction warning | works |
| Start screen Recent × and Clear, steady layout, red errors, Security reads the file, repaired file wording | works |

It found two more problems, fixed afterwards: a saved signature was not loaded when the app started, and one Security
refusal showed engine text. The same pass also sized the page box for three-digit pages, removed handles from read-only
fields (a click says "This field is read-only."), put the page's top at the top on Fit page, kept the panel tabs on one
row at 1920, and recorded that Fit width means "fit to a comfortable reading width" (capped at 1,180 px).

## Not done (recorded gaps)

- **Export** covers the current page or all text; all pages, a range or a flattened PDF need core work.
- **Stamps and batch** have no Centre position (the engine places left or right) and no preview.
- **Wrong password**: no way back to the document that was open before.
- **A second launch** from the command line does not hand its file to the running app; that needs Tauri's
  single-instance plugin, a new dependency.
- **Objects** (images, text blocks) are selected with the Text tool, not with Select. **Shapes, highlights and stamps**
  can be selected and deleted but not moved; the engine moves only text boxes and signatures.
- **Kept because tests pin them:** the batch output folder is still named `pdf4all-out`; a pasted object lands 10 pt
  from the original.
- Smaller items: JPEG exports record 72 dpi; text export separates pages with form feeds; the labels dialog has no
  preview; a Security change cannot be undone (Help no longer promises it can).

## Measurements

| What | Result |
|---|---|
| `Tandem PDF.app` / `.dmg` | 38 MB / 15 MB (Electron: about 250 MB) |
| Release build and bundle (warm) | 2 min |
| Release build and bundle (cold) | 8 min, peak 1.8 GB |
| UI to WASM (release) | 266 KB wasm, 32 KB glue, 25 KB CSS |
| Open nav.pdf (3 pages), native engine | 3 ms |
| Smoke run, open large.pdf (500 pages) and print 20 | 75 ms |

All builds ran under `tpdf-wt/tools/slot` (two at a time, 3 GB cap) on the 8 GB MacBook Air.

## Questions for you

1. **Licences.** Tauri brings in `option-ext` (MPL-2.0, used unmodified) and the ICU4X crates (Unicode-3.0, permissive).
   Everything else linked in is MIT, Apache, BSD, ISC or Zlib. MPL-2.0 is file-level copyleft and fine to ship unmodified
   with its notice (it is in the bundle); OK to accept both, or should they be engineered out?
2. **Developer ID.** The app is ad-hoc signed, so it runs on this Mac only. `scripts/package-mac.sh` signs and notarizes
   when the Apple variables are set; it needs a Developer ID Application certificate and an app-specific password.
3. **GPT review.** The independent GPT-6 Astra reviewer could not run: `pi` has no openai-codex login on this Mac.
   Running `pi`, then `/login`, enables it; a Claude reviewer did the code review instead.
4. **Merging.** `p4-rust-mac` is ready to merge into local `main` (nothing is pushed). Merge now, or keep it on the
   branch until you have tried the app?

## Try it

```bash
open "$REPO/crates/target/release/bundle/macos/Tandem PDF.app"
```

The disk image is `crates/target/release/bundle/dmg/Tandem PDF_0.1.0_aarch64.dmg`. The Rust app keeps its recent files,
trust list and drawn signature in `~/Library/Application Support/Tandem PDF`, apart from the Electron app's.

Decisions made along the way are in `docs/rust-and-ux-plan.md`, "Decisions made building the Mac app".
