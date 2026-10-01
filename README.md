# Tandem PDF

A local, offline PDF editor for macOS and Windows that a person and an AI agent can both drive (Electron + MuPDF 1.28.1 WASM, TypeScript). No account, no backend, no network at run time. This is a private build, not a release. It was called PDF4All until 2026-09-30; internal identifiers (`app://pdf4all`, `window.pdf4allState()`, `/PDF4AllF` in written PDFs, the repo folder) keep the old name on purpose.

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
- New blank document (File → New… ⌘N; on Windows every ⌘ shortcut is Ctrl and ⌥ is Alt, in the menus and in Help).

Every edit is one undoable journal op; saves are `incremental=no, garbage=yes`; originals are never touched until you save. PDF JavaScript never runs.

## Setup

Node 22.12+ and npm 10; exact dependencies in `package-lock.json`. Supported hosts: macOS 13+ on Apple Silicon, and Windows 10/11 x64 (ported 2026-10-01; the commands below run in PowerShell or Git Bash).

```sh
npm ci
npm exec -- install-electron
```

## Commands

```sh
npm run build      # tsc + preload + asset copy into build/
npm start          # build and launch the app
npm test           # build and run tests/*.test.mjs (CI_NO_ELECTRON=1 skips Electron-spawning tests)
npm run smoke -- --report=<path>   # build, then a source (not packaged) `electron . --smoke` self-test
npm run package    # unsigned bundle for this host: out/Tandem PDF-darwin-arm64/Tandem PDF.app or out/Tandem PDF-win32-x64/Tandem PDF.exe
```

Install: `ditto "out/Tandem PDF-darwin-arm64/Tandem PDF.app" "/Applications/Tandem PDF.app"` (quit the running app first). The bundle has its own icon (`assets/icon.svg` → `scripts/make-icon.sh` → `assets/icon.icns`), registers as an "Open With" editor for PDFs, and carries the MCP server under `Contents/Resources/agent/`. App data lives in `~/Library/Application Support/Tandem PDF/`.

Install on Windows (per user, no admin; quit the running app first): `powershell -NoProfile -ExecutionPolicy Bypass -File scripts\install-windows.ps1` copies `out\Tandem PDF-win32-x64` to `%LOCALAPPDATA%\Programs\Tandem PDF`, adds a Start menu shortcut and lists the app under "Open with" for PDFs (the default PDF app is not changed); `-Uninstall` removes all three. The exe carries `assets/icon.ico` and the MCP server under `resources\agent\`. App data lives in `%APPDATA%\Tandem PDF\`. The build is unsigned, so Windows SmartScreen may warn on first run.

Python QA helpers (`scripts/verify-*.py`) use a dev-only venv: `python3 -m venv .venv-qa && .venv-qa/bin/pip install pypdf==6.18.1 pyHanko==0.37.0` (Windows: `python -m venv .venv-qa`, then `.venv-qa\Scripts\pip`; the independent page render there is `scripts/render-page.ps1`, Windows' own PDF engine, in place of `qlmanage`).

## Driving the app

`node scripts/drive.mjs <script.jsonl> [--gui] [--out=<dir>] [--report=<path>]` replays a script against the built app (headless unless `--gui`) and prints a report with the STATE after every step; it exits 0 only if every `expect` matched; without `--out`, `${OUT}` is a temp folder deleted at exit. Scripts live in `tests/drive/`; the command and STATE vocabulary is in `.quad/shared/p3/design-agent-drive.md`. Run `npm run build` first.

Agents: `node scripts/drive.mjs --session` keeps one app running and answers one JSON step per stdin line (`{"cmd":"new","contents":{"size":"a4"}}` makes a blank Untitled.pdf; File → New… ⌘N does the same in the GUI).
MCP (stdio, no dependency beyond `node`; tools `tandem_open/new/do/state/save/help`): `claude mcp add tandem-pdf -- node "/Applications/Tandem PDF.app/Contents/Resources/agent/scripts/tandem-mcp.mjs"`, or in Claude Desktop `{"mcpServers":{"tandem-pdf":{"command":"node","args":["/Applications/Tandem PDF.app/Contents/Resources/agent/scripts/tandem-mcp.mjs"]}}}`. On Windows the installed server is `%LOCALAPPDATA%\Programs\Tandem PDF\resources\agent\scripts\tandem-mcp.mjs` (register it by its full path). From the repo the same server is `scripts/tandem-mcp.mjs` (needs `npm run build`).
The agent reference (tools, create/modify/fill recipes, refusals, ceilings) is `docs/agent.md`.

## Fixtures

Every file in `fixtures/` is generated by a deterministic `make-*` script, has a `<name>.provenance.json`, and is pinned by sha256 in `tests/fixtures.test.mjs`. Regenerate from the generator; never hand-edit.

## Status and licence

Progress, decisions and open debt: `progress.md`. Tandem PDF's own code is MIT (`LICENSE`). Builds bundle MuPDF (AGPL-3.0-or-later), so any distributed build must meet AGPL obligations; see `docs/third-party/` and `.quad/shared/p0/p0-dependencies.md`. Do not redistribute `out/`: these are private, unsigned development bundles.
