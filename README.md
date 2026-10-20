# Tandem PDF

A local, offline PDF editor for macOS, Windows and Linux that a person and an AI agent can both drive. It is all Rust: the
Tandem PDF engine (`engine/`), the app core (`crates/tandem-app`), the window (Tauri 2, `crates/tandem-desktop`) and its
interface (Leptos compiled to WASM, `crates/tandem-ui`); MuPDF 1.28.1 is a test-only oracle. No account, no backend, no
network at run time. It was called PDF4All until 2026-09-30; internal identifiers (`/PDF4AllF` in written PDFs, the repo
folder) keep the old name on purpose. Until 2026-10 the desktop app was an Electron program; that app is retired
(`docs/electron-retirement-report.md`), and its settings carry over on first start.

## Download

Installers are on the [latest release](https://github.com/Shiro-Legacy/TPDF/releases/latest).

| System | File | Install |
|---|---|---|
| macOS 12 or later, Apple Silicon | `Tandem-PDF-<version>-macos-arm64.dmg` | Open it and drag Tandem PDF to Applications. There is no Intel build. |
| Windows 10 and 11, x64 | `Tandem-PDF-<version>-windows-x64-setup.exe` | Run it: a per-user install to `%LOCALAPPDATA%\Tandem PDF`, no admin prompt. The installer is not code-signed yet, so SmartScreen may warn: More info → Run anyway. |
| Ubuntu 22.04, 24.04 and 26.04, x64 | `tandem-pdf_<version>_amd64.deb` | `sudo apt install ./tandem-pdf_<version>_amd64.deb` |
| Other Linux, x64 | `Tandem-PDF-<version>-linux-x86_64.AppImage` | `chmod +x` it and run it. |

Linux support: the three Ubuntu releases above are the supported systems; the release workflow installs the `.deb` and
runs the AppImage on them and runs the app's self-test there. Debian and other glibc distributions on x64 with
WebKitGTK 4.1 are expected to work but are not tested. There is no ARM build and no Flatpak or Snap.

Every install puts the app under "Open with" for PDFs and leaves the default PDF app as it is. Limits worth knowing
first: files up to 120 MiB open and up to 50 MiB save, and there is no OCR. App data lives in
`~/Library/Application Support/Tandem PDF/` (macOS), `%APPDATA%\Tandem PDF\` (Windows) or `~/.config/Tandem PDF/`
(Linux).

## What it does

- Read, navigate, search, zoom, print; thumbnails, bookmarks, page labels.
- Annotate (highlight, notes, ink, shapes, stamps); organise pages (rotate, delete, move, insert, extract, crop).
- Fill and design AcroForm fields, on the page or in the panel.
- Sign by hand: draw your signature once, or use a picture of it (Fill forms → Draw signature…), then Sign and click the page; drag, resize or delete it there. It is a picture kept on this computer, not a certificate signature.
- Edit existing content: text runs (including symbolic "coded" simple fonts with no `/Encoding`, e.g. LibreOffice exports), multi-run paragraphs, images and objects, find/replace, add text.
- Redact (mark, preview, apply into a sanitised new file).
- Certificate signing (PKCS#12/PEM; RSA, P-256 or P-384 keys; PAdES baseline B-B: SubFilter `ETSI.CAdES.detached`, DocMDP) and in-app verification (also of older `adbe.pkcs7.detached` signatures), including signing an empty field on the page. The cryptography is the Rust crate `crates/tandem-sign`.
- Direct manipulation on the page: move/resize objects, links, fields and redaction marks; Tab order from `/Tabs`.
- Bates numbering (`{bates}` in header/footer stamps, counter continues across files) and Batch: one stamp action over a folder of PDFs with a progress table and report.
- Compare two documents side by side (line-level text diff plus pixel diff).
- New blank document (File → New…, ⌘N on a Mac, Ctrl+N on Windows and Linux; the menus, tooltips and Help show each system's own keys).

Every edit is one undoable journal op; saves are `incremental=no, garbage=yes`; originals are never touched until you save. PDF JavaScript never runs.

## Build from source

Rust 1.98.1 builds the app; 1.92 must also compile it (both are checked by CI). You also need the `wasm32-unknown-unknown`
target, `wasm-bindgen-cli` 0.2.129 (the UI build looks for it on the PATH or in `WASM_BINDGEN_BIN`), and for installers
`tauri-cli` 2.12.3 (`cargo install tauri-cli --version 2.12.3 --locked`). On Linux: `libwebkit2gtk-4.1-dev libgtk-3-dev
librsvg2-dev libdbus-1-dev patchelf`. Node 22.12+ and npm 10 are only for the test suites and the MuPDF oracle (`npm ci`;
exact versions in `package-lock.json`).

```sh
sh crates/tandem-ui/build.sh                     # the interface, to WASM (crates/tandem-ui/dist); the window embeds it
cd crates && cargo run -p tandem-desktop         # build and open the app
cargo build --release -p tandem-desktop          # crates/target/release/tandem-pdf, which the test suites drive
cargo test                                       # the core, protocol, UI logic and shell tests
```

The engine's WASM package (`engine/web/pkg`, for the mobile app and the engine-adapter tests) is built by
`engine/tools/build-workflow.sh`; a build refuses a package made from other sources than `engine/` holds
(`sources.sha256`), or set `TPDF_ALLOW_STALE_ENGINE=1` to use it anyway.

## Commands

```sh
npm run build      # the interface to WASM, and the engine-adapter JS the tests load
npm run build:oracle  # the test-only MuPDF oracle (engine/oracle/app-engine, AGPL) into build-oracle/; tests reopen the app's output with it
npm start          # build and open the app
npm test           # build, the oracle, and tests/*.test.mjs against crates/target/release/tandem-pdf (or TANDEM_APP_BIN)
npm run test:rust  # the engine's WASM package through the adapter
npm run smoke -- --report=<path>   # the release binary's --smoke self-test
npm run package    # this system's installers (scripts/package-mac.sh, package-windows.ps1, package-linux.sh) into crates/target/release/bundle/
```

macOS packages are signed ad hoc unless `APPLE_SIGNING_IDENTITY` (and `APPLE_ID`/`APPLE_PASSWORD`/`APPLE_TEAM_ID` for
notarization) are set; Windows ones are unsigned unless `TANDEM_WIN_CERT_THUMBPRINT` names a certificate. Each script says
what it built. What to check by hand on a real Windows PC and Linux desktop before a release is in
`docs/windows-linux-checklist.md`.

Release: pushing a tag `v<version>` (or running the workflow by hand) runs `.github/workflows/release.yml` on macOS,
Windows and Linux: it builds the interface, runs fmt, clippy (1.98.1 and 1.92) and the crate tests, packages the app,
runs the app suite against that runner's own release binary, tests the installed app, and drafts the GitHub Release with
`SHA256SUMS`; the signing secrets it reads are listed at its top.

Python QA helpers (`scripts/verify-*.py`) use a dev-only venv: `python3 -m venv .venv-qa && .venv-qa/bin/pip install pypdf==6.18.1 pyHanko==0.37.0` (Windows: `python -m venv .venv-qa`, then `.venv-qa\Scripts\pip`; the independent page render there is `scripts/render-page.ps1`, Windows' own PDF engine, in place of `qlmanage`; on Linux it is poppler's `pdftoppm`, package `poppler-utils`).

## Driving the app

`node scripts/drive.mjs <script.jsonl> [--gui] [--out=<dir>] [--report=<path>]` replays a script against the release
build (headless unless `--gui`; `TANDEM_APP_BIN` or `--app=` picks another binary) and prints a report with the STATE after
every step; it exits 0 only if every `expect` matched; without `--out`, `${OUT}` is a temp folder deleted at exit. Scripts
live in `tests/drive/`; the command and STATE vocabulary is in `.quad/shared/p3/design-agent-drive.md`. The binary takes
the same scripts itself: `tandem-pdf --drive=<script.jsonl|->`.

Agents: the app is its own MCP server (stdio; tools `tandem_open/new/do/state/save/help`), nothing else to install:
`claude mcp add tandem-pdf -- "/Applications/Tandem PDF.app/Contents/MacOS/tandem-pdf" --mcp`. On Windows the binary is
`%LOCALAPPDATA%\Tandem PDF\tandem-pdf.exe`, on Linux `/usr/bin/tandem-pdf` (deb) or the AppImage by its full path. The
agent reference (install lines, tools, create/modify/fill recipes, refusals, ceilings) is `docs/agent.md`.

## Fixtures

Every file in `fixtures/` is generated by a deterministic `make-*` script, has a `<name>.provenance.json`, and is pinned by sha256 in `tests/fixtures.test.mjs`. Regenerate from the generator; never hand-edit.

## Status and licence

Progress, decisions and open debt: `progress.md`. Tandem PDF is MIT (`LICENSE`), including the Rust PDF engine it bundles (`engine/`; its third-party notices are in `engine/NOTICE`). MuPDF is only a black-box test oracle (`engine/oracle/`, AGPL-3.0-or-later, outside the MIT grant and not shipped); the clean-room rewrite that removed MuPDF-derived code is recorded in `docs/mit-clean-room-plan.md`. Each release's tag is the app's source (engine included), and every installed copy carries the notices in its `licenses` folder (`NOTICE.txt` first, then every Rust crate's licence in `THIRD-PARTY.txt`; the AppImage also carries the licences of the system libraries it bundles). The wording of those notices awaits the owner's sign-off (`engine/design/app-m9-mobile-release.md`).
