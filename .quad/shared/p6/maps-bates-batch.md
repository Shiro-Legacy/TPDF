# Bates + Batch map (Explore/Opus, 2026-09-24)

## Header/footer stamp op
- engine.ts:385 StampRequest {id, op:"stamp", kind, text, pages?}; STAMP_KINDS :145 ["watermark","header","footer"]; MAX_STAMP_CHARS 200 :142; OP_PERMISSION "stamp": "edit" (engine-core.ts:252); dispatch :5765 requireMutable → stampPages.
- stampPages :3193-3208: kind check, checkStampText, pages via checkPages (dedupe+sort), ONE mutate("add header"|…) for all pages.
- checkStampText twice: engine.ts:3045 (local) and engine-core.ts:210 (exported, max arg); winAnsiBytes shadowed locally :3059 — Bates must reuse core versions, no third copy.
- stampTextFor :3072-3078 tokens {page} {pages} {label} only; no date/start/zero-pad/prefix.
- stampPage :3106-3187: header/footer size 10 at visual x=36, y = header ? 36 : visualHeight-36, left-aligned only (Bates usually bottom-right → needs text-width measure; watermark uses half-em-per-char estimate). Stream `Q\n/PDF4All <</Kind /Header|Footer|Watermark>> BDC … /PDF4AllF 10 Tf … (…) Tj ET EMC` appended, `q` prepended; font put once per page resources; kind ternary :3150.
- Dialog index.html:636-656 ids #stamp-dialog #stamp-dialog-heading #stamp-kind #stamp-text (maxlength 200) #stamp-hint ("Placeholders: {page} {pages} {label}") #stamp-pages #stamp-save #stamp-cancel; toolbar #stamp-header :129 aria-keyshortcuts Alt+Meta+F.
- renderer.ts refs :399-408; openStampDialog(kind) :7578 default "Page {page} of {pages}"; saveStampDialog :7595 parsePageRange; addStamp :7612 request → applyEdit → announcePage; enable :2096 canEdit.
- Menu main.ts:1275-1279 'Header and Footer…' Alt+CmdOrCtrl+F → edit-request {op:'header-footer'}; renderer.ts:1565 routes → openStampDialog("header") or driveStamp(contents) :1162 {kind,text,pages:"1-3"}; OP_MODE :612 "organize".
- STATE decorate :7904, shape :700 {stamps:{watermark,header}, labels, layers, layersSaved, last:{action:"stamp",kind,pages,ok,detail}} — no footer/bates key; decorateLast :7688.
- Tests: p2-decorate.test.mjs:164 (rotation), :194 (placeholders); tests/drive/dialogs.jsonl:15-16 header-footer step; p3-drive.test.mjs:277-292; main.ts:240 decorateOk.

## Headless paths / document lifetime
- Smoke flags main.ts:27-40; --edit allowlist :440 (no header-footer kind; decorate = watermark+header hard-coded renderer.ts:7708); smokeEditPayload :189-203; one file per Electron run (deliverOpen :1521 → P1_RESULT → edit-request → P2_EDIT → save-request → P1_SAVE → finish).
- Drive startDrive :1680-1744: open/save-to/save-in-place/print-to/wait/state; else edit-request {op, contents}; 60 s per step; ${OUT}; to/from/images pre-answer dialogs. A drive can already loop open → header-footer → save-to over N files.
- Save IPC: save-file :583-647 (openPaths url check → 'document-changed'; saveWithinCeiling policy.ts:14, MAX_SAVE_BYTES 50 MiB limits.ts:13; saveInPlace/saveCopy); save-new :704-747 writeNew, no source check; MAX_PDF_BYTES 120 MiB.
- One doc per session: worker.ts:11 createSession; open :4129-4172 replaces session.doc + closeDocument(previous); close :4253; generationCounter/editScanCache closure globals :1436-1437. Batch must open/stamp/save/close sequentially; renderer `doc` :635, revision/dirty :645-646; path only in main openPaths :49 (per window) → batch must not go through doc/openPaths, or must refuse with unsaved changes.

## Folder access
- preload.cts:23-140 channels open-file, open-path, save-file, insert-source, save-new, pick-images (multiSelections), pick-certificate, trust-add, print, sign-file, verify-signatures. NO directory channel (no openDirectory/readdir in src/).
- Template main.ts:883-914 pick-images (isTrustedOpen :121-126; smoke imagesArg override; showOpenDialog; IMAGE_MAX_FILES; isAbsolute; inspectRegularFile; readFile). app:// assets policy.ts:31-48, one-shot open tokens policy.ts:19 / main.ts:395-399.
- Needed: `pick-folder` channel (openDirectory → readdir *.pdf → inspectRegularFile → ceiling), main keeps the folder path; per-file bytes like insert-source; output via writeNew to a separate folder / `${stem}-<suffix>.pdf`; smoke override `--batch-dir=`; preload entry; Pdf4AllApi member renderer.ts:105.

## Shell for a progress view
- Sidebar tabs index.html:183-198 (#tab-pages/#tab-bookmarks); inspector tabs :226-236 (#tab-notes|fields|links|text|layers, panels .panel role=tabpanel); selectTab renderer.ts:5237.
- Dialog model: #verify-dialog (:765-775) with <p id="verify-summary" role="status" aria-live="polite">, <tbody id="verify-list"> rows, Refresh/Close; openVerifyDialog :7989 showModal; reset closes dialogs :939-945; Escape :1405.
- Live regions #error (alert :171), #banner, #warning; #status/#page-status/#fields-status/#nav-status/#edit-status :476-481; announce() :5232 → #edit-status; announcePage :7148.
- p2-shell: REQUIRED_IDS :16 (asserted :93); every button/textarea/input/select needs id + accessible name (:96-123); :127 literal help dialog tag. CSS tokens styles.css:6-25; shared button rule :131-161; dialog :1031.

## Reports / errors
- finish(result, ok) main.ts:100-118 (P0_SMOKE line, writes --report JSON, app.exit) — exits the app, not reusable in-app. Drive steps[] pair cmd+STATE (:1743); driveSecrets :1665.
- Batch report: renderer-built table (verify-dialog pattern), optionally saved via save-new with a new kind (isSaveKind/KIND_EXT/KIND_FILTER in main.ts).
- showError/clearError renderer.ts:9211-9219; SAVE_REASON_TEXT :9023-9030; open-error :971-982; STATE error :7840; driven() :1013. Batch needs its own STATE key with per-file {name, ok, reason, pages, batesFrom, batesTo}.
