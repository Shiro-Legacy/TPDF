/**
 * P6 U1 pure helpers. renderer.js starts a worker on import, so the asserts run
 * in a child that exits. The drive script is asserted in tests/p3-drive.test.mjs.
 */
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

if (process.env.P6_HELPERS === '1') {
  process.on('unhandledRejection', () => {});
  const el = () => new Proxy(function () {}, {
    get(_t, prop) {
      if (prop === 'style') return new Proxy({}, { get: () => '', set: () => true });
      if (prop === 'classList') return { add() {}, remove() {}, toggle() {}, contains: () => false };
      if (prop === 'dataset') return {};
      if (prop === 'children') return [];
      if (typeof prop === 'symbol' || prop === 'then') return undefined;
      return () => el();
    },
    set: () => true,
    apply: () => el(),
  });
  globalThis.window = globalThis;
  globalThis.Worker = class { constructor() {} postMessage() {} terminate() {} addEventListener() {} };
  globalThis.ResizeObserver = class { observe() {} disconnect() {} };
  globalThis.document = new Proxy({
    getElementById: () => el(), createElement: () => el(), createElementNS: () => el(),
    createDocumentFragment: () => el(), querySelector: () => null, querySelectorAll: () => [], addEventListener() {},
  }, { get(target, prop) { return prop in target ? target[prop] : () => el(); } });
  globalThis.localStorage = { getItem: () => null, setItem() {} };
  const { readingOrder, editorBox } = await import('../build/renderer/renderer.js');
  const field = (over) => ({ id: 1, name: 'a', label: 'a', kind: 'text', value: '', readOnly: false, required: false, scripted: false, rect: [0, 0, 10, 10], ...over });
  const right = field({ name: 'right', rect: [100, 0, 140, 20] });
  const left = field({ name: 'left', rect: [0, 2, 40, 22] });
  const below = field({ name: 'below', rect: [0, 40, 40, 60] });
  assert.deepEqual(readingOrder([right, below, left]).map(item => item.name), ['left', 'right', 'below']);
  const small = field({ name: 'size', kind: 'radio', on: 'S', value: false, rect: [0, 0, 10, 10] });
  const medium = field({ name: 'size', kind: 'radio', on: 'M', value: true, rect: [20, 0, 30, 10] });
  const large = field({ name: 'size', kind: 'radio', on: 'L', value: false, rect: [40, 0, 50, 10] });
  const locked = field({ name: 'fixed', readOnly: true, rect: [0, 30, 40, 50] });
  const sig = field({ name: 'sig', kind: 'signature', rect: [0, 60, 40, 80] });
  const ordered = readingOrder([large, sig, small, locked, medium]);
  assert.deepEqual(ordered.map(item => item.name), ['size', 'fixed']);
  assert.equal(ordered[0].on, 'M');
  assert.deepEqual(editorBox([10, 20, 110, 40], 2), { left: 20, top: 40, width: 200, height: 40, fontPx: 24 });
  assert.equal(editorBox([0, 0, 100, 10], 1).fontPx, 10);
  assert.equal(editorBox([0, 0, 50, 20], 1).fontPx, 12);
  assert.equal(editorBox([0, 0, 50, 1000], 1).fontPx, 400);
  // P6 U3 §1.1/§9.8: the page-transform round trip and the pure gesture rect.
  const { viewBox, pdfBox, gestureRect } = await import('../build/renderer/renderer.js');
  const flip = [1, 0, 0, -1, 0, 792];
  assert.deepEqual(viewBox([72, 600, 172, 650], flip), [72, 142, 172, 192]);
  assert.deepEqual(pdfBox([72, 142, 172, 192], flip), [72, 600, 172, 650]);
  const shifted = [1, 0, 0, -1, -10, 802]; // MediaBox [10 10 622 802]
  assert.deepEqual(viewBox([20, 700, 70, 750], shifted), [10, 52, 60, 102]);
  assert.deepEqual(pdfBox(viewBox([20.5, 700.25, 70, 750], shifted), shifted), [20.5, 700.25, 70, 750]);
  const rotated = [0, 1, 1, 0, 0, 0]; // /Rotate 90
  assert.deepEqual(viewBox([72, 600, 172, 650], rotated), [600, 72, 650, 172]);
  assert.deepEqual(pdfBox(viewBox([72, 600, 172, 650], rotated), rotated), [72, 600, 172, 650]);
  assert.deepEqual(pdfBox([0, 0, 1 / 3, 2 / 3], [1, 0, 0, 1, 0, 0]), [0, 0, 0.3333, 0.6667]);
  assert.throws(() => pdfBox([0, 0, 1, 1], [0, 0, 0, 0, 0, 0]), /not invertible/);
  const page = [612, 792], im1 = [72, 142, 172, 192];
  assert.deepEqual(gestureRect(im1, 'se', 20, 20, { min: 1, lockAspect: true, page }), [72, 142, 212, 212]);
  assert.deepEqual(gestureRect(im1, 'se', 20, 20, { min: 1, lockAspect: false, page }), [72, 142, 192, 212]);
  assert.deepEqual(gestureRect(im1, 'se', 1000, 0, { min: 1, lockAspect: true, page }), [72, 142, 612, 412]);
  assert.deepEqual(gestureRect(im1, 'nw', 150, 150, { min: 1, lockAspect: true, page }), [170, 191, 172, 192]);
  assert.deepEqual(gestureRect(im1, 'w', 200, 0, { min: 8, lockAspect: false, page }), [164, 142, 172, 192]);
  assert.deepEqual(gestureRect(im1, 'e', 0, 50, { min: 1, lockAspect: true, page }), [72, 142, 172, 192]);
  assert.deepEqual(gestureRect([400, 52, 460, 92], 'grip', 500, -500, { min: 1, lockAspect: false, page }), [552, 0, 612, 40]);
  // P6 U4 §2.2: tabOrder per /Tabs (tabs.pdf geometry: 120 × 20 fields, y-down).
  const { tabOrder } = await import('../build/renderer/renderer.js');
  const at = (name, x, y, over) => field({ name, rect: [x, y, x + 120, y + 20], ...over });
  const grid = [at('b3', 300, 172), at('a1', 72, 72), at('b1', 300, 72), at('a2', 72, 122), at('b2', 300, 122), at('a3', 72, 172)];
  const names = (result) => result.order.map(item => item.name);
  assert.deepEqual(names(tabOrder(grid, 'C')), ['a1', 'a2', 'a3', 'b1', 'b2', 'b3']);
  assert.equal(tabOrder(grid, 'C').used, 'C');
  assert.deepEqual(names(tabOrder(grid, 'R')), ['a1', 'b1', 'a2', 'b2', 'a3', 'b3']);
  assert.notDeepEqual(names(tabOrder(grid, 'R')), grid.map(item => item.name));
  assert.deepEqual(tabOrder(grid, null), { order: tabOrder(grid, 'R').order, used: 'R' });
  const row = [at('t3', 368, 72), at('t1', 72, 72), at('t2', 220, 72)];
  assert.deepEqual(names(tabOrder(row, 'A')), ['t3', 't1', 't2']);
  assert.deepEqual(tabOrder(row, 'W').used, 'W');
  const struct = [at('s1', 72, 72, { structIndex: 2 }), at('s2', 220, 72, { structIndex: 0 }), at('s3', 368, 72, { structIndex: 1 }), at('s4', 72, 200)];
  assert.deepEqual(names(tabOrder(struct, 'S')), ['s2', 's3', 's1', 's4']);
  assert.equal(tabOrder(struct, 'S').used, 'S');
  const bare = [at('u2', 220, 72), at('u1', 72, 72)];
  assert.deepEqual(tabOrder(bare, 'S'), { order: [bare[1], bare[0]], used: 'fallback' });
  const kids = [at('sig', 72, 300, { kind: 'signature' }), at('g', 300, 72, { kind: 'radio', on: 'B', value: true }), at('g', 72, 72, { kind: 'radio', on: 'A', value: false }), at('x', 72, 122)];
  assert.deepEqual(tabOrder(kids, 'A').order.map(item => `${item.name}${item.on ?? ''}`), ['gB', 'x']);
  assert.deepEqual(tabOrder(kids, 'C').order.map(item => `${item.name}${item.on ?? ''}`), ['gB', 'x']);
  assert.deepEqual(tabOrder(kids, null).order.map(item => `${item.name}${item.on ?? ''}`), ['gB', 'x']);
  // P6 U5 §2.2/§9.25: a group moves as its union (dy -80 clamps to -52); the menu duplicate goes +10/+10, -10 where no room.
  const { unionRect, menuDuplicateRect } = await import('../build/renderer/renderer.js');
  const union = unionRect([[72, 142, 172, 192], [400, 52, 460, 92]]);
  assert.deepEqual(union, [72, 52, 460, 192]);
  assert.deepEqual(gestureRect(union, 'grip', 0, -80, { min: 1, lockAspect: false, page }), [72, 0, 460, 140]);
  assert.deepEqual(menuDuplicateRect([400, 52, 460, 92], page), [410, 62, 470, 102]);
  assert.deepEqual(menuDuplicateRect([560, 760, 612, 792], page), [550, 750, 602, 782]);
  assert.deepEqual(menuDuplicateRect([0, 0, 612, 792], page), [0, 0, 612, 792]);
  // P6 U6 §2.4/§9.19: snapRect (view space, 4 pt, ties to the first moving edge then the first candidate, page last).
  const { snapRect, turnRect, mapRect } = await import('../build/renderer/renderer.js');
  const size = { width: 612, height: 792 }, im2 = [400, 42, 450, 142];
  assert.deepEqual(snapRect([398, 142, 498, 192], [im2], size), { rect: [400, 142, 500, 192], guides: [{ axis: 'x', at: 400 }, { axis: 'y', at: 142 }] });
  assert.deepEqual(snapRect([102, 300, 152, 350], [[115, 500, 135, 600]], size), { rect: [100, 300, 150, 350], guides: [{ axis: 'x', at: 125 }] });
  assert.deepEqual(snapRect([3, 300, 53, 350], [], size), { rect: [0, 300, 50, 350], guides: [{ axis: 'x', at: 0 }] });
  assert.deepEqual(snapRect([4, 300, 54, 350], [], size), { rect: [0, 300, 50, 350], guides: [{ axis: 'x', at: 0 }] });
  assert.deepEqual(snapRect([4.01, 300, 54.01, 350], [], size), { rect: [4.01, 300, 54.01, 350], guides: [] });
  assert.deepEqual(snapRect([398, 300, 448, 350], [im2], size, 4, ['x1']), { rect: [400, 300, 450, 350], guides: [{ axis: 'x', at: 450 }] });
  assert.deepEqual(snapRect([398, 300, 448, 350], [im2], size), { rect: [400, 300, 450, 350], guides: [{ axis: 'x', at: 400 }] });
  assert.deepEqual(snapRect([398, 300, 448, 350], [im2], size, 4, ['y0', 'y1']), { rect: [398, 300, 448, 350], guides: [] });
  // §2.3/§9.18: `within` (crop mode) replaces the page clamp and never locks the aspect.
  const im4 = [72, 352, 132, 392], full = [72, 342, 172, 392];
  assert.deepEqual(gestureRect(im4, 'w', -20, 0, { min: 1, lockAspect: true, page, within: full }), im4);
  assert.deepEqual(gestureRect(im4, 'ne', 40, -10, { min: 1, lockAspect: true, page, within: full }), full);
  assert.deepEqual(gestureRect(im4, 'grip', 100, 0, { min: 1, lockAspect: false, page, within: full }), [112, 352, 172, 392]);
  // §2.1/§2.2: a group turns about the union centre; a group-box resize maps members by the union affine.
  assert.deepEqual(turnRect([72, 142, 172, 192], 'rotateCw', 186, 167), [161, 53, 211, 153]);
  assert.deepEqual(turnRect([200, 142, 300, 192], 'rotateCw', 186, 167), [161, 181, 211, 281]);
  assert.deepEqual(turnRect([72, 142, 172, 192], 'rotateCcw', 122, 167), [97, 117, 147, 217]);
  assert.deepEqual(turnRect([72, 142, 172, 192], 'flipH', 186, 167), [200, 142, 300, 192]);
  assert.deepEqual(mapRect([200, 142, 300, 192], [72, 142, 300, 192], [72, 142, 528, 192]), [328, 142, 528, 192]);
  // P6 U7 §10.30: the DOM clipboard events belong to the page selection only outside text entry, dialogs, menus and editors.
  const { clipboardChordOk, pasteAnnouncement } = await import('../build/renderer/renderer.js');
  const ctx = (over = {}) => ({ mode: 'edit', layerLive: true, dialogOpen: false, menuOpen: false, editorOpen: false, active: null, collapsed: true, selection: 1, clipboard: true, editable: true, ...over });
  assert.equal(clipboardChordOk('copy', ctx()), true, '#page / body focus copies');
  assert.equal(clipboardChordOk('copy', ctx({ active: { tag: 'BUTTON', type: 'button' } })), true, 'a thumbnail button copies');
  assert.equal(clipboardChordOk('copy', ctx({ active: { tag: 'INPUT', type: 'radio' } })), true, 'a radio input copies');
  assert.equal(clipboardChordOk('copy', ctx({ active: { tag: 'INPUT', type: 'text' } })), false, 'a text input keeps native copy');
  assert.equal(clipboardChordOk('copy', ctx({ active: { tag: 'INPUT', type: 'text', inlineEditor: true } })), false);
  assert.equal(clipboardChordOk('paste', ctx({ active: { tag: 'TEXTAREA' } })), false);
  assert.equal(clipboardChordOk('copy', ctx({ active: { tag: 'DIV', contentEditable: true } })), false);
  assert.equal(clipboardChordOk('copy', ctx({ dialogOpen: true })), false, 'an open dialog owns the chord');
  assert.equal(clipboardChordOk('copy', ctx({ collapsed: false })), false, 'a text selection keeps native copy');
  assert.equal(clipboardChordOk('copy', ctx({ menuOpen: true })), false);
  assert.equal(clipboardChordOk('cut', ctx({ editorOpen: true })), false);
  assert.equal(clipboardChordOk('copy', ctx({ selection: 0 })), false);
  assert.equal(clipboardChordOk('paste', ctx({ selection: 0 })), true, 'paste needs no selection');
  assert.equal(clipboardChordOk('paste', ctx({ clipboard: false })), false);
  assert.equal(clipboardChordOk('paste', ctx({ editable: false })), false);
  assert.equal(clipboardChordOk('copy', ctx({ mode: 'read' })), false, 'Read has no selection');
  assert.equal(clipboardChordOk('copy', ctx({ layerLive: false })), false);
  // §10.33: the paste announcement.
  assert.equal(pasteAnnouncement({ image: 1 }, 1, 0, 'edit', ['PA1']), 'Pasted image PA1');
  assert.equal(pasteAnnouncement({ image: 1, form: 1 }, 2, 0, 'edit', ['PA1', 'PA2']), 'Pasted 2 objects');
  assert.equal(pasteAnnouncement({ link: 2 }, 1, 1, 'annotate', []), 'Pasted 1 item; 1 link skipped (its page is not in this document)');
  assert.equal(pasteAnnouncement({ image: 1, field: 2, mark: 1 }, 4, 0, 'edit', ['PA1']), 'Pasted 4 items; 2 fields shown in Fill forms; 1 redaction mark shown in Redact');
  assert.equal(pasteAnnouncement({ text: 1 }, 1, 0, 'forms', []), 'Pasted 1 item; 1 text run shown in Edit text');
  // Debt sweep 2 §9.10: the Cut announcement agrees in number with its text runs.
  const { cutAnnouncement } = await import('../build/renderer/renderer.js');
  assert.equal(cutAnnouncement(1, 1), 'Cut 1 item; 1 text run stays (it cannot be deleted)');
  assert.equal(cutAnnouncement(2, 2), 'Cut 2 items; 2 text runs stay (they cannot be deleted)');
  assert.equal(cutAnnouncement(3, 0), 'Cut 3 items');
  process.exit(0);
}

test('readingOrder and editorBox match §8', () => {
  const child = spawnSync(process.execPath, [fileURLToPath(import.meta.url)], {
    env: { ...process.env, P6_HELPERS: '1' }, encoding: 'utf8',
  });
  assert.equal(child.status, 0, child.stderr || child.stdout);
});

// P6 U5 review R1-R3 (real-mouse repro: .quad/shared/p6/checks/review-u5-renderer-probe.mjs K1, K2, b3x, e1). The drive
// cannot release after Escape or read the Inspector form, so the source keeps the three wirings (each absent pre-fix).
test('U5 review: undo keeps the selection, Inspector follows the anchor, Escaped gesture adds no text', async () => {
  const { readFile } = await import('node:fs/promises');
  const src = await readFile(fileURLToPath(new URL('../src/renderer/renderer.ts', import.meta.url)), 'utf8');
  const body = name => { const at = src.indexOf(`function ${name}(`); assert.ok(at >= 0, name); return src.slice(at, src.indexOf('\n}\n', at)); };
  for (const name of ['undoEdit', 'redoEdit']) assert.doesNotMatch(body(name), /clearDirectFocus\(\)|directSelection = \[\]/, `${name} keeps the selection (§9.28)`);
  assert.equal(body('toggleObject').match(/syncObjectInspector\(/g)?.length, 2, 'Shift-click add and remove sync the Inspector');
  assert.match(body('selectCopies'), /syncObjectInspector\(directFocusedObject\.targetId\)/);
  assert.match(body('startRectGesture'), /const cancel = [^\n]*addBlocked = true/, 'Escape mid-drag blocks the trailing click-to-add');
});

// P6 U6 review R1-R4 (real-mouse repro: .quad/shared/p6/checks/review-u6-renderer-probe.mjs c4/b5, b8, c9/c12, a3). The drive never
// releases off the object, focuses an Inspector button or reads the form, so the source keeps the four wirings (each absent pre-fix).
test('U6 review: drag release swallows its click, gesture Escape skips the button, form Escape leaves crop, restore syncs Inspector', async () => {
  const { readFile } = await import('node:fs/promises');
  const src = await readFile(fileURLToPath(new URL('../src/renderer/renderer.ts', import.meta.url)), 'utf8');
  const body = name => { const at = src.indexOf(`function ${name}(`); assert.ok(at >= 0, name); return src.slice(at, src.indexOf('\n}\n', at)); };
  assert.match(body('swallowNextClick'), /addEventListener\("click", swallow, \{ capture: true, once: true \}\)/);
  assert.match(body('startRectGesture'), /const gesture = directGesture; directGesture = null;\n\s+if \(moved\) swallowNextClick\(\);/, 'a moved gesture swallows its trailing click (R1)');
  assert.match(body('startRectGesture'), /const cancel = [^\n]*addEventListener\("pointerup", swallowNextClick, \{ once: true \}\)/, 'an Escaped gesture swallows the release click (R1)');
  assert.match(src, /button\.addEventListener\("keydown", event => \{\n\s+\/\/[^\n]*\n\s+if \(event\.key === "Escape" && directGesture\) return;\n\s+if \(cropping\(\)\) \{/, 'gesture Escape is the window\'s outside crop mode too (R2)');
  assert.match(src, /contentObjectForm\.addEventListener\("keydown", event => \{[^}]*if \(directCrop\) \{ leaveCrop\(\); return; \}/, 'form Escape leaves crop first (R3)');
  assert.match(body('restoreDirectFocus'), /syncObjectInspector\(directFocusedObject\.targetId\)/, 'restoreDirectFocus syncs a stale Inspector (R4)');
});

// P6 U7 §10.35: ⌘C/⌘X/⌘V reach the renderer only as DOM clipboard events (the Edit-menu roles); no keydown handler reads them.
test('U7: no keydown handler owns the clipboard chords', async () => {
  const { readFile } = await import('node:fs/promises');
  const src = await readFile(fileURLToPath(new URL('../src/renderer/renderer.ts', import.meta.url)), 'utf8');
  assert.doesNotMatch(src, /key(?:\.toLowerCase\(\))? === "[cvx]"/, 'no key === "c" / "v" / "x" test');
  assert.doesNotMatch(src, /key === "[CVX]"/);
  assert.match(src, /\["copy", "cut", "paste"\] as const\) document\.addEventListener\(kind/, 'one DOM listener per clipboard event');
});

// P6 U7 review F2 (real-mouse repro: .quad/shared/p6/checks/review-u7-renderer-probe.mjs misc m1/m2). The drive's `menu` never sends a
// secondary pointerdown, so the source keeps the guard: no armed tool (Mark area, Draw link, signature field, ink) starts on it (absent pre-fix).
test('U7 review: a secondary or Control-click never starts an armed area gesture', async () => {
  const { readFile } = await import('node:fs/promises');
  const src = await readFile(fileURLToPath(new URL('../src/renderer/renderer.ts', import.meta.url)), 'utf8');
  const at = src.indexOf('P6 U4 §9.14: armed redact'); assert.ok(at >= 0);
  const guard = src.indexOf('if (event.button !== 0 || event.ctrlKey) return;', at), armed = src.indexOf('if (redactAreaButton.getAttribute("aria-pressed") === "true")', at);
  assert.ok(guard > at && guard < armed, 'the secondary-click guard precedes every armed branch');
});
