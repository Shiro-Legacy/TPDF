// Decoded content held through what a form inherits, on the Rust engine's
// WASM build (the integration acceptance review, round 3). hayro decodes a
// content stream whole and holds it while what it invokes runs, and a form
// inherits its caller's fill pattern, Type3 font and soft mask, which the
// engine's walk counts where they are set: forms of 120 MiB chained through
// inherited patterns or Type3 fonts grew WASM memory by 128 MiB a level
// (1,157 MB at eight levels from a 6,923-byte file; about 31 levels reach
// wasm32's 4 GiB abort). hayro-interpret now counts the content it holds
// and reads and refuses past either budget, so each page here is refused
// (`too complex`) holding one form, and the WASM memory of the whole run
// stays under 512 MiB. Native cases, with the counter itself:
// engine/tests/view_inherited.rs. Runs under npm run test:rust
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

const flate = (dict, data) => stream(`${dict} /Filter /FlateDecode`, deflateSync(Buffer.from(data, 'latin1')));

/** A 100 × 100 page with `resources` and `content` (object 4); `extra` objects follow as 5, 6, ... */
function page(resources, content, extra = []) {
  return pdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources ${resources} /Contents 4 0 R >>`,
    flate('', content),
    ...extra,
  ]);
}

/** `megabytes` MiB of spaces, then `tail`, behind two Flate filters: a few kilobytes. */
async function nestedFlate(megabytes, tail) {
  const inner = await new Promise((resolve, reject) => {
    const deflate = createDeflate({ level: 9 });
    const parts = [];
    deflate.on('data', part => parts.push(part));
    deflate.on('end', () => resolve(Buffer.concat(parts)));
    deflate.on('error', reject);
    const chunk = Buffer.alloc(16 << 20, 0x20);
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

const FORM = '/Type /XObject /Subtype /Form /BBox [0 0 100 100]';
const CELL = '/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10';
const big = (dict, data) => stream(`${dict} /Filter [/FlateDecode /FlateDecode]`, data);

/** The page sets fill pattern P1 and draws form G1, which fills with the inherited P1; P1's cell sets P2 and draws G2; ... */
function patternChain(levels, form) {
  const extra = [];
  for (let level = 1; level <= levels; level++) {
    const g = 5 + 2 * (level - 1);
    extra.push(big(FORM, form));
    extra.push(level < levels
      ? flate(`${CELL} /Resources << /Pattern << /P ${g + 3} 0 R >> /XObject << /G ${g + 2} 0 R >> >>`, '/Pattern cs /P scn /G Do')
      : flate(`${CELL} /Resources << >>`, '0 0 1 rg 0 0 5 5 re f'));
  }
  return page('<< /Pattern << /P 6 0 R >> /XObject << /G 5 0 R >> >>', '/Pattern cs /P scn /G Do', extra);
}

/** The page selects Type3 font T1 and draws G1, which shows a glyph with the inherited T1; T1's glyph selects T2 and draws G2; ... */
function type3Chain(levels, form) {
  const extra = [];
  for (let level = 1; level <= levels; level++) {
    const g = 5 + 3 * (level - 1);
    extra.push(big(FORM, form));
    const resources = level < levels ? `/Resources << /Font << /T ${g + 4} 0 R >> /XObject << /G ${g + 3} 0 R >> >>` : '';
    extra.push(`<< /Type /Font /Subtype /Type3 /FontBBox [0 0 100 100] /FontMatrix [0.01 0 0 0.01 0 0] /CharProcs << /a ${g + 2} 0 R >> /Encoding << /Type /Encoding /Differences [97 /a] >> /FirstChar 97 /LastChar 97 /Widths [100] ${resources} >>`);
    extra.push(flate('', level < levels ? '100 0 0 0 100 100 d1 BT /T 50 Tf ET /G Do' : '100 0 0 0 100 100 d1 0 0 100 100 re f'));
  }
  return page('<< /Font << /T 6 0 R >> /XObject << /G 5 0 R >> >>', 'BT /T 20 Tf ET /G Do', extra);
}

test('rust engine: content inherited into forms is held and read within bounded WASM memory', { skip, timeout: 600_000 }, async () => {
  const { engine, memory } = await rustEngine();
  const megabytes = () => Math.round(memory() / 1048576);
  const run = (name, bytes) => {
    assert.ok(bytes.byteLength < 32 << 10, `${name}: ${bytes.byteLength} bytes`);
    const session = engine.createSession();
    const open = engine.dispatch({ id: 1, op: 'open', name: 'x.pdf', pdf: bytes }, session).reply;
    assert.equal(open.ok, true, `${name}: ${JSON.stringify(open)}`);
    const render = engine.dispatch({ id: 2, op: 'render', page: 0, scale: 1 }, session).reply;
    assert.notEqual(render.reason, 'worker', `${name}: ${render.error}`);
    const text = engine.dispatch({ id: 3, op: 'text-all' }, session).reply;
    assert.notEqual(text.reason, 'worker', `${name}: ${text.error}`);
    engine.dispatch({ id: 4, op: 'close' }, session);
    assert.ok(megabytes() < 512, `${name}: WASM memory ${megabytes()} MB`);
    return render;
  };
  const refused = (name, render, why) => {
    assert.equal(render.ok, false, `${name}: drawn`);
    assert.match(render.error, /too complex/, `${name}: ${render.error}`);
    assert.match(render.error, why, `${name}: ${render.error}`);
  };

  // The review's cases: eight levels of 120 MiB (1,157 and 1,158 MB before).
  const patternForm = await nestedFlate(120, ' 0 0 100 100 re f');
  const one = run('a pattern inherited by one form of 120 MiB', patternChain(1, patternForm));
  assert.equal(one.ok, true, one.error);
  assert.ok(one.nonWhite > 2000, JSON.stringify(one.nonWhite));
  refused('patterns inherited through eight forms of 120 MiB', run('pattern chain', patternChain(8, patternForm)), /decodes past the size limit/);

  const glyphForm = await nestedFlate(120, ' BT 10 10 Td (a) Tj ET');
  const glyph = run('a Type3 font inherited by one form of 120 MiB', type3Chain(1, glyphForm));
  assert.equal(glyph.ok, true, glyph.error);
  assert.ok(glyph.nonWhite > 300, JSON.stringify(glyph.nonWhite));
  refused('Type3 fonts inherited through eight forms of 120 MiB', run('Type3 chain', type3Chain(8, glyphForm)), /decodes past the size limit/);

  // Eight nested graphics states each keeping a soft mask of 60 MiB.
  const mask = big('/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Group << /S /Transparency /CS /DeviceGray >>', await nestedFlate(60, ' 1 g 0 0 100 100 re f'));
  const masked = page('<< /ExtGState << /S << /SMask << /Type /Mask /S /Luminosity /G 5 0 R >> >> >> >>',
    `${'q /S gs '.repeat(8)}0 0 50 50 re f${' Q'.repeat(8)}`, [mask]);
  refused('eight soft masks of 60 MiB kept with their states', run('soft masks', masked), /decodes past the size limit/);

  // A tiny form drawn 30 times fills with the inherited pattern, whose cell
  // draws a form of 60 MiB: 1.8 GB to inflate. A page reads at most 1 GiB.
  const repeated = page('<< /Pattern << /P 6 0 R >> /XObject << /G1 5 0 R >> >>', `/Pattern cs /P scn ${'/G1 Do '.repeat(30)}`, [
    flate(FORM, '0 0 10 10 re f'),
    flate(`${CELL} /Resources << /XObject << /G 7 0 R >> >>`, '/G Do'),
    big(FORM, await nestedFlate(60, ' 0 0 5 5 re f')),
  ]);
  refused('an inherited cell drawing 60 MiB, filled 30 times', run('repeated cell', repeated), /expands past the work limit/);

  // The engine still opens and draws an ordinary page.
  const session = engine.createSession();
  engine.dispatch({ id: 5, op: 'open', name: 'ok.pdf', pdf: page('<< >>', '0 0 50 50 re f') }, session);
  const reply = engine.dispatch({ id: 6, op: 'render', page: 0, scale: 1 }, session).reply;
  assert.equal(reply.ok, true, JSON.stringify(reply));
  assert.ok(reply.nonWhite > 2000, JSON.stringify(reply.nonWhite));
});
