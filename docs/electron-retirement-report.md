# Retiring Electron: report

Branch `p7-retire-electron` (from `fc728ba`, the reviewed Rust Mac app `p4-rust-mac`), October 19, 2026. Not merged
and not pushed: the owner decides.

## In one paragraph

The all-Rust app (Tauri 2 + Leptos, `crates/`) is now the desktop app on macOS, Windows and Linux, and Electron is
gone: `src/`, the TypeScript app build, the Electron packages, its packaging and install scripts, and its tests. The
Rust app gained everything Electron still did for it: an MCP server in the binary (`--mcp`), a second launch handing
its files to the running window, a one-time import of the Electron development builds' settings, and a Windows and
Linux shell (native questions, real printing, an in-window menu bar with Ctrl shortcuts and Open Recent, the file name
and "(edited)" in the title, screenshots), with installers (NSIS; deb and AppImage) and a release workflow for all three
systems. Mobile is unchanged and still builds and passes; the little TypeScript it shares with the test tooling lives
in `web/`. **Windows and Linux are built and checked from this Mac but have not run on real machines yet**: the first
CI run and `docs/windows-linux-checklist.md` do that.

## What changed, step by step (the brief's order)

1. **MCP server in Rust** (`3af9a8c`, `f25b1b7`). `tandem-pdf --mcp` (and `tandem-headless --mcp`) is a stdio
   JSON-RPC server with the same six tools and schemas as the Node server it replaces (`crates/tandem-app/src/mcp.rs`).
   The session runs on its own thread with a 64 MB stack, so `ping` is answered while a long step runs. The Node tests
   for it pass against the Rust binary. `docs/agent.md` gives the install line per OS; the bundle no longer ships
   `agent/*.mjs`, and nothing needs Node at run time.
2. **Single instance** (`09af930`). A second launch sends its files to the running window over a loopback port with a
   secret token kept in `<user data>/SingletonHandover` (0600), then quits; the window comes to the front and opens
   them. Not the Tauri plugin: it needs Tauri 2.12 (we pin 2.11.5) and allows one app per machine, which would join
   test windows that use different data folders. `--smoke`/`--drive` on a held folder still exit 3.
3. **Electron's data, once** (`52144b9`). Packaged Electron already used the same `Tandem PDF` folder, so nothing
   moves there. The Electron development builds' folder (`pdf4all-p0`) is copied once: `recent-files.json`,
   `trusted-certs.pem`, `drawn-signature.json`; nothing is overwritten, Electron's files are not touched, and a note
   appears only when something was brought over. A marker (`electron-import.json`) stops it running again.
4. **Windows and Linux shell** (`5640bc2`, `8d18fbc`, `d3b63d0`, `919b6e0`, `589acb6`).
   - Questions: Task Dialogs on Windows, GTK message dialogs on Linux, with the core's buttons, default and Esc
     button (the dialog plugin cannot choose the default button). On Linux the buttons are in GNOME order.
   - Printing: the system Print dialog, then pages drawn by the engine (up to 300 dpi; the engine's largest scale is
     288 dpi) and laid out as the Mac's print panel does (real size, shrunk only to fit, turned to the paper). Windows
     sends them through GDI in bands; Linux through GtkPrintOperation and Cairo, rendered on a worker thread with a
     large stack.
   - Menus: an in-window menu bar. Each Mac shortcut has a decided Windows/Linux one (`tandem_protocol::keys::PC`,
     unique per OS, never Ctrl+Alt, which is AltGr on many European keyboards); the Electron app's 47 items are still
     counted by a test. File has Open Recent (from the app's own list), Close Window and Exit/Quit; Help is F1. On
     Windows, menu shortcuts typed while the page has the keyboard come through WebView2's AcceleratorKeyPressed (the
     window's accelerator table never sees them), and the browser's own keys (F5 reload, Ctrl+P page print, the find
     bar) are off.
   - Title: "report.pdf — Tandem PDF", "report.pdf (edited) — Tandem PDF".
   - The UI says the system's keys and words: Ctrl in hints and Help, Ctrl as the command key for clicks and keys,
     "Show in Explorer" / "Show in Folder". The Mac text is unchanged, byte for byte.
   - Clipboard, links, Show in Explorer/Folder, `--shot` and `--walk` screenshots per OS.
   - The core runs on Windows paths, permissions and file identity (`fsx.rs`), finds installed resources on all
     three systems, and no longer warns after every Windows save.
   - Packaging (`fca975c`): NSIS per-user installer to `%LOCALAPPDATA%\Tandem PDF`, deb and AppImage, PDF association
     ("Open with", the default app unchanged), icons, licence texts per target; `scripts/package-windows.ps1`,
     `scripts/package-linux.sh`; `docs/windows-linux-checklist.md`.
5. **CI** (`fca975c`, `cee9010`). `.github/workflows/release.yml` runs on macOS, Windows and Linux: the UI to WASM,
   fmt, clippy on 1.98.1 and 1.92, crate tests, packaging, the app suite with `TANDEM_APP_BIN` set to that runner's own
   release binary (under xvfb on Linux), a test of the installed app, and a draft release with `SHA256SUMS`. It also
   runs on `workflow_dispatch`. Private signing-test keys never go to GitHub, so CI leaves out the 8 suite files and 2
   crate tests that need them, and says so.
6. **The Electron-only tests** (`926609a`). The checks that still matter moved to Rust (`crates/tandem-app/tests/`:
   safe_write, sign, sign_store, recovery, words, helpers, gui — 162 tests through the public API) or to window
   scripts (`tests/walk/`). Electron, Chromium, CDP and packager internals were deleted. The MuPDF oracle stays as
   dev-only tooling: the engine tests reopen the app's output in it.
7. **Mobile without `src/`** (`4ea9be1`, `cee9010`). Mobile imports the protocol types and the engine worker from
   `web/` (one home, shared with the engine-adapter tests and the oracle) instead of the Electron renderer. It keeps
   Electron 44.4.0 as a devDependency only for its browser-test harness (`npm run test:ui`); nothing shipped contains it.
8. **Electron deleted** (`cee9010`, `cb7163f`): `src/`, `tsconfig.json`, `tsconfig.preload.json`, the Electron and
   `@electron/packager` dependencies, `scripts/{build,package,check-package,check-macos-package,make-icon}.mjs`,
   `make-dmg.sh`, `make-linux.sh`, `install-linux.sh`, `install-windows.ps1`, `installer.iss`, `engine/desktop`,
   `op-log-electron.cjs`. `package.json` scripts point at the Rust app; `npm run build:test` builds what the
   engine-adapter tests load into `build-test/`. `CI_NO_ELECTRON` is now `TANDEM_NO_APP`. README, `docs/agent.md`, the
   app's `NOTICE` (new: it names every file in `licenses/`, and the CJK font's notice now ships beside it), the
   third-party records and the plan's Phase 7 status are updated.

## Bugs found and fixed on the way

Porting the old tests to Rust found six bugs in the Rust app (fixed in `2b4ece9`, reviewed, follow-ups `b132594`):

1. **Edits lost after an engine restart.** Fill text, signatures, text edits, annotation and field changes,
   redaction marks and a few others went straight to the engine, past the recovery log. After an engine failure the
   app said the changes were restored, but those were gone. All edits now go through the log. Because the log now
   holds typed text and signatures, its files are private (0600 in 0700 folders).
2. A page that had stopped the engine was drawn again at once after the restore.
3. After a failed restore reopened an Untitled document, its next save left the window showing "Saving…".
4. Escape during a text drag did not cancel the drag.
5. Page messages ("Added header to 1 page", "Exported text…") never reached the window.
6. A drawn signature file that was readable by others stayed so after a save.

Two independent reviews of the Windows/Linux shell and the fixes (Claude reviewers; the GPT reviewer has no login)
found, among others, a possible freeze when a menu shortcut met a menu update, Windows page shortcuts that never
matched, a false "may not survive a power loss" warning after every Windows save, and blurry Linux prints; all fixed
in `589acb6`.

## What is proven, and where

On this Mac (gates, see below):
- Every crate is clean under `cargo fmt`, and under clippy `-D warnings` on stable and 1.92, for macOS. The Windows
  (`x86_64-pc-windows-msvc`) and Linux (`x86_64-unknown-linux-gnu`) targets are also clippy-clean on both toolchains
  (a stub resource compiler and pkg-config; nothing is linked).
- Crate tests, the app suite against the Rust release binary, the engine-adapter suite, and mobile's tests
  (21/21; its four browser suites passed when run by hand).
- The Mac window still works (open, menus, screenshots), and the Mac package (`scripts/package-mac.sh`) built and
  passed its self-test on this branch, before the last fix rounds.
- The pure parts of the Windows and Linux code have unit tests here: shortcut table and key mapping, titles, page
  layout and rotation, resource folders, error numbers.

Not proven: anything that needs a real Windows or Linux desktop — the dialogs, printing, the menu bar and its keys,
installers, file association, the second-launch hand-over there, high-DPI and AltGr typing. That is
`docs/windows-linux-checklist.md`, plus the first CI run (which needs a push).

Gates (`tpdf-wt/tools/gates.sh`, changed with Electron going: the Electron build and Electron suite gates are gone; a
test-build gate, the app suite against the Rust release binary, a mobile gate and Windows/Linux cross-clippy gates are
new): **38 of 38 pass** on the final tree. The Windows cross-clippy gate failed on the first run (imports used only by
Unix-only tests) and passed after that fix. Counts:

| Gate | Result |
|---|---|
| Engine tests (stable and 1.92) | 1,474 pass, 0 fail |
| tandem-app / tandem-protocol / tandem-sign / tandem-desktop tests | 432 / 19 / 42 / 6 pass, 0 fail |
| App suite against `crates/target/release/tandem-pdf` | 606 tests: 588 pass, 0 fail, 18 skipped (the WASM tests the next row runs) |
| Engine-adapter suite (`tests/run-rust.mjs`) | 490 pass, 0 fail (116 app-launching tests skipped there by design) |
| Mobile (`npm test`) | 21 of 21 |

Before this branch the Mac app suite was 793 pass, 0 fail, 53 skipped. The 53 were Electron-only tests, now ported to
Rust or deleted (`926609a`). The suite is smaller because the ported checks run as Rust tests.

## Questions for the owner

1. **Merge and push.** The branch is local. `main` is unpushed and holds the MIT relicense, whose lawyer review is
   still open.
2. **Licences.** Accept Tauri's `option-ext` (MPL-2.0) and the ICU4X crates (Unicode-3.0)? Both are unmodified and
   listed in `THIRD-PARTY.txt`; `scripts/crate-licenses.py` accepts them by name.
3. **The Linux AppImage** carries LGPL system libraries (GTK, WebKitGTK and others) with their licence texts. Does the
   licence review want a written source offer too? Ship the AppImage now, or only the .deb?
4. **WebView2 on Windows.** Keep the small embedded bootstrapper (needs internet only where WebView2 is missing), or
   ship the offline runtime (+127 MB per download)?
5. **Signing.** A Developer ID for macOS (notarized installers) and a Windows code-signing certificate (SmartScreen
   warns until then).
6. **Reviews.** The `pi` openai-codex login, for GPT reviews alongside the Claude ones.
7. Smaller: the plain-words Help dropped some limits the Electron Help stated (key and password memory, verification
   limits); bring some back? The Tauri content-security policy allows inline styles and sets no `frame-ancestors`; fine
   for an app with no remote content, but say if you want it stricter.

## Follow-ups (not done here)

- Run the release workflow once (needs a push) and go through `docs/windows-linux-checklist.md` on real machines.
- Windows file identity uses the creation time with size and modification time; the real file index (the
  `same-file`/`winapi-util` crates are already in the lockfile) would remove a rare false "same file".
- The view checks every recent file on each state; a dead network drive in the list could slow the window on Windows.
- `crates/tandem-sign/pkg` (the signing WASM) is now read only by a packaging check; stop building it?
- Ctrl+Z in a text field with document Undo available runs document undo, as ⌘Z does on the Mac today.
- Phase 5 (mobile on Tauri) would retire the remaining TypeScript in `web/` and `mobile/`.
