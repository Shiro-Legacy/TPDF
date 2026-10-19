// Redact tool at phone size (docs/rust-and-ux-plan.md §2.3): draw a box over the visible secret on fixtures/redact.pdf,
// confirm, and check that the copy is saved as a new library document and opened, the original is unchanged, and
// the secret is gone from the copy's text there (read with the mupdf npm API as an independent black box).
// Run after `npm run build` with the preview server on port 4173:
//   ../node_modules/.bin/electron tests/redaction-browser.cjs
// SCREENSHOTS=<dir> also writes the p0m-*.png captures there (docs/ux-audit/after).
const { app, BrowserWindow } = require('electron');
const { mkdtempSync, writeFileSync, readFileSync, mkdirSync } = require('node:fs');
const { tmpdir } = require('node:os');
const { resolve, join } = require('node:path');
const assert = require('node:assert/strict');
app.setPath('userData', mkdtempSync(join(tmpdir(), 'tandem-mobile-redact-')));
const shots = process.env.SCREENSHOTS ? resolve(process.env.SCREENSHOTS) : mkdtempSync(join(tmpdir(), 'tandem-redact-shots-'));
mkdirSync(shots, { recursive: true });
const SECRET = 'P0B-SECRET-7731';
// The line "visible P0B-SECRET-7731 here" (baseline 740 pt up, so about 40 to 56 pt down on the 612 x 792 page).
const BOX = [66, 34, 300, 60];
const overlaps = (a, b) => a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3];
async function linesNear(bytes, rect) {
  const mupdf = await import('mupdf');
  const doc = mupdf.Document.openDocument(bytes, 'application/pdf');
  const text = JSON.parse(doc.loadPage(0).toStructuredText().asJSON());
  return text.blocks.flatMap(block => block.lines ?? [])
    .filter(line => overlaps([line.bbox.x, line.bbox.y, line.bbox.x + line.bbox.w, line.bbox.y + line.bbox.h], rect))
    .map(line => line.text);
}
app.whenReady().then(async () => {
  const window = new BrowserWindow({ width: 375, height: 812, useContentSize: true, show: false, webPreferences: { contextIsolation: true, nodeIntegration: false, sandbox: true } });
  const web = window.webContents;
  const errors = [];
  let step = 'startup';
  const watchdog = setTimeout(() => { console.error('Timed out at', step); app.exit(1); }, 240_000);
  web.on('console-message', (_event, level, message) => { if (level >= 3) errors.push(message); });
  const js = code => web.executeJavaScript(code);
  async function wait(expression) {
    for (let i = 0; i < 200; i++) { if (await js(expression)) return; await new Promise(r => setTimeout(r, 100)); }
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
  /** Page pt (y-down) to window coordinates on the drawn page. */
  async function at(x, y) {
    return js(`(() => { const r = document.getElementById('overlay').getBoundingClientRect(); return [r.left + ${x} * r.width / 612, r.top + ${y} * r.height / 792]; })()`);
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
  async function tap(p) { const c = await at(...p); await mouse('mousePressed', c); await mouse('mouseReleased', c); await idle(); }
  /** The red, green and blue of the boxes layer at a page point. */
  async function shadeAt(x, y) {
    return js(`(() => { const c = document.getElementById('redactions'); const d = c.getContext('2d').getImageData(Math.round(${x} * c.width / 612), Math.round(${y} * c.height / 792), 1, 1).data; return [d[0], d[1], d[2], d[3]]; })()`);
  }
  async function library() {
    return js(`new Promise((resolve, reject) => { const open = indexedDB.open('tandem-mobile', 1); open.onerror = () => reject(open.error); open.onsuccess = () => { const all = open.result.transaction('documents').objectStore('documents').getAll(); all.onsuccess = () => resolve(all.result.map(d => ({ id: d.id, name: d.name, bytes: Array.from(new Uint8Array(d.bytes)) }))); }; })`);
  }
  const noOverflow = () => js('document.documentElement.scrollWidth <= innerWidth');
  try {
    await window.loadURL('http://127.0.0.1:4173'); await idle();
    web.debugger.attach('1.3');
    step = 'import';
    const { root } = await web.debugger.sendCommand('DOM.getDocument');
    const { nodeId } = await web.debugger.sendCommand('DOM.querySelector', { nodeId: root.nodeId, selector: '#picker' });
    await web.debugger.sendCommand('DOM.setFileInputFiles', { nodeId, files: [resolve('../fixtures/redact.pdf')] });
    await wait('!document.getElementById("editor").hidden && !document.body.classList.contains("busy")');
    const original = readFileSync(resolve('../fixtures/redact.pdf'));
    assert.ok((await linesNear(original, BOX)).some(line => line.includes(SECRET)), 'the fixture shows the secret on the boxed line');

    // 1. The tool: a chip says what to do, with a way to stop.
    assert.equal(await js('document.querySelector("[data-mode=redact]").disabled'), false);
    await click('[data-mode=redact]');
    assert.equal(await js('document.getElementById("redact-bar").hidden'), false);
    assert.equal(await js('document.querySelector(".redact-chip span").textContent'), 'Drag over anything you want removed.');
    assert.equal(await js('document.getElementById("redact-start").hidden'), true, 'no Redact button before a box exists');
    assert.equal(await noOverflow(), true, 'the tool fits at 375 pt');
    const targets = await js(`Array.from(document.querySelectorAll('.redact-bar button, .redact-peek, .tools button')).filter(e => e.offsetParent).map(e => { const r = e.getBoundingClientRect(); return [e.textContent.trim(), Math.round(r.width), Math.round(r.height)]; })`);
    for (const [name, w, h] of targets) assert.ok(w >= 44 && h >= 44, `${name} is a 44 pt target (${w} x ${h})`);
    await shot('p0m-1-tool-on.png');

    // 2. Drag a box: translucent while the finger is down, then solid black with a red outline.
    await drag([BOX[0], BOX[1]], [BOX[2], BOX[3]], async () => {
      const [r, g, b, a] = await shadeAt(150, 47);
      assert.ok(a > 0 && a < 200, `the box is translucent while the finger is down (${r},${g},${b},${a})`);
    });
    assert.equal(await text('redact-count'), '1');
    assert.equal(await text('redact-start'), 'Redact 1 area…');
    assert.equal(await text('redact-list'), 'Redactions (1)');
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
    await click('#undo'); assert.equal(await js('document.getElementById("redact-count").hidden'), true);
    await click('#redo'); assert.equal(await text('redact-count'), '1');
    await tap([20, 400]); assert.equal(await js('document.getElementById("redact-remove").hidden'), true, 'a tap elsewhere deselects');
    await tap([150, 47]); assert.equal(await js('document.getElementById("redact-remove").hidden'), false, 'a tap on a box selects it');
    await click('#redact-remove'); assert.equal(await js('document.getElementById("redact-count").hidden'), true);
    await click('#undo'); assert.equal(await text('redact-count'), '1');

    // Leaving with a box asks first; Cancel stays.
    await press('#back', 'redact-leave');
    assert.equal(await text('redact-leave-title'), '1 area is marked but not redacted yet.');
    await answer('redact-leave', 'cancel'); await idle();
    assert.equal(await js('document.getElementById("editor").hidden'), false);

    // 4. The list by page, then the plain confirmation.
    await press('#redact-list', 'redactions-dialog'); await idle();
    assert.ok((await text('redactions-list')).includes('Page 1'));
    assert.ok((await text('redactions-list')).includes(SECRET), 'the list says what the box removes');
    assert.equal(await text('redactions-apply'), 'Redact 1 area…');
    await press('#redactions-apply', 'redact-confirm');
    assert.equal(await text('redact-confirm-title'), 'Permanently remove everything inside 1 box?');
    assert.ok((await text('redact-confirm')).includes('deleted from the file, not just covered'));
    assert.equal(await js('document.getElementById("redact-hidden").checked'), true);
    assert.deepEqual(await js(`Array.from(document.querySelectorAll('#redact-sanitize input')).map(i => i.checked)`), Array(9).fill(true));
    await js(`document.getElementById('redact-hidden').click()`);
    assert.deepEqual(await js(`Array.from(document.querySelectorAll('#redact-sanitize input')).map(i => i.checked)`), Array(9).fill(false), 'the master box turns the nine off');
    await js(`document.getElementById('redact-hidden').click()`);
    await shot('p0m-4-confirm.png');
    await js(`document.getElementById('redact-details').open = true`);
    await shot('p0m-4b-confirm-details.png');
    assert.equal(await js(`(() => { const d = document.getElementById('redact-confirm'), b = document.getElementById('redact-confirm-apply').getBoundingClientRect(), r = d.getBoundingClientRect(); return b.bottom <= r.bottom && b.top >= r.top; })()`), true, 'Redact and save stays in view with Details open');
    assert.equal(await js(`document.getElementById('redact-confirm').scrollWidth <= document.getElementById('redact-confirm').clientWidth`), true);

    // 5. Redact and save: a new library document, opened, with the banner.
    await answer('redact-confirm', 'apply'); await idle();
    await wait('!document.getElementById("redacted-banner").hidden');
    assert.equal(await text('name'), 'redact redacted.pdf');
    assert.equal(await text('redacted-text'), 'You are viewing the redacted copy ‘redact redacted.pdf’. 1 area removed and checked.');
    assert.equal(await js('document.querySelector("[data-mode=read]").getAttribute("aria-pressed")'), 'true');
    assert.equal(await js('document.getElementById("redact-count").hidden'), true, 'the boxes went with the original');
    assert.equal(await noOverflow(), true);
    await shot('p0m-5-banner.png');
    const docs = await library();
    assert.equal(docs.length, 2, 'the copy is a new document beside the original');
    const stored = docs.find(d => d.name === 'redact.pdf'), copy = docs.find(d => d.name === 'redact redacted.pdf');
    assert.ok(Buffer.from(stored.bytes).equals(original), 'the original is unchanged');
    const near = await linesNear(Buffer.from(copy.bytes), BOX);
    assert.ok(!near.some(line => line.includes(SECRET)), `the secret is gone from the boxed line: ${JSON.stringify(near)}`);
    assert.ok(!near.some(line => line.includes('visible')), 'everything inside the box went');
    assert.ok((await linesNear(Buffer.from(copy.bytes), [66, 85, 300, 100])).some(line => line.includes('invisible')), 'text outside the box stays');

    // Open original from the banner.
    await click('#open-original');
    assert.equal(await text('name'), 'redact.pdf');
    assert.equal(await js('document.getElementById("redacted-banner").hidden'), true);

    // Esc stops the tool; boxes stay pending, and Discard boxes lets the user leave.
    await click('[data-mode=redact]');
    await drag([300, 300], [400, 360]);
    web.sendInputEvent({ type: 'keyDown', keyCode: 'Escape' }); web.sendInputEvent({ type: 'keyUp', keyCode: 'Escape' });
    await wait('document.getElementById("redact-bar").hidden');
    assert.equal(await js('document.querySelector("[data-mode=read]").getAttribute("aria-pressed")'), 'true');
    assert.equal(await text('redact-count'), '1');
    await press('#back', 'redact-leave');
    await answer('redact-leave', 'discard'); await idle();
    assert.equal(await js('document.getElementById("library").hidden'), false);
    assert.equal((await library()).length, 2, 'discarding writes nothing');

    // The smallest phones keep the tool bar on one row.
    window.setContentSize(320, 700); await new Promise(r => setTimeout(r, 350));
    await click('.file-open'); await click('[data-mode=redact]');
    assert.equal(await noOverflow(), true, 'the Redact tool fits at 320 pt');
    assert.deepEqual(errors, []);
    clearTimeout(watchdog);
    console.log(`PASS: Redact tool at 375 x 812: chip and Done, box drawing, translucent press, underneath tint, undo/redo, select and remove, leave prompt, list, plain confirmation, new copy opened with banner, original unchanged, secret gone (mupdf), Open original, Esc, discard. Screenshots: ${shots}`);
    app.exit(0);
  } catch (error) { console.error(error); console.error(errors); app.exit(1); }
});
