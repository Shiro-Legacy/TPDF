#!/usr/bin/env node
/**
 * P3 U1b COW probe. Never shipped.
 * Usage: node scripts/p3/cow-probe.mjs <output-dir>
 * mupdf 1.28.1 wasm; save incremental=no,garbage=yes.
 */
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import mupdf from 'mupdf';

const SAVE = 'incremental=no,garbage=yes';
const ROOT = path.join(fileURLToPath(import.meta.url), '..', '..', '..');
const SHARED = 'fixtures/p0b/edit-shared.pdf';
const STREAMS = 'fixtures/p3/targets-streams.pdf';
const PY = '.venv-qa/bin/python';
const TOKEN_OLD = 'ALPHA';
const TOKEN_NEW = 'BRAVO';

function extract(pdf, page) {
  const p = pdf.loadPage(page);
  try { return p.toStructuredText().asText(); }
  finally { p.destroy(); }
}

function journalOf(pdf) {
  const j = pdf.getJournal();
  return { position: j.position, steps: [...j.steps], canUndo: pdf.canUndo(), canRedo: pdf.canRedo() };
}

function streamU8(obj) {
  if (!obj || obj.isNull()) return new Uint8Array();
  if (obj.isArray()) return streamU8(obj.get(0));
  const buf = obj.readStream();
  try { return Uint8Array.from(buf.asUint8Array()); }
  finally { buf.destroy(); }
}

function replaceFirst(hay, needle, neu) {
  const s = Buffer.from(hay).toString('latin1');
  const i = s.indexOf(needle);
  if (i < 0) throw new Error(`missing ${needle}`);
  return Buffer.from(s.slice(0, i) + neu + s.slice(i + needle.length), 'latin1');
}

function copyForm(pdf, src, method) {
  if (method === 'graftObject') {
    const map = pdf.newGraftMap();
    try { return { obj: map.graftObject(src), method: 'graftObject' }; }
    finally { map.destroy?.(); }
  }
  const raw = src.readRawStream();
  const extra = {};
  for (const key of ['Type', 'Subtype', 'BBox', 'Resources', 'Matrix', 'Filter', 'Length']) {
    try {
      const v = src.get(key);
      if (v && !v.isNull()) extra[key] = v;
    } catch { /* */ }
  }
  const obj = pdf.addRawStream(raw, extra);
  return { obj, method: 'addRawStream' };
}

function pageObj(pdf, i) {
  const p = pdf.loadPage(i);
  try { return p.getObject(); }
  finally { p.destroy(); }
}

function xoGet(page, name) {
  const res = page.get('Resources');
  if (!res || res.isNull()) return null;
  const xo = res.get('XObject');
  if (!xo || xo.isNull()) return null;
  const v = xo.get(name);
  return v && !v.isNull() ? v : null;
}

function putXObject(pdf, page, name, obj) {
  let res = page.get('Resources');
  if (!res || res.isNull()) {
    res = pdf.newDictionary();
    page.put('Resources', res);
  }
  let xo = res.get('XObject');
  if (!xo || xo.isNull()) {
    xo = pdf.newDictionary();
    res.put('XObject', xo);
  }
  xo.put(name, obj);
}

function asNum(obj) {
  try { return obj.isIndirect() ? obj.asIndirect() : null; }
  catch { return null; }
}

async function savePdf(pdf, dest) {
  const buf = pdf.saveToBuffer(SAVE);
  try {
    const bytes = Buffer.from(buf.asUint8Array());
    await writeFile(dest, bytes);
    return bytes.length;
  } finally { buf.destroy(); }
}

function pypdfText(file) {
  const py = `
import json, sys
from pypdf import PdfReader
r = PdfReader(sys.argv[1])
print(json.dumps([(p.extract_text() or "") for p in r.pages]))
`;
  const r = spawnSync(PY, ['-c', py, file], { encoding: 'utf8', cwd: ROOT });
  if (r.status !== 0) throw new Error(r.stderr || r.stdout);
  return JSON.parse(r.stdout);
}

function recover(file, token) {
  const r = spawnSync(PY, ['scripts/p0b/recover.py', file, token], { encoding: 'utf8', cwd: ROOT });
  return r.stdout ? JSON.parse(r.stdout) : { ok: false };
}

function ql(file, dir) {
  return spawnSync('qlmanage', ['-t', '-s', '800', '-o', dir, file], { encoding: 'utf8' }).status;
}

function tryMethods(pdf, src) {
  const tried = [];
  for (const method of ['graftObject', 'addRawStream']) {
    try {
      const { obj } = copyForm(pdf, src, method);
      const n = asNum(obj);
      tried.push({ method, ok: true, obj: n });
      return { method, obj, tried };
    } catch (err) {
      tried.push({ method, ok: false, error: err.message });
    }
  }
  throw new Error(`no copy method worked: ${JSON.stringify(tried)}`);
}

async function caseSharedDo(outDir) {
  const pdf = mupdf.Document.openDocument(await readFile(SHARED), 'application/pdf');
  pdf.enableJournal();
  const before = { extract0: extract(pdf, 0), extract1: extract(pdf, 1), journal: journalOf(pdf) };
  const p0 = pageObj(pdf, 0);
  const fm1 = xoGet(p0, 'Fm1');
  const srcNum = asNum(fm1);
  pdf.beginOperation('cow-shared-do');
  const copied = tryMethods(pdf, fm1);
  putXObject(pdf, p0, 'Fm1c', copied.obj);
  const contents = p0.get('Contents');
  writeFirstDo(contents, '/Fm1', '/Fm1c');
  const formBytes = streamU8(copied.obj);
  copied.obj.writeStream(replaceFirst(formBytes, '(ALPHA)', '(BRAVO)'));
  pdf.endOperation();
  const after = { extract0: extract(pdf, 0), extract1: extract(pdf, 1), journal: journalOf(pdf), srcNum, copyNum: asNum(copied.obj), method: copied.method, tried: copied.tried };
  const edited = path.join(outDir, 'cow-shared-edited.pdf');
  await savePdf(pdf, edited);
  pdf.undo();
  const undone = { extract0: extract(pdf, 0), extract1: extract(pdf, 1), journal: journalOf(pdf), hasFm1c: !!xoGet(pageObj(pdf, 0), 'Fm1c') };
  const undonePath = path.join(outDir, 'cow-shared-undone.pdf');
  await savePdf(pdf, undonePath);
  pdf.destroy();
  return { before, after, undone, edited, undonePath, pypdf: pypdfText(edited), recoverOld: recover(edited, TOKEN_OLD), ql: ql(edited, path.join(outDir, 'ql')) };
}

function writeFirstDo(contents, oldName, newName) {
  const bytes = streamU8(contents);
  const next = replaceFirst(bytes, `${oldName} Do`, `${newName} Do`);
  if (contents.isArray()) contents.get(0).writeStream(next);
  else contents.writeStream(next);
}

async function caseInherited(outDir) {
  const pdf = mupdf.Document.openDocument(await readFile(SHARED), 'application/pdf');
  pdf.enableJournal();
  const p0 = pageObj(pdf, 0);
  const pages = pdf.getTrailer().get('Root').get('Pages');
  const res = p0.get('Resources');
  pdf.beginOperation('setup-inherit');
  pages.put('Resources', res);
  p0.delete('Resources');
  pdf.endOperation();
  pdf.beginOperation('cow-inherited');
  const inherited = pages.get('Resources');
  const copyRes = pdf.newDictionary();
  inherited.forEach((v, k) => copyRes.put(k, v));
  p0.put('Resources', copyRes);
  const fm1 = xoGet(p0, 'Fm1');
  const copied = tryMethods(pdf, fm1);
  putXObject(pdf, p0, 'Fm1c', copied.obj);
  writeFirstDo(p0.get('Contents'), '/Fm1', '/Fm1c');
  copied.obj.writeStream(replaceFirst(streamU8(copied.obj), '(ALPHA)', '(BRAVO)'));
  pdf.endOperation();
  const edited = path.join(outDir, 'cow-inherited-edited.pdf');
  await savePdf(pdf, edited);
  const after = { extract0: extract(pdf, 0), extract1: extract(pdf, 1), method: copied.method, copyNum: asNum(copied.obj) };
  pdf.undo();
  const undone = { extract0: extract(pdf, 0), hasOwnRes: !pageObj(pdf, 0).get('Resources').isNull?.() };
  pdf.destroy();
  return { after, undone, edited, pypdf: pypdfText(edited) };
}

function sharedContentsBytes() {
  const content = 'BT /F1 12 Tf 72 700 Td (ALPHA) Tj ET\n';
  const font = '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>';
  const stream = `<< /Length ${content.length} >>\nstream\n${content}endstream`;
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 6 0 R >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 6 0 R >>',
    font,
    stream,
  ];
  const chunks = [Buffer.from('%PDF-1.4\n')];
  const offsets = [];
  let pos = chunks[0].length;
  for (const [i, body] of objects.entries()) {
    offsets.push(pos);
    const block = Buffer.from(`${i + 1} 0 obj\n${body}\nendobj\n`);
    chunks.push(block);
    pos += block.length;
  }
  let xref = `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const off of offsets) xref += `${String(off).padStart(10, '0')} 00000 n \n`;
  xref += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${pos}\n%%EOF\n`;
  chunks.push(Buffer.from(xref));
  return Buffer.concat(chunks);
}

async function caseSharedContents(outDir) {
  const pdf = mupdf.Document.openDocument(sharedContentsBytes(), 'application/pdf');
  pdf.enableJournal();
  const p0 = pageObj(pdf, 0);
  pdf.beginOperation('cow-contents');
  const shared = p0.get('Contents');
  const raw = shared.readRawStream();
  const copy = pdf.addRawStream(raw, {});
  p0.put('Contents', copy);
  copy.writeStream(replaceFirst(streamU8(copy), '(ALPHA)', '(BRAVO)'));
  pdf.endOperation();
  const edited = path.join(outDir, 'cow-contents-edited.pdf');
  await savePdf(pdf, edited);
  const after = { extract0: extract(pdf, 0), extract1: extract(pdf, 1) };
  pdf.undo();
  const undone = { extract0: extract(pdf, 0), extract1: extract(pdf, 1) };
  pdf.destroy();
  return { after, undone, edited, pypdf: pypdfText(edited), method: 'addRawStream contents' };
}

async function caseStreamsDo(outDir) {
  const pdf = mupdf.Document.openDocument(await readFile(STREAMS), 'application/pdf');
  pdf.enableJournal();
  const p0 = pageObj(pdf, 0);
  const fm1 = xoGet(p0, 'Fm1');
  pdf.beginOperation('cow-streams');
  const copied = tryMethods(pdf, fm1);
  putXObject(pdf, p0, 'Fm1c', copied.obj);
  const contents = p0.get('Contents');
  const idx = contents.isArray() ? contents.get(1) : contents;
  idx.writeStream(replaceFirst(streamU8(idx), '/Fm1 Do', '/Fm1c Do'));
  copied.obj.writeStream(replaceFirst(streamU8(copied.obj), '(FORM)', '(XXXX)'));
  pdf.endOperation();
  const edited = path.join(outDir, 'cow-streams-edited.pdf');
  await savePdf(pdf, edited);
  const after = { extract0: extract(pdf, 0), extract1: extract(pdf, 1), method: copied.method };
  pdf.destroy();
  return { after, edited, pypdf: pypdfText(edited) };
}

async function main() {
  const outDir = process.argv[2];
  if (!outDir) {
    console.error('usage: node scripts/p3/cow-probe.mjs <output-dir>');
    process.exit(2);
  }
  const abs = path.resolve(outDir);
  await mkdir(path.join(abs, 'ql'), { recursive: true });
  const report = { mupdf: '1.28.1', save: SAVE, cases: {} };
  const failures = [];
  for (const [name, fn] of [
    ['shared-do', caseSharedDo],
    ['inherited', caseInherited],
    ['shared-contents', caseSharedContents],
    ['streams-do', caseStreamsDo],
  ]) {
    try {
      report.cases[name] = await fn(abs);
    } catch (err) {
      report.cases[name] = { error: err.message };
      failures.push(name);
    }
  }
  const jsonPath = path.join(abs, 'cow-probe.json');
  await writeFile(jsonPath, `${JSON.stringify(report, null, 2)}\n`);
  console.log(JSON.stringify({ ok: failures.length === 0, failures, json: jsonPath }, null, 2));
  if (failures.length) process.exit(1);
}

main().catch((e) => { console.error(e); process.exit(1); });
