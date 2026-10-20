// Run after `npm run build` with the preview server on port 4173:
// node_modules/.bin/electron tests/browser.cjs
const { app, BrowserWindow } = require('electron');
const { mkdtempSync, writeFileSync, readFileSync } = require('node:fs');
const { tmpdir } = require('node:os');
const { resolve, join } = require('node:path');
const assert = require('node:assert/strict');
app.setPath('userData', mkdtempSync(join(tmpdir(), 'tandem-mobile-ui-')));
app.whenReady().then(async () => {
  const window = new BrowserWindow({ width: 390, height: 844, show: false, webPreferences: { contextIsolation: true, nodeIntegration: false, sandbox: true } });
  const web = window.webContents;
  const errors = [];
  web.on('console-message', (_event, level, message) => { if (level >= 3) errors.push(message); });
  const js = code => web.executeJavaScript(code);
  async function wait(expression) {
    for (let i = 0; i < 150; i++) { if (await js(expression)) return; await new Promise(r => setTimeout(r, 100)); }
    throw new Error(`Timed out: ${expression}\n${await js('document.body.innerText')}`);
  }
  async function idle() { await wait('!document.body.classList.contains("busy")'); }
  async function click(selector) { await js(`document.querySelector(${JSON.stringify(selector)}).click()`); await idle(); }
  async function importFixture(name, password) {
    const { root } = await web.debugger.sendCommand('DOM.getDocument');
    const { nodeId } = await web.debugger.sendCommand('DOM.querySelector', { nodeId: root.nodeId, selector: '#picker' });
    await web.debugger.sendCommand('DOM.setFileInputFiles', { nodeId, files: [resolve('../fixtures', name)] });
    if (password) {
      await wait('document.getElementById("password-dialog").open');
      await js(`document.getElementById('password').value='incorrect'; document.querySelector('#password-dialog button[value=unlock]').click()`);
      await wait('document.getElementById("password-dialog").open && document.getElementById("password").value === ""');
      await js(`document.getElementById('password').value=${JSON.stringify(password)}; document.querySelector('#password-dialog button[value=unlock]').click()`);
    }
    await wait('!document.getElementById("editor").hidden && !document.body.classList.contains("busy")');
    assert.ok(await js('document.getElementById("page").width > 0'));
  }
  try {
    await window.loadURL('http://127.0.0.1:4173'); await idle();
    assert.equal(await js('document.documentElement.scrollWidth <= innerWidth'), true, 'no horizontal overflow');
    writeFileSync(join(tmpdir(), 'tandem-mobile-library.png'), (await web.capturePage()).toPNG());
    web.debugger.attach('1.3');
    await importFixture('form.pdf');
    await click('#forms');
    await js(`(() => { const input = document.querySelector('#field-list input:not([type=checkbox]):not([type=radio])'); input.value = 'Mobile UI verified'; input.dispatchEvent(new Event('change')); })()`);
    await idle();
    await click('[data-close="fields-dialog"]');
    await click('[data-mode="note"]');
    await js(`(() => { const c=document.getElementById('overlay'),r=c.getBoundingClientRect(); for (const type of ['pointerdown','pointerup']) c.dispatchEvent(new PointerEvent(type,{pointerId:1,clientX:r.left+50,clientY:r.top+60})); })()`);
    await wait('document.getElementById("note-dialog").open');
    await js(`document.getElementById('note-text').value='Note from phone-sized UI'; document.querySelector('#note-dialog button[value=add]').click()`);
    await idle();
    await click('#sign');
    await js(`(() => { const c=document.getElementById('signature'),r=c.getBoundingClientRect(); [['pointerdown',20,60],['pointermove',70,30],['pointermove',130,90],['pointerup',160,60]].forEach(([type,x,y])=>c.dispatchEvent(new PointerEvent(type,{pointerId:1,clientX:r.left+x,clientY:r.top+y}))); })()`);
    await click('#use-signature');
    await js(`(() => { const c=document.getElementById('overlay'),r=c.getBoundingClientRect(); for (const type of ['pointerdown','pointerup']) c.dispatchEvent(new PointerEvent(type,{pointerId:1,clientX:r.left+170,clientY:r.top+210})); })()`);
    await idle();
    await click('#undo'); await click('#redo');
    assert.equal(await js('document.getElementById("save-state").textContent'), 'Saved on device');
    writeFileSync(join(tmpdir(), 'tandem-mobile-editor.png'), (await web.capturePage()).toPNG());
    await click('#back');
    await click('.file-open'); await click('#forms');
    assert.ok(await js(`Array.from(document.querySelectorAll('#field-list input')).some(i=>i.value==='Mobile UI verified')`));
    await click('[data-close="fields-dialog"]');
    // Reload from persistent storage, proving the workflow survives app restart.
    await window.loadURL('http://127.0.0.1:4173'); await idle(); await click('.file-open');
    await click('#forms');
    assert.ok(await js(`Array.from(document.querySelectorAll('#field-list input')).some(i=>i.value==='Mobile UI verified')`));
    await click('[data-close="fields-dialog"]');
    await click('#menu'); await click('#find');
    await js(`document.getElementById('search-query').value='PDF4All'; document.getElementById('search-form').requestSubmit()`);
    await idle(); assert.ok(await js('document.querySelectorAll(".search-result").length > 0'));
    await click('.search-result');
    await click('#menu'); await click('#annotations');
    assert.ok(await js(`document.getElementById('annotation-list').textContent.includes('Note from phone-sized UI')`));
    assert.ok(await js(`document.getElementById('annotation-list').textContent.includes('Drawn signature')`));
    await click('[data-close="annotations-dialog"]');
    // A quota failure leaves edits dirty and the durable copy unchanged.
    await js(`void (() => { window.realTransaction = IDBDatabase.prototype.transaction; IDBDatabase.prototype.transaction = function(...args) { if(args[1] === 'readwrite') throw new DOMException('Full','QuotaExceededError'); return window.realTransaction.apply(this,args); }; })()`);
    await click('#forms');
    await js(`(() => { const input = document.querySelector('#field-list input:not([type=checkbox]):not([type=radio])'); input.value='Unsaved due to quota'; input.dispatchEvent(new Event('change')); })()`);
    await idle();
    assert.equal(await js('document.getElementById("save-state").textContent'), 'Unsaved changes');
    assert.ok(await js(`document.getElementById('status').textContent.includes('could not be saved')`));
    await js('void (IDBDatabase.prototype.transaction = window.realTransaction)');
    await click('[data-close="fields-dialog"]'); await click('#menu'); await click('#save');
    assert.equal(await js('document.getElementById("save-state").textContent'), 'Saved on device');
    // Small Android phones and landscape must keep all tools reachable.
    window.setSize(320, 700); await new Promise(r => setTimeout(r, 350)); await idle();
    assert.equal(await js('document.documentElement.scrollWidth <= innerWidth'), true, '320px editor has no horizontal overflow');
    window.setSize(844, 390); await new Promise(r => setTimeout(r, 350)); await idle();
    assert.equal(await js('document.documentElement.scrollWidth <= innerWidth'), true, 'landscape has no horizontal overflow');
    window.setSize(390, 844); await new Promise(r => setTimeout(r, 350)); await idle();
    await click('#back');
    await js(`void (IDBDatabase.prototype.transaction = function(...args) { if(args[1] === 'readwrite') throw new DOMException('Full','QuotaExceededError'); return window.realTransaction.apply(this,args); })`);
    await importFixture('p5/signed-ec.pdf');
    assert.equal(await js('document.getElementById("save-state").textContent'), 'Not saved on device');
    const exportPath = join(tmpdir(), 'tandem-signed-export.pdf');
    const download = new Promise((resolveDownload, rejectDownload) => {
      web.session.once('will-download', (_event, item) => {
        item.setSavePath(exportPath);
        item.once('done', (_event, state) => state === 'completed' ? resolveDownload() : rejectDownload(new Error(state)));
      });
    });
    await click('#share'); await Promise.race([download, new Promise((_, reject) => setTimeout(() => reject(new Error('Export did not download')), 10_000))]);
    assert.deepEqual(readFileSync(exportPath), readFileSync(resolve('../fixtures/p5/signed-ec.pdf')), 'signed originals export byte-for-byte even when local storage fails');
    await js('void (IDBDatabase.prototype.transaction = window.realTransaction)');
    await click('#menu'); await click('#save');
    assert.equal(await js('document.getElementById("save-state").textContent'), 'Saved on device');
    await click('#back');
    await click('.privacy a');
    assert.ok(await js('document.getElementById("license-dialog").open'));
    await click('[data-close="license-dialog"]');
    await importFixture('encrypted.pdf', 'pdf4all-user');
    assert.equal(await js('document.getElementById("password").value'), '');
    assert.deepEqual(errors, []);
    console.log('PASS: responsive UI, worker render, forms, notes, signatures, undo/redo, autosave/restart, search, storage recovery, signed byte-preserving export, and password retry/unlock.');
    app.exit(0);
  } catch (error) { console.error(error); console.error(errors); app.exit(1); }
});
