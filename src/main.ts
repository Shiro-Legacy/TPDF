import { app, BrowserWindow, Menu, protocol, session, ipcMain, dialog, shell, type IpcMainEvent, type IpcMainInvokeEvent, type WebContents } from 'electron';
import { createHash } from 'node:crypto';
import { createReadStream, readFileSync } from 'node:fs';
import { lstat, mkdir, mkdtemp, readdir, readFile, stat, writeFile } from 'node:fs/promises';
import { Socket } from 'node:net';
import { basename, dirname, isAbsolute, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { tmpdir } from 'node:os';
import { createInterface } from 'node:readline';
import { blankPdf, pageSizeOf } from './blank-pdf.js';
import { ENGINE } from './engine-build.js';
import { batchOutName, CSP, isFixture, localAsset, MAX_PDF_BYTES, openToken, pdfWithinCeiling, saveWithinCeiling, securityPolicy } from './policy.js';
import { cmsDetached, patchSignature, signingWhenOk, type SignOffsets } from './sign/cms.js';
import { createCertHold } from './sign/hold.js';
import { certUsable, parseP12, parsePem, SignError } from './sign/pkcs12.js';
import { createTrustStore } from './sign/trust.js';
import { createSignatureStore } from './sign/drawn.js';
import { verifySignature, type SignatureEntry } from './sign/verify.js';
import { cappedReadStream, cleanupOrphans, copyBasename, inspectRegularFile, saveCopy, saveInPlace, writeNew, type FileId, type SaveResult } from './safe-write.js';

// `--drive=<script.jsonl>` replays commands over the same IPC the menus use (design-agent-drive.md);
// it shares the smoke's headless/no-dialog behaviour, with `--gui` for a visible window.
function argvValue(name: string): string | undefined {
  const prefix = `${name}=`;
  const eq = process.argv.find(arg => arg.startsWith(prefix));
  if (eq) return eq.slice(prefix.length);
  const i = process.argv.indexOf(name);
  if (i >= 0 && process.argv[i + 1] && !process.argv[i + 1].startsWith('-')) return process.argv[i + 1];
}
const driveArg = argvValue('--drive');
const driveGui = process.argv.includes('--gui');
const smoke = process.argv.includes('--smoke') || driveArg !== undefined;
const reportPath = process.argv.find(arg => arg.startsWith('--report='))?.slice(9);
const netLogPath = process.argv.find(arg => arg.startsWith('--net-log='))?.slice(10);
const fixture = process.argv.find(arg => arg.startsWith('--fixture='))?.slice(10) ?? 'hello.pdf';
const openArg = process.argv.find(arg => arg.startsWith('--open='))?.slice(7);
let saveToArg = process.argv.find(arg => arg.startsWith('--save-to='))?.slice(10);
const saveInPlaceArg = process.argv.includes('--save-in-place');
const editArg = process.argv.find(arg => arg.startsWith('--edit='))?.slice(7);
const passwordArg = argvValue('--password');
const newPasswordsArg = argvValue('--new-passwords');
let insertFromArg = process.argv.find(arg => arg.startsWith('--insert-from='))?.slice('--insert-from='.length);
let printToArg = process.argv.find(arg => arg.startsWith('--print-to='))?.slice(11);
const printRangeArg = process.argv.find(arg => arg.startsWith('--print-range='))?.slice(14);
const expectThumbs = process.argv.includes('--expect=P1_THUMBS');
const newFromImages = process.argv.includes('--new-from-images');
let imagesArg = process.argv.find(arg => arg.startsWith('--images='))?.slice('--images='.length);
// Batch (contract-batch.md §9.9): the pick-folder pre-answer; a drive line's `folder` reassigns it.
let batchDirArg = process.argv.find(arg => arg.startsWith('--batch-dir='))?.slice('--batch-dir='.length);
const started = performance.now();
const blockedRequests: string[] = [];
const localRequests = new Set<string>();
const protocolRequests = new Set<string>();
type OpenRecord = FileId & { url: string; name: string };
type PrintResult = { ok: true } | { ok: false; reason: 'cancelled' | 'busy' | 'failed' | 'no-printer' };
const openPaths = new Map<number, OpenRecord>();
/** Bugfix 1 D3: the temp paths `newBlank` wrote; Save (in place) on one of them is Save As. */
const untitledPaths = new Set<string>();
const openDialogs = new Set<number>();
const saveBusy = new Set<number>();
const certHold = createCertHold();
const certPaths = new Map<number, string>();
/** Batch folder listings, per window (contract-batch.md D3, §9.12): only these names are readable. */
const batchListings = new Map<number, { folder: string; names: string[] }>();
const BATCH_MAX_FILES = 200;
const BATCH_OUT = 'pdf4all-out';
const printBusy = new Set<number>();
const dirtyWindows = new Set<number>();
const DISCARD_BOX = {
  type: 'warning' as const,
  buttons: ['Discard Changes', 'Cancel'],
  defaultId: 1,
  cancelId: 1,
  message: 'Discard unsaved changes?',
};
const tokens = new Map<string, { path: string; size: number; contentsId: number; expires: number; timer: ReturnType<typeof setTimeout> }>();
const queuedOpens: string[] = [];
let finishing = false;
let flushingOpens = false;
let windowReady = false;
let smokePrintDone: ((result: PrintResult) => void) | undefined;
let lastP2Link: { uri: string; allowed: boolean } | undefined;
let window: BrowserWindow;

app.on('open-file', (event, path) => {
  event.preventDefault();
  queuedOpens.push(path);
  void flushQueuedOpens();
});

async function flushQueuedOpens(): Promise<void> {
  if (!windowReady || flushingOpens) return;
  flushingOpens = true;
  try {
    while (queuedOpens.length) {
      const next = queuedOpens.shift();
      if (!next || window.isDestroyed()) return;
      await deliverOpen(window.webContents, next);
    }
  } finally {
    flushingOpens = false;
    if (queuedOpens.length && windowReady) void flushQueuedOpens();
  }
}

app.enableSandbox();
app.commandLine.appendSwitch('disable-background-networking');
app.commandLine.appendSwitch('disable-component-update');
app.commandLine.appendSwitch('disable-domain-reliability');
protocol.registerSchemesAsPrivileged([{
  scheme: 'app', privileges: { standard: true, secure: true, supportFetchAPI: true, corsEnabled: true },
}]);

async function finish(result: object, ok: boolean) {
  if (finishing) return;
  finishing = true;
  const report = {
    ok, packaged: app.isPackaged, platform: process.platform, arch: process.arch,
    versions: process.versions, elapsedMs: Math.round(performance.now() - started),
    localRequests: [...localRequests], protocolRequests: [...protocolRequests], blockedRequests,
    ...(lastP2Link ? { p2Link: lastP2Link } : {}), ...result,
  };
  console.log(`P0_SMOKE ${JSON.stringify(report)}`);
  try {
    if (reportPath) await writeFile(reportPath, JSON.stringify(report, null, 2) + '\n');
    if (netLogPath) await session.fromPartition('p0').netLog.stopLogging();
  } catch {
    console.error('Unable to write smoke report');
    ok = false;
  }
  app.exit(ok ? 0 : 1);
}

function isTrustedOpen(event: IpcMainInvokeEvent | IpcMainEvent): boolean {
  if (!window || window.isDestroyed() || event.sender !== window.webContents) return false;
  const frame = event.senderFrame;
  if (!frame || frame !== event.sender.mainFrame) return false;
  return frame.url === 'app://pdf4all/index.html';
}

function linkAllowed(uri: string): boolean {
  try {
    const protocol = new URL(uri).protocol;
    return protocol === 'http:' || protocol === 'https:' || protocol === 'mailto:';
  } catch {
    return false;
  }
}

type NavState = {
  pages?: { count?: number };
  nav?: {
    outline?: { title?: string; page?: number | null }[];
    links?: Record<string, { kind?: string; page?: number }[]>;
  };
};

function navEditOk(state: NavState): boolean {
  const outline = state.nav?.outline;
  if (!Array.isArray(outline) || !outline.length) return false;
  const last = outline[outline.length - 1];
  if (last?.title !== 'PDF4All P2 nav' || last?.page !== 1) return false;
  const byTitle = new Map(outline.map(item => [item.title, item]));
  if (byTitle.get('Two')?.page !== null || byTitle.get('Two-A')?.page !== null) return false;
  if (byTitle.get('Three')?.page !== 1) return false;
  if (state.pages?.count !== 2) return false;
  const p0 = state.nav?.links?.['0'];
  const p1 = state.nav?.links?.['1'];
  if (!Array.isArray(p0) || !Array.isArray(p1)) return false;
  if (p0.filter(item => item.kind === 'page' && item.page === 1).length !== 1) return false;
  if (!p0.some(item => item.kind === 'uri')) return false;
  return p1.length === 1 && p1[0]?.kind === 'blocked';
}

function navUndone(state: NavState): boolean {
  return state.pages?.count === 3 && state.nav?.outline?.find(item => item.title === 'Two')?.page === 1;
}

function suggestedPdfName(raw: unknown): string | undefined {
  if (typeof raw !== 'string' || !raw || raw.length > 255) return;
  if (basename(raw) !== raw) return;
  const name = raw.toLowerCase().endsWith('.pdf') ? raw : `${raw}.pdf`;
  return name.length <= 255 ? name : undefined;
}

function uniqueNames(names: unknown): names is string[] {
  return Array.isArray(names) && names.every(name => typeof name === 'string') && new Set(names).size === names.length;
}

function assembleOk(state: { pages?: { count?: number }; assemble?: { inserted?: number; fieldNames?: unknown } }, openedCount: number): boolean {
  const inserted = state.assemble?.inserted;
  if (typeof inserted !== 'number' || inserted < 1) return false;
  if (state.pages?.count !== openedCount + inserted) return false;
  return uniqueNames(state.assemble?.fieldNames);
}

function extractPagesOk(state: { extract?: { pages?: unknown } }): boolean {
  const pages = state.extract?.pages;
  return Array.isArray(pages) && pages.length === 1 && pages[0] === 1;
}

function smokeEditPayload(): { op: string; contents?: string } {
  if (editArg === 'pages') return { op: 'pages' };
  if (editArg === 'encrypt') {
    const colon = newPasswordsArg?.indexOf(':') ?? -1;
    const user = colon >= 0 ? newPasswordsArg!.slice(0, colon) : (newPasswordsArg ?? '');
    const owner = colon >= 0 ? newPasswordsArg!.slice(colon + 1) : user;
    return { op: 'encrypt', contents: JSON.stringify({ user, owner, permissions: ['print', 'annotate'] }) };
  }
  if (editArg === 'encrypt-keep') {
    return { op: 'encrypt-keep', contents: JSON.stringify({ password: passwordArg, note: 'PDF4All P2 encrypt-keep' }) };
  }
  // P3 content smoke on a locked file: the user password rides in the payload, as encrypt-keep does.
  if (editArg === 'content') return { op: 'content', contents: passwordArg ? JSON.stringify({ password: passwordArg }) : undefined };
  return { op: editArg ?? '', contents: `PDF4All P2 ${editArg}` };
}

function smokePasswords(): string[] {
  const out: string[] = [];
  if (passwordArg) out.push(passwordArg);
  if (newPasswordsArg) {
    const colon = newPasswordsArg.indexOf(':');
    out.push(colon >= 0 ? newPasswordsArg.slice(0, colon) : newPasswordsArg);
    if (colon >= 0) out.push(newPasswordsArg.slice(colon + 1));
  }
  return out.filter(Boolean);
}

type SaveKind = 'pdf' | 'png' | 'jpeg' | 'txt';
const KIND_EXT: Record<SaveKind, string> = { pdf: '.pdf', png: '.png', jpeg: '.jpg', txt: '.txt' };
const KIND_FILTER: Record<SaveKind, { name: string; extensions: string[] }[]> = {
  pdf: [{ name: 'PDF', extensions: ['pdf'] }],
  png: [{ name: 'PNG', extensions: ['png'] }],
  jpeg: [{ name: 'JPEG', extensions: ['jpg', 'jpeg'] }],
  txt: [{ name: 'Text', extensions: ['txt'] }],
};
const IMAGE_MAX_BYTES = 32 * 1024 * 1024;
const IMAGE_MAX_FILES = 50;

function isSaveKind(value: unknown): value is SaveKind {
  return value === 'pdf' || value === 'png' || value === 'jpeg' || value === 'txt';
}

function suggestedKindName(raw: unknown, kind: SaveKind): string | undefined {
  if (typeof raw !== 'string' || !raw || raw.length > 255) return;
  if (basename(raw) !== raw) return;
  const ext = KIND_EXT[kind];
  const lower = raw.toLowerCase();
  const name = lower.endsWith(ext) || (kind === 'jpeg' && lower.endsWith('.jpeg')) ? raw : `${raw}${ext}`;
  return name.length <= 255 ? name : undefined;
}

function decorateOk(state: { pages?: { count?: number }; decorate?: { stamps?: { watermark?: number; header?: number }; labels?: string[]; layers?: { visible?: boolean }[] } }, pageCount: number, wantLayers: boolean): boolean {
  const d = state.decorate;
  if (!d) return false;
  if (d.stamps?.watermark !== pageCount || d.stamps?.header !== pageCount) return false;
  if (state.pages?.count !== pageCount) return false;
  if (!Array.isArray(d.labels) || d.labels[0] !== 'i') return false;
  if (wantLayers) {
    if (!Array.isArray(d.layers) || d.layers.length < 2) return false;
    if (d.layers[1]?.visible !== false) return false;
  }
  return true;
}

function decorateUndone(state: { decorate?: { layersSaved?: boolean; labels?: string[]; layers?: { visible?: boolean }[] } }): boolean {
  if (state.decorate?.layersSaved) return state.decorate.layers?.[1]?.visible === true;
  return state.decorate?.labels?.[0] === '1';
}

function shapesOk(state: { annots?: Record<string, number>; sign?: { subj?: string } }): boolean {
  const a = state.annots;
  if (a?.Square !== 1 || a?.Circle !== 1 || a?.Line !== 1 || a?.Ink !== 1 || a?.Stamp !== 2) return false;
  return state.sign?.subj === 'Signature appearance';
}

function shapesUndone(state: { annots?: Record<string, number> }): boolean {
  return state.annots?.Stamp === 1;
}

function exportMagicOk(kind: string, bytes: Uint8Array): boolean {
  if (kind === 'png' || kind === 'export-png') return bytes.length >= 8 && bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4e && bytes[3] === 0x47;
  if (kind === 'jpeg' || kind === 'export-jpeg') return bytes.length >= 3 && bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff;
  if (kind === 'txt' || kind === 'export-txt') return Buffer.from(bytes).toString('utf8').includes('PDF4All');
  return false;
}

/**
 * Is this child process OS-sandboxed? Electron's `sandboxed` exists on macOS and Windows only. On Linux the kernel
 * says: seccomp filter mode (2), and for a renderer a PID namespace of its own as well (a second `NSpid` column).
 * `/proc/<pid>/status` stays readable whichever sandbox helper (user namespace or setuid) made the process.
 */
function osSandboxed(metric: Electron.ProcessMetric): boolean | undefined {
  if (process.platform !== 'linux') return metric.sandboxed;
  try {
    const status = readFileSync(`/proc/${metric.pid}/status`, 'utf8');
    return /^Seccomp:\s*2$/m.test(status) && (metric.type !== 'Tab' || /^NSpid:\s*\d+\s+\d+/m.test(status));
  } catch { return false; }
}

function metrics() {
  return app.getAppMetrics().map(metric => ({ type: metric.type, sandboxed: osSandboxed(metric), ...metric.memory }));
}

function printFailure(reason: string): 'cancelled' | 'no-printer' | 'failed' {
  const text = reason.toLowerCase();
  if (text.includes('cancel')) return 'cancelled';
  if (/no printers?/.test(text) || text.includes('no valid printer') || text.includes('not found')) return 'no-printer';
  return 'failed';
}

function settleSmokePrint(result: PrintResult): PrintResult {
  const notify = smokePrintDone;
  smokePrintDone = undefined;
  notify?.(result);
  return result;
}

function issuedToken(url: string): string | undefined {
  try {
    const parsed = new URL(url);
    if (parsed.protocol !== 'app:' || parsed.host !== 'pdf4all' || parsed.username || parsed.password || parsed.search || parsed.hash) return;
    return openToken(parsed.pathname);
  } catch { /* deny */ }
}

async function serveOpen(token: string): Promise<Response> {
  const issued = tokens.get(token);
  if (issued) {
    tokens.delete(token);
    clearTimeout(issued.timer);
  }
  const forbidden = new Response(null, { status: 403 });
  if (!issued || Date.now() > issued.expires) return forbidden;
  if (!window || window.isDestroyed() || window.webContents.id !== issued.contentsId) return forbidden;
  try {
    const meta = await inspectRegularFile(issued.path);
    if (!meta || meta.size !== issued.size || issued.size <= 0) return forbidden;
    return new Response(cappedReadStream(issued.path, issued.size), {
      headers: {
        'Content-Type': 'application/pdf',
        'Content-Length': String(issued.size),
        'Content-Security-Policy': CSP,
        'X-Content-Type-Options': 'nosniff',
        'Cache-Control': 'no-store',
      },
    });
  } catch {
    return forbidden;
  }
}

async function inspectOpen(path: string): Promise<{ reason: 'too-large' | 'unreadable'; name?: string; limit?: number } | undefined> {
  const name = basename(path);
  if (!path || !isAbsolute(path)) return { reason: 'unreadable', name };
  const meta = await inspectRegularFile(path);
  if (!meta) return { reason: 'unreadable', name };
  if (!pdfWithinCeiling(meta.size)) return { reason: 'too-large', name, limit: MAX_PDF_BYTES };
}

function setSaveEnabled(enabled: boolean) {
  const item = Menu.getApplicationMenu()?.getMenuItemById('save-as');
  if (item) item.enabled = enabled;
}

function setWindowDirty(id: number, dirty: boolean) {
  if (dirty) dirtyWindows.add(id);
  else dirtyWindows.delete(id);
  if (!window.isDestroyed()) window.setDocumentEdited(dirty);
  const save = Menu.getApplicationMenu()?.getMenuItemById('save');
  if (save) save.enabled = dirty && openPaths.has(id);
}

async function fileSha256(path: string): Promise<string> {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest('hex');
}

async function deliverOpen(contents: WebContents, filePath?: string): Promise<void> {
  if (contents.isDestroyed()) return;
  const id = contents.id;
  if (openDialogs.has(id)) return;
  openDialogs.add(id);
  try {
    if (!smoke && dirtyWindows.has(id)) {
      if (window.isDestroyed()) return;
      const { response } = await dialog.showMessageBox(window, DISCARD_BOX);
      if (contents.isDestroyed()) return;
      if (response !== 0) {
        contents.send('open-error', { reason: 'cancelled' });
        return;
      }
    }
    let path = filePath;
    if (!path) {
      if (window.isDestroyed() || contents.isDestroyed()) return;
      const picked = await dialog.showOpenDialog(window, {
        filters: [{ name: 'PDF', extensions: ['pdf'] }],
        properties: ['openFile'],
      });
      if (picked.canceled || !picked.filePaths[0]) {
        if (!contents.isDestroyed()) contents.send('open-error', { reason: 'cancelled' });
        return;
      }
      path = picked.filePaths[0];
    }
    if (contents.isDestroyed()) return;
    const name = basename(path);
    try {
      const meta = await inspectRegularFile(path);
      if (!meta) {
        if (!contents.isDestroyed()) contents.send('open-error', { reason: 'unreadable', name });
        return;
      }
      if (!pdfWithinCeiling(meta.size)) {
        if (!contents.isDestroyed()) contents.send('open-error', { reason: 'too-large', name, limit: MAX_PDF_BYTES });
        return;
      }
      if (contents.isDestroyed()) return;
      const token = crypto.randomUUID();
      const url = `app://pdf4all/open/${token}`;
      openPaths.set(id, { ...meta, path, url, name });
      const timer = setTimeout(() => tokens.delete(token), 30_000).unref();
      tokens.set(token, { path, size: meta.size, contentsId: id, expires: Date.now() + 30_000, timer });
      // M9: an Untitled document's Save asks for a path first; the renderer must not wait on that dialog.
      contents.send('document', { name, size: meta.size, url, ...(untitledPaths.has(path) ? { untitled: true } : {}) });
      setWindowDirty(id, false);
      setSaveEnabled(true);
    } catch {
      if (!contents.isDestroyed()) contents.send('open-error', { reason: 'unreadable', name });
    }
  } finally {
    openDialogs.delete(id);
  }
}

/**
 * contract-agent-session.md D4: File → New… and the drive `new` step. A blank one-page PDF is
 * written as `Untitled.pdf` into a fresh temp folder and opened like any file, so every op works
 * on it and Save a copy writes it out. A size outside `pageSizeOf` is the `unreadable` open-error.
 */
// ponytail: each New leaves its temp folder behind (the OS temp cleaner owns it); delete on close if it ever matters
async function newBlank(contents: WebContents, size?: unknown): Promise<void> {
  if (contents.isDestroyed()) return;
  const name = 'Untitled.pdf';
  const box = pageSizeOf(size);
  if (!box) return void contents.send('open-error', { reason: 'unreadable', name });
  let path: string;
  try {
    path = join(await mkdtemp(join(app.getPath('temp'), 'pdf4all-new-')), name);
    await writeFile(path, blankPdf(box[0], box[1]));
    untitledPaths.add(path);
  } catch {
    if (!contents.isDestroyed()) contents.send('open-error', { reason: 'unreadable', name });
    return;
  }
  await deliverOpen(contents, path);
}

/** The PDFs named as plain arguments, absolute against the directory the instance was started in. */
function pdfArgs(argv: string[], cwd: string): string[] {
  return argv.slice(1).filter(arg => /\.pdf$/i.test(arg) && !arg.startsWith('--')).map(arg => resolve(cwd, arg));
}

// Linux: the packager embeds no icon in the binary, so the window and the About panel load this PNG
// (assets/icon.png, copied by scripts/build.mjs; a Wayland shell takes the icon from the .desktop file instead).
const LINUX_ICON = fileURLToPath(new URL('./icon.png', import.meta.url));

// ponytail: one instance per userData; the recovery journal has no lock
if (!app.requestSingleInstanceLock()) {
  app.exit(3);
} else {
  // Off macOS "Open with" starts the app with the PDF as a plain argument (macOS sends `open-file`).
  if (!smoke && process.platform !== 'darwin') queuedOpens.push(...pdfArgs(process.argv, process.cwd()));
  app.on('second-instance', (_event, argv, workingDirectory) => {
    queuedOpens.push(...pdfArgs(argv, workingDirectory));
    void flushQueuedOpens();
    if (window && !window.isDestroyed()) { if (window.isMinimized()) window.restore(); window.focus(); }
  });
}

app.whenReady().then(async () => {
  // The bundled engine's licence and credit follow the build (ENGINE). MuPDF's IJG libjpeg wants its sentence
  // wherever only the executable ships (docs/third-party/README.md); the Rust engine is MuPDF-informed and
  // AGPL (engine/NOTICE). Linux (GTK) shows no `credits`, so the credit rides under the copyright line there.
  const credit = ENGINE === 'rust'
    ? 'The PDF engine is informed by MuPDF, Copyright © Artifex Software, Inc.; see its NOTICE.'
    : 'This software is based in part on the work of the Independent JPEG Group.';
  const copyright = `Copyright © 2026 Shiro-Legacy. MIT licence; the bundled ${ENGINE === 'rust' ? 'Rust PDF engine' : 'MuPDF'} is AGPL-3.0-or-later.`;
  app.setAboutPanelOptions({
    applicationName: 'Tandem PDF', applicationVersion: app.getVersion(),
    ...(process.platform === 'linux' ? { copyright: `${copyright}\n${credit}`, iconPath: LINUX_ICON } : { copyright, credits: credit }),
  });
  const journalPath = join(app.getPath('userData'), 'pending-saves.json');
  const trustStore = createTrustStore(smoke ? null : join(app.getPath('userData'), 'trusted-certs.pem'));
  /** draw-signature D3: the drawn signature lives on this Mac only; a drive or smoke keeps it in memory. */
  const signatureStore = createSignatureStore(smoke ? null : join(app.getPath('userData'), 'drawn-signature.json'));
  await cleanupOrphans(journalPath);
  if (!isFixture(fixture)) {
    if (smoke) return void finish({ error: 'unknown fixture' }, false);
    app.exit(1);
    return;
  }
  if (smoke && printToArg !== undefined && (saveToArg !== undefined || saveInPlaceArg)) {
    return void finish({ error: 'print and save together' }, false);
  }
  if (smoke && saveToArg !== undefined && saveInPlaceArg) {
    return void finish({ error: 'save-to and save-in-place together' }, false);
  }
  if (smoke && saveInPlaceArg && openArg === undefined) {
    return void finish({ error: 'save-in-place needs --open' }, false);
  }
  if (smoke && editArg !== undefined && editArg !== 'note' && editArg !== 'highlight' && editArg !== 'freetext' && editArg !== 'pages' && editArg !== 'form' && editArg !== 'nav' && editArg !== 'assemble' && editArg !== 'extract' && editArg !== 'export-png' && editArg !== 'export-jpeg' && editArg !== 'export-txt' && editArg !== 'decorate' && editArg !== 'shapes' && editArg !== 'encrypt' && editArg !== 'encrypt-keep' && editArg !== 'content') {
    return void finish({ error: 'unknown edit' }, false);
  }
  if (smoke && editArg !== undefined && openArg === undefined) {
    return void finish({ error: 'edit needs --open' }, false);
  }
  if (smoke && editArg === 'assemble' && insertFromArg === undefined) {
    return void finish({ error: 'assemble needs --insert-from' }, false);
  }
  if (smoke && editArg === 'shapes' && imagesArg === undefined) {
    return void finish({ error: 'shapes needs --images' }, false);
  }
  if (smoke && editArg === 'extract' && saveToArg === undefined) {
    return void finish({ error: 'extract needs --save-to' }, false);
  }
  if (smoke && editArg === 'extract' && saveInPlaceArg) {
    return void finish({ error: 'extract is a copy' }, false);
  }
  if (smoke && editArg !== undefined && editArg.startsWith('export-') && saveToArg === undefined) {
    return void finish({ error: 'export needs --save-to' }, false);
  }
  if (smoke && editArg === 'encrypt' && (newPasswordsArg === undefined || saveToArg === undefined)) {
    return void finish({ error: 'encrypt needs --new-passwords and --save-to' }, false);
  }
  if (smoke && editArg === 'encrypt-keep' && (passwordArg === undefined || saveToArg === undefined)) {
    return void finish({ error: 'encrypt-keep needs --password and --save-to' }, false);
  }
  if (smoke && newFromImages && (saveToArg === undefined || imagesArg === undefined)) {
    return void finish({ error: 'new-from-images needs --images and --save-to' }, false);
  }
  if (smoke && newFromImages && (openArg !== undefined || editArg !== undefined)) {
    return void finish({ error: 'new-from-images is its own smoke' }, false);
  }
  if (smoke && openArg !== undefined) {
    const openError = await inspectOpen(openArg);
    if (openError) return void finish({ openError }, false);
  }
  if (smoke && insertFromArg !== undefined) {
    const insertError = await inspectOpen(insertFromArg);
    if (insertError) return void finish({ insertError }, false);
  }
  const preload = fileURLToPath(new URL('./preload.cjs', import.meta.url));
  const security = securityPolicy(app.getAppPath(), preload);
  if (!security) {
    if (smoke) return void finish({ error: 'preload outside app' }, false);
    app.exit(1);
    return;
  }
  // In-memory session: no browsing history/cache persisted by this viewer.
  const isolated = session.fromPartition('p0');
  if (smoke && netLogPath) await isolated.netLog.startLogging(netLogPath);
  isolated.setPermissionRequestHandler((_contents, _permission, callback) => callback(false));
  isolated.setPermissionCheckHandler(() => false);
  isolated.setDevicePermissionHandler(() => false);
  isolated.on('will-download', event => event.preventDefault());
  isolated.webRequest.onBeforeRequest((details, callback) => {
    const asset = localAsset(details.url);
    if (asset) {
      localRequests.add(asset.path);
      callback({});
      return;
    }
    const allow = details.method === 'GET' && !!issuedToken(details.url);
    if (!allow) {
      try { blockedRequests.push(new URL(details.url).protocol); }
      catch { blockedRequests.push('invalid:'); }
    } // No document paths/URLs in logs.
    callback({ cancel: !allow });
  });
  const root = fileURLToPath(new URL('./renderer/', import.meta.url));
  isolated.protocol.handle('app', async request => {
    if (request.method !== 'GET') return new Response(null, { status: 403 });
    const token = issuedToken(request.url);
    if (token) return serveOpen(token);
    const asset = localAsset(request.url);
    if (!asset) return new Response(null, { status: 403 });
    protocolRequests.add(asset.path);
    try {
      let body = await readFile(join(root, asset.path));
      if (asset.path === 'index.html') {
        let page = body.toString('utf8').replace(
          '<meta name="pdf4all-fixture" content="hello.pdf">',
          `<meta name="pdf4all-fixture" content="${fixture}">`,
        );
        // Off macOS a scrollbar takes layout space: a smoke/drive page drops the page view's (styles.css `.drive-geometry`),
        // so a drive point lands on the same pixel as under macOS overlay scrollbars.
        if (smoke && process.platform !== 'darwin') page = page.replace('<html lang="en">', '<html lang="en" class="drive-geometry">');
        body = Buffer.from(page);
      }
      return new Response(new Uint8Array(body), {
        headers: { 'Content-Type': asset.type, 'Content-Security-Policy': CSP, 'X-Content-Type-Options': 'nosniff', 'Cache-Control': 'no-store' },
      });
    } catch { return new Response(null, { status: 404 }); }
  });

  window = new BrowserWindow({
    // Content size: the page gets 1360 x 860 on every platform (the Windows frame and menu bar come on top).
    title: 'Tandem PDF', width: 1360, height: 860, minWidth: 900, minHeight: 600, useContentSize: true,
    show: !smoke || driveGui, backgroundColor: '#1b1e25',
    ...(process.platform === 'darwin' ? { titleBarStyle: 'hiddenInset' as const, trafficLightPosition: { x: 14, y: 12 } } : {}),
    ...(process.platform === 'linux' ? { icon: LINUX_ICON } : {}),
    webPreferences: {
      session: isolated, sandbox: true, contextIsolation: true,
      nodeIntegration: false, nodeIntegrationInWorker: false,
      webviewTag: false, webSecurity: true, spellcheck: false, devTools: false,
      // A headless smoke/drive window is never shown. Off macOS a hidden window gets about one frame a second (rAF,
      // a dialog's close event), so a drive step would read STATE before the page settled; rendered offscreen it
      // gets every frame, as a hidden window does on macOS.
      offscreen: smoke && !driveGui && process.platform !== 'darwin',
      preload: security.preload,
    },
  });
  const contents = window.webContents;
  const contentsId = contents.id;
  contents.setWindowOpenHandler(() => ({ action: 'deny' }));
  contents.on('will-navigate', event => event.preventDefault());
  contents.on('will-attach-webview', event => event.preventDefault());
  contents.on('destroyed', () => {
    certHold.clear(contentsId);
    certPaths.delete(contentsId);
    batchListings.delete(contentsId);
    openPaths.delete(contentsId);
    openDialogs.delete(contentsId);
    saveBusy.delete(contentsId);
    printBusy.delete(contentsId);
    setWindowDirty(contentsId, false);
    setSaveEnabled(false);
  });
  window.on('close', event => {
    if (smoke) return;
    if (!dirtyWindows.has(contentsId)) return;
    const response = dialog.showMessageBoxSync(window, DISCARD_BOX);
    if (response !== 0) event.preventDefault();
    else setWindowDirty(contentsId, false);
  });
  contents.on('render-process-gone', () => {
    certHold.clear(contentsId);
    certPaths.delete(contentsId);
    if (smoke) void finish({ error: 'renderer process exited' }, false);
  });
  ipcMain.handle('open-file', event => {
    if (!isTrustedOpen(event)) return;
    return deliverOpen(event.sender);
  });
  ipcMain.handle('open-path', (event, path: unknown) => {
    if (!isTrustedOpen(event)) return;
    if (typeof path !== 'string' || !isAbsolute(path)) {
      if (!event.sender.isDestroyed()) {
        event.sender.send('open-error', { reason: 'unreadable', ...(typeof path === 'string' && path ? { name: basename(path) } : {}) });
      }
      return;
    }
    return deliverOpen(event.sender, path);
  });
  ipcMain.handle('save-file', async (event, raw: unknown, documentUrl: unknown, options?: unknown): Promise<SaveResult> => {
    if (!isTrustedOpen(event)) return { ok: false, reason: 'unreadable' };
    const id = event.sender.id;
    if (saveBusy.has(id)) return { ok: false, reason: 'busy' };
    let inPlace = false;
    let filled = false;
    let suggestedName: string | undefined;
    if (options !== undefined) {
      if (options === null || typeof options !== 'object' || Array.isArray(options)) return { ok: false, reason: 'unreadable' };
      const rec = options as { inPlace?: unknown; filled?: unknown; suggestedName?: unknown };
      if (rec.inPlace === undefined) inPlace = false;
      else if (typeof rec.inPlace === 'boolean') inPlace = rec.inPlace;
      else return { ok: false, reason: 'unreadable' };
      // fill-anywhere D7: Save on a filled document writes a "(filled)" copy (the Save-As route).
      if (rec.filled === undefined) filled = false;
      else if (typeof rec.filled === 'boolean') filled = rec.filled;
      else return { ok: false, reason: 'unreadable' };
      suggestedName = suggestedPdfName(rec.suggestedName);
    }
    if (typeof documentUrl !== 'string' || !documentUrl) return { ok: false, reason: 'document-changed' };
    const recorded = openPaths.get(id);
    if (!recorded || recorded.url !== documentUrl) {
      return { ok: false, reason: inPlace && !recorded ? 'unreadable' : 'document-changed' };
    }
    if (!(raw instanceof ArrayBuffer)) return { ok: false, reason: 'unreadable' };
    if (!saveWithinCeiling(raw.byteLength)) return { ok: false, reason: 'too-large' };
    if (raw.byteLength < 5) return { ok: false, reason: 'unreadable' };
    const bytes = new Uint8Array(raw);
    saveBusy.add(id);
    try {
      const current = openPaths.get(id);
      if (!current || current.url !== documentUrl) return { ok: false, reason: 'document-changed' };
      const untitled = inPlace && untitledPaths.has(current.path);
      const asCopy = inPlace && (untitled || filled);
      if (inPlace && !asCopy) {
        const result = await saveInPlace({
          bytes,
          source: current,
          isCurrent: () => openPaths.get(id)?.url === documentUrl,
          journalPath,
        });
        if (result.ok && result.file) openPaths.set(id, { ...result.file, url: current.url, name: current.name });
        if (result.ok) {
          const { file: _file, ...rest } = result;
          return rest;
        }
        return result;
      }
      let dest: string | undefined;
      if (smoke && saveToArg !== undefined) {
        dest = saveToArg;
      } else if (asCopy && smoke) {
        return { ok: false, reason: 'cancelled' }; // a drive save-in-place on Untitled or a filled document with no path
      } else {
        if (window.isDestroyed() || event.sender.isDestroyed()) return { ok: false, reason: 'cancelled' };
        const picked = await dialog.showSaveDialog(window, {
          defaultPath: untitled ? 'Untitled.pdf'
            : asCopy ? join(dirname(current.path), `${basename(current.path).replace(/\.pdf$/i, '')} (filled).pdf`)
              : suggestedName ?? copyBasename(openPaths.get(id)?.name ?? 'document.pdf'),
          filters: [{ name: 'PDF', extensions: ['pdf'] }],
        });
        if (picked.canceled || !picked.filePath) return { ok: false, reason: 'cancelled' };
        dest = picked.filePath;
      }
      if (!openPaths.get(id) || openPaths.get(id)?.url !== documentUrl) return { ok: false, reason: 'document-changed' };
      const copied = await saveCopy({
        dest,
        bytes,
        source: current,
        isCurrent: () => openPaths.get(id)?.url === documentUrl,
        journalPath,
      });
      if (asCopy && copied.ok) {
        // Save As (Untitled or filled): the window now edits the written file, so the next Save is a plain in-place save.
        const file = await inspectRegularFile(dest);
        if (file && openPaths.get(id)?.url === documentUrl) {
          openPaths.set(id, { ...file, url: current.url, name: copied.name });
          untitledPaths.delete(current.path);
        } else {
          // Review F5: never report a Save As the window was not rebound to, or the next Save would overwrite the original.
          return { ok: false, reason: 'write-failed' };
        }
      }
      return copied;
    } finally {
      saveBusy.delete(id);
    }
  });
  ipcMain.on('dirty', (event, dirty: unknown) => {
    if (!isTrustedOpen(event) || typeof dirty !== 'boolean') return;
    setWindowDirty(event.sender.id, dirty);
  });
  ipcMain.handle('open-link', async (event, uri: unknown) => {
    if (!isTrustedOpen(event)) return;
    if (typeof uri !== 'string' || !uri || uri.length > 2048) return;
    const allowed = linkAllowed(uri);
    if (smoke) {
      lastP2Link = { uri, allowed };
      console.log(`P2_LINK ${JSON.stringify({ uri, allowed })}`);
      return;
    }
    if (!allowed || window.isDestroyed()) return;
    const { response } = await dialog.showMessageBox(window, {
      type: 'question',
      buttons: ['Open', 'Cancel'],
      defaultId: 0,
      cancelId: 1,
      message: `Open ${uri} in your browser?`,
    });
    if (response !== 0) return;
    await shell.openExternal(uri);
  });
  ipcMain.handle('insert-source', async (event) => {
    if (!isTrustedOpen(event)) return { ok: false, reason: 'unreadable' };
    let path: string | undefined;
    if (smoke && insertFromArg) {
      path = insertFromArg;
    } else {
      if (window.isDestroyed()) return { ok: false, reason: 'cancelled' };
      const picked = await dialog.showOpenDialog(window, {
        filters: [{ name: 'PDF', extensions: ['pdf'] }],
        properties: ['openFile'],
      });
      if (picked.canceled || !picked.filePaths[0]) return { ok: false, reason: 'cancelled' };
      path = picked.filePaths[0];
    }
    const name = basename(path);
    try {
      if (!isAbsolute(path)) return { ok: false, reason: 'unreadable' };
      const meta = await inspectRegularFile(path);
      if (!meta) return { ok: false, reason: 'unreadable' };
      if (!pdfWithinCeiling(meta.size)) return { ok: false, reason: 'too-large' };
      const data = await readFile(path);
      const bytes = data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength);
      return { ok: true, name, bytes };
    } catch {
      return { ok: false, reason: 'unreadable' };
    }
  });
  ipcMain.handle('save-new', async (event, raw: unknown, suggestedName: unknown, kind: unknown, options?: unknown) => {
    if (!isTrustedOpen(event)) return { ok: false, reason: 'unreadable' };
    if (!isSaveKind(kind)) return { ok: false, reason: 'unreadable' };
    const id = event.sender.id;
    if (saveBusy.has(id)) return { ok: false, reason: 'busy' };
    if (!(raw instanceof ArrayBuffer)) return { ok: false, reason: 'unreadable' };
    if (!saveWithinCeiling(raw.byteLength) || raw.byteLength < 1) return { ok: false, reason: raw.byteLength < 1 ? 'unreadable' : 'too-large' };
    let openAfter = false;
    if (options !== undefined) {
      if (options === null || typeof options !== 'object' || Array.isArray(options)) return { ok: false, reason: 'unreadable' };
      const flag = (options as { open?: unknown }).open;
      if (flag === undefined) openAfter = false;
      else if (typeof flag === 'boolean') openAfter = flag;
      else return { ok: false, reason: 'unreadable' };
    }
    const bytes = new Uint8Array(raw);
    saveBusy.add(id);
    try {
      let dest: string | undefined;
      if (smoke && saveToArg !== undefined) {
        dest = saveToArg;
      } else {
        if (window.isDestroyed() || event.sender.isDestroyed()) return { ok: false, reason: 'cancelled' };
        const picked = await dialog.showSaveDialog(window, {
          defaultPath: suggestedKindName(suggestedName, kind) ?? `untitled${KIND_EXT[kind]}`,
          filters: KIND_FILTER[kind],
        });
        if (picked.canceled || !picked.filePath) return { ok: false, reason: 'cancelled' };
        dest = picked.filePath;
      }
      const result = await writeNew({ dest, bytes, journalPath });
      if (result.ok && kind === 'pdf' && openAfter && !smoke) await deliverOpen(event.sender, dest);
      return result.ok ? { ok: true, name: result.name } : { ok: false, reason: result.reason === 'disk-full' || result.reason === 'permission' || result.reason === 'write-failed' ? 'unreadable' : result.reason };
    } finally {
      saveBusy.delete(id);
    }
  });
  function signOffsets(value: unknown): SignOffsets | undefined {
    if (!value || typeof value !== 'object' || Array.isArray(value)) return;
    const rec = value as Record<string, unknown>;
    const { contentsAt, contentsLen, byteRangeAt } = rec;
    if (!Number.isInteger(contentsAt) || !Number.isInteger(contentsLen) || !Number.isInteger(byteRangeAt)) return;
    return { contentsAt: contentsAt as number, contentsLen: contentsLen as number, byteRangeAt: byteRangeAt as number };
  }

  ipcMain.handle('pick-certificate', async (event, opts?: unknown) => {
    if (!isTrustedOpen(event)) return { ok: false, reason: 'unreadable' };
    let path: string | undefined;
    if (smoke && opts && typeof opts === 'object' && !Array.isArray(opts)) {
      const given = (opts as { path?: unknown }).path;
      if (typeof given === 'string' && given) path = given;
    }
    if (!path) {
      if (window.isDestroyed()) return { ok: false, reason: 'cancelled' };
      const picked = await dialog.showOpenDialog(window, {
        filters: [{ name: 'Certificate', extensions: ['p12', 'pfx', 'pem'] }],
        properties: ['openFile'],
      });
      if (picked.canceled || !picked.filePaths[0]) return { ok: false, reason: 'cancelled' };
      path = picked.filePaths[0];
    }
    certHold.clear(event.sender.id);
    certPaths.set(event.sender.id, path);
    return { ok: true, name: basename(path) };
  });
  ipcMain.handle('certificate-info', async (event, password: unknown) => {
    if (!isTrustedOpen(event)) return { ok: false, reason: 'unreadable' };
    if (typeof password !== 'string' || password.length > 1024) return { ok: false, reason: 'unreadable' };
    const path = certPaths.get(event.sender.id);
    if (!path) return { ok: false, reason: 'no-certificate' };
    // A new attempt drops any key a previous attempt left, so a failed retry never signs with it.
    certHold.clear(event.sender.id);
    try {
      const data = await readFile(path);
      if (data.byteLength < 1 || data.byteLength > 1024 * 1024) return { ok: false, reason: 'unreadable' };
      const parsed = path.toLowerCase().endsWith('.pem') ? parsePem(data, password) : parseP12(data, password);
      const facts = certUsable(parsed.certs, parsed.key, Date.now());
      if (!facts.ok) return { ok: false, reason: facts.reason, ...(facts.date ? { date: facts.date, status: facts.status } : {}) };
      certHold.put(event.sender.id, { key: parsed.key, certs: parsed.certs, cn: facts.cn });
      return { ok: true, cn: facts.cn, notAfter: facts.notAfter, certs: parsed.certs.length };
    } catch (err) {
      return { ok: false, reason: err instanceof SignError ? err.reason : 'unreadable' };
    }
  });
  ipcMain.handle('certificate-clear', (event) => {
    if (!isTrustedOpen(event)) return { ok: false };
    certHold.clear(event.sender.id);
    return { ok: true };
  });
  ipcMain.handle('sign-file', async (event, raw: unknown, offsets: unknown, when: unknown, suggestedName: unknown) => {
    if (!isTrustedOpen(event)) return { ok: false, reason: 'unreadable' };
    const id = event.sender.id;
    if (saveBusy.has(id)) return { ok: false, reason: 'busy' };
    const held = certHold.take(id);
    if (!held) return { ok: false, reason: 'no-certificate' };
    saveBusy.add(id);
    try {
      if (!(raw instanceof ArrayBuffer)) return { ok: false, reason: 'unreadable' };
      if (!saveWithinCeiling(raw.byteLength)) return { ok: false, reason: 'too-large' };
      if (typeof when !== 'number' || !signingWhenOk(when)) return { ok: false, reason: 'invalid' };
      const parsed = signOffsets(offsets);
      if (!parsed) return { ok: false, reason: 'unreadable' };
      const facts = certUsable(held.certs, held.key, when);
      if (!facts.ok) return { ok: false, reason: facts.reason, ...(facts.date ? { date: facts.date, status: facts.status } : {}) };
      const patched = patchSignature(new Uint8Array(raw), parsed, content => cmsDetached({
        content, certs: held.certs, key: held.key, signingTime: new Date(when),
      }));
      if (!patched.ok) return patched;
      let dest: string | undefined;
      if (smoke && saveToArg !== undefined) {
        dest = saveToArg;
      } else {
        if (window.isDestroyed() || event.sender.isDestroyed()) return { ok: false, reason: 'cancelled' };
        const picked = await dialog.showSaveDialog(window, {
          defaultPath: suggestedKindName(suggestedName, 'pdf') ?? 'signed.pdf',
          filters: [{ name: 'PDF', extensions: ['pdf'] }],
        });
        if (picked.canceled || !picked.filePath) return { ok: false, reason: 'cancelled' };
        dest = picked.filePath;
      }
      const result = await writeNew({ dest, bytes: patched.bytes, journalPath });
      if (!result.ok) {
        return { ok: false, reason: result.reason === 'cancelled' || result.reason === 'too-large' ? result.reason : 'unreadable' };
      }
      return { ok: true, name: result.name, signer: held.cn };
    } catch (err) {
      return { ok: false, reason: err instanceof SignError ? err.reason : 'unreadable' };
    } finally {
      saveBusy.delete(id);
    }
  });
  ipcMain.handle('verify-signatures', (event, raw: unknown, entries: unknown) => {
    if (!isTrustedOpen(event)) return { ok: false, reason: 'unreadable' };
    if (!(raw instanceof ArrayBuffer) || !Array.isArray(entries)) return { ok: false, reason: 'unreadable' };
    if (!saveWithinCeiling(raw.byteLength)) return { ok: false, reason: 'too-large' };
    if (entries.length > 200) return { ok: false, reason: 'too-many' };
    try {
      const src = Buffer.from(raw);
      const anchors = trustStore.certs();
      const verdicts = entries.map(entry => verifySignature(src, entry as SignatureEntry, anchors));
      return { ok: true, verdicts, trusted: trustStore.list().map(({ cn, notAfter }) => ({ cn, notAfter })) };
    } catch {
      return { ok: false, reason: 'unreadable' };
    }
  });
  ipcMain.handle('trust-add', async (event, opts?: unknown) => {
    if (!isTrustedOpen(event)) return { ok: false, reason: 'unreadable' };
    let path: string | undefined;
    if (smoke && opts && typeof opts === 'object' && !Array.isArray(opts)) {
      const given = (opts as { path?: unknown }).path;
      if (typeof given === 'string' && given) path = given;
    }
    if (!path) {
      if (window.isDestroyed()) return { ok: false, reason: 'cancelled' };
      const picked = await dialog.showOpenDialog(window, {
        filters: [{ name: 'Certificates', extensions: ['pem', 'crt', 'cer'] }],
        properties: ['openFile'],
      });
      if (picked.canceled || !picked.filePaths[0]) return { ok: false, reason: 'cancelled' };
      path = picked.filePaths[0];
    }
    try {
      const data = await readFile(path);
      if (data.byteLength < 1 || data.byteLength > 1024 * 1024) return { ok: false, reason: 'unreadable' };
      const { added, total } = trustStore.add(data);
      return { ok: true, added, total };
    } catch {
      return { ok: false, reason: 'unreadable' };
    }
  });
  ipcMain.handle('trust-clear', (event) => {
    if (!isTrustedOpen(event)) return { ok: false };
    trustStore.clear();
    return { ok: true };
  });
  /** draw-signature D3 / sign-merge D2: read and replace (or remove, null) the saved drawing or image; validated, never logged. */
  ipcMain.handle('signature-get', (event) => {
    if (!isTrustedOpen(event)) return { ok: false, signature: null };
    return { ok: true, signature: signatureStore.get() };
  });
  ipcMain.handle('signature-set', (event, value: unknown) => {
    if (!isTrustedOpen(event)) return { ok: false };
    try { signatureStore.set(value === null ? null : value as Parameters<typeof signatureStore.set>[0]); return { ok: true }; } catch { return { ok: false }; }
  });
  ipcMain.handle('pick-images', async (event) => {
    if (!isTrustedOpen(event)) return { ok: false, reason: 'unreadable' };
    let paths: string[] = [];
    if (smoke && imagesArg) {
      paths = imagesArg.split(',').map(item => item.trim()).filter(Boolean);
    } else if (smoke) {
      // sign-merge D6: a headless run never shows a native sheet; no pre-answer means cancelled (as pick-folder).
      return { ok: false, reason: 'cancelled' };
    } else {
      if (window.isDestroyed()) return { ok: false, reason: 'cancelled' };
      const picked = await dialog.showOpenDialog(window, {
        filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg'] }],
        properties: ['openFile', 'multiSelections'],
      });
      if (picked.canceled || !picked.filePaths.length) return { ok: false, reason: 'cancelled' };
      paths = picked.filePaths;
    }
    if (paths.length > IMAGE_MAX_FILES) return { ok: false, reason: 'too-many' };
    try {
      const images: { name: string; bytes: ArrayBuffer }[] = [];
      let total = 0;
      for (const path of paths) {
        if (!isAbsolute(path)) return { ok: false, reason: 'unreadable' };
        const meta = await inspectRegularFile(path);
        if (!meta) return { ok: false, reason: 'unreadable' };
        if (meta.size > IMAGE_MAX_BYTES || !pdfWithinCeiling(total + meta.size)) return { ok: false, reason: 'too-large' };
        const data = await readFile(path);
        total += data.byteLength;
        images.push({ name: basename(path), bytes: data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength) });
      }
      return { ok: true, images };
    } catch {
      return { ok: false, reason: 'unreadable' };
    }
  });
  ipcMain.handle('pick-folder', async (event) => {
    if (!isTrustedOpen(event)) return { ok: false, reason: 'unreadable' };
    let folder: string | undefined;
    if (smoke) {
      // Headless runs never show a native sheet (§9.9): no pre-answer means cancelled.
      if (!batchDirArg) return { ok: false, reason: 'cancelled' };
      folder = batchDirArg;
    } else {
      if (window.isDestroyed()) return { ok: false, reason: 'cancelled' };
      const picked = await dialog.showOpenDialog(window, { properties: ['openDirectory'] });
      if (picked.canceled || !picked.filePaths[0]) return { ok: false, reason: 'cancelled' };
      folder = picked.filePaths[0];
    }
    try {
      if (!isAbsolute(folder)) return { ok: false, reason: 'unreadable' };
      const entries = await readdir(folder, { withFileTypes: true });
      // One order for the queue and the Bates numbers (§9.8).
      const names = entries.map(entry => entry.name).filter(name => name.toLowerCase().endsWith('.pdf'))
        .sort((a, b) => a.localeCompare(b, 'en', { numeric: true, sensitivity: 'base' }));
      if (names.length > BATCH_MAX_FILES) return { ok: false, reason: 'too-many-files' };
      const files: { name: string; bytes: number }[] = [];
      for (const name of names) {
        const meta = await inspectRegularFile(join(folder, name));
        if (meta) files.push({ name, bytes: meta.size });
      }
      batchListings.set(event.sender.id, { folder, names: files.map(file => file.name) });
      return { ok: true, folder, out: join(folder, BATCH_OUT), files };
    } catch {
      return { ok: false, reason: 'unreadable' };
    }
  });
  ipcMain.handle('batch-io', async (event, name: unknown, action: unknown, raw?: unknown) => {
    if (!isTrustedOpen(event)) return { ok: false, reason: 'unreadable' };
    const listing = batchListings.get(event.sender.id);
    if (!listing) return { ok: false, reason: 'not-listed' };
    try {
      if (action === 'read') {
        if (typeof name !== 'string' || !listing.names.includes(name)) return { ok: false, reason: 'not-listed' };
        const path = join(listing.folder, name);
        const meta = await inspectRegularFile(path);
        if (!meta) return { ok: false, reason: 'unreadable' };
        if (!pdfWithinCeiling(meta.size)) return { ok: false, reason: 'too-large' };
        const data = await readFile(path);
        return { ok: true, bytes: data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength) };
      }
      if (action !== 'write' && action !== 'exists') return { ok: false, reason: 'unreadable' };
      const safe = batchOutName(name, listing.names);
      if (!safe) return { ok: false, reason: 'bad-name' };
      const out = join(listing.folder, BATCH_OUT);
      const dest = join(out, safe);
      const taken = async () => { try { await lstat(dest); return true; } catch { return false; } };
      if (action === 'exists') return { ok: true, exists: await taken() };
      if (!(raw instanceof ArrayBuffer) || raw.byteLength < 1) return { ok: false, reason: 'unreadable' };
      if (!saveWithinCeiling(raw.byteLength)) return { ok: false, reason: 'too-large' };
      await mkdir(out).catch((err: NodeJS.ErrnoException) => { if (err.code !== 'EEXIST') throw err; });
      // Never write through a symlinked output folder, never over an existing file (§9.12).
      const dir = await lstat(out);
      if (!dir.isDirectory() || dir.isSymbolicLink()) return { ok: false, reason: 'unwritable' };
      if (await taken()) return { ok: false, reason: 'exists' };
      const result = await writeNew({ dest, bytes: new Uint8Array(raw), journalPath });
      return result.ok ? { ok: true, name: safe } : { ok: false, reason: result.reason === 'too-large' ? 'too-large' : 'unwritable' };
    } catch {
      return { ok: false, reason: 'unwritable' };
    }
  });
  ipcMain.handle('print', async (event): Promise<PrintResult> => {
    try {
      if (!isTrustedOpen(event) || event.sender.isDestroyed()) return { ok: false, reason: 'failed' };
      const id = event.sender.id;
      if (printBusy.has(id)) return { ok: false, reason: 'busy' };
      printBusy.add(id);
      try {
        if (smoke && printToArg !== undefined) {
          if (!isAbsolute(printToArg)) return settleSmokePrint({ ok: false, reason: 'failed' });
          const pdf = await event.sender.printToPDF({ printBackground: false, preferCSSPageSize: false });
          await writeFile(printToArg, pdf);
          return settleSmokePrint({ ok: true });
        }
        const printers = await event.sender.getPrintersAsync();
        if (printers.length === 0) {
          if (window.isDestroyed() || event.sender.isDestroyed()) return { ok: false, reason: 'failed' };
          const picked = await dialog.showSaveDialog(window, {
            defaultPath: (openPaths.get(id)?.name ?? 'document.pdf').replace(/\.pdf$/i, '') + '-print.pdf',
            filters: [{ name: 'PDF', extensions: ['pdf'] }],
          });
          if (picked.canceled || !picked.filePath) return { ok: false, reason: 'cancelled' };
          if (openPaths.get(id)?.path === picked.filePath) return { ok: false, reason: 'failed' };
          try {
            const pdf = await event.sender.printToPDF({ printBackground: false, preferCSSPageSize: false });
            await writeFile(picked.filePath, pdf);
            return { ok: true };
          } catch (err) {
            console.error(err);
            return { ok: false, reason: 'failed' };
          }
        }
        return await new Promise<PrintResult>(resolve => {
          if (event.sender.isDestroyed()) {
            resolve(settleSmokePrint({ ok: false, reason: 'failed' }));
            return;
          }
          event.sender.print({ printBackground: false }, (success, failureReason) => {
            if (!success) console.error(`print failed: ${failureReason}`); // terminal only; the renderer gets a reason token
            resolve(success ? settleSmokePrint({ ok: true }) : settleSmokePrint({ ok: false, reason: printFailure(failureReason ?? '') }));
          });
        });
      } finally {
        printBusy.delete(id);
      }
    } catch {
      return settleSmokePrint({ ok: false, reason: 'failed' });
    }
  });
  Menu.setApplicationMenu(Menu.buildFromTemplate([
    ...(process.platform === 'darwin' ? [{ role: 'appMenu' as const }] : []),
    {
      label: 'File',
      submenu: [
        {
          label: 'Open…',
          accelerator: 'CmdOrCtrl+O',
          click: () => {
            if (window.isDestroyed()) return;
            void deliverOpen(window.webContents);
          },
        },
        {
          label: 'New…',
          accelerator: 'CmdOrCtrl+N',
          click: () => {
            if (window.isDestroyed()) return;
            void newBlank(window.webContents);
          },
        },
        {
          label: 'New from Images…',
          accelerator: 'Alt+CmdOrCtrl+N',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'new-from-images' });
          },
        },
        {
          id: 'save',
          label: 'Save',
          accelerator: 'CmdOrCtrl+S',
          enabled: false,
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('save-request', { inPlace: true });
          },
        },
        {
          id: 'save-as',
          label: 'Save As…',
          accelerator: 'CmdOrCtrl+Shift+S',
          enabled: false,
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('save-request');
          },
        },
        {
          id: 'print',
          label: 'Print…',
          accelerator: 'CmdOrCtrl+P',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('print-request', {});
          },
        },
        {
          label: 'Export…',
          accelerator: 'Shift+CmdOrCtrl+E',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'export' });
          },
        },
        {
          label: 'Batch…',
          accelerator: 'Alt+CmdOrCtrl+B',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'batch' });
          },
        },
        {
          id: 'security',
          label: 'Security…',
          accelerator: 'Alt+CmdOrCtrl+S',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'security' });
          },
        },
        {
          label: 'Sign with Certificate…',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'cert-sign-open' });
          },
        },
        {
          label: 'Verify Signatures…',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'verify-open' });
          },
        },
        {
          // contract-compare.md §9.9: a second PDF beside the current one (the renderer picks it via insert-source).
          label: 'Compare with…',
          accelerator: 'Alt+CmdOrCtrl+K',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'compare' });
          },
        },
        { role: 'close' },
        // macOS has Quit in the app menu; elsewhere it is File ▸ Exit.
        ...(process.platform === 'darwin' ? [] : [{ type: 'separator' as const }, { role: 'quit' as const }]),
      ],
    },
    {
      label: 'Edit',
      submenu: [
        {
          label: 'Undo',
          accelerator: 'CmdOrCtrl+Z',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'undo' });
          },
        },
        {
          label: 'Redo',
          accelerator: 'Shift+CmdOrCtrl+Z',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'redo' });
          },
        },
        { type: 'separator' },
        { role: 'cut' },
        { role: 'copy' },
        { role: 'paste' },
        { role: 'selectAll' },
        { type: 'separator' },
        {
          label: 'Existing Content…',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'content-edit' });
          },
        },
        {
          label: 'Redact',
          submenu: [
            {
              label: 'Mark Area',
              click: () => {
                if (window.isDestroyed()) return;
                window.webContents.send('edit-request', { op: 'redact-area' });
              },
            },
            {
              label: 'Mark Search Hits…',
              click: () => {
                if (window.isDestroyed()) return;
                window.webContents.send('edit-request', { op: 'redact-search' });
              },
            },
            {
              label: 'Apply Redactions…',
              click: () => {
                if (window.isDestroyed()) return;
                window.webContents.send('edit-request', { op: 'redact-apply-open' });
              },
            },
          ],
        },
      ],
    },
    {
      label: 'Insert',
      submenu: [
        {
          label: 'Note',
          accelerator: 'Shift+CmdOrCtrl+N',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'note' });
          },
        },
        {
          label: 'Highlight Match',
          accelerator: 'Shift+CmdOrCtrl+H',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'highlight' });
          },
        },
        {
          label: 'Text Box',
          accelerator: 'Shift+CmdOrCtrl+L',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'freetext' });
          },
        },
        {
          label: 'Text as Page Content…',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'text-add-open' });
          },
        },
        {
          label: 'Bookmark…',
          accelerator: 'Shift+CmdOrCtrl+B',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'bookmark' });
          },
        },
        {
          label: 'Link to Page…',
          accelerator: 'Shift+CmdOrCtrl+K',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'link' });
          },
        },
        {
          label: 'Square',
          accelerator: 'Shift+CmdOrCtrl+G',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'square' });
          },
        },
        {
          label: 'Circle',
          accelerator: 'Shift+CmdOrCtrl+O',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'circle' });
          },
        },
        {
          label: 'Line',
          accelerator: 'Shift+CmdOrCtrl+\\',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'line' });
          },
        },
        {
          label: 'Ink',
          accelerator: 'Shift+CmdOrCtrl+I',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'ink' });
          },
        },
        {
          label: 'Stamp…',
          accelerator: 'Shift+CmdOrCtrl+M',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'stamp' });
          },
        },
        {
          // sign-merge D5: the one signature item — arm Fill forms placement of the saved signature (the pad opens first when none is saved).
          label: 'Place Signature',
          accelerator: 'Shift+CmdOrCtrl+J',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'sign-place' });
          },
        },
      ],
    },
    {
      label: 'Page',
      submenu: [
        {
          label: 'Rotate Right',
          accelerator: 'Shift+CmdOrCtrl+]',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'rotate-right' });
          },
        },
        {
          label: 'Rotate Left',
          accelerator: 'Shift+CmdOrCtrl+[',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'rotate-left' });
          },
        },
        {
          label: 'Move Page Up',
          accelerator: 'Alt+CmdOrCtrl+Up',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'move-up' });
          },
        },
        {
          label: 'Move Page Down',
          accelerator: 'Alt+CmdOrCtrl+Down',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'move-down' });
          },
        },
        {
          label: 'Delete Page',
          accelerator: 'Shift+CmdOrCtrl+Backspace',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'delete-page' });
          },
        },
        {
          label: 'Insert Pages from File…',
          accelerator: 'Alt+CmdOrCtrl+I',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'insert' });
          },
        },
        {
          label: 'Extract Pages…',
          accelerator: 'Alt+CmdOrCtrl+E',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'extract' });
          },
        },
        {
          label: 'Watermark…',
          accelerator: 'Alt+CmdOrCtrl+W',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'watermark' });
          },
        },
        {
          label: 'Header and Footer…',
          accelerator: 'Alt+CmdOrCtrl+F',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'header-footer' });
          },
        },
        {
          label: 'Page Labels…',
          accelerator: 'Alt+CmdOrCtrl+L',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'labels' });
          },
        },
      ],
    },
    { role: 'windowMenu' },
    {
      role: 'help',
      submenu: [
        {
          label: 'What Can Be Edited',
          click: () => {
            if (window.isDestroyed()) return;
            window.webContents.send('edit-request', { op: 'help' });
          },
        },
        // macOS has About in the app menu; elsewhere the About panel (and its IJG sentence) is reached from Help.
        ...(process.platform === 'darwin' ? [] : [{ type: 'separator' as const }, { role: 'about' as const }]),
      ],
    },
  ]));

  if (driveArg !== undefined) {
    void startDrive(contents, driveArg);
  } else if (smoke) {
    setTimeout(() => void finish({ error: 'smoke timeout' }, false), openArg || newFromImages ? 120_000 : 30_000).unref();
    if (openArg !== undefined) {
      let pushed = false;
      let saving = false;
      let printing = false;
      let editing = false;
      let undoing = false;
      let opened: unknown;
      let thumbs: unknown;
      let edit: unknown;
      const saveTo = saveToArg !== undefined ? saveToArg : undefined;
      const printTo = printToArg !== undefined ? printToArg : undefined;
      const printed = printTo ? new Promise<PrintResult>(resolve => { smokePrintDone = resolve; }) : undefined;
      let baseline: { mtimeMs: number; hash: string; mode: number } | undefined;
      if (saveTo || saveInPlaceArg) {
        const meta = await inspectRegularFile(openArg);
        if (meta) baseline = { mtimeMs: meta.mtimeMs, hash: await fileSha256(openArg), mode: (await stat(openArg)).mode };
      }
      contents.on('console-message', async details => {
        if (details.message.startsWith('P0_ERROR')) {
          await finish({ error: details.message }, false);
          return;
        }
        if ((saveTo || saveInPlaceArg) && details.message.startsWith('P1_SAVE_ERROR')) {
          await finish({ error: details.message }, false);
          return;
        }
        if (details.message.startsWith('P3_EDIT ')) {
          try {
            const state = JSON.parse(details.message.slice('P3_EDIT '.length));
            edit = state.content ?? state;
            if (saveTo) {
              if (saving) return;
              saving = true;
              contents.send('save-request');
              return;
            }
            await finish({ open: opened, content: edit, memory: metrics() }, true);
          } catch { await finish({ error: 'smoke assertion failed' }, false); }
          return;
        }
        if (details.message.startsWith('P2_EDIT ')) {
          try {
            const state = JSON.parse(details.message.slice('P2_EDIT '.length));
            if (undoing) {
              const undone = editArg === 'pages' || editArg === 'assemble' ? state?.pages?.count === (opened as { pageCount?: number }).pageCount
                : editArg === 'highlight' ? state?.annots?.Highlight === 0
                : editArg === 'freetext' ? state?.annots?.FreeText === 0
                : editArg === 'form' ? state?.fields?.values?.size === ''
                : editArg === 'nav' ? navUndone(state)
                : editArg === 'decorate' ? decorateUndone(state)
                : editArg === 'shapes' ? shapesUndone(state)
                : state.notes === 0;
              await finish({ open: opened, edit, undo: state, memory: metrics() }, undone);
              return;
            }
            if (smokePasswords().some(pw => details.message.includes(pw))) {
              await finish({ error: 'password leaked in P2_EDIT', edit: state }, false);
              return;
            }
            edit = state;
            if (editArg === 'encrypt' && !(state?.security?.mode === 'aes-256' && Array.isArray(state?.security?.permissions) && state.security.permissions.includes('print') && state.security.permissions.includes('annotate'))) {
              await finish({ open: opened, edit, memory: metrics() }, false);
              return;
            }
            if (editArg === 'encrypt-keep' && state?.security?.mode !== 'keep') {
              await finish({ open: opened, edit, memory: metrics() }, false);
              return;
            }
            if (editArg === 'highlight' && state?.annots?.Highlight !== 1) {
              await finish({ open: opened, edit, memory: metrics() }, false);
              return;
            }
            const openCount = (opened as { pageCount?: number }).pageCount;
            if (editArg === 'pages' && !(typeof openCount === 'number' && state?.pages?.count === openCount - 1 && state?.pages?.rotate?.[1] === 90)) {
              await finish({ open: opened, edit, memory: metrics() }, false);
              return;
            }
            const filled = state?.fields?.values;
            if (editArg === 'form' && !(filled?.name === 'PDF4All P2 form' && filled?.agree === true && filled?.choice === 'two' && filled?.size === 'M')) {
              await finish({ open: opened, edit, memory: metrics() }, false);
              return;
            }
            if (editArg === 'nav' && !navEditOk(state)) {
              await finish({ open: opened, edit, memory: metrics() }, false);
              return;
            }
            if (editArg === 'assemble' && !(typeof openCount === 'number' && assembleOk(state, openCount))) {
              await finish({ open: opened, edit, memory: metrics() }, false);
              return;
            }
            if (editArg === 'decorate' && !(typeof openCount === 'number' && decorateOk(state, openCount, basename(openArg ?? '') === 'layers.pdf'))) {
              await finish({ open: opened, edit, memory: metrics() }, false);
              return;
            }
            if (editArg === 'shapes' && !shapesOk(state)) {
              await finish({ open: opened, edit, memory: metrics() }, false);
              return;
            }
            if (editArg === 'extract') {
              if (!extractPagesOk(state)) {
                await finish({ open: opened, edit, memory: metrics() }, false);
                return;
              }
              if (saveTo) {
                const target = await inspectRegularFile(saveTo);
                if (target && target.size >= 5) {
                  const after = await inspectRegularFile(openArg as string);
                  const afterHash = await fileSha256(openArg as string);
                  const sourceOk = !!baseline && after?.mtimeMs === baseline.mtimeMs && afterHash === baseline.hash;
                  await finish({ open: opened, edit, sourceUnchanged: sourceOk, memory: metrics() }, sourceOk);
                }
                return;
              }
              await finish({ open: opened, edit, memory: metrics() }, true);
              return;
            }
            if (editArg !== undefined && editArg.startsWith('export-')) {
              const want = editArg.slice('export-'.length);
              if (state?.export?.kind !== want) {
                await finish({ open: opened, edit, memory: metrics() }, false);
                return;
              }
              if (saveTo) {
                try {
                  const buf = await readFile(saveTo);
                  await finish({ open: opened, edit, memory: metrics() }, exportMagicOk(editArg, buf));
                } catch {
                  await finish({ open: opened, edit, memory: metrics() }, false);
                }
                return;
              }
              await finish({ open: opened, edit, memory: metrics() }, false);
              return;
            }
            if (editArg === 'encrypt' || editArg === 'encrypt-keep') return; // renderer smoke already saveAs()
            if (saveTo) {
              if (saving) return;
              saving = true;
              contents.send('save-request');
              return;
            }
            if (saveInPlaceArg) {
              if (saving) return;
              saving = true;
              contents.send('save-request', { inPlace: true });
              return;
            }
            undoing = true;
            contents.send('edit-request', { op: 'undo' });
          } catch { await finish({ error: 'smoke assertion failed' }, false); }
          return;
        }
        if (saveTo && details.message.startsWith('P1_SAVE ')) {
          try {
            const save = JSON.parse(details.message.slice('P1_SAVE '.length));
            const after = await inspectRegularFile(openArg);
            const afterHash = await fileSha256(openArg);
            const target = await inspectRegularFile(saveTo);
            const sourceOk = !!baseline && after?.mtimeMs === baseline.mtimeMs && afterHash === baseline.hash;
            if (editArg === 'extract') {
              const ok = extractPagesOk((edit ?? {}) as { extract?: { pages?: unknown } }) && !!target && target.size >= 5 && sourceOk;
              await finish({ open: opened, ...(edit ? { edit } : {}), save, sourceUnchanged: sourceOk, memory: metrics() }, ok);
              return;
            }
            const nameOk = save.name === basename(saveTo);
            const sizeOk = !!target && target.size === save.bytes;
            const ok = nameOk && sizeOk && sourceOk && save.validated === true;
            await finish({ open: opened, ...(edit ? { edit } : {}), ...(editArg === 'content' && edit ? { content: edit } : {}), save, sourceUnchanged: sourceOk, memory: metrics() }, ok);
          } catch { await finish({ error: 'smoke assertion failed' }, false); }
          return;
        }
        if (saveInPlaceArg && details.message.startsWith('P1_SAVE ')) {
          try {
            const save = JSON.parse(details.message.slice('P1_SAVE '.length));
            const afterHash = await fileSha256(openArg);
            const afterMode = (await stat(openArg)).mode;
            const changed = !!baseline && afterHash !== baseline.hash;
            const modeOk = !!baseline && (afterMode & 0o777) === (baseline.mode & 0o777);
            const ok = changed && modeOk && save.validated === true;
            await finish({ open: opened, edit, save, memory: metrics() }, ok);
          } catch { await finish({ error: 'smoke assertion failed' }, false); }
          return;
        }
        if (printTo && printed && details.message.startsWith('P1_PRINT ')) {
          if (!opened) return;
          try {
            const print = JSON.parse(details.message.slice('P1_PRINT '.length));
            const outcome = await printed;
            const target = await inspectRegularFile(printTo);
            const header = target ? await readFile(printTo) : undefined;
            const magic = !!header && header.subarray(0, 5).toString() === '%PDF-';
            const pagesOk = typeof print.pages === 'number' && print.pages > 0;
            const ok = outcome.ok === true && !!target && target.size >= 1024 && magic && pagesOk;
            await finish({ open: opened, print, printedBytes: target?.size ?? 0, memory: metrics() }, ok);
          } catch { await finish({ error: 'smoke assertion failed' }, false); }
          return;
        }
        if (expectThumbs && details.message.startsWith('P1_THUMBS ')) {
          try { thumbs = JSON.parse(details.message.slice('P1_THUMBS '.length)); }
          catch { await finish({ error: 'smoke assertion failed' }, false); return; }
          if (!opened || saveTo) return;
          const ok = typeof (opened as { pageCount?: unknown }).pageCount === 'number' && (opened as { pageCount: number }).pageCount >= 1;
          await finish({ open: opened, thumbs, memory: metrics() }, ok);
          return;
        }
        if (!details.message.startsWith('P1_RESULT ')) return;
        try {
          const open = JSON.parse(details.message.slice('P1_RESULT '.length));
          if (!pushed) {
            pushed = true;
            await deliverOpen(contents, openArg);
            return;
          }
          if (open.name !== basename(openArg)) return;
          opened = open;
          thumbs = undefined;
          if (editArg) {
            if (editing) return;
            editing = true;
            contents.send('edit-request', smokeEditPayload());
            return;
          }
          if (saveInPlaceArg) {
            if (saving) return;
            saving = true;
            contents.send('save-request', { inPlace: true });
            return;
          }
          if (saveTo) {
            if (saving) return;
            saving = true;
            contents.send('save-request');
            return;
          }
          if (printTo) {
            if (printing) return;
            printing = true;
            contents.send('print-request', { range: printRangeArg ?? '' });
            return;
          }
          if (expectThumbs) return;
          const ok = typeof open.pageCount === 'number' && open.pageCount >= 1;
          await finish({ open, memory: metrics() }, ok);
        } catch { await finish({ error: 'smoke assertion failed' }, false); }
      });
    } else if (newFromImages) {
      let sent = false;
      contents.on('console-message', async details => {
        if (details.message.startsWith('P0_ERROR')) {
          await finish({ error: details.message }, false);
          return;
        }
        if (details.message.startsWith('P2_NEW ')) {
          try {
            const created = JSON.parse(details.message.slice('P2_NEW '.length));
            const target = saveToArg ? await inspectRegularFile(saveToArg) : undefined;
            const ok = typeof created?.pages === 'number' && created.pages >= 1 && typeof created?.bytes === 'number' && !!target && target.size >= 5;
            await finish({ new: created, memory: metrics() }, ok);
          } catch { await finish({ error: 'smoke assertion failed' }, false); }
          return;
        }
        if (!details.message.startsWith('P0_RESULT ') || sent) return;
        sent = true;
        contents.send('edit-request', { op: 'new-from-images' });
      });
    } else {
      window.webContents.on('console-message', async details => {
        if (details.message.startsWith('P0_ERROR')) {
          await finish({ error: details.message }, false);
        }
        if (!details.message.startsWith('P0_RESULT ')) return;
        try {
          const renderer = JSON.parse(details.message.slice('P0_RESULT '.length));
          const isolation = await window.webContents.executeJavaScript(`(() => {
          const canvas = document.querySelector('canvas:not(#thumbs canvas)');
          const pixels = canvas?.getContext('2d')?.getImageData(0, 0, canvas.width, canvas.height).data;
          let nonwhitePixels = 0;
          if (pixels) for (let i = 0; i < pixels.length; i += 4) {
            if (pixels[i] < 250 || pixels[i + 1] < 250 || pixels[i + 2] < 250) nonwhitePixels++;
          }
          return {
            nodeAbsent: typeof process === 'undefined' && typeof require === 'undefined',
            // The page view holds one canvas (the compare panes and the signature pad have their own, elsewhere).
            canvasCount: document.querySelectorAll('#viewport canvas').length,
            width: canvas?.width, height: canvas?.height, nonwhitePixels
          };
        })()`);
          const workerProbe = await window.webContents.executeJavaScript(`new Promise((resolve, reject) => {
          const probe = new Worker('./worker.js', { type: 'module' });
          const timer = setTimeout(() => { probe.terminate(); reject(new Error('worker probe timeout')); }, 5000);
          probe.onerror = () => { clearTimeout(timer); probe.terminate(); reject(new Error('worker probe failed')); };
          probe.onmessage = ({ data }) => {
            if (data.ready) probe.postMessage({ id: 7, probe: true });
            if (data.id === 7 && data.probe) { clearTimeout(timer); probe.terminate(); resolve(data); }
          };
        })`);
          const roundtrip = await window.webContents.executeJavaScript(`new Promise((resolve, reject) => {
          const w = new Worker('./worker.js', { type: 'module' });
          const timer = setTimeout(() => { w.terminate(); reject(new Error('worker roundtrip timeout')); }, 15000);
          w.onerror = () => { clearTimeout(timer); w.terminate(); reject(new Error('worker roundtrip failed')); };
          w.onmessage = ({ data }) => {
            if (data.ready) {
              fetch('/fixtures/form.pdf').then(r => { if (!r.ok) throw new Error('form HTTP ' + r.status); return r.arrayBuffer(); })
                .then(pdf => w.postMessage({ id: 8, roundtrip: true, pdf }, [pdf]))
                .catch(err => { clearTimeout(timer); w.terminate(); reject(err); });
              return;
            }
            if (data.id === 8 && data.roundtrip) {
              clearTimeout(timer); w.terminate();
              const savedBytes = data.savedBytes ? Array.from(new Uint8Array(data.savedBytes)) : [];
              resolve({ ...data, savedBytes });
            }
          };
        })`);
          const noUnexpectedRequests = blockedRequests.length === 0;
          // Deliberate request tests the session firewall with networking available.
          let networkProbeDenied = false;
          try { await isolated.fetch('https://example.invalid/pdf4all-network-probe'); }
          catch { networkProbeDenied = true; }
          const rendererProcess = app.getAppMetrics().find(metric => metric.pid === window.webContents.getOSProcessId());
          const sandbox = !!rendererProcess && osSandboxed(rendererProcess) === true;
          const nonblank = isolation.nonwhitePixels > 100 && isolation.nonwhitePixels < isolation.width * isolation.height / 2;
          const square = Array.isArray(roundtrip.readback?.annots) && roundtrip.readback.annots.find((annot: { type?: string; contents?: string }) => annot.type === 'Square' && annot.contents === 'PDF4All P0 note');
          // The round trip ran on the engine this build ships (scripts/build.mjs, TPDF_ENGINE).
          const roundtripOk = roundtrip.ok === true && roundtrip.engine === ENGINE && roundtrip.readback?.fieldValue === 'PDF4All P0' && !!square;
          const helloOk = isolation.width === renderer.pixelWidth && isolation.height === renderer.pixelHeight && renderer.width === 612 && renderer.height === 792 && JSON.stringify(renderer).includes('PDF4All');
          const ok = noUnexpectedRequests && networkProbeDenied && blockedRequests.includes('https:') && sandbox && isolation.nodeAbsent && isolation.canvasCount === 1 && nonblank && workerProbe.nodeAbsent && workerProbe.cspBlocked && helloOk && roundtripOk;
          const savedList = Array.isArray(roundtrip.savedBytes) ? roundtrip.savedBytes as number[] : [];
          const { savedBytes: _savedBytes, ...roundtripReport } = roundtrip;
          if (reportPath) {
            await writeFile(`${reportPath}.png`, (await window.webContents.capturePage()).toPNG());
            if (savedList.length) await writeFile(`${reportPath}.roundtrip.pdf`, Buffer.from(savedList));
          }
          await finish({ engine: ENGINE, renderer, isolation, sandbox, noUnexpectedRequests, networkProbeDenied, workerProbe, roundtrip: roundtripReport, fixture, openMs: renderer.openMs, firstPageRenderMs: renderer.firstPageRenderMs, memory: metrics() }, ok);
        } catch { await finish({ error: 'smoke assertion failed' }, false); }
      });
    }
  }
  await window.loadURL('app://pdf4all/index.html');
  windowReady = true;
  await flushQueuedOpens();
  if (openArg !== undefined && !smoke) await deliverOpen(window.webContents, openArg);
}).catch(() => smoke ? void finish({ error: 'startup failed' }, false) : app.exit(1));

// ------------------------------------------------------------------ --drive

type DriveLine = { cmd?: string; expect?: unknown; [key: string]: unknown };

/** `expect` is a subset match: every key in `want` must deep-equal the same key in `got`. */
function matches(got: unknown, want: unknown): boolean {
  if (want === null || typeof want !== 'object') return got === want;
  if (Array.isArray(want)) return Array.isArray(got) && want.length === got.length && want.every((item, i) => matches(got[i], item));
  if (got === null || typeof got !== 'object') return false;
  return Object.entries(want).every(([key, value]) => matches((got as Record<string, unknown>)[key], value));
}

function driveSecrets(line: DriveLine): string[] {
  const contents = line.contents;
  if (line.cmd === 'password' && typeof contents === 'string') return [contents];
  if (line.cmd !== 'encrypt' && line.cmd !== 'encrypt-keep' && line.cmd !== 'sign') return [];
  const body = typeof contents === 'string' ? (() => { try { return JSON.parse(contents); } catch { return {}; } })() : contents;
  // P5 U3 §2.6: a certificate sign step's password is a secret like an encrypt step's.
  return (line.cmd === 'sign' ? ['password'] : ['user', 'owner', 'password']).map(key => (body as Record<string, unknown> | null)?.[key]).filter((v): v is string => typeof v === 'string' && v.length > 0);
}

/**
 * One script line per step: `{"cmd": <edit-request op>, "contents"?: string|object, "expect"?: {...}}`
 * plus `open`/`save-to`/`print-to` (`path`), `save-in-place`, `wait` (`ms`), `state`; `to`/`from`/`images`
 * answer the native dialogs. A bare `{"expect": {...}}` re-checks the last state. `${OUT}` in any
 * string is the output directory (`--drive-out`, default a temp dir). Exit 0 iff every expect matched.
 * `new` (contents `{size}`) opens a blank Untitled.pdf; `close` (or the end of the lines) finishes.
 * `--drive=-` (contract-agent-session.md D1) reads the lines from stdin instead, one at a time, and
 * writes one `STEP <json>` line to stdout per line read; a line that is not a JSON object is
 * answered `STEP {"error":"bad line: …"}` and the session goes on (a file script fails fast).
 */
async function startDrive(contents: WebContents, scriptPath: string): Promise<void> {
  const out = argvValue('--drive-out') ?? tmpdir();
  // `${OUT}/…` lands inside JSON strings: the path takes this platform's separators (a path in STATE is native, so an
  // `expect` matches on Windows too) and JSON escaping for its backslashes.
  const withOut = (text: string) => text.replace(/\$\{OUT\}([^"\\\r\n]*)/g, (_all, rest: string) => JSON.stringify(out + (sep === '/' ? rest : rest.replaceAll('/', sep))).slice(1, -1));
  const live = scriptPath === '-';
  let lines: DriveLine[] = [];
  if (!live) {
    try {
      const text = withOut(await readFile(scriptPath, 'utf8'));
      lines = text.split('\n').map(line => line.trim()).filter(line => line && !line.startsWith('#')).map(line => JSON.parse(line) as DriveLine);
    } catch (err) {
      return void finish({ error: `drive script: ${(err as Error).message}` }, false);
    }
  }
  const steps: Record<string, unknown>[] = [];
  const secrets: string[] = [];
  let waiting: ((state: unknown) => void) | undefined;
  let ready: (() => void) | undefined;
  const fixtureOpened = new Promise<void>(resolve => { ready = resolve; });
  contents.on('console-message', details => {
    const { message } = details;
    if (message.startsWith('P0_ERROR')) void finish({ error: message, out, steps }, false);
    if (message.startsWith('P1_RESULT ')) ready?.();
    if (!message.startsWith('STATE ')) return;
    if (secrets.some(secret => message.includes(secret))) return void finish({ error: 'password leaked in STATE', out, steps }, false);
    const cb = waiting;
    waiting = undefined;
    try { cb?.(JSON.parse(message.slice('STATE '.length))); } catch { void finish({ error: 'unparseable STATE', out, steps }, false); }
  });
  const nextState = () => new Promise<unknown>((resolveState, reject) => {
    waiting = resolveState;
    setTimeout(() => reject(new Error('step timeout')), 60_000).unref();
  });
  const stringOf = (value: unknown) => typeof value === 'string' ? value : undefined;
  const pathOf = (value: unknown) => { const text = stringOf(value); return text ? resolve(text) : undefined; };
  await fixtureOpened;
  let failed = false;
  let last: unknown;
  const emit = (step: Record<string, unknown>) => { if (live) process.stdout.write(`STEP ${JSON.stringify(step)}\n`); };
  // readline is made only now, so no stdin line arrives before the loop reads it (stdin stays paused until then).
  // Windows: Electron's main `process.stdin` is an empty stream for a piped GUI process (it ends at once). The pipe on
  // fd 0 reads fine as a socket; an fs stream there would block `app.exit()` on its pending read while stdin stays open.
  const stdin = () => process.platform === 'win32' ? new Socket({ fd: 0, readable: true, writable: false }) : process.stdin;
  const source: AsyncIterable<string> | Iterable<DriveLine> = live ? createInterface({ input: stdin(), crlfDelay: Infinity }) : lines;
  for await (const raw of source) {
    let line: DriveLine;
    if (typeof raw === 'string') {
      const text = raw.trim();
      if (!text || text.startsWith('#')) continue;
      try {
        const parsed: unknown = JSON.parse(withOut(text));
        if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) throw new Error('not a JSON object');
        line = parsed as DriveLine;
      } catch (err) {
        const bad = { error: `bad line: ${(err as Error).message}` };
        steps.push(bad);
        emit(bad);
        continue;
      }
    } else line = raw;
    if (line.cmd === 'close') break;
    const { cmd, expect, ...args } = line;
    const step: Record<string, unknown> = { ...(cmd !== undefined ? { cmd } : {}), ...args };
    if (cmd !== undefined) {
      const lineSecrets = driveSecrets(line);
      secrets.push(...lineSecrets);
      if (lineSecrets.length) step.contents = '[redacted]';
      // draw-signature review: the stroke points never reach the report or a STEP line.
      if (cmd === 'sign-draw' && args.contents !== undefined) step.contents = '[redacted]';
      if (args.to !== undefined) saveToArg = pathOf(args.to);
      if (args.from !== undefined) insertFromArg = pathOf(args.from);
      if (args.folder !== undefined) batchDirArg = pathOf(args.folder);
      if (args.images !== undefined) imagesArg = (Array.isArray(args.images) ? args.images : String(args.images).split(',')).map(item => resolve(String(item).trim())).join(',');
      const pending = nextState();
      if (cmd === 'open') void deliverOpen(contents, pathOf(args.path) ?? '');
      else if (cmd === 'new') {
        // Contents are optional; present ones must be an object (or its JSON) whose `size` pageSizeOf accepts.
        let body: unknown = args.contents;
        if (typeof body === 'string') { try { body = JSON.parse(body); } catch { body = null; } }
        void newBlank(contents, body === undefined ? undefined : body && typeof body === 'object' ? (body as { size?: unknown }).size : null);
      }
      else if (cmd === 'save-to') { saveToArg = pathOf(args.path); contents.send('save-request'); }
      else if (cmd === 'save-in-place') { saveToArg = pathOf(args.path); contents.send('save-request', { inPlace: true }); }
      else if (cmd === 'print-to') { printToArg = pathOf(args.path); contents.send('print-request', { range: stringOf(args.range) ?? '' }); }
      else if (cmd === 'wait') setTimeout(() => { if (!contents.isDestroyed()) contents.send('edit-request', { op: 'state' }); }, Number(args.ms) || 0);
      else {
        // A drive `batch` without contents is refused by the renderer, never the menu's dialog (§9.9).
        const body = args.contents ?? (cmd === 'batch' ? '' : cmd === 'content' ? '{}' : undefined);
        contents.send('edit-request', { op: cmd, ...(body === undefined ? {} : { contents: typeof body === 'string' ? body : JSON.stringify(body) }) });
      }
      try { last = await pending; } catch (err) {
        const timedOut = { ...step, error: (err as Error).message };
        steps.push(timedOut);
        emit(timedOut);
        return void finish({ error: 'step timeout', out, steps }, false);
      }
      step.state = last;
    }
    if (expect !== undefined) {
      step.expect = expect;
      step.ok = matches(last, expect);
      if (!step.ok) failed = true;
    }
    steps.push(step);
    emit(step);
  }
  await finish({ out, steps, memory: metrics() }, !failed);
}

app.on('window-all-closed', () => app.quit());
