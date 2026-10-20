import type { AnnotsReply, FieldsReply, JournalState, Line, OpenReply, PageSize, RedactApplyReply, RedactInspectReply, RedactPreview, RenderReply, SaveReply, SearchReply, SessionRequest } from '../../web/protocol.js';
import { highlightQuads, pagePoint, placeSignature, type Point } from './geometry.js';
import { Boxes, boxAt, boxFrom, flattenQuestion, pagesNamed, plural, redactedName, refusalMessage, SANITIZE_LABELS, type Box, type Rect } from './redact.js';
import { getDocument, listDocuments, putDocument, putDocuments, removeDocument, type LocalDocument } from './storage.js';
import { setupScanner } from './scan.js';
import { sharePDF } from './export.js';
import { Capacitor } from '@capacitor/core';
import { App } from '@capacitor/app';

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const canvas = $<HTMLCanvasElement>('page'), overlay = $<HTMLCanvasElement>('overlay');
const context = canvas.getContext('2d')!, ink = overlay.getContext('2d')!;
const picker = $<HTMLInputElement>('picker');
const worker = new Worker('engine/worker.js', { type: 'module' });
let engineReady: () => void;
let engineStartupFailed: (error: Error) => void;
const ready = new Promise<void>((resolve, reject) => { engineReady = resolve; engineStartupFailed = reject; });
const startupTimer = setTimeout(() => {
  workerFailed = true;
  engineStartupFailed(new Error('The PDF engine could not start. Reopen the app and try again.'));
}, 30_000);
type WithoutId<T> = T extends unknown ? Omit<T, 'id'> : never;
type Request = WithoutId<SessionRequest>;
let sequence = 0;
const pending = new Map<number, { resolve: (reply: unknown) => void; reject: (error: Error) => void }>();
let workerFailed = false;
/** A worker `{ok: false}` reply. `message` is the engine's own text (shown as before by other tools); redaction words its `reason`. */
class EngineRefusal extends Error {
  constructor(readonly reply: Record<string, unknown>) { super(String(reply.error || 'This PDF operation could not be completed.')); }
  get reason() { return typeof this.reply.reason === 'string' ? this.reply.reason : undefined; }
}
worker.onmessage = ({ data }) => {
  if (data.ready) { clearTimeout(startupTimer); engineReady(); return; }
  const task = pending.get(data.id);
  if (!task) return;
  pending.delete(data.id);
  if (data.ok) task.resolve(data);
  else task.reject(new EngineRefusal(data));
};
worker.onerror = () => {
  workerFailed = true;
  const error = new Error('The PDF engine stopped. Reopen the app to recover your last saved copy.');
  clearTimeout(startupTimer); engineStartupFailed(error);
  pending.forEach(task => task.reject(error)); pending.clear(); tell(error.message);
};
async function rpc<T>(request: Request, transfer: Transferable[] = []): Promise<T> {
  await ready;
  if (workerFailed) return Promise.reject(new Error('Reopen the app to restart the PDF engine.'));
  const id = ++sequence;
  return new Promise<T>((resolve, reject) => {
    pending.set(id, { resolve: reply => resolve(reply as T), reject });
    try { worker.postMessage({ ...request, id }, transfer); }
    catch (error) { pending.delete(id); reject(error); }
  });
}

type Mode = 'read' | 'highlight' | 'ink' | 'note' | 'signature' | 'redact';
let mode: Mode = 'read', current: LocalDocument | null = null, pages: PageSize[] = [], page = 0, zoom = 1;
let lines: Line[] = [], dirty = false, unsavedLocalCopy = false, busy = false, statusTimer: ReturnType<typeof setTimeout>;
let canAnnotate = true, canFill = true, canRedact = true, signedDocument = false;
let lastJournal: JournalState = { revision: 0, dirty: false, canUndo: false, canRedo: false };
// Redact tool: boxes held in memory (sent as `regions`), the selected box, the "Show what's underneath" toggle,
// whether a finger is down, and the original of the redacted copy on screen.
const boxes = new Boxes();
let selected: number | null = null, peek = false, pressing = false;
let redactPress: { start: Point; x: number; y: number; on: Box | null; moved: boolean } | null = null;
let redactedFrom: { original: LocalDocument; count: number; flattened: number[]; unsigned: boolean; overRedacted: boolean } | null = null;
/** The password the open document was unlocked with, kept only while it is open: its redacted copy is encrypted the same way. */
let documentPassword: string | null = null;
/** The last redaction removed only Adobe Reader usage rights, not signatures (the banner says which). */
let rightsOnly = false;
const shade = $<HTMLCanvasElement>('redactions'), shading = shade.getContext('2d')!;
function tell(text: string) {
  clearTimeout(statusTimer); $('status').textContent = text; $('status').hidden = false;
  statusTimer = setTimeout(() => { $('status').hidden = true; }, 6000);
}
async function run(action: () => Promise<void>) {
  if (busy) return;
  busy = true; document.body.classList.add('busy');
  document.body.setAttribute('aria-busy', 'true');
  $('field-list').inert = true;
  $('library').inert = true;
  try { await action(); } catch (error) { tell(error instanceof Error ? error.message : String(error)); }
  finally { busy = false; document.body.classList.remove('busy'); document.body.setAttribute('aria-busy', 'false'); $('field-list').inert = false; $('library').inert = false; }
}
function journal(state: JournalState) {
  // An edit can change what lies under a box: ask redact-inspect again when it is next needed.
  if (state.revision !== lastJournal.revision) for (const box of boxes.all) box.inspect = undefined;
  dirty = state.dirty; lastJournal = state; updateUndo();
  $('save-state').textContent = dirty ? 'Unsaved changes' : unsavedLocalCopy ? 'Not saved on device' : 'Saved on device';
}
/** In the Redact tool, Undo and Redo step through the boxes; elsewhere through the document's edits. */
function updateUndo() {
  const boxesOwnUndo = mode === 'redact';
  $<HTMLButtonElement>('undo').disabled = boxesOwnUndo ? !boxes.canUndo : !lastJournal.canUndo;
  $<HTMLButtonElement>('redo').disabled = boxesOwnUndo ? !boxes.canRedo : !lastJournal.canRedo;
}
function setMode(next: Mode) {
  if (next !== 'redact') { selected = null; pressing = false; }
  mode = next;
  document.querySelectorAll<HTMLButtonElement>('[data-mode]').forEach(button => button.setAttribute('aria-pressed', String(button.dataset.mode === mode)));
  overlay.style.pointerEvents = mode === 'read' ? 'none' : 'auto';
  overlay.style.cursor = mode === 'redact' ? 'crosshair' : '';
  $('hint').hidden = mode === 'redact'; $('redact-bar').hidden = mode !== 'redact';
  $('hint').textContent = ({ read: 'Swipe to scroll. Pinch or use + and − to zoom.', highlight: 'Drag across text to highlight whole lines.', ink: 'Draw on the page. Switch to Read to scroll.', note: 'Tap the page to add a note.', signature: 'Tap the page to place your signature.', redact: '' })[mode];
  ink.clearRect(0, 0, overlay.width, overlay.height);
  updateUndo(); updateRedactUi(); paintRedactions();
}
async function renderPage() {
  const size = pages[page];
  const width = Math.max(180, $('viewport').clientWidth - 28);
  const cssScale = Math.min(width / size.width, 1.3) * zoom;
  const scale = Math.max(.25, Math.min(4, cssScale * Math.min(devicePixelRatio, 2), 2400 / Math.max(size.width, size.height)));
  const result = await rpc<RenderReply>({ op: 'render', page, scale });
  canvas.width = result.pixelWidth; canvas.height = result.pixelHeight;
  context.putImageData(new ImageData(new Uint8ClampedArray(result.pixels), result.pixelWidth, result.pixelHeight), 0, 0);
  const cssWidth = size.width * cssScale, cssHeight = size.height * cssScale;
  canvas.style.width = `${cssWidth}px`; canvas.style.height = `${cssHeight}px`;
  overlay.width = result.pixelWidth; overlay.height = result.pixelHeight;
  overlay.style.width = `${cssWidth}px`; overlay.style.height = `${cssHeight}px`;
  shade.width = result.pixelWidth; shade.height = result.pixelHeight;
  shade.style.width = `${cssWidth}px`; shade.style.height = `${cssHeight}px`;
  lines = result.lines;
  $('page-text').textContent = result.text;
  canvas.setAttribute('aria-label', `PDF page ${page + 1} of ${pages.length}`);
  $('page-label').textContent = `${page + 1} / ${pages.length}`;
  $<HTMLButtonElement>('prev').disabled = page === 0;
  $<HTMLButtonElement>('next').disabled = page === pages.length - 1;
  paintRedactions();
  if (mode === 'redact' && peek) { await inspectBoxes(boxes.onPage(page)); paintRedactions(); }
}
async function refreshLibrary() {
  const docs = (await listDocuments()).sort((a, b) => b.updated - a.updated);
  $('files').replaceChildren(); $('empty').hidden = docs.length > 0;
  $('file-count').textContent = `${docs.length} document${docs.length === 1 ? '' : 's'}`;
  for (const doc of docs) {
    const row = document.createElement('div'); row.className = 'file-row';
    const button = document.createElement('button'); button.className = 'file-open';
    const icon = document.createElement('span'); icon.className = 'file-icon'; icon.textContent = 'PDF';
    const detail = document.createElement('span'); detail.className = 'file-detail';
    const name = document.createElement('strong'); name.textContent = doc.name;
    const meta = document.createElement('small'); meta.textContent = `${new Date(doc.updated).toLocaleDateString()} · ${(doc.size / 1024 / 1024).toFixed(1)} MB`;
    detail.append(name, meta); button.append(icon, detail);
    button.onclick = () => void run(async () => {
      const saved = await getDocument(doc.id);
      if (!saved) throw new Error('This copy is no longer on the device.');
      await openDocument(saved);
    });
    const remove = document.createElement('button'); remove.className = 'delete-file'; remove.textContent = '×';
    remove.setAttribute('aria-label', `Remove local copy of ${doc.name}`);
    remove.onclick = () => void run(async () => {
      if (confirm(`Remove the saved copy of “${doc.name}” from this app?`)) { await removeDocument(doc.id); await refreshLibrary(); }
    });
    row.append(button, remove); $('files').append(row);
  }
}
function ask(dialog: HTMLDialogElement): Promise<string> {
  dialog.returnValue = ''; dialog.showModal();
  return new Promise(resolve => dialog.addEventListener('close', () => resolve(dialog.returnValue), { once: true }));
}
/** Opens `doc`; `password` (the one the redacted copy's original was unlocked with) is tried before asking. */
async function openDocument(doc: LocalDocument, password: string | null = null) {
  const pdf = doc.bytes.slice(0);
  let result = await rpc<OpenReply>({ op: 'open', pdf, name: doc.name }, [pdf]);
  let unlockedWith: string | null = null;
  if (result.info.needsPassword && password) {
    const unlocked = await rpc<OpenReply & { authenticated: boolean }>({ op: 'authenticate', password });
    if (unlocked.authenticated) { result = unlocked; unlockedWith = password; }
  }
  while (result.info.needsPassword) {
    $<HTMLInputElement>('password').value = '';
    if (await ask($<HTMLDialogElement>('password-dialog')) !== 'unlock') {
      await rpc({ op: 'close' }); documentPassword = null;
      // The engine closed the document on screen too: back to the library rather than a page with no session.
      if (current) {
        current = null; dirty = false; unsavedLocalCopy = false; boxes.clear(); selected = null; showRedactedBanner(null); updateRedactUi();
        $('editor').hidden = true; $('library').hidden = false; await refreshLibrary();
      }
      return;
    }
    const typed = $<HTMLInputElement>('password').value;
    try {
      const unlocked = await rpc<OpenReply & { authenticated: boolean }>({ op: 'authenticate', password: typed });
      if (!unlocked.authenticated) { tell('That password did not unlock the PDF. Try again.'); continue; }
      result = unlocked; unlockedWith = typed;
    } finally { $<HTMLInputElement>('password').value = ''; }
  }
  documentPassword = unlockedWith;
  current = doc; unsavedLocalCopy = false; pages = result.pages; page = 0; zoom = 1;
  boxes.clear(); selected = null; showRedactedBanner(null);
  canAnnotate = result.info.permissions.annotate && !result.info.certified && result.info.signatures === 0;
  canFill = result.info.permissions.form && !result.info.certified && result.info.signatures === 0;
  // Redaction writes a new file, so a signed original stays as it is (the engine offers an unsigned copy).
  signedDocument = result.info.signatures > 0 || result.info.certified;
  canRedact = result.info.permissions.edit;
  $<HTMLButtonElement>('copy-text').disabled = !result.info.permissions.copy;
  document.querySelectorAll<HTMLButtonElement>('[data-mode]').forEach(button => { button.disabled = button.dataset.mode === 'redact' ? !canRedact : button.dataset.mode !== 'read' && !canAnnotate; });
  $<HTMLButtonElement>('sign').disabled = !canAnnotate;
  $<HTMLButtonElement>('forms').disabled = !canFill;
  $('name').textContent = doc.name;
  journal({ revision: 0, dirty: false, canUndo: false, canRedo: false }); setMode('read');
  $('library').hidden = true; $('editor').hidden = false;
  await renderPage();
  if (result.info.signatures || result.info.certified) tell('This signed PDF is open for reading.');
}
async function serialized(): Promise<ArrayBuffer> {
  const result = await rpc<SaveReply>({ op: 'save' });
  const validated = await rpc<{ bytes: ArrayBuffer }>({ op: 'validate', pdf: result.bytes }, [result.bytes]);
  return validated.bytes;
}
async function saveLocal() {
  if (!current) return;
  const bytes = dirty ? await serialized() : current.bytes;
  // A redacted copy that is edited is no longer just the redacted copy (its shared name says so).
  const saved = { ...current, bytes, updated: Date.now(), ...(dirty ? { redacted: false } : {}) };
  await putDocument(saved); current = saved; unsavedLocalCopy = false;
  journal(await rpc<JournalState>({ op: 'saved' }));
}
async function mutate(request: Request) {
  journal(await rpc<JournalState>(request));
  await renderPage();
  // Commit to device storage before acknowledging a saved state in the engine.
  try { await saveLocal(); } catch { tell('Your edit is open but could not be saved. Use Share to export before closing.'); }
}
$('import').onclick = () => picker.click();
$('sample').onclick = () => void run(async () => {
  const response = await fetch('welcome.pdf');
  if (!response.ok) throw new Error('The sample could not be opened. Try choosing a PDF from your files.');
  const doc = { id: crypto.randomUUID(), name: 'Welcome to Tandem.pdf', bytes: await response.arrayBuffer(), updated: Date.now() };
  await openDocument(doc);
  try { await putDocument(doc); } catch { unsavedLocalCopy = true; $('save-state').textContent = 'Not saved on device'; throw new Error('Device storage is unavailable. Export your work before closing.'); }
});
picker.onchange = () => {
  const file = picker.files?.[0]; picker.value = '';
  if (!file) return;
  void run(async () => {
    if (file.size > 20 * 1024 * 1024) throw new Error('Choose a PDF smaller than 20 MB for this mobile version.');
    const bytes = await file.arrayBuffer();
    const doc = { id: crypto.randomUUID(), name: file.name, bytes, updated: Date.now() };
    await openDocument(doc);
    if (current?.id === doc.id) {
      try { await putDocument(doc); } catch { unsavedLocalCopy = true; $('save-state').textContent = 'Not saved on device'; tell('Device storage is full or unavailable. Export your work before closing.'); }
    }
  });
};
$('back').onclick = () => void run(async () => {
  if (!await pendingBoxes('leave')) return;
  if ((dirty || unsavedLocalCopy) && !confirm('Leave this PDF and discard changes that could not be saved?')) return;
  await rpc({ op: 'close' }); current = null; documentPassword = null; dirty = false; unsavedLocalCopy = false; showRedactedBanner(null);
  $('editor').hidden = true; $('library').hidden = false; await refreshLibrary();
});
$('save').onclick = () => void run(async () => {
  $<HTMLDialogElement>('menu-dialog').close();
  if (!await pendingBoxes('save')) return;
  await saveLocal(); tell('Saved on this device.');
});
$('share').onclick = () => void run(async () => {
  if (!current || !await pendingBoxes('share')) return;
  // Unedited files can be shared byte-for-byte, including signed/encrypted PDFs.
  // An unchanged redacted copy keeps its name; edits, or boxes drawn on it and shared anyway, mark it edited.
  await sharePDF(dirty ? await serialized() : current.bytes.slice(0), current.name, !(current.redacted && !dirty && !boxes.count));
});
for (const [id, delta] of [['prev', -1], ['next', 1]] as const) $(id).onclick = () => void run(async () => {
  page = Math.max(0, Math.min(pages.length - 1, page + delta)); await renderPage(); $('viewport').scrollTo(0, 0);
});
for (const [id, delta] of [['zoom-in', .25], ['zoom-out', -.25]] as const) $(id).onclick = () => void run(async () => {
  zoom = Math.max(.75, Math.min(3, zoom + delta)); await renderPage();
});
for (const op of ['undo', 'redo'] as const) $(op).onclick = () => void run(async () => {
  if (mode !== 'redact') { await mutate({ op }); return; }
  const box = op === 'undo' ? boxes.undo() : boxes.redo();
  selected = null;
  if (box && box.page !== page) { page = box.page; await renderPage(); }
  updateRedactUi(); paintRedactions();
});
document.querySelectorAll<HTMLButtonElement>('[data-mode]').forEach(button => button.onclick = () => setMode(button.dataset.mode as Mode));
document.querySelectorAll<HTMLButtonElement>('[data-close]').forEach(button => button.onclick = () => $<HTMLDialogElement>(button.dataset.close!).close());
document.querySelectorAll<HTMLAnchorElement>('.privacy a').forEach(link => link.onclick = event => {
  event.preventDefault();
  void run(async () => {
    const response = await fetch(link.href);
    if (!response.ok) throw new Error('The license could not be opened.');
    $('license-title').textContent = link.textContent;
    $('license-text').textContent = await response.text();
    $<HTMLDialogElement>('license-dialog').showModal();
  });
});
$('menu').onclick = () => {
  $<HTMLInputElement>('jump-page').value = String(page + 1);
  $<HTMLInputElement>('jump-page').max = String(pages.length);
  $<HTMLDialogElement>('menu-dialog').showModal();
};
$('jump-form').onsubmit = event => {
  event.preventDefault();
  const requested = Number($<HTMLInputElement>('jump-page').value);
  if (!Number.isInteger(requested) || requested < 1 || requested > pages.length) return;
  $<HTMLDialogElement>('menu-dialog').close();
  void run(async () => { page = requested - 1; await renderPage(); $('viewport').scrollTo(0, 0); });
};
$('find').onclick = () => {
  $<HTMLDialogElement>('menu-dialog').close();
  $('search-results').replaceChildren(); $('search-summary').textContent = '';
  $<HTMLDialogElement>('search-dialog').showModal();
};
$('copy-text').onclick = () => {
  $<HTMLDialogElement>('menu-dialog').close();
  $<HTMLTextAreaElement>('copy-page-text').value = $('page-text').textContent || 'No recognized text on this page.';
  $('copy-text-boxes').hidden = boxes.count === 0;
  $<HTMLDialogElement>('copy-text-dialog').showModal();
};
$('search-form').onsubmit = event => {
  event.preventDefault();
  void run(async () => {
    const query = $<HTMLInputElement>('search-query').value.trim();
    if (!query) return;
    const result = await rpc<SearchReply>({ op: 'search', query, maxHits: 200 });
    $('search-results').replaceChildren();
    $('search-summary').textContent = result.hits.length ? `${result.hits.length}${result.truncated ? '+' : ''} matches. Tap a result to view it.` : 'No matches. Scanned pages may not contain searchable text.';
    result.hits.forEach((hit, index) => {
      const button = document.createElement('button'); button.className = 'search-result'; button.textContent = `Page ${hit.page + 1} · Match ${index + 1}`;
      button.onclick = () => void run(async () => {
        $<HTMLDialogElement>('search-dialog').close(); page = hit.page; setMode('read'); await renderPage();
        const sx = overlay.width / pages[page].width, sy = overlay.height / pages[page].height;
        ink.fillStyle = '#eba82b66';
        for (const quad of hit.quads) {
          ink.beginPath(); ink.moveTo(quad[0] * sx, quad[1] * sy);
          for (const i of [2, 6, 4]) ink.lineTo(quad[i] * sx, quad[i + 1] * sy);
          ink.closePath(); ink.fill();
        }
        const cssScale = canvas.getBoundingClientRect().height / pages[page].height;
        $('viewport').scrollTop = Math.max(0, hit.quads[0][1] * cssScale - 80);
      });
      $('search-results').append(button);
    });
  });
};
async function showAnnotations() {
  const result = await rpc<AnnotsReply>({ op: 'annots', page });
  $('annotation-list').replaceChildren();
  if (!result.items.length) $('annotation-list').textContent = 'No annotations on this page yet.';
  for (const item of result.items) {
    const row = document.createElement('div'); row.className = 'annotation-row';
    const title = document.createElement('strong'); title.textContent = item.subject || item.type;
    const content = document.createElement('p'); content.textContent = item.contents || 'Mark on this page';
    row.append(title, content);
    if (item.editable && canAnnotate) {
      if (item.type === 'Text') {
        const edit = document.createElement('button'); edit.textContent = 'Edit note';
        edit.onclick = () => void run(async () => {
          $<HTMLDialogElement>('annotations-dialog').close();
          document.querySelector('#note-dialog h2')!.textContent = 'Edit note';
          document.querySelector('#note-dialog button[value="add"]')!.textContent = 'Save changes';
          $<HTMLTextAreaElement>('note-text').value = item.contents;
          if (await ask($<HTMLDialogElement>('note-dialog')) === 'add') {
            await mutate({ op: 'annot-set', page, annotId: item.id, contents: $<HTMLTextAreaElement>('note-text').value });
          }
          await showAnnotations(); $<HTMLDialogElement>('annotations-dialog').showModal();
        });
        row.append(edit);
      }
      const remove = document.createElement('button'); remove.textContent = 'Delete';
      remove.onclick = () => void run(async () => {
        await mutate({ op: 'annot-delete', page, annotId: item.id }); await showAnnotations();
      });
      row.append(remove);
    }
    $('annotation-list').append(row);
  }
}
$('annotations').onclick = () => void run(async () => {
  $<HTMLDialogElement>('menu-dialog').close(); await showAnnotations(); $<HTMLDialogElement>('annotations-dialog').showModal();
});

let pointer: number | null = null, stroke: Point[] = [], start: Point = [0, 0];
function point(event: PointerEvent): Point { return pagePoint(event.clientX, event.clientY, overlay.getBoundingClientRect(), pages[page].width, pages[page].height); }
function drawStroke(ctx: CanvasRenderingContext2D, points: Point[], sx: number, sy: number, color: string, width: number) {
  if (points.length < 2) return;
  ctx.strokeStyle = color; ctx.lineWidth = width; ctx.lineCap = 'round'; ctx.lineJoin = 'round'; ctx.beginPath();
  points.forEach(([x, y], i) => i ? ctx.lineTo(x * sx, y * sy) : ctx.moveTo(x * sx, y * sy)); ctx.stroke();
}
overlay.onpointerdown = event => {
  if (mode === 'redact') { redactDown(event); return; }
  if (busy || pointer !== null || mode === 'read' || !canAnnotate) return;
  pointer = event.pointerId; overlay.setPointerCapture(pointer); start = point(event); stroke = [start];
};
overlay.onpointermove = event => {
  if (event.pointerId !== pointer) return;
  if (mode === 'redact') { redactMove(event); return; }
  const p = point(event);
  ink.clearRect(0, 0, overlay.width, overlay.height);
  const sx = overlay.width / pages[page].width, sy = overlay.height / pages[page].height;
  if (mode === 'ink') {
    if (stroke.length < 2048) stroke.push(p);
    drawStroke(ink, stroke, sx, sy, '#365eac', 1.5 * sx);
  } else if (mode === 'highlight') {
    ink.fillStyle = '#e9bd3555'; ink.fillRect(start[0] * sx, start[1] * sy, (p[0] - start[0]) * sx, (p[1] - start[1]) * sy);
  }
};
overlay.onpointercancel = () => { pointer = null; stroke = []; ink.clearRect(0, 0, overlay.width, overlay.height); redactPress = null; pressing = false; paintRedactions(); };
overlay.onpointerup = event => {
  if (event.pointerId !== pointer) return;
  if (mode === 'redact') { redactUp(event); return; }
  pointer = null; const end = point(event), drawn = stroke.slice();
  ink.clearRect(0, 0, overlay.width, overlay.height);
  void run(async () => {
    if (mode === 'ink' && drawn.length >= 2) await mutate({ op: 'annot-add', page, type: 'Ink', strokes: [drawn], contents: '' });
    if (mode === 'highlight') {
      const quads = highlightQuads(lines, start, end);
      if (!quads.length) { tell('No selectable text here. You can draw on scanned pages.'); return; }
      await mutate({ op: 'annot-add', page, type: 'Highlight', quads, contents: '' });
    }
    if (mode === 'note') {
      document.querySelector('#note-dialog h2')!.textContent = 'Add a note';
      document.querySelector('#note-dialog button[value="add"]')!.textContent = 'Add note';
      $<HTMLTextAreaElement>('note-text').value = '';
      if (await ask($<HTMLDialogElement>('note-dialog')) === 'add') {
        const contents = $<HTMLTextAreaElement>('note-text').value.trim();
        if (contents) await mutate({ op: 'annot-add', page, type: 'Text', point: [Math.min(end[0], pages[page].width - 20), Math.min(end[1], pages[page].height - 20)], contents });
      }
    }
    if (mode === 'signature') {
      await mutate({ op: 'annot-add', page, type: 'Ink', strokes: placeSignature(signatureStrokes, end, pages[page].width, pages[page].height), signature: true, contents: '' });
      setMode('read');
    }
  });
};

$('forms').onclick = () => void run(async () => {
  const result = await rpc<FieldsReply>({ op: 'fields', page });
  $('field-list').replaceChildren();
  if (!result.items.length) { tell('There are no fillable form fields on this page.'); return; }
  for (const field of result.items) {
    const label = document.createElement('label'); label.textContent = `${field.label || field.name}${field.required ? ' *' : ''}`;
    let input: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement;
    if ((field.kind === 'combo' || field.kind === 'list') && !field.edit) {
      input = document.createElement('select');
      for (const value of ['', ...(field.options ?? [])]) { const option = document.createElement('option'); option.value = option.textContent = value; input.append(option); }
    } else if (field.multiline) input = document.createElement('textarea');
    else input = document.createElement('input');
    if (field.kind === 'checkbox' || field.kind === 'radio') {
      const check = input as HTMLInputElement; check.type = field.kind; check.checked = field.value === true;
      if (field.kind === 'radio') check.name = field.name;
      label.className = 'field-check';
    } else input.value = String(field.value);
    if (field.maxLen && input instanceof HTMLInputElement) input.maxLength = field.maxLen;
    input.disabled = field.readOnly || field.kind === 'signature' || !canFill;
    input.onchange = () => void run(async () => {
      const value = field.kind === 'checkbox' || field.kind === 'radio' ? (input as HTMLInputElement).checked : input.value;
      try { await mutate({ op: 'field-set', page, widgetId: field.id, value }); }
      finally {
        // A refused edit must not leave an uncommitted value appearing accepted.
        const actual = (await rpc<FieldsReply>({ op: 'fields', page })).items.find(item => item.id === field.id);
        if (actual) { if (typeof actual.value === 'boolean') (input as HTMLInputElement).checked = actual.value; else input.value = actual.value; }
      }
    });
    label.append(input); $('field-list').append(label);
  }
  $<HTMLDialogElement>('fields-dialog').showModal();
});

const signature = $<HTMLCanvasElement>('signature'), signatureInk = signature.getContext('2d')!;
let signatureStrokes: Point[][] = [], signaturePointer: number | null = null, signatureStroke: Point[] = [];
function signaturePoint(event: PointerEvent) { return pagePoint(event.clientX, event.clientY, signature.getBoundingClientRect(), 660, 300); }
function repaintSignature() { signatureInk.clearRect(0, 0, 660, 300); for (const s of signatureStrokes) drawStroke(signatureInk, s, 1, 1, '#253b34', 3); drawStroke(signatureInk, signatureStroke, 1, 1, '#253b34', 3); }
signature.onpointerdown = event => { if (signaturePointer !== null || signatureStrokes.length >= 64) return; signaturePointer = event.pointerId; signature.setPointerCapture(event.pointerId); signatureStroke = [signaturePoint(event)]; };
signature.onpointermove = event => {
  if (event.pointerId !== signaturePointer) return;
  const total = signatureStrokes.reduce((sum, stroke) => sum + stroke.length, signatureStroke.length);
  if (signatureStroke.length < 2048 && total < 8192) signatureStroke.push(signaturePoint(event));
  repaintSignature();
};
signature.onpointerup = event => { if (event.pointerId !== signaturePointer) return; if (signatureStroke.length >= 2) signatureStrokes.push(signatureStroke); signatureStroke = []; signaturePointer = null; repaintSignature(); };
signature.onpointercancel = () => { signatureStroke = []; signaturePointer = null; repaintSignature(); };
$('sign').onclick = () => { signatureStrokes = []; signatureStroke = []; repaintSignature(); $<HTMLDialogElement>('signature-dialog').showModal(); };
$('clear-signature').onclick = () => { signatureStrokes = []; signatureStroke = []; repaintSignature(); };
$('use-signature').onclick = () => { if (!signatureStrokes.length) { tell('Draw your signature first.'); return; } $<HTMLDialogElement>('signature-dialog').close(); setMode('signature'); };
window.addEventListener('beforeunload', event => { if (dirty || unsavedLocalCopy || boxes.count) { event.preventDefault(); event.returnValue = ''; } });
if (Capacitor.getPlatform() === 'android') {
  void App.addListener('backButton', () => {
    const dialogs = Array.from(document.querySelectorAll<HTMLDialogElement>('dialog[open]'));
    if (dialogs.length) { dialogs.at(-1)!.close('cancel'); return; }
    if (busy) return;
    if (current) $('back').click();
    else void App.minimizeApp();
  });
}
let resizeTimer: ReturnType<typeof setTimeout>;
window.addEventListener('resize', () => { clearTimeout(resizeTimer); resizeTimer = setTimeout(() => { if (current && !busy) void run(renderPage); }, 200); });
let pinch: { distance: number; zoom: number; ratio: number; anchorX: number; anchorY: number; midX: number; midY: number; left: number; top: number } | null = null;
const viewport = $('viewport');
const distance = (touches: TouchList) => Math.hypot(touches[0].clientX - touches[1].clientX, touches[0].clientY - touches[1].clientY);
const middle = (touches: TouchList) => [(touches[0].clientX + touches[1].clientX) / 2, (touches[0].clientY + touches[1].clientY) / 2];
// Two fingers scroll and zoom in Read and in Redact (where one finger draws a box).
viewport.addEventListener('touchstart', event => {
  // A second finger never draws a box, even while the app is busy.
  if (event.touches.length >= 2 && mode === 'redact') cancelRedactPress();
  if (event.touches.length !== 2 || busy || (mode !== 'read' && mode !== 'redact')) return;
  if (event.cancelable) event.preventDefault();
  const bounds = canvas.getBoundingClientRect();
  const [midX, midY] = middle(event.touches);
  pinch = { distance: distance(event.touches), zoom, ratio: 1, midX, midY, left: viewport.scrollLeft, top: viewport.scrollTop,
    anchorX: (event.touches[0].clientX + event.touches[1].clientX) / 2 - bounds.left,
    anchorY: (event.touches[0].clientY + event.touches[1].clientY) / 2 - bounds.top };
  $('paper').style.transformOrigin = `${pinch.anchorX}px ${pinch.anchorY}px`;
}, { passive: false });
viewport.addEventListener('touchmove', event => {
  if (!pinch || event.touches.length !== 2) return;
  if (event.cancelable) event.preventDefault();
  pinch.ratio = Math.max(.75, Math.min(3, pinch.zoom * distance(event.touches) / Math.max(1, pinch.distance))) / pinch.zoom;
  const [midX, midY] = middle(event.touches);
  // Measured from where the fingers started, so a touchmove the browser coalesces or drops loses nothing.
  viewport.scrollTo(pinch.left + pinch.midX - midX, pinch.top + pinch.midY - midY);
  $('paper').style.transform = `scale(${pinch.ratio})`;
}, { passive: false });
function finishPinch() {
  if (!pinch) return;
  const previous = pinch; pinch = null;
  $('paper').style.transform = '';
  const left = viewport.scrollLeft, top = viewport.scrollTop;
  if (Math.abs(previous.ratio - 1) < .02) return;
  void run(async () => {
    zoom = previous.zoom * previous.ratio;
    await renderPage();
    viewport.scrollLeft = left + previous.anchorX * (previous.ratio - 1);
    viewport.scrollTop = top + previous.anchorY * (previous.ratio - 1);
  });
}
viewport.addEventListener('touchend', finishPinch);
viewport.addEventListener('touchcancel', () => { pinch = null; $('paper').style.transform = ''; });
// A tap on an existing note opens its readable contents and editing controls.
canvas.onclick = event => void run(async () => {
  if (mode !== 'read') return;
  const result = await rpc<AnnotsReply>({ op: 'annots', page });
  const [x, y] = pagePoint(event.clientX, event.clientY, canvas.getBoundingClientRect(), pages[page].width, pages[page].height);
  const tapped = result.items.some(item => item.type === 'Text' && x >= item.rect[0] - 10 && x <= item.rect[2] + 10 && y >= item.rect[1] - 10 && y <= item.rect[3] + 10);
  if (tapped) { await showAnnotations(); $<HTMLDialogElement>('annotations-dialog').showModal(); }
});
void run(async () => { await ready; await refreshLibrary(); });
setupScanner(async (bytes, name, original) => {
  // The scan worker independently reopens and validates each newly created PDF.
  // The editor's validator checks edits against its open document and cannot validate a new scan.
  const doc = { id: crypto.randomUUID(), name, bytes, updated: Date.now() };
  const documents = [doc];
  if (original) {
    documents.push({ id: crypto.randomUUID(), name: name.replace(' — rebuilt.pdf', ' — original scan.pdf'), bytes: original, updated: doc.updated - 1 });
  }
  await putDocuments(documents);
  await openDocument(doc);
});


// ---- Redact: draw a box, everything inside is gone (docs/rust-and-ux-plan.md §2.3) ----
const TINT = 'rgba(214, 40, 36, .45)';
/** Canvas pixels per CSS pixel of the page drawing. */
const pixelRatio = () => shade.width / Math.max(1, parseFloat(shade.style.width) || shade.width);
/** redact-inspect's quads list their corners in order around the glyph run (inspect.rs `rect_quad`). */
function quadPath(ctx: CanvasRenderingContext2D, q: number[], sx: number, sy: number) {
  ctx.beginPath(); ctx.moveTo(q[0] * sx, q[1] * sy);
  for (const i of [2, 4, 6]) ctx.lineTo(q[i] * sx, q[i + 1] * sy);
  ctx.closePath();
}
/** What goes under a box, tinted red: glyphs, pictures, shapes, and comments and fields (whole). */
function tint(box: Box, sx: number, sy: number) {
  const found = box.inspect, [x0, y0, x1, y1] = box.rect;
  shading.fillStyle = TINT;
  if (found === undefined) return;
  if (found === null || found.truncated) { shading.fillRect(x0 * sx, y0 * sy, (x1 - x0) * sx, (y1 - y0) * sy); return; }
  for (const quad of found.glyphs) { quadPath(shading, quad, sx, sy); shading.fill(); }
  for (const item of [...found.images, ...found.shapes, ...found.annots]) {
    const [a, b, c, d] = item.rect; shading.fillRect(a * sx, b * sy, (c - a) * sx, (d - b) * sy);
  }
}
/** The boxes of this page: solid black with a thin red outline; translucent with the tint while a finger is down or "Show what's underneath" is on. */
function paintRedactions(draft?: Rect | null) {
  shading.clearRect(0, 0, shade.width, shade.height);
  const size = pages[page];
  if (!current || !size) { placeRemoveButton(); return; }
  const sx = shade.width / size.width, sy = shade.height / size.height, px = pixelRatio();
  const see = mode === 'redact' && (peek || pressing);
  for (const box of boxes.onPage(page)) {
    const [x0, y0, x1, y1] = box.rect, x = x0 * sx, y = y0 * sy, w = (x1 - x0) * sx, h = (y1 - y0) * sy;
    if (see) { shading.fillStyle = 'rgba(20, 20, 20, .12)'; shading.fillRect(x, y, w, h); tint(box, sx, sy); }
    else { shading.fillStyle = '#000'; shading.fillRect(x, y, w, h); }
    const chosen = mode === 'redact' && box.id === selected;
    shading.strokeStyle = '#d62824'; shading.lineWidth = (chosen ? 3 : 1.25) * px;
    shading.strokeRect(x, y, w, h);
  }
  if (draft) {
    const [x0, y0, x1, y1] = draft;
    shading.fillStyle = 'rgba(20, 20, 20, .18)'; shading.fillRect(x0 * sx, y0 * sy, (x1 - x0) * sx, (y1 - y0) * sy);
    shading.strokeStyle = '#d62824'; shading.lineWidth = 1.5 * px; shading.strokeRect(x0 * sx, y0 * sy, (x1 - x0) * sx, (y1 - y0) * sy);
  }
  placeRemoveButton();
}
/** The × on the selected box: a 44 pt target centred on its top-right corner, kept on the page. */
function placeRemoveButton() {
  const button = $<HTMLButtonElement>('redact-remove'), box = selected === null ? undefined : boxes.find(selected);
  if (mode !== 'redact' || !box || box.page !== page || !pages[page]) { button.hidden = true; return; }
  const cssWidth = parseFloat(shade.style.width), cssHeight = parseFloat(shade.style.height), k = cssWidth / pages[page].width;
  button.style.left = `${Math.max(0, Math.min(cssWidth - 44, box.rect[2] * k - 22))}px`;
  button.style.top = `${Math.max(0, Math.min(cssHeight - 44, box.rect[1] * k - 22))}px`;
  button.hidden = false;
}
function updateRedactUi() {
  const n = boxes.count, areas = `Redact ${plural(n, 'area')}…`;
  $('redact-count').textContent = String(n); $('redact-count').hidden = n === 0;
  if (n) $('redact-tool').setAttribute('aria-label', `Redact, ${plural(n, 'box')} pending`); else $('redact-tool').removeAttribute('aria-label');
  $<HTMLButtonElement>('redact-list').textContent = `Redactions (${n})`;
  $<HTMLButtonElement>('redact-list').disabled = n === 0;
  $('redact-start').textContent = areas; $('redact-start').hidden = n === 0;
  $('redactions-apply').textContent = areas; $('redactions-title').textContent = `Redactions (${n})`;
  updateUndo();
}
/** redact-inspect for the boxes that have no answer yet (on finger-up, never while moving). A box it cannot answer for is tinted whole. */
async function inspectBoxes(list: readonly Box[]) {
  const missing = list.filter(box => box.inspect === undefined);
  if (!missing.length) return;
  try {
    const reply = await rpc<RedactInspectReply>({ op: 'redact-inspect', regions: missing.map(box => ({ page: box.page, rect: [...box.rect] })) });
    missing.forEach((box, i) => { box.inspect = reply.regions[i] ?? null; });
  } catch (error) {
    if (!(error instanceof EngineRefusal)) throw error;
    missing.forEach(box => { box.inspect = null; });
  }
}
function removeBox(id: number) {
  if (!boxes.remove(id)) return;
  if (selected === id) selected = null;
  updateRedactUi(); paintRedactions();
}
/** A second finger turns the drag into scrolling: the box being drawn is dropped. */
function cancelRedactPress() {
  if (pointer !== null && overlay.hasPointerCapture(pointer)) overlay.releasePointerCapture(pointer);
  pointer = null; redactPress = null; pressing = false; paintRedactions();
}
function redactDown(event: PointerEvent) {
  if (busy || pointer !== null || pinch || !canRedact || !pages[page]) return;
  pointer = event.pointerId; overlay.setPointerCapture(pointer);
  const start = point(event);
  redactPress = { start, x: event.clientX, y: event.clientY, on: boxAt(boxes.all, page, start, 4), moved: false };
  pressing = true; paintRedactions();
  // The boxes already on the page show what goes under them while the finger is down.
  const unknown = boxes.onPage(page).filter(box => box.inspect === undefined);
  if (unknown.length) void inspectBoxes(unknown).then(() => { if (pressing) paintRedactions(redactPress?.moved ? draftOf(redactPress) : null); }, () => {});
}
let lastPoint: Point = [0, 0];
const draftOf = (press: { start: Point }) => { const [a, b] = press.start; return [Math.min(a, lastPoint[0]), Math.min(b, lastPoint[1]), Math.max(a, lastPoint[0]), Math.max(b, lastPoint[1])] as Rect; };
function redactMove(event: PointerEvent) {
  if (!redactPress) return;
  lastPoint = point(event);
  if (Math.hypot(event.clientX - redactPress.x, event.clientY - redactPress.y) > 6) redactPress.moved = true;
  if (redactPress.moved) paintRedactions(draftOf(redactPress));
}
function redactUp(event: PointerEvent) {
  pointer = null; pressing = false;
  const press = redactPress; redactPress = null;
  if (!press) return;
  if (!press.moved) { selected = press.on?.id ?? null; paintRedactions(); return; }
  const rect = boxFrom(press.start, point(event), pages[page].width, pages[page].height);
  if (!rect) { paintRedactions(); return; }
  const box = boxes.add(page, rect);
  if (!box) { paintRedactions(); tell('That’s the most boxes one redaction can take. Redact these first.'); return; }
  selected = box.id; updateRedactUi(); paintRedactions();
  void run(async () => { await inspectBoxes([box]); paintRedactions(); });
}
$('redact-remove').onclick = () => { if (selected !== null && !busy) removeBox(selected); };
$<HTMLInputElement>('redact-peek').onchange = () => {
  peek = $<HTMLInputElement>('redact-peek').checked;
  paintRedactions();
  if (peek) void run(async () => { await inspectBoxes(boxes.onPage(page)); paintRedactions(); });
};
$('redact-done').onclick = () => setMode('read');
document.addEventListener('keydown', event => {
  if (!current || mode !== 'redact' || document.querySelector('dialog[open]')) return;
  if (event.key === 'Escape') { event.preventDefault(); setMode('read'); }
  else if ((event.key === 'Delete' || event.key === 'Backspace') && selected !== null && !busy) { event.preventDefault(); removeBox(selected); }
});

/** The sheet "Redactions (N)": the boxes by page, each with what it removes, and the Redact button. */
async function showRedactionList() {
  await inspectBoxes(boxes.all);
  const list = $('redactions-list'); list.replaceChildren();
  for (const group of boxes.byPage()) {
    const heading = document.createElement('h3'); heading.textContent = `Page ${group.page + 1}`; list.append(heading);
    group.boxes.forEach((box, i) => {
      const row = document.createElement('div'); row.className = 'redaction-row';
      const go = document.createElement('button'); go.className = 'redaction-go';
      const title = document.createElement('strong'); title.textContent = `Area ${i + 1}`;
      const detail = document.createElement('small');
      const text = box.inspect?.text.replace(/\s+/g, ' ').trim();
      detail.textContent = text ? `Removes “${text.length > 60 ? `${text.slice(0, 60)}…` : text}”` : 'Removes everything inside the box';
      go.append(title, detail);
      if (box.inspect?.needsFlatten) {
        const note = document.createElement('small'); note.className = 'redaction-note';
        note.textContent = `Covers artwork that can’t be cut precisely. You’ll be asked before page ${group.page + 1} is turned into an image.`;
        go.append(note);
      }
      go.onclick = () => void run(async () => {
        $<HTMLDialogElement>('redactions-dialog').close();
        if (mode !== 'redact') setMode('redact');
        selected = box.id;
        if (page !== box.page) { page = box.page; await renderPage(); } else paintRedactions();
        const k = parseFloat(shade.style.width) / pages[page].width;
        $('viewport').scrollTop = Math.max(0, box.rect[1] * k - 80);
      });
      const remove = document.createElement('button'); remove.className = 'redaction-remove'; remove.textContent = '×';
      remove.setAttribute('aria-label', `Remove area ${i + 1} on page ${group.page + 1}`);
      remove.onclick = () => { removeBox(box.id); if (boxes.count) void run(showRedactionList); else $<HTMLDialogElement>('redactions-dialog').close(); };
      row.append(go, remove); list.append(row);
    });
  }
  const dialog = $<HTMLDialogElement>('redactions-dialog');
  if (!dialog.open) dialog.showModal();
}
$('redact-list').onclick = () => void run(showRedactionList);
$('redact-start').onclick = () => void run(async () => { await confirmRedaction(); });
$('redactions-apply').onclick = () => { $<HTMLDialogElement>('redactions-dialog').close(); void run(async () => { await confirmRedaction(); }); };

// The confirmation: "Also remove hidden information" turns the nine Details options on or off together.
const hiddenAll = $<HTMLInputElement>('redact-hidden');
const sanitizeBoxes = SANITIZE_LABELS.map(([key, text]) => {
  const label = document.createElement('label'); label.className = 'field-check';
  const input = document.createElement('input'); input.type = 'checkbox'; input.checked = true; input.dataset.key = key;
  const span = document.createElement('span'); span.textContent = text;
  label.append(input, span); $('redact-sanitize').append(label);
  input.onchange = () => {
    const on = sanitizeBoxes.filter(box => box.checked).length;
    hiddenAll.checked = on === sanitizeBoxes.length; hiddenAll.indeterminate = on > 0 && on < sanitizeBoxes.length;
  };
  return input;
});
hiddenAll.onchange = () => { hiddenAll.indeterminate = false; sanitizeBoxes.forEach(box => { box.checked = hiddenAll.checked; }); };
/** A question with a primary answer ('yes'), an optional second answer ('other') and Cancel. */
function question(title: string, text: string, yes: string, other?: string): Promise<string> {
  $('redact-question-title').textContent = title; $('redact-question-text').textContent = text; $('redact-question-yes').textContent = yes;
  $('redact-question-other').textContent = other ?? ''; $('redact-question-other').hidden = !other;
  $('redact-question-actions').classList.toggle('stacked', !!other);
  return ask($<HTMLDialogElement>('redact-question'));
}
function notice(title: string, text: string): Promise<string> {
  $('redact-notice-title').textContent = title; $('redact-notice-text').textContent = text;
  return ask($<HTMLDialogElement>('redact-notice'));
}
/** Other edits that are not saved yet would be in the copy but not in the original: ask first. False to stop. */
async function unsavedChangesFirst(): Promise<boolean> {
  if (!dirty && !unsavedLocalCopy) return true;
  // On a redacted copy, the document being redacted again is that copy, not the original.
  const copy = !!(redactedFrom || current?.redacted);
  const choice = await question('You have other unsaved changes.',
    copy ? `They will be in the new redacted copy, but not in ‘${current!.name}’.` : 'They will be in the redacted copy, but not in your original.',
    copy ? 'Save them to this copy first' : 'Save them to the original first', 'Continue');
  if (choice === 'other') return true;
  if (choice !== 'yes') return false;
  try { await saveLocal(); return true; }
  catch { await notice('Your changes couldn’t be saved', 'Device storage is full or unavailable. Free some space and try again, or choose Continue.'); return false; }
}
/** Confirm, apply, and on success open the redacted copy. True when the copy was made. */
async function confirmRedaction(): Promise<boolean> {
  const n = boxes.count;
  if (!current || !n) return false;
  if (!await unsavedChangesFirst()) return false;
  $('redact-confirm-title').textContent = `Permanently remove everything inside ${plural(n, 'box', 'boxes')}?`;
  // Areas marked earlier (for example on the desktop app) are applied with the boxes: say so.
  let earlier = 0;
  try { earlier = (await rpc<RedactPreview>({ op: 'redact-preview' })).marks; } catch { earlier = 0; }
  $('redact-confirm-earlier').textContent = earlier ? `This file also has ${plural(earlier, 'area')} marked for redaction earlier. ${earlier === 1 ? 'It is' : 'They are'} removed too.` : '';
  $('redact-confirm-earlier').hidden = !earlier;
  hiddenAll.checked = true; hiddenAll.indeterminate = false; sanitizeBoxes.forEach(box => { box.checked = true; });
  $<HTMLDetailsElement>('redact-details').open = false;
  if (await ask($<HTMLDialogElement>('redact-confirm')) !== 'apply') return false;
  const sanitize = Object.fromEntries(sanitizeBoxes.map(box => [box.dataset.key!, box.checked]));
  let flatten: number[] = [], unsign = false, reason: string | undefined, detail: string | undefined;
  rightsOnly = false;
  // At most one flatten question per round and one signature question: a few rounds settle every refusal.
  for (let attempt = 0; attempt < 4; attempt++) {
    try {
      const reply = await rpc<RedactApplyReply>({ op: 'redact-apply', sanitize, regions: boxes.regions(), ...(flatten.length ? { flatten } : {}), ...(unsign ? { unsign } : {}) });
      return await openRedactedCopy(reply);
    } catch (error) {
      if (!(error instanceof EngineRefusal)) throw error;
      reason = error.reason;
      detail = typeof error.reply.detail === 'string' ? error.reply.detail : undefined;
      const asked = error.reply.pages;
      const wanted = Array.isArray(asked) ? asked.filter((p): p is number => Number.isInteger(p) && !flatten.includes(p)) : [];
      if (reason === 'needs-flatten' && wanted.length) {
        const words = flattenQuestion(wanted);
        if (await question(words.title, words.text, words.action) !== 'yes') return false;
        flatten = [...flatten, ...wanted]; continue;
      }
      if (reason === 'signed' && !unsign) {
        // Only Adobe Reader usage rights (detail `usage-rights`): no one's signature is lost, so say what is.
        rightsOnly = error.reply.detail === 'usage-rights';
        const words = rightsOnly
          ? ['This file has special permissions from Adobe Reader. Redacting removes them. Make a redacted copy without them?', 'Your original keeps them.', 'Make copy without them']
          : ['Redacting removes the signatures. Make an unsigned redacted copy?', 'Your signed original is not changed.', 'Make unsigned copy'];
        if (await question(words[0], words[1], words[2]) !== 'yes') return false;
        unsign = true; continue;
      }
      break;
    }
  }
  const words = refusalMessage(reason, signedDocument, detail);
  await notice(words.title, words.text);
  return false;
}
/**
 * Saves the copy as a new library document (the original is never overwritten) and opens it in place of the
 * original. When it cannot be stored, the user stays on the original with the boxes, and may share the copy.
 */
async function openRedactedCopy(reply: RedactApplyReply): Promise<boolean> {
  const original = current!;
  const taken = await listDocuments().then(docs => docs.map(doc => doc.name), () => [] as string[]);
  const copy: LocalDocument = { id: crypto.randomUUID(), name: redactedName(original.name, taken), bytes: reply.bytes, updated: Date.now(), redacted: true };
  try { await putDocument(copy); }
  catch {
    const choice = await question('The redacted copy couldn’t be saved on this device', 'Device storage is full or unavailable. Your original and your boxes are unchanged. You can share the redacted copy now, or free some space and try again.', 'Share redacted copy');
    if (choice === 'yes') await sharePDF(copy.bytes.slice(0), copy.name, false);
    return false;
  }
  const password = documentPassword;
  await openDocument(copy, password);
  // The copy needs the original's password and none was given: openDocument went back to the library, where the copy is saved.
  if (current?.id !== copy.id) return true;
  showRedactedBanner({ original, count: reply.marks, flattened: reply.flattened ?? [], unsigned: reply.unsigned === true, overRedacted: reply.overRedacted === true });
  return true;
}
function showRedactedBanner(from: typeof redactedFrom) {
  redactedFrom = from;
  $('redacted-banner').hidden = !from;
  if (!from || !current) return;
  const extra = [
    from.flattened.length ? `${pagesNamed(from.flattened).replace(/^p/, 'P')} ${from.flattened.length === 1 ? 'is' : 'are'} now ${from.flattened.length === 1 ? 'an image' : 'images'}.` : '',
    from.unsigned ? (rightsOnly ? 'The copy no longer has the Adobe Reader permissions.' : 'The copy has no signatures.') : '',
    from.overRedacted ? 'Some content outside your boxes was also removed, because it is shared with other parts of the document. Check the copy.' : '',
  ].filter(Boolean).join(' ');
  $('redacted-text').textContent = `You are viewing the redacted copy ‘${current.name}’. ${plural(from.count, 'area')} removed and checked.${extra ? ` ${extra}` : ''}`;
}
$('open-original').onclick = () => void run(async () => {
  const from = redactedFrom;
  if (!from || !await pendingBoxes('leave')) return;
  if (dirty || unsavedLocalCopy) {
    const choice = await question('The redacted copy has changes that aren’t saved.', 'They will be lost if you open the original now.', 'Save them first', 'Open original');
    if (choice === 'yes') { try { await saveLocal(); } catch { await notice('The changes couldn’t be saved', 'Device storage is full or unavailable. Use Share to keep the redacted copy.'); return; } }
    else if (choice !== 'other') return;
  }
  const saved = await getDocument(from.original.id).catch(() => undefined);
  await openDocument(saved ?? from.original, documentPassword);
});
/**
 * Boxes that are not redacted yet, before leaving, sharing or saving: [Redact now…] (the default), the action
 * anyway (leaving discards the boxes; sharing and saving keep them), or Cancel. True when the action may go ahead.
 */
async function pendingBoxes(action: 'leave' | 'share' | 'save'): Promise<boolean> {
  const n = boxes.count;
  if (!n) return true;
  $('redact-leave-title').textContent = `${plural(n, 'area is', 'areas are')} marked but not redacted yet.`;
  $('redact-leave-text').textContent = ({
    leave: 'The boxes are not saved. Redact them now, or discard them.',
    share: 'Sharing now sends the file with everything under the boxes still in it.',
    save: 'Saving now keeps everything under the boxes in the file.',
  })[action];
  $('redact-leave-other').textContent = ({ leave: 'Discard boxes', share: 'Share without redacting', save: 'Save without redacting' })[action];
  const choice = await ask($<HTMLDialogElement>('redact-leave'));
  if (choice === 'other') {
    if (action === 'leave') { boxes.clear(); selected = null; updateRedactUi(); paintRedactions(); }
    return true;
  }
  if (choice === 'redact') await confirmRedaction();
  return false;
}
