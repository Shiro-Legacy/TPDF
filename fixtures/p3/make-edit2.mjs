#!/usr/bin/env node
/**
 * P3 U2 fixtures. Run: node fixtures/p3/make-edit2.mjs
 * Helvetica Base-14, no network.
 */
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const DIR = path.dirname(fileURLToPath(import.meta.url));
const FONT = '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>';

function stream(data, extra = '') {
  const buf = Buffer.isBuffer(data) ? data : Buffer.from(data);
  return Buffer.concat([
    Buffer.from(`<< /Length ${buf.length}${extra} >>\nstream\n`),
    buf,
    Buffer.from('endstream'),
  ]);
}

function pdfBytes(objects) {
  const chunks = [Buffer.from('%PDF-1.4\n%\xE2\xE3\xCF\xD3\n')];
  const offsets = [];
  let pos = chunks[0].length;
  for (const [i, body] of objects.entries()) {
    offsets.push(pos);
    const prefix = Buffer.from(`${i + 1} 0 obj\n`);
    const buf = Buffer.isBuffer(body) ? body : Buffer.from(body);
    const suffix = Buffer.from('\nendobj\n');
    chunks.push(prefix, buf, suffix);
    pos += prefix.length + buf.length + suffix.length;
  }
  let xref = `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const off of offsets) xref += `${String(off).padStart(10, '0')} 00000 n \n`;
  xref += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${pos}\n%%EOF\n`;
  chunks.push(Buffer.from(xref));
  return Buffer.concat(chunks);
}

const ops = Buffer.concat([
  Buffer.from(`BT /F1 12 Tf 72 720 Td (ALPHA) Tj ( TAIL) Tj ET
BT /F1 12 Tf 72 680 Td (BETA) Tj 120 0 Td (FAR) Tj ET
/Span << /ActualText (Gamma) >> BDC
BT /F1 12 Tf 72 640 Td (GAMMA) Tj ET
EMC
/OC /oc1 BDC
BT /F1 12 Tf 72 600 Td (DELTA) Tj ET
EMC
q
BI /W 1 /H 1 /CS /DeviceGray /BPC 8 ID
`),
  Buffer.from([0xff]),
  Buffer.from(`
EI
Q
`),
]);

const data = pdfBytes([
  '<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [6 0 R] /D << /Order [6 0 R] /ON [6 0 R] >> >> >>',
  '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
  '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> /Properties << /oc1 6 0 R >> >> /Contents 5 0 R >>',
  FONT,
  stream(ops),
  '<< /Type /OCG /Name (oc1) >>',
]);

writeFileSync(path.join(DIR, 'edit-tail.pdf'), data);
const rec = {
  id: 'edit-tail',
  file: 'fixtures/p3/edit-tail.pdf',
  bytes: data.length,
  sha256: createHash('sha256').update(data).digest('hex'),
  pages: 1,
  page_size_pt: [612, 792],
  expected: {
    line1: { ops: '(ALPHA) Tj ( TAIL) Tj', tailFollows: true, tailWouldMove: 'BETA', equalWidth: 'AHPLA' },
    line2: { ops: '(BETA) Tj 120 0 Td (FAR) Tj', tailFollows: false },
    line3: { actualText: 'Gamma', payload: 'GAMMA', ambiguous: true },
    line4: { oc: '/oc1', payload: 'DELTA', support: 'unsupported-graphics-state' },
    bi: { filter: false, W: 1, H: 1, CS: 'DeviceGray', BPC: 8 },
  },
  note: 'targets-unsupported.pdf BI is filtered (/F /ASCIIHexDecode); unfiltered BI is this file. layers.pdf has OC text (base/notes) — reuse for OC discovery; this file has ALPHA under /OC /oc1.',
  generator: {
    path: 'fixtures/p3/make-edit2.mjs',
    sha256: createHash('sha256').update(readFileSync(new URL(import.meta.url))).digest('hex'),
    deterministic: true,
    timestamps_in_output: false,
    reproduce: 'node fixtures/p3/make-edit2.mjs',
  },
  provenance: {
    authorship: 'written by the project team for P3 U2 (grok); constructed byte-by-byte',
    third_party_content: 'none',
    fonts: 'Base-14 Helvetica, referenced not embedded',
    network_access: 'none',
  },
  license: "fixture content follows the app's final license choice",
  generated_on: '2026-09-16',
};
writeFileSync(path.join(DIR, 'edit-tail.provenance.json'), `${JSON.stringify(rec, null, 2)}\n`);
console.log(`wrote fixtures/p3/edit-tail.pdf (${data.length} bytes)`);
