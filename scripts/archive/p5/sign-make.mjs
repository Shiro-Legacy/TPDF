#!/usr/bin/env node
/**
 * P5 U0: mupdf incremental /Sig placeholder + node:crypto CMS patch.
 * Usage: node scripts/p5/sign-make.mjs
 * Does not touch src/. Outputs under .quad/shared/p5/checks/.
 */
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import mupdf from 'mupdf';
import { cmsDetached, pemDer } from './cms.mjs';

const root = path.resolve(path.dirname(new URL(import.meta.url).pathname), '../..');
const ca = path.join(root, 'fixtures/p0b/.ca');
const outDir = path.join(root, '.quad/shared/p5/checks');
const CONTENTS_BYTES = 4096;
const RANGE = '/ByteRange [0000000000 0000000000 0000000000 0000000000]';
const leafDer = pemDer(readFileSync(path.join(ca, 'leaf.pem')));
const caDer = pemDer(readFileSync(path.join(ca, 'ca.pem')));
const keyPem = readFileSync(path.join(ca, 'leaf.key'));

function pdfDate(date) {
  const p = (n) => String(n).padStart(2, '0');
  return `D:${date.getUTCFullYear()}${p(date.getUTCMonth() + 1)}${p(date.getUTCDate())}`
    + `${p(date.getUTCHours())}${p(date.getUTCMinutes())}${p(date.getUTCSeconds())}Z`;
}

function day(date) {
  const p = (n) => String(n).padStart(2, '0');
  return `${date.getUTCFullYear()}-${p(date.getUTCMonth() + 1)}-${p(date.getUTCDate())}`;
}

function placeholder(src, opts) {
  const doc = mupdf.Document.openDocument(src, 'application/pdf').asPDF();
  const page = doc.loadPage(0);
  const annot = page.createAnnotation('Widget');
  const widget = annot.getObject();
  widget.put('FT', 'Sig');
  widget.put('T', doc.newString(opts.field));
  widget.put('F', 4);
  annot.setRect(opts.rect);
  const font = doc.addObject({
    Type: 'Font', Subtype: 'Type1', BaseFont: 'Helvetica', Encoding: 'WinAnsiEncoding',
  });
  const w = opts.rect[2] - opts.rect[0];
  const h = opts.rect[3] - opts.rect[1];
  const label = `Signed by ${opts.name} on ${opts.day}`.replace(/[()\\]/g, '\\$&');
  const stream = `q 0.93 0.95 1 rg 0 0 ${w} ${h} re f 0.1 0.2 0.5 RG 1 w `
    + `0.5 0.5 ${w - 1} ${h - 1} re S BT /Helv 10 Tf 0 0 0.15 rg 8 ${Math.round(h / 2 - 4)} Td (${label}) Tj ET Q\n`;
  const ap = doc.addStream(Buffer.from(stream), {
    Type: 'XObject', Subtype: 'Form', BBox: [0, 0, w, h],
    Resources: { Font: { Helv: font } },
  });
  widget.put('AP', { N: ap });
  const sig = {
    Type: 'Sig',
    Filter: 'Adobe.PPKLite',
    SubFilter: 'adbe.pkcs7.detached',
    Contents: doc.newByteString(new Uint8Array(CONTENTS_BYTES)),
    ByteRange: [0, 0, 0, 0],
    Name: doc.newString(opts.name),
    Reason: doc.newString(opts.reason),
    M: doc.newString(opts.pdfDate),
  };
  if (opts.certify) {
    sig.Reference = [{
      Type: 'SigRef',
      TransformMethod: 'DocMDP',
      TransformParams: { Type: 'TransformParams', P: 2, V: '1.2' },
    }];
  }
  const sigObj = doc.addObject(sig);
  widget.put('V', sigObj);
  const rootObj = doc.getTrailer().get('Root');
  const acro = rootObj.get('AcroForm');
  if (acro.isNull()) {
    const fields = doc.newArray();
    fields.push(widget);
    rootObj.put('AcroForm', doc.addObject({ Fields: fields, SigFlags: 3 }));
  } else {
    const fields = acro.get('Fields');
    fields.push(widget);
    acro.put('SigFlags', 3);
  }
  if (opts.certify) rootObj.put('Perms', { DocMDP: sigObj });
  const saved = Buffer.from(doc.saveToBuffer('incremental').asUint8Array());
  page.destroy();
  doc.destroy();
  return saved;
}

function fixOffsets(buf, insertAt, delta) {
  if (!delta) return buf;
  const out = Buffer.from(buf);
  const xrefAt = out.lastIndexOf('\nxref\n');
  const trailerAt = out.lastIndexOf('\ntrailer');
  if (xrefAt < 0 || trailerAt < xrefAt) throw new Error('incremental xref missing');
  let cursor = out.indexOf(0x0a, xrefAt + 1) + 1;
  while (cursor < trailerAt) {
    const eol = out.indexOf(0x0a, cursor);
    const line = out.toString('latin1', cursor, eol);
    if (/^\d{10} \d{5} [nf] $/.test(line)) {
      const off = Number(line.slice(0, 10));
      if (off > insertAt) out.write(String(off + delta).padStart(10, '0'), cursor, 10, 'latin1');
    }
    cursor = eol + 1;
  }
  const mark = 'startxref\n';
  const at = out.lastIndexOf(mark);
  const start = at + mark.length;
  const end = out.indexOf(0x0a, start);
  const next = String(Number(out.toString('latin1', start, end)) + delta);
  if (next.length === end - start) {
    out.write(next, start, next.length, 'latin1');
    return out;
  }
  return Buffer.concat([out.subarray(0, start), Buffer.from(next), out.subarray(end)]);
}

function widen(buf) {
  const text = buf.toString('latin1');
  const match = /\/ByteRange\s*\[\s*0\s+0\s+0\s+0\s*\]/.exec(text);
  if (!match) throw new Error('no /ByteRange [0 0 0 0]');
  const next = Buffer.concat([
    buf.subarray(0, match.index),
    Buffer.from(RANGE),
    buf.subarray(match.index + match[0].length),
  ]);
  return fixOffsets(next, match.index, RANGE.length - match[0].length);
}

function patch(buf, signingTime) {
  const widened = widen(buf);
  const text = widened.toString('latin1');
  const contents = /\/Contents<([0-9A-Fa-f]+)>/g;
  let hit = null;
  for (let match = contents.exec(text); match; match = contents.exec(text)) {
    if (/^0+$/.test(match[1])) hit = match;
  }
  if (!hit) throw new Error('no zero /Contents placeholder');
  const open = hit.index + '/Contents'.length;
  const close = hit.index + hit[0].length - 1;
  if (widened[open] !== 0x3c || widened[close] !== 0x3e) throw new Error('Contents brackets moved');
  const rangeAt = text.lastIndexOf(RANGE);
  if (rangeAt < 0) throw new Error('fixed ByteRange missing');
  const b = open;
  const c = close + 1;
  const numbers = [0, b, c, widened.length - c].map((n) => String(n).padStart(10, '0')).join(' ');
  const ranged = Buffer.from(widened);
  ranged.write(numbers, rangeAt + '/ByteRange ['.length, numbers.length, 'latin1');
  const signed = Buffer.concat([ranged.subarray(0, b), ranged.subarray(c)]);
  const cms = cmsDetached({ content: signed, leafDer, caDer, keyPem, signingTime });
  const hex = cms.toString('hex').toUpperCase();
  if (hex.length > hit[1].length) throw new Error(`CMS ${cms.length} exceeds placeholder ${hit[1].length / 2}`);
  const patched = Buffer.from(ranged);
  patched.write(hex.padEnd(hit[1].length, '0'), open + 1, hit[1].length, 'latin1');
  return patched;
}

function sign(src, opts) {
  const when = opts.signingTime;
  const drafted = placeholder(src, {
    ...opts,
    day: day(when),
    pdfDate: pdfDate(when),
    name: 'P0B Signer',
    reason: 'p5-probe',
  });
  if (!drafted.subarray(0, src.length).equals(src)) throw new Error('incremental save rewrote the prefix');
  const signed = patch(drafted, when);
  if (!signed.subarray(0, src.length).equals(src)) throw new Error('patch rewrote the prefix');
  return signed;
}

function render(pdf, name) {
  const doc = mupdf.Document.openDocument(pdf, 'application/pdf');
  const page = doc.loadPage(0);
  const pix = page.toPixmap(mupdf.Matrix.scale(2, 2), mupdf.ColorSpace.DeviceRGB, false);
  const file = path.join(outDir, name);
  writeFileSync(file, pix.asPNG());
  page.destroy();
  doc.destroy();
  return file;
}

function engineSave(pdf) {
  return import('../../build/renderer/engine.js').then(({ createEngine }) => {
    const engine = createEngine(mupdf);
    const session = engine.createSession();
    const bytes = pdf.buffer.slice(pdf.byteOffset, pdf.byteOffset + pdf.byteLength);
    const opened = engine.dispatch({ id: 1, op: 'open', name: 'signed.pdf', pdf: bytes }, session);
    const saved = engine.dispatch({ id: 2, op: 'save' }, session);
    engine.closeDocument(session.doc);
    return {
      open: opened.reply.ok,
      signatures: opened.reply.info?.signatures,
      certified: opened.reply.info?.certified,
      saveOk: saved.reply.ok,
      reason: saved.reply.reason,
    };
  });
}

const base = readFileSync(path.join(root, 'fixtures/edit-tj.pdf'));
const when = new Date('2026-09-23T16:00:00Z');
mkdirSync(outDir, { recursive: true });
const once = sign(base, {
  field: 'Sig1', certify: false, signingTime: when, rect: [72, 620, 340, 680],
});
const twice = sign(once, {
  field: 'Sig2', certify: false, signingTime: new Date('2026-09-23T16:05:00Z'), rect: [72, 540, 340, 600],
});
const certified = sign(base, {
  field: 'Sig1', certify: true, signingTime: when, rect: [72, 620, 340, 680],
});
writeFileSync(path.join(outDir, 'signed-once.pdf'), once);
writeFileSync(path.join(outDir, 'signed-twice.pdf'), twice);
writeFileSync(path.join(outDir, 'certified-p2.pdf'), certified);
const png = render(once, 'signed-once-mupdf.png');
const save = await engineSave(once);
const certSave = await engineSave(certified);
console.log(JSON.stringify({
  once: once.length,
  twice: twice.length,
  certified: certified.length,
  prefixOnce: once.subarray(0, base.length).equals(base),
  prefixTwice: twice.subarray(0, once.length).equals(once),
  sha256: createHash('sha256').update(once).digest('hex'),
  png,
  save,
  certSave,
}, null, 2));
