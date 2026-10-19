# Tandem PDF: everything in Rust, and a UX pass

Status: **approved** (October 18, 2026; `main` at 7ddf541). The owner accepted every decision, D1 to D8, as recommended ("accept all"). Phases 0, 1 and 2 may start; Phase 4 waits for the Phase 3 spike (D7). Nothing in it has been built yet. Screenshots are in `docs/ux-audit/`. They were taken from the current desktop build at 1280×820 (one at 1900×820) on the repository's synthetic fixtures, and from the mobile web build at 375×812.

The plan covers four things:

1. How to make the whole app Rust, with a recommended architecture.
2. A redesign of redaction around "draw a box, everything inside is gone".
3. A walk through every feature as a first-time user, with findings and fixes.
4. A phased order of work, with costs and risks.

The decisions the owner needs to make are listed at the end, in "Decisions for the owner".

---

## 1. What "everything in Rust" should mean

### 1.1 What is not Rust today

| Part | Where | Size | What it does |
|---|---|---|---|
| Desktop shell | `src/main.ts`, `src/safe-write.ts`, `src/policy.ts`, `src/preload.cts` | about 2,650 lines | Electron window, 46 menu items, 19 IPC handlers, file dialogs, journaled safe writes, CSP and network lockdown, printing, the `--drive`/`--smoke` harness that the agent (MCP) and about 760 app tests use |
| Desktop UI | `src/renderer/renderer.ts` and its helpers | about 13,000 lines | All UI state and DOM in one file with no framework. Calls the WASM engine in a Worker. Page pixels arrive as RGBA and are painted to a canvas. |
| Signing crypto | `src/sign/*` | about 1,070 lines | PKCS#12 and PEM reading (KDF, MAC, PBES2, 3DES, RC2-40), CMS SignedData building (RSA PKCS#1 v1.5, SHA-256), verification (RSA v1.5 and PSS, ECDSA; SHA-1 and SHA-2), chain building to a user trust store. The hashes, ciphers and RSA come from `node:crypto`. The DER code is hand-written. |
| Mobile shell and UI | `mobile/src/*` | about 1,400 lines of TypeScript | Capacitor 8 with the filesystem, share and app plugins. Library in IndexedDB, viewer, highlight, ink, note, fill, drawn signature, scanner UI. |
| Mobile native glue | `mobile/ios`, `mobile/android` | about 1,500 lines of Swift and Java | VisionKit or ML Kit document scanner with on-device OCR; ChatGPT account OAuth with tokens in Keychain or Keystore |
| AI | `mobile/src/scan-ai.ts`, `chatgpt.ts` | about 270 lines | "AI-rebuilt document" from scanned pages: page images go to a vision model from one of six providers (the user's own key or ChatGPT plan). Desktop has no in-app AI; agents drive it through `--drive` and `scripts/tandem-mcp.mjs`. |

The engine (`engine/`, about 120 Rust source files, MIT, `unsafe_code = forbid`) already does all PDF work. Today it runs as WASM inside a Web Worker on both platforms. The October 17 benchmarks show it is 2 to 20 times slower than MuPDF at renders and thumbnails because of a fixed cost per call, and up to 26 times slower at warm opens of large files. Running the engine natively instead of as WASM removes part of that cost.

### 1.2 The practical definition

"Everything in Rust" should mean the following:

- **Every line of application logic we write is Rust.** That covers document state, modes and tools, undo, saving, signing, verification, AI calls, the agent drive protocol and file handling.
- **The engine runs natively** on desktop and mobile, not as WASM. WASM remains only as a build target for a possible web version.
- **Platform glue stays in each platform's language, kept as thin as possible:**
  - the OS web view or windowing;
  - Swift and Kotlin for the iOS and Android camera, scanner, OCR, keychain and share sheet;
  - Xcode and Gradle project shells.
- **Markup and styles are not "code"**, so CSS (and HTML that Rust generates) is acceptable. No hand-written TypeScript or JavaScript remains.

### 1.3 UI toolkit options

| Option | Licence | Desktop | iOS/Android | Accessibility | Text input/IME | Fit for us | Main risk |
|---|---|---|---|---|---|---|---|
| **A. Tauri 2 + Leptos** (Rust UI compiled to WASM, rendered by the system web view; native Rust core) | MIT/Apache | Mature (wry/tao; WKWebView, WebView2, WebKitGTK) | Official since Tauri 2.0 (2024). Swift and Kotlin plugin API close to Capacitor's | Full, from the web view | Full, from the web view | Reuses our HTML/CSS design. Page canvas, dialogs and forms are easy. Mobile plugins port almost one to one. | Two Rust halves (core and UI) with IPC between them. Web view differences on Linux. |
| B. Dioxus (Rust UI running natively, DOM edits sent to a web view) | MIT/Apache | Good | Supported, younger than Tauri's | Full (web view) | Full (web view) | No IPC layer: UI and engine share one process. | Smaller ecosystem for native mobile plugins, packaging and updates. Fewer security controls than Tauri's capability model. |
| C. Slint (native rendering, declarative `.slint` markup) | GPLv3, royalty-free (attribution required) or commercial | Good | Android good; iOS newer | AccessKit, partial | Good | Truly native look and speed. | **Licence:** not MIT, so an MIT app needs the royalty-free licence with attribution, or a paid licence. Every view must be rewritten. Weaker accessibility on mobile. |
| D. egui/eframe (immediate mode) | MIT/Apache | Good | Experimental | AccessKit, partial | Weak spots | Fast to build tools. | Looks like a developer tool, not a consumer editor. Weak mobile and IME support. Wrong for "nothing is confusing". |
| E. Iced, Makepad, Xilem, Freya | MIT/Apache | Varies | Little or none | Partial | Partial | | Too young for an app with forms, IME and mobile. |

**Recommendation: A, Tauri 2 with a Leptos UI, around a headless Rust app core. A two-week spike decides between A and B before any real porting (Phase 3).**

The reasons:

- **One UI for desktop and mobile.** Today there are two different UIs (Electron and Capacitor) with different words for the same things; see §3. Tauri 2 runs the same Rust UI on macOS, Windows, Linux, iOS and Android.
- **It keeps what works.** The page is pixels from the engine on a canvas, and everything else is ordinary forms, lists and dialogs, where a web view gives us accessibility, IME, spell-check and printing at no cost. A native toolkit (C or D) would make us rebuild those, worse.
- **The mobile native code ports directly.** `DocumentScannerPlugin.swift`, `ChatGPTAccountPlugin.swift` and their Android twins become Tauri plugins with the same shape (`@objc` methods called by name).
- **Security carries over.** Electron's sandbox, CSP, network denial and one-shot open tokens map onto Tauri capabilities, CSP and a custom `tandem://` protocol. There is no Node in the UI at all.
- **The package gets much smaller**: about 15 to 25 MB instead of about 250 MB for Electron.
- **Licences are clean.** Tauri, wry, tao and Leptos are MIT/Apache. WebView2 and WebKit are system components. The engine stays MIT-clean.

**The key structural change is a headless app core.** Most of `renderer.ts` is application state: the open document, its journal, the mode and armed tool, selections, the field-designer state and the redaction marks. All of that moves into a Rust crate, `tandem-app`. It exposes one command API, which the UI, the menus, the `--drive` protocol and the MCP agent all call. The UI becomes a view of the core's state.

This has three benefits:

- The agent no longer needs a window.
- App tests run as `cargo test`, without Electron.
- The `STATE` dump the tests and agents read is the core's state, not a scrape of the DOM.

```
┌──────────── Tauri 2 process (Rust) ────────────┐
│ tandem-app   state, commands, undo, journal,   │◄── --drive / MCP (same JSON commands as today)
│              autosave, save/print/share         │
│ tandem-pdf-engine (native, MIT)                 │
│ tandem-sign  PKCS#12, CMS, verify (RustCrypto)  │
│ tandem-ai    scan rebuild over HTTPS (rustls)   │
│ platform     dialogs, fs, menus (Tauri plugins) │
└───────────────┬────────────────────────────────┘
                │ typed commands and events (serde); page pixels over tandem:// as raw RGBA
┌───────────────▼────────────────────────────────┐
│ System web view: Leptos UI (Rust → WASM) + CSS │
└────────────────────────────────────────────────┘
 iOS/Android: the same, plus Swift/Kotlin plugins (scanner and OCR, keychain, share sheet)
```

### 1.4 What stays platform glue

| Platform | Stays in the platform's language | Why |
|---|---|---|
| macOS, Windows, Linux | Nothing of ours. Tauri is Rust and uses the system web view. | Optional later: Keychain or Windows certificate-store signing identities through Rust crates (`security-framework`, `windows`), still Rust. |
| iOS (Swift, about 300 lines after porting) | VisionKit document camera and Vision OCR; Keychain plus `ASWebAuthenticationSession` for the ChatGPT account; `UIActivityViewController` share sheet; Info.plist and entitlements | These are OS APIs with no Rust binding that Apple supports. Keys and tokens stay in the Keychain and never cross into Rust or the UI, as today. |
| Android (Kotlin, about 350 lines) | ML Kit document scanner and text recognition; Android Keystore; share intent; Gradle | Same reasons |
| Build | Xcode and Gradle projects that Tauri generates | |

### 1.5 Signing in Rust

The new crate, `tandem-sign`, runs in the core process, so private keys never reach the UI. That matches today's rule of keeping them in the main process. Key buffers are zeroized, and the five-minute one-shot key hold (`hold.ts`) is kept.

| Need | Crate (all RustCrypto, MIT OR Apache-2.0) |
|---|---|
| DER, X.509, OIDs | `der`, `x509-cert`, `spki`, `const-oid` |
| PKCS#12 container and KDF | `pkcs12` (kdf feature), `pkcs8` |
| PBES2 and PBKDF2 | `pkcs5` (pbes2), `pbkdf2`, `hmac` |
| Legacy PBE (3DES, RC2-40) | `des`, `rc2`, `cbc` composed with the PKCS#12 KDF, as `pkcs12.ts` does by hand today |
| Hashes | `sha1`, `sha2` (the engine already uses `sha2` and `aes`) |
| CMS SignedData build and parse | `cms` (has a builder) |
| RSA PKCS#1 v1.5 and PSS | `rsa` |
| ECDSA verify (and later sign) | `ecdsa`, `p256`, `p384`, `p521` |

Notes:

- **`rsa` carries advisory RUSTSEC-2023-0071** (the Marvin timing side channel). Exploiting it means measuring many private-key operations. Tandem signs locally, with the user's own key, a few times per session, so the risk is low. We record it as an accepted risk.
  - The alternative is `aws-lc-rs` (Apache-2.0/ISC, constant-time), but it is not pure Rust: it contains C and assembly.
  - **Owner decision D3.**
- **Certificate chain building stays our own code**, as in `verify.ts` today. `rustls-webpki` is built for TLS and its EKU rules do not fit document signing.
- **What is added while porting** (small items):
  - the ESS signing-certificate-v2 attribute, so signatures meet PAdES B-B and not just `adbe.pkcs7.detached`;
  - ECDSA signing keys;
  - a plain-language verification report (see §3).
  - RFC 3161 timestamps need the network. They would be off by default and opt-in, because the app promises "nothing is uploaded".
- **Differential oracle.** OpenSSL (through `node:crypto` and `verify-sign.py`) remains a test-only oracle, like MuPDF for the engine. Every existing case in `tests/p5-sign-*.test.mjs` and `p5-verify-*.test.mjs` must give the same answers.
- **The crate ships early.** It can land before the shell moves: built for WASM, it loads into Electron's main process in place of `src/sign/*`, which proves it against the current tests.

### 1.6 AI in Rust

- **`tandem-ai`.** The provider calls in `scan-ai.ts` (OpenAI and xAI Responses, Anthropic Messages, DeepSeek and Z.ai chat, Meta) become Rust, using `reqwest` with `rustls` and `serde`. The consent rules stay as they are: images are sent only after the user ticks "Send these page images to …", the API key is held for the session only, and there is a "may still be billed" warning on Stop.
- **ChatGPT account sign-in.** The PKCE logic can move to Rust (`oauth2` crate). Token storage stays in the Swift and Kotlin Keychain or Keystore plugins.
- **Desktop.** It has no AI features. The agent (MCP) path gets better, because it talks to `tandem-app` directly.

### 1.7 Costs and risks of the rewrite

- **Cost: about 6 to 9 months of one experienced Rust engineer**, or the equivalent in agent lanes with independent review (phase table in §4). The largest item is the desktop UI: 13,000 lines of TypeScript, about 760 app tests in 81 test files and 51 drive scripts to port or re-point.
- **Risk: test continuity.** The `--drive` JSON-lines protocol and the `STATE` shape are the contract that the tests, the MCP server and `docs/agent.md` rely on.
  - Mitigation: freeze the protocol first (Phase 3) and implement it in `tandem-app`. Then the same scripts run against both apps until Electron is retired.
- **Risk: web view differences** (WebKitGTK on Linux, WebView2 on Windows).
  - Mitigation: the page itself is engine pixels, so PDF output cannot differ. Only the chrome can. Run CI screenshot checks on all three systems, which needs a push and therefore owner approval.
- **Risk: build memory on this 8 GB Mac.** Tauri plus Leptos plus the engine is a heavy Rust build.
  - Mitigation: keep the slot and memcap rule. Split the workspace so that a UI edit does not rebuild the engine. Measure peak memory in the spike.
- **Risk: Leptos churn** (pre-1.0).
  - Mitigation: Dioxus is the fallback, and the spike compares them. The headless core keeps the UI layer thin, so swapping it is bounded.
- **Risk: two apps live at once** during the migration.
  - Mitigation: from Phase 3 on, Electron only gets fixes, and every new feature lands in the new app.

---

## 2. Redaction redesign: draw a box, everything inside is gone

### 2.1 What happens today

Screenshots: `11-redact-marks.png` and `12-redact-dialog.png`.

1. Pick the **Redact** mode, the sixth of six mode tabs.
2. The tool row offers "Mark area" and a "Mark every match" search field. Dragging on empty page space in this mode also marks an area.
   - The count and the **Apply redactions…** button are past the edge of the tool row. That row scrolls sideways with no visible scrollbar: 357 px shown of 748 (`.ctx`, `styles.css:280`).
3. Marks show as pink boxes over content that is still visible. Each mark also appears in the Comments list as "Redaction mark: (no comment)".
4. **Apply redactions…** opens a dialog with:
   - a table of marks per page;
   - a stale warning: "alternate text on this page is not removed by a rect mark". Apply does remove the alternate text around removed content;
   - "limited to documents up to 12 MiB";
   - nine checkboxes of jargon ("XMP metadata", "Tagged alternate text");
   - an **Apply into a new file** button.
5. The redacted copy is saved, but **the window stays on the unredacted original, which still carries its marks.** The user can easily keep working in the wrong file.

On `fixtures/redact.pdf`, "Mark every match" for the secret marked 9 places. It left **three visible copies on page 1 unmarked**: one drawn as a bitmap picture, and two held in a form field and a text-box comment (`11-redact-marks.png`). This is the owner's point: matching is not enough, and a box is what the user means.

### 2.2 What the engine already does

The engine already works on areas. An area mark is a Redact annotation with a rectangle. `redact-apply` already does the following:

- **Text:** removes glyphs under the box, rewriting `TJ` so that kept glyphs do not move.
- **Paths:** cuts vector paths exactly against the box (`geometry::cut`); fills keep only the area outside the box.
- **Images:** overwrites image pixels under the box, decoding DCT, JPX, JBIG2 and CCITT and re-encoding with Flate. A placement wholly under the box loses its `Do`.
- **Shadings:** clips axial and radial shadings.
- **Annotations:** removes annotations, links and form widgets that a box touches.
- **Hidden data:** strips alternate text from changed content and drops page thumbnails and metadata.
- **Checks:** verifies the copy three ways: text, pixels, and raw strings for search marks.

Details are in `engine/design/app-redact.md`. So the redesign is mostly UX, plus six engine changes where today's rules fall short of "everything inside".

### 2.3 The new flow (desktop and mobile alike)

1. **Redact is a tool, not a mode.** Choose **Redact** in the tool strip (§3.2). The pointer becomes a crosshair, and a chip says "Drag over anything you want removed. Esc to stop."
2. **Drag a box.** It appears at once as a **solid black box**, which is what the result will look like, with a thin red outline to show it is pending.
   - While the pointer is down, or with the "Show what's underneath" toggle, the box turns translucent. Content that will go is tinted red: glyphs, picture areas, shapes, comments and fields.
   - The user therefore sees, before applying, that a glyph cut by the edge goes whole.
3. **Boxes can be moved, resized, deleted, undone and copied**, as today (annotation-backed marks keep all of that).
   - There is also "Redact whole page" in the page's context menu.
   - "Find text to redact…" stays as an optional helper that adds boxes. It is never required.
4. **A side panel**, "Redactions (3)", lists the boxes by page, with a prominent **Redact 3 areas…** button. It is always visible while the tool is active and never sits in a scrolled-away toolbar.
5. **The confirmation is in plain words:**
   > **Permanently remove everything inside 3 boxes?**
   > Text, pictures, drawings, comments and form fields inside the boxes are deleted from the file, not just covered. The result is saved as a new file; your original is not changed.
   > ☑ Also remove hidden information (author, document properties, attachments, bookmarks…) — *Recommended* [Details]
   > [Redact and save as…] [Cancel]

   The nine checkboxes move under **Details**, renamed in plain words. All of them are on by default, as today.
6. **After applying, the redacted copy opens in the window,** with a banner: "You are viewing the redacted copy 'contract redacted.pdf'. 3 areas removed and checked." The original is closed. "Open original" is in the banner, so the user cannot keep editing the unredacted file by mistake.
7. **Saving with pending boxes** asks "3 areas are marked but not redacted yet." The choices are [Redact now…], [Save marks only] and [Cancel]. This replaces today's status line, "marks are still marks".

### 2.4 Engine changes

Each change is small, comes with tests, and is reviewed independently as before.

**R1. Every glyph the box touches goes.** Today a glyph is removed only if its central half meets the box (`region.rs` `removes_glyph`, line 490). A glyph cut by the edge therefore stays in the file, under the black box, where its text can still be extracted.

- New rule for area marks: a glyph goes if its box overlaps the area by any positive area. A tolerance of 0.5 pt keeps a box that only grazes the line above from taking its descenders.
- Search-helper boxes keep today's tight rule.
- The preview tints exactly the glyphs this rule takes, so nothing surprises the user.

**R2. Blanked pixels match the box and round outward.** Image blanking today writes sample value 0. That is black in DeviceGray and RGB, but not in CMYK or Indexed images.

- New behaviour: write the box colour converted into the image's colour space. Indexed images use the nearest palette entry, or are re-encoded as RGB when none is close.
- Any pixel that touches the box is blanked, not only pixels whose centre lies inside it.

**R3. Shared pictures and Forms are copied, not refused.** Today, an image or Form drawn on another page without a box refuses ("this shared image also appears on page(s) … Mark it there too").

- New behaviour: clone the object for the boxed placements (copy-on-write, which the engine already does for edits, milestone M2) and redact the clone. Unboxed placements keep the original.
- The refusal and its dialog line disappear.

**R4. Content that cannot be cut exactly is flattened, with consent.** Function and mesh shadings, patterns or soft-mask groups that draw text or images, and Type 3 glyph procedures under a box refuse today.

- New behaviour: offer to flatten only the affected page, by rendering it with the boxes burnt in and replacing its content with that image. The engine OCR used by the scanner could make it searchable again later.
- Wording: "Page 4 contains artwork that can't be cut precisely. Redact page 4 by turning it into an image? Its text will no longer be selectable." [Flatten page 4] [Cancel]
- The result is that nothing refuses outright.

**R5. A raw check for area boxes.** Today the raw-string check runs only for search marks.

- New behaviour: before applying, collect the text the view reads inside each box (runs of 4 or more characters). After applying, require that none of it remains anywhere in the copy: shown strings, alternate text, annotation and field values, metadata.
- This gives box redaction the same proof that search redaction has.

**R6. A clear region API for the agent and tests.** Add `redact-apply {regions: [{page, rect}], sanitize, flatten?: [page]}`.

- It builds marks in memory, so an agent can redact without creating annotations first. `apply` already takes `&[Mark]`.
- The existing ops stay, so current scripts keep working.

**Kept as they are:**

- Annotations a box touches go whole. The engine design doc already decided this: a comment's text cannot be cut at the box edge. The preview tint now shows it.
- Widgets go with their fields.
- Alternate text and metadata are stripped as today.
- Signed files still refuse, but with an offer: "Redacting removes the signatures. Make an unsigned redacted copy?" (owner decision D5).
- The 12 MiB limit stays until a separate performance item. The dialog states it in MB, not MiB.

**Removed or changed in the UI:**

- The stale "not removed by a rect mark" warning goes.
- "Mark every match" moves into "Find text to redact…".
- Redaction marks no longer appear in the Comments list.
- Success no longer shows in the red error bar.

### 2.5 Tests and proof

- Engine tests per change, on the existing fixtures plus new ones:
  - a glyph cut by the box edge;
  - CMYK and Indexed images half under a box;
  - an image shared across pages with a box on only one;
  - a mesh shading under a box (flatten path);
  - text in a widget, a FreeText, a bitmap picture and a Form, all inside one box.
- `fixtures/redact.pdf` acceptance: **one box drawn over the whole of page 1 removes every occurrence of the secret on it**, and the copy passes the text, pixel and new raw checks.
  - Proof uses MuPDF as the black-box oracle (`oracle/compare-app-redact.mjs`): the secret appears in no object of the copy.
- A drive script for the new flow: drag, confirm, the copy opens, the original is closed, and Undo is unchanged.

### 2.6 Prototype

**Low risk, can start on approval (Phase 0), in the current Electron app:**

- the UI flow in §2.3 (black-box preview, side panel, plain dialog, opening the copy);
- R1;
- R6.

R2 to R5 follow in the same phase, with independent review of the engine changes. I have not started the prototype because R1 changes what gets removed, and the owner should approve that rule, and the flatten fallback (R4), first.

---

## 3. UX audit: every feature as a first-time user

I walked through each feature in the current desktop build. Code references are `R` = `src/renderer/renderer.ts`, `H` = `src/renderer/index.html` and `A` = `mobile/src/app.ts`. Severity: **high** blocks or misleads, **med** slows the user down or confuses them, **low** is polish.

### 3.1 Cross-cutting problems

| # | Finding | Evidence | Sev |
|---|---|---|---|
| X1 | **Tools are hidden in a row that scrolls sideways with no visible scrollbar.** At 1280 px, Annotate shows 4 of 12 tools; even at 1900 px Draw ink, Stamp and Sign are off-screen. Organize shows 7 of 12. Redact hides its own Apply button. | `02-annotate-1280.png`, `03-annotate-1900.png`, `04-organize.png`; `.ctx` `overflow-x:auto` at `styles.css:280` | high |
| X2 | **Six modes gate the tools** (Read, Annotate, Organize pages, Fill forms, Edit text, Redact). The user has to know which mode holds a tool. Sign lives in two modes. Choosing Annotate ▸ Sign switches to Fill forms. | H:55-161, R:10090 | high |
| X3 | **The inspector jumps to unrelated tabs per mode.** Organize opens "Text", with an unstyled "Add text" form. Redact opens "Comments". Read opens "Links", with "Draw link" (an editing tool) in Read mode. | `04-organize.png`, `01-read.png`; R:574-581 | med |
| X4 | **First launch opens a built-in sample, "hello.pdf"** (a page saying "PDF4All"). Saving it fails ("This is the built-in fixture…"). There is no start screen with Open, New, Scan or recent files; the mobile app has one. | R:905, R:2217 | high |
| X5 | **Two Previous/Next pairs sit side by side in the top row**, one for search and one for pages. | `01-read.png` | med |
| X6 | **Armed tools have no visible state and no way out.** Note, text box, ink, link and baseline tools stay armed with no indicator other than a pressed button. Escape only disarms Sign. In Edit text, clicking empty page space adds text; in Fill forms it adds a text box. Stray clicks change the document. | R:10014-10135, R:10544, R:5013 | high |
| X7 | **Jargon and internals reach the user.** Examples: "show-text runs", "Identity-H", "copy-on-write", "incremental update", "DocMDP", "P=1", "MiB", "byte limit 125829120", "XFA", "refused". Raw op prefixes too: "sign-place: …", "fill-text: …", "sign-prepare: incremental save changed source bytes". | H:521-601 (help matrix), R:2268, R:4530-4824, R:11117 | high |
| X8 | **Success messages are drawn in the red error bar**, for example after Apply redactions and after recovery. | R:10856, R:1414 | med |
| X9 | **Unfinished layout.** Hint text squeezed into a narrow column makes the Fill forms toolbar 170 px tall. Native controls are unstyled (fieldset, select). The "Outline objects" checkbox is oversized. The status bar holding the zoom controls was off-screen in our nav.pdf captures and on-screen on form.pdf. | `06-fill-forms.png`, `05-edit-text.png`, `01-read.png` | med |
| X10 | **Desktop and mobile disagree.** They use different words (Annotate vs Highlight, Draw and Note; Fill forms vs Fill), different save models (desktop saves manually; mobile autosaves and still offers "Save on this device") and different visual design. The mobile design is the better one. | `13-mobile-editor.jpg` | med |
| X11 | **There are no settings and no View menu**, and About is Electron's default panel. | M:488 | low |
| X12 | A drag that starts just outside the page selects the UI's own labels and tabs as text. | redaction capture | low |

### 3.2 Proposed interaction model for both platforms

This model replaces modes with an always-visible tool strip and context bars. Both apps get it, and it is the specification for the Rust UI.

- **Start screen:** Open, New blank, New from images or scan, recent files, drop zone. On mobile it is today's library.
- **Top bar:**
  - file name and save state ("Saved", "Edited", "Saving…");
  - Open, Save, Share or Export ▾, Undo, Redo;
  - one search field, with its own ‹ › and "2 of 7";
  - page "3 / 12";
  - zoom ▾.
- **Tool strip:** one visible button each, never scrolled, with icons and labels. On mobile it is today's bottom bar, extended.
  - **Select** (default): select text or objects. Selecting text shows a small bar with Highlight, Underline, Strike, Copy, Comment and Redact.
  - **Highlight**: drag across text.
  - **Draw**: pen, line, rectangle and ellipse, sized by dragging.
  - **Note**: click to place and type at once.
  - **Text**: click to add text, or click existing text to edit it.
  - **Fill & Sign**.
  - **Redact**.
  - **Pages**: opens a page grid.
- **Escape always returns to Select.** An active tool shows a chip ("Drawing · Esc to stop").
- **The right panel shows what is relevant**: properties of the selection, the comment list, fields, or the redaction list. It never shows an unrelated form.
- **Messages in plain words**, from one catalogue file. Each error says what happened and what to do next. Internal operation names are never shown. Success never appears in red.

### 3.3 Feature by feature

**Open** (`07-password.png`)
- *Seen:*
  - The password prompt appears above the *previous* document's page.
  - The banner says "encrypted, password required. Read-only.", which is wrong: once unlocked the file is editable.
  - Files over the limit report a raw byte count. Signed files open read-only, with the reason only in a tooltip.
  - Repaired files say "Detected: …, repaired." without saying whether anything was lost.
- *Fix:*
  - A clean password screen: "'x.pdf' is password-protected" with a field and Open.
  - Limits in MB.
  - For signed files: "This file is signed. Editing it would break the signature, so it opens for reading. [Make an editable copy]".
  - For repaired files: "This file was damaged and has been repaired. Check it before saving."

**Read and navigate** (`01-read.png`)
- *Seen:*
  - Zoom lives only in the bottom status bar.
  - The "Thumbnails" button collapses the whole sidebar.
  - Thumbnails cannot be dragged. Links can be followed only in Read mode.
  - The mode hint reads "Reading draws nothing on the page…".
- *Fix:*
  - Zoom moves to the top bar, with pinch and ⌘ plus the scroll wheel.
  - A sidebar toggle with an icon, plus Pages and Outline tabs ("Outline", not "Bookmarks", or both words).
  - Links work with the Select tool; ⌘-click when a tool is active.
  - Remove the hint.

**Search** (`08-highlight-match.png`)
- *Seen:* it works ("Match 1 of 1"), but its Previous and Next sit beside the page Previous and Next.
- *Fix:* a single search field, with a matches popover listing the hits and their context. Its arrows are grouped inside the field.

**Highlight, underline, strike** (`08-highlight-match.png`)
- *Seen:* **the only way to highlight is to search first and then press "Highlight match".** You cannot select text and highlight it. With no match, the button silently does nothing.
- *Fix:* select text, then Highlight, the same as on mobile. Searching gains a "Highlight all matches" action.

**Draw** (`03-annotate-1900.png`)
- *Seen:*
  - "Draw ink" is off-screen.
  - Each stroke disarms the tool.
  - Rectangle, circle and line drop at a fixed size over the search match or the page centre, with no drag-to-draw.
  - Pressing the button from the keyboard draws a "stand-in zigzag".
- *Fix:*
  - The Draw tool stays on until Esc.
  - Shapes are drawn by dragging.
  - Colour and width are in the context bar.
  - Remove the zigzag. Keyboard users get a "place shape" with arrow-key sizing.

**Note and comments** (`09-note-empty.png`)
- *Seen:*
  - Add note, then click: an **empty note** is created with no editor. Its text can only be typed through Comments ▸ Edit.
  - "Add text box" opens a dialog titled "Add text box" with the field "Note text".
  - The comment list shows raw types ("StrikeOut", "Highlight: (no comment)").
- *Fix:*
  - Placing a note opens an inline editor at once. An empty note is discarded when the editor closes, as on mobile.
  - The text box is typed in place on the page.
  - The list shows "Strikethrough on page 2: 'quarterly'".

**Edit text** (`05-edit-text.png`)
- *Seen:*
  - Refusal reasons in internal terms, such as "Quote operators are not editable", "Subset font" and "Shared placement".
  - The Text inspector mixes "Pick baseline point", PostScript font names ("Helvetica-BoldOblique") and "Find and replace on this page".
- *Fix:*
  - Click existing text to edit it.
  - When editing is not possible, say why and offer what can be done: "This text can't be edited in place (the font is incomplete). [Cover and retype]".
  - Adding text is a click with the Text tool, not a baseline picker.
  - Fonts shown by family name (Helvetica, Bold, Italic).

**Fill forms** (`06-fill-forms.png`)
- *Seen:*
  - The toolbar is 170 px tall from a squeezed hint.
  - Fields are listed in the inspector.
  - **The first Save after filling silently writes "name (filled).pdf"** instead of the file the user opened.
  - Clicking empty page space adds a text box.
  - On mobile, Fill opens a per-page dialog instead of letting you tap the field.
- *Fix:*
  - Click a field on the page to type in it, and Tab moves to the next one.
  - "Add text" anywhere only with the Text tool.
  - On the first save of a filled form, ask once: "Save over 'form.pdf' or as a new file?"
  - On mobile, tap the field directly.

**Sign** (`10-sign-certificate.png`)
- *Seen:* **four entry points**: Annotate ▸ Sign…, Fill forms ▸ Sign, Fill forms ▸ "Draw signature…" and "Sign with certificate…". The certificate dialog:
  - asks for X, Y, Width and Height in points;
  - has a confirm button labelled **"Go"**, which does nothing silently when no certificate is chosen;
  - leaves the user looking at the *unsigned* file after signing.
  - Verify reports "Certified (P=n)" and "first N of M bytes".
- *Fix:* one **Sign** button, then a sheet with two clear choices:
  - **"Add my signature"**: a handwritten picture, with no proof of identity. Draw or pick an image, then drag it onto the page.
  - **"Sign digitally with a certificate"**: verifiable. Choose a certificate, then drag the signature box onto the page instead of typing coordinates. The button reads **Sign and save as…**, and the signed copy opens afterwards.

  Verify speaks plainly: "Signed by Jane Doe on 18 Oct 2026. The document has not changed since it was signed. The signer's certificate is not in your trusted list. [Trust…]".

**Pages** (`04-organize.png`)
- *Seen:*
  - Every action works on the current page only. There is no multi-select and no dragging.
  - Insert asks "After page", where **0 means before the first page**.
  - Merge exists only as "Insert pages…".
  - A cancelled extract shows an empty error bar.
  - The page tools share a row with Watermark, Header/footer ("Bates start"), Page labels and Security.
- *Fix:*
  - A **page grid**: select one or many pages; drag to reorder; rotate, delete or extract the selection; drop a PDF or image between pages to insert it.
  - Insert offers "Before page 1", "After the current page" and "At the end".
  - "Combine files…" goes on the start screen.
  - Watermark, header/footer and labels move to a "Page setup" menu.
  - Security moves to File ▸ Protect…

**Scan** (mobile; code review, as the browser preview cannot reach the camera)
- *Seen:*
  - Three "Save as" choices.
  - A required "I reviewed the recognized text on every page" checkbox.
  - Rotate or crop throws away the OCR without warning.
  - The size limits disagree: the dialog says 30 MB, `scan-compose.ts` says 50 MB.
  - The AI dialog asks for a "Vision-capable model ID".
- *Fix:*
  - Default to "Searchable PDF" with on-device OCR. The image-only and AI rebuild options go under "More options".
  - Replace the required checkbox with a review screen per page and a "Looks good" step.
  - Rotating and cropping keep the OCR, or ask first.
  - Use one limit.
  - The AI provider picker offers a recommended model per provider and hides the free-text model field under Advanced.

**Share and export**
- *Seen:*
  - Desktop has no Share.
  - Export offers only the current page as PNG or JPEG, or all text as .txt.
  - On mobile, Share always names the file "…-edited.pdf", even when nothing was edited.
- *Fix:*
  - Desktop Share uses the macOS share sheet (platform glue) and "Email…", "Copy file".
  - Export adds all pages as images, a flattened PDF, a PDF without comments, and a selected page range.
  - Mobile keeps the original name when the file is unchanged.

**Save, Save As and recovery**
- *Seen:*
  - **"Save As…" is styled as the main button but saves a copy**: the window stays on the original, still marked edited.
  - There is no autosave to disk. A crash replays edits in memory only.
  - Mobile autosaves but still has "Save on this device".
  - Some failure messages: "…comes in a later unit", "The saved bytes did not reopen as the same document".
- *Fix:*
  - Save is the main action.
  - Save As… rebinds the window to the new file, as on every other platform. "Save a copy…" is a separate item.
  - Edits are journaled to disk every few seconds, and the next launch offers "Restore unsaved changes to x.pdf?".
  - Remove mobile's "Save on this device".
  - Every failure message ends with what to do next.

**Redact**: see §2.

---

## 4. Phased order of work

Costs are in engineer-weeks for one experienced engineer, or the same work in agent lanes under the house rules (at most two builds at once, independent review). The ranges are honest uncertainty, not padding.

| Phase | Work | Depends on | Cost | Output |
|---|---|---|---|---|
| **0. Redaction redesign** (in the current app) | §2.3 UI flow; R1, R6, then R2 to R5; tests; MuPDF oracle proof | owner approval of R1 and R4 | 3 to 4 weeks | "Draw a box" redaction shipped on Electron and mobile web. The engine rules carry over to the Rust app unchanged. |
| **1. UX quick wins** (current app, kept small because this UI will be replaced) | X1 (wrap the tool rows), X4 (start screen), X6 (Esc and an armed-tool chip), X7 and X8 (message catalogue, no red success), highlight-from-selection, note opens its editor, Save As rebinds, filled-form save prompt, sign button labels | — | 2 to 3 weeks | The worst confusions fixed now. The message catalogue is reused by the Rust app. |
| **2. `tandem-sign` crate** | RustCrypto port of `src/sign/*`; ESS cert-v2; differential tests against OpenSSL; WASM build loaded by Electron's main process | — (can run alongside 0 and 1) | 3 to 4 weeks | Signing crypto in Rust, proven by the existing sign and verify tests |
| **3. Contracts and spike** | `tandem-protocol` crate (ops and `STATE` as serde types; TypeScript types generated from them while Electron lives); freeze the drive protocol; two-week spike of Tauri+Leptos vs Dioxus on the reader view (open, render, scroll, zoom, search) measuring build memory, render latency and package size | 0, 1 | 3 to 4 weeks | **Decision gate: toolkit choice, signed off by the owner** |
| **4. Headless core and desktop app** | `tandem-app` (state, commands, undo, journal, autosave, save/print, drive/MCP); Leptos UI built to the §3.2 model; engine native; port the app tests and drive scripts; macOS package and notarization | 3 | 12 to 18 weeks | Rust desktop app at feature parity, with the new UX |
| **5. Mobile on Tauri** | Same core and UI on iOS and Android; port the scanner, OCR, ChatGPT and share plugins to the Tauri plugin API; library store; device testing | 4 | 5 to 7 weeks | Capacitor and TypeScript removed from mobile |
| **6. AI in Rust** | `tandem-ai` for scan rebuild and providers; OAuth PKCE in Rust, tokens kept in the native keychain plugins | 5 (can overlap) | 1 to 2 weeks | No TypeScript left |
| **7. Retire Electron** | Delete `src/`, Electron and the TS build; CI packages for Windows and Linux (needs a push); update docs and NOTICE | 4, 5 | 2 to 3 weeks | Everything in Rust |

The total is about 31 to 45 engineer-weeks. Phases 0, 1 and 2 are independent of the toolkit choice and useful on their own; they can start as soon as they are approved. Phase 4 is the large rewrite and is not started without the Phase 3 decision.

What is not in this plan: the lawyer's clean-room review, the remaining owner items from `docs/mit-clean-room-plan.md`, and the engine performance follow-ups. They stay open as listed in `engine/STATUS.md`.

---

## 5. Decisions for the owner

| # | Decision | Recommendation |
|---|---|---|
| D1 | The meaning of "everything in Rust" in §1.2: all logic in Rust, with platform glue (system web view, Swift and Kotlin scanner, keychain and share) kept thin | Accept |
| D2 | UI toolkit | Tauri 2 + Leptos, with Dioxus as the alternative; confirmed after the Phase 3 spike |
| D3 | RSA implementation: pure-Rust `rsa` (accepting the Marvin advisory for local signing) or `aws-lc-rs` (constant-time, contains C) | `rsa` |
| D4 | Redaction glyph rule R1: anything the box touches goes, with a 0.5 pt tolerance | Accept |
| D5 | Redaction flatten fallback R4, and redacting signed files into an unsigned copy | Accept both, with the confirmations in §2.4 |
| D6 | After redacting or signing, open the new copy in place of the original | Accept |
| D7 | Phase order: start 0, 1 and 2 now; do not start 4 until the spike decision | Accept |
| D8 | Disk autosave and recovery journal (new behaviour: unsaved edits are written to the app's data folder, never next to the user's file) | Accept |
