import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import test from 'node:test';
import mupdf from 'mupdf';

test('form fixture annotation and AcroForm value survive save/reopen', async () => {
  const bytes = await readFile('fixtures/form.pdf');
  assert.equal(createHash('sha256').update(bytes).digest('hex'), 'b689bb44942a514b4416435e8d1981a0b5b868bdaa53918911b807b5549ee66b');
  const doc = mupdf.Document.openDocument(bytes, 'application/pdf').asPDF();
  assert.ok(doc);
  let page, saved, again, page2;
  try {
    doc.disableJS();
    page = doc.loadPage(0);
    assert.ok(page.toStructuredText().asText().includes('PDF4All form'));
    const square = page.createAnnotation('Square');
    square.setRect([100, 100, 300, 200]);
    square.setInteriorColor([1, 0, 0]);
    square.setContents('PDF4All P0 note');
    square.update();
    const field = page.getWidgets().find(widget => widget.getName() === 'name');
    assert.ok(field);
    field.setTextValue('PDF4All P0');
    page.update();
    saved = doc.saveToBuffer('incremental=no');
    again = mupdf.Document.openDocument(saved.asUint8Array(), 'application/pdf').asPDF();
    page2 = again.loadPage(0);
    const annot = page2.getAnnotations().find(item => item.getType() === 'Square');
    assert.equal(annot?.getContents(), 'PDF4All P0 note');
    assert.deepEqual([...annot.getRect()], [100, 100, 300, 200]);
    assert.equal(page2.getWidgets().find(widget => widget.getName() === 'name')?.getValue(), 'PDF4All P0');
  } finally {
    page2?.destroy(); again?.destroy(); saved?.destroy(); page?.destroy(); doc.destroy();
  }
});
