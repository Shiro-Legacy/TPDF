# Windows and Linux: what to check by hand before a release

The Rust desktop app is built, linted, unit-tested and smoke-tested on Windows and Linux by
`.github/workflows/release.yml`, but nobody has used it there in a real window yet: CI has no display (Linux runs
under `xvfb`) and no person clicking. This list is what a person checks on a real Windows PC and a real Linux
desktop, with the installers from the workflow run, before the release draft is published.

Write down for each line: pass, or what you saw instead (a screenshot helps). Work on copies of your PDFs, never on
the only copy.

## 0. Get the build

1. On GitHub: Actions ▸ **release** ▸ the run for the commit or tag (or **Run workflow** on the branch to start one).
   Download the artifacts **installers-Windows** and **installers-Linux** (zip files) and unzip them.
   - Windows: `Tandem-PDF-<version>-windows-x64-setup.exe`
   - Linux: `tandem-pdf_<version>_amd64.deb` and `Tandem-PDF-<version>-linux-x86_64.AppImage`
   - (from a tag's draft release instead: the same files, and `SHA256SUMS`)
2. Check the files are the ones CI built: Windows PowerShell `Get-FileHash -Algorithm SHA256 <file>`, Linux
   `sha256sum -c SHA256SUMS --ignore-missing`. Expected: the hashes match `SHA256SUMS` (on a release) or the
   `sha256` line the packaging step printed in the run's log.
3. For section 9 (the app suite) also get the source of the same commit (`git clone` and `git checkout <commit>`).

Test systems: Windows 11 (and Windows 10 22H2 if at hand), x64. Linux: Ubuntu 24.04 desktop (GNOME, Wayland is the
default; also try "Ubuntu on Xorg" from the login screen's gear menu), and if possible 22.04 and a KDE desktop.
For the keyboard checks, add a German keyboard layout (Windows: Settings ▸ Time & language ▸ Language & region ▸
English ▸ Language options ▸ Add a keyboard ▸ German; Linux: Settings ▸ Keyboard ▸ Add input source ▸ German).

## 1. Install, start, uninstall

### Windows

| # | Do | Expected |
|---|----|----------|
| W1 | Double-click the setup .exe. | SmartScreen may say "Windows protected your PC" (the installer is not signed unless the release says so): More info ▸ Run anyway. No administrator (UAC) prompt appears. |
| W2 | Go through the installer. | A licence page shows the MIT licence text (LICENSE) in its lines and paragraphs (not run together into one line: the file has Unix line ends). The default folder is `C:\Users\<you>\AppData\Local\Tandem PDF`. It finishes without errors and offers to start the app. |
| W3 | Without internet (Wi-Fi off) on a PC that has WebView2 (every Windows 11; Windows 10 with updates): install again. | Installs; no WebView2 download is attempted. (On a PC without WebView2 and without internet the installer stops with a WebView2 message: record it, see "Questions".) |
| W4 | Explorer: open `%LOCALAPPDATA%\Tandem PDF`. | `tandem-pdf.exe`, `uninstall.exe`, folders `fixtures`, `fonts`, `licenses` (with `LICENSE-Tandem-PDF.txt`, `NOTICE-engine.txt`, `THIRD-PARTY.txt`, `crates\`, `standard\`). |
| W5 | Start menu ▸ type "Tandem". | "Tandem PDF" is there with the app icon; it starts the app. |
| W6 | Start the app. | One window, title "Tandem PDF", the start screen. No console window opens behind it. |
| W7 | Settings ▸ Apps ▸ Installed apps ▸ Tandem PDF. | Listed with the icon, version, publisher "Tandem PDF contributors". |
| W8 | Uninstall from there (app closed). Then look at `%LOCALAPPDATA%\Tandem PDF` and right-click a PDF ▸ Open with. | The folder is gone; Tandem PDF is no longer offered for PDFs; the default PDF app is what it was before installing. `%APPDATA%\Tandem PDF` (your data, F9) stays, also when "Delete the application data" is ticked (that box only removes `%APPDATA%` / `%LOCALAPPDATA%\com.shiro-legacy.tandem-pdf`, the web view's cache). |
| W9 | Install again over a running app. | The installer asks to close Tandem PDF first (or closes it), then installs. |

### Linux (.deb)

| # | Do | Expected |
|---|----|----------|
| L1 | `sudo apt install ./tandem-pdf_<version>_amd64.deb` | Installs; pulls in `libwebkit2gtk-4.1-0`, `libgtk-3-0` (or their `t64` names) if missing. `apt show tandem-pdf` shows Depends and Recommends `xdg-utils, libglib2.0-bin`. |
| L2 | `ls -l /usr/bin/tandem-pdf "/usr/lib/Tandem PDF"` | The binary, and `fixtures`, `fonts`, `licenses` (with `THIRD-PARTY.txt`). `/usr/share/doc/tandem-pdf/copyright` holds the MIT licence. |
| L3 | Activities ▸ type "Tandem". | "Tandem PDF" with its icon (not a generic gear). It starts the app. |
| L4 | While the app runs, look at the dock / taskbar. | Its icon is Tandem PDF's, and pinning it to the dock pins the app (on Wayland a generic icon here means the window's app id does not match the `.desktop` file: record it). |
| L5 | `tandem-pdf` from a terminal. | The window opens; nothing alarming on the terminal. |
| L6 | `sudo apt remove tandem-pdf` | Removed; the launcher is gone. `~/.config/Tandem PDF` stays. |

### Linux (AppImage)

| # | Do | Expected |
|---|----|----------|
| A1 | `chmod +x Tandem-PDF-*.AppImage && ./Tandem-PDF-*.AppImage` | The window opens. (On Ubuntu 22.04+ without `libfuse2` it says FUSE is missing: `sudo apt install libfuse2` (22.04) / `libfuse2t64` (24.04+), or run with `--appimage-extract-and-run`. Record which.) |
| A2 | On Ubuntu 24.04 or later, open a PDF in the AppImage and scroll. | Pages show. (If the window stays blank or the log says the web process crashed or "bwrap"/"user namespaces", that is AppArmor's user-namespace restriction against the bundled WebKit: record it.) |
| A3 | `./Tandem-PDF-*.AppImage --appimage-extract` then look at `squashfs-root/usr/lib/Tandem PDF/licenses/system-libraries/`. | `NOTE.txt`, `SYSTEM-LIBRARIES.txt` (every bundled library with its package and version), `packages/<package>/copyright`, `common-licenses/`. No "Not traced" or "without a copyright file" lines at its end (if there are, list them). |

## 2. Opening files

| # | Do | Expected |
|---|----|----------|
| O1 | Windows: right-click a PDF ▸ Open with. Linux: right-click a PDF in Files ▸ Open With… | Tandem PDF is offered. It is not made the default PDF app by installing. |
| O2 | Make it the default (Windows: Open with ▸ Choose another app ▸ Tandem PDF ▸ Always; Linux: Open With… ▸ Tandem PDF ▸ Set as default). Double-click a PDF whose path has spaces and non-ASCII letters, e.g. `C:\Users\<you>\Documents\Prüfung 1\Bericht €.pdf` or `~/Dokumente/Prüfung 1/Bericht €.pdf`. | The app starts with that file open. |
| O3 | With the app open on a document, double-click another PDF. | No second window or second app: the running window comes to the front and the second file opens there (asking first if the current document has unsaved changes). |
| O4 | With the app closed, select three PDFs in Explorer/Files and open them together (Enter, or Open With). | One window; the first opens; a note names the others that were not opened. |
| O5 | From a terminal: Windows `& "$env:LOCALAPPDATA\Tandem PDF\tandem-pdf.exe" a.pdf` while it runs; Linux `tandem-pdf a.pdf` while it runs. | Same as O3: the running window opens `a.pdf`, the command returns. |
| O6 | Drag a PDF from Explorer/Files onto the window; then drag a .txt file. | The PDF opens; for the .txt a veil/note says it can't be opened (only PDFs). |
| O7 | File ▸ Open… (Ctrl+O). | The system's file dialog, filtered to PDF; the chosen file opens. |
| O8 | Open `fixtures/encrypted.pdf` from the source checkout (copy it first). | It asks for the password; `pdf4all-user` opens it. |

## 3. The window: title, menus, keys

| # | Do | Expected |
|---|----|----------|
| T1 | Open `report.pdf`. | The title bar reads `report.pdf — Tandem PDF`. |
| T2 | Make an edit (e.g. a highlight). | The title reads `report.pdf (edited) — Tandem PDF`. After Ctrl+S it goes back to `report.pdf — Tandem PDF`. |
| T3 | Look at the window. | A menu bar inside the window: File, Edit, View, Tools, Insert, Page, Help. File has Open Recent, Close Window and Exit (Windows) / Quit (Linux); Help has Tandem PDF Help and About Tandem PDF. |
| T4 | Click into the page (so the page, not the menu, has the focus), then press Ctrl+O, Ctrl+S, Ctrl+Shift+S, Ctrl+P, Ctrl+Z, Ctrl+Shift+Z, Ctrl+F, Ctrl+= / Ctrl+- / Ctrl+0 (also Ctrl++ and the number pad's + and -), Ctrl+Shift+1 … Ctrl+Shift+8, Ctrl+PageDown / Ctrl+PageUp, F9 / Shift+F9; in the Pages tool Ctrl+Shift+PageUp / PageDown and Ctrl+Shift+Backspace. Then hold Ctrl+O down for two seconds, and hold Ctrl+PageDown. | Each does what its menu item says (open dialog, save, save as, print, undo, redo, find, zoom, the eight tools, next/previous page, sidebar/panel, move/delete pages). Holding Ctrl+O opens one dialog only; holding Ctrl+PageDown keeps paging. F5 and Ctrl+R do not reload the window. The shortcut shown next to each menu item is the one that works. Nothing is typed into the page instead. |
| T5 | With a text field of the app focused (Find box, a form field on the page), press Ctrl+C, Ctrl+V, Ctrl+A, Ctrl+Z. | They act on the text in the field (copy, paste, select all, undo typing), not on the document. |
| T6 | File ▸ Open Recent after opening two files. | A submenu with both files, newest first, and "Clear Menu". Choosing one opens it; Clear empties the list (also on the start screen). |
| T7 | Ctrl+W, then (Windows) File ▸ Exit / (Linux) File ▸ Quit or Ctrl+Q. | Ctrl+W closes the window (the app ends: it has one window). Exit/Quit ends the app. With unsaved changes both ask first (T9). |
| T8 | F1. | Help ("Tandem PDF Help") opens, listing Ctrl (not ⌘) shortcuts. |
| T9 | With unsaved changes, close the window (the title bar's X, Alt+F4, Ctrl+W). | A question with Save, Don't Save and Cancel (Windows: Save first; Linux: GNOME order, Cancel on the left and Save on the right). Save is the default button (Enter saves); Esc is Cancel and keeps the window and the changes; Don't Save closes without writing. For an Untitled document the first button reads "Save…" and opens a save dialog. |
| T10 | Alt (Windows) / F10. | Windows: Alt underlines menu access keys or focuses the menu bar; F10 is not taken by the app. |
| T11 | AltGr on the German layout: in the Find box and in a form field type `€` (AltGr+E), `@` (AltGr+Q), `{ [ ] }` (AltGr+7 8 9 0), `\` (AltGr+ß), `~` (AltGr++). | Every character is typed. No menu command runs (AltGr is Ctrl+Alt on Windows: no app shortcut may be Ctrl+Alt). |
| T12 | On the German layout: Ctrl+Z, Ctrl+Shift+Z, then zoom with Ctrl++ (the + key) and Ctrl+-. | Undo/redo use the key labelled Z (where Y is on a US keyboard). Record which keys zoom in: `=` is Shift+0 on German keys. |

## 4. System integration

| # | Do | Expected |
|---|----|----------|
| S1 | File ▸ Print… (Ctrl+P) on a 3-page document. | The system print dialog (Windows: the Windows print dialog; Linux: GTK's) with the printers, page range, copies. |
| S2 | Print pages 2–3, 2 copies, to a real printer. | Two copies of pages 2 and 3, the right way up, sharp, at the right size (a ruler: 100% scale, no shrinking of an A4/Letter page beyond the printer's margins). |
| S3 | Print to "Microsoft Print to PDF" (Windows) / "Print to File" PDF (Linux). Open the result in another viewer. | The pages as shown in the app (annotations and form values included). Record whether text is selectable (it may be an image: the print path renders pages). |
| S4 | Cancel the print dialog. Windows: also choose "Microsoft Print to PDF" and cancel its Save dialog. | Nothing printed; the window is usable; no error. |
| S4b | Print a landscape page and a small page (e.g. A6) on A4/Letter paper. | The landscape page is turned to fill the portrait paper; the small page prints at its real size, centred, not blown up (as on a Mac). |
| S5 | Select text on a page (Select tool, drag over words), Ctrl+C, paste into Notepad / a text editor. | The selected words, in reading order. Also Edit ▸ Copy. |
| S6 | Save a file, then in the "Saved" notice click "Show in Explorer" / "Show in Folder". | Explorer / Files opens that folder with the file selected (Linux: selected where the file manager supports it, else the folder opens). |
| S7 | Open a PDF with a link to a website (copy) and click the link. | A question first (default Cancel); "Open" opens the link in the default browser. A link to a local file is not opened and says why. |
| S8 | Settings: switch the system to dark mode (Windows: Settings ▸ Personalisation ▸ Colours ▸ Dark; Ubuntu: Settings ▸ Appearance ▸ Dark), with the app open, then restart it. | The app follows (dark start screen, panels, dialogs; pages stay white). Text everywhere readable. The in-window menu bar too (record if it stays light). |
| S9 | High-DPI: Windows display scale 150% and 200%; Ubuntu Settings ▸ Displays ▸ Scale 200% (and fractional 125% if offered). Also move the window between a 100% and a 200% monitor. | Text and icons sharp (not blurry), page renders sharp at 100% zoom, the window keeps sensible size, hit targets line up with what is drawn (click a form field: the editor opens on it). |

## 5. Feature click-through (copy fixtures from the source checkout, or use your own PDFs)

| # | Area | Do | Expected |
|---|------|----|----------|
| F1 | Open/view | Open `large.pdf`; scroll, Ctrl+= / Ctrl+-, Fit Width (View), Page Down; thumbnails sidebar. | Smooth enough; page number follows; thumbnails click to the page. |
| F2 | Annotate | Highlight a sentence (Ctrl+Shift+2 then drag), a note, a drawing, a stamp. Save, reopen. | All there after reopening; also visible in another PDF viewer (Edge / Evince). |
| F3 | Edit text | Tools ▸ Text on `fixtures/p3/*.pdf` (copy): change a word. | The word changes in place in the same font; Undo takes it back. |
| F4 | Forms | `form2.pdf`: fill text fields, tick boxes, choose list items; Save; reopen. | Values kept; visible in another viewer. |
| F5 | Redact | `redact.pdf`: Redact tool, draw a box over text, Apply, save as a copy. | The copy has a black box and the text under it is gone (copy-paste and Find in another viewer find nothing there). |
| F6 | Pages | Pages tool: rotate, move, delete, insert pages from another file, extract pages. | Each works; Undo undoes one action at a time. |
| F7 | Sign | Fill & Sign ▸ Add My Signature (draw one), place it, save. Then Sign Digitally… with a .p12 you have (or skip and note it). | The drawn signature appears and is saved; a digital signature verifies as "Valid"/"Not trusted" in the app and in Adobe Reader if available. |
| F8 | Print | (Section 4, S1–S4.) | |
| F9 | Data folder | Close the app; Windows open `%APPDATA%\Tandem PDF`, Linux `~/.config/Tandem PDF`. | `recent-files.json` (and `drawn-signature.json` after F7, trust list after trusting a certificate). If you had the old Electron app, its recent files appear in the new app's list once (imported from `pdf4all-p0`), and the old folder is unchanged. |

## 6. Screenshots and window scripts (`--shot`, `--walk`)

Use a fresh data folder and copies of fixtures.

- Windows (PowerShell):
  `& "$env:LOCALAPPDATA\Tandem PDF\tandem-pdf.exe" --user-data-dir="$env:TEMP\tp-udata" --window-size=1280x820 --shot=1500:"$env:TEMP\shot.png" "$env:TEMP\hello.pdf"`
- Linux: `tandem-pdf --user-data-dir=/tmp/tp-udata --window-size=1280x820 --shot=1500:/tmp/shot.png /tmp/hello.pdf`

Expected: the app opens, waits 1.5 s, writes the PNG of the window's content (no title bar) and quits; the PNG shows
hello.pdf. Then a walk: a file `walk.jsonl` with
`{"open":"<abs path of a copy of hello.pdf>"}`, `{"wait":800}`, `{"shot":"<abs path>/w1.png"}`,
`{"menu":"view:zoom-in"}`, `{"wait":800}`, `{"shot":"<abs path>/w2.png"}` (one per line), run with
`--walk=<abs path>/walk.jsonl`. Expected: two PNGs, the second zoomed in; the app quits by itself. Record if a shot
is black or empty (the web view's snapshot is not available on that OS) — the docs must then say so.

## 7. Agents (MCP)

From docs/agent.md:

- Windows: `claude mcp add tandem-pdf -- "%LOCALAPPDATA%\Tandem PDF\tandem-pdf.exe" --mcp` (in PowerShell write the
  path as `"$env:LOCALAPPDATA\Tandem PDF\tandem-pdf.exe"`; in Claude Desktop's JSON each `\` as `\\`).
- Linux: `claude mcp add tandem-pdf -- tandem-pdf --mcp` (.deb), or the AppImage's full path with `--mcp`.

Expected: in Claude Code, `/mcp` lists tandem-pdf as connected with six tools (tandem_open, tandem_new, tandem_do,
tandem_state, tandem_save, tandem_help). Ask it to open a copy of hello.pdf, add a highlight and save to a new file:
the file is written and opens in the app with the highlight. No window opens while the agent works. Also start the
binary by hand with `--mcp` and type nothing: it waits silently (no window); Ctrl+C ends it.

## 8. Licences in the packages

| # | Do | Expected |
|---|----|----------|
| C1 | Open `licenses\THIRD-PARTY.txt` (Windows install folder) / `/usr/lib/Tandem PDF/licenses/THIRD-PARTY.txt`. | Its first line names the target (`x86_64-pc-windows-msvc` / `x86_64-unknown-linux-gnu`); every crate has a folder under `crates\` with its licence file or a `LICENSE-NOTE.txt`. |
| C2 | Linux AppImage: A3 above. | System libraries' texts present. |

## 9. The app suite on the real machine

From the source checkout of the same commit, with Node 22, after installing (paths as installed):

- Windows (Git Bash): `npm ci && npm run build:test && npm run build:oracle` (needs the engine WASM package in
  `engine/web/pkg`: download the **engine-pkg** artifact of the same run into it), then
  `TANDEM_APP_BIN="$LOCALAPPDATA/Tandem PDF/tandem-pdf.exe" node --test --test-concurrency=2 tests/*.test.mjs`
- Linux: the same with `TANDEM_APP_BIN=/usr/bin/tandem-pdf` (from a desktop session, not over ssh without a display).

Expected: `# fail 0` at the end (tests that need the private signing keys fail without them: run them where the keys
are, or leave those files out as the workflow does). Keep the output.

## Proven on the Mac

What the Mac can and cannot prove for this list (details in `docs/archive/electron-retirement-report.md`):

- **Compiles and lints for both targets.** The Windows and Linux shells, and the core with its tests, pass clippy
  `-D warnings` for `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu` on Rust 1.98.1 and 1.92. A stub resource
  compiler and pkg-config are used, so nothing is linked. These are gates in `tools/gates.sh`.
- **Unit-tested here.** The decisions behind sections 2–4 are unit-tested on the Mac:
  - the Windows/Linux shortcut table: unique, no Ctrl+Alt, every Mac item decided;
  - the WebView2 key mapping: page keys, the number pad, Ctrl++, AltGr never a shortcut;
  - the window title with and without "(edited)";
  - print layout and rotation;
  - the installed resource folders;
  - the per-OS save error numbers;
  - the Windows folder flush being skipped;
  - the Electron data import;
  - the second-launch hand-over, which was also proven with the real Mac window.
- **The same core runs everywhere.** Everything behind the menus is the core, which is the same on every OS. The app
  suite (606 tests, 0 fail) and 432 core tests exercise it on the Mac, and the drive and MCP protocol are identical.
- **Configs checked.** The Tauri bundle configs for each OS were checked against Tauri's schema, and the licence lists
  for each target against the allow-list.
- **Only on real machines.** Sections 1–9 above themselves: dialogs, printing, the in-window menu bar and its keys,
  installers and file association, high-DPI, AltGr, dark mode, and the AppImage's system libraries.
- **Mac-only by design.** The proxy icon and edited dot in the title bar, the system's Open Recent, ⌘? for Help,
  PDFKit printing and Finder's open events.

## Questions for the owner (from packaging)

- WebView2 on Windows: the installer embeds the small WebView2 bootstrapper (about 1.8 MB), which installs WebView2
  only where it is missing and needs internet for that. Every Windows 11 and updated Windows 10 already has it, so
  offline installs work there. The alternative is the offline WebView2 installer inside every download (about
  +127 MB). Keep the bootstrapper?
- The AppImage ships system libraries (GTK, WebKitGTK, GLib, ... mostly LGPL-2.1+). Their licence texts are inside
  it (licenses/system-libraries). Whether the LGPL also needs a written source offer for those exact versions (they
  are Ubuntu 22.04's packages, whose sources Ubuntu publishes) is a question for the licence review. Ship the
  AppImage, or only the .deb until that is answered?
- Code signing: Windows installers are unsigned (SmartScreen warns) until a code-signing certificate exists (secrets
  WIN_CERT_PFX / WIN_CERT_PASSWORD); macOS needs the Developer ID (already open).
