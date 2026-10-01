/**
 * P2 app shell — static checks. Real-window CDP lives in scripts/archive/gui-shell-check.mjs
 * (not npm test: concurrent Electron/CDP is flaky next to smoke).
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';

const root = fileURLToPath(new URL('..', import.meta.url));
const html = () => readFile(join(root, 'src/renderer/index.html'), 'utf8');
const rendererSrc = () => readFile(join(root, 'src/renderer/renderer.ts'), 'utf8');
const mainSrc = () => readFile(join(root, 'src/main.ts'), 'utf8');

const REQUIRED_IDS = [
  'mode-read', 'mode-annotate', 'mode-organize', 'mode-forms', 'mode-edit', 'mode-redact', 'hit-layer', 'field-bar', 'field-highlight', 'field-next', 'fill-textbox-add', 'fill-sign', 'fill-sign-draw',
  'sigpad-dialog', 'sigpad-heading', 'sigpad-canvas', 'sigpad-status', 'sigpad-clear', 'sigpad-image', 'sigpad-remove', 'sigpad-save', 'sigpad-cancel',
  'tab-pages', 'tab-bookmarks', 'tab-notes', 'tab-fields', 'tab-links', 'tab-text', 'tab-layers',
  'inspector-toggle', 'titlebar', 'document-title', 'text-panel',
  'open', 'save', 'save-copy', 'print', 'new-images', 'export', 'undo', 'redo',
  'thumbs-toggle', 'prev', 'next', 'page-input', 'page-label',
  'zoom-out', 'zoom-in', 'fit-width', 'fit-page', 'zoom-100', 'zoom-level',
  'search-input', 'search-prev', 'search-next', 'search-status',
  'canvas', 'page', 'viewport', 'thumbs', 'extracted', 'print-pages',
  'notes', 'fields', 'links', 'layers', 'outline', 'note-list', 'field-list',
  'note-add', 'markup-highlight', 'markup-underline', 'markup-strikeout',
  'freetext-add', 'shape-square', 'shape-circle', 'shape-line', 'ink-draw',
  'content-add-group', 'content-add-arm', 'content-add-font', 'content-add-size', 'content-add-color', 'content-add-text', 'content-add-go',
  'stamp-icon', 'sign-add',
  'page-rotate-right', 'page-rotate-left', 'page-move-up', 'page-move-down',
  'page-delete', 'insert-pages', 'extract-pages', 'stamp-watermark', 'stamp-header', 'page-labels',
  'status', 'error', 'banner', 'warning', 'edit-status', 'verify-open', 'verify-dialog', 'verify-heading', 'verify-summary', 'verify-list', 'verify-certified', 'verify-trusted', 'verify-trust-add', 'verify-trust-clear', 'verify-refresh', 'verify-close',
  'password-form', 'password-input', 'print-dialog', 'note-dialog',
  'help-dialog', 'help-matrix', 'help-ceilings', 'help-close',
  'redact-area', 'redact-search-text', 'redact-search-go', 'redact-count', 'redact-apply-open',
  'cert-sign-open', 'cert-sign-dialog', 'cert-sign-pick', 'cert-sign-file', 'cert-sign-password',
  'cert-sign-page', 'cert-sign-visible', 'cert-sign-x', 'cert-sign-y', 'cert-sign-w', 'cert-sign-h',
  'cert-sign-reason', 'cert-sign-certify', 'cert-sign-go', 'cert-sign-cancel', 'cert-sign-status',
  'sigfield-draw', 'cert-sign-field',
  'redact-dialog', 'redact-preview', 'redact-shared', 'redact-limit', 'redact-apply', 'redact-cancel',
  'redact-sanitize-info', 'redact-sanitize-xmp', 'redact-sanitize-attachments',
  'redact-sanitize-outlines', 'redact-sanitize-annots', 'redact-sanitize-links',
  'redact-sanitize-fields', 'redact-sanitize-layers', 'redact-sanitize-struct',
  'field-new-label', 'field-new-readonly', 'field-new-required',
  'field-new-default', 'field-new-default-check',
  'field-prop-label', 'field-prop-readonly', 'field-prop-required',
  'field-prop-default', 'field-prop-default-check',
  'object-menu', 'object-menu-cut', 'object-menu-copy', 'object-menu-paste', 'object-menu-replace', 'object-menu-duplicate', 'object-menu-delete',
  'object-menu-rotate-cw', 'object-menu-rotate-ccw', 'object-menu-flip-h', 'object-menu-flip-v', 'object-menu-crop',
  'content-rotate-cw', 'content-rotate-ccw', 'content-flip-h', 'content-flip-v', 'content-crop',
  'help-bates', 'stamp-align', 'stamp-bates-start', 'stamp-bates-pad',
  'batch-dialog', 'batch-heading', 'batch-folder', 'batch-pick', 'batch-kind', 'batch-text', 'batch-pages', 'batch-align',
  'batch-bates-start', 'batch-bates-pad', 'batch-summary', 'batch-list', 'batch-start', 'batch-cancel', 'batch-close',
  'compare-view', 'compare-canvas-a', 'compare-canvas-b', 'compare-overlay', 'compare-show-diff', 'compare-summary',
  'compare-list', 'compare-page-b', 'compare-close', 'help-compare', 'compare-name-a', 'compare-name-b',
];

function ids(src) {
  return new Set([...src.matchAll(/\sid="([^"]+)"/g)].map(m => m[1]));
}

function attrs(tag) {
  const out = {};
  for (const m of tag.matchAll(/([\w:-]+)=["']([^"]*)["']/g)) out[m[1]] = m[2];
  return out;
}

function named(open, inner, src, id) {
  const a = attrs(open);
  if (a['aria-label'] || a['aria-labelledby'] || a.title) return true;
  if (inner.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ').trim()) return true;
  return src.includes(`for="${id}"`) || src.includes(`for='${id}'`);
}

test('window chrome: size, min, dark bg, mac inset, sandbox prefs', async () => {
  const main = await mainSrc();
  const block = /new BrowserWindow\(\{[\s\S]*?\n  \}\);/.exec(main)?.[0] ?? '';
  assert.match(block, /width:\s*1360/);
  assert.match(block, /height:\s*860/);
  assert.match(block, /minWidth:\s*900/);
  assert.match(block, /minHeight:\s*600/);
  assert.match(block, /backgroundColor:\s*'#1b1e25'/);
  assert.match(block, /titleBarStyle:\s*'hiddenInset'/);
  assert.match(block, /trafficLightPosition:\s*\{\s*x:\s*14,\s*y:\s*12\s*\}/);
  assert.equal(block.includes('titleBarOverlay'), false);
  assert.match(block, /sandbox:\s*true/);
  assert.match(block, /contextIsolation:\s*true/);
  assert.match(block, /nodeIntegration:\s*false/);
  assert.match(block, /nodeIntegrationInWorker:\s*false/);
  assert.match(block, /webviewTag:\s*false/);
  assert.match(block, /webSecurity:\s*true/);
  assert.match(block, /devTools:\s*false/);
});

test('shell HTML has required control IDs', async () => {
  const src = await html();
  const have = ids(src);
  assert.deepEqual(REQUIRED_IDS.filter(id => !have.has(id)), []);
  assert.match(src, /<p id="stamp-hint">Placeholders: \{page\} \{pages\} \{label\} \{bates\}<\/p>/);
});

test('modes are radios; every control has id + accessible name', async () => {  const src = await html();
  assert.match(src, /role="radiogroup"/);
  for (const id of ['mode-read', 'mode-annotate', 'mode-organize', 'mode-forms', 'mode-edit', 'mode-redact']) {
    const tag = src.match(new RegExp(`<[^>]*\\sid="${id}"[^>]*>`))?.[0];
    assert.ok(tag, id);
    assert.ok(/type="radio"/.test(tag) || tag.includes('role="radio"'), `${id} radio`);
  }
  const missing = [];
  for (const m of src.matchAll(/<(button|textarea)\b([^>]*)>([\s\S]*?)<\/\1>/gi)) {
    const open = `<${m[1]}${m[2]}>`;
    const id = attrs(open).id;
    if (!id) { missing.push(`no-id:${open.slice(0, 72)}`); continue; }
    if (!named(open, m[3], src, id)) missing.push(id);
  }
  for (const m of src.matchAll(/<(input|select)\b([^>]*)>/gi)) {
    const open = m[0];
    if (/\btype="hidden"/i.test(open) || /\bname="stamp-icon"/.test(open)) continue;
    const id = attrs(open).id;
    if (!id) { missing.push(`no-id:${open.slice(0, 72)}`); continue; }
    if (!named(open, '', src, id)) missing.push(id);
  }
  assert.deepEqual(missing, []);
});

/**
 * U7 §5.7: the help dialog is the user-visible support/refusal matrix, so every reason text
 * the panel can show has to be explained there. The values are matched with tags stripped
 * (§8.12), which is also what keeps them entity-free.
 */
test('U7/P4/P5 help dialog: ten matrix rows, a ceilings list and every panel reason text', async () => {
  const src = await html();
  assert.match(src, /<dialog id="help-dialog" aria-labelledby="help-heading">/);
  const matrix = /<table id="help-matrix">([\s\S]*?)<\/table>/.exec(src)?.[1] ?? '';
  const rows = /<tbody>([\s\S]*?)<\/tbody>/.exec(matrix)?.[1] ?? '';
  assert.equal((rows.match(/<tr>/g) ?? []).length, 11, 'the matrix has eleven body rows including direct editing and signature verification');
  assert.match(rows, /<th scope="row">Certificate signing<\/th>/);
  // P5 U2 §8.8 + review: the verification row names the encrypted refusal and the missing key-usage checks.
  const verifyRow = /<th scope="row">Signature verification<\/th>([\s\S]*?)<\/tr>/.exec(rows)?.[1] ?? '';
  for (const ceiling of ['encrypted signed documents cannot be verified in this version', 'no key usage or extended key usage checks', 'any program running as you can add to the trusted list']) {
    assert.ok(verifyRow.includes(ceiling), `the signature verification row does not say: ${ceiling}`);
  }
  assert.match(src, /<ul id="help-ceilings">[\s\S]*?<\/ul>/);
  assert.ok(src.replace(/<[^>]+>/g, ' ').includes('undo does not move it back'), 'Help names the Bates undo ceiling');
  assert.ok(src.replace(/<[^>]+>/g, ' ').includes('results go to a pdf4all-out folder'), 'Help names the Batch output folder (contract-batch.md §9.15)');
  assert.ok(src.replace(/<[^>]+>/g, '').includes('Compare with… (⌥⌘K) opens a second PDF beside the current one: the list shows lines that appear only on the left or only on the right (a moved line counts as removed and added), and the red overlay marks pixels that differ at 72 dpi. Nothing is written.'),
    'Help carries the Compare sentence (contract-compare.md §9.15)');
  assert.match(src, /<ul id="help-bates">[\s\S]*?<\/ul>\s*<h3>Compare<\/h3>\s*<ul id="help-compare">/, 'the Compare list follows #help-bates');
  assert.equal((src.match(/Redaction is limited to documents up to 12 MiB in this version/g) ?? []).length, 2,
    'the redaction Help row and apply dialog both name the measured memory cap');

  const renderer = await rendererSrc();
  assert.ok(!renderer.includes('verification unavailable'), 'P5 U2 ships verification; the read-only banner must not deny it');
  const block = /const CONTENT_REASON_TEXT: Record<string, string> = \{[\s\S]*?\n\};/.exec(renderer)?.[0] ?? '';
  const values = [...block.matchAll(/:\s*"([^"]+)"/g)].map(match => match[1]);
  assert.ok(values.length >= 10, `expected the reason-text object, got ${values.length} values`);
  const text = src.replace(/<[^>]+>/g, ' ');
  for (const value of values) {
    assert.ok(!/[&<>]/.test(value), `a reason text must stay entity-free: ${value}`);
    assert.ok(text.includes(value), `the help matrix does not explain: ${value}`);
  }
});

/**
 * A1 §5.1: the engine has one permission table (`OP_PERMISSION` in engine-core.ts) and
 * `requireMutable` falls back to `"edit"` for ops it does not list. This test is the list
 * of exceptions: every op name the renderer puts in a request payload must be either a
 * table key, one of the read-only ops the table never guards, or one of the ops that
 * deliberately take the `?? "edit"` fallback. Delete one table key and this fails.
 */
import { OP_PERMISSION } from '../build/renderer/engine.js';

test('A1: every renderer request op is classified by the engine permission table', async () => {
  const src = await rendererSrc();
  const ops = new Set([...src.matchAll(/(?:[{,]\s*)op:\s*"([a-z-]+)"/g)].map(m => m[1]));
  assert.ok(ops.size >= 40, `expected the renderer's request ops, got ${ops.size}`);

  // Ops the engine never routes through `requireMutable`: reads, save/load plumbing.
  const READ_ONLY = new Set([
    'open', 'render', 'thumbnail', 'search', 'fields', 'annots', 'outline', 'links',
    'edit-targets', 'edit-replace-preview', 'text-all', 'labels', 'layers', 'redact-preview', 'save',
    'validate', 'saved', 'close', 'authenticate', 'extract-pages', 'pdf-from-images', 'batch-file', 'page-ids',
    'redact', 'sign', 'sign-prepare', 'sign-list', 'verify', // STATE/custom-guard ops outside requireMutable
    'compare-open', 'compare-close', 'compare-render', 'compare-text', // contract-compare.md §9.1: view-only, no write gate
  ]);
  // Mutations with no table entry: `requireMutable` reads them as `"edit"` (engine.ts).
  const DEFAULT_EDIT = new Set(['edit-text', 'edit-paragraph', 'edit-object', 'edit-objects', 'edit-replace-apply', 'security-set', 'layer-set']);

  const unexplained = [...ops].filter(op => !(op in OP_PERMISSION) && !READ_ONLY.has(op) && !DEFAULT_EDIT.has(op));
  assert.deepEqual(unexplained, [], 'add the op to OP_PERMISSION, READ_ONLY or DEFAULT_EDIT');
  const doubled = [...READ_ONLY, ...DEFAULT_EDIT].filter(op => op in OP_PERMISSION);
  assert.deepEqual(doubled, [], 'a read-only or fallback op must not also claim a permission rule');
});

test('review R-U3-1: every button shipped disabled with its own const is enabled somewhere in the renderer', async () => {
  const src = await html(), ts = await rendererSrc();
  for (const [tag] of src.matchAll(/<button\b[^>]*>/g)) {
    if (!/\sdisabled[\s>]/.test(tag)) continue;
    const id = tag.match(/id="([^"]+)"/)?.[1];
    const name = ts.match(new RegExp(`const (\\w+) = document\\.getElementById\\("${id}"\\)`))?.[1];
    if (name) assert.match(ts, new RegExp(`\\b${name}\\.disabled\\s*=`), `#${id} ships disabled and is never enabled`);
  }
});

test('review S2: a drive key on a focused .sig-target reaches its own keydown (Enter signs, §9.17)', async () => {
  const onTarget = (await rendererSrc()).match(/const onTarget = [^\n]*/)?.[0] ?? '';
  assert.match(onTarget, /#hit-layer \.sig-target/);
});

test('debt1 row 18: the Save As button reads the same as the File menu item', async () => {
  const menu = (await mainSrc()).match(/id: 'save-as',\s*label: '([^']+)'/)?.[1];
  const button = (await html()).match(/<button id="save-copy"[^>]*>([^<]*)<\/button>/)?.[1];
  assert.ok(menu);
  assert.equal(button, menu);
});

test('P6 U7 Help: the clipboard, multi-kind selection and their ceilings are explained', async () => {
  const src = await html();
  for (const text of [
    'Cut, Copy and Paste work on any selection: images, forms, text runs, fields, links and redaction marks',
    'right-click empty space to paste there',
    'Shift-click adds a text run, field, link or mark to a selection',
    'A selection stays on one page and in one mode',
    'The clipboard belongs to this window and is not the system clipboard.',
    'Pasted text keeps its font, size and whether it is filled, stroked or both, but is black with a 1 pt stroke; kerning and text rise are dropped.',
    'A signature field, a scripted field and a run inside a Form cannot be copied.',
    'Text runs cannot be cut or deleted; Cut copies them and removes the rest.',
    'A link to a page the target document does not have is skipped when pasting.',
    'Text placed exactly on top of the same text cannot be edited; text inside a form drawn more than once has no on-page target (edit it from the Text panel, this placement only).',
    'A run that continues a line cannot be copied.',
    'Paragraphs cannot be selected, copied or cut.',
    "Once a picture's full box leaves the page, its crop cannot be removed until it is moved back inside.",
    // Debt sweep 2 §9.7: the U6 clip ceiling (U+2026 in both places).
    "A clip set before an object's own q…Q block is not seen, so that object's box is its full drawn rectangle.",
  ]) assert.ok(src.includes(text), `Help is missing: ${text}`);
  for (const gone of [
    'only images and forms can be copied', 'no multi-select of text runs',
    // P6 U8 D2/§9.8: duplicate direct text is editable per run.
    'A pasted run whose text already appears as a whole line on the page cannot be edited until one of them changes.',
    'Added text that exactly repeats an existing line on the page cannot be selected on the page; edit it from the Text panel.',
    // R8-1: a run in a Form drawn twice is editable from the Text panel (§9.2); only its on-page target is missing.
    'and text inside a form drawn more than once, cannot be edited.',
    // Debt sweep 2 §9.4/§9.7: the render mode is kept now.
    'Pasted text keeps its font and size but is black; kerning is dropped.',
  ]) assert.ok(!src.includes(gone), `Help still says: ${gone}`);
  // §9.9: the overprint announce in renderer source.
  const ts = await rendererSrc();
  assert.ok(ts.includes('Added text is not selectable on the page (it lies exactly on top of the same text)'));
  // R8-3: a text-add the engine gives no target (targetId "") is announced, not silent.
  assert.ok(ts.includes('else if (contentLastAdd.targetId === "") announce("Added text is not editable on the page");'));
  assert.ok(!ts.includes('it repeats an existing line'), 'old announce gone');
});

test('debt2 R-B1: the Inspector box inputs accept any fraction (step="any"), so a rounded 594.64 never blocks Apply', async () => {
  const src = await html();
  for (const k of ['x', 'y', 'w', 'h']) {
    assert.match(src, new RegExp(`<input id="content-box-${k}" type="number" step="any">`), `content-box-${k} step="any"`);
  }
});

/** Bates review B-R1..B-R3: the dialog path clears a stale error, records a GUI refusal, and names a one-page number once. */
test('Bates stamp dialog: clearError first, refusal records decorate.last, one-page announcement', async () => {
  const src = await rendererSrc();
  const body = /async function saveStampDialog\(\): Promise<boolean> \{([\s\S]*?)\n\}/.exec(src)?.[1] ?? '';
  assert.match(body, /^\s*clearError\(\);/, 'saveStampDialog clears a stale error first');
  assert.match(body, /showError\("Bates start must be a whole number and digits 1\\u201315"\);\s*decorateLast\(\{ action: "stamp", kind, ok: false \}\);\s*return false;/,
    'a GUI Bates refusal records decorate.last ok:false');
  assert.ok(src.includes('suffix = from === to ? ` (Bates ${from})` : ` (Bates ${from}\\u2013${to})`;'), 'a one-page Bates stamp announces a single number');
});

test('Batch review R1-R6: counter prefill after a run, pad fallback, Close while running, Bates column, scroll box, list name', async () => {
  const src = await rendererSrc();
  const run = /async function runBatch\([\s\S]*?\n\}\n/.exec(src)?.[0] ?? '';
  const fin = /\} finally \{([\s\S]*?)\n  \}\n\}/.exec(run)?.[1] ?? '';
  assert.ok(fin.includes('batchBatesStartInput.value = String(batesNext);') && fin.includes('batchBatesPadInput.value = String(batesPad);'),
    'R1: a second Start continues the Bates counter (§9.13), so runBatch refreshes the prefill when it ends');
  assert.ok(run.includes('const pad = action.batesPad ?? batesPad;'), 'R5: batch.bates.pad falls back to the renderer pad, not 6');
  const render = /function renderBatch\(\) \{[\s\S]*?\n\}\n/.exec(src)?.[0] ?? '';
  assert.ok(render.includes('batchCloseButton.disabled = state.running;'), 'R4: Close is disabled while a batch runs');
  assert.match(render, /\[row\.name, row\.status, [^\]]*, range, row\.detail\]/, 'R3: each row shows its Bates range');
  const page = await html();
  assert.match(page, /<div id="batch-list-box"><table id="batch-list" aria-label="Batch files">/, 'R2/R6: the list scrolls in its own box and has a name');
  assert.match(page, /<th scope="col">Pages<\/th><th scope="col">Bates<\/th><th scope="col">Detail<\/th>/, 'R3: Bates header');
  const css = await readFile(join(root, 'src/renderer/styles.css'), 'utf8');
  assert.match(css, /#batch-list-box \{ max-height: 45vh; overflow: auto; \}/, 'R2: Start and Cancel stay on screen with 40 files');
  assert.match(css, /#batch-list th, #batch-list td \{ padding:/, 'R3: header cells are padded');
});

test('Compare review fixes are wired (contract-compare.md §9.19)', async () => {
  const src = await rendererSrc();
  const page = await html();
  const css = await readFile(join(root, 'src/renderer/styles.css'), 'utf8');
  // (e) R1: the overlay is B's pixel grid and the B wrapper hugs B, so red pixels sit on B's pixels.
  assert.match(src, /const overlay = new ImageData\(b\.pixelWidth, b\.pixelHeight\)/, 'R1: overlay sized to B');
  assert.ok(!/new ImageData\(W, H\)/.test(src), 'R1: no maxW x maxH overlay');
  assert.match(css, /\.compare-b \{ position: relative; width: fit-content; max-width: 100%; \}/, 'R1: B wrapper is fit-content');
  // (h) R2: a locked main clears A/list/summary; the unlock refreshes compare.
  const run = /async function runCompare[\s\S]*?\n\}/.exec(src)?.[0] ?? '';
  const locked = /if \(!doc \|\| doc\.info\.needsPassword\) \{([\s\S]*?)\n  \}/.exec(run)?.[1] ?? '';
  for (const bit of ['compareSummaryEl.textContent = ""', 'compareListEl.tBodies[0]?.replaceChildren()', 'canvas.width = 0']) assert.ok(locked.includes(bit), `R2: locked main clears ${bit}`);
  const unlocked = /async function applyUnlocked[\s\S]*?\n\}/.exec(src)?.[0] ?? '';
  assert.match(unlocked, /if \(compareDoc\) await refreshCompare\(true\);[^\n]*\n  emitResult/, 'R2: applyUnlocked ends with a compare refresh');
  // (c) R6: the unlock and a successful compare-open clear the error bar.
  assert.match(unlocked, /clearError\(\)/, 'R6: applyUnlocked clears the error bar');
  const openCmp = /async function openCompare[\s\S]*?\n\}/.exec(src)?.[0] ?? '';
  assert.match(openCmp, /compareLast = \{ action: "open", ok: true \};\s*clearError\(\);/, 'R6: a successful compare-open clears the error bar');
  // (f) R3: each refusal reason in its own words.
  assert.ok(src.includes('"copying is not permitted, only pixels compared"'), 'R3: read-only sentence');
  assert.ok(src.includes('"more than 2000 lines on a page, only pixels compared"'), 'R3: too-many sentence');
  assert.ok(!src.includes('this document does not permit copying, so only pixels are compared'), 'R3: no single sentence for every refusal');
  // (g) R4 + R7: an emptied page field reverts, 4 rem wide; a caption per side; a spaced bar.
  assert.match(src, /comparePageBInput\.value\.trim\(\) !== "" && Number\.isInteger\(page\)/, 'R4: empty field reverts');
  assert.match(css, /#compare-page-b \{ width: 4rem; \}/, 'R4: field is 4rem wide');
  assert.match(css, /\.compare-bar \{[^}]*gap:/, 'R7: bar has a gap');
  assert.match(src, /compareNameA\.textContent = doc\?\.name[\s\S]{0,80}compareNameB\.textContent = c\.name/, 'R7: captions name both files');
  assert.match(page, /<p id="compare-name-b" class="compare-name"><\/p><div class="compare-b">/, 'R7: B caption sits above the B wrapper');
  // (d) R-E1: a failed open that dropped the slot clears the renderer mirror and says so.
  const opener = /async function openDocument[\s\S]*?\n\}/.exec(src)?.[0] ?? '';
  const failed = /\} catch \(err\) \{([\s\S]*?)return null;\n  \}/.exec(opener)?.[1] ?? '';
  assert.match(failed, /reply\?\.compareClosed === true/, 'R-E1: the open rejection is checked for compareClosed');
  assert.match(failed, /clearCompare\(\);/, 'R-E1: the renderer drops its compare state');
  assert.ok(failed.includes('announce("Compare closed: the two files together exceed the size limit")'), 'R-E1: announced');
});
