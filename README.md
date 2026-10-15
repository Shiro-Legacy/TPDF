# Tandem PDF

A local, offline PDF editor for macOS, Windows and Linux that a person and an AI agent can both drive (Electron + MuPDF 1.28.1 WASM, TypeScript). No account, no backend, no network at run time. It was called PDF4All until 2026-09-30; internal identifiers (`app://pdf4all`, `window.pdf4allState()`, `/PDF4AllF` in written PDFs, the repo folder) keep the old name on purpose.

## Download

Installers are on the [latest release](https://github.com/Shiro-Legacy/TPDF/releases/latest).

| System | File | Install |
|---|---|---|
| macOS 13 or later, Apple Silicon | `Tandem-PDF-<version>-mac-arm64.dmg` | Open it and drag Tandem PDF to Applications. There is no Intel build. |
| Windows 10 and 11, x64 | `Tandem-PDF-<version>-windows-x64-setup.exe` | Run it: a per-user install, no admin prompt. The installer is not code-signed, so SmartScreen may warn: More info → Run anyway. |
| Ubuntu 22.04, 24.04 and 26.04, x64 | `tandem-pdf_<version>_amd64.deb` | `sudo apt install ./tandem-pdf_<version>_amd64.deb` |
| Other Linux, x64 | `Tandem-PDF-<version>-linux-x64.tar.gz` | Unpack it, then `bash scripts/install-linux.sh` in the unpacked folder (per user, no root). |

Linux support: the three Ubuntu releases above are the supported systems; the release workflow installs the `.deb` on each and runs the app's self-test there, and the app is developed on 26.04. Debian and other glibc distributions on x64 are expected to work through the `.deb` or the tarball but are not tested. There is no ARM build and no Flatpak, Snap or AppImage. The `.deb` sets up Chromium's sandbox itself; after a tarball install on Ubuntu 24.04 or later, read "Linux sandbox" below.

Every install puts the app under "Open with" for PDFs and leaves the default PDF app as it is. Limits worth knowing first: files up to 120 MiB open and up to 50 MiB save, there is no OCR, and the interface is dark only.

## What it does

- Read, navigate, search, zoom, print; thumbnails, bookmarks, page labels.
- Annotate (highlight, notes, ink, shapes, stamps); organise pages (rotate, delete, move, insert, extract, crop).
- Fill and design AcroForm fields, on the page or in the panel.
- Sign by hand: draw your signature once, or use a picture of it (Fill forms → Draw signature…), then Sign and click the page; drag, resize or delete it there. It is a picture kept on this computer, not a certificate signature.
- Edit existing content: text runs (including symbolic "coded" simple fonts with no `/Encoding`, e.g. LibreOffice exports), multi-run paragraphs, images and objects, find/replace, add text.
- Redact (mark, preview, apply into a sanitised new file).
- Certificate signing (PKCS#12/PEM, CMS `adbe.pkcs7.detached`, DocMDP) and in-app verification, including signing an empty field on the page.
- Direct manipulation on the page: move/resize objects, links, fields and redaction marks; Tab order from `/Tabs`.
- Bates numbering (`{bates}` in header/footer stamps, counter continues across files) and Batch: one stamp action over a folder of PDFs with a progress table and report.
- Compare two documents side by side (line-level text diff plus pixel diff).
- New blank document (File → New… ⌘N; on Windows and Linux every ⌘ shortcut is Ctrl and ⌥ is Alt, in the menus and in Help).

Every edit is one undoable journal op; saves are `incremental=no, garbage=yes`; originals are never touched until you save. PDF JavaScript never runs.

## Build from source

Node 22.12+ and npm 10; exact dependencies in `package-lock.json`. Supported hosts: macOS 13+ on Apple Silicon, Windows 10/11 x64 (ported 2026-10-01; the commands below run in PowerShell or Git Bash), and Linux x64 (ported 2026-10-01 on Ubuntu 26.04; Electron needs a display, so on a headless box or to keep test windows off your screen run `xvfb-run -a npm test`; read "Linux sandbox" below before the first run).

```sh
npm ci
npm exec -- install-electron
```

## Commands

```sh
npm run build      # tsc + preload + asset copy into build/, with the Rust worker engine (needs an up-to-date engine/web/pkg: engine/tools/build-workflow.sh)
npm run build:mupdf   # the same with the MuPDF worker engine (also TPDF_ENGINE=mupdf npm run build)
npm start          # build and launch the app
npm test           # build (Rust engine) and run tests/*.test.mjs (CI_NO_ELECTRON=1 skips Electron-spawning tests)
npm run test:mupdf # the same suite on a MuPDF build: the engine `npm run package` ships
npm run smoke -- --report=<path>   # build, then a source (not packaged) `electron . --smoke` self-test
npm run package    # bundle a MuPDF build for this host (macOS: unsigned unless APPLE_SIGN_IDENTITY is set, see scripts/package.mjs): out/Tandem PDF-darwin-arm64/Tandem PDF.app, out/Tandem PDF-win32-x64/Tandem PDF.exe or out/Tandem PDF-linux-x64/Tandem PDF
npm run package:rust   # the same bundle with the Rust worker engine and its notices (not what releases ship yet)
```

A Rust build refuses an `engine/web/pkg` built from other sources than `engine/` holds (`sources.sha256`, written by `engine/tools/build-workflow.sh`): rebuild it first, or set `TPDF_ALLOW_STALE_ENGINE=1` to build with it anyway.

Install: `ditto "out/Tandem PDF-darwin-arm64/Tandem PDF.app" "/Applications/Tandem PDF.app"` (quit the running app first). The bundle has its own icon (`assets/icon.svg` → `scripts/make-icon.mjs` → `assets/icon.icns`), registers as an "Open With" editor for PDFs, and carries the MCP server under `Contents/Resources/agent/`. App data lives in `~/Library/Application Support/Tandem PDF/`.

Install on Windows (per user, no admin; quit the running app first): `powershell -NoProfile -ExecutionPolicy Bypass -File scripts\install-windows.ps1` copies `out\Tandem PDF-win32-x64` to `%LOCALAPPDATA%\Programs\Tandem PDF`, adds a Start menu shortcut and lists the app under "Open with" for PDFs (the default PDF app is not changed); `-Uninstall` removes all three. The exe carries `assets/icon.ico` and the MCP server under `resources\agent\`. App data lives in `%APPDATA%\Tandem PDF\`. The build is unsigned, so Windows SmartScreen may warn on first run.

Install on Linux (per user, no root; quit the running app first): `bash scripts/install-linux.sh` copies `out/Tandem PDF-linux-x64` to `~/.local/share/tandem-pdf`, adds a launcher (`~/.local/share/applications/tandem-pdf.desktop` and a 512 px icon) and lists the app under "Open with" for PDFs (the default PDF app is not changed); `--uninstall` removes all three, `--dry-run` only prints, `--to=<dir>` puts the app folder somewhere else (the launcher and icon still go under `~/.local/share`, where the desktop looks; pass the same `--to` to `--uninstall`). The MCP server is under `resources/agent/`. App data lives in `~/.config/Tandem PDF/`.

Linux sandbox: the app never runs with `--no-sandbox`. Ubuntu 23.10 and later restrict unprivileged user namespaces through AppArmor (`sysctl kernel.apparmor_restrict_unprivileged_userns` = 1), and there Electron aborts at start ("No usable sandbox" / "The SUID sandbox helper binary was found, but is not configured correctly") until an AppArmor profile names the binary. `scripts/install-linux.sh` writes that profile for the installed app and prints the two `sudo` commands that load it. The repo's own `node_modules/electron/dist/electron` (`npm start`, `npm test`) and `out/Tandem PDF-linux-x64/Tandem PDF` need the same from a plain terminal: a profile like the installed one with those paths, or a parent process whose profile already allows `userns`.

Release: installers come from the packaged build with `bash scripts/make-dmg.sh` (macOS), `iscc /DVersion=<version> scripts\installer.iss` (Windows, Inno Setup 6) and `bash scripts/make-linux.sh` (Linux, needs `dpkg-deb`). Before tagging, run `npm run test:mupdf` on each seat: releases ship the MuPDF build. Pushing a tag `v<version>` runs `.github/workflows/release.yml`, which builds all of them, self-tests each packaged app and drafts the GitHub Release; the signing secrets it reads are listed at its top.

Python QA helpers (`scripts/verify-*.py`) use a dev-only venv: `python3 -m venv .venv-qa && .venv-qa/bin/pip install pypdf==6.18.1 pyHanko==0.37.0` (Windows: `python -m venv .venv-qa`, then `.venv-qa\Scripts\pip`; the independent page render there is `scripts/render-page.ps1`, Windows' own PDF engine, in place of `qlmanage`; on Linux it is poppler's `pdftoppm`, package `poppler-utils`).

## Driving the app

`node scripts/drive.mjs <script.jsonl> [--gui] [--out=<dir>] [--report=<path>]` replays a script against the built app (headless unless `--gui`) and prints a report with the STATE after every step; it exits 0 only if every `expect` matched; without `--out`, `${OUT}` is a temp folder deleted at exit. Scripts live in `tests/drive/`; the command and STATE vocabulary is in `.quad/shared/p3/design-agent-drive.md`. Run `npm run build` first.

Agents: `node scripts/drive.mjs --session` keeps one app running and answers one JSON step per stdin line (`{"cmd":"new","contents":{"size":"a4"}}` makes a blank Untitled.pdf; File → New… ⌘N does the same in the GUI).
MCP (stdio, no dependency beyond `node`; tools `tandem_open/new/do/state/save/help`): `claude mcp add tandem-pdf -- node "/Applications/Tandem PDF.app/Contents/Resources/agent/scripts/tandem-mcp.mjs"`, or in Claude Desktop `{"mcpServers":{"tandem-pdf":{"command":"node","args":["/Applications/Tandem PDF.app/Contents/Resources/agent/scripts/tandem-mcp.mjs"]}}}`. On Windows the installed server is `%LOCALAPPDATA%\Programs\Tandem PDF\resources\agent\scripts\tandem-mcp.mjs`, on Linux `/opt/tandem-pdf/resources/agent/scripts/tandem-mcp.mjs` after the `.deb`, or `~/.local/share/tandem-pdf/resources/agent/scripts/tandem-mcp.mjs` after `install-linux.sh` (register it by its full path). The server needs `node` on the PATH. From the repo the same server is `scripts/tandem-mcp.mjs` (needs `npm run build`).
The agent reference (tools, create/modify/fill recipes, refusals, ceilings) is `docs/agent.md`.

## Fixtures

Every file in `fixtures/` is generated by a deterministic `make-*` script, has a `<name>.provenance.json`, and is pinned by sha256 in `tests/fixtures.test.mjs`. Regenerate from the generator; never hand-edit.

## Status and licence

Progress, decisions and open debt: `progress.md`. Tandem PDF's own code is MIT (`LICENSE`). Builds bundle MuPDF (AGPL-3.0-or-later), so any distributed build must meet AGPL obligations; see `docs/third-party/` and `.quad/shared/p0/p0-dependencies.md`. Each release is that distribution: its tag is the app's corresponding source, the MuPDF source archive is attached to it, and every installed copy carries the notices in its `licenses` folder (`NOTICE.txt` first).
