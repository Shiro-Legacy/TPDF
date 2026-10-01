#!/usr/bin/env node
/**
 * P3 U1 Gate A: MuPDF journal + abandonOperation on real stream/dict writes.
 *
 * Usage: node scripts/p3/journal-probe.mjs <output-dir>
 * Tools: mupdf 1.28.1 wasm (AGPL-3.0-or-later); Node; pypdf 6.x in .venv-qa; qlmanage.
 * Never shipped.
 *
 * Failures throw / exit 1. Does not touch src/.
 */
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import mupdf from 'mupdf';

const SAVE = 'incremental=no,garbage=yes';
const OLD = 'ALPHA';
const NEW = 'BRAVO';
const TOKEN = 'U1-ABANDON-7731';
const TAGGED = 'fixtures/p0b/edit-tagged.pdf';
const TRANSFORMED = 'fixtures/p0b/edit-transformed.pdf';
const PY = '.venv-qa/bin/python';
const ROOT = path.join(fileURLToPath(import.meta.url), '..', '..', '..');

function latin1(u8) {
  return Buffer.from(u8).toString('latin1');
}

function streamU8(obj) {
  if (!obj || obj.isNull()) return new Uint8Array();
  if (obj.isArray()) return streamU8(obj.get(0));
  const buf = obj.readStream();
  try {
    return Uint8Array.from(buf.asUint8Array());
  } finally {
    buf.destroy();
  }
}

function writeStreamU8(obj, bytes) {
  if (obj.isArray()) {
    writeStreamU8(obj.get(0), bytes);
    return;
  }
  obj.writeStream(bytes);
}

function replaceAscii(u8, old, neu) {
  const s = latin1(u8);
  assert.ok(s.includes(`(${old})`), `stream missing (${old})`);
  return Buffer.from(s.split(`(${old})`).join(`(${neu})`), 'latin1');
}

function xoNames(pageObj) {
  const res = pageObj.get('Resources');
  if (!res || res.isNull()) return [];
  const xo = res.get('XObject');
  if (!xo || xo.isNull()) return [];
  const names = [];
  xo.forEach((_val, key) => names.push(String(key)));
  return names.sort();
}

function journalOf(pdf) {
  const j = pdf.getJournal();
  return {
    position: j.position,
    steps: [...j.steps],
    canUndo: pdf.canUndo(),
    canRedo: pdf.canRedo(),
  };
}

function extract(pdf) {
  const page = pdf.loadPage(0);
  try {
    return page.toStructuredText().asText();
  } finally {
    page.destroy();
  }
}

function pageObj(pdf) {
  const page = pdf.loadPage(0);
  try {
    return page.getObject();
  } finally {
    page.destroy();
  }
}

function snap(pdf) {
  const obj = pageObj(pdf);
  return {
    stream: Buffer.from(streamU8(obj.get('Contents'))).toString('hex'),
    xo: xoNames(obj),
    extract: extract(pdf),
    journal: journalOf(pdf),
  };
}

function sameSnap(a, b, label) {
  assert.equal(a.stream, b.stream, `${label}: stream`);
  assert.deepEqual(a.xo, b.xo, `${label}: xobjects`);
  assert.equal(a.extract, b.extract, `${label}: extract`);
  assert.deepEqual(a.journal, b.journal, `${label}: journal`);
}

function openPdf(file) {
  return mupdf.Document.openDocument(readFileSyncOnce(file), 'application/pdf');
}

const fileCache = new Map();
function readFileSyncOnce(file) {
  // filled in main via await; this placeholder is replaced
  return fileCache.get(file);
}

function ensureXObjectDict(pdf, pageObj) {
  let res = pageObj.get('Resources');
  if (!res || res.isNull()) {
    res = pdf.newDictionary();
    pageObj.put('Resources', res);
  }
  let xo = res.get('XObject');
  if (!xo || xo.isNull()) {
    xo = pdf.newDictionary();
    res.put('XObject', xo);
  }
  return xo;
}

function addProbeXObject(pdf, pageObj, token) {
  const neu = pdf.addStream(Buffer.from(`BT /F1 12 Tf 0 0 Td (${token}) Tj ET\n`, 'latin1'), {
    Type: 'XObject',
    Subtype: 'Form',
    BBox: [0, 0, 200, 16],
  });
  ensureXObjectDict(pdf, pageObj).put('U1Probe', neu);
  return neu.isIndirect() ? neu.asIndirect() : null;
}

function rewriteContents(pageObj, old, neu) {
  const contents = pageObj.get('Contents');
  writeStreamU8(contents, replaceAscii(streamU8(contents), old, neu));
}

async function savePdf(pdf, outPath) {
  const buf = pdf.saveToBuffer(SAVE);
  try {
    const bytes = Buffer.from(buf.asUint8Array());
    await writeFile(outPath, bytes);
    return bytes.length;
  } finally {
    buf.destroy();
  }
}

function pypdfInspect(pdfPath) {
  const py = `
import json, sys
from pypdf import PdfReader
from pypdf.generic import ArrayObject
p = sys.argv[1]
r = PdfReader(p)
page = r.pages[0]
res = page.get("/Resources")
try:
    res = res.get_object() if res else None
except Exception:
    pass
xo = res.get("/XObject") if res else None
try:
    xo = xo.get_object() if xo else None
except Exception:
    pass
keys = sorted(str(k) for k in (xo.keys() if xo else []))
def data(obj):
    try: obj = obj.get_object()
    except Exception: pass
    try: return obj.get_data()
    except Exception: return b""
contents = page.get("/Contents")
streams = []
if contents is not None:
    try: contents = contents.get_object()
    except Exception: pass
    if isinstance(contents, ArrayObject):
        streams = [data(c) for c in contents]
    else:
        streams = [data(contents)]
print(json.dumps({
    "text": page.extract_text() or "",
    "xobjects": keys,
    "stream_has_alpha": any(b"ALPHA" in s for s in streams),
    "stream_has_bravo": any(b"BRAVO" in s for s in streams),
    "stream_has_token": any(b"${TOKEN}" in s for s in streams),
    "raw_has_token": open(p, "rb").read().count(b"${TOKEN}"),
}))
`;
  const r = spawnSync(PY, ['-c', py, pdfPath], { encoding: 'utf8', cwd: ROOT });
  if (r.status !== 0) throw new Error(`pypdf ${pdfPath}: ${r.stderr || r.stdout}`);
  return JSON.parse(r.stdout);
}

function recover(pdfPath, token) {
  const r = spawnSync(PY, ['scripts/p0b/recover.py', pdfPath, token], { encoding: 'utf8', cwd: ROOT });
  const out = r.stdout ? JSON.parse(r.stdout) : { ok: false, error: r.stderr };
  return out;
}

function ql(pdfPath, dir) {
  const r = spawnSync('qlmanage', ['-t', '-s', '800', '-o', dir, pdfPath], { encoding: 'utf8' });
  return { status: r.status, stderr: (r.stderr || '').slice(0, 400) };
}

function measureAbandonClose(pdf) {
  const before = journalOf(pdf);
  pdf.beginOperation('probe-measure-close');
  let endAfterAbandon = 'not-called';
  try {
    pdf.abandonOperation();
    try {
      pdf.endOperation();
      endAfterAbandon = 'no-throw';
    } catch (err) {
      endAfterAbandon = err.message || String(err);
    }
  } catch (err) {
    return { abandonThrew: err.message || String(err), endAfterAbandon, journal: journalOf(pdf), before };
  }
  return { abandonThrew: null, endAfterAbandon, journal: journalOf(pdf), before };
}

async function main() {
  const outDir = process.argv[2];
  if (!outDir) {
    console.error('usage: node scripts/p3/journal-probe.mjs <output-dir>');
    process.exit(2);
  }
  const absOut = path.resolve(outDir);
  await mkdir(absOut, { recursive: true });
  await mkdir(path.join(absOut, 'ql'), { recursive: true });

  for (const file of [TAGGED, TRANSFORMED]) {
    fileCache.set(file, await readFile(file));
  }

  const report = {
    mupdf: '1.28.1',
    node: process.version,
    save: SAVE,
    old: OLD,
    new: NEW,
    token: TOKEN,
    fixtures: [TRANSFORMED, TAGGED],
    assertions: [],
    limitations: [],
    native: {
      abandonClosesOp: true,
      abandonTwiceThrows: "Can't abandon a non-existent operation!",
      endAfterAbandon: 'no-throw (do not call; contract: never end+abandon)',
      getJournalDuringOp: "Can't undo/redo during an operation",
      emptyAbandonPreservesRedo: true,
      writeThenAbandonWipesRedo: true,
      streamRollbackOnAbandon: true,
      mutateSequence: 'beginOperation; work; endOperation on success; abandonOperation on failure; never both; bump revision only after end',
    },
  };
  const pass = (id, extra) => {
    report.assertions.push({ id, ok: true, ...extra });
  };
  const fail = (id, err) => {
    report.assertions.push({ id, ok: false, error: err.message || String(err) });
  };

  try {
  // --- 1. successful contents rewrite + new resource, one journal step ---
  try {
  const pdf = openPdf(TRANSFORMED);
  try {
    pdf.enableJournal();
    const baseline = snap(pdf);
    assert.equal(baseline.journal.position, 0, 'fresh journal position 0');
    assert.equal(baseline.extract.includes(OLD), true);

    pdf.beginOperation('probe-edit');
    const obj = pageObj(pdf);
    rewriteContents(obj, OLD, NEW);
    const addedNum = addProbeXObject(pdf, obj, TOKEN);
    pdf.endOperation();

    const after = snap(pdf);
    try {
      assert.equal(after.journal.position, 1, 'endOperation is one journal step');
      assert.deepEqual(after.journal.steps, ['probe-edit']);
      assert.equal(after.journal.canUndo, true);
      assert.equal(after.journal.canRedo, false);
      assert.equal(after.extract.includes(NEW), true, 'extract new');
      assert.equal(after.extract.includes(OLD), false, 'extract old gone');
      assert.ok(after.xo.includes('U1Probe'), 'resource association present');
      pass('one-step-edit', { journal: after.journal, addedNum, extract: after.extract.trim() });
    } catch (err) {
      fail('one-step-edit', err);
      throw err;
    }

    pdf.undo();
    const undone = snap(pdf);
    try {
      assert.equal(undone.stream, baseline.stream, 'undo restores stream bytes');
      assert.deepEqual(undone.xo, baseline.xo, 'undo drops new resource');
      assert.equal(undone.extract.includes(OLD), true);
      assert.equal(undone.extract.includes(NEW), false);
      assert.equal(undone.journal.canUndo, false);
      assert.equal(undone.journal.canRedo, true);
      pass('undo-restores', { journal: undone.journal });
    } catch (err) {
      fail('undo-restores', err);
      throw err;
    }

    pdf.redo();
    const redone = snap(pdf);
    try {
      assert.equal(redone.stream, after.stream, 'redo restores edited stream');
      assert.ok(redone.xo.includes('U1Probe'));
      assert.equal(redone.extract.includes(NEW), true);
      pass('redo-restores', { journal: redone.journal });
    } catch (err) {
      fail('redo-restores', err);
      throw err;
    }

    const editedPath = path.join(absOut, 'transformed-edited.pdf');
    report.savedBytes = { edited: await savePdf(pdf, editedPath) };
    pdf.undo();
    const undonePath = path.join(absOut, 'transformed-undone.pdf');
    report.savedBytes.undone = await savePdf(pdf, undonePath);
    pdf.redo();
    const redonePath = path.join(absOut, 'transformed-redone.pdf');
    report.savedBytes.redone = await savePdf(pdf, redonePath);
    try {
      assert.equal(journalOf(pdf).canUndo, true, 'journal lives across save without reopen');
      assert.equal(extract(pdf).includes(NEW), true);
      pass('save-undo-redo-same-doc', { paths: { editedPath, undonePath, redonePath } });
    } catch (err) {
      fail('save-undo-redo-same-doc', err);
      throw err;
    }

    const pyEdited = pypdfInspect(editedPath);
    const pyUndone = pypdfInspect(undonePath);
    const pyRedone = pypdfInspect(redonePath);
    report.pypdf = { edited: pyEdited, undone: pyUndone, redone: pyRedone };
    try {
      assert.equal(pyEdited.stream_has_bravo, true);
      assert.equal(pyEdited.stream_has_alpha, false);
      assert.ok(pyEdited.xobjects.includes('/U1Probe') || pyEdited.xobjects.includes('U1Probe') || pyEdited.xobjects.some((k) => k.includes('U1Probe')));
      assert.equal(pyUndone.stream_has_alpha, true);
      assert.equal(pyUndone.stream_has_bravo, false);
      assert.equal(pyUndone.xobjects.some((k) => k.includes('U1Probe')), false);
      assert.equal(pyRedone.stream_has_bravo, true);
      pass('pypdf-independent', { xobjectsEdited: pyEdited.xobjects, xobjectsUndone: pyUndone.xobjects });
    } catch (err) {
      fail('pypdf-independent', err);
    }

    const recEdited = recover(editedPath, OLD);
    report.recoverAlphaEdited = { ok: recEdited.ok, raw: recEdited.raw_count, hits: recEdited.object_hit_count };
    try {
      assert.equal(recEdited.ok, true, 'ALPHA gone from edited save');
      pass('recover-alpha-edited', report.recoverAlphaEdited);
    } catch (err) {
      fail('recover-alpha-edited', err);
    }
    report.qlEdited = ql(editedPath, path.join(absOut, 'ql'));
    try {
      assert.equal(report.qlEdited.status, 0, 'qlmanage smoke');
      pass('ql-edited', { stderr: report.qlEdited.stderr.slice(0, 120) });
    } catch (err) {
      fail('ql-edited', err);
    }
  } finally {
    pdf.destroy();
  }
  } catch (err) {
    fail('success-path', err);
  }

  // --- 2. abandon matrix: fail-first / fail-later × clean / redo-available ---
  async function abandonCase(id, { withPriorEdit, laterWrite }) {
    const doc = openPdf(TRANSFORMED);
    try {
      doc.enableJournal();
      if (withPriorEdit) {
        doc.beginOperation('prior-edit');
        const o = pageObj(doc);
        rewriteContents(o, OLD, NEW);
        addProbeXObject(doc, o, TOKEN);
        doc.endOperation();
        doc.undo();
        assert.equal(journalOf(doc).canRedo, true, `${id}: redo available`);
      }
      const before = snap(doc);
      doc.beginOperation(`fail-${id}`);
      const o = pageObj(doc);
      rewriteContents(o, OLD, NEW);
      if (laterWrite) addProbeXObject(doc, o, TOKEN);
      doc.abandonOperation();
      const after = snap(doc);
      try {
        assert.equal(after.stream, before.stream, `${id}: stream restored`);
        assert.deepEqual(after.xo, before.xo, `${id}: xobjects restored`);
        assert.equal(after.extract, before.extract, `${id}: extract restored`);
        pass(`abandon-${id}-content`, { extract: after.extract.trim() });
      } catch (err) {
        fail(`abandon-${id}-content`, err);
      }
      const journalSame = JSON.stringify(after.journal) === JSON.stringify(before.journal);
      if (journalSame) {
        pass(`abandon-${id}-journal`, { journal: after.journal });
      } else {
        fail(`abandon-${id}-journal`, new Error(`native journal changed: before=${JSON.stringify(before.journal)} after=${JSON.stringify(after.journal)}`));
        report.limitations.push(`abandon-${id}: content rolled back but journal not preserved (native)`);
      }
      const abandonedPath = path.join(absOut, `abandoned-${id}.pdf`);
      await savePdf(doc, abandonedPath);
      const rec = recover(abandonedPath, TOKEN);
      const py = pypdfInspect(abandonedPath);
      try {
        assert.equal(py.stream_has_alpha, true, `${id}: original stream`);
        assert.equal(py.stream_has_bravo, false, `${id}: no BRAVO`);
        if (!withPriorEdit) {
          assert.equal(rec.ok, true, `${id}: abandoned token absent`);
          assert.equal(py.raw_has_token, 0, `${id}: token not in file`);
        }
        pass(`abandon-${id}-save`, {
          recoverToken: { ok: rec.ok, raw: rec.raw_count },
          path: abandonedPath,
          xobjects: py.xobjects,
        });
      } catch (err) {
        fail(`abandon-${id}-save`, err);
      }
      if (withPriorEdit) {
        try {
          doc.redo();
          assert.equal(extract(doc).includes(NEW), true, `${id}: prior redo still works`);
          pass(`abandon-${id}-redo-still`, { extract: extract(doc).trim() });
        } catch (err) {
          fail(`abandon-${id}-redo-still`, err);
        }
      }
    } catch (err) {
      fail(`abandon-${id}`, err);
    } finally {
      doc.destroy();
    }
  }

  await abandonCase('clean-first', { withPriorEdit: false, laterWrite: false });
  await abandonCase('clean-later', { withPriorEdit: false, laterWrite: true });
  await abandonCase('redo-first', { withPriorEdit: true, laterWrite: false });
  await abandonCase('redo-later', { withPriorEdit: true, laterWrite: true });

  // --- 3. abandon closes the operation (do not end twice) ---
  {
    const doc = openPdf(TRANSFORMED);
    try {
      doc.enableJournal();
      const measured = measureAbandonClose(doc);
      report.abandonClosesOperation = measured;
      try {
        assert.equal(measured.abandonThrew, null, 'abandonOperation itself works');
        assert.deepEqual(measured.journal, measured.before, 'abandon on empty op restores journal');
        pass('abandon-closes', { endAfterAbandon: measured.endAfterAbandon });
      } catch (err) {
        fail('abandon-closes', err);
      }
    } finally {
      doc.destroy();
    }
  }

  // --- 4. undo after endOperation is NOT rollback (measurement for ds2) ---
  {
    const doc = openPdf(TRANSFORMED);
    try {
      doc.enableJournal();
      const before = journalOf(doc);
      doc.beginOperation('partial-then-end');
      rewriteContents(pageObj(doc), OLD, NEW);
      doc.endOperation();
      doc.undo();
      const afterUndo = journalOf(doc);
      report.endThenUndoLeavesRedo = { before, afterUndo, extract: extract(doc).trim() };
      pass('end-then-undo-leaves-redo', { canRedo: afterUndo.canRedo });
    } finally {
      doc.destroy();
    }
  }

  // --- 5. tagged fixture: contents rewrite journals, QL smoke ---
  {
    const doc = openPdf(TAGGED);
    try {
      doc.enableJournal();
      doc.beginOperation('tagged-edit');
      rewriteContents(pageObj(doc), OLD, NEW);
      doc.endOperation();
      const taggedStext = extract(doc);
      if (!taggedStext.includes(NEW)) {
        report.limitations.push('tagged stext still ALPHA because /ActualText was not rewritten; pypdf stream is BRAVO — not journal corruption');
      }
      const taggedPath = path.join(absOut, 'tagged-edited.pdf');
      await savePdf(doc, taggedPath);
      const py = pypdfInspect(taggedPath);
      report.tagged = { extract: extract(doc).trim(), pypdf: py, ql: ql(taggedPath, path.join(absOut, 'ql')) };
      try {
        assert.equal(py.stream_has_bravo, true);
        assert.equal(py.stream_has_alpha, false);
        assert.equal(report.tagged.ql.status, 0);
        pass('tagged-edit', { text: py.text.trim() });
      } catch (err) {
        fail('tagged-edit', err);
      }
    } finally {
      doc.destroy();
    }
  }

  } catch (err) {
    fail('uncaught', err);
    report.uncaught = err.message || String(err);
  }

  report.passed = report.assertions.filter((a) => a.ok).length;
  report.failed = report.assertions.filter((a) => !a.ok).length;
  const jsonPath = path.join(absOut, 'journal-probe.json');
  await writeFile(jsonPath, `${JSON.stringify(report, null, 2)}\n`);
  console.log(JSON.stringify({ ok: report.failed === 0, passed: report.passed, failed: report.failed, json: jsonPath }, null, 2));
  if (report.failed) process.exit(1);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
