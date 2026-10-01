# Compare map (Explore/Opus, 2026-09-24)

## Rasterisation
- engine.ts:1079 renderPage(doc, pageIndex, scale): RenderResult; MIN_SCALE 0.25 / MAX_SCALE 4 (:75/:81); MAX_PIXEL_EDGE 4096 (:74) throws at :1093; toPixmap DeviceRGB :1097 → toRgba :6173. RenderResult :255 {width,height (pt), pixelWidth, pixelHeight, pixels: ArrayBuffer RGBA, text, lines: Line[], nonWhite, paintMs, renderMs}; every render also builds structured text :1106 toStructuredText("preserve-spans").
- renderThumbnail :1133 (maxEdge 32..256, :1146) pixels only. Worker ops "render" :4176 {op,page,scale} → transfer pixels; "thumbnail" :4188; both refuse a locked doc.
- renderer.ts paint() :2162 putImageData on #canvas (:195); zoomNow :1912, renderScale :1927 (zoom*dpr clamped); cache createRenderCache engine.ts:915, renderer CACHE_LIMIT 8 / CACHE_BYTES 96 MiB (:163-164), key `${page}:${scale}:${revision}` :1944; drain() :1988 single-flight/latest-wins.
- Print renderPrintSheet :9331 (150 DPI clamped, canvas2 → JPEG data URL → #print-pages, PRINT_MAX_BYTES 40 MiB :189); export :7398 OffscreenCanvas. Budget: page render ≤ 250 ms (measured 105–141).

## Structured text
- content-edit.ts:1342 editStructuredRuns(page): EditRun[] (per line, live font handles, engine-internal). searchDocument engine.ts:1175 {hits:[{page,quads}],total,truncated}. "text-all" :5217 per-page asText joined by \f, needs copy permission, capped MAX_SAVE_BYTES, no geometry.
- "render" already returns lines: Line[] from parseLines(structured.asJSON()) :6047; Line :222 {text,x,y,w,h,size,family,weight,style} (y = baseline). Cheapest reuse: a text-only op (`page-lines`) ≈ 10 lines modelled on renderPage :1106-1111, or render at MIN_SCALE gives lines + pixels in one call.

## Documents per worker
- One doc per session: Session.doc engine.ts:760; createSession :6003; open :4129 replaces + closeDocument(previous); close :4253.
- Scratch-doc precedent: clipboard :3362, :3486-3488 `new mupdf.PDFDocument()`, session.clipboard {doc, items}, comment :796 "open/close leave it alone". Transient second doc: insert-pages :5103 mupdf.Document.openDocument(bytes,"application/pdf"), disableJS(), refuses needsPassword/encrypted (:5134-5135), closes in finally.
- Worker: renderer.ts:773 new Worker(worker.js, module); single worker, pending map, request(); 20 s watchdog. CSP policy.ts:4 worker-src 'self' (second worker allowed but second WASM heap breaks the 600 MB budget; 120 MiB open alone reached 552 MiB). Recommendation: `session.compare` slot in the same session with a smaller size cap.

## Shell
- index.html:182-224 main.app > aside#page-sidebar (#thumbs :195) > section#workspace > #reader > figure > #viewport > #page > canvas#canvas, svg#text-layer, div#hit-layer, #object-menu; figcaption#caption; aside#inspector :226. No second canvas; #print-pages :834 hidden sheets; #redact-preview :584 is a table in #redact-dialog.
- UiMode renderer.ts:161 read|annotate|organize|forms|edit|redact; #mode-* index.html:57-70; modeInputs :486-491; MODE_TAB :516; setMode :5288; OP_MODE :607; OP_PERMISSION engine-core.ts:244 (coverage test).
- Dialogs: #help-dialog :504 helpOp :1599 showModal, #help-close :8480; #verify-dialog :765 openVerifyDialog :7989, verifyClose :8524, Escape :1405. Live regions #status :476, #page-status/#fields-status/#nav-status/#edit-status :478-481, #error :171, #banner/#warning :172-173.
- p2-shell REQUIRED_IDS :16 includes canvas, page, viewport, thumbs, extracted, print-pages, mode-*; asserted :93. STATE stateSummary :7818, window.pdf4allState :7855; top-level keys doc, sign, signatures, verify, mode, page, zoom, search, tab, comments, annotLast?, redact, status, error, announcement, canRedo, help{open,rows}, …editSummary (notes, pages, fields, direct, nav, links…), content, save?, print?. New key `compare` is safe.

## Opening a second file
- open-file preload.cts:24 / open-path :29 → main.ts:569/:573 deliverOpen → token app://pdf4all/open/<token> :396 → 'document' :400 → renderer openFromUrl :1612 → openDocument :1661 (resetDocument + op open) — always replaces. --open= main.ts:31; drive open :1724.
- Password: "authenticate" engine.ts:4212 from session.lockedBytes; renderer :1865, :8362 — no path for a second doc.
- Read-only second file today: insert-source (preload.cts:40 pickInsertSource; main.ts:677-702 showOpenDialog or --insert-from=/`from` :37/:1720, pdfWithinCeiling, readFile → {ok,name,bytes}); renderer pickInsertSource :7153, request insert-pages with transfer :7212-7214. REUSE for compare: pick bytes → new `compare-open` op.

## What Compare must add
- Engine: session.compare slot (clipboard pattern); ops compare-open (unencrypted only), compare-close, compare-render / compare-lines (or `doc:"a"|"b"` on render); OP_PERMISSION rows; decide open/close interaction with the slot.
- Diff: line LCS over Line.text (renderer or engine); pixel diff over two RGBA buffers at one scale (normalise page sizes; countNonWhite :6096 is the loop model).
- Renderer: a compare view (#compare-view, #compare-canvas-a/-b, diff overlay) or a dialog-style panel (less work than a new UiMode which touches MODE_TAB, modeInputs, REQUIRED_IDS, OP_MODE); pick button reusing pickInsertSource; a live region; cache keyed a:/b:.
- Harness: `compare` STATE key + drive step; `from` arg already feeds insert-source. Simplest: no pixel op — render both at one scale, diff pixels in the renderer, lines come in the same reply.
