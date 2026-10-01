/**
 * P2 unit 4 — AcroForm fill: list the widgets on a page, set one value through
 * one journal operation, refuse what must be refused, and regenerate
 * appearances so a reopened copy shows what was typed.
 *
 * Runs against src/renderer/engine.ts exactly as the worker does. MuPDF's own
 * setters do not validate (measured: `setTextValue` happily wrote 5000 chars
 * into a /MaxLen 20 field and into a read-only field, all returning 1), so every
 * bound in this file comes from the engine's own checks.
 */
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import mupdf from 'mupdf';
import { createEngine, MAX_NOTE_CHARS } from '../build/renderer/engine.js';

const engine = createEngine(mupdf);

function toArrayBuffer(buffer) {
  return buffer.buffer.slice(buffer.byteOffset, buffer.byteOffset + buffer.byteLength);
}

async function openBytes(bytes, name = 'scratch.pdf', session = engine.createSession()) {
  const reply = engine.dispatch({ id: 1, op: 'open', name, pdf: bytes }, session).reply;
  assert.equal(reply.ok, true, reply.error);
  return session;
}

async function open(id, session = engine.createSession()) {
  return openBytes(toArrayBuffer(await readFile(`fixtures/${id}.pdf`)), `${id}.pdf`, session);
}

function op(session, request) {
  const reply = engine.dispatch({ id: 900 + (request.id ?? 0), ...request }, session).reply;
  return reply;
}

function fields(session) {
  const reply = op(session, { op: 'fields', page: 0 });
  assert.equal(reply.ok, true, reply.error);
  return reply.items;
}

function byName(session, name) {
  return fields(session).filter(item => item.name === name);
}

function set(session, widgetId, value) {
  return op(session, { op: 'field-set', page: 0, widgetId, value });
}

function reopen(bytes) {
  const doc = mupdf.Document.openDocument(bytes, 'application/pdf');
  doc.asPDF().disableJS();
  return doc;
}

/** Object-numbering-safe PDF from a 1-based list of indirect object bodies. */
function buildPdf(objects) {
  const parts = ['%PDF-1.4\n'];
  const offsets = [];
  objects.forEach((body, index) => {
    offsets.push(parts.join('').length);
    parts.push(`${index + 1} 0 obj\n${body}\nendobj\n`);
  });
  const xref = parts.join('').length;
  parts.push(`xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`);
  for (const offset of offsets) parts.push(`${String(offset).padStart(10, '0')} 00000 n \n`);
  parts.push(`trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`);
  return toArrayBuffer(Buffer.from(parts.join(''), 'latin1'));
}

function appearance(w, h, glyph) {
  const stream = `q 1 1 ${w} ${h} re W n 0 g BT /Helv 10 Tf 2 2 Td (${glyph}) Tj ET Q\n`;
  return `<< /Type /XObject /Subtype /Form /BBox [0 0 ${w} ${h}] /Resources << /Font << /Helv 5 0 R >> >> `
    + `/Length ${stream.length} >>\nstream\n${stream}endstream`;
}

/**
 * The contract's own field set, built here so these tests do not depend on the
 * shared fixture's object numbering: name (MaxLen 20), notes (multiline), agree
 * (checkbox /Yes), size (radio /S /M /L), choice (combo), pick (list box),
 * fixed (read-only), amount (/AA JavaScript, listed in /CO), sig.
 */
function richFormPdf() {
  const captions = ['name', 'notes', 'agree', 'size', 'choice', 'pick', 'fixed', 'amount', 'sig']
    .map((label, index) => `BT /F1 9 Tf 40 ${700 - index * 42} Td (${label}) Tj ET\n`).join('');
  const content = `BT /F1 20 Tf 40 750 Td (PDF4All form2) Tj ET\n${captions}`;
  return buildPdf([
    '<< /Type /Catalog /Pages 2 0 R /AcroForm 6 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] '
      + '/Resources << /Font << /F1 4 0 R >> >> /Contents 7 0 R '
      + '/Annots [8 0 R 9 0 R 10 0 R 12 0 R 13 0 R 14 0 R 15 0 R 16 0 R 17 0 R 18 0 R 19 0 R] >>',
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>',
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>',
    '<< /Fields [8 0 R 9 0 R 10 0 R 11 0 R 15 0 R 16 0 R 17 0 R 18 0 R 19 0 R] '
      + '/DR << /Font << /Helv 5 0 R >> >> /DA (/Helv 12 Tf 0 g) /CO [18 0 R] >>',
    `<< /Length ${content.length} >>\nstream\n${content}endstream`,
    '<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /V () /DV () /MaxLen 20 '
      + '/Rect [120 700 340 716] /F 4 /P 3 0 R /DA (/Helv 12 Tf 0 g) >>',
    '<< /Type /Annot /Subtype /Widget /FT /Tx /Ff 4096 /T (notes) /V () '
      + '/Rect [120 658 340 690] /F 4 /P 3 0 R /DA (/Helv 12 Tf 0 g) >>',
    '<< /Type /Annot /Subtype /Widget /FT /Btn /T (agree) /V /Off /Rect [120 616 132 628] /F 4 /P 3 0 R '
      + '/DA (/Helv 12 Tf 0 g) /AP << /N << /Yes 20 0 R /Off 21 0 R >> >> >>',
    '<< /FT /Btn /Ff 32768 /T (size) /Kids [12 0 R 13 0 R 14 0 R] /V /Off >>',
    '<< /Type /Annot /Subtype /Widget /Parent 11 0 R /AS /Off /Rect [120 580 132 592] /P 3 0 R '
      + '/AP << /N << /S 20 0 R /Off 21 0 R >> >> /DA (/Helv 12 Tf 0 g) >>',
    '<< /Type /Annot /Subtype /Widget /Parent 11 0 R /AS /Off /Rect [160 580 172 592] /P 3 0 R '
      + '/AP << /N << /M 20 0 R /Off 21 0 R >> >> /DA (/Helv 12 Tf 0 g) >>',
    '<< /Type /Annot /Subtype /Widget /Parent 11 0 R /AS /Off /Rect [200 580 212 592] /P 3 0 R '
      + '/AP << /N << /L 20 0 R /Off 21 0 R >> >> /DA (/Helv 12 Tf 0 g) >>',
    '<< /Type /Annot /Subtype /Widget /FT /Ch /Ff 131072 /T (choice) /V () /Opt [(one) (two) (three)] '
      + '/Rect [120 538 340 554] /F 4 /P 3 0 R /DA (/Helv 12 Tf 0 g) >>',
    '<< /Type /Annot /Subtype /Widget /FT /Ch /T (pick) /V () /Opt [(alpha) (beta) (gamma)] '
      + '/Rect [120 466 340 526] /F 4 /P 3 0 R /DA (/Helv 12 Tf 0 g) >>',
    '<< /Type /Annot /Subtype /Widget /FT /Tx /Ff 1 /T (fixed) /V (locked) '
      + '/Rect [120 424 340 440] /F 4 /P 3 0 R /DA (/Helv 12 Tf 0 g) >>',
    '<< /Type /Annot /Subtype /Widget /FT /Tx /T (amount) /V () '
      + '/Rect [120 382 340 398] /F 4 /P 3 0 R /DA (/Helv 12 Tf 0 g) /AA << /F << /S /JavaScript '
      + '/JS (AFNumber_Format\\(2,0,0,0,"",true\\)) >> >> >>',
    '<< /Type /Annot /Subtype /Widget /FT /Sig /T (sig) /Rect [120 340 340 356] /F 4 /P 3 0 R >>',
    appearance(12, 12, 'X'),
    appearance(12, 12, ' '),
  ]);
}

function richForm() {
  return openBytes(richFormPdf(), 'richform.pdf');
}

function signedPdf() {
  return buildPdf([
    '<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Annots [6 0 R] >>',
    '<< /Length 2 >>\nstream\nq \nendstream',
    '<< /Fields [6 0 R] /SigFlags 3 >>',
    '<< /Type /Annot /Subtype /Widget /FT /Sig /T (sig1) /Rect [72 72 300 120] /P 3 0 R /V 7 0 R >>',
    '<< /Type /Sig /Filter /Adobe.PPKLite >>',
  ]);
}

// ------------------------------------------------------------------ listing

test('form.pdf: the one text field lists with its kind, value and no script', async () => {
  const session = await open('form');
  try {
    const items = fields(session);
    assert.equal(items.length, 1);
    const [item] = items;
    assert.equal(item.kind, 'text');
    assert.equal(item.name, 'name');
    assert.equal(item.value, '');
    assert.equal(item.readOnly, false);
    assert.equal(item.scripted, false);
    assert.equal(item.multiline, undefined);
    assert.ok(Number.isInteger(item.id));
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('form2/rich form: every contract kind lists with flags only amount is scripted', async () => {
  const session = await richForm();
  try {
    const items = fields(session);
    const names = items.map(item => `${item.name}:${item.kind}`);
    assert.deepEqual(names, [
      'name:text', 'notes:text', 'agree:checkbox', 'size:radio', 'size:radio', 'size:radio',
      'choice:combo', 'pick:list', 'fixed:text', 'amount:text', 'sig:signature',
    ]);
    const item = name => items.find(entry => entry.name === name);
    assert.equal(item('name').maxLen, 20);
    assert.equal(item('notes').multiline, true);
    assert.equal(item('agree').on, 'Yes');
    assert.equal(item('agree').value, false);
    assert.deepEqual(items.filter(entry => entry.kind === 'radio').map(entry => entry.on), ['S', 'M', 'L']);
    assert.deepEqual(item('choice').options, ['one', 'two', 'three']);
    assert.deepEqual(item('pick').options, ['alpha', 'beta', 'gamma']);
    assert.equal(item('fixed').readOnly, true);
    assert.equal(item('fixed').value, 'locked');
    assert.equal(item('sig').kind, 'signature');
    assert.deepEqual(items.filter(entry => entry.scripted).map(entry => entry.name), ['amount']);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('hello.pdf has nothing to fill', async () => {
  const session = await open('hello');
  try {
    assert.deepEqual(fields(session), []);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('large.pdf: listing 500 pages of fields stays inside the request budget', async () => {
  const session = await open('large');
  try {
    const started = performance.now();
    for (let page = 0; page < 20; page++) {
      const reply = op(session, { op: 'fields', page });
      assert.equal(reply.ok, true, reply.error);
      assert.deepEqual(reply.items, []);
    }
    assert.ok(performance.now() - started < 2000, '20 empty pages of fields took too long');
  } finally {
    engine.closeDocument(session.doc);
  }
});

// ------------------------------------------------------------------ setting

test('set: each kind takes its own value and journals one fill field operation', async () => {
  const session = await richForm();
  try {
    const item = name => fields(session).find(entry => entry.name === name);

    const text = item('name');
    const textReply = set(session, text.id, 'PDF4All P2 form');
    assert.equal(textReply.ok, true, textReply.error);
    assert.equal(textReply.dirty, true);
    assert.equal(textReply.revision, 1);

    const notes = item('notes');
    assert.equal(set(session, notes.id, 'line one\nline two').ok, true);
    const agree = item('agree');
    assert.equal(set(session, agree.id, true).ok, true);
    const choice = item('choice');
    assert.equal(set(session, choice.id, 'two').ok, true);
    const pick = item('pick');
    assert.equal(set(session, pick.id, 'beta').ok, true);
    const radioM = fields(session).find(entry => entry.kind === 'radio' && entry.on === 'M');
    assert.equal(set(session, radioM.id, true).ok, true);

    const after = fields(session);
    const value = name => after.find(entry => entry.name === name).value;
    assert.equal(value('name'), 'PDF4All P2 form');
    assert.equal(value('notes'), 'line one\nline two');
    assert.equal(value('agree'), true);
    assert.equal(value('choice'), 'two');
    assert.equal(value('pick'), 'beta');
    assert.deepEqual(after.filter(entry => entry.kind === 'radio' && entry.value === true).map(entry => entry.on), ['M']);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('set: a checkbox toggles only when the wanted state differs', async () => {
  const session = await richForm();
  try {
    const agree = fields(session).find(entry => entry.name === 'agree');
    const on = set(session, agree.id, true);
    const off = set(session, agree.id, false);
    const again = set(session, agree.id, false);
    assert.equal(on.revision, 1);
    assert.equal(off.revision, 2);
    // Nothing to change: no operation, no revision bump, still ok.
    assert.equal(again.ok, true);
    assert.equal(again.revision, 2);
    assert.equal(fields(session).find(entry => entry.name === 'agree').value, false);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('set: undo and redo restore every value the fill touched', async () => {
  const session = await richForm();
  try {
    const name = fields(session).find(entry => entry.name === 'name');
    const agree = fields(session).find(entry => entry.name === 'agree');
    const choice = fields(session).find(entry => entry.name === 'choice');
    assert.equal(set(session, name.id, 'typed').ok, true);
    assert.equal(set(session, agree.id, true).ok, true);
    assert.equal(set(session, choice.id, 'two').ok, true);

    const undone = op(session, { op: 'undo' });
    assert.equal(undone.ok, true, undone.error);
    assert.equal(undone.step, 'fill field');
    assert.equal(fields(session).find(entry => entry.name === 'choice').value, '');
    const redone = op(session, { op: 'redo' });
    assert.equal(redone.ok, true, redone.error);
    assert.equal(fields(session).find(entry => entry.name === 'choice').value, 'two');
    assert.equal(fields(session).find(entry => entry.name === 'name').value, 'typed');
    assert.equal(fields(session).find(entry => entry.name === 'agree').value, true);
  } finally {
    engine.closeDocument(session.doc);
  }
});

// ------------------------------------------------------------------ refusals

test('refusals: MaxLen, not-an-option, read-only field, signature, radio false, bad id', async () => {
  const session = await richForm();
  try {
    const item = name => fields(session).find(entry => entry.name === name);
    const radio = fields(session).find(entry => entry.kind === 'radio');
    const before = fields(session);
    const cases = [
      [item('name'), 'x'.repeat(21), 'at most 20'],
      [item('name'), 'x'.repeat(MAX_NOTE_CHARS + 1), `at most ${MAX_NOTE_CHARS}`],
      [item('choice'), 'bogus', 'not one of this field'],
      [item('pick'), 'delta', 'not one of this field'],
      [item('fixed'), 'nope', 'read-only'],
      [item('sig'), 'nope', 'click the signature field'],
      [radio, false, 'cannot be turned off'],
      [null, 'nope', 'no form field'],
      [item('name'), true, 'takes a string'],
      [item('agree'), 'yes', 'takes true or false'],
    ];
    for (const [widget, value, expected] of cases) {
      const reply = set(session, widget ? widget.id : 999999, value);
      assert.equal(reply.ok, false, `${expected} was accepted`);
      assert.match(reply.error, new RegExp(expected));
    }
    // A refusal is not a mutation: nothing changed and nothing is dirty.
    assert.deepEqual(fields(session), before);
    assert.equal(engine.journalState(session).dirty, false);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('refusals: a signed document lists its fields but fills none of them', async () => {
  const session = await openBytes(signedPdf(), 'signed.pdf');
  try {
    const items = fields(session);
    assert.deepEqual(items.map(entry => entry.kind), ['signature']);
    const reply = set(session, items[0].id, 'x');
    assert.equal(reply.ok, false);
    assert.equal(reply.reason, 'read-only');
  } finally {
    engine.closeDocument(session.doc);
  }
});

// ------------------------------------------------------------------ round trip

test('save → validate → reopen keeps every value and gives each set widget an /AP', async () => {
  const session = await richForm();
  try {
    const item = name => fields(session).find(entry => entry.name === name);
    const touched = [
      item('name').id, item('agree').id, item('choice').id, item('pick').id,
      fields(session).find(entry => entry.kind === 'radio' && entry.on === 'M').id,
    ];
    assert.equal(set(session, touched[0], 'PDF4All P2 form').ok, true);
    assert.equal(set(session, item('agree').id, true).ok, true);
    assert.equal(set(session, item('choice').id, 'two').ok, true);
    assert.equal(set(session, item('pick').id, 'beta').ok, true);
    assert.equal(set(session, fields(session).find(entry => entry.kind === 'radio' && entry.on === 'M').id, true).ok, true);

    const save = op(session, { op: 'save' });
    assert.equal(save.ok, true, save.error);
    const validated = op(session, { op: 'validate', pdf: save.bytes });
    assert.equal(validated.ok, true, validated.error);

    const doc = reopen(validated.bytes);
    try {
      const page = doc.loadPage(0);
      try {
        const widgets = page.getWidgets();
        assert.equal(widgets.length, 11);
        const byId = id => widgets.find(widget => widget.getObject().asIndirect() === id);
        for (const id of touched) {
          const widget = byId(id);
          assert.ok(widget, `widget ${id} survived the save`);
          const ap = widget.getObject().get('AP');
          assert.ok(ap && !ap.isNull(), `widget ${id} has no /AP`);
        }
        assert.equal(byId(touched[0]).getValue(), 'PDF4All P2 form');
        assert.equal(byId(item('choice').id).getValue(), 'two');
        assert.equal(byId(item('pick').id).getValue(), 'beta');
        assert.equal(byId(item('agree').id).getValue(), 'Yes');
        const reopened = doc.loadPage(0).getWidgets().filter(widget => widget.getName() === 'size');
        assert.deepEqual(reopened.map(widget => String(widget.getObject().get('AS'))), ['/Off', '/M', '/Off']);
      } finally {
        page.destroy();
      }
    } finally {
      doc.destroy();
    }
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('P0: form.pdf still round-trips its text field through save, validate and reopen', async () => {
  const session = await open('form');
  try {
    const item = fields(session)[0];
    assert.equal(set(session, item.id, 'PDF4All P0').ok, true);
    const save = op(session, { op: 'save' });
    assert.equal(save.ok, true, save.error);
    const validated = op(session, { op: 'validate', pdf: save.bytes });
    assert.equal(validated.ok, true, validated.error);
    const doc = reopen(validated.bytes);
    try {
      const page = doc.loadPage(0);
      try {
        const widget = page.getWidgets().find(entry => entry.getName() === 'name');
        assert.equal(widget.getValue(), 'PDF4All P0');
        const ap = widget.getObject().get('AP');
        assert.ok(ap && !ap.isNull());
      } finally {
        page.destroy();
      }
    } finally {
      doc.destroy();
    }
  } finally {
    engine.closeDocument(session.doc);
  }
});

// ------------------------------------------------------------------ shared fixture

test('form2.pdf: the shared fixture lists the contract fields, then fills and round-trips', async t => {
  if (!existsSync('fixtures/form2.pdf')) {
    t.skip('fixtures/form2.pdf arrives with the fixture deliverable (grok)');
    return;
  }
  const session = await open('form2');
  try {
    const items = fields(session);
    const item = name => items.find(entry => entry.name === name);
    for (const name of ['name', 'notes', 'agree', 'size', 'choice', 'pick', 'fixed', 'amount', 'sig']) {
      assert.ok(item(name), `${name} is listed`);
    }
    assert.equal(item('name').kind, 'text');
    assert.equal(item('name').maxLen, 20);
    assert.equal(item('notes').multiline, true);
    assert.equal(item('agree').kind, 'checkbox');
    assert.equal(item('agree').on, 'Yes');
    assert.deepEqual(items.filter(entry => entry.kind === 'radio').map(entry => entry.on), ['S', 'M', 'L']);
    assert.deepEqual(item('choice').options, ['one', 'two', 'three']);
    assert.deepEqual(item('pick').options, ['alpha', 'beta', 'gamma']);
    assert.equal(item('fixed').readOnly, true);
    assert.equal(item('sig').kind, 'signature');
    assert.deepEqual(items.filter(entry => entry.scripted).map(entry => entry.name), ['amount']);

    // The smoke's own path, field by field.
    assert.equal(set(session, item('name').id, 'PDF4All P2 form').ok, true);
    assert.equal(set(session, item('agree').id, true).ok, true);
    assert.equal(set(session, item('choice').id, 'two').ok, true);
    const radioM = fields(session).find(entry => entry.kind === 'radio' && entry.on === 'M');
    assert.equal(set(session, radioM.id, true).ok, true);
    const after = fields(session);
    const value = name => after.find(entry => entry.name === name).value;
    assert.equal(value('name'), 'PDF4All P2 form');
    assert.equal(value('agree'), true);
    assert.equal(value('choice'), 'two');
    assert.deepEqual(after.filter(entry => entry.kind === 'radio' && entry.value).map(entry => entry.on), ['M']);

    const save = op(session, { op: 'save' });
    assert.equal(save.ok, true, save.error);
    const validated = op(session, { op: 'validate', pdf: save.bytes });
    assert.equal(validated.ok, true, validated.error);
    const doc = reopen(validated.bytes);
    try {
      const page = doc.loadPage(0);
      try {
        for (const widget of page.getWidgets()) {
          const name = widget.getName();
          if (!['name', 'agree', 'choice', 'size'].includes(name)) continue;
          const ap = widget.getObject().get('AP');
          assert.ok(ap && !ap.isNull(), `${name} ${widget.getObject().asIndirect()} has no /AP`);
        }
        const field = page.getWidgets().find(widget => widget.getName() === 'name');
        assert.equal(field.getValue(), 'PDF4All P2 form');
      } finally {
        page.destroy();
      }
    } finally {
      doc.destroy();
    }
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('debt1 row 6: a bad page is refused with reason invalid and the same message', async () => {
  const session = await open('form');
  try {
    for (const page of [-1, 99, 0.5, '0']) {
      const reply = op(session, { op: 'fields', page });
      assert.equal(reply.ok, false);
      assert.equal(reply.reason, 'invalid', JSON.stringify(page));
      assert.match(reply.error, /^fields: page must be an integer in 0\.\.\d+$/);
    }
  } finally {
    engine.closeDocument(session.doc);
  }
});
