# Tandem PDF: everything in Rust, and a UX pass

Status: **approved** (October 18, 2026; `main` at 7ddf541). The owner accepted every decision, D1 to D8, as recommended ("accept all"). Phases 0, 1 and 2 were built overnight on Oct 18–19 on local branches (not merged to `main`); see `docs/overnight-report.md` and "Decisions made overnight" at the end. Phase 4 was then authorised directly (Tauri 2 + Leptos) and built for macOS on Oct 19 on the local branch `p4-rust-mac`; see `docs/rust-mac-app-report.md` and "Decisions made building the Mac app" at the end. Screenshots are in `docs/ux-audit/`. They were taken from the current desktop build at 1280×820 (one at 1900×820) on the repository's synthetic fixtures, and from the mobile web build at 375×812.

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

---

## Decisions made overnight (Oct 18–19, 2026)

Made while the owner slept, under the instruction "pick what is best for users and record it". Each one is easy to revisit.

- **Branch layout.** Each phase works on its own local branch and worktree under `tpdf-wt/`: `p0-redact-engine` (R1–R6), `p0-redact-ui` (the §2.3 flow in Electron), `p1-ux-quick-wins`, `p2-tandem-sign`. The redaction UI merges the engine branch in when it is ready. Nothing is merged to `main` or pushed.
- **Where new Rust crates live.** `crates/<name>/` at the repository root, starting with `crates/tandem-sign`. The engine stays in `engine/`. Phase 3/4 crates (`tandem-protocol`, `tandem-app`) go next to it.
- **Gates cover the new crates.** `tpdf-wt/tools/gates.sh` now runs fmt, clippy (stable and 1.92) and release tests for every `crates/*/Cargo.toml`. The previous script is kept as `gates.sh.bak-2026-10-18`.
- **Phase 0 ships on desktop first.** The mobile web app has no redaction today; adding the box tool there is a stretch goal for tonight, otherwise it follows in Phase 5 with the shared Rust UI.
- **New engine op for the preview tint.** `redact-inspect {regions}` returns exactly what applying would remove (glyphs, picture areas, drawings, comments, fields), using the same rules as apply, so the red tint never disagrees with the result. (`redact-preview` already exists and still feeds the dialog's table.)
- **Independent reviews use Claude reviewer agents.** The GPT-6 Astra reviewer (`pi`) is not logged in on this Mac, and logging in needs the owner's credentials, so each phase is reviewed by a separate Claude agent that did not write the code.
- **Which marks use the R1 "touches" rule.** Every redaction mark that is not tagged as a search mark uses the area rule, per quad when the mark has quads. That includes marks made by other editors and pasted search marks that lost their tag. Removing slightly too much is the safe mistake; leaving a half-covered glyph in the file is not.
- **The 0.5 pt tolerance applies on every side that is not a page edge**, as D4 says. A sliver on the left or right side is visible on screen anyway. A related older gap is fixed too: glyphs with zero width (hostile fonts) get a minimum width for the removal test, so a box over their ink cannot miss them.
- **The preview is bounded.** `redact-inspect` caps how much it reports (about 10,000 items) and returns `truncated` past that, so a hostile page cannot exhaust memory.
- **Agent scripts keep their behaviour.** The scripted (`--drive`) `redact-apply` still leaves the original open, because existing scripts and the MCP agent rely on that. Only the on-screen flow (and the new `redact-confirm` step) opens the redacted copy.
- **Closing a window with boxes not yet redacted** asks [Redact now…] [Close without redacting] [Cancel]. "Save marks only" is offered when saving, not when closing.
- **Signed files keep boxes in memory.** A signed file cannot take new annotations, so boxes drawn on it are held by the app and sent as regions. Tonight they can be added, removed and redacted, but not moved, resized or undone.
- **The size limit reads "12.5 MB".** That is 12 MiB rounded down, so the stated size always works.
- **The redacted copy can never replace the original.** "Redact and save as…" refuses the original's own file name, because the dialog promises the original is unchanged.
- **Other unsaved edits are not lost on redaction.** If the document has unsaved changes besides the boxes, the user is asked first: "You have other unsaved changes. They will be in the redacted copy, but not in your original." [Save them to the original first] [Continue] [Cancel].
- **Find text to redact says what it cannot see**: text inside pictures, form fields and comments. Drawing a box is the reliable way.
- **Phase 1: agents see raw messages, people see plain ones.** The drive state's `error` keeps the exact text so scripts and the MCP agent don't break; a new `message {kind, text}` field holds what the person sees.
- **Phase 1: placing tools stay armed until Esc**, like the Draw tool in §3.3. Escape removes one layer per press (open editor, then selection, then the tool), so a selection is never lost together with the tool.
- **Phase 1: an empty note is never created**, rather than created and deleted.
- **Phase 1: "Combine files" is not on the start screen yet**, because the app has no such feature, only Insert pages. The start screen has Open, New blank, New from images and recent files. `--sample` still opens the built-in sample for demos and tests.
- **Phase 1: after signing, the signed copy opens in the app.** Scripted runs stay on the original, so existing signing scripts are unchanged.
- **Phase 1: filled forms.** The first save after filling asks "Save over 'form.pdf' or as a new file?". A script that does not answer gets the old behaviour (a new file).
- **Phase 1: recovery notices are amber, not green**: something was stopped, even though the document was restored.
- **Phase 2: the stable RustCrypto line.** `tandem-sign` uses `rsa` 0.9.10, `der` 0.7, `p256`/`p384` 0.13 and `pkcs12` 0.1.0, the newest line where every piece is a stable release (`rsa` 0.10 and `pkcs12` 0.2 are release candidates). RUSTSEC-2023-0071 is recorded as an accepted risk under D3.
- **Phase 2: certificates are embedded byte for byte.** A small DER writer is used instead of re-encoding certificates through `cms`/`x509-cert`, because re-encoding a certificate that is not perfectly DER would break its issuer's signature.
- **Phase 2: hostile-file limits.** Key-file KDF iterations above 5,000,000, RSA keys above 16,384 bits and nesting deeper than 16 are refused. The old code had no limits.
- **Phase 2: P-256 and P-384 keys can now sign**; P-521 is verify-only. Unknown curves report "unsupported" instead of "invalid signature", so they are not mistaken for tampering.
- **Phase 2: dates in verification sentences are UTC**, because the module does not know the time zone. The UI can localise later.
- **Phase 2: the plain verification sentence is shown as the row's tooltip and to screen readers.** It is not visible row text yet; the Verify redesign belongs to the UI phases.
- **Mobile redaction is being added tonight** on its own branch (`p0-redact-mobile`), in the mobile app's own style. Mobile boxes are kept in memory and sent as regions, not saved as annotations, which also makes them work on signed files. The redacted copy is saved as a new library document named "… redacted.pdf" and opened; the original is never overwritten.
- **Phase 1: Return never overwrites a filled form.** In the "Save over or as a new file?" question, the default button is "Save as New File…".
- **Phase 1: Esc keeps a note that has text.** Only an empty note is discarded. Save, Save As and closing first commit any open note or text editor, so typed text is never lost.
- **Phase 1: X12 is limited to the app's chrome** (toolbars, tabs, side labels). Notes, the Verify table, help, banners and link addresses stay selectable and copyable.
- **Phase 1: plain-language cleanup only touches the app's internal text**, never file names, quoted page text or signer names.
- **Owner answers (Oct 19, during the night):**
  - Signatures switch to PAdES B-B: SubFilter `ETSI.CAdES.detached`, with the signing time in `/M` instead of a signed attribute. Done on the signing branch, with the pinned tests updated.
  - `cms` 0.2.3 ships no licence file, so the exact LICENSE texts are copied from the RustCrypto/formats repository into `licenses/cms-0.2.3`.
  - Claude reviewer agents are fine for the independent reviews; no Astra pass is needed.
- **Phase 2: outdated certificates are not trusted.** A chain link signed with SHA-1, or with an RSA key under 2048 bits, makes the chain untrusted, with the plain reason "uses outdated security". The signature itself can still be reported intact.
- **Phase 2: opening a certificate file has a work limit**: about 10 million key-derivation rounds per file, and at most 2 million per step. A hostile file cannot freeze the app.
- **Mobile redaction choices.** The bottom tool bar stays visible in Redact. A new box is selected at once, with its ×, so a mistake is one tap to undo. Undo and Redo act on boxes while Redact is on. Boxes stay drawn after Done, and the Redact button shows a count badge. The flatten button reads "Turn page N into an image" instead of "Flatten", to avoid jargon.
- **One set of plain names for the nine "hidden information" options**, shared by desktop and mobile: author, title and other document properties; extra document details saved by other apps; attached files; bookmarks; other comments and notes, on every page; web and file addresses in links; everything typed into form fields; layer names; descriptions written for screen readers.
- **R4 flatten resolution.** A flattened page is drawn at 200 dpi, capped at about 16.7 megapixels, with the boxes burnt in before the image is made. When a box will need this, the panel says so before the question appears.
- **R5 fails safe, and says what to do.** With "Also remove hidden information" switched off, text from inside a box may still sit in comments, form values or document properties; then nothing is saved, and the message says to switch that option on and try again. A word that is boxed in one place but also printed elsewhere does not block redaction.
- **D5 details.** A redacted copy of a signed file drops the signed signature fields and the document's signing permissions. Empty (unsigned) signature fields stay. The open signed document is not changed.
- **Two engine timing tests no longer depend on a quiet machine.** The deep page-tree test is now measured against a smaller tree (it must stay within 40×), and the forms synthesis bound is raised from 10 s to 30 s; the bugs they guard against take minutes. The remaining wall-clock tests still flake when two gate runs share the machine; a shared load-tolerant timing helper is a follow-up.
- **Phase 2: reading certificate files and signing run in a worker thread**, so a slow certificate file never freezes the window. Private keys live only in that worker.
- **Phase 2: the `cms` licence texts** come from RustCrypto's own later commit (f76fafc), because the commit `cms` 0.2.3 was published from has no licence files. `SOURCE.txt` records where they came from and their hashes.
- **Phase 2: accepted signing purposes.** A signing certificate may be marked for any purpose, document signing, email protection, or the Adobe and Microsoft document-signing purposes, or carry no purpose at all. A certificate marked only for something else (for example web servers) is "not meant for signing", and its name is not confirmed.
- **The "Also remove hidden information" checkbox now tells the whole truth.** It also removes every other comment and form entry in the file, which people can see, so its label reads "Also remove hidden information, other comments and form entries (author, document properties, attachments, bookmarks…)". It stays on by default, as today. Same wording on desktop and mobile.
- **Sharing or saving with boxes not yet redacted asks first** (mobile, like desktop's Save): "N areas are marked but not redacted yet." [Redact now…] is the default. A black box on screen must never go out as the unredacted file.
- **Mobile scrolling in Redact:** one finger draws a box, two fingers scroll and zoom.
- **A whole-file check after every redaction.** The independent review found that copying a shared picture (R3) or flattening a page (R4) could leave the original object, secret included, elsewhere in the file when pages share resources, and every check still passed. Those bugs are being fixed. In addition, the copy is now scanned after writing: text from every content stream (pages, forms, patterns, glyph procedures, annotation appearances) and every picture is checked, and any leftover boxed content makes the redaction fail with nothing saved.
- **Phase 2: the signing date is the signer's claim.** Without a timestamp server, a PAdES B-B signature's date is only what the signer wrote, so the app says "Signed by "X", dated 18 Oct 2026" and uses the date from the signed revision only. A later edit cannot change it.
- **Phase 2: no code-signing certificates for documents.** Signing with a certificate not meant for documents shows a warning first ("…other apps may not trust the signature. Sign anyway?").
- **Phase 2: issuers with rules the app cannot check yet** (name constraints and similar) are reported as exactly that, never as "not in your trusted list".
- **One integration branch to try everything together.** `overnight-integration` merges Phase 1, the Phase 0 desktop and mobile redaction (with the engine) and Phase 2. Where the branches touched the same code, both behaviours were kept: Phase 1's layered Escape and message catalogue, with redaction registered as one more layer. The integration lives in the `tpdf-wt/p1-ux` worktree to save disk; `p1-ux-quick-wins` itself is unchanged at fa2e1b8.
- **Phase 2: the signing date is read from the signed bytes themselves.** The verifier reads the signature dictionary inside the signed range, so a later edit to the date is ignored and noted ("A later change edited the signature's date; the date shown is the one that was signed."). This works for old and new signatures alike and needed no engine change.
- **Phase 2: certificates in the user's trusted list are taken as they are.** Unsupported rules matter only on intermediate certificates.
- **Phase 2: name constraints are never silently ignored.** A trusted certificate, or an intermediate, that limits which names it may vouch for gets "uses rules Tandem can't check yet, so the name is not confirmed". Trust is never widened quietly.
- **R3 and R4 redact the original in place.** The third review showed that keeping originals whenever they might be drawn elsewhere fails open: unused patterns, soft masks, inherited resources and invisible placements all kept the secret. The rule is now inverted. The original is always redacted or emptied, and only placements a walk proves are painted, unboxed and visible on another page get a private untouched copy. Any uncertainty removes too much, never too little. If this cannot be finished safely, the fallback is the old refusal ("shared content") for shared objects. Leaving content behind is never acceptable.
- **Mobile: copying page text with boxes pending shows a note** ("Text under your boxes is still in the file until you redact."). It does not ask the full question, because copying to the clipboard is something the user does deliberately.
- **When more than the boxes is removed, the user is told.** Under the in-place model, a picture or drawing shared with other parts of the document (through patterns, hidden layers, comments, or a shared form drawn on several pages) loses the boxed content everywhere. The engine reports `overRedacted`, and the banner says "Some content outside your boxes was also removed, because it is shared with other parts of the document. Check the copy." A later refinement can give each visible page its own copy with only its own boxes applied.
- **Reader-extended files** (Adobe usage rights) count as signed for redaction. Their redacted copy drops those rights, after the same question.
- **`p0-redact-ui` stays at its reviewed head (15dc0e5).** The final engine merge and the small UI follow-ups (the "more was removed" banner line, the plain Verify date) live on `overnight-integration`, which is the branch to try.
- **No untouched copies at all (supersedes the entry above).** The fourth review showed that proving another placement is "visible, so keep it untouched" breaks in many ways: partial clips, transparency, soft masks, even-odd clips, content covered by later drawing. So the engine no longer keeps untouched copies. Every object a box changes is redacted in place, everywhere it is used, and `overRedacted` tells the user when that affected anything outside their boxes. Image masks are redacted in place too. A pixel check makes sure no reachable picture under a box kept its original data.
- **Signing a signed file that has pending boxes warns about losing them.** Boxes on a signed file live only in memory, so the signed copy cannot carry them. The question says so before signing, instead of claiming the copy keeps them.
- **Extract pages asks about pending boxes too**, like every other way out of the app.
- **Adobe Reader usage rights get their own wording.** When the only "signature" is Adobe Reader's usage rights, the question says "This file has special permissions from Adobe Reader. Redacting removes them. Make a redacted copy without them?"
- **On `fixtures/redact.pdf`, one box over page 1 now also removes the shared form's text from page 2.** That is the in-place rule at work, and the banner says so.
- **Scripts count as hidden information.** With "Also remove hidden information" on, JavaScript that runs when the file opens, or from links, is removed too, and the final text check also reads scripts. A secret copied into a script would otherwise survive a box.
- **Scripts are removed with the "links" option**, not a tenth option. Its label reads "Web and file addresses in links, and scripts that run when the file opens".
- **"Too complex to check" is its own message.** When the final check cannot finish within its limits, nothing is saved, and the message suggests fewer pages at a time or turning pages into images.
- **A comment a box touches is removed from every page that lists it**, because it is one object.
- **More hidden places are checked and cleared.** The final text check now reads layer names, page-label prefixes, named destinations, the document language and attachment names. "Layers" renames every layer in the file, "Bookmarks" renames named destinations (and updates the links that use them), and "Author, title and other properties" removes the language and page-label prefixes when they quote boxed text. Anything left that cannot be cleared makes the redaction fail with nothing saved.
- **Comment authors and subjects** that quote boxed text count as hidden information.
- **Owner answer (Oct 19, morning):** over-redaction of shared objects is fine as it is. Boxing part of a shared picture or form removes that part wherever it is shown, and the banner says so afterwards.

## Decisions made building the Mac app (Oct 19, 2026)

Phase 4 for macOS was built on the local branch `p4-rust-mac` (report: `docs/rust-mac-app-report.md`). These were made while building it and fixing the UX walkthroughs, under the same instruction: pick what is best for users and record it. Later entries supersede earlier ones where they disagree (for example ⇧⌘G).

### Building the app

- **Workspace layout.** `crates/Cargo.toml` is a workspace of `tandem-protocol`, `tandem-app`, `tandem-ui`, `tandem-desktop` (shared target dir `crates/target`); `tandem-sign` stays outside it with its own lockfile and is used by path. The engine stays in `engine/`.
- **Tauri 2.11.5 pinned with its own 2.11-era runtime crates.** Cargo resolved `tauri-runtime 2.12.2` (made for a newer Tauri) under `tauri 2.11.5`, which does not compile; the lockfile pins `tauri-runtime 2.11.3`, `tauri-runtime-wry 2.11.4`, `tauri-macros/codegen/plugin 2.6.3`, `tauri-utils 2.9.3`. Leptos 0.8.22 (latest stable; 0.9 is a release candidate).
- **No trunk.** The UI is built by `crates/tandem-ui/build.sh` (`cargo build --target wasm32-unknown-unknown` + the wasm-bindgen 0.2.129 already used for the engine). One fewer tool to install and pin.
- **A faithful port, not a rewrite, of the renderer's logic.** The drive protocol compares exact STATE values, so `renderer.ts` (with `redact-ui.ts`, `recovery.ts`, `messages.ts`) is ported function by function into `tandem-app::session`, and `main.ts` into `tandem-app::host`. Where the renderer read its own page back, the port keeps a small virtual DOM (`tandem-app::dom`, keyed by the old element ids). The new Leptos UI draws from the core and drives these same functions.
- **The headless drive never opens a window.** `--smoke`/`--drive` without `--gui` runs the core's own harness and exits; Tauri is not started. Agents (MCP) and the test suites therefore need no window server.
- **One core thread owns the document.** The UI, menus, the pixel protocol and window events send it jobs; Tauri commands are async, so the window never waits on the engine.
- **Page pixels over `tandem://localhost/page/<n>?scale=…&gen=…`** as raw RGBA (`pixelWidth`×`pixelHeight` headers), painted with `putImageData`. The generation in the URL means a stale page is never shown.
- **The engine's panics are caught** (`EngineHost`), which then reports `reason: "worker"` like the WASM trap did, so the M9 recovery (restart + replay) keeps working natively.
- **One unsafe module.** `crates/tandem-desktop/src/macos.rs` holds the only native calls (PDFKit print panel, NSAlert questions, Open Recent, the window's proxy icon and edited dot, web-view snapshots). Everything else forbids `unsafe`.
- **Native questions are NSAlerts** (app-modal), not web dialogs: discard changes, pending redaction boxes, filled-form save, open link, certificate not for signing.
- **Screenshots come from the web view itself** (`WKWebView.takeSnapshot`, `--shot=<ms>:<file>`), because this session has no Screen Recording permission. 1920×1080 is reached on the 1280×800 screen with `--window-size=960x540 --webview-zoom=0.5` (the page lays out at 1920×1080 CSS px).
- **Separate data folder.** The Rust app keeps its recent files, trust list and drawn signature in `~/Library/Application Support/Tandem PDF`, not Electron's `pdf4all-p0`, so the two apps never step on each other while both exist.
- **Test infrastructure runs against either app.** `TANDEM_APP_BIN` (or `TPDF_DRIVE_APP`) points `scripts/drive.mjs`, `scripts/session.mjs` (so the MCP server) and the `--smoke` tests at the Rust binary (`tests/app-bin.mjs`). Tests that only make sense on Electron (CDP, a patched `build/`, Chromium files) skip there with `electronOnly`.
- **Gates.** `tpdf-wt/tools/gates.sh` builds the UI to WASM, runs fmt/clippy (stable and 1.92)/tests for every `crates/*` member (workspace members test in the dev profile, where the workspace optimises the engine), and adds `app-mac-build` + `app-mac` (every app-launching test against the release binary, two files at a time). Backup: `gates.sh.bak-2026-10-19`.
- **Menus.** The 47 Electron items with their shortcuts, plus a View menu (zoom, fit, sidebar, panel, find, page) and a Tools menu (the tool strip, ⌃⌘1–8) (X11). ⇧⌘G stays Insert ▸ Square, so Find Previous has no shortcut (⇧↩ in the search field does it).
- **Print** builds the PDF in the core and hands it to PDFKit's print operation (`runOperation`, app-modal): the macOS print panel with its own preview, page ranges, printers and "Save as PDF".
- **Licences.** Everything linked in is MIT/Apache/BSD/ISC/Zlib except two items Tauri requires: `option-ext` (MPL-2.0, unmodified) and the ICU4X crates (Unicode-3.0, permissive). MPL/BSL/CC0 crates elsewhere in the tree are compile-time only (proc-macros, build scripts). `scripts/crate-licenses.py` collects every shipped crate's licence text into the bundle and fails on anything else. Owner question recorded.
- **Bundle.** Identifier `com.shiro-legacy.tandem-pdf` (as Electron), PDF file association (Editor, Alternate rank), hardened runtime with an empty entitlements file (no network, camera or JIT), ad-hoc signed until a Developer ID exists.

### Lane 3 (Mac shell and window chrome)
- **⇧⌘G is Find Previous** (Mac standard); Insert ▸ Square moves to ⌥⇧⌘G. Supersedes the earlier "⇧⌘G stays Insert ▸ Square". Still 47 Electron items, shortcuts unique (menu.rs tests).
- **Help item is "Tandem PDF Help" with ⌘?** ("What Can Be Edited" read like jargon and the dialog now covers tools, signatures, redaction and keys). ⌘? is set natively (macos::help_key) since the accelerator parser cannot express a shifted "/"; the Help menu is registered as the app's help menu (search field).
- **File ▸ "New" without an ellipsis**: no dialog follows (no test pins the label).
- **"Sign Digitally…" and "Add My Signature"** in the menus: the chooser's and Help's words; the certificate dialog's title is "Sign digitally".
- **One name for password protection: "Security…"** in File and Share and "Security" as the dialog title (the catalogue's messages already say "File ▸ Security…"). The dialog's "Currently: …" line is shown in plain words (the core's text stays in STATE) and says the change is written on the next save.
- **Menu items grey out with the document**: Undo/Redo, Previous/Next Page at the ends, Find Next/Previous without a query, tools the core disables, everything document-only on the start screen.
- **Drops and Finder opens**: the first PDF opens; a note (closable toast, 9 s) names what was not opened; a veil while dragging says what a drop will do (refusal in warning colours for non-PDFs).
- **Start screen is top-aligned** (padding approximates centring) so a notice never moves the cards; notices without their own choices get a close button.
- **Dialog statuses that report a problem are drawn as errors (role=alert)** by wording, since the core does not mark the kind; summaries opt out.
- **Fields that do not apply are hidden, not greyed**: Position for a watermark, picture quality for a text export, running-number fields without {bates}, the visible/position controls when signing into a field.

### Lane 1 (window model and page area)
- **One zoom.** The window draws at `zoom_now`; `ui_pump` repaints the session until its `shown_zoom` (used to place hit targets and inline editors) equals it, so pixels, overlay and editors share one scale.
- **Pixels past the 4096 px edge** are rendered at the largest scale that fits and stretched by CSS; high zoom never fails.
- **Placing tools stay armed** (Phase 1): the Text tool keeps add-text armed, the pen re-arms after each stroke, Note stays armed; Esc goes back to Select.
- **Menu annotate items map to a neutral "annotate" tool**, not Highlight; dialogs that change the mode (Watermark, Security, Labels) never switch the strip, the mode goes back when they close.
- **One Undo per window action.** The engine has no multi-page op or step group, so the window keeps how many engine steps each action took and Undo/Redo take them together; the buttons are named from what the action said ("Undo “Rotated 3 pages”"), unnamed when it said nothing.
- **In the Pages tool the Page menu acts on the grid's selection**; moved pages stay selected at their new places.
- **Redaction stays available on signed files** (it makes an unsigned copy); Pages, comments, Text, Fill & Sign and Sign are off with the reason as their tooltip.
- **Pending redaction boxes are always black**, also with the tool off; the red tint belongs to the tool.
- **Compare speaks plainly** ("The text is the same. 0.4% of the page looks different.", "Only in a.pdf"); the drive keeps its own summary. A page the other file lacks shows its last page with a note.
- **Page targets are focusable**: Tab reaches fields, signatures, text boxes, links and objects (text runs only by VoiceOver); keys go to the focused target in the core; handles are drawn exactly where the core hit-tests them.
- **Certificate signature placement by dragging**: "Drag on the page…" sets the dialog aside (kept open in the core), the dragged box fills its page and position fields, a click places the 200×60 default, Esc returns to the dialog.

### Fix lane 2 (core logic and words)
- **Unsaved changes ask Save / Don't Save / Cancel** (Save the default, "Save…" for an Untitled file). Save cancels the open or close, the session saves, then the open goes on or the window may close (`Session::close_requested`); a cancelled or failed save keeps the document. A drive run never asks.
- **A failed open keeps the open document** (engine and window, Save target, edited mark); the failed file is not added to Recent Files; a missing file says it can't be found and is dropped from Recent Files.
- **New from images opens as an unsaved "Untitled.pdf"** in the window, like New; a smoke run keeps the save-first path.
- **Batch**: the window's text starts empty (no "{bates}" default); after a run only Done shows; the dialog summary is plain, the STATE announcement keeps its old words; Position stays Left/Right (centre needs engine support).
- **Range errors stay in the dialog** (Insert, Extract, Header/Footer, Print) and name the page and file ("Page 7 doesn't exist: hello.pdf has 1 page."); a drive keeps the old error text.
- **Export, Extract** show a success notice naming the file and its folder; a drive only announces.
- **Refused text edits** show the reason and a next step in the window; the editor stays open.
- **"From the top"** in the placement form is measured from the page's top edge, like the field designer.
- **Form XObjects are "Drawing"** to people; resource names (Fm1, PA1), "runs", "placement", font file names are not shown.
- **Links the app does not follow** say what they are ("Link to a file on this computer (not opened, for safety)") and a click shows a notice; the STATE announcement keeps "Blocked link (file:)".
- **Note editor**: Esc or ⌘Return finishes; an empty note is not kept (Phase 1 decision), said once.
- **Signatures**: the window's pad arms placement after Save when opened from "Add my signature"; rows say "Signature1, page 1"; the summary is plain ("The signature is broken: the document changed after signing"), STATE keeps its summary; Forget trusted certificates asks first (Cancel the default).
- **Security line** reads the file (password to open, what is withheld) and says what the next save does.
- **Verify rows show a verdict word with its own icon and colour** (Valid ✓ green, Not trusted ⚠ amber, Broken ✕ red, Not signed / Unknown grey) above the core's plain sentence, from `data.rowKinds`.
- **Certificate signature placement**: "Drag on the page…" appears under "Show the signature on the page" (hidden when not shown or when signing into a field); the typed-in points stay under "Where the signature goes".
- **styles.css overrides the overlay's inline fallback styles** for `.ctx-menu` and `.area-draft` with `!important` until viewport.rs drops them.

### Editable copy
- **The signed banner offers "Make an editable copy…" first, then "Verify signatures…"** (the copy is what someone who wants to edit is after); Dismiss stays last.
- **Signature fields stay, empty**: the copy keeps each signed field as an unsigned signature field (no /V, /Lock or appearance), so it can be signed again in the same place; the picture of the old signature goes, since a signature look without a signature would mislead. A signed field the in-place pass cannot reach (written directly in an array) is removed as redaction removes it.
- **Certification, usage rights and validation data go** (/Perms, /DSS, /SigFlags); form values, comments, pages and the field read-only flags the author set stay; the copy is one full revision with the file's security.
- **The save panel suggests "<name> (editable).pdf"** and refuses the signed original as the destination (asks for another name, as the redacted copy does); the copy opens in place of the original, which is not changed.
- **The result is an information notice that stays until dismissed** ("Opened an editable copy, X. The signatures were removed; the original is unchanged."), not a 5-second toast: it says what changed in the file.
- **Boxes drawn on the signed file (held in memory) carry over** to the copy as ordinary boxes (the copy is then unsaved).
- **Drive**: `{"cmd":"editable-copy","to":"…"}`; no `to` is a cancelled save panel; an unsigned file shows an error (STATE `editable-copy: the document is not signed`).

### Review fixes 1
- **An Untitled document with content counts as unsaved**: New from Images and a restored autosave open as Untitled with no edits, yet that file is the only copy, so closing the window or opening another file asks Save… / Don't Save / Cancel (and the window shows the edited mark); a cancelled Save… keeps the window. A blank New still closes without asking (nothing is lost). A drive never asks.
- **Undo groups count journal steps, not revisions**: a security change (not undoable) or an action that changed nothing adds no Undo entry, so a grid action's Undo takes back exactly its pages and the button names it.
- **A dialog that armed a placement keeps it**: after the signature pad's Save (from Add My Signature) the strip shows Fill & Sign and the signature is ready to place; the old tool is not put back over it.
- **"From the top" is on the shown page**: the placement form's four numbers are the box as the page is shown (rotation and a MediaBox/CropBox not at 0 included), converted back exactly; unchanged numbers send the placement's own box.
- **The security line says whether opening needs a password**: a file encrypted with an empty open password reads "anyone can open this file without a password, but it is encrypted", then what it withholds.
- **A refused open is not a new document**: the generation moves only when a document opened, so the open document's undo groups, clipboard and scans stay valid.
- **An encrypted signed file's editable copy keeps the password and opens locked**: the notice says so ("enter it to edit") and that the marked areas come back; they are added once the copy is unlocked.

### Round 2 lane C
- **One way to close a notice**: a "Dismiss" the core offers (signed banner, redaction banner) is drawn as the same × button as the notice bar's own, start screen and shell notes too; every one is named "Dismiss" for VoiceOver.
- **The start notice has its own slot** (about one notice tall, there with or without a notice), so a message never moves the cards; only a notice of three lines or more grows it.
- **Start cards at ≥1600 px are 1100 px wide** and the Open card says "Or drop one on this window.", so each card's line fits at 1920 instead of wrapping unevenly.
- **The shell's drop note comes after the open**: "Opened “a.pdf”. …" only once the window shows that file (or asks for its password); a refused open says why itself. It goes away when another document opens or the start screen returns. Several PDFs plus other files names both.
- **Save toasts offer "Show in Finder"** (the file the toast names: the last save panel's answer, else the document's file); the button does nothing when that file is gone.
- **Dialog status lines use the core's kind** (error red with role=alert, warning amber, success green, info plain); the word guess stays only for lines the core has not marked.
- **The live region speaks repeats**: two live regions take turns by the core's announce sequence number.
- **Security's line is the core's own sentence** (the window's "Currently:" rewrite is gone: the core's words are plain).
- **The 1920 walk's text clipping is a harness effect**: `--webview-zoom=0.5` makes WebKit floor every font under 18 px at the same size, so labels grow against CSS-pixel boxes. Fixed what large text breaks anyway (selects grow, list details get two lines, the note editor is wider, tabs keep a deliberate second row); at a real 1920×1080 display the panel tabs fit one row.
- **Radio buttons are drawn like the checkboxes** (ring, filled centre); `--selected` is defined as the green selection (primary-soft) so the outline and list highlights stop using a blue fallback.
- **A long batch folder path loses its middle**, keeping its start and last folders; the full path is on hover.

### Round 2 lane A (window model and page area)
- **Fill & Sign types where you click**: a click on a field fills it; a click anywhere else opens a text box there (the engine's fill-anywhere), as Preview and Acrobat do. Supersedes the plan's "Add text anywhere only with the Text tool" for Fill & Sign: the tool now shows a chip ("Click a field, or anywhere to type · Esc to stop"), so the click is no longer a stray one (X6).
- **Insert ▸ Text Box arms a "textbox" tool** (Forms mode with the text box button standing): each click places a box with its editor; Esc goes back to Select. No dialog at a fixed spot. The strip has no button for it (menu and chip only).
- **Another document starts with Select**; a tool the open file stops allowing (a signed file opens for reading) falls back to Select.
- **Fields are filled, not moved, in Fill & Sign**: no handles there, a press on a handle does nothing, a press on a signature field is a click (never a drag). The field designer's handles stay in Select; a read-only field's label says "read-only (its value can't be changed)".
- **Select picks annotations** (comments, highlights, drawings, stamps, text boxes, placed signatures): a click picks the smallest one under the pointer (a link or field on top wins), Tab reaches them, Delete removes, Return opens the text, Esc lets go. Text boxes and placed signatures move by dragging or the arrows (⇧ 10 pt); the engine moves no other kind, so a drag on a highlight still selects text.
- **One scale for every page**: fit width fits the widest page, fit page the largest, so mixed portrait/landscape documents keep one zoom as they scroll; zoom in/out from a fit steps from the scale shown.
- **Fit width keeps its 1180 CSS px reading-width cap** (≈192% for Letter at 1920): it is the default zoom on open, and an earlier review asked that a page not grow to 1660 px; explicit zoom goes higher. Recorded, not changed.
- **The view keeps what is read**: the page and the point 35% down the view stay put across zoom, resize, sidebar, panel and notice changes (a pinch keeps the point under the pointer); the page number is the page at that point.
- **The live region follows the latest words** (an action's announcement, else "Page n of m" when the page changed last), with `announceSeq` bumping on each change; Copy is spoken on every copy.
- **Status lines carry a kind**: `Session::set_status_line(id, text, kind)` / `set_status_kind` keep it in the element's `data-kind`; `NodeView.kind` gives it to the window ("" = not marked).
- **Undo names**: words that carry quotes are not quoted again ("Undo Highlighted “quarterly”"); "in n runs", coordinates and resource names (PA1) are dropped; an action that said nothing is named from the engine step ("Undo Add stamp").
- **Redaction rows** read whole words from the box's own page and keep them while another page is shown; Find boxes say "Found ‘…’"; rows wrap and speak their details. On a signed file a press inside a box picks it (Delete removes it) instead of drawing a box in the box.
- **The page menu leaves out object items that cannot apply** when nothing is under the click.
- **Text box has no tool-strip button**: it is armed from Insert ▸ Text Box (and Note's menu) and shows its chip; the strip keeps its eight tools (Text adds text where you click, so a ninth button would be a near-duplicate).
- **Recent files**: "Clear list" sits beside the heading (no question: only the list is forgotten), each row has a quiet × that forgets that file.
- **Compare's page field follows only the core's page**, so a typed number is not put back while other parts of the view change.
- **Help no longer says every change can be undone**: "Changes to the pages and what is on them can be undone with ⌘Z" stays true whether or not a security change joins Undo.
- **The side panel is 320 px** (was 300) with 11 px tab labels, so its five tabs fit one row at normal text sizes.

### Round 2 lane B (core logic and words)
- **A refused page has no targets**: a refused `edit-targets` drops the scan (the old one could be the last document's) and repaints; a new document starts with none. The panel and, in the window, an information notice say why in plain words ("This page's text can't be edited: …").
- **The Text tool on a refused page places a text box** (a comment, which needs nothing from the page): `text-add` needs the page's scan to keep its drawing state balanced, so the engine is not changed.
- **A refused edit keeps what was typed** in the open editor (the renderer put the old text back).
- **A Text-tool click on text the editor gave no target** (drawn twice, unreadable font) says why with a next step instead of opening new text on top of it.
- **List boxes edit with a select of their options**, single choice (the engine reports one value).
- **Field editor letters**: one line at most 14 pt, multi-line and list boxes 12 pt, never under 10 px (the field's own DA size is not in the field list).
- **Object moves are announced in the panel's terms**: "Moved image to 290 from the left, 142 from the top".
- **Find & replace**: the section stays open on its results; "Replaced “X” in 3 places." in the panel and as a success notice; an earlier error notice goes.
- **Names in warnings**: the scripted-field warning names its fields; "A field named “X” already exists. Type another name, or select that field to change it." (STATE keeps the catalogue's words).
- **Copy confirms, Paste with nothing copied says so** (window only). The 10 pt paste cascade stays: an engine test pins it.
- **Trusting a file without a certificate is refused by name**, nothing is written.
- **Certificate dialog**: typing clears its last problem; a dragged box line ("Page 1, 180 × 80 pt, …") from `cert_sign_box_placed`; "No certificate chosen yet" everywhere; the reason is cleared when another document opens.
- **Pressing the armed Sign button stops placing and says so** (it stays a toggle).
- **The visible signature appearance is right** (one fill, one stroke at the widget's /Rect, checked by rendering); the second outline in sign shot 10 is the window's overlay, offset ~18 pt — not a tandem-sign issue.
- **A signature's corner drag follows the pointer**: the corner goes to the point of the box's diagonal nearest the pointer (the larger side ratio made short, wide signatures jump to the page edge); other aspect-locked drags (images) keep `gestureRect`.
- **Same-name refusal**: "Choose another name…" is the default; Cancel is silent (editable copy and redacted copy); a drive keeps its error.
- **Signed banner**: "This file is signed, so it opens for reading: editing it would break the signatures. To change it, make an editable copy without the signatures; the original stays as it is."
- **Batch output folder stays "pdf4all-out"** (drive tests and the Help test pin it); the window's Start is disabled until the text is typed.
- **Recent on the start screen**: `Click{id:"start-recent-clear"}` clears, `Click{id:"start-recent-remove:<path>"}` forgets one (the file is untouched).
- **Security reads the file's own /P**: an encrypted file is parsed once more at open (only when it names /Encrypt) so the dialog's boxes and its line show what the file allows, not what the owner password allows; the pending notice says "When you save, the file gets the new password protection." Help already limits undo to "the pages and what is on them".
- **Export**: a cancelled save panel brings the dialog back. **Print** in the window: a range problem reopens the print dialog with its status, never a lasting notice.
- **An outside link's question defaults to Cancel.** **Compare** has its own pick (`Pick::CompareSource`, "Choose a PDF to compare with"); a drive answers it like Insert.
- **Redaction Find** shows "Added 8 boxes over …" in the window (success) and says when nothing was found; the over-redacted banner is marked a warning (`data-kind` on its text).
- **Dialog statuses carry a kind** through `set_status_line` (error / info / success).
- **Stamp dialog**: says the stamp goes in the middle of the page or over the Find match and can't be dragged yet.
- GAP (recorded): export page ranges and flattened PDF; stamp Centre (engine); password-screen Back; second-launch hand-over (needs a single-instance plugin, a new dependency).

### Round 2 leftovers
- **Redacted-copy banner**: a warning when `banner_state.over_redacted` (content outside the boxes went), info otherwise.
- **Repaired file**: the window says "Repaired — not saved yet" (not "Saved") and offers Save even with no edits; the save is a full rewrite, so once it lands `info.repaired` clears and the label reads "Saved". STATE keeps its raw values.
- **Certificate box drag**: goes through `Session::cert_sign_box_placed`, so the dialog says the page, size and position of the box.
- **Text panel empty line**: shows the core's `content-empty` text (it explains refused pages); the panel's own line only when the core has none.
- **Redaction hint**: a plain line explains the pink tint can reach past a box because a touched form field is removed whole (the core had no words for it).
- **Field outline 12 px low** (sign#10): not a coordinate bug — the core's rect is right; a field target's class `field` also matched the form-label rule `.field { margin: 12px 0 }`. `.hit` and `.hit.field` now reset margin (and display); unit test pins the rect and the reset.
- **Test user-data folders**: every memory-only session (ui tests, `Session::new`) gets `tag-{pid}/{n}` via `host::util::session_scratch`; they no longer share a journal.

### Re-check fixes
- **Saved signature at launch** (08g): `Session::with_host` reads the saved signature once (R:5060), so a relaunch has it and "Remove saved signature" is live; a drive run's store is memory-only, so it still starts empty.
- **Security dialog** (10m): "Require a password" with no open password says "Type a password in "Password to open". The password to change these settings is optional." without asking the engine; a refusal keeps both fields (they are cleared only once the change is accepted, and on close); any other engine refusal goes through `messages::user_text`.
- **Page number box**: as wide as the page count's digits (at least 3ch) plus padding.
- **Read-only fields**: no handles in any tool; a press on one in Select or Fill & Sign is never a drag and says "This field is read-only." in a toast. The core's hit nodes (and the drive) are unchanged.
- **Esc after placing a text box** (05i): checked, no press is lost — the commit leaves the new box selected, the first Esc lets it go, the second returns to Select (§3.2); a unit test pins it.
- **Fit page**: when the zoom changes in fit-page mode (not a pinch) the current page's top goes to the top of the view (half the page gap above it), so the page before is not visible.
- **Fit width at 1920**: the 1180 px reading-width cap stays and so does the label "Fit width"; it means "fit to a comfortable reading width", and the menu item's tooltip says so.
- **Panel tabs**: always one row; labels shrink to fit the panel (container query, never below 10px), past that the row scrolls sideways.
