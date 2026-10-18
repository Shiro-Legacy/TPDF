// Hostile images on the Rust engine's WASM build (32-bit): sizes whose
// products wrap a 32-bit usize, and decoders that allocate from declared
// sizes, end in a page or a render error, never in a stopped engine. The
// engine's native tests (engine/tests/view_robust.rs) cannot see 32-bit
// overflow. Runs under npm run test:rust (TPDF_ENGINE=rust).
import test from 'node:test';
import assert from 'node:assert/strict';
import { deflateSync } from 'node:zlib';
import mupdf from 'mupdf';
import { createEngine } from './rust-engine.mjs';

const skip = process.env.TPDF_ENGINE === 'rust' ? false : 'runs under npm run test:rust';

/** A classic-xref PDF from object bodies (strings or Buffers) numbered from 1. */
function pdf(objects) {
  const parts = [Buffer.from('%PDF-1.7\n', 'latin1')];
  let length = parts[0].length;
  const offsets = [];
  objects.forEach((body, index) => {
    offsets.push(length);
    const chunk = Buffer.concat([
      Buffer.from(`${index + 1} 0 obj\n`, 'latin1'),
      Buffer.isBuffer(body) ? body : Buffer.from(body, 'latin1'),
      Buffer.from('\nendobj\n', 'latin1'),
    ]);
    parts.push(chunk);
    length += chunk.length;
  });
  const size = objects.length + 1;
  const xref = [`xref\n0 ${size}\n0000000000 65535 f \n`, ...offsets.map(o => `${String(o).padStart(10, '0')} 00000 n \n`)].join('');
  parts.push(Buffer.from(`${xref}trailer\n<< /Size ${size} /Root 1 0 R >>\nstartxref\n${length}\n%%EOF\n`, 'latin1'));
  const bytes = Buffer.concat(parts);
  return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.length);
}

function stream(dict, data) {
  return Buffer.concat([
    Buffer.from(`<< ${dict} /Length ${data.length} >>\nstream\n`, 'latin1'),
    data,
    Buffer.from('\nendstream', 'latin1'),
  ]);
}

/** A 100 × 100 page drawing image object 5 (`dict`, `data`) with `cm`. */
function imagePage(dict, data, cm) {
  return pdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>',
    stream('', Buffer.from(`q ${cm} cm /Im0 Do Q`, 'latin1')),
    stream(`/Type /XObject /Subtype /Image ${dict}`, data),
  ]);
}

/**
 * A 16 × 16 grey JPEG ramp (made for this test with cjpeg), its frame
 * header patched to claim `width` × `height`.
 */
function jpegClaiming(width, height) {
  const jpeg = Buffer.from(
    'ffd8ffe000104a46494600010100000100010000ffdb0043000302020302020303030304030304050805050404050a07070608'
    + '0c0a0c0c0b0a0b0b0d0e12100d0e110e0b0b1016101113141515150c0f171816141812141514ffc0000b08001000100101110'
    + '0ffc4001f0000010501010101010100000000000000000102030405060708090a0bffc400b51000020103030204030505040400'
    + '00017d01020300041105122131410613516107227114328191a1082342b1c11552d1f02433627282090a161718191a25262728'
    + '292a3435363738393a434445464748494a535455565758595a636465666768696a737475767778797a838485868788898a9293'
    + '9495969798999aa2a3a4a5a6a7a8a9aab2b3b4b5b6b7b8b9bac2c3c4c5c6c7c8c9cad2d3d4d5d6d7d8d9dae1e2e3e4e5e6e7e8'
    + 'e9eaf1f2f3f4f5f6f7f8f9faffda0008010100003f00f8ff00f66dff00975fc2bf557f66dff975fc2bf2abf66dff00975fc2bf55'
    + '7f66dff975fc2bffd9',
    'hex',
  );
  const sof = 89;
  jpeg.writeUInt16BE(height, sof + 5);
  jpeg.writeUInt16BE(width, sof + 7);
  return jpeg;
}

/** An embedded JBIG2 page of `width` × `height` (default pixel black), no regions. */
function jbig2Page(width, height) {
  const info = Buffer.alloc(19);
  info.writeUInt32BE(width, 0);
  info.writeUInt32BE(height, 4);
  info[16] = 0x04;
  return Buffer.concat([
    Buffer.from([0, 0, 0, 0, 0x30, 0, 1, 0, 0, 0, 19]),
    info,
    Buffer.from([0, 0, 0, 1, 0x31, 0, 1, 0, 0, 0, 0]),
  ]);
}

const CASES = [
  // 16 × 2^30 8-bit samples with one row of data: the padded length wrapped
  // to 0 on 32-bit while the height stayed, and the box filter divided by it.
  ['a tall image with one row', imagePage(
    '/Width 16 /Height 1073741824 /BitsPerComponent 8 /ColorSpace /DeviceGray /Filter /FlateDecode',
    deflateSync(Buffer.alloc(16)), '16 0 0 1 0 10',
  )],
  // 65,536 × 65,536 samples, one row of data, drawn at 8,192 units.
  ['a 2^32-sample image with one row', imagePage(
    '/Width 65536 /Height 65536 /BitsPerComponent 8 /ColorSpace /DeviceGray /Filter /FlateDecode',
    deflateSync(Buffer.alloc(65536)), '8192 0 0 8192 0 0',
  )],
  // zune-jpeg's output size overflowed (65,535² × 3) and unwrapped None.
  ['a JPEG claiming 65,535 × 65,535', imagePage(
    '/Width 65535 /Height 65535 /BitsPerComponent 8 /ColorSpace /DeviceRGB /Filter /DCTDecode',
    jpegClaiming(65535, 65535), '100 0 0 100 0 0',
  )],
  // hayro-syntax reserved Columns × Rows bytes before decoding.
  ['CCITT data declaring 65,535 × 65,535', imagePage(
    '/Width 65535 /Height 65535 /BitsPerComponent 1 /ColorSpace /DeviceGray /Filter /CCITTFaxDecode /DecodeParms << /K -1 /Columns 65535 /Rows 65535 >>',
    Buffer.alloc(6, 0xff), '100 0 0 100 0 0',
  )],
  // hayro-syntax reserved the page's width × height bytes before decoding.
  ['a JBIG2 page of 100,000 × 100,000', imagePage(
    '/Width 100000 /Height 100000 /BitsPerComponent 1 /ColorSpace /DeviceGray /Filter /JBIG2Decode',
    jbig2Page(100000, 100000), '100 0 0 100 0 0',
  )],
];

for (const [name, bytes] of CASES) {
  test(`rust engine: ${name} renders or is refused, and the engine keeps running`, { skip }, () => {
    const engine = createEngine(mupdf);
    const session = engine.createSession();
    const open = engine.dispatch({ id: 1, op: 'open', name: 'x.pdf', pdf: bytes }, session).reply;
    assert.equal(open.ok, true, JSON.stringify(open));
    for (const [id, scale] of [[2, 1], [3, 0.25]]) {
      const reply = engine.dispatch({ id, op: 'render', page: 0, scale }, session).reply;
      assert.notEqual(reply.reason, 'worker', JSON.stringify(reply));
      if (!reply.ok) assert.match(reply.error, /too complex/, JSON.stringify(reply));
    }
    const text = engine.dispatch({ id: 4, op: 'text-all' }, session).reply;
    assert.equal(text.ok, true, JSON.stringify(text));
  });
}
