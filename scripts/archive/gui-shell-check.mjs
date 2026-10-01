#!/usr/bin/env node
/**
 * Real-app CDP check for the design-3 shell. Not part of npm test.
 * Usage: node scripts/gui-shell-check.mjs
 */
import { spawn } from 'node:child_process';
import { createServer } from 'node:net';
import { mkdir, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';

const root = fileURLToPath(new URL('..', import.meta.url));
// The real Electron binary, not the node shim in node_modules/.bin (the shim forwards
// only SIGINT/SIGTERM, so SIGKILL would leave the real process running).
const electron = createRequire(import.meta.url)('electron');
const outDir = join(root, '.quad/shared/ui-shell');
const pdf = join(root, 'fixtures/form2.pdf');
const errors = [];

function freePort() {
  return new Promise((resolve, reject) => {
    const s = createServer();
    s.listen(0, '127.0.0.1', () => {
      const port = s.address().port;
      s.close(err => err ? reject(err) : resolve(port));
    });
    s.on('error', reject);
  });
}

function sleep(ms) {
  return new Promise(r => setTimeout(r, ms));
}

async function wait(fn, ms = 20_000, step = 150) {
  const t0 = Date.now();
  let last;
  while (Date.now() - t0 < ms) {
    try { last = await fn(); } catch (err) { last = err; }
    if (last && last !== false && !(last instanceof Error)) return last;
    await sleep(step);
  }
  throw new Error(`timeout: ${last instanceof Error ? last.message : last}`);
}

class Cdp {
  constructor(url) {
    this.url = url;
    this.n = 0;
    this.pending = new Map();
    this.pageErrors = [];
  }
  async connect() {
    this.ws = new WebSocket(this.url);
    await new Promise((resolve, reject) => {
      this.ws.addEventListener('open', resolve, { once: true });
      this.ws.addEventListener('error', reject, { once: true });
    });
    this.ws.addEventListener('message', ev => {
      const msg = JSON.parse(String(ev.data));
      if (msg.method === 'Runtime.exceptionThrown') {
        const text = msg.params?.exceptionDetails?.text || msg.params?.exceptionDetails?.exception?.description || 'exception';
        if (!/sandboxed_renderer|preloadScripts/.test(text)) this.pageErrors.push(text);
      }
      if (msg.method === 'Runtime.consoleAPICalled' && msg.params?.type === 'error') {
        const text = (msg.params.args || []).map(a => a.value ?? a.description ?? '').join(' ');
        if (text && !text.startsWith('P0_') && !text.startsWith('P1_') && !text.startsWith('P2_') && !/sandboxed_renderer|preloadScripts/.test(text)) {
          this.pageErrors.push(text);
        }
      }
      if (msg.id && this.pending.has(msg.id)) {
        const { resolve, reject } = this.pending.get(msg.id);
        this.pending.delete(msg.id);
        if (msg.error) reject(new Error(msg.error.message));
        else resolve(msg.result);
      }
    });
  }
  send(method, params = {}) {
    const id = ++this.n;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`cdp timeout ${method}`));
      }, 15_000);
      this.pending.set(id, {
        resolve: v => { clearTimeout(timer); resolve(v); },
        reject: e => { clearTimeout(timer); reject(e); },
      });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }
  async eval(expression) {
    const r = await this.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.text || expression);
    return r.result?.value;
  }
  click(id) {
    return this.eval(`document.getElementById(${JSON.stringify(id)}).click()`);
  }
  close() {
    try { this.ws.close(); } catch { /* already closed */ }
  }
}

async function jsonGet(port, path) {
  const res = await fetch(`http://127.0.0.1:${port}${path}`);
  if (!res.ok) throw new Error(`${path} ${res.status}`);
  return res.json();
}

function check(cond, msg) {
  if (!cond) throw new Error(msg);
  console.log(`ok  ${msg}`);
}

async function shot(cdp, name) {
  const { data } = await cdp.send('Page.captureScreenshot', { format: 'png' });
  const file = join(outDir, name);
  await writeFile(file, Buffer.from(data, 'base64'));
  console.log(`ok  screenshot ${file}`);
}

async function overflow(cdp, label) {
  const info = await cdp.eval(`({
    sw: document.documentElement.scrollWidth,
    cw: document.documentElement.clientWidth,
    canvas: document.getElementById('canvas')?.width || 0,
    vw: document.getElementById('viewport')?.clientWidth || 0,
  })`);
  check(info.sw <= info.cw + 1, `${label} no horizontal overflow (${info.sw}<=${info.cw + 1})`);
  check(info.canvas > 0, `${label} canvas painted (${info.canvas})`);
}

const CHROME_IDS = ['open', 'save-copy', 'mode-read', 'mode-annotate', 'mode-organize', 'mode-forms', 'inspector-toggle', 'fit-page', 'thumbs-toggle', 'search-input'];

async function notClipped(cdp, label) {
  const bad = await cdp.eval(`(() => {
    const vw = document.documentElement.clientWidth, vh = document.documentElement.clientHeight;
    const bad = [];
    for (const id of ${JSON.stringify(CHROME_IDS)}) {
      const el = document.getElementById(id) || document.querySelector('label[for="' + id + '"]');
      if (!el) { bad.push(id + ':missing'); continue; }
      const r = el.getBoundingClientRect();
      if (r.width < 1 || r.height < 1) bad.push(id + ':zero');
      else if (r.right > vw + 2 || r.bottom > vh + 2 || r.left < -2 || r.top < -2) bad.push(id + ':clip');
    }
    return bad;
  })()`);
  check(bad.length === 0, `${label} chrome in view (${bad.join(',') || 'ok'})`);
}

async function tabsFit(cdp, label) {
  const bad = await cdp.eval(`(() => [...document.querySelectorAll('.tabs')]
    .map((el, i) => ({ i, sw: el.scrollWidth, cw: el.clientWidth }))
    .filter(t => t.sw > t.cw + 1))()`);
  check(bad.length === 0, `${label} .tabs fit (${bad.map(t => t.i + ':' + t.sw + '>' + t.cw).join(',') || 'ok'})`);
}

async function resize(cdp, w, h) {
  try {
    const { windowId } = await cdp.send('Browser.getWindowForTarget');
    await cdp.send('Browser.setWindowBounds', { windowId, bounds: { width: w, height: h, windowState: 'normal' } });
  } catch {
    await cdp.eval(`window.resizeTo(${w}, ${h})`);
  }
  await cdp.send('Emulation.setDeviceMetricsOverride', {
    width: w, height: h, deviceScaleFactor: 1, mobile: false,
  });
  await sleep(400);
}

async function main() {
  await mkdir(outDir, { recursive: true });
  const port = await freePort();
  const child = spawn(electron, [
    `--remote-debugging-port=${port}`,
    '--remote-allow-origins=*',
    '.',
    `--open=${pdf}`,
  ], { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
  let stderr = '';
  child.stderr.setEncoding('utf8');
  child.stderr.on('data', chunk => { stderr += chunk; });
  let cdp;
  try {
    const page = await wait(async () => {
      const list = await jsonGet(port, '/json/list').catch(() => jsonGet(port, '/json'));
      return list.find(t => t.webSocketDebuggerUrl && (t.type === 'page' || /index\.html/.test(t.url || '')));
    }, 25_000);
    cdp = new Cdp(page.webSocketDebuggerUrl);
    await cdp.connect();
    await cdp.send('Runtime.enable');
    await cdp.send('Page.enable');
    await wait(() => cdp.eval(`document.getElementById('canvas')?.width > 0`));
    await wait(() => cdp.eval(`(() => {
      const t = (document.title||'') + (document.getElementById('document-title')?.textContent||'') + (document.getElementById('status')?.textContent||'');
      return /form2/i.test(t);
    })()`), 20_000);

    await resize(cdp, 1440, 900);
    await cdp.click('fit-page');
    await sleep(400);
    await overflow(cdp, '1440x900');
    await notClipped(cdp, '1440x900');
    await tabsFit(cdp, '1440x900');
    await shot(cdp, '1440x900.png');

    for (const id of ['mode-read', 'mode-annotate', 'mode-organize', 'mode-forms']) {
      await cdp.click(id);
      const on = await cdp.eval(`document.getElementById(${JSON.stringify(id)}).checked === true`);
      check(on, `${id} selected`);
    }

    await cdp.eval(`document.getElementById('mode-read').focus()`);
    await cdp.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'ArrowRight', code: 'ArrowRight', windowsVirtualKeyCode: 39 });
    await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'ArrowRight', code: 'ArrowRight', windowsVirtualKeyCode: 39 });
    check(await cdp.eval(`document.getElementById('mode-annotate').checked === true`), 'ArrowRight selects Annotate');

    await wait(() => cdp.eval(`!document.getElementById('note-add').disabled`));
    await cdp.eval(`document.getElementById('note-add').dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 1 }))`);
    check(await cdp.eval(`document.getElementById('note-add').getAttribute('aria-pressed') === 'true'`), 'note armed');
    await cdp.click('mode-read');
    check(await cdp.eval(`document.getElementById('note-add').getAttribute('aria-pressed') === 'false'`), 'leaving Annotate cancels armed note');
    await cdp.click('mode-annotate');
    await cdp.eval(`document.getElementById('ink-draw').dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 1 }))`);
    check(await cdp.eval(`document.getElementById('ink-draw').getAttribute('aria-pressed') === 'true'`), 'ink armed');
    await cdp.click('mode-organize');
    check(await cdp.eval(`document.getElementById('ink-draw').getAttribute('aria-pressed') === 'false'`), 'leaving Annotate cancels armed ink');
    await cdp.click('mode-annotate');

    for (const [tab, panel] of [['tab-pages', 'panel-pages'], ['tab-bookmarks', 'panel-bookmarks']]) {
      await cdp.click(tab);
      const v = await cdp.eval(`({ sel: document.getElementById(${JSON.stringify(tab)}).getAttribute('aria-selected') === 'true', shown: !document.getElementById(${JSON.stringify(panel)}).hidden })`);
      check(v.sel && v.shown, `${tab} shows ${panel}`);
    }
    await cdp.click('tab-pages');
    await cdp.eval(`document.getElementById('tab-pages').focus()`);
    await cdp.eval(`document.getElementById('tab-pages').dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }))`);
    check(await cdp.eval(`document.getElementById('tab-bookmarks').getAttribute('aria-selected') === 'true' && !document.getElementById('panel-bookmarks').hidden`), 'sidebar ArrowRight shows bookmarks');
    await cdp.click('tab-pages');

    for (const [tab, panel] of [['tab-notes', 'panel-notes'], ['tab-fields', 'panel-fields'], ['tab-links', 'panel-links'], ['tab-text', 'text-panel'], ['tab-layers', 'panel-layers']]) {
      await cdp.click(tab);
      const v = await cdp.eval(`({ sel: document.getElementById(${JSON.stringify(tab)}).getAttribute('aria-selected') === 'true', shown: !document.getElementById(${JSON.stringify(panel)}).hidden })`);
      check(v.sel && v.shown, `${tab} shows ${panel}`);
    }
    await cdp.eval(`document.getElementById('tab-notes').click(); document.getElementById('tab-notes').focus(); document.getElementById('tab-notes').dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }))`);
    check(await cdp.eval(`document.getElementById('tab-fields').getAttribute('aria-selected') === 'true' && !document.getElementById('panel-fields').hidden`), 'inspector ArrowRight shows fields');
    await cdp.click('tab-notes');

    await wait(() => cdp.eval(`!document.getElementById('thumbs-toggle').disabled`));
    const thumbsBefore = await cdp.eval(`document.getElementById('thumbs-toggle').getAttribute('aria-pressed')`);
    await cdp.click('thumbs-toggle');
    const thumbsAfter = await cdp.eval(`document.getElementById('thumbs-toggle').getAttribute('aria-pressed')`);
    check(thumbsBefore !== thumbsAfter, 'thumbs-toggle flips');
    await cdp.click('thumbs-toggle');

    const inspBefore = await cdp.eval(`document.getElementById('inspector-toggle').getAttribute('aria-expanded')`);
    await cdp.click('inspector-toggle');
    const inspAfter = await cdp.eval(`document.getElementById('inspector-toggle').getAttribute('aria-expanded')`);
    check(inspBefore !== inspAfter, 'inspector-toggle flips');
    await cdp.click('inspector-toggle');

    await cdp.eval(`(() => {
      const i = document.getElementById('search-input');
      i.focus();
      i.value = 'PDF4All';
      i.dispatchEvent(new Event('input', { bubbles: true }));
    })()`);
    await wait(() => cdp.eval(`document.querySelectorAll('.hit').length > 0`));
    check(true, 'search hits');
    await wait(() => cdp.eval(`!document.getElementById('markup-highlight').disabled`));
    await cdp.click('markup-highlight');
    await wait(() => cdp.eval(`(document.getElementById('edit-status')?.textContent||'').toLowerCase().includes('highlight') || (document.getElementById('note-list')?.textContent||'').toLowerCase().includes('highlight')`));
    check(true, 'highlight match');
    await shot(cdp, '1440x900-annotate.png');

    const notesBefore = await cdp.eval(`document.querySelectorAll('#note-list li').length`);
    await wait(() => cdp.eval(`!document.getElementById('note-add').disabled`));
    await cdp.click('note-add');
    await wait(() => cdp.eval(`document.querySelectorAll('#note-list li').length > ${notesBefore}`));
    check(true, 'note added');
    await cdp.click('undo');
    await wait(() => cdp.eval(`document.querySelectorAll('#note-list li').length <= ${notesBefore}`));
    check(true, 'note undone');

    await cdp.click('mode-forms');
    await cdp.click('tab-fields');
    await wait(() => cdp.eval(`document.querySelectorAll('#field-list input, #field-list textarea, #field-list select').length > 0`));
    const fieldOk = await cdp.eval(`(() => {
      const el = document.querySelector('#field-list input, #field-list textarea, #field-list select');
      return !!(el && !el.disabled && el.getBoundingClientRect().height > 0);
    })()`);
    check(fieldOk, 'form field reachable');

    await cdp.click('print');
    await wait(() => cdp.eval(`document.getElementById('print-dialog').open`));
    check(true, 'print dialog visible');
    await cdp.click('print-cancel');
    await wait(() => cdp.eval(`!document.getElementById('print-dialog').open`));

    await resize(cdp, 900, 600);
    await overflow(cdp, '900x600');
    await notClipped(cdp, '900x600');
    await tabsFit(cdp, '900x600');
    await shot(cdp, '900x600.png');

    const pageErrors = cdp.pageErrors.filter(e => !/ResizeObserver|P0_|P1_|P2_|sandboxed_renderer|preloadScripts/.test(e));
    check(pageErrors.length === 0, `no page errors (${pageErrors.join('; ') || 'none'})`);
    console.log('PASS');
  } catch (err) {
    errors.push(err.message);
    console.error(`not ok  ${err.message}`);
    if (stderr.trim()) console.error(stderr.slice(-2000));
    console.log('FAIL');
    process.exitCode = 1;
  } finally {
    cdp?.close();
    try { child.kill('SIGKILL'); } catch { /* gone */ }
    await Promise.race([new Promise(r => child.on('close', r)), sleep(2000)]);
  }
}

main().catch(err => {
  console.error(err);
  process.exit(1);
});
