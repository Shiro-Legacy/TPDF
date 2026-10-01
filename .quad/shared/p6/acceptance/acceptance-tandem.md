# Acceptance — tandem (2026-09-30, fable51)

Order: human in chat — name the app and package it as its own app. Answers: name **Tandem PDF**, scope **install on this Mac**.

| Check | Outcome |
|---|---|
| `npx tsc --noEmit -p tsconfig.json` | clean |
| `npm test` (`tandem-suite.log`) | 626 tests / 625 pass / 0 fail / 1 skip (baseline unchanged) |
| `shasum -c checks/fixtures.sha256` | 9 OK |
| fixture snapshot vs `checks/tandem-hash-before.txt` (56 lines) | identical |
| `npm run package` | `out/Tandem PDF-darwin-arm64/Tandem PDF.app` |
| Info.plist | CFBundleName/DisplayName "Tandem PDF", id `com.shiro-legacy.tandem-pdf`, 0.1.0, CFBundleDocumentTypes com.adobe.pdf Editor/Alternate |
| icon | `Contents/Resources/electron.icns` byte-equal to `assets/icon.icns`; Finder-rendered icon exported and viewed |
| installed | `ditto` → `/Applications/Tandem PDF.app`, `lsregister -f` |
| `checks/packaged-mcp-check.mjs` on the installed app (`tandem-packaged-mcp.log`) | server `tandem-pdf` 0.1.0; 6 `tandem_*` tools; new → text-add → save = validated 781-byte PDF; help ok |
| stray Electron | none |

Not done: GUI launch, About panel and Finder "Open With" by eye (human at the machine); no red-before log (no new test file). Backup: `.quad/backup/pre-tandem-20260930.tgz`.
