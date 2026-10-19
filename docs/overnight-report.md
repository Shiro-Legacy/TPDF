# Overnight report (Oct 18–19, 2026)

Phases 0, 1 and 2 are done.

- Each phase is on its own local branch, and every branch passed the full gates.
- Each phase was reviewed by Claude agents that did not write the code, over several rounds, until a round found no blocking issues.
- **Nothing is merged to `main` and nothing is pushed.**
- **To try everything at once, use `overnight-integration`**: all the branches merged, with the integration fixes.

Every decision I made is listed at the end of `docs/rust-and-ux-plan.md`, under "Decisions made overnight". The three questions you answered during the night (PAdES B-B, fetching the `cms` licence files, Claude reviewers) are done.

## What's done

| Branch | Head | What | Independent review | Gates |
|---|---|---|---|---|
| `overnight-integration` | 8b6352e | Everything below merged, plus fixes found while integrating | 1 round, one blocking issue, fixed | All PASS. Engine 1469/1469 (stable and 1.92); app-suite 675/0; app-rust 693/0; full Electron suite 817 pass, 0 fail (29 skipped, as before); mobile node 21/21 and 4/4 browser suites |
| `p0-redact-engine` | 9aa4c3e | R1–R6, `redact-inspect` (the "what's underneath" preview), D5 signed files into an unsigned copy, R4 flatten with consent | 6 rounds; the last: no blocking issues, no regressions | All PASS, engine 1469 |
| `p0-redact-ui` | 15dc0e5 | §2.3 desktop flow: Redact as a tool, solid black boxes, red tint underneath, side panel, plain dialog, the redacted copy opens with a banner, every way out (Save, Save As, Export, Print, Sign, close, open) asks about pending boxes | 3 rounds; the last: no blocking issues | All PASS at that head. The final engine and small UI follow-ups are on `overnight-integration`. |
| `p0-redact-mobile` | e44878c | Box redaction in the mobile app (one finger draws, two fingers scroll), same wording as desktop | 2 rounds; the last: no blocking issues | All PASS, engine 1469; 4/4 mobile browser suites |
| `p1-ux-quick-wins` | fa2e1b8 | Tool rows wrap; start screen with recent files; Esc always stops a tool, plus a chip; message catalogue, success never red; highlight from a text selection; a note opens its editor at once; Save As switches to the new file, plus Save a Copy; filled form asks "save over or as new?"; one Sign chooser with "Sign and save as…"; X3, X5, X12 | 3 rounds; the last: no blocking issues | All PASS |
| `p2-tandem-sign` | 6e910ef | `crates/tandem-sign` (RustCrypto, pure-Rust `rsa` per D3) replaces `src/sign/*` crypto; WASM in a worker thread; PAdES B-B; signing date read from the signed bytes only; plain verification sentences; ESS cert-v2; P-256/P-384 signing | 4 rounds; the last: no blocking issues | All PASS, including the new `tandem-sign` gates |

## What was hard: redacting shared content

R1, R2, R5 and R6 were straightforward. R3 (shared pictures and forms) and R4 (flattening) were not. Five review rounds each found valid PDFs where the secret survived somewhere in the file while every check passed: shared or inherited resources, unused patterns, soft masks, invisible second placements, and appearance streams.

The design that held is simple. **Everything a box touches is redacted in place, wherever else it is used. Nothing is kept as an untouched copy.** When that also removes content outside the boxes, the engine reports it, and the banner says: "Some content outside your boxes was also removed, because it is shared with other parts of the document. Check the copy."

There are also two final checks that fail closed: one reads the text of every content stream in the written copy, the other checks every picture under a box. Anything they can't verify makes the redaction stop with nothing saved, and the message says why and what to try.

## In progress

Nothing. All lanes finished.

## Questions for you

1. ~~Is over-redaction OK as shipped?~~ **Answered (Oct 19, morning): yes, fine as it is.** Boxing part of a shared object (logo, letterhead) removes that part everywhere it is shown, and the banner says so.
2. **A few rare things make redaction refuse instead of clearing them.** These are remote go-to destinations, `/Lang` on structure elements, layer usage text, and alternate text that spans boxed and unboxed words (unless "Descriptions written for screen readers" is on). They are safe but blocking. Do you want options for them?
3. **Mobile two-finger scrolling** in Redact tracks 1:1 in the test browser. It needs a check on a real phone.

None of these blocks anything.

## Known gaps, written down rather than fixed

- **Picture duplicates are not checked.** An identical picture stored as a separate object and drawn nowhere under a box keeps its pixels. The text check does catch text duplicates.
- **Mobile has no keyboard path for drawing boxes**, no "Find text to redact", no "Redact whole page", and boxes can't be moved or resized. Desktop has all of these.
- **Ten engine tests assert wall-clock limits** and can fail when two gate runs share the machine. Two were made load-robust tonight. A shared load-tolerant timing helper is a good small follow-up.
- **The Verify date can show different days near midnight.** The table shows local time, while the hover sentence uses the UTC day.
- **Desktop modes (X2) are still there.** Phase 1 kept them on purpose; the tool strip is the Phase 4 UI.

## How to try the redaction prototype

```bash
cd /Users/owner/Documents/tpdf-wt/p1-ux && npx electron .
```

That worktree holds `overnight-integration`, and its build is current. Add `--sample` to start on the old sample file.

1. On the start screen, click **Open a PDF…** and choose `fixtures/redact.pdf`.
2. Click the **Redact** tab. The pointer becomes a crosshair, a chip says "Drag over anything you want removed. Esc to stop.", and **Redactions (0)** appears on the right.
3. Drag over "visible P0B-SECRET-7731 here". While the mouse is down the box is see-through, with what will go tinted red. When you let go, it is solid black with a thin red outline.
4. Optional:
   - Tick **Show what's underneath**.
   - Right-click the page for **Redact whole page**.
   - Try **Find text to redact…**.
   - Press ⌘S, Export… or Print…: each asks "1 area is marked but not redacted yet."
5. Click **Redact 1 area…** (open **Details** to see the nine options in plain words), then **Redact and save as…** and pick a new name. Choosing the original's own name is refused.
6. The redacted copy opens with a blue banner: "You are viewing the redacted copy … 1 area removed and checked." **Open original** goes back.
7. Signed file: open `fixtures/p0b/signed-intact.pdf`, draw a box and redact. You are asked "Redacting removes the signatures. Make an unsigned redacted copy?"

Mobile:

```bash
cd /Users/owner/Documents/tpdf-wt/p1-ux/mobile && npm run dev
```

Open http://127.0.0.1:4173 at 375×812, then **Open a PDF** → `fixtures/redact.pdf` → **Redact**. Drag with one finger (two fingers scroll), then tap **Redact 1 area…** → **Redact and save**.

## Screenshots

`docs/ux-audit/after/` on `overnight-integration` holds:

- `int-01`…`int-07`: the integrated app (start screen, tool rows, redact box and panel, dialog, banner, sign chooser, Verify);
- `p0-01`…`p0-17`: desktop redaction, including the signed and flatten questions;
- `p0m-*`: mobile redaction;
- `p1-*`: the Phase 1 screens.

## Housekeeping

- **Worktree:** only `tpdf-wt/p1-ux` is left (it holds `overnight-integration`). The other worktrees were removed; their branches remain.
- **Disk:** 48 GB free.
- **Gates script:** `tpdf-wt/tools/gates.sh` now also gates every `crates/*/Cargo.toml`. The old script is kept as `gates.sh.bak-2026-10-18`.
- **Build cache:** I cleared main's `engine/target/debug` cache to save disk; the next build there rebuilds it.
- **Uncommitted in the main checkout:** `docs/rust-and-ux-plan.md` (status line and "Decisions made overnight") and this report.
- **Logs:** review probe crates and logs are in `tpdf-wt/scratch/review-*`; gate logs are in `tpdf-wt/gates-out/<branch>`.
- **Merging:** `overnight-integration` already contains every branch, so merging it into `main` brings everything in one step.
