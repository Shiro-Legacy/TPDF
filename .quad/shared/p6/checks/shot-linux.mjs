#!/usr/bin/env node
/**
 * Linux port GUI evidence (shot-windows.mjs with the Linux binary): the packaged app in its real (shown) window.
 * Starts it the way the .desktop entry does (a PDF as a plain argument, relative to the cwd), then a second instance
 * with another PDF, and reads the page over CDP: the opened document, Ctrl/Alt wording, the reserved scrollbar
 * gutter; under X11 also the window class the .desktop entry's StartupWMClass names. Screenshots → review-out/linux-*.png.
 * Not part of npm test: it shows a window for a few seconds. On a virtual display nothing reaches the human's screen:
 *   env -u WAYLAND_DISPLAY XDG_SESSION_TYPE=x11 xvfb-run -a -s "-screen 0 1920x1200x24" node .quad/shared/p6/checks/shot-linux.mjs [--source]
 * Run after `npm run package` (--source: `electron .` from the repo).
 */
import { spawn, spawnSync } from 'node:child_process';
import { createServer } from 'node:net';
import { mkdir, rm, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';

const root = fileURLToPath(new URL('../../../../', import.meta.url));
const out = join(fileURLToPath(new URL('./', import.meta.url)), 'review-out');
const udata = join(out, 'linux-udata');
const [bin, entry] = process.argv.includes('--source')
  ? [createRequire(join(root, 'package.json'))('electron'), ['.']]
  : [join(root, 'out/Tandem PDF-linux-x64/Tandem PDF'), []];
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
const app = launch('fixtures/hello.pdf');
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
  const shot = async name => writeFile(join(out, `linux-${name}.png`), Buffer.from((await cdp('Page.captureScreenshot')).data, 'base64'));
  const docName = () => evaluate('window.pdf4allState().doc?.name');

  check('first instance opens its PDF argument', await wait(async () => await docName() === 'hello.pdf', 'hello.pdf open').catch(() => false), await docName());
  const page = await evaluate(`(() => {
    const text = document.body.textContent, viewport = document.getElementById('viewport');
    return { mac: (text.match(/[⌘⌥⇧⌫]|this Mac|Cmd\\//g) ?? []).length, ctrl: (text.match(/Ctrl\\+/g) ?? []).length,
      gutter: getComputedStyle(viewport).scrollbarGutter, driveGeometry: document.documentElement.classList.contains('drive-geometry'),
      inner: [innerWidth, innerHeight], visible: document.visibilityState };
  })()`);
  check('no macOS key glyphs in the page', page.mac === 0 && page.ctrl > 0, JSON.stringify(page));
  check('scrollbar gutter reserved, real scrollbars kept', page.gutter.startsWith('stable') && !page.driveGeometry);
  check('window content is 1360x860 and shown', page.inner[0] === 1360 && page.inner[1] === 860 && page.visible === 'visible', JSON.stringify(page.inner));
  await shot('open');
  // X11 only (a Wayland shell pairs the window through the app id, package.json desktopName, instead).
  const tree = spawnSync('xwininfo', ['-root', '-tree'], { encoding: 'utf8' });
  if (tree.status === 0) check('window class is tandem-pdf (StartupWMClass)', /hello\.pdf[^\n]*\("tandem-pdf" "tandem-pdf"\)/.test(tree.stdout));
  else console.log('skip window class (no xwininfo or no X display)');

  const second = launch('fixtures/form.pdf');
  const code = await new Promise(resolve => second.on('close', resolve));
  check('second instance hands over and exits', code === 3, `exit ${code}`);
  check('first instance opens the second PDF', await wait(async () => await docName() === 'form.pdf', 'form.pdf open').catch(() => false), await docName());
  await shot('second');
  ws.close();
} finally {
  app.kill();
  await new Promise(resolve => app.on('close', resolve));
  await rm(udata, { recursive: true, force: true, maxRetries: 5 }).catch(() => {});
}
console.log(failed ? `${failed} FAILED` : 'all ok', `(screenshots in ${out})`);
process.exit(failed ? 1 : 0);
