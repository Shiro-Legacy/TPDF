# Tandem PDF mobile

A mobile version of Tandem PDF for iPhone/iPad and Android, with local PDF tools and optional cloud AI reconstruction. It uses a touch interface inside Capacitor, with a PDF worker on the Tandem PDF Rust engine (WASM). This is a development build, not a store release.

## Included

- Import PDFs through the system file picker, with a 20 MiB input limit.
- Read, navigate pages, zoom, and search page text.
- Highlight whole text lines, draw ink, add/edit/delete notes, and remove annotations.
- Fill supported AcroForm text, checkbox, radio, combo and list fields.
- Draw a handwritten signature and tap to place it on a page.
- Redact: choose **Redact**, drag a box over anything you want removed (text, pictures, drawings, comments, form fields). Boxes are solid black; **Show what's underneath** (or holding a finger down) tints in red what will go. Tap a box and × to remove it; Undo/Redo step through the boxes while the tool is on. **Redact N areas…** confirms in plain words and saves the result as a new library document, "<name> redacted.pdf", which opens with an **Open original** banner; the original is never changed. Leaving, sharing or saving with boxes asks first. One finger draws; two fingers scroll and zoom. Signed files get an unsigned redacted copy, and artwork that can't be cut precisely turns its page into an image, each only after asking; an encrypted file's copy keeps its password.
- Undo/redo within the open session.
- Automatically save a validated PDF copy to the local library after each edit.
- Share a PDF through the native share sheet; browser previews download a copy.
- Try a bundled sample PDF with a fillable field.
- Scan paper with the native document camera, or combine photos into a PDF. Crop, rotate, reorder and remove pages before saving.
- Keep the original scan with searchable/selectable text using on-device OCR, or rebuild it as a typeset document with GPT, Claude, Grok, DeepSeek, GLM or Muse (Meta).
- Review and correct recognition results before saving. AI output includes a separate original image PDF in the library.
- Select/copy recognized page text from Document options.

Reading, editing, image conversion and standard OCR stay on the device and need no account. AI reconstruction sends the selected page images directly to the chosen provider only after explicit consent. Imported originals are not overwritten. Uninstalling the app or clearing its storage removes the local library; use Share to save copies elsewhere.

## Scanning and optional AI

Open **Scan & create**, then **Scan pages** or **Add photos**. Native scanning detects document boundaries; imported images also have a manual rectangular crop and rotation tool. Choose:

- **Original scan + selectable text**: Apple Vision on iOS, bundled ML Kit Latin text recognition on Android. Review each recognized line, correct it, confirm review, and save. No document upload. Android OCR currently supports Latin-script languages; iOS language support depends on the OS. Pages without recognized text retain their image.
- **AI-rebuilt digital document**: choose GPT (OpenAI), Claude (Anthropic), Grok (xAI), DeepSeek, GLM (Z.ai) or Muse (Meta). For GPT, choose **ChatGPT subscription → Continue with ChatGPT**, authorize plan usage in the system browser, and choose a model from your account. Alternatively, connect a personal API key and enter a vision-capable model ID. Consent to sending the page images to the named provider before processing. Review/edit the Markdown text and layout preview, then save. The app typesets headings, paragraphs, lists and up to eight-column tables as real PDF text. It also retains the original image PDF. Figures are represented by text placeholders in the rebuilt version; complex layouts, handwriting and non-Latin shaping need review.
- **Image PDF**: combine the images without recognizing text.

Scan batches are limited to 12 pages, 20 MiB per imported image and 30 MiB of normalized images. Images are normalized to upright JPEGs with a maximum dimension of 2400 pixels. Large reconstructed documents paginate automatically, up to 100 output pages. Cropping or rotation invalidates that page's previous OCR/AI result. Unsaved scans and review edits remain in memory while the app is open; **Save PDF** is required before closing/restarting the app.

The iOS camera scanner needs camera permission and physical hardware. Android's scanner requires Google Play services, at least 1.7 GB RAM and an initial scanner download; Add photos remains available when the scanner cannot start. The bundled Android OCR model works offline. Browser previews support image PDFs and editing the scan flow, but native OCR and provider calls require the installed app.

**ChatGPT subscription sign-in** is implemented on both native platforms using OpenAI's documented local-app OAuth flow. It supports saved accounts, account switching, model discovery, explicit plan permission, token renewal, sign-out and a shortcut to ChatGPT usage settings. It uses the selected account's eligible plan allowance or credits; limits and model access are controlled by OpenAI. Identity-only sign-in cannot send pages until plan usage is enabled. Disconnect clears the current scan connection; Sign out removes that account's local tokens and attempts remote revocation. Registration metadata remains for reconnecting the same account.

**API keys remain available for all six providers**, including GPT. API charges or eligible provider credits apply; the other providers' chat subscriptions are not connected by this API-key flow. Any subscription-linked API credits must be enabled with the provider. Model availability and image support depend on the user's API account; the model hints are examples, not a guarantee of access. Muse uses the Meta Model API with an image-capable model such as `muse-spark-1.3`. Meta AI and Muse Code subscriptions are not connected by this API-key flow. Meta Contributor model variants allow training on submitted content; selecting one adds that permission explicitly to the upload consent. Standard model IDs are shown in the hints.

ChatGPT OAuth tokens stay entirely in native code: device-only Keychain on iOS and an AES-GCM encrypted, non-backed-up file protected by Android Keystore on Android. Authorization uses the system browser, PKCE, state and nonce validation, a loopback callback, and signed identity validation. Tokens never enter the web interface, browser storage or logs. A non-secret welcome-dismissal preference is stored in localStorage.

No shared developer key is embedded. API keys stay in memory for the current app session, are cleared from the input after connecting, and can be removed with Disconnect. Switching providers clears the previous connection, model, entered key and upload consent before another provider can be connected. Disconnecting also clears consent. Keys are not written to IndexedDB, localStorage or preferences. Native bridge logging is disabled to keep credentials and image payloads out of debug logs. Requests use fixed HTTPS provider endpoints and disallow redirects. OpenAI, Grok and Meta requests set `store: false`; provider data policies still apply.

Processing sends one page at a time, preserves successful pages after errors, and does not automatically retry paid requests. Stop prevents additional pages being submitted after the current request finishes; an in-flight request cannot be recalled and may be billed. A retry skips completed pages, including pages completed with a previously connected provider. Reprocess this page discards that page's result after confirmation and enables an explicit new request.

This is a personal-account development integration. OpenAI documents the subscription flow for open-source and locally hosted apps; a paid or remotely hosted product must follow OpenAI's separate onboarding before release. Live account eligibility and the complete phone browser-to-app sign-in still need testing with an eligible account. A consumer service with an app-funded AI allowance would require a server-side credential, authenticated accounts, quotas and billing before deployment. Never put an app-wide production API key into the client.

ChatGPT references: [Overview and deployment scope](https://developers.openai.com/siwc/token-sharing-open-source), [Registration and sign-in](https://developers.openai.com/siwc/token-sharing-open-source/sign-in), [Accounts and sessions](https://developers.openai.com/siwc/token-sharing-open-source/profiles-and-sessions), [Models and inference](https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference), [Preview limitations](https://developers.openai.com/siwc/token-sharing-open-source/preview-limitations).

Provider references: [OpenAI image inputs](https://developers.openai.com/api/docs/guides/images-vision), [OpenAI authentication](https://developers.openai.com/api/reference/overview), [Claude image inputs](https://platform.claude.com/docs/en/build-with-claude/vision), [Grok image understanding](https://docs.x.ai/developers/model-capabilities/images/understanding), [DeepSeek vision](https://api-docs.deepseek.com/guides/vision/), [GLM vision](https://docs.z.ai/guides/vlm/glm-4.6v), [Meta Model API](https://dev.meta.ai/docs/overview), [Meta data/pricing modes](https://dev.meta.ai/docs/pricing-rate-limits). Native references: [Apple Vision](https://developer.apple.com/documentation/vision/recognizing-text-in-images), [Android document scanner](https://developers.google.com/ml-kit/vision/doc-scanner/android), [Android OCR](https://developers.google.com/ml-kit/vision/text-recognition/v2/android).

## Run locally

Node 22.12 or newer is required, with the Rust engine's WASM package in `engine/web/pkg` (built by `engine/tools/build-workflow.sh`; the build refuses a missing or stale one). The type check (`tsc`) and the tests' MuPDF oracle (`mupdf`) come from the repository root's dependencies. From the repository root:

```sh
npm ci
cd mobile
npm ci
npm run dev
```

Open `http://127.0.0.1:4173`. The preview server listens only on this computer. It serves a production bundle and does not hot reload; rebuild after source changes. Native builds bundle these same assets and do not need the server.

## iPhone / iPad

Xcode is required. The generated project targets iOS 15 and newer.

```sh
cd mobile
npm run sync
npm run ios
```

In Xcode, select the App scheme and an iPhone simulator, then Run. For a physical phone, select your signing team and device. The development bundle ID is `com.tandempdf.mobile`; choose an identifier you control before distribution.

Native UI tests cover the document picker, sample form, touch-drawn signature, app restart, native share sheet, on-device OCR, and ChatGPT account-panel initialization without signing in:

```sh
xcodebuild -project ios/App/App.xcodeproj -scheme App \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro' \
  -derivedDataPath /tmp/tandem-mobile-derived CODE_SIGN_IDENTITY=- test
```

Use an installed simulator name. Test targets use local simulator signing; they do not require an Apple developer account. The app includes the Filesystem privacy manifest and native plugin manifests.

## Android

JDK 21, an Android SDK with API 36, and the Android build tools are required. The generated project targets Android 7 / API 24 and newer. Android Studio is the usual way to install these.

```sh
cd mobile
npm run android:build
```

The build script uses `JAVA_HOME` and `ANDROID_HOME` if configured, then checks the ignored `.tooling` directory and standard SDK locations. The current workstation has a verified JDK and SDK under `.tooling`; these are not committed. Gradle may download dependencies on its first run.

The APK is `android/app/build/outputs/apk/debug/app-debug.apk`. Install it on a development device with `adb install -r` or open the native project with `npm run android`. This APK uses a development key.

## Verification

```sh
npm test                  # Type check, build, PDF/OCR round trips, provider contracts
npm run test:ui           # Editor, scanning and ChatGPT account UI; preview server must be running
npm run test:security:ios # Production OAuth/JWT/SSE checks; macOS and Xcode required
npm run test:security:android # Production protocol helpers; JAVA_HOME or bundled JDK required
```

The browser UI tests use Electron (`electron`, a development dependency of this folder, so `npm ci` here downloads it) as a test harness; Electron is not included in the mobile app. They exercise the real PDF workers at phone dimensions. Scanning tests mock only native OCR and paid provider responses; no live API keys or paid requests are used. Native tests run separately through Xcode. Android compilation does not establish physical-device correctness.

Current web verification: all 21 PDF/geometry/redaction/provider-contract tests and all four browser suites pass (`tests/redaction-browser.cjs` draws a box over the secret of `fixtures/redact.pdf` at 375 x 812, applies it, and reads the copy back with MuPDF as an independent check). ChatGPT coverage includes cancelled sign-in and late callbacks, identity-only permission gating, model refresh, account switching, plan usage consent, usage-limit errors with scans retained, no silent API fallback or automatic retry, real PDF creation from mocked successful responses, and sign-out. The browser suites mock native authentication and paid requests; they do not establish live OAuth correctness. The existing six-provider credential-isolation and PDF editing regressions also pass.

Native authentication checks pass: 37 iOS checks cover callback validation, signed identity claims and completed versus failed/refused streams; 31 Android checks cover PKCE, callback parsing, state comparison, encoding and input bounds. The iOS harness uses temporary RSA fixtures and the actual production functions. These tests use no network or accounts. Both native builds succeed, and the Android APK and synchronized iOS web assets match the current web build. No live sign-in, paid inference or physical-camera capture has been performed for this change; no Android device is available for runtime testing.

All four iPhone simulator workflows passed across focused runs: document picker; local OCR through a saved searchable PDF; form editing, signature, persistence and native share sheet; and the ChatGPT account panel. The account-panel smoke test exercises the real native bridge and Keychain, verifies that an unconnected account cannot submit a plan request, and does not open external sign-in. The initial share test failed while disk space was nearly exhausted; after clearing rebuildable mobile caches and stabilizing dialog focus handling in the test, the focused rerun passed.

## Structure

- `src/app.ts`: touch UI and worker requests.
- `src/storage.ts`: IndexedDB library, with transaction completion before acknowledging a save.
- `src/export.ts`: native cache-file sharing and browser download.
- `src/geometry.ts`: page-coordinate mapping, line highlighting and signature placement.
- `src/redact.ts`: the Redact tool's boxes (held in memory and sent as the engine's `regions`), their undo, the copy's name and the flow's words; `app.ts` draws them and runs the flow.
- `src/scan.ts`: image preparation, scan review, OCR corrections and consent flow.
- `src/scan-compose.ts`, `src/scan-worker.ts`: searchable image PDFs and typeset reconstruction, isolated from the editor's worker. The worker has the Rust engine the app ships write the PDF (`scan-compose`; JPEGs stored as they are) and reopens it with that engine before saving; `scan-compose.ts` only lays rebuilt pages out, with the widths the engine measures (`scan-measure`). Text takes Helvetica for WinAnsi, Helvetica's Latin glyph names, then embedded subsets of the engine's own Nimbus Sans (Greek, Cyrillic, arrows, maths, currency), then a subset of the bundled Droid Sans Fallback (CJK, kana, Hangul). In a rebuilt document, characters none of these has (most of Vietnamese, IPA, Hebrew, Arabic, Thai, emoji, most symbols) draw as missing-glyph boxes, though their text still searches and copies (the coverage per block is in `engine/design/app-m9-mobile-release.md`).
- `src/chatgpt.ts`: account and model controls; metadata-only bridge to native ChatGPT authentication and inference.
- `ios/App/App/ChatGPTAccountPlugin.swift`, `android/app/src/main/java/com/tandempdf/mobile/ChatGPTAccountPlugin.java`: native OAuth, protected credentials, refresh, model discovery and streamed inference.
- `src/scan-ai.ts`: shared provider metadata and fixed-endpoint image reconstruction adapters for OpenAI, Anthropic, xAI, DeepSeek, Z.ai and Meta.
- `ios/App/App/DocumentScannerPlugin.swift`, `android/app/src/main/java/com/tandempdf/mobile/DocumentScannerPlugin.java`: native document scanner and on-device OCR.
- `../web/` (the repository's shared TypeScript, which the root tests and the MuPDF test oracle use too): the editor's engine worker (`worker.ts`), its loader (`worker-engine.ts`: the Rust engine's WASM package and CJK font, which the scan worker loads too), the adapter that moves requests and replies across as JSON (`engine-rust.ts`, with `engine-core.ts`), and the worker protocol's types (`protocol.ts`), which the app imports.
- `scripts/build.mjs`: type-checked first by `tsc --noEmit`; checks that `engine/web/pkg` is built from the current `engine/` sources (the root's `scripts/engine-package.mjs`; `TPDF_ALLOW_STALE_ENGINE=1` builds anyway with a warning), builds the UI and the workers (`src/` and `../web/`), copies the Rust engine's glue and WASM, the pinned CJK font (the root's `scripts/cjk-font.mjs`) and the notices into `dist/`, and fails if any MuPDF file reaches `dist/`.
- `scripts/sample.mjs`: generates the bundled practice PDF and sample scan with the Rust engine.
- `ios/`, `android/`: native Capacitor projects.

The mobile app needs nothing from the desktop app's source. The mobile runtime ships no MuPDF code or asset; the tests still read scan output with MuPDF as an independent check.

## Current boundaries

This version has no existing-text editing, certificate signing, PDF page organisation, cloud sync, or incoming share-extension support. OCR currently applies to newly scanned/imported images, not existing image-only PDF files. Handwritten signatures are PDF annotations. Signed/certified PDFs open for reading and byte-preserving sharing. Unsupported PDF operations retain the engine's refusal behavior.

Only one page is rendered at a time. Drawing requires switching back to Read to scroll. Highlights select whole lines. Search uses PDF page text; image PDFs without OCR and form appearance text may not be searchable. Local saves happen after an edit completes; a force-quit during an edit or save may leave the previous saved copy. A storage failure leaves the current edit marked unsaved and available for export.

App-store distribution still requires release signing, device testing (the Rust engine on an iOS WebView is not yet verified on a device), and store metadata. Tandem, including its Rust engine, is MIT (`licenses/Tandem-LICENSE.txt`); the engine's third-party notices are in `licenses/Engine-NOTICE.txt`, and the footer links both. Mobile assets include the engine's notices, its dependencies' licences (`licenses/engine-licenses`), the CJK font's Apache-2.0 notice and the Capacitor plugin licenses.

Architecture references: [Capacitor](https://capacitorjs.com/docs), [Filesystem](https://capacitorjs.com/docs/apis/filesystem), [Share](https://capacitorjs.com/docs/apis/share).
