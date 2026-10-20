// Small hostile pages on the Rust engine's WASM build stop neither the engine
// nor the machine (the integration acceptance review): PostScript functions,
// colour spaces and stitching functions nested thousands deep overflowed the
// WASM stack (a trap: the worker restarted and the document was lost), and a
// few kilobytes of nested Flate content, nested `q`, a long path or a dash
// pattern grew WASM memory by gigabytes toward wasm32's 4 GiB, where Rust
// aborts. Each case now draws or is refused (`too complex`), the engine keeps
// running, and the WASM memory of the whole file stays under 1 GiB. Native
// cases: engine/tests/view_hostile.rs. Runs under npm run test:rust
// (TPDF_ENGINE=rust).
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createDeflate, deflateSync } from 'node:zlib';

const skip = process.env.TPDF_ENGINE === 'rust' ? false : 'runs under npm run test:rust';
const pkg = new URL('../engine/web/pkg/', import.meta.url);

/** The production adapter over the WASM package, and the package's memory. */
async function rustEngine() {
  const wasm = await import(new URL('tandem_rust_workflow.js', pkg).href);
  const exports = wasm.initSync({ module: readFileSync(new URL('tandem_rust_workflow_bg.wasm', pkg)) });
  const { createRustEngine } = await import('../build-test/engine-rust.js');
  return { engine: createRustEngine(wasm), memory: () => exports.memory.buffer.byteLength };
}

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
  const bytes = Buffer.isBuffer(data) ? data : Buffer.from(data, 'latin1');
  return Buffer.concat([Buffer.from(`<< ${dict} /Length ${bytes.length} >>\nstream\n`, 'latin1'), bytes, Buffer.from('\nendstream', 'latin1')]);
}

/** A 100 × 100 page with `resources` and `content` (a stream object 4); `extra` objects follow as 5, 6, ... */
function page(resources, content, extra = []) {
  return pdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources ${resources} /Contents 4 0 R >>`,
    typeof content === 'string' ? stream('/Filter /FlateDecode', deflateSync(Buffer.from(content, 'latin1'))) : content,
    ...extra,
  ]);
}

/** `megabytes` MiB of spaces (or `fill`) after `head`, then `tail`, behind two Flate filters: a few kilobytes. */
async function nestedFlate(megabytes, tail, { head = Buffer.alloc(0), fill = 0x20 } = {}) {
  const inner = await new Promise((resolve, reject) => {
    const deflate = createDeflate({ level: 9 });
    const parts = [];
    deflate.on('data', part => parts.push(part));
    deflate.on('end', () => resolve(Buffer.concat(parts)));
    deflate.on('error', reject);
    deflate.write(head);
    const chunk = Buffer.alloc(16 << 20, fill);
    let written = 0;
    const pump = () => {
      while (written < megabytes) {
        const size = Math.min(16, megabytes - written);
        written += size;
        if (!deflate.write(chunk.subarray(0, size << 20))) return void deflate.once('drain', pump);
      }
      deflate.end(Buffer.from(tail, 'latin1'));
    };
    pump();
  });
  return deflateSync(inner, { level: 9 });
}

const shading = fn => ['<< /ShadingType 2 /ColorSpace /DeviceGray /Coords [0 0 100 0] /Function 6 0 R >>', fn];
const deepProgram = depth => stream('/FunctionType 4 /Domain [0 1] /Range [0 1]', `{ ${'{ '.repeat(depth)}${'} if '.repeat(depth)}}`);

function chain(n, make, last) {
  return Array.from({ length: n }, (_, i) => (i + 1 < n ? make(i + 6) : last));
}

const CASES = [
  ['a PostScript function nested 5,000 deep', () => page('<< /Shading << /Sh 5 0 R >> >>', '/Sh sh', shading(deepProgram(5000)))],
  ['a tint transform nested 5,000 deep', () => page('<< /ColorSpace << /C [/Separation /S /DeviceGray 5 0 R] >> >>', '/C cs 1 sc 0 0 100 100 re f', [deepProgram(5000)])],
  ['3,000 Indexed spaces, each the base of the one before', () => page('<< /ColorSpace << /C 5 0 R >> >>', '/C cs 1 sc 0 0 100 100 re f',
    chain(3000, next => `[/Indexed ${next} 0 R 1 <000000ffffff>]`, '/DeviceRGB'))],
  ['3,000 Separation spaces, each the alternate of the one before', () => page('<< /ColorSpace << /C 5 0 R >> >>', '/C cs 1 sc 0 0 100 100 re f',
    chain(3000, next => `[/Separation /S ${next} 0 R << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >>]`, '/DeviceGray'))],
  ['3,000 stitching functions, each the part of the one before', () => page('<< /Shading << /Sh 5 0 R >> >>', '/Sh sh', [
    shading('')[0],
    ...Array.from({ length: 3000 }, (_, i) => (i + 1 < 3000
      ? `<< /FunctionType 3 /Domain [0 1] /Functions [${i + 7} 0 R] /Bounds [] /Encode [0 1] >>`
      : '<< /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >>')),
  ])],
  ['a stitching function that is its own part', () => page('<< /Shading << /Sh 5 0 R >> >>', '/Sh sh', shading('<< /FunctionType 3 /Domain [0 1] /Functions [6 0 R] /Bounds [] /Encode [0 1] >>'))],
  ['a sampled function of 2^31 one-bit samples', () => page('<< /Shading << /Sh 5 0 R >> >>', '/Sh sh',
    shading(stream('/FunctionType 0 /Domain [0 1] /Range [0 1] /Size [2147483648] /BitsPerSample 1 /Filter /FlateDecode', deflateSync(Buffer.alloc(8 << 20)))))],
  ['600,000 nested q (2.8 KB)', () => page('<< >>', `${'q '.repeat(600000)}0 0 10 10 re f${' Q'.repeat(600000)}`)],
  ['a path of 8,000,000 segments (94 KB)', () => page('<< >>', `0 0 m ${'1 1 l 2 0 l '.repeat(4000000)}S`)],
  ['a stroke of 800,000 segments (20 KB)', () => page('<< >>', `0 0 m ${'1 1 l 2 0 l '.repeat(400000)}S`)],
  ['a dash pattern along a line of 10^9 units', () => page('<< >>', '[1] 0 d 0 50 m 1000000000 50 l S')],
  ['a dash pattern of 10^-5 units', () => page('<< >>', '[0.00001] 0 d 0 50 m 100 50 l S')],
  ['content behind a predictor with /Columns 0', () => page('<< >>', stream('/Filter /FlateDecode /DecodeParms << /Predictor 2 /Columns 0 >>', deflateSync(Buffer.from('0 0 10 10 re f'))))],
  ['content decoding to 256 MiB behind nested Flate', async () => page('<< >>', stream('/Filter [/FlateDecode /FlateDecode]', await nestedFlate(256, '0 0 10 10 re f')))],
  ['an image decoding to 256 MiB behind nested Flate', async () => page('<< /XObject << /Im 5 0 R >> >>', 'q 100 0 0 100 0 0 cm /Im Do Q',
    [stream('/Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray /Filter [/FlateDecode /FlateDecode]', await nestedFlate(256, ''))])],
];

test('rust engine: hostile pages draw or are refused within bounded WASM memory', { skip, timeout: 600_000 }, async () => {
  const { engine, memory } = await rustEngine();
  const megabytes = () => Math.round(memory() / 1048576);
  const before = megabytes();
  for (const [name, make] of CASES) {
    const session = engine.createSession();
    const bytes = await make();
    assert.ok(bytes.byteLength < 1 << 20, `${name}: ${bytes.byteLength} bytes`);
    const open = engine.dispatch({ id: 1, op: 'open', name: 'x.pdf', pdf: bytes }, session).reply;
    assert.equal(open.ok, true, `${name}: ${JSON.stringify(open)}`);
    const render = engine.dispatch({ id: 2, op: 'render', page: 0, scale: 1 }, session).reply;
    assert.notEqual(render.reason, 'worker', `${name}: ${render.error}`);
    if (!render.ok) assert.match(render.error, /too complex/, `${name}: ${render.error}`);
    const text = engine.dispatch({ id: 3, op: 'text-all' }, session).reply;
    assert.notEqual(text.reason, 'worker', `${name}: ${text.error}`);
    engine.dispatch({ id: 4, op: 'close' }, session);
    assert.ok(megabytes() < 1024, `${name}: WASM memory ${megabytes()} MB (from ${before} MB)`);
  }
  // The engine still opens and draws an ordinary page.
  const session = engine.createSession();
  engine.dispatch({ id: 5, op: 'open', name: 'ok.pdf', pdf: page('<< >>', '0 0 50 50 re f') }, session);
  const reply = engine.dispatch({ id: 6, op: 'render', page: 0, scale: 1 }, session).reply;
  assert.equal(reply.ok, true, JSON.stringify(reply));
  assert.ok(reply.nonWhite > 2000, JSON.stringify(reply.nonWhite));
});

// Decoded streams held together (the integration review, round 2): each
// stream above decodes within its 128 MiB limit, but hayro holds a form while
// the forms it draws run, hayro-syntax kept every page's decoded content for
// the life of the document, and hayro keeps every font of a render with its
// program copied whole. Each grew WASM memory by about 128 MiB a step from a
// file of kilobytes (a font: of the app's own fallback font). Native cases:
// view_hostile.rs (`nested_forms_hold_...`, `the_fonts_a_page_selects_...`,
// `pages_past_the_kept_content_...`).
test('rust engine: decoded content held along forms, across pages and by fonts stays bounded', { skip, timeout: 600_000 }, async () => {
  const { engine, memory } = await rustEngine();
  const megabytes = () => Math.round(memory() / 1048576);
  const run = (name, bytes, pages) => {
    const session = engine.createSession();
    const open = engine.dispatch({ id: 1, op: 'open', name: 'x.pdf', pdf: bytes }, session).reply;
    assert.equal(open.ok, true, `${name}: ${JSON.stringify(open)}`);
    const renders = [];
    for (let page = 0; page < pages; page++) {
      const render = engine.dispatch({ id: 2 + page, op: 'render', page, scale: 0.5 }, session).reply;
      assert.notEqual(render.reason, 'worker', `${name}: ${render.error}`);
      if (!render.ok) assert.match(render.error, /too complex/, `${name}: ${render.error}`);
      renders.push(render);
    }
    const text = engine.dispatch({ id: 1000, op: 'text-all' }, session).reply;
    assert.notEqual(text.reason, 'worker', `${name}: ${text.error}`);
    engine.dispatch({ id: 1001, op: 'close' }, session);
    assert.ok(megabytes() < 1024, `${name}: WASM memory ${megabytes()} MB`);
    return renders;
  };

  // Eight forms of 120 MiB, each drawing the next (357 MB at two levels,
  // 613 MB at four before): refused once they hold more than 128 MiB.
  const forms = [];
  for (let level = 0; level < 8; level++) {
    const next = level < 7 ? `/F${level + 1} Do` : '0 0 50 50 re f';
    const resources = level < 7 ? `/Resources << /XObject << /F${level + 1} ${6 + level} 0 R >> >>` : '';
    forms.push(stream(`/Type /XObject /Subtype /Form /BBox [0 0 100 100] ${resources} /Filter [/FlateDecode /FlateDecode]`, await nestedFlate(120, ` q ${next} Q`)));
  }
  const nested = page('<< /XObject << /F0 5 0 R >> >>', '/F0 Do', forms);
  assert.ok(nested.byteLength < 16 << 10, `${nested.byteLength} bytes`);
  assert.equal(run('eight nested forms of 120 MiB', nested, 1)[0].ok, false);

  // Nine pages sharing one content stream of 120 MiB, all drawn and read:
  // each kept its 128 MiB before.
  const count = 9;
  const kids = Array.from({ length: count }, (_, i) => `${3 + i} 0 R`).join(' ');
  const shared = pdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    `<< /Type /Pages /Kids [${kids}] /Count ${count} >>`,
    ...Array.from({ length: count }, () => `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << >> /Contents ${3 + count} 0 R >>`),
    stream('/Filter [/FlateDecode /FlateDecode]', await nestedFlate(120, ' 0 0 50 50 re f')),
  ]);
  assert.ok(shared.byteLength < 16 << 10, `${shared.byteLength} bytes`);
  for (const render of run('nine pages sharing 120 MiB of content', shared, count)) {
    assert.equal(render.ok, true, render.error);
    assert.ok(render.nonWhite > 400, JSON.stringify(render.nonWhite));
  }

  // Ten TrueType fonts whose program is the app's fallback font padded to
  // 100 MiB: refused once the page's fonts pass 256 MiB.
  const font = readFileSync(new URL('../assets/fonts/DroidSansFallback.ttf', import.meta.url));
  const fonts = [];
  let names = '';
  let shows = 'BT ';
  for (let i = 0; i < 10; i++) {
    names += `/F${i} ${6 + 2 * i} 0 R `;
    shows += `/F${i} 12 Tf 10 ${5 + 9 * i} Td (A) Tj `;
    fonts.push(`<< /Type /Font /Subtype /TrueType /BaseFont /Padded${i} /FirstChar 65 /LastChar 65 /Widths [600] /FontDescriptor ${7 + 2 * i} 0 R >>`);
    fonts.push(`<< /Type /FontDescriptor /FontName /Padded${i} /Flags 32 /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 900 /Descent -200 /CapHeight 700 /StemV 80 /FontFile2 5 0 R >>`);
  }
  const program = stream('/Filter [/FlateDecode /FlateDecode]', await nestedFlate(96, '', { head: font, fill: 0 }));
  const padded = page(`<< /Font << ${names}>> >>`, `${shows}ET`, [program, ...fonts]);
  assert.equal(run('ten fonts of a padded 100 MiB program', padded, 1)[0].ok, false);
});
