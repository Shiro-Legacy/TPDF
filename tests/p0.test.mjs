import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import test from 'node:test';
import { createEngine, toArrayBuffer, fieldValue } from './rust-engine.mjs';

const engine = createEngine();

test('form fixture annotation and AcroForm value survive save/reopen', async () => {
  const bytes = await readFile('fixtures/form.pdf');
  assert.equal(createHash('sha256').update(bytes).digest('hex'), 'b689bb44942a514b4416435e8d1981a0b5b868bdaa53918911b807b5549ee66b');
  const session = engine.createSession();
  const request = req => engine.dispatch({ id: 1, ...req }, session).reply;
  const opened = request({ op: 'open', name: 'form.pdf', pdf: toArrayBuffer(bytes) });
  assert.equal(opened.ok, true, opened.error);
  let again;
  try {
    assert.ok(session.doc.text(0).includes('PDF4All form'));
    // The square is added by the engine's op (it takes no interior colour; the check never read one).
    const square = request({ op: 'annot-add', page: 0, type: 'Square', rect: [100, 100, 300, 200], contents: 'PDF4All P0 note' });
    assert.equal(square.ok, true, square.error);
    const fields = request({ op: 'fields', page: 0 });
    assert.equal(fields.ok, true, fields.error);
    const field = fields.items.find(item => item.name === 'name');
    assert.ok(field);
    const set = request({ op: 'field-set', page: 0, widgetId: field.id, value: 'PDF4All P0' });
    assert.equal(set.ok, true, set.error);
    const saved = request({ op: 'save' });
    assert.equal(saved.ok, true, saved.error);
    again = engine.reopen(new Uint8Array(saved.bytes));
    const annot = again.annotsOf(0, 'Square')[0];
    assert.equal(annot?.string('Contents'), 'PDF4All P0 note');
    // The rect in page space, as the engine reads it back from the saved bytes (the file's /Rect carries the /RD border margin).
    const reader = engine.createSession();
    try {
      assert.equal(engine.dispatch({ id: 2, op: 'open', name: 'saved.pdf', pdf: toArrayBuffer(new Uint8Array(saved.bytes)) }, reader).reply.ok, true);
      const items = engine.dispatch({ id: 3, op: 'annots', page: 0 }, reader).reply.items;
      assert.deepEqual(items.find(item => item.type === 'Square')?.rect, [100, 100, 300, 200]);
    } finally {
      engine.closeDocument(reader.doc);
    }
    assert.equal(fieldValue(again.widget(0, 'name')), 'PDF4All P0');
  } finally {
    engine.closeDocument(again);
    engine.closeDocument(session.doc);
  }
});
