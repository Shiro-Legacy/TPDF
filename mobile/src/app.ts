import type { AnnotsReply, FieldsReply, JournalState, Line, OpenReply, PageSize, RenderReply, SaveReply, SearchReply, SessionRequest } from '../../src/renderer/protocol.js';
import { highlightQuads, pagePoint, placeSignature, type Point } from './geometry.js';
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
worker.onmessage = ({ data }) => {
  if (data.ready) { clearTimeout(startupTimer); engineReady(); return; }
  const task = pending.get(data.id);
  if (!task) return;
  pending.delete(data.id);
  if (data.ok) task.resolve(data);
  else task.reject(new Error(data.error || 'This PDF operation could not be completed.'));
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

type Mode = 'read' | 'highlight' | 'ink' | 'note' | 'signature';
let mode: Mode = 'read', current: LocalDocument | null = null, pages: PageSize[] = [], page = 0, zoom = 1;
let lines: Line[] = [], dirty = false, unsavedLocalCopy = false, busy = false, statusTimer: ReturnType<typeof setTimeout>;
let canAnnotate = true, canFill = true;
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
  dirty = state.dirty;
  $<HTMLButtonElement>('undo').disabled = !state.canUndo;
  $<HTMLButtonElement>('redo').disabled = !state.canRedo;
  $('save-state').textContent = dirty ? 'Unsaved changes' : unsavedLocalCopy ? 'Not saved on device' : 'Saved on device';
}
function setMode(next: Mode) {
  mode = next;
  document.querySelectorAll<HTMLButtonElement>('[data-mode]').forEach(button => button.setAttribute('aria-pressed', String(button.dataset.mode === mode)));
  overlay.style.pointerEvents = mode === 'read' ? 'none' : 'auto';
  $('hint').textContent = ({ read: 'Swipe to scroll. Pinch or use + and − to zoom.', highlight: 'Drag across text to highlight whole lines.', ink: 'Draw on the page. Switch to Read to scroll.', note: 'Tap the page to add a note.', signature: 'Tap the page to place your signature.' })[mode];
  ink.clearRect(0, 0, overlay.width, overlay.height);
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
  lines = result.lines;
  $('page-text').textContent = result.text;
  canvas.setAttribute('aria-label', `PDF page ${page + 1} of ${pages.length}`);
  $('page-label').textContent = `${page + 1} / ${pages.length}`;
  $<HTMLButtonElement>('prev').disabled = page === 0;
  $<HTMLButtonElement>('next').disabled = page === pages.length - 1;
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
async function openDocument(doc: LocalDocument) {
  const pdf = doc.bytes.slice(0);
  let result = await rpc<OpenReply>({ op: 'open', pdf, name: doc.name }, [pdf]);
  if (result.info.needsPassword) {
    while (result.info.needsPassword) {
      $<HTMLInputElement>('password').value = '';
      if (await ask($<HTMLDialogElement>('password-dialog')) !== 'unlock') { await rpc({ op: 'close' }); return; }
      try {
        const unlocked = await rpc<OpenReply & { authenticated: boolean }>({ op: 'authenticate', password: $<HTMLInputElement>('password').value });
        if (!unlocked.authenticated) { tell('That password did not unlock the PDF. Try again.'); continue; }
        result = unlocked;
      } finally { $<HTMLInputElement>('password').value = ''; }
    }
  }
  current = doc; unsavedLocalCopy = false; pages = result.pages; page = 0; zoom = 1;
  canAnnotate = result.info.permissions.annotate && !result.info.certified && result.info.signatures === 0;
  canFill = result.info.permissions.form && !result.info.certified && result.info.signatures === 0;
  $<HTMLButtonElement>('copy-text').disabled = !result.info.permissions.copy;
  document.querySelectorAll<HTMLButtonElement>('[data-mode]').forEach(button => { button.disabled = button.dataset.mode !== 'read' && !canAnnotate; });
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
  const saved = { ...current, bytes, updated: Date.now() };
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
  if ((dirty || unsavedLocalCopy) && !confirm('Leave this PDF and discard changes that could not be saved?')) return;
  await rpc({ op: 'close' }); current = null; dirty = false; unsavedLocalCopy = false;
  $('editor').hidden = true; $('library').hidden = false; await refreshLibrary();
});
$('save').onclick = () => void run(async () => { await saveLocal(); $<HTMLDialogElement>('menu-dialog').close(); tell('Saved on this device.'); });
$('share').onclick = () => void run(async () => {
  if (!current) return;
  // Unedited files can be shared byte-for-byte, including signed/encrypted PDFs.
  await sharePDF(dirty ? await serialized() : current.bytes.slice(0), current.name);
});
for (const [id, delta] of [['prev', -1], ['next', 1]] as const) $(id).onclick = () => void run(async () => {
  page = Math.max(0, Math.min(pages.length - 1, page + delta)); await renderPage(); $('viewport').scrollTo(0, 0);
});
for (const [id, delta] of [['zoom-in', .25], ['zoom-out', -.25]] as const) $(id).onclick = () => void run(async () => {
  zoom = Math.max(.75, Math.min(3, zoom + delta)); await renderPage();
});
for (const op of ['undo', 'redo'] as const) $(op).onclick = () => void run(() => mutate({ op }));
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
  if (busy || pointer !== null || mode === 'read' || !canAnnotate) return;
  pointer = event.pointerId; overlay.setPointerCapture(pointer); start = point(event); stroke = [start];
};
overlay.onpointermove = event => {
  if (event.pointerId !== pointer) return;
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
overlay.onpointercancel = () => { pointer = null; stroke = []; ink.clearRect(0, 0, overlay.width, overlay.height); };
overlay.onpointerup = event => {
  if (event.pointerId !== pointer) return;
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
window.addEventListener('beforeunload', event => { if (dirty || unsavedLocalCopy) { event.preventDefault(); event.returnValue = ''; } });
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
let pinch: { distance: number; zoom: number; ratio: number; anchorX: number; anchorY: number } | null = null;
const viewport = $('viewport');
const distance = (touches: TouchList) => Math.hypot(touches[0].clientX - touches[1].clientX, touches[0].clientY - touches[1].clientY);
viewport.addEventListener('touchstart', event => {
  if (event.touches.length !== 2 || busy || mode !== 'read') return;
  event.preventDefault();
  const bounds = canvas.getBoundingClientRect();
  pinch = { distance: distance(event.touches), zoom, ratio: 1,
    anchorX: (event.touches[0].clientX + event.touches[1].clientX) / 2 - bounds.left,
    anchorY: (event.touches[0].clientY + event.touches[1].clientY) / 2 - bounds.top };
  $('paper').style.transformOrigin = `${pinch.anchorX}px ${pinch.anchorY}px`;
}, { passive: false });
viewport.addEventListener('touchmove', event => {
  if (!pinch || event.touches.length !== 2) return;
  event.preventDefault();
  pinch.ratio = Math.max(.75, Math.min(3, pinch.zoom * distance(event.touches) / Math.max(1, pinch.distance))) / pinch.zoom;
  $('paper').style.transform = `scale(${pinch.ratio})`;
}, { passive: false });
function finishPinch() {
  if (!pinch) return;
  const previous = pinch; pinch = null;
  $('paper').style.transform = '';
  const left = viewport.scrollLeft, top = viewport.scrollTop;
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
