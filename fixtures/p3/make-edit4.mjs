#!/usr/bin/env node
/** P3 U4 image/form placement fixture. Run: node fixtures/p3/make-edit4.mjs */
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { deflateSync } from 'node:zlib';

const DIR = import.meta.dirname;
const GEN = 'fixtures/p3/make-edit4.mjs';
const SOURCE = 'fixtures/img-gray.png';

function pngData(file) {
  const png = readFileSync(file);
  if (png.subarray(0, 8).toString('hex') !== '89504e470d0a1a0a') throw new Error('not a PNG');
  const idat = [];
  let width, height, depth, color, interlace;
  for (let at = 8; at < png.length;) {
    const length = png.readUInt32BE(at);
    const type = png.subarray(at + 4, at + 8).toString('ascii');
    const data = png.subarray(at + 8, at + 8 + length);
    if (type === 'IHDR') {
      width = data.readUInt32BE(0); height = data.readUInt32BE(4);
      depth = data[8]; color = data[9]; interlace = data[12];
    } else if (type === 'IDAT') idat.push(data);
    at += length + 12;
  }
  if (width !== 200 || height !== 300 || depth !== 8 || color !== 0 || interlace !== 0 || !idat.length) {
    throw new Error('img-gray.png must be non-interlaced 200x300 8-bit grayscale');
  }
  return { width, height, compressed: Buffer.concat(idat), source: png };
}

function stream(data, extra = '') {
  const buf = Buffer.isBuffer(data) ? data : Buffer.from(data);
  return Buffer.concat([Buffer.from(`<< /Length ${buf.length}${extra} >>\nstream\n`), buf, Buffer.from('\nendstream')]);
}

function pdfBytes(objects) {
  const chunks = [Buffer.from('%PDF-1.4\n%\xE2\xE3\xCF\xD3\n', 'latin1')];
  const offsets = [];
  let pos = chunks[0].length;
  for (const [i, body] of objects.entries()) {
    offsets.push(pos);
    const parts = [Buffer.from(`${i + 1} 0 obj\n`), Buffer.isBuffer(body) ? body : Buffer.from(body), Buffer.from('\nendobj\n')];
    chunks.push(...parts);
    pos += parts.reduce((n, part) => n + part.length, 0);
  }
  let xref = `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const offset of offsets) xref += `${String(offset).padStart(10, '0')} 00000 n \n`;
  xref += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${pos}\n%%EOF\n`;
  chunks.push(Buffer.from(xref));
  return Buffer.concat(chunks);
}

const image = pngData(path.join(DIR, '..', 'img-gray.png'));
const imageParams = ` /Type /XObject /Subtype /Image /Width ${image.width} /Height ${image.height} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode /DecodeParms << /Predictor 15 /Colors 1 /BitsPerComponent 8 /Columns ${image.width} >>`;
const page0 = `q 64 0 0 96 72 650 cm /Im1 Do Q
q 10 10 100 100 re W n 64 0 0 64 72 400 cm /Im1 Do Q
q .75 0 0 .75 0 0 cm
q 64 0 0 64 72 300 cm /Im1 Do Q Q
q 64 0 0 96 180 650 cm /Im2 Do Q
q 1 0 0 1 72 320 cm /Fm1 Do Q
q 0 1 -1 0 300 300 cm /Fm1 Do Q
q 1 0 0 1 72 200 cm /Fm2 Do Q
BT /F1 12 Tf 72 140 Td (NEIGHBOR) Tj ET
`;
const page1 = `q 64 0 0 96 72 650 cm /Im1 Do Q
q 1 0 0 1 72 500 cm /Fm1 Do Q
`;
const resources = '<< /Font << /F1 8 0 R >> /XObject << /Im1 9 0 R /Im2 10 0 R /Fm1 11 0 R /Fm2 13 0 R >> >>';
const data = pdfBytes([
  '<< /Type /Catalog /Pages 2 0 R >>',
  '<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /Resources 5 0 R >>',
  '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources 5 0 R /Contents 6 0 R >>',
  '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 7 0 R >>',
  resources,
  stream(page0),
  stream(page1),
  '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>',
  stream(image.compressed, imageParams),
  stream(image.compressed, `${imageParams} /SMask 12 0 R`),
  stream('BT /F1 12 Tf 0 4 Td (FORM) Tj ET\n', ' /Type /XObject /Subtype /Form /BBox [0 0 120 20] /Resources << /Font << /F1 8 0 R >> >>'),
  stream(deflateSync(Buffer.alloc(image.width * image.height, 255)), ` /Type /XObject /Subtype /Image /Width ${image.width} /Height ${image.height} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode`),
  stream('BT /F1 12 Tf 0 4 Td (FORM2) Tj ET\n', ' /Type /XObject /Subtype /Form /BBox [0 0 120 40] /Matrix [1 0 0 1 10 10] /Resources << /Font << /F1 8 0 R >> >>'),
]);
const file = path.join(DIR, 'edit-objects.pdf');
writeFileSync(file, data);

const provenance = {
  id: 'edit-objects',
  file: 'fixtures/p3/edit-objects.pdf',
  bytes: data.length,
  sha256: createHash('sha256').update(data).digest('hex'),
  pages: 2,
  page_size_pt: [612, 792],
  notes: [
    'p0 Im1 simple: candidate; objectOps move/resize/delete/replace; no refusal reason',
    'p0 Im1 clipped: candidate; objectOps replace; unsupported-transform for move/resize/delete',
    'p0 Im1 canonical block under outer scale CTM: candidate; objectOps replace; unsupported-transform for move/resize/delete',
    'p0 Im2 simple with SMask: candidate; objectOps move/resize/delete; unsupported-image for replace',
    'p0 Fm1 simple: candidate; objectOps move/resize/delete; no refusal reason',
    'p0 Fm1 rotated: unsupported; objectOps empty; unsupported-transform',
    'p0 Fm2 simple with Matrix [1 0 0 1 10 10] and BBox [0 0 120 40]: candidate; box [82,210,202,250]; objectOps move/resize/delete; no refusal reason',
    'p1 Im1 simple, shared object via inherited /Resources: candidate; objectOps move/resize/delete/replace; no refusal reason',
    'p1 Fm1 simple, shared object via inherited /Resources: candidate; objectOps move/resize/delete; no refusal reason',
  ],
  generator: {
    path: GEN,
    sha256: createHash('sha256').update(readFileSync(new URL(import.meta.url))).digest('hex'),
    deterministic: true,
    timestamps_in_output: false,
    dependencies: ['node stdlib'],
    reproduce: 'node fixtures/p3/make-edit4.mjs',
  },
  provenance: {
    authorship: 'placement streams written by the project team for P3 U4; image is the project synthetic fixture',
    third_party_content: 'none',
    source_assets: [{ file: SOURCE, sha256: createHash('sha256').update(image.source).digest('hex') }],
    fonts: 'Base-14 Helvetica, referenced not embedded',
    network_access: 'none',
  },
  license: "fixture content is MIT (LICENSE at the repo root)",
  generated_on: '2026-09-17',
};
writeFileSync(path.join(DIR, 'edit-objects.provenance.json'), `${JSON.stringify(provenance, null, 2)}\n`);
console.log(`wrote fixtures/p3/edit-objects.pdf (${data.length} bytes)`);
