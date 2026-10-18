#!/usr/bin/env node
/** P3 U5 page-scoped find/replace fixture. Run: node fixtures/p3/make-edit5.mjs */
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const DIR = import.meta.dirname;
const GEN = 'fixtures/p3/make-edit5.mjs';

function stream(text) {
  const data = Buffer.from(text);
  return Buffer.concat([Buffer.from(`<< /Length ${data.length} >>\nstream\n`), data, Buffer.from('endstream')]);
}

function pdfBytes(objects) {
  const chunks = [Buffer.from('%PDF-1.4\n%\xE2\xE3\xCF\xD3\n', 'latin1')];
  const offsets = [];
  let pos = chunks[0].length;
  for (const [i, body] of objects.entries()) {
    offsets.push(pos);
    const parts = [Buffer.from(`${i + 1} 0 obj\n`), Buffer.isBuffer(body) ? body : Buffer.from(body), Buffer.from('\nendobj\n')];
    chunks.push(...parts);
    pos += parts.reduce((sum, part) => sum + part.length, 0);
  }
  let xref = `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const offset of offsets) xref += `${String(offset).padStart(10, '0')} 00000 n \n`;
  xref += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${pos}\n%%EOF\n`;
  return Buffer.concat([...chunks, Buffer.from(xref)]);
}

const page0 = `BT /F1 12 Tf 72 720 Td (ALPHA one) Tj ET
BT /F1 12 Tf 72 680 Td (ALPHA and ALPHA) Tj ET
BT /F1 12 Tf 72 640 Td (GAMMA ALPHA) Tj ( TAIL) Tj ET
/Span << /ActualText (Alpha) >> BDC
BT /F1 12 Tf 72 600 Td (ALPHA four) Tj ET
EMC
BT /F1 12 Tf 72 560 Td (ALPHABET) Tj ET
BT /F1 12 Tf 72 520 Td (NEIGHBOR) Tj ET
`;
const page1 = 'BT /F1 12 Tf 72 720 Td (ALPHA page two) Tj ET\n';
const data = pdfBytes([
  '<< /Type /Catalog /Pages 2 0 R >>',
  '<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>',
  '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 6 0 R >>',
  '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 7 0 R >>',
  '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>',
  stream(page0),
  stream(page1),
]);
const file = path.join(DIR, 'edit-replace.pdf');
writeFileSync(file, data);

const provenance = {
  id: 'edit-replace',
  file: 'fixtures/p3/edit-replace.pdf',
  bytes: data.length,
  sha256: createHash('sha256').update(data).digest('hex'),
  pages: 2,
  page_size_pt: [612, 792],
  notes: [
    'p0 ALPHA one: ALPHA→BETA matches once; next BETA one; expected replaceable',
    'p0 ALPHA and ALPHA: ALPHA→BETA matches twice; next BETA and BETA; expected replaceable',
    'p0 GAMMA ALPHA + same-line TAIL: matches once; next GAMMA BETA; expected rejected tail-would-move (unique run text so U1 association is not ambiguous)',
    'p0 ActualText Alpha over payload ALPHA four: matches once; next BETA four; expected rejected ambiguous-text-association (tag mismatch)',
    'p0 ALPHABET: substring ALPHA→BETA matches once; next BETABET; expected replaceable',
    'p0 NEIGHBOR: no ALPHA match; omitted from preview',
    'p1 ALPHA page two: matches once; next BETA page two; replaceable only when previewing page 1',
  ],
  generator: {
    path: GEN,
    sha256: createHash('sha256').update(readFileSync(new URL(import.meta.url))).digest('hex'),
    deterministic: true,
    timestamps_in_output: false,
    dependencies: ['node stdlib'],
    reproduce: 'node fixtures/p3/make-edit5.mjs',
  },
  provenance: {
    authorship: 'written by the project team for P3 U5; constructed byte-by-byte',
    third_party_content: 'none',
    fonts: 'Base-14 Helvetica, referenced not embedded',
    network_access: 'none',
  },
  license: "fixture content is MIT (LICENSE at the repo root)",
  generated_on: '2026-09-17',
};
writeFileSync(path.join(DIR, 'edit-replace.provenance.json'), `${JSON.stringify(provenance, null, 2)}\n`);
console.log(`wrote fixtures/p3/edit-replace.pdf (${data.length} bytes)`);
