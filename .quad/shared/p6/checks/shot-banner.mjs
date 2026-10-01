#!/usr/bin/env node
/**
 * The document notice bar ("Detected: …") can be closed: a real click on its × hides it, focus goes to the page,
 * the bar returns when another document opens and again when the first one is reopened.
 * Real (shown) window, read over CDP; screenshots → review-out/banner-*.png. Not part of npm test.
 * Linux, off the human's screen:
 *   env -u WAYLAND_DISPLAY XDG_SESSION_TYPE=x11 xvfb-run -a -s "-screen 0 1920x1200x24" node .quad/shared/p6/checks/shot-banner.mjs [<app binary>]
 * With no argument it runs `electron .` from the repo (needs `npm run build`).
 */
import { spawn } from 'node:child_process';
import { createServer } from 'node:net';
import { mkdir, rm, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';

const root = fileURLToPath(new URL('../../../../', import.meta.url));
const out = join(fileURLToPath(new URL('./', import.meta.url)), 'review-out');
const udata = join(out, 'banner-udata');
const [bin, entry] = process.argv[2] ? [process.argv[2], []] : [createRequire(join(root, 'package.json'))('electron'), ['.']];
const sleep = ms => new Promise(r => setTimeout(r, ms));
const port = await new Promise((resolve, reject) => {
  const s = createServer();
  s.listen(0, '127.0.0.1', () => { const { port } = s.address(); s.close(err => err ? reject(err) : resolve(port)); });
});
async function wait(fn, what, ms = 25_000) {
  const t0 = Date.now();
  let last;
  while (Date.now() - t0 < ms) {
    try { last = await fn(); if (last) return last; } catch (err) { last = err.message; }
    await sleep(200);
  }
  throw new Error(`timeout: ${what} (${JSON.stringify(last)})`);
}

await rm(udata, { recursive: true, force: true, maxRetries: 5 });
await mkdir(out, { recursive: true });
const launch = pdf => spawn(bin, [`--user-data-dir=${udata}`, `--remote-debugging-port=${port}`, ...entry, pdf], { cwd: root, stdio: 'ignore' });
const SIGNED = 'fixtures/p0b/signed-intact.pdf';
const app = launch(SIGNED);
let failed = 0;
const check = (name, ok, detail = '') => { if (!ok) failed++; console.log(ok ? 'ok  ' : 'FAIL', name, detail); };
try {
  const target = await wait(async () => (await (await fetch(`http://127.0.0.1:${port}/json`)).json()).find(t => t.type === 'page'), 'CDP page');
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
  const pending = new Map(); let id = 0;
  ws.onmessage = event => { const msg = JSON.parse(event.data); pending.get(msg.id)?.(msg.result); pending.delete(msg.id); };
  const cdp = (method, params = {}) => new Promise(resolve => { pending.set(++id, resolve); ws.send(JSON.stringify({ id, method, params })); });
  const evaluate = async expression => (await cdp('Runtime.evaluate', { expression, returnByValue: true })).result?.value;
  const shot = async name => writeFile(join(out, `banner-${name}.png`), Buffer.from((await cdp('Page.captureScreenshot')).data, 'base64'));
  const docName = () => evaluate('window.pdf4allState().doc?.name');
  const bar = () => evaluate(`(() => {
    const bar = document.getElementById('banner'), button = document.getElementById('banner-dismiss'), r = button.getBoundingClientRect();
    return { hidden: bar.hidden, text: document.getElementById('banner-text').textContent, label: button.getAttribute('aria-label'),
      at: [r.x + r.width / 2, r.y + r.height / 2], pageTop: document.getElementById('viewport').getBoundingClientRect().top, focus: document.activeElement?.id };
  })()`);
  const secondInstance = async (pdf, name) => {
    const code = await new Promise(resolve => launch(pdf).on('close', resolve));
    await wait(async () => await docName() === name, `${name} open`);
    return code;
  };

  await wait(async () => await docName() === 'signed-intact.pdf', 'signed-intact.pdf open');
  const shown = await wait(async () => { const b = await bar(); return !b.hidden && b; }, 'notice shown');
  check('the notice shows with a labelled close button', /signature field/.test(shown.text) && shown.label === 'Dismiss this notice', JSON.stringify(shown.text));
  await shot('shown');
  for (const type of ['mousePressed', 'mouseReleased']) await cdp('Input.dispatchMouseEvent', { type, x: shown.at[0], y: shown.at[1], button: 'left', clickCount: 1 });
  const closed = await wait(async () => { const b = await bar(); return b.hidden && b; }, 'notice hidden after the click');
  check('a real click on × hides it and gives its height back to the page', closed.hidden && closed.pageTop < shown.pageTop, `page top ${shown.pageTop} → ${closed.pageTop}`);
  check('focus moves to the page', closed.focus === 'page', String(closed.focus));
  await shot('closed');

  check('second instance hands over', await secondInstance('fixtures/encrypted.pdf', 'encrypted.pdf') === 3);
  const other = await wait(async () => { const b = await bar(); return !b.hidden && b; }, 'notice for the next document');
  check('another document shows its own notice', /encrypted/.test(other.text), JSON.stringify(other.text));
  await secondInstance(SIGNED, 'signed-intact.pdf');
  const again = await wait(async () => { const b = await bar(); return !b.hidden && b; }, 'notice back on reopen');
  check('reopening the first document shows its notice again', again.text === shown.text, JSON.stringify(again.text));
  ws.close();
} finally {
  app.kill();
  await new Promise(resolve => app.on('close', resolve));
  await rm(udata, { recursive: true, force: true, maxRetries: 5 }).catch(() => {});
}
console.log(failed ? `${failed} FAILED` : 'all ok', `(screenshots in ${out})`);
process.exit(failed ? 1 : 0);
