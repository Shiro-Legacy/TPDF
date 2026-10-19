// Redact tool at phone size (docs/rust-and-ux-plan.md §2.3): draw a box over the visible secret on fixtures/redact.pdf,
// confirm, and check that the copy is saved as a new library document and opened, the original is unchanged, and
// the secret is gone from the copy's text there (read with the mupdf npm API as an independent black box). Then:
// sharing and saving with pending boxes, unsaved edits and a full device, a box cutting glyphs on a rotated and
// cropped page 2 at zoom 2.5 with two-finger scrolling, artwork that has to become an image, a signed file, and an
// encrypted file.
// Run after `npm run build` with the preview server on port 4173:
//   ../node_modules/.bin/electron tests/redaction-browser.cjs
// SCREENSHOTS=<dir> also writes the p0m-*.png captures there (docs/ux-audit/after).
const { app, BrowserWindow } = require('electron');
const { mkdtempSync, writeFileSync, readFileSync, mkdirSync } = require('node:fs');
const { tmpdir } = require('node:os');
const { resolve, join } = require('node:path');
const { pathToFileURL } = require('node:url');
const assert = require('node:assert/strict');
const { rotatedPages, meshPages, sharedHeader } = require('./redaction-pdfs.cjs');
app.setPath('userData', mkdtempSync(join(tmpdir(), 'tandem-mobile-redact-')));
const shots = process.env.SCREENSHOTS ? resolve(process.env.SCREENSHOTS) : mkdtempSync(join(tmpdir(), 'tandem-redact-shots-'));
mkdirSync(shots, { recursive: true });
const work = mkdtempSync(join(tmpdir(), 'tandem-redact-files-'));
const SECRET = 'P0B-SECRET-7731';
// The line "visible P0B-SECRET-7731 here" (baseline 740 pt up, so about 40 to 56 pt down on the 612 x 792 page).
const BOX = [66, 34, 300, 60];
const overlaps = (a, b) => a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3];
let mupdf;
const pdfText = (bytes, page) => mupdf.Document.openDocument(bytes, 'application/pdf').loadPage(page).toStructuredText().asText();
function linesNear(bytes, rect, page = 0) {
  const text = JSON.parse(mupdf.Document.openDocument(bytes, 'application/pdf').loadPage(page).toStructuredText().asJSON());
  return text.blocks.flatMap(block => block.lines ?? [])
    .filter(line => overlaps([line.bbox.x, line.bbox.y, line.bbox.x + line.bbox.w, line.bbox.y + line.bbox.h], rect))
    .map(line => line.text);
}
const pause = ms => new Promise(r => setTimeout(r, ms));
app.whenReady().then(async () => {
  mupdf = await import('mupdf');
  // The page sizes the app gets: the engine's open reply for the same file.
  const { rustEngine } = await import(pathToFileURL(resolve(__dirname, 'rust.mjs')).href);
  function openReply(bytes, password) {
    const engine = rustEngine(), session = engine.createSession(); let id = 0;
    const send = request => engine.dispatch({ ...request, id: ++id }, session).reply;
    let reply = send({ op: 'open', pdf: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength), name: 'x.pdf' });
    if (password) reply = send({ op: 'authenticate', password });
    return reply;
  }
  const window = new BrowserWindow({ width: 375, height: 812, useContentSize: true, show: false, webPreferences: { contextIsolation: true, nodeIntegration: false, sandbox: true } });
  const web = window.webContents;
  const errors = [], downloads = [];
  let step = 'startup', sizes = [], pageIndex = 0;
  const watchdog = setTimeout(() => { console.error('Timed out at', step); app.exit(1); }, 420_000);
  web.on('console-message', (_event, level, message) => { if (level >= 3) errors.push(message); });
  web.session.on('will-download', (_event, item) => { item.setSavePath(join(work, `dl-${downloads.length}.pdf`)); downloads.push(item.getFilename()); });
  const js = code => web.executeJavaScript(code);
  async function wait(expression) {
    for (let i = 0; i < 300; i++) { if (await js(expression)) return; await pause(100); }
    throw new Error(`Timed out at ${step}: ${expression}\n${await js('document.body.innerText')}`);
  }
  const idle = () => wait('!document.body.classList.contains("busy")');
  const settle = () => js('new Promise(r => requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(r, 60))))');
  async function click(selector) { step = selector; await js(`document.querySelector(${JSON.stringify(selector)}).click()`); await idle(); }
  /** A click that opens a dialog while the app is still busy with it. */
  async function press(selector, dialog) { step = selector; await js(`document.querySelector(${JSON.stringify(selector)}).click()`); await wait(`document.getElementById(${JSON.stringify(dialog)}).open`); }
  async function answer(dialog, value) { step = `${dialog}:${value}`; await js(`document.querySelector('#${dialog} button[value="${value}"]').click()`); }
  async function shot(name) { await settle(); writeFileSync(join(shots, name), (await web.capturePage()).toPNG()); }
  const text = id => js(`document.getElementById(${JSON.stringify(id)}).textContent`);
  const isOpen = id => js(`document.getElementById(${JSON.stringify(id)}).open`);
  const count = () => js('document.getElementById("redact-count").hidden ? 0 : Number(document.getElementById("redact-count").textContent)');
  async function goTo(index) { while (pageIndex < index) { await click('#next'); pageIndex++; } while (pageIndex > index) { await click('#prev'); pageIndex--; } }
  /** Page pt (y-down, the open reply's page size) to window coordinates on the drawn page. */
  async function at(x, y) {
    const { width, height } = sizes[pageIndex];
    return js(`(() => { const r = document.getElementById('overlay').getBoundingClientRect(); return [r.left + ${x} * r.width / ${width}, r.top + ${y} * r.height / ${height}]; })()`);
  }
  /** Scrolls so the page rect's centre is in the middle of the viewport. */
  async function reveal(rect) {
    const [cx, cy] = await at((rect[0] + rect[2]) / 2, (rect[1] + rect[3]) / 2);
    await js(`(() => { const v = document.getElementById('viewport'), r = v.getBoundingClientRect(); v.scrollBy(${cx} - (r.left + r.width / 2), ${cy} - (r.top + r.height / 2)); })()`);
    await settle();
  }
  async function mouse(type, [x, y]) { await web.debugger.sendCommand('Input.dispatchMouseEvent', { type, x, y, button: 'left', buttons: type === 'mouseReleased' ? 0 : 1, clickCount: 1 }); }
  /** A finger drag from one page point to another, as real pointer events. */
  async function drag(from, to, beforeUp) {
    const a = await at(...from), b = await at(...to);
    await mouse('mousePressed', a);
    for (let i = 1; i <= 6; i++) await mouse('mouseMoved', [a[0] + (b[0] - a[0]) * i / 6, a[1] + (b[1] - a[1]) * i / 6]);
    if (beforeUp) await beforeUp();
    await mouse('mouseReleased', b); await idle();
  }
  const dragBox = rect => drag([rect[0], rect[1]], [rect[2], rect[3]]);
  async function tap(p) { const c = await at(...p); await mouse('mousePressed', c); await mouse('mouseReleased', c); await idle(); }
  /** The red, green and blue of the boxes layer at a page point. */
  async function shadeAt(x, y) {
    const { width, height } = sizes[pageIndex];
    return js(`(() => { const c = document.getElementById('redactions'); const d = c.getContext('2d').getImageData(Math.round(${x} * c.width / ${width}), Math.round(${y} * c.height / ${height}), 1, 1).data; return [d[0], d[1], d[2], d[3]]; })()`);
  }
  /** The bounds of the page's dark ink, in page pt. */
  async function inkBounds() {
    const { width, height } = sizes[pageIndex];
    return js(`(() => { const c = document.getElementById('page'), d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data; let x0 = 1e9, y0 = 1e9, x1 = -1, y1 = -1;
      for (let y = 0; y < c.height; y++) for (let x = 0; x < c.width; x++) if (d[(y * c.width + x) * 4] < 128) { x0 = Math.min(x0, x); x1 = Math.max(x1, x); y0 = Math.min(y0, y); y1 = Math.max(y1, y); }
      return [x0 * ${width} / c.width, y0 * ${height} / c.height, (x1 + 1) * ${width} / c.width, (y1 + 1) * ${height} / c.height]; })()`);
  }
  async function library() {
    return js(`new Promise((resolve, reject) => { const open = indexedDB.open('tandem-mobile', 1); open.onerror = () => reject(open.error); open.onsuccess = () => { const all = open.result.transaction('documents').objectStore('documents').getAll(); all.onsuccess = () => resolve(all.result.map(d => ({ id: d.id, name: d.name, bytes: Array.from(new Uint8Array(d.bytes)) }))); }; })`);
  }
  const stored = async name => Buffer.from((await library()).find(d => d.name === name).bytes);
  const noOverflow = () => js('document.documentElement.scrollWidth <= innerWidth');
  async function importFile(path, password) {
    step = `import ${path}`;
    const bytes = readFileSync(path);
    sizes = openReply(bytes, password).pages; pageIndex = 0;
    const { root } = await web.debugger.sendCommand('DOM.getDocument');
    const { nodeId } = await web.debugger.sendCommand('DOM.querySelector', { nodeId: root.nodeId, selector: '#picker' });
    await web.debugger.sendCommand('DOM.setFileInputFiles', { nodeId, files: [path] });
    if (password) {
      await wait('document.getElementById("password-dialog").open');
      await js(`document.getElementById('password').value=${JSON.stringify(password)}; document.querySelector('#password-dialog button[value=unlock]').click()`);
    }
    await wait('!document.getElementById("editor").hidden && !document.body.classList.contains("busy")');
    return bytes;
  }
  /** From the Redact bar through the confirmation (and any follow-up question) to the opened copy. */
  async function redactNow(followUp) {
    await press('#redact-start', 'redact-confirm');
    await answer('redact-confirm', 'apply');
    if (followUp) { await wait(`document.getElementById('redact-question').open`); await followUp(); }
    await idle();
    pageIndex = 0;
  }
  const backToLibrary = async () => { await click('#back'); assert.equal(await js('document.getElementById("library").hidden'), false); };
  /** Every visible control of the editor is a 44 pt target. */
  async function targets(label) {
    const small = await js(`Array.from(document.querySelectorAll('#editor button, .redact-peek')).filter(e => e.offsetParent && !e.closest('#paper')).map(e => { const r = e.getBoundingClientRect(); return [e.textContent.trim() || e.getAttribute('aria-label'), Math.round(r.width), Math.round(r.height)]; }).filter(([, w, h]) => w < 44 || h < 44)`);
    assert.deepEqual(small, [], `${label}: every control is at least 44 x 44 pt`);
  }
  try {
    await window.loadURL('http://127.0.0.1:4173'); await idle();
    web.debugger.attach('1.3');
    const original = await importFile(resolve('../fixtures/redact.pdf'));
    assert.ok(linesNear(original, BOX).some(line => line.includes(SECRET)), 'the fixture shows the secret on the boxed line');

    // 1. The tool: a chip says what to do and how to scroll, with a way to stop.
    assert.equal(await js('document.querySelector("[data-mode=redact]").disabled'), false);
    await click('[data-mode=redact]');
    assert.equal(await js('document.getElementById("redact-bar").hidden'), false);
    assert.equal(await text('redact-chip-text'), 'Drag over anything you want removed.');
    assert.match(await js('document.querySelector(".redact-chip small").textContent'), /two fingers to scroll/i);
    assert.equal(await js('document.getElementById("redact-start").hidden'), true, 'no Redact button before a box exists');
    assert.equal(await noOverflow(), true, 'the tool fits at 375 pt');
    await targets('375 x 812');
    await shot('p0m-1-tool-on.png');

    // 2. Drag a box: translucent while the finger is down, then solid black with a red outline.
    await drag([BOX[0], BOX[1]], [BOX[2], BOX[3]], async () => {
      const [r, g, b, a] = await shadeAt(150, 47);
      assert.ok(a > 0 && a < 200, `the box is translucent while the finger is down (${r},${g},${b},${a})`);
    });
    assert.equal(await count(), 1);
    assert.equal(await text('redact-start'), 'Redact 1 area…');
    assert.equal(await text('redact-list'), 'Redactions (1)');
    assert.equal(await js('document.getElementById("redact-tool").getAttribute("aria-label")'), 'Redact, 1 box pending');
    assert.deepEqual(await shadeAt(150, 47), [0, 0, 0, 255], 'a drawn box is solid black');
    assert.equal(await js('document.getElementById("redact-remove").hidden'), false, 'the new box is selected, with its ×');
    await shot('p0m-2-box.png');

    // 3. "Show what's underneath": translucent, with what goes tinted red (from redact-inspect).
    await js(`document.getElementById('redact-peek').click()`); await idle(); await settle();
    const tinted = await js(`(() => { const c = document.getElementById('redactions'), x = c.getContext('2d'); const d = x.getImageData(0, 0, c.width, c.height).data; let red = 0; for (let i = 0; i < d.length; i += 4) if (d[i + 3] > 60 && d[i] > 150 && d[i + 1] < 120) red++; return red; })()`);
    assert.ok(tinted > 200, `the glyphs under the box are tinted red (${tinted} pixels)`);
    await shot('p0m-3-underneath.png');
    await js(`document.getElementById('redact-peek').click()`); await idle();

    // Undo and redo the box; select it and remove it with ×; undo brings it back.
    await click('#undo'); assert.equal(await count(), 0);
    assert.equal(await js('document.getElementById("redact-tool").hasAttribute("aria-label")'), false);
    await click('#redo'); assert.equal(await count(), 1);
    await tap([20, 400]); assert.equal(await js('document.getElementById("redact-remove").hidden'), true, 'a tap elsewhere deselects');
    await tap([150, 47]); assert.equal(await js('document.getElementById("redact-remove").hidden'), false, 'a tap on a box selects it');
    await click('#redact-remove'); assert.equal(await count(), 0);
    await click('#undo'); assert.equal(await count(), 1);

    // Leaving, sharing or saving with a box asks first; Cancel stays and nothing is downloaded or saved.
    await press('#back', 'redact-leave');
    assert.equal(await text('redact-leave-title'), '1 area is marked but not redacted yet.');
    assert.equal(await text('redact-leave-other'), 'Discard boxes');
    await answer('redact-leave', 'cancel'); await idle();
    assert.equal(await js('document.getElementById("editor").hidden'), false);
    await press('#share', 'redact-leave');
    assert.equal(await text('redact-leave-other'), 'Share without redacting');
    assert.equal(await js('document.activeElement.value'), 'redact', 'Redact now… is the default');
    await answer('redact-leave', 'cancel'); await idle(); await pause(400);
    assert.deepEqual(downloads, [], 'nothing is shared until the question is answered');
    await click('#menu'); await press('#save', 'redact-leave');
    assert.equal(await text('redact-leave-other'), 'Save without redacting');
    await answer('redact-leave', 'cancel'); await idle();
    assert.equal(await count(), 1);

    // 4. The list by page, then the plain confirmation.
    await press('#redact-list', 'redactions-dialog'); await idle();
    assert.ok((await text('redactions-list')).includes('Page 1'));
    assert.ok((await text('redactions-list')).includes(SECRET), 'the list says what the box removes');
    assert.equal(await text('redactions-apply'), 'Redact 1 area…');
    await press('#redactions-apply', 'redact-confirm');
    assert.equal(await text('redact-confirm-title'), 'Permanently remove everything inside 1 box?');
    assert.ok((await text('redact-confirm')).includes('deleted from the file, not just covered'));
    assert.ok((await text('redact-confirm')).includes('Also remove hidden information, other comments and form entries (author, document properties, attachments, bookmarks…)'));
    assert.equal(await js('document.getElementById("redact-hidden").checked'), true);
    assert.deepEqual(await js(`Array.from(document.querySelectorAll('#redact-sanitize input')).map(i => i.checked)`), Array(9).fill(true));
    await js(`document.getElementById('redact-hidden').click()`);
    assert.deepEqual(await js(`Array.from(document.querySelectorAll('#redact-sanitize input')).map(i => i.checked)`), Array(9).fill(false), 'the master box turns the nine off');
    await js(`document.getElementById('redact-hidden').click()`);
    await shot('p0m-4-confirm.png');
    await js(`document.getElementById('redact-details').open = true`);
    await shot('p0m-4b-confirm-details.png');
    assert.equal(await js(`(() => { const d = document.getElementById('redact-confirm'), b = document.getElementById('redact-confirm-apply').getBoundingClientRect(), r = d.getBoundingClientRect(); return b.bottom <= r.bottom && b.top >= r.top; })()`), true, 'Redact and save stays in view with Details open');

    // 5. Redact and save: a new library document, opened, with the banner.
    await answer('redact-confirm', 'apply'); await idle();
    await wait('!document.getElementById("redacted-banner").hidden');
    assert.equal(await text('name'), 'redact redacted.pdf');
    assert.equal(await text('redacted-text'), 'You are viewing the redacted copy ‘redact redacted.pdf’. 1 area removed and checked.');
    assert.equal(await js('document.querySelector("[data-mode=read]").getAttribute("aria-pressed")'), 'true');
    assert.equal(await count(), 0, 'the boxes went with the original');
    assert.equal(await noOverflow(), true);
    await shot('p0m-5-banner.png');
    assert.equal((await library()).length, 2, 'the copy is a new document beside the original');
    assert.ok((await stored('redact.pdf')).equals(original), 'the original is unchanged');
    const copy = await stored('redact redacted.pdf');
    const near = linesNear(copy, BOX);
    assert.ok(!near.some(line => line.includes(SECRET)), `the secret is gone from the boxed line: ${JSON.stringify(near)}`);
    assert.ok(!near.some(line => line.includes('visible')), 'everything inside the box went');
    assert.ok(linesNear(copy, [66, 85, 300, 100]).some(line => line.includes('invisible')), 'text outside the box stays');
    // The unchanged copy is shared under its own name.
    await click('#share');
    for (let i = 0; i < 50 && !downloads.length; i++) await pause(100);
    assert.deepEqual(downloads, ['redact redacted.pdf']);
    // With a new box on the copy: the page-text dialog warns, and sharing it anyway marks the name edited.
    await click('[data-mode=redact]');
    await dragBox([66, 600, 300, 640]);
    await click('#menu'); await click('#copy-text');
    assert.equal(await js('document.getElementById("copy-text-boxes").hidden'), false);
    assert.equal(await text('copy-text-boxes'), 'Text under your boxes is still in the file until you redact.');
    await click('[data-close=copy-text-dialog]');
    await press('#share', 'redact-leave'); await answer('redact-leave', 'other'); await idle();
    for (let i = 0; i < 50 && downloads.length < 2; i++) await pause(100);
    assert.deepEqual(downloads, ['redact redacted.pdf', 'redact redacted-edited.pdf']);
    // Unsaved edits on the copy itself: the question names the copy, not "your original".
    await js(`void (() => { window.realTransaction = IDBDatabase.prototype.transaction; IDBDatabase.prototype.transaction = function(...args) { if (args[1] === 'readwrite') throw new DOMException('Full', 'QuotaExceededError'); return window.realTransaction.apply(this, args); }; })()`);
    await click('[data-mode=highlight]');
    await drag([70, 662], [200, 676]);
    assert.equal(await js('document.getElementById("save-state").textContent'), 'Unsaved changes');
    await click('[data-mode=redact]');
    await press('#redact-start', 'redact-question');
    assert.equal(await text('redact-question-text'), 'They will be in the new redacted copy, but not in ‘redact redacted.pdf’.');
    assert.equal(await text('redact-question-yes'), 'Save them to this copy first');
    await answer('redact-question', 'cancel'); await idle();
    await js('void (IDBDatabase.prototype.transaction = window.realTransaction)');
    await click('#menu'); await press('#save', 'redact-leave'); await answer('redact-leave', 'other'); await idle();
    assert.equal(await js('document.getElementById("save-state").textContent'), 'Saved on device');

    // Open original from the banner (the box on the copy is discarded first).
    await press('#open-original', 'redact-leave'); await answer('redact-leave', 'other'); await idle();
    assert.equal(await text('name'), 'redact.pdf');
    assert.equal(await js('document.getElementById("redacted-banner").hidden'), true);

    // Unsaved edits and a full device: asked before redacting; when the copy cannot be stored, the user stays on
    // the original with the box.
    await js(`void (() => { window.realTransaction = IDBDatabase.prototype.transaction; IDBDatabase.prototype.transaction = function(...args) { if (args[1] === 'readwrite') throw new DOMException('Full', 'QuotaExceededError'); return window.realTransaction.apply(this, args); }; })()`);
    await click('[data-mode=highlight]');
    await drag([70, 662], [200, 676]);
    assert.equal(await js('document.getElementById("save-state").textContent'), 'Unsaved changes');
    await click('[data-mode=redact]');
    await dragBox([66, 600, 300, 640]);
    await press('#redact-start', 'redact-question');
    assert.equal(await text('redact-question-title'), 'You have other unsaved changes.');
    assert.equal(await text('redact-question-text'), 'They will be in the redacted copy, but not in your original.');
    assert.equal(await text('redact-question-yes'), 'Save them to the original first');
    assert.equal(await text('redact-question-other'), 'Continue');
    await answer('redact-question', 'other');
    await wait(`document.getElementById('redact-confirm').open`);
    await answer('redact-confirm', 'apply');
    await wait(`document.getElementById('redact-question').open && document.getElementById('redact-question-title').textContent.includes('couldn’t be saved')`);
    await answer('redact-question', 'cancel'); await idle();
    assert.equal(await text('name'), 'redact.pdf', 'the user stays on the original');
    assert.equal(await count(), 1, 'with the box');
    await js('void (IDBDatabase.prototype.transaction = window.realTransaction)');
    assert.equal((await library()).length, 2);
    await click('#menu'); await press('#save', 'redact-leave'); await answer('redact-leave', 'other'); await idle();
    assert.equal(await js('document.getElementById("save-state").textContent'), 'Saved on device');
    await press('#back', 'redact-leave'); await answer('redact-leave', 'other'); await idle();
    assert.equal(await js('document.getElementById("library").hidden'), false);

    // A box whose edge cuts glyphs (R1) on a cropped, turned page 2 at zoom 2.5, scrolled; two fingers scroll.
    writeFileSync(join(work, 'turned.pdf'), rotatedPages());
    const turned = await importFile(join(work, 'turned.pdf'));
    await goTo(1);
    for (let i = 0; i < 6; i++) await click('#zoom-in');
    await click('[data-mode=redact]');
    const ink = await inkBounds();
    assert.ok(ink[3] - ink[1] > ink[2] - ink[0], `the word runs down the turned page: ${ink}`);
    const cut = [ink[0] - 3, ink[1] + (ink[3] - ink[1]) * .3, ink[2] + 3, ink[1] + (ink[3] - ink[1]) * .7];
    await reveal(cut);
    assert.ok(await js('document.getElementById("viewport").scrollTop + document.getElementById("viewport").scrollLeft') > 0, 'the page is scrolled');
    await dragBox(cut);
    assert.equal(await count(), 1);
    // Two fingers move the page and draw nothing.
    await web.debugger.sendCommand('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 5 });
    await js('document.getElementById("viewport").scrollTo(0, 0)');
    const [fx, fy] = await js('(() => { const r = document.getElementById("viewport").getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2 + 40]; })()');
    const before = 0;
    await web.debugger.sendCommand('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: fx - 30, y: fy, id: 1 }, { x: fx + 30, y: fy, id: 2 }] });
    for (let i = 1; i <= 5; i++) await web.debugger.sendCommand('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: fx - 30, y: fy - i * 20, id: 1 }, { x: fx + 30, y: fy - i * 20, id: 2 }] });
    await web.debugger.sendCommand('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
    await idle();
    const panned = await js('document.getElementById("viewport").scrollTop');
    assert.ok(Math.abs(panned - 100) <= 1, `two fingers track 1:1 (${panned} for 100)`);
    // A drag that a second finger joins draws no box either.
    await web.debugger.sendCommand('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: fx, y: fy, id: 1 }] });
    for (let i = 1; i <= 3; i++) await web.debugger.sendCommand('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: fx + i * 15, y: fy + i * 15, id: 1 }] });
    await web.debugger.sendCommand('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: fx + 45, y: fy + 45, id: 1 }, { x: fx + 100, y: fy + 45, id: 2 }] });
    await web.debugger.sendCommand('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: fx + 45, y: fy + 25, id: 1 }, { x: fx + 100, y: fy + 25, id: 2 }] });
    await web.debugger.sendCommand('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
    await idle();
    await web.debugger.sendCommand('Emulation.setTouchEmulationEnabled', { enabled: false });
    assert.ok(await js('document.getElementById("viewport").scrollTop') > before + 50, `two fingers scroll the page (${await js('document.getElementById("viewport").scrollTop')})`);
    assert.equal(await count(), 1, 'two fingers draw no box');
    await redactNow();
    const cutCopy = await stored('turned redacted.pdf');
    const page2 = pdfText(cutCopy, 1).replace(/\s/g, '');
    assert.ok(pdfText(turned, 1).includes('AAAASECRETAAAA'));
    assert.doesNotMatch(page2, /[SECRT]/, `every glyph the box touches went: ${page2}`);
    assert.equal(page2.replace(/[^A]/g, '').length, 8, `the glyphs outside the box stay: ${page2}`);
    await backToLibrary();

    // Artwork that can't be cut precisely: the list says so, the user is asked, and the page becomes a picture.
    writeFileSync(join(work, 'mesh.pdf'), meshPages());
    await importFile(join(work, 'mesh.pdf'));
    await click('[data-mode=redact]');
    await dragBox([90, 170, 330, 200]);
    await press('#redact-list', 'redactions-dialog'); await idle();
    assert.match(await text('redactions-list'), /can’t be cut precisely/);
    await click('[data-close=redactions-dialog]');
    await redactNow(async () => {
      assert.equal(await text('redact-question-title'), 'Page 1 contains artwork that can\'t be cut precisely.');
      assert.equal(await text('redact-question-text'), 'Redact page 1 by turning it into an image? Its text will no longer be selectable.');
      await shot('p0m-6-flatten.png');
      await answer('redact-question', 'yes');
    });
    await wait('!document.getElementById("redacted-banner").hidden');
    assert.match(await text('redacted-text'), /Page 1 is now an image\./);
    const meshCopy = await stored('mesh redacted.pdf');
    assert.ok(!pdfText(meshCopy, 0).includes('MESHSECRET') && pdfText(meshCopy, 1).includes('PAGETWO'));
    await backToLibrary();

    // Content shared with an unboxed place goes there too, and the banner says so.
    writeFileSync(join(work, 'header.pdf'), sharedHeader());
    await importFile(join(work, 'header.pdf'));
    await click('[data-mode=redact]');
    await dragBox([90, 370, 400, 400]); await dragBox([90, 470, 400, 500]);
    await goTo(1); await dragBox([90, 470, 400, 500]);
    await redactNow();
    await wait('!document.getElementById("redacted-banner").hidden');
    assert.match(await text('redacted-text'), /3 areas removed and checked\. Some content outside your boxes was also removed, because it is shared with other parts of the document\. Check the copy\.$/);
    const header = await stored('header redacted.pdf');
    assert.ok(![0, 1].some(p => pdfText(header, p).includes('ACCT-4417-0092')), 'the boxed number is gone from both pages');
    await backToLibrary();

    // A signed file: the tool is on, the user is asked, and an unsigned copy opens.
    await importFile(resolve('../fixtures/p5/signed-ec.pdf'));
    assert.equal(await js('document.querySelector("[data-mode=redact]").disabled'), false, 'Redact works on signed files');
    await click('[data-mode=redact]');
    await dragBox([100, 100, 300, 160]);
    await redactNow(async () => {
      assert.equal(await text('redact-question-title'), 'Redacting removes the signatures. Make an unsigned redacted copy?');
      await answer('redact-question', 'yes');
    });
    await wait('!document.getElementById("redacted-banner").hidden');
    assert.match(await text('redacted-text'), /The copy has no signatures\./);
    assert.equal(await js('document.querySelector("[data-mode=highlight]").disabled'), false, 'the unsigned copy can be edited');
    assert.ok((await stored('signed-ec.pdf')).equals(readFileSync(resolve('../fixtures/p5/signed-ec.pdf'))), 'the signed original is unchanged');
    await backToLibrary();

    // An encrypted file: the copy opens with the password the original was unlocked with.
    await importFile(resolve('../fixtures/encrypted.pdf'), 'pdf4all-user');
    await click('[data-mode=redact]');
    await dragBox([60, 60, 300, 120]);
    await redactNow();
    assert.equal(await isOpen('password-dialog'), false, 'no second password prompt');
    assert.equal(await text('name'), 'encrypted redacted.pdf');
    assert.ok(mupdf.Document.openDocument(await stored('encrypted redacted.pdf'), 'application/pdf').needsPassword(), 'the stored copy still needs the password');
    assert.equal(await js('document.getElementById("redacted-banner").hidden'), false);

    // Cancelling the password of a document on screen goes back to the library, not to a page with no document.
    await backToLibrary();
    await js(`Array.from(document.querySelectorAll('.file-open')).find(b => b.textContent.includes('encrypted redacted.pdf')).click()`);
    await wait('document.getElementById("password-dialog").open');
    await answer('password-dialog', 'cancel'); await idle();
    assert.equal(await js('document.getElementById("editor").hidden && !document.getElementById("library").hidden'), true);
    await js(`Array.from(document.querySelectorAll('.file-open')).find(b => b.textContent.includes('encrypted redacted.pdf')).click()`);
    await wait('document.getElementById("password-dialog").open');
    await js(`document.getElementById('password').value='pdf4all-user'; document.querySelector('#password-dialog button[value=unlock]').click()`);
    await wait('!document.getElementById("editor").hidden && !document.body.classList.contains("busy")');

    // Short landscape screens and the smallest phones.
    await click('[data-mode=redact]');
    window.setContentSize(812, 375); await pause(400); await idle();
    assert.equal(await noOverflow(), true, 'landscape has no sideways scroll');
    await targets('812 x 375');
    window.setContentSize(320, 700); await pause(400); await idle();
    assert.equal(await noOverflow(), true, 'the Redact tool fits at 320 pt');
    assert.deepEqual(errors, []);
    clearTimeout(watchdog);
    console.log(`PASS: Redact tool at 375 x 812: chip and Done, box drawing, translucent press, underneath tint, undo/redo, select and remove, leave/share/save prompts, list, plain confirmation, new copy opened with banner, original unchanged, secret gone (mupdf), unchanged copy's shared name, Open original, unsaved edits and full storage, R1 cut on a turned cropped page at zoom 2.5 with two-finger scrolling, flatten consent, over-redaction note, unsigned copy of a signed file, encrypted copy without a second prompt, landscape 44 pt targets, 320 pt. Screenshots: ${shots}`);
    app.exit(0);
  } catch (error) { console.error(error); console.error(errors); app.exit(1); }
});
