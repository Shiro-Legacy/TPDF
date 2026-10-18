#!/usr/bin/env node
/**
 * Deterministic P3 U1 target fixtures. No network, no new packages.
 * Run: node fixtures/p3/make-targets.mjs
 */
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { deflateSync } from 'node:zlib';
import { fileURLToPath } from 'node:url';
import mupdf from 'mupdf';

const DIR = path.dirname(fileURLToPath(import.meta.url));
const GEN = 'fixtures/p3/make-targets.mjs';
const LIMIT = 2 * 1024 * 1024 + 1;
const FONT = '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>';
const IMG = Buffer.from(
  Array.from({ length: 8 * 8 }, (_, i) => {
    const x = i % 8;
    return x < 4 ? [255, 0, 0] : [0, 0, 255];
  }).flat(),
);

function stream(data, extra = '') {
  const buf = Buffer.isBuffer(data) ? data : Buffer.from(data);
  return Buffer.concat([
    Buffer.from(`<< /Length ${buf.length}${extra} >>\nstream\n`),
    buf,
    Buffer.from('endstream'),
  ]);
}

function flate(data, extra = '') {
  const raw = Buffer.isBuffer(data) ? data : Buffer.from(data);
  const compressed = deflateSync(raw);
  return { raw, compressed, object: stream(compressed, ` /Filter /FlateDecode${extra}`) };
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

function provenance(id, file, data, extra) {
  const rec = {
    id,
    file: `fixtures/p3/${file}`,
    bytes: data.length,
    sha256: createHash('sha256').update(data).digest('hex'),
    pages: extra.pages,
    page_size_pt: [612, 792],
    ...extra,
    generator: {
      path: GEN,
      sha256: createHash('sha256').update(readFileSync(new URL(import.meta.url))).digest('hex'),
      deterministic: true,
      timestamps_in_output: false,
      dependencies: ['node stdlib', 'mupdf 1.28.1 (limits memory probe only)'],
      reproduce: 'node fixtures/p3/make-targets.mjs',
    },
    provenance: {
      authorship: 'written by the project team for P3 U1 (grok); constructed byte-by-byte',
      third_party_content: 'none',
      fonts: 'Base-14 Helvetica, referenced not embedded',
      network_access: 'none',
    },
    license: "fixture content is MIT (LICENSE at the repo root)",
    generated_on: '2026-09-16',
  };
  writeFileSync(path.join(DIR, file.replace(/\.pdf$/, '.provenance.json')), `${JSON.stringify(rec, null, 2)}\n`);
  return rec;
}

function writePdf(name, objects, extra) {
  const data = pdfBytes(objects);
  writeFileSync(path.join(DIR, name), data);
  const rec = provenance(name.replace('.pdf', ''), name, data, extra);
  console.log(`wrote fixtures/p3/${name} (${data.length} bytes)`);
  return { data, rec };
}

function pypdf(file) {
  const py = `
import json, sys
from pypdf import PdfReader
from pypdf.generic import ArrayObject, StreamObject
r = PdfReader(sys.argv[1])
out = {"pages": len(r.pages), "text": [], "xobjects": [], "contents_count": [], "decoded_lens": []}
for i, page in enumerate(r.pages):
    out["text"].append(page.extract_text() or "")
    res = page.get("/Resources")
    try: res = res.get_object() if res else None
    except Exception: res = None
    xo = res.get("/XObject") if res else None
    try: xo = xo.get_object() if xo else None
    except Exception: pass
    out["xobjects"].append(sorted(str(k) for k in (xo.keys() if xo else [])))
    contents = page.get("/Contents")
    try: contents = contents.get_object() if contents else None
    except Exception: pass
    streams = list(contents) if isinstance(contents, ArrayObject) else ([contents] if contents else [])
    out["contents_count"].append(len(streams))
    lens = []
    for c in streams:
        try:
            obj = c.get_object() if hasattr(c, "get_object") else c
            lens.append(len(obj.get_data()) if isinstance(obj, StreamObject) else 0)
        except Exception:
            lens.append(-1)
    out["decoded_lens"].append(lens)
print(json.dumps(out))
`;
  const r = spawnSync('.venv-qa/bin/python', ['-c', py, path.join(DIR, file)], { encoding: 'utf8' });
  if (r.status !== 0) throw new Error(`pypdf ${file}: ${r.stderr || r.stdout}`);
  return JSON.parse(r.stdout);
}

const syntaxOps = `% comment must not yield targets: (ALPHA) Tj /Fm1 Do
BT /F1 12 Tf 72 720 Td (ALPHA) Tj 120 0 Td (TAIL) Tj ET
BT /F1 12 Tf 72 680 Td (ALPHA) Tj ET
BT /F1 12 Tf 72 640 Td (hello \\(world\\)) Tj ET
BT /F1 12 Tf 72 600 Td (\\101LPHA) Tj ET
BT /F1 12 Tf 72 560 Td <414C504841> Tj ET
BT /F1 12 Tf 72 520 Td (fake Tj and Do tokens) Tj ET
BT /F1 12 Tf 72 480 Td (\\351caf\\351) Tj ET
`;

const syntax = writePdf('targets-syntax.pdf', [
  '<< /Type /Catalog /Pages 2 0 R >>',
  '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
  '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>',
  FONT,
  stream(syntaxOps),
], {
  pages: 1,
  note: 'literal nest/escape/octal, hex, comment with fake operators, duplicate ALPHA, same-line TAIL, non-ASCII octal',
  expected: {
    malformed: false,
    tj: 8,
    do: 0,
    tj_duplicate_alpha: 2,
    same_line_following: 'TAIL',
    comment_is_not_a_target: true,
  },
});

const form1 = stream('BT /F1 12 Tf 0 0 Td (FORM) Tj ET\n', ' /Type /XObject /Subtype /Form /BBox [0 0 200 16] /Resources << /Font << /F1 5 0 R >> >>');
const form2 = stream(
  'q 1 0 0 1 0 0 cm /Fm1 Do Q\nBT /F1 12 Tf 0 20 Td (NEST) Tj ET\n',
  ' /Type /XObject /Subtype /Form /BBox [0 0 200 40] /Resources << /Font << /F1 5 0 R >> /XObject << /Fm1 8 0 R >> >>',
);
const p1a = 'BT /F1 12 Tf 72 720 Td ';
const p1b = `(SPLIT) Tj ET
q 1 0 0 1 72 640 cm /Fm1 Do Q
q 1 0 0 1 72 600 cm /Fm1 Do Q
q 1 0 0 1 72 560 cm /Fm2 Do Q
q 8 0 0 8 400 700 cm /Im1 Do Q
`;
const p2 = `q 1 0 0 1 72 700 cm /Fm1 Do Q
q 8 0 0 8 72 600 cm /Im1 Do Q
BT /F1 12 Tf 72 500 Td (PAGE2) Tj ET
`;
const streams = writePdf('targets-streams.pdf', [
  '<< /Type /Catalog /Pages 2 0 R >>',
  '<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>',
  '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> /XObject << /Fm1 8 0 R /Fm2 9 0 R /Im1 10 0 R >> >> /Contents [6 0 R 7 0 R] >>',
  '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> /XObject << /Fm1 8 0 R /Im1 10 0 R >> >> /Contents 11 0 R >>',
  FONT,
  stream(p1a),
  stream(p1b),
  form1,
  form2,
  stream(IMG, ' /Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceRGB /BitsPerComponent 8'),
  stream(p2),
], {
  pages: 2,
  note: 'Contents array carries BT state into stream 2; Fm1 twice on p1 plus nested Fm2; Fm1+Im1 shared with p2',
  expected: {
    malformed: false,
    page1_contents_streams: 2,
    page1_tj: 1,
    page1_do: { Fm1: 2, Fm2: 1, Im1: 1 },
    page2_do: { Fm1: 1, Im1: 1 },
    page2_tj: 1,
    form_fm1_tj: 1,
    form_fm2_do_fm1: 1,
    shared: ['Fm1', 'Im1'],
    cow: 'U1b — U1 discovery only, all Form descendants shared-or-unproven-sharing',
  },
});

const unsupOps = `BT /F1 12 Tf 72 700 Td [(T) -20 (JARR)] TJ ET
BT /F1 12 Tf 72 660 Td (SQ) ' ET
BT /F1 12 Tf 72 620 Td 1 0 (DQ) " ET
BT /F1 12 Tf 72 580 Td (OK) Tj ET
q
BI /W 1 /H 1 /CS /DeviceGray /BPC 8 /F /ASCIIHexDecode ID
FF
EI
Q
`;
const unsupported = writePdf('targets-unsupported.pdf', [
  '<< /Type /Catalog /Pages 2 0 R >>',
  '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
  '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>',
  FONT,
  stream(unsupOps),
], {
  pages: 1,
  note: 'valid PDF: TJ, quote operators, one supported Tj, inline image. Not malformed. Reuse fixtures/cjk.pdf, rtl.pdf, p0b/edit-subset.pdf, p0b/edit-embedded.pdf for encoding cases — do not duplicate.',
  expected: {
    malformed: false,
    tj: 1,
    tj_text: 'OK',
    TJ: 1,
    quote_single: 1,
    quote_double: 1,
    inline_image: 1,
    type3: 'untested-pending',
    vertical: 'untested-pending',
  },
});

const prefix = 'BT /F1 12 Tf 72 72 Td (HI) Tj ET\n';
const decoded = Buffer.concat([Buffer.from(prefix), Buffer.alloc(LIMIT - prefix.length, 0x20)]);
if (decoded.length !== LIMIT) throw new Error(`decoded ${decoded.length} != ${LIMIT}`);
const packed = flate(decoded);
const ratio = decoded.length / packed.compressed.length;
if (ratio < 100) throw new Error(`expansion ${ratio.toFixed(1)} < 100`);
const limits = writePdf('targets-limits.pdf', [
  '<< /Type /Catalog /Pages 2 0 R >>',
  '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
  '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>',
  FONT,
  packed.object,
], {
  pages: 1,
  note: 'one Flate contents stream, decoded 2MiB+1, expansion >= 100:1. Scan bound is 2MiB; expect too-large. readStream decompresses natively before JS sees length.',
  expected: {
    malformed: false,
    decoded_bytes: decoded.length,
    compressed_bytes: packed.compressed.length,
    expansion: Number(ratio.toFixed(2)),
    refusal: 'too-large',
    targets: 0,
  },
});

const before = process.memoryUsage();
const doc = mupdf.Document.openDocument(readFileSync(path.join(DIR, 'targets-limits.pdf')), 'application/pdf');
const page = doc.loadPage(0);
const buf = page.getObject().get('Contents').readStream();
const got = buf.asUint8Array().length;
buf.destroy();
page.destroy();
doc.destroy();
const after = process.memoryUsage();
const memory = {
  decoded: got,
  rss_delta_mb: Number(((after.rss - before.rss) / 1048576).toFixed(2)),
  heap_delta_mb: Number(((after.heapUsed - before.heapUsed) / 1048576).toFixed(2)),
  rss_after_mb: Number((after.rss / 1048576).toFixed(2)),
  note: 'JS 2MiB bound does not cap native inflate. App budget remains ≤600 MB renderer+worker.',
};

const observed = {
  'targets-syntax.pdf': pypdf('targets-syntax.pdf'),
  'targets-streams.pdf': pypdf('targets-streams.pdf'),
  'targets-unsupported.pdf': pypdf('targets-unsupported.pdf'),
  'targets-limits.pdf': pypdf('targets-limits.pdf'),
};

const manifest = {
  generator: GEN,
  save: 'fixtures only; not src/',
  reuse_not_duplicated: ['fixtures/cjk.pdf', 'fixtures/rtl.pdf', 'fixtures/p0b/edit-subset.pdf', 'fixtures/p0b/edit-embedded.pdf', 'fixtures/p0b/edit-tagged.pdf', 'fixtures/p0b/edit-transformed.pdf', 'fixtures/p0b/edit-shared.pdf'],
  type3_vertical: 'untested-pending — classify/refuse, do not mark passing',
  expected: {
    'targets-syntax.pdf': syntax.rec.expected,
    'targets-streams.pdf': streams.rec.expected,
    'targets-unsupported.pdf': unsupported.rec.expected,
    'targets-limits.pdf': { ...limits.rec.expected, memory },
  },
  pypdf: observed,
};
writeFileSync(path.join(DIR, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`);

const readme = `# P3 U1 target fixtures

Generated by \`node fixtures/p3/make-targets.mjs\`. Helvetica Base-14, no network. None of these files are malformed.

| file | what | constructed | pypdf |
|---|---|---|---|
| targets-syntax.pdf | comment with fake \`Tj\`/\`Do\`; two distinct \`(ALPHA) Tj\`; same-line \`TAIL\`; nested/escaped parens; octal \`\\\\101LPHA\`; hex \`<414C504841>\`; string containing "Tj" and "Do"; non-ASCII octal \`\\\\351\` | 8 \`Tj\`, 0 \`Do\` | text ${JSON.stringify(observed['targets-syntax.pdf'].text)} |
| targets-streams.pdf | \`/Contents [6 0 R 7 0 R]\` carries BT into stream 2 \`(SPLIT) Tj\`; \`Fm1\` Do ×2 on p1; nested \`Fm2→Fm1\`; \`Im1\` on p1; p2 shares \`Fm1\`+\`Im1\` | p1: 1 Tj + Do Fm1×2,Fm2×1,Im1×1; p2: 1 Tj + Do Fm1,Im1 | xobjects ${JSON.stringify(observed['targets-streams.pdf'].xobjects)} contents_count ${JSON.stringify(observed['targets-streams.pdf'].contents_count)} text ${JSON.stringify(observed['targets-streams.pdf'].text)} |
| targets-unsupported.pdf | valid \`TJ\`, \`'\`, \`"\`, one supported \`(OK) Tj\`, inline image \`BI…EI\` | TJ 1, quote 1+1, Tj 1, BI 1 | text ${JSON.stringify(observed['targets-unsupported.pdf'].text)} |
| targets-limits.pdf | one Flate stream decoded **${LIMIT}** bytes (${(LIMIT / 1048576).toFixed(4)} MiB), compressed **${packed.compressed.length}** bytes, expansion **${ratio.toFixed(1)}:1** | refuse \`too-large\`, no targets | decoded_lens ${JSON.stringify(observed['targets-limits.pdf'].decoded_lens)} |

Limits memory probe (mupdf \`readStream\` of the inflated contents): rss Δ ${memory.rss_delta_mb} MB, heap Δ ${memory.heap_delta_mb} MB, rss after ${memory.rss_after_mb} MB. Native inflate happens before JS can see the length. App budget still ≤600 MB.

Reuse, do not copy: \`cjk.pdf\`, \`rtl.pdf\`, \`fixtures/p0b/edit-{subset,embedded,tagged,transformed,shared}.pdf\`.

Type3 and vertical writing: **untested / pending**. Discovery must refuse conservatively; do not claim tested coverage.

U1b COW is not in these fixtures' write path. Shared/nested Form descendants stay \`shared-or-unproven-sharing\`.
`;
writeFileSync(path.join(DIR, 'README.md'), readme);
console.log(JSON.stringify({ limits: { decoded: LIMIT, compressed: packed.compressed.length, ratio: Number(ratio.toFixed(2)), memory } }, null, 2));
