import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import mupdf from 'mupdf';
import { build } from 'esbuild';
import { createEngine } from '../../build/renderer/engine.js';

const bundled = await build({ entryPoints: ['src/geometry.ts'], bundle: true, format: 'esm', write: false });
const { pagePoint, highlightQuads, placeSignature } = await import(`data:text/javascript;base64,${Buffer.from(bundled.outputFiles[0].text).toString('base64')}`);
const engine = createEngine(mupdf);
function client() {
  const session = engine.createSession(); let id = 0;
  return request => {
    const { reply } = engine.dispatch({ ...request, id: ++id }, session);
    assert.equal(reply.ok, true, JSON.stringify(reply)); return reply;
  };
}
async function fixture(name) { const bytes = await readFile(`../fixtures/${name}.pdf`); return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength); }

test('mobile annotate, fill, sign, undo, validate, save and fresh reopen preserve edits', async () => {
  const send = client();
  const opened = send({ op: 'open', pdf: await fixture('form'), name: 'form.pdf' });
  assert.ok(opened.pageCount > 0);
  const rendered = send({ op: 'render', page: 0, scale: .6 });
  assert.equal(rendered.pixels.byteLength, rendered.pixelWidth * rendered.pixelHeight * 4);
  const fields = send({ op: 'fields', page: 0 }).items;
  const field = fields.find(field => field.kind === 'text' && !field.readOnly);
  assert.ok(field);
  send({ op: 'field-set', page: 0, widgetId: field.id, value: 'Mobile test' });
  send({ op: 'annot-add', page: 0, type: 'Text', point: [40, 40], contents: 'A mobile note' });
  send({ op: 'annot-add', page: 0, type: 'Ink', strokes: [[[60, 80], [90, 110], [110, 90]]], contents: '' });
  const signatures = placeSignature([[[0, 0], [20, 20], [40, 0]]], [200, 180], opened.pages[0].width, opened.pages[0].height);
  send({ op: 'annot-add', page: 0, type: 'Ink', strokes: signatures, signature: true, contents: '' });
  const beforeUndo = send({ op: 'annots', page: 0 }).items.length;
  send({ op: 'undo' }); assert.equal(send({ op: 'annots', page: 0 }).items.length, beforeUndo - 1);
  send({ op: 'redo' }); assert.equal(send({ op: 'annots', page: 0 }).items.length, beforeUndo);
  const saved = send({ op: 'save' });
  const validated = send({ op: 'validate', pdf: saved.bytes });
  const fresh = client(); fresh({ op: 'open', pdf: validated.bytes, name: 'saved.pdf' });
  assert.equal(fresh({ op: 'fields', page: 0 }).items.find(item => item.name === field.name).value, 'Mobile test');
  const annots = fresh({ op: 'annots', page: 0 }).items;
  assert.ok(annots.some(item => item.contents === 'A mobile note'));
  assert.ok(annots.some(item => item.subject === 'Drawn signature'));
  assert.equal(send({ op: 'saved' }).dirty, false);
  send({ op: 'close' }); fresh({ op: 'close' });
});

test('mobile line selection writes highlights which survive save/reopen', async () => {
  const send = client(); send({ op: 'open', pdf: await fixture('hello'), name: 'hello.pdf' });
  const render = send({ op: 'render', page: 0, scale: 1 });
  const line = render.lines[0]; assert.ok(line);
  const quads = highlightQuads(render.lines, [line.x, line.y], [line.x + line.w, line.y + line.h]);
  assert.ok(quads.length);
  send({ op: 'annot-add', page: 0, type: 'Highlight', quads, contents: '' });
  const bytes = send({ op: 'save' }).bytes;
  send({ op: 'open', pdf: bytes, name: 'highlighted.pdf' });
  assert.ok(send({ op: 'annots', page: 0 }).items.some(item => item.type === 'Highlight'));
  send({ op: 'close' });
});

test('touch coordinates and signatures stay inside the page at zoom and edges', () => {
  assert.deepEqual(pagePoint(210, 420, { left: 10, top: 20, width: 400, height: 800 }, 600, 1200), [300, 600]);
  assert.deepEqual(pagePoint(-100, 9999, { left: 10, top: 20, width: 400, height: 800 }, 600, 1200), [0, 1200]);
  for (const at of [[0, 0], [600, 800]]) {
    for (const [x, y] of placeSignature([[[10, 10], [100, 50]]], at, 600, 800).flat()) {
      assert.ok(x >= 0 && x <= 600); assert.ok(y >= 0 && y <= 800);
    }
  }
});
