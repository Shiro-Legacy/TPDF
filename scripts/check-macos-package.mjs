#!/usr/bin/env node
// M10: the macOS package checklist, run against a packaged app (engine/design/m10-macos-package.md).
// Usage: node scripts/check-macos-package.mjs --work=<dir> [--app=<Tandem PDF.app>] [--report=<path>] [--only=<item,...>]
// Items: startup, assets, dialogs, recovery, m9, dpi, attribution (and always wasm-paths). Needs `npm run package:rust` on darwin-arm64 and,
// for the dev-build comparisons, the `npm run build` the package was made from (build/ in this repository).
// Every launch gets its own userData under --work; logs, reports and outputs stay there (the caller deletes it).
// Prints the report and exits 0 iff no item failed. Native dialog UI is never shown: the drive and smoke
// arguments answer the pickers (`to`/`--save-to`), as everywhere else in the app's tests.
import { spawn, spawnSync } from 'node:child_process';
import { createHash, randomBytes } from 'node:crypto';
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { homedir } from 'node:os';
import { join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { inflateSync } from 'node:zlib';
import * as asar from '@electron/asar';
import { CJK_FONT } from './cjk-font.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const args = process.argv.slice(2);
const option = name => args.find(arg => arg.startsWith(`--${name}=`))?.slice(name.length + 3);
const manifest = JSON.parse(await readFile(join(root, 'package.json'), 'utf8'));
const appPath = resolve(root, option('app') ?? `out/${manifest.productName}-darwin-arm64/${manifest.productName}.app`);
const binary = join(appPath, 'Contents/MacOS', manifest.productName);
const resources = join(appPath, 'Contents/Resources');
const archive = join(resources, 'app.asar');
if (!option('work')) { console.error('--work=<dir> is required (logs, userData and outputs go there)'); process.exit(2); }
const work = resolve(option('work'));
const only = option('only')?.split(',');
if (!existsSync(binary) || !existsSync(archive)) { console.error(`no packaged app at ${appPath}`); process.exit(2); }
await mkdir(work, { recursive: true });
const electron = createRequire(import.meta.url)('electron');
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const median = values => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];

/** One app launch (packaged unless `dev`), its stdout+stderr kept as `<name>.log` and its `--report` read back. */
async function launch(name, appArgs, { udata, dev = false, binaryPath = binary, timeoutMs = 180_000, env = {} } = {}) {
  const report = join(work, `${name}.json`);
  const started = Date.now();
  const child = spawn(dev ? electron : binaryPath, [
    `--user-data-dir=${udata ?? join(work, `udata-${name}`)}`, ...(dev ? ['.'] : []), `--report=${report}`, ...appArgs,
  ], { cwd: root, stdio: ['ignore', 'pipe', 'pipe'], env: { ...process.env, ...env } });
  const chunks = [];
  child.stdout.on('data', chunk => chunks.push(chunk));
  child.stderr.on('data', chunk => chunks.push(chunk));
  const timer = setTimeout(() => child.kill('SIGKILL'), timeoutMs);
  const code = await new Promise(done => child.on('close', done));
  clearTimeout(timer);
  const wallMs = Date.now() - started;
  const log = Buffer.concat(chunks).toString('utf8');
  await writeFile(join(work, `${name}.log`), log);
  let result = null;
  try { result = JSON.parse(await readFile(report, 'utf8')); } catch { /* no report */ }
  logs.push({ name, log });
  return { name, code, wallMs, started, log, result, report };
}
const logs = [];

/** Lines of a launch log that are not the report line and not known Chromium noise. */
const NOISE = [
  /task_policy_set TASK_SUPPRESSION_POLICY/, // Chromium on macOS 26 for a background helper; harmless
  /^memcap: /,
];
const ERRORS = /error|fail|missing|not found|404|ERR_|could not|uncaught|exception|denied/i;
function logProblems() {
  const problems = [];
  for (const { name, log } of logs) {
    for (const line of log.split('\n')) {
      if (!line.trim() || line.startsWith('P0_SMOKE ') || line.startsWith('M10_ABOUT ') || NOISE.some(re => re.test(line))) continue;
      if (ERRORS.test(line)) problems.push(`${name}: ${line.slice(0, 300)}`);
    }
  }
  return problems;
}

/** RGBA pixels of an 8-bit, non-interlaced RGB or RGBA PNG (tests/rust-cjk-font.test.mjs). */
function decodePng(bytes) {
  const width = bytes.readUInt32BE(16), height = bytes.readUInt32BE(20);
  const channels = { 2: 3, 6: 4 }[bytes[25]];
  if (bytes[24] !== 8 || !channels || bytes[28] !== 0) throw new Error('not an 8-bit RGB(A) non-interlaced PNG');
  const chunks = [];
  for (let at = 8; at < bytes.length;) {
    const length = bytes.readUInt32BE(at);
    if (bytes.toString('latin1', at + 4, at + 8) === 'IDAT') chunks.push(bytes.subarray(at + 8, at + 8 + length));
    at += 12 + length;
  }
  const raw = inflateSync(Buffer.concat(chunks));
  const stride = width * channels;
  const rows = Buffer.alloc(stride * height);
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)];
    for (let x = 0; x < stride; x++) {
      const value = raw[y * (stride + 1) + 1 + x];
      const left = x >= channels ? rows[y * stride + x - channels] : 0;
      const up = y > 0 ? rows[(y - 1) * stride + x] : 0;
      const corner = x >= channels && y > 0 ? rows[(y - 1) * stride + x - channels] : 0;
      const p = left + up - corner;
      const [a, b, c] = [Math.abs(p - left), Math.abs(p - up), Math.abs(p - corner)];
      const paeth = a <= b && a <= c ? left : b <= c ? up : corner;
      rows[y * stride + x] = (value + [0, left, up, (left + up) >> 1, paeth][filter]) & 0xff;
    }
  }
  const gray = new Uint8Array(width * height);
  for (let i = 0; i < width * height; i++) {
    const at = i * channels;
    gray[i] = Math.round(0.299 * rows[at] + 0.587 * rows[at + 1] + 0.114 * rows[at + 2]);
  }
  return { width, height, gray };
}

/** Pixels darker than mid-grey. */
const inkOf = ({ gray }) => gray.reduce((n, v) => n + (v < 128 ? 1 : 0), 0);

/**
 * Edge sharpness of the dark-on-white content inside the largest white region (the page in a window capture):
 * `blurPx` is anti-aliased pixels per ink boundary pixel (device px), `blurCss` the same in CSS px.
 */
function sharpness(image, dpr) {
  const { width, height, gray } = image;
  // The page: rows and columns where most pixels are paper white.
  const rowWhite = new Array(height).fill(0), colWhite = new Array(width).fill(0);
  for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) if (gray[y * width + x] >= 250) { rowWhite[y]++; colWhite[x]++; }
  const maxRow = Math.max(...rowWhite), maxCol = Math.max(...colWhite);
  const ys = rowWhite.map((n, y) => n > maxRow * 0.5 ? y : -1).filter(y => y >= 0);
  const xs = colWhite.map((n, x) => n > maxCol * 0.5 ? x : -1).filter(x => x >= 0);
  const [x0, x1, y0, y1] = [xs[0], xs[xs.length - 1], ys[0], ys[ys.length - 1]];
  let ink = 0, partial = 0, boundary = 0;
  for (let y = y0 + 1; y < y1; y++) for (let x = x0 + 1; x < x1; x++) {
    const v = gray[y * width + x];
    if (v < 128) {
      ink++;
      if ([gray[y * width + x - 1], gray[y * width + x + 1], gray[(y - 1) * width + x], gray[(y + 1) * width + x]].some(n => n >= 128)) boundary++;
    }
    if (v >= 32 && v <= 223) partial++;
  }
  const blurPx = boundary ? partial / boundary : null;
  return { page: { x0, y0, x1, y1 }, ink, inkCss: Math.round(ink / dpr / dpr), boundary, partial, blurPx: blurPx && +blurPx.toFixed(3), blurCss: blurPx && +(blurPx / dpr).toFixed(3) };
}

/** 日本 at 80 pt in a GB1 Type0 font without a program (tests/rust-cjk-font.test.mjs `cjkPage`). */
function cjkPage() {
  const content = 'BT /C0 80 Tf 10 30 Td <65E5672C> Tj ET';
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 120] /Resources << /Font << /C0 5 0 R >> >> /Contents 4 0 R >>',
    `<< /Length ${content.length} >>\nstream\n${content}\nendstream`,
    '<< /Type /Font /Subtype /Type0 /BaseFont /Song /Encoding /UniGB-UTF16-H /DescendantFonts [6 0 R] >>',
    '<< /Type /Font /Subtype /CIDFontType0 /BaseFont /Song /CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 5 >> /FontDescriptor 7 0 R >>',
    '<< /Type /FontDescriptor /FontName /Song /FontBBox [-200 -200 1200 1200] /Flags 6 /ItalicAngle 0 /Ascent 1000 /Descent -200 /StemV 80 >>',
  ];
  let out = '%PDF-1.7\n';
  const offsets = [];
  objects.forEach((body, index) => { offsets.push(out.length); out += `${index + 1} 0 obj\n${body}\nendobj\n`; });
  const xref = out.length;
  out += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.map(offset => `${String(offset).padStart(10, '0')} 00000 n \n`).join('')}`;
  out += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(out, 'latin1');
}

/** A drive run of `lines` (objects; `${OUT}` is the run's output folder) on the packaged app, or on `binaryPath`. */
async function driveRun(name, lines, { binaryPath = binary, timeoutMs = 180_000 } = {}) {
  const out = join(work, `${name}-out`);
  await mkdir(out, { recursive: true });
  const script = join(work, `${name}.jsonl`);
  await writeFile(script, lines.map(line => JSON.stringify(line).replaceAll('${OUT}', out)).join('\n') + '\n');
  const run = await launch(name, [`--drive=${script}`, `--drive-out=${out}`], { binaryPath, timeoutMs });
  const failed = (run.result?.steps ?? []).map((step, index) => ({ index, cmd: step.cmd, ok: step.ok, error: step.state?.error })).filter(step => step.ok === false);
  return { ...run, out, failed };
}

const report = { app: relative(root, appPath), checkedAt: new Date().toISOString(), items: {} };
const want = item => !only || only.includes(item);
const pass = (item, ok, evidence) => { report.items[item] = { result: ok ? 'pass' : 'fail', ...evidence }; };

// ----------------------------------------------------------------- package facts
const files = asar.listPackage(archive, { isPack: false }).map(path => path.split(sep).join('/'));
const fromAsar = path => asar.extractFile(archive, path.replace(/^\//, ''));
const wasmPackaged = fromAsar('/build/renderer/vendor/tandem_rust_workflow_bg.wasm');
report.package = {
  bytes: Number(spawnSync('du', ['-sk', appPath], { encoding: 'utf8' }).stdout.split('\t')[0]) * 1024,
  asarBytes: statSync(archive).size,
  asarFiles: files.length,
  engine: fromAsar('/build/engine-build.js').toString('utf8').match(/ENGINE\s*=\s*"(\w+)"/)?.[1] ?? null,
  wasmSha256: sha256(wasmPackaged),
  wasmBytes: wasmPackaged.length,
  wasmMatchesEnginePkg: existsSync(join(root, 'engine/web/pkg/tandem_rust_workflow_bg.wasm')) ? sha256(await readFile(join(root, 'engine/web/pkg/tandem_rust_workflow_bg.wasm'))) === sha256(wasmPackaged) : null,
  wasmMatchesBuild: existsSync(join(root, 'build/renderer/vendor/tandem_rust_workflow_bg.wasm')) ? sha256(await readFile(join(root, 'build/renderer/vendor/tandem_rust_workflow_bg.wasm'))) === sha256(wasmPackaged) : null,
  mupdfFiles: files.filter(path => /mupdf/i.test(path)),
  // Absolute build-machine paths in the packaged WASM (engine/tools/build-workflow.sh remaps them to /tpdf, /cargo, ...).
  wasmPathLeaks: [homedir(), root.replace(/\/$/, ''), '/Users/', '/home/'].filter(needle => wasmPackaged.toString('latin1').includes(needle)),
  codesign: spawnSync('codesign', ['-dv', appPath], { encoding: 'utf8' }).stderr.split('\n').filter(line => /^(Signature|Identifier|Format)=/.test(line)),
};

// ----------------------------------------------------------------- startup: launch to first page drawn
if (want('startup')) {
  const timed = async (name, udata) => {
    const run = await launch(name, ['--smoke'], { udata });
    const r = run.result;
    return {
      ok: run.code === 0 && r?.ok === true && r?.packaged === true && r?.engine === 'rust',
      firstPageMs: r?.startup ? r.startup.firstPageEpochMs - run.started : null,
      mainStartedAtMs: r?.startup?.mainStartedAtMs ?? null, firstPageAtMs: r?.startup?.firstPageAtMs ?? null,
      fixtureToPageMs: r?.renderer?.renderMs ?? null, openMs: r?.openMs ?? null, firstPageRenderMs: r?.firstPageRenderMs ?? null,
      exitMs: run.wallMs, wasmRequested: !!r?.protocolRequests?.includes('vendor/tandem_rust_workflow_bg.wasm'),
      fontRequested: !!r?.protocolRequests?.includes(CJK_FONT.served),
    };
  };
  const cold = [], warm = [];
  // Cold: a fresh userData each time (no Chromium code cache, no compiled-WASM cache). The OS file cache is not purged.
  for (let i = 1; i <= 3; i++) cold.push(await timed(`startup-cold-${i}`, join(work, `udata-startup-cold-${i}`)));
  // Warm: the same userData after one unmeasured priming launch.
  await timed('startup-prime', join(work, 'udata-startup-warm'));
  for (let i = 1; i <= 3; i++) warm.push(await timed(`startup-warm-${i}`, join(work, 'udata-startup-warm')));
  const all = [...cold, ...warm];
  pass('startup', all.every(run => run.ok && run.wasmRequested && !run.fontRequested && run.firstPageMs > 0), {
    cold, warm,
    medianFirstPageMs: { cold: median(cold.map(run => run.firstPageMs)), warm: median(warm.map(run => run.firstPageMs)) },
  });
}

// ----------------------------------------------------------------- fonts and assets
if (want('assets')) {
  const fontPath = `/build/renderer/${CJK_FONT.served}`;
  const font = files.includes(fontPath) ? fromAsar(fontPath) : null;
  const page = join(work, 'cjk-nonembedded.pdf');
  await writeFile(page, cjkPage());
  const exports = {};
  // Packaged and dev build export the same page at 150 dpi: the bytes must agree (assets served from app.asar
  // behave as the dev build's files). hello.pdf: base-14 Helvetica; edit-tj.pdf: Helvetica and Times-Roman.
  for (const [label, pdf] of [['cjk', page], ['hello', join(root, 'fixtures/hello.pdf')], ['base14', join(root, 'fixtures/edit-tj.pdf')]]) {
    for (const dev of [false, true]) {
      const name = `assets-${label}-${dev ? 'dev' : 'pkg'}`;
      const png = join(work, `${name}.png`);
      const run = await launch(name, ['--smoke', `--open=${pdf}`, '--edit=export-png', `--save-to=${png}`], { dev });
      const bytes = existsSync(png) ? await readFile(png) : null;
      exports[name] = {
        ok: run.code === 0 && run.result?.ok === true, packaged: run.result?.packaged ?? null,
        fontRequested: !!run.result?.protocolRequests?.includes(CJK_FONT.served),
        pngSha256: bytes && sha256(bytes), ...(bytes ? (({ width, height }) => ({ width, height }))(decodePng(bytes)) : {}),
        ink: bytes ? inkOf(decodePng(bytes)) : null, firstPageRenderMs: run.result?.open?.firstPageRenderMs ?? null,
      };
    }
  }
  // Every asset the packaged policy serves is in app.asar: a missing one would only 404 at run time (a packaged
  // renderer's console is not printed, so the log scan cannot see it).
  const policy = fromAsar('/build/policy.js').toString('utf8');
  const listed = text => [...(text ?? '').matchAll(/\['(\/[^']+)', '/g)].map(match => match[1]);
  const served = [
    ...listed(policy.match(/rust: \[([\s\S]*?)\n\s*\],/)?.[1]),
    ...listed(policy.match(/const assets = new Map\(\[([\s\S]*?)\n\]\);/)?.[1]),
  ];
  const missingAssets = served.filter(path => !files.includes(`/build/renderer${path}`));
  const same = label => exports[`assets-${label}-pkg`].pngSha256 !== null && exports[`assets-${label}-pkg`].pngSha256 === exports[`assets-${label}-dev`].pngSha256;
  const fontOk = !!font && sha256(font) === CJK_FONT.sha256 && font.length === CJK_FONT.size;
  const ok = fontOk && served.length >= 16 && missingAssets.length === 0 && Object.values(exports).every(run => run.ok) && exports['assets-cjk-pkg'].packaged === true
    && exports['assets-cjk-pkg'].fontRequested && !exports['assets-hello-pkg'].fontRequested && !exports['assets-base14-pkg'].fontRequested
    && exports['assets-cjk-pkg'].ink > 4000 && exports['assets-hello-pkg'].ink > 1000 && exports['assets-base14-pkg'].ink > 1000
    && same('cjk') && same('hello') && same('base14');
  pass('assets', ok, { served: served.length, missingAssets, font: { inAsar: !!font, sha256: font && sha256(font), bytes: font?.length ?? null, pinned: fontOk }, exports, samePixels: { cjk: same('cjk'), hello: same('hello'), base14: same('base14') } });
}

// ----------------------------------------------------------------- dialogs (pre-answered) and printing
if (want('dialogs')) {
  const run = await driveRun('dialogs', [
    { cmd: 'open', path: join(root, 'fixtures/hello.pdf'), expect: { doc: { name: 'hello.pdf' }, notes: 0, dirty: false, error: '' } },
    { cmd: 'note', contents: 'M10 package one', expect: { notes: 1, dirty: true, error: '' } },
    // Save a copy (the Save As picker answered by `path`): the document stays dirty and on its original path.
    { cmd: 'save-to', path: '${OUT}/m10-copy.pdf', expect: { save: { ok: true, name: 'm10-copy.pdf' }, dirty: true, error: '' } },
    { cmd: 'open', path: '${OUT}/m10-copy.pdf', expect: { doc: { name: 'm10-copy.pdf' }, notes: 1, dirty: false, error: '' } },
    { cmd: 'note', contents: 'M10 package two', expect: { notes: 2, dirty: true } },
    // Save (in place, through the safe-write journal).
    { cmd: 'save-in-place', expect: { save: { ok: true, name: 'm10-copy.pdf' }, dirty: false, error: '' } },
    { cmd: 'open', path: '${OUT}/m10-copy.pdf', expect: { doc: { name: 'm10-copy.pdf' }, notes: 2, dirty: false, error: '' } },
    // Export (the export picker answered by `to`).
    { cmd: 'export-png', to: '${OUT}/m10-page.png', expect: { error: '' } },
    { cmd: 'export-txt', to: '${OUT}/m10-page.txt', expect: { error: '' } },
    // Print to PDF (Chromium's printToPDF of the print view; the system print dialog is not shown).
    { cmd: 'print-to', path: '${OUT}/m10-print.pdf', range: '1', expect: { print: { ok: true, pages: 1 } } },
  ]);
  const read = async name => { try { return await readFile(join(run.out, name)); } catch { return null; } };
  const copy = await read('m10-copy.pdf'), png = await read('m10-page.png'), txt = await read('m10-page.txt'), printed = await read('m10-print.pdf');
  const pdfOk = bytes => !!bytes && bytes.subarray(0, 5).toString('latin1') === '%PDF-' && bytes.subarray(-1024).toString('latin1').includes('%%EOF');
  const printedPages = printed ? (printed.toString('latin1').match(/\/Type\s*\/Page(?!s)/g) ?? []).length : 0;
  const image = png ? decodePng(png) : null;
  const leftovers = readdirSync(run.out).filter(name => name.endsWith('.tmp'));
  const evidence = {
    exit: run.code, packaged: run.result?.packaged ?? null, failedSteps: run.failed,
    saved: { bytes: copy?.length ?? null, pdf: pdfOk(copy) },
    exportPng: image && { width: image.width, height: image.height, ink: inkOf(image) },
    exportTxt: txt && { bytes: txt.length, hasText: txt.toString('utf8').includes('PDF4All') },
    print: { bytes: printed?.length ?? null, pdf: pdfOk(printed), pages: printedPages, state: run.result?.steps?.at(-1)?.state?.print ?? null },
    tempFilesLeft: leftovers,
  };
  pass('dialogs', run.code === 0 && run.result?.ok === true && run.result?.packaged === true && pdfOk(copy) && !!image && image.width > 0 && inkOf(image) > 1000
    && evidence.exportTxt?.hasText === true && leftovers.length === 0, evidence);
  pass('print', run.result?.packaged === true && pdfOk(printed) && printedPages === 1 && evidence.print.state?.ok === true, { print: evidence.print, failedSteps: run.failed.filter(step => step.cmd === 'print-to') });
}

// ----------------------------------------------------------------- recovery: orphan temp cleanup at start
if (want('recovery')) {
  const udata = join(work, 'udata-orphan');
  const folder = join(work, 'orphan-folder');
  await mkdir(udata, { recursive: true });
  await mkdir(folder, { recursive: true });
  // What a crash between the temp write and the rename leaves: the temp file and its journal entry (src/safe-write.ts).
  const temp = join(folder, `.m10-orphan.pdf.${randomBytes(12).toString('hex')}.tmp`);
  await writeFile(temp, '%PDF-1.7 partial');
  // A journalled temp that is not in its destination's folder is never deleted (cleanupOrphans keeps the entry).
  const foreign = join(work, `.m10-foreign.pdf.${randomBytes(12).toString('hex')}.tmp`);
  await writeFile(foreign, 'keep');
  const journal = join(udata, 'pending-saves.json');
  await writeFile(journal, JSON.stringify([
    { temp, dest: join(folder, 'm10-orphan.pdf'), started: Date.now() - 60_000 },
    { temp: foreign, dest: join(folder, 'm10-foreign.pdf'), started: Date.now() - 60_000 },
  ]));
  const run = await launch('recovery-orphan', ['--smoke'], { udata });
  const after = JSON.parse(await readFile(journal, 'utf8'));
  const evidence = { exit: run.code, smokeOk: run.result?.ok ?? null, packaged: run.result?.packaged ?? null, orphanRemoved: !existsSync(temp), foreignKept: existsSync(foreign), journalAfter: after.map(entry => ({ temp: relative(work, entry.temp) })) };
  pass('recovery', run.code === 0 && run.result?.packaged === true && evidence.orphanRemoved && evidence.foreignKept && after.length === 1 && after[0].temp === foreign, evidence);
}

// ----------------------------------------------------------------- M9 replay after an injected hang
if (want('m9')) {
  // The hang hooks are never in a build: tests/m9-recovery-drive.test.mjs patches a copy of build/. Show they are absent
  // from the package, then run the same kind of patch on a clone of the packaged bundle with a repacked app.asar.
  const worker = fromAsar('/build/renderer/worker-engine.js').toString('utf8');
  const renderer = fromAsar('/build/renderer/renderer.js').toString('utf8');
  const hooksInPackage = ['m9Born', 'HANG', 'tpdf-m9-loads'].filter(token => worker.includes(token) || renderer.includes(token));
  const clone = join(work, 'm9', `${manifest.productName}.app`);
  await rm(join(work, 'm9'), { recursive: true, force: true });
  await mkdir(join(work, 'm9'), { recursive: true });
  // APFS clone: no second copy of the 345 MB bundle on disk.
  const cloned = spawnSync('cp', ['-c', '-R', appPath, clone], { encoding: 'utf8' });
  if (cloned.status !== 0) throw new Error(`cp -c: ${cloned.stderr}`);
  const unpacked = join(work, 'm9', 'asar');
  asar.extractAll(archive, unpacked);
  const patch = async (file, change) => {
    const path = join(unpacked, file);
    const source = await readFile(path, 'utf8');
    const patched = change(source);
    if (patched === source) throw new Error(`the M9 patch does not apply to ${file}`);
    await writeFile(path, patched);
  };
  await patch('build/renderer/worker-engine.js', source => `${source.replace('export async function loadEngine()', 'async function builtLoadEngine()')}
export async function loadEngine() {
  const engine = await builtLoadEngine();
  return { ...engine, dispatch(request, session) {
    if (typeof request?.contents === "string" && request.contents.includes("M10-HANG")) { const end = Date.now() + 120000; while (Date.now() < end) { /* hung */ } }
    return engine.dispatch(request, session);
  } };
}
`);
  await patch('build/renderer/renderer.js', source => source.replace('const REQUEST_TIMEOUT_MS = 20_000;', 'const REQUEST_TIMEOUT_MS = 3000;'));
  await rm(join(clone, 'Contents/Resources/app.asar'));
  await asar.createPackage(unpacked, join(clone, 'Contents/Resources/app.asar'));
  await rm(unpacked, { recursive: true, force: true });
  const cloneBinary = join(clone, 'Contents/MacOS', manifest.productName);
  const hello = join(root, 'fixtures/hello.pdf');
  const edits = [
    { cmd: 'open', path: hello, expect: { doc: { name: 'hello.pdf' }, error: '', recovery: { available: true, entries: 0 } } },
    { cmd: 'freetext', contents: 'M10 box one', expect: { annots: { FreeText: 1 }, dirty: true } },
    { cmd: 'note', contents: 'M10 note two', expect: { notes: 1 } },
    { cmd: 'undo', expect: { notes: 0, canRedo: true } },
    { cmd: 'redo', expect: { notes: 1 } },
    { cmd: 'note', contents: 'M10 note three', expect: { notes: 2, recovery: { available: true, entries: 5 } } },
  ];
  const hung = await driveRun('m9-hang', [
    ...edits,
    { cmd: 'note', contents: 'M10-HANG' },
    { cmd: 'wait', ms: 6000, expect: {
      doc: { name: 'hello.pdf', pageCount: 1 }, notes: 2, annots: { FreeText: 1, Text: 2 }, dirty: true, canUndo: true, canRedo: false,
      error: 'Adding the annotation took too long, so it was stopped. That change was not applied; your other changes were restored.',
      recovery: { available: true, entries: 5, last: { outcome: 'restored', dropped: 'annot-add', cause: 'slow', lost: 0, steps: 5 } },
    } },
    { cmd: 'undo', expect: { notes: 1, canRedo: true } },
    { cmd: 'redo', expect: { notes: 2, canRedo: false } },
    { cmd: 'save-to', path: '${OUT}/recovered.pdf', expect: { save: { ok: true, name: 'recovered.pdf' } } },
  ], { binaryPath: cloneBinary });
  const control = await driveRun('m9-control', [
    ...edits,
    { cmd: 'undo', expect: { notes: 1 } },
    { cmd: 'redo', expect: { notes: 2 } },
    { cmd: 'save-to', path: '${OUT}/control.pdf', expect: { save: { ok: true, name: 'control.pdf' } } },
  ], { binaryPath: cloneBinary });
  const recovered = existsSync(join(hung.out, 'recovered.pdf')) ? await readFile(join(hung.out, 'recovered.pdf')) : null;
  const expected = existsSync(join(control.out, 'control.pdf')) ? await readFile(join(control.out, 'control.pdf')) : null;
  const restart = hung.result?.steps?.[7]?.state?.recovery?.last ?? null;
  await rm(join(work, 'm9'), { recursive: true, force: true });
  pass('m9', hooksInPackage.length === 0 && hung.code === 0 && hung.result?.ok === true && hung.result?.packaged === true && control.code === 0
    && !!recovered && !!expected && recovered.equals(expected), {
    hooksInPackage, ranOn: 'an APFS clone of the package whose app.asar was repacked with the hang patch (the package itself has no hook)',
    hang: { exit: hung.code, failedSteps: hung.failed, restart }, control: { exit: control.code, failedSteps: control.failed },
    savedIdentical: !!recovered && !!expected && recovered.equals(expected), savedBytes: recovered?.length ?? null,
  });
}

// ----------------------------------------------------------------- high-DPI
if (want('dpi')) {
  const runs = {};
  for (const dpr of [1, 2]) {
    const run = await launch(`dpi-${dpr}`, ['--smoke', `--force-device-scale-factor=${dpr}`]);
    const capture = existsSync(`${run.report}.png`) ? decodePng(await readFile(`${run.report}.png`)) : null;
    runs[dpr] = {
      ok: run.code === 0 && run.result?.ok === true && run.result?.packaged === true,
      render: run.result?.renderer && { pixelWidth: run.result.renderer.pixelWidth, pixelHeight: run.result.renderer.pixelHeight },
      canvas: run.result?.isolation && { width: run.result.isolation.width, height: run.result.isolation.height },
      capture: capture && { width: capture.width, height: capture.height, ...sharpness(capture, dpr) },
    };
  }
  const [one, two] = [runs[1], runs[2]];
  const near = (a, b, tolerance) => Math.abs(a - b) <= tolerance;
  const ratio = two.canvas && one.canvas ? +(two.canvas.width / one.canvas.width).toFixed(3) : null;
  const ok = one.ok && two.ok && ratio !== null && near(ratio, 2, 0.01)
    && two.render.pixelWidth === two.canvas.width && one.render.pixelWidth === one.canvas.width
    && near(two.capture.width, 2 * one.capture.width, 2) && near(two.capture.height, 2 * one.capture.height, 2)
    // The same ink in CSS px, with edges about half as wide in CSS px: drawn at the device resolution, not scaled up.
    && near(two.capture.inkCss, one.capture.inkCss, one.capture.inkCss * 0.1) && two.capture.blurCss < one.capture.blurCss * 0.75;
  pass('dpi', ok, { runs, canvasRatio: ratio });
}

// ----------------------------------------------------------------- attribution
if (want('attribution')) {
  const licences = join(resources, 'licenses');
  const same = async (shipped, source) => { try { return sha256(await readFile(join(licences, shipped))) === sha256(await readFile(join(root, source))); } catch { return false; } };
  const walk = dir => readdirSync(dir, { withFileTypes: true }).flatMap(entry => entry.isDirectory() ? walk(join(dir, entry.name)).map(path => `${entry.name}/${path}`) : [entry.name]);
  const engineLicences = walk(join(root, 'engine/reference/licenses'));
  const shippedEngineLicences = existsSync(join(licences, 'engine-licenses')) ? walk(join(licences, 'engine-licenses')) : [];
  const notice = await readFile(join(licences, 'NOTICE.txt'), 'utf8').catch(() => '');
  const droid = await readFile(join(licences, 'third-party/droid-sans-fallback-NOTICE.txt'), 'utf8').catch(() => '');
  // The licences inside app.asar (/licenses) are the same files as the readable folder, minus Electron's.
  const asarLicences = files.filter(path => path.startsWith('/licenses/') && !statFile(path).isDir).map(path => path.slice('/licenses/'.length));
  const readable = walk(licences).filter(path => !path.startsWith('Electron-'));
  const asarMatches = asarLicences.length === readable.length && readable.every(path => asarLicences.includes(path) && sha256(fromAsar(`/licenses/${path}`)) === sha256(readFileSync(join(licences, path))));
  const main = fromAsar('/build/main.js').toString('utf8');
  // Every crate compiled into the WASM (cargo tree, normal edges, wasm32) has a licence folder in the package.
  const tree = spawnSync('cargo', ['tree', '--offline', '--locked', '--target', 'wasm32-unknown-unknown', '-e', 'normal', '--prefix', 'none', '--manifest-path', join(root, 'engine/bridge/Cargo.toml')], { encoding: 'utf8' });
  const crates = tree.status === 0 ? [...new Set(tree.stdout.split('\n').map(line => line.trim()).filter(Boolean).map(line => line.replace(/ \(.*$/, '').replace(/ v(\S+).*/, '-$1')))] : null;
  const shippedDirs = new Set(readdirSync(join(licences, 'engine-licenses')));
  const ownCrates = ['tandem-pdf-engine', 'tandem-rust-workflow'];
  const cratesWithoutLicence = crates?.filter(crate => !ownCrates.some(own => crate.startsWith(`${own}-`)) && !shippedDirs.has(crate)) ?? null;
  const checks = {
    'Engine-NOTICE.txt is engine/NOTICE': await same('Engine-NOTICE.txt', 'engine/NOTICE'),
    'Engine-AGPL-3.0.txt is engine/COPYING': await same('Engine-AGPL-3.0.txt', 'engine/COPYING'),
    'PDF4All-MIT.txt is LICENSE': await same('PDF4All-MIT.txt', 'LICENSE'),
    'engine-licenses is engine/reference/licenses': engineLicences.length === shippedEngineLicences.length && (await Promise.all(engineLicences.map(path => same(`engine-licenses/${path}`, `engine/reference/licenses/${path}`)))).every(Boolean),
    'lgpl-relink.md is docs/third-party/lgpl-relink.md': await same('third-party/lgpl-relink.md', 'docs/third-party/lgpl-relink.md'),
    'droid-sans-fallback-NOTICE.txt is the committed notice': await same('third-party/droid-sans-fallback-NOTICE.txt', CJK_FONT.notice),
    'Droid notice: Apache-2.0 text and the AOSP copyright': /Apache License/.test(droid) && /Version 2\.0/.test(droid) && /Android Open Source Project/.test(droid),
    'Electron LICENSE and LICENSES.chromium.html in licenses': existsSync(join(licences, 'Electron-LICENSE')) && existsSync(join(licences, 'Electron-LICENSES.chromium.html')),
    'NOTICE.txt: Rust engine AGPL-3.0-or-later, informed by MuPDF': /Rust engine, AGPL-3\.0-or-later and informed by MuPDF/.test(notice),
    'NOTICE.txt: corresponding source at the tag, engine included': /Corresponding source: the app's source at this version's tag, the engine in its engine folder/.test(notice),
    'NOTICE.txt: Droid Sans Fallback, AOSP copyright, Apache-2.0': /Droid Sans Fallback/.test(notice) && /Android Open Source Project/.test(notice) && /Apache-2\.0/.test(notice),
    'NOTICE.txt: no IJG sentence (no libjpeg in a Rust build)': !/Independent JPEG Group/.test(notice),
    'NOTICE.txt: no MuPDF archive or MuPDF-AGPL file named': !/MuPDF-AGPL-3\.0\.txt|MuPDF source archive/.test(notice),
    'licences in app.asar match the readable folder': asarMatches,
    'About panel (build/main.js, ENGINE rust): engine and AGPL named, MuPDF credit': report.package.engine === 'rust'
      && main.includes("'The PDF engine is informed by MuPDF, Copyright © Artifex Software, Inc.; see its NOTICE.'")
      && main.includes("Copyright © 2026 Shiro-Legacy. MIT licence; the bundled ${ENGINE === 'rust' ? 'Rust PDF engine' : 'MuPDF'} is AGPL-3.0-or-later."),
    // cargo tree failing (offline registry, no toolchain) is a failed check, not a skipped one.
    'every crate in the WASM has a licence folder': cratesWithoutLicence !== null && cratesWithoutLicence.length === 0,
    'agent folder ships session.mjs, tandem-mcp.mjs and agent.md': ['agent/scripts/session.mjs', 'agent/scripts/tandem-mcp.mjs', 'agent/docs/agent.md'].every(path => existsSync(join(resources, path))),
  };
  pass('attribution', Object.values(checks).every(value => value !== false), {
    checks, cratesWithoutLicence, crates: crates?.length ?? null,
    sourceUrl: notice.match(/https:\/\/\S+?(?=[ )])/)?.[0] ?? null,
    noticeText: notice,
  });
}

// The packaged WASM names no build-machine path (docs/building-rust-release.md).
pass('wasm-paths', report.package.wasmPathLeaks.length === 0, { leaks: report.package.wasmPathLeaks });
report.logProblems = logProblems();
// An unexplained error line in any launch log fails the check too (known Chromium noise is filtered out above).
report.ok = Object.values(report.items).every(item => item.result !== 'fail') && report.logProblems.length === 0;
await writeFile(option('report') ?? join(work, 'm10-package-check.json'), JSON.stringify(report, null, 2) + '\n');
process.stdout.write(JSON.stringify(report, null, 2) + '\n');
process.exitCode = report.ok ? 0 : 1;

function statFile(path) {
  const entry = asar.statFile(archive, path.replace(/^\//, ''));
  return { isDir: 'files' in entry };
}
