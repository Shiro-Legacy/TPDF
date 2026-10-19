import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { build } from 'esbuild';
import mupdf from 'mupdf';
import { readCjkFont } from '../scripts/cjk-font.mjs';
import { rustClient, rustEngine } from './rust.mjs';

// The scanner has the Rust engine the app ships compose its PDFs (scan-compose; scan-compose.ts lays the pages
// out) and checks them with the same engine (scan-worker.ts). MuPDF reads them here only as an independent
// oracle; it is not part of the mobile app.
async function module(path) {
  const result = await build({ entryPoints: [path], bundle: true, format: 'esm', write: false });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}`);
}
const { composeScan, checkScanPDF, failure } = await module('src/scan-compose.ts');
const { providerRequest, providerText } = await module('src/scan-ai.ts');
const { validateLines, parseDocument } = await module('src/scan-model.ts');
// One engine for the file, as the scan worker has one: the CJK font is read the first time a scan needs it.
let fontReads = 0;
const engine = rustEngine(() => { fontReads++; return readCjkFont(); });
const compose = (pages, output) => composeScan(engine, pages, output).bytes;

// The bundled sample page as a JPEG like the scanner's canvas output (the scanner's input is always JPEG),
// rendered at 2x and encoded by the oracle.
const welcome = mupdf.Document.openDocument(await readFile('dist/welcome.pdf'), 'application/pdf');
const pixmap = welcome.loadPage(0).toPixmap(mupdf.Matrix.scale(2, 2), mupdf.ColorSpace.DeviceRGB, false);
const jpeg = pixmap.asJPEG(90);
const scan = { id: 'scan', data: `data:image/jpeg;base64,${Buffer.from(jpeg).toString('base64')}`, width: pixmap.getWidth(), height: pixmap.getHeight(),
  lines: [{ text: 'Hello scan résumé 中文', x: .1, y: .2, width: .5, height: .03 }] };
const latin = [{ ...scan, lines: [{ text: 'Hello scan résumé', x: .1, y: .2, width: .5, height: .03 }] }];
const open = bytes => mupdf.Document.openDocument(new Uint8Array(bytes), 'application/pdf');
const text = bytes => {
  const doc = open(bytes);
  let all = '';
  for (let i = 0; i < doc.countPages(); i++) { const page = doc.loadPage(i), st = page.toStructuredText(); all += st.asText(); st.destroy(); page.destroy(); }
  doc.destroy();
  return all;
};

// First, while no CJK font is installed in this process's engine.
test('only text that needs it reads the CJK font, and only the glyphs used are embedded', () => {
  fontReads = 0;
  const plain = compose(latin, 'searchable');
  assert.equal(fontReads, 0, 'Latin text needs no CJK font');
  assert.ok(plain.byteLength < jpeg.length + 12_000, `${plain.byteLength} bytes`);
  // Greek, Cyrillic and Latin Extended letters take the engine's own fonts, not the CJK one.
  compose([{ ...scan, markdown: '# Ωμέγα Привет\n\nZażółć gęślą jaźń ≤ ≥ → €' }], 'digital');
  assert.equal(fontReads, 0);
  const cjk = compose([scan], 'searchable');
  assert.equal(fontReads, 1);
  // MuPDF embedded its whole fallback font (about 1.7 MB) for the same page; the subset holds 3 glyphs.
  assert.ok(cjk.byteLength < jpeg.length + 40_000, `${cjk.byteLength} bytes`);
  // A character no font has still reads back (ToUnicode), drawn as the CJK font's .notdef.
  const rare = compose([{ ...scan, lines: [{ text: 'Rare 𝔘 ✓ ภาษา', x: .1, y: .2, width: .5, height: .03 }] }], 'searchable');
  assert.match(text(rare), /Rare 𝔘 ✓ ภาษา/);
  checkScanPDF(engine, rare, 1);
  assert.deepEqual(Buffer.from(compose([scan], 'searchable')), Buffer.from(cjk), 'the same scan, the same bytes');
  assert.equal(fontReads, 1);
});

test('searchable scan retains identical pixels and searchable Unicode text after reopen', () => {
  const imageDoc = open(compose([scan], 'image'));
  const textBytes = compose([scan], 'searchable');
  const textDoc = open(textBytes);
  const imagePage = imageDoc.loadPage(0), textPage = textDoc.loadPage(0);
  const a = imagePage.toPixmap(mupdf.Matrix.identity, mupdf.ColorSpace.DeviceRGB, false);
  const b = textPage.toPixmap(mupdf.Matrix.identity, mupdf.ColorSpace.DeviceRGB, false);
  assert.deepEqual(a.getPixels(), b.getPixels(), 'OCR must not change scan appearance');
  const st = textPage.toStructuredText();
  assert.match(st.asText(), /Hello scan résumé 中文/);
  assert.ok(textPage.search('résumé').length);
  const line = JSON.parse(st.asJSON()).blocks.find(block => block.type === 'text').lines[0];
  assert.ok(line.bbox.x >= 60 && line.bbox.x < 63, JSON.stringify(line));
  // The OCR line spans the recognized width (0.5 of the page): the run is scaled to fit.
  assert.ok(Math.abs(line.bbox.w - 306) < 4, JSON.stringify(line.bbox));
  st.destroy(); a.destroy(); b.destroy(); imagePage.destroy(); textPage.destroy(); imageDoc.destroy(); textDoc.destroy();
  // The Rust engine reads the same text and draws the page the same with or without it.
  const send = rustClient(assert);
  send({ op: 'open', pdf: textBytes.slice(0), name: 'scan.pdf' });
  assert.match(send({ op: 'text-all' }).text, /Hello scan résumé 中文/);
  assert.ok(send({ op: 'search', query: 'résumé' }).hits.length);
  assert.ok(send({ op: 'search', query: '中文' }).hits.length);
  const withText = new Uint8Array(send({ op: 'render', page: 0, scale: 1 }).pixels);
  send({ op: 'open', pdf: compose([scan], 'image').slice(0), name: 'image.pdf' });
  assert.deepEqual(new Uint8Array(send({ op: 'render', page: 0, scale: 1 }).pixels), withText, 'OCR must not change scan appearance (Rust)');
  send({ op: 'close' });
});

test('digital reconstruction produces real text, tables and overflow pages without losing the tail', () => {
  const content = '# Project notes\n\nRésumé 中文\n\n| Item | Quantity |\n| --- | --- |\n| Paper | 17 |\n\n' + Array.from({length:150}, (_,i) => `- Line ${i} carries exact words and figures 123.45.`).join('\n') + '\n\nEND-OF-DOCUMENT';
  const { bytes, pageCount } = composeScan(engine, [{...scan, markdown:content}], 'digital');
  const doc = open(bytes);
  assert.ok(doc.countPages() > 2);
  assert.equal(doc.countPages(), pageCount);
  doc.destroy();
  const all = text(bytes);
  for (const expected of ['Project notes', 'Résumé 中文', 'Quantity', 'Paper', '17', 'Line 149', 'END-OF-DOCUMENT']) assert.ok(all.includes(expected), expected);
  // Streams are compressed by the engine on every platform (no CompressionStream needed).
  assert.ok(Buffer.from(bytes).includes('FlateDecode'));
  const send = rustClient(assert);
  assert.equal(send({ op: 'open', pdf: bytes.slice(0), name: 'rebuilt.pdf' }).pageCount, pageCount);
  const rust = send({ op: 'text-all' }).text;
  for (const expected of ['Project notes', 'Résumé 中文', 'Quantity', 'Paper', 'Line 149', 'END-OF-DOCUMENT']) assert.ok(rust.includes(expected), expected);
  send({ op: 'close' });
});

test('scan output keeps the MuPDF scanner page size, untouched JPEG, embedded profiles and multi-page order', () => {
  const wide = { ...scan, id: 'wide', width: scan.height, height: scan.width };
  const { bytes, pageCount } = composeScan(engine, [scan, scan, scan], 'image');
  assert.equal(pageCount, 3);
  const doc = open(bytes), pdf = doc.asPDF();
  assert.equal(doc.countPages(), 3);
  assert.equal(pdf.wasRepaired(), false);
  // Long side 792 pt, aspect from the scan: 1224 x 1584 px -> 612 x 792 pt, as before.
  assert.deepEqual(doc.loadPage(0).getBounds(), [0, 0, 612, 792]);
  const image = pdf.findPage(0).get('Resources').get('XObject').get('Im0');
  assert.equal(image.get('Filter').asName(), 'DCTDecode');
  assert.deepEqual(Buffer.from(image.readRawStream().asUint8Array()), Buffer.from(jpeg), 'the JPEG is stored as it is, not recompressed');
  // MuPDF wrote its own sRGB profile for DeviceRGB; the scan's JPEG names none, so the page says DeviceRGB.
  assert.equal(image.get('ColorSpace').asName(), 'DeviceRGB');
  assert.equal(pdf.getTrailer().get('Info').get('Producer').asString(), 'Tandem PDF');
  // Three pages of one image store it once.
  assert.ok(bytes.byteLength < jpeg.length + 4000, `${bytes.byteLength} bytes`);
  doc.destroy();
  // A landscape scan gets a landscape page; mixed orientations keep their order.
  const mixed = open(compose([scan, { ...scan, width: wide.width, height: wide.height }], 'image'));
  assert.deepEqual(mixed.loadPage(1).getBounds().map(Math.round), [0, 0, 792, 612]);
  mixed.destroy();
  // A profile the JPEG carries (APP2 ICC_PROFILE chunks, here split in two) becomes its ICCBased colour space.
  const profile = Buffer.from('a stand-in ICC profile, split across two chunks');
  const app2 = (part, count, data) => Buffer.concat([Buffer.from([0xff, 0xe2, 0, 16 + data.length]), Buffer.from('ICC_PROFILE\0'), Buffer.from([part, count]), data]);
  const tagged = Buffer.concat([jpeg.subarray(0, 2), app2(2, 2, profile.subarray(20)), app2(1, 2, profile.subarray(0, 20)), jpeg.subarray(2)]);
  const icc = open(compose([{ ...scan, data: `data:image/jpeg;base64,${tagged.toString('base64')}` }], 'image'));
  const space = icc.asPDF().findPage(0).get('Resources').get('XObject').get('Im0').get('ColorSpace');
  assert.equal(space.get(0).asName(), 'ICCBased');
  assert.equal(space.get(1).get('N').asNumber(), 3);
  assert.deepEqual(Buffer.from(space.get(1).readStream().asUint8Array()), profile);
  icc.destroy();
  // The engine the app ships accepts what it composes.
  checkScanPDF(engine, bytes, 3);
  assert.throws(() => checkScanPDF(rustEngine(), bytes, 2), /did not pass validation/);
  assert.throws(() => checkScanPDF(rustEngine(), bytes.slice(0, bytes.byteLength - 200), 3), /did not pass validation/);
});

// Which font draws each character outside WinAnsi in rebuilt pages, by Unicode block (scan-measure's `fonts`):
// Helvetica's Latin glyph names (`latin`), the engine's embedded Nimbus Sans outlines (`nimbus`), the CJK font
// (`cjk`), or none (`missing`: the CJK font's box; the text still extracts).
const BLOCKS = {
  'Latin Extended-A': [0x100, 0x17f], 'Latin Extended-B': [0x180, 0x24f], 'IPA': [0x250, 0x2af],
  'Combining marks': [0x300, 0x36f], 'Greek': [0x370, 0x3ff], 'Cyrillic': [0x400, 0x4ff],
  'Hebrew': [0x590, 0x5ff], 'Arabic': [0x600, 0x6ff], 'Thai': [0xe00, 0xe7f],
  'Latin Extended Additional (Vietnamese)': [0x1e00, 0x1eff], 'General Punctuation': [0x2000, 0x206f],
  'Currency': [0x20a0, 0x20cf], 'Letterlike': [0x2100, 0x214f], 'Arrows': [0x2190, 0x21ff],
  'Math Operators': [0x2200, 0x22ff], 'Box Drawing': [0x2500, 0x257f], 'Misc Symbols': [0x2600, 0x26ff],
  'CJK Symbols': [0x3000, 0x303f], 'Hiragana': [0x3040, 0x309f], 'Katakana': [0x30a0, 0x30ff],
  'CJK Unified (BMP)': [0x4e00, 0x9fff], 'Hangul Syllables': [0xac00, 0xd7a3], 'Fullwidth forms': [0xff00, 0xffef],
  'Emoji': [0x1f300, 0x1faff],
};
// [assigned, Latin names, Nimbus, CJK font, box]; the assigned totals follow Node's Unicode version, the CJK
// counts are pinned by the font's sha256 (scripts/cjk-font.mjs).
const COVERAGE = {
  'Latin Extended-A': [121, 121, 0, 0, 0],
  'Latin Extended-B': [207, 4, 6, 0, 197],
  'IPA': [96, 0, 0, 0, 96],
  'Combining marks': [112, 0, 0, 0, 112],
  'Greek': [135, 0, 72, 0, 63],
  'Cyrillic': [256, 0, 137, 0, 119],
  'Hebrew': [88, 0, 0, 0, 88],
  'Arabic': [256, 0, 0, 0, 256],
  'Thai': [87, 0, 0, 1, 86],
  'Latin Extended Additional (Vietnamese)': [256, 0, 8, 0, 248],
  'General Punctuation': [96, 0, 8, 4, 84],
  'Currency': [33, 0, 4, 0, 29],
  'Letterlike': [79, 0, 5, 0, 74],
  'Arrows': [112, 0, 7, 0, 105],
  'Math Operators': [256, 0, 28, 0, 228],
  'Box Drawing': [128, 0, 0, 0, 128],
  'Misc Symbols': [256, 0, 11, 0, 245],
  'CJK Symbols': [64, 0, 0, 36, 28],
  'Hiragana': [93, 0, 0, 90, 3],
  'Katakana': [96, 0, 0, 91, 5],
  'CJK Unified (BMP)': [20992, 0, 0, 20902, 90],
  'Hangul Syllables': [11172, 0, 0, 11172, 0],
  'Fullwidth forms': [225, 0, 0, 164, 61],
  'Emoji': [1895, 0, 0, 0, 1895],
};
test('font coverage per script outside WinAnsi: Latin names, Nimbus Sans, the CJK font, boxes', () => {
  const send = rustClient(assert);
  const rows = [];
  for (const [name, [from, to]] of Object.entries(BLOCKS)) {
    let chars = '';
    for (let code = from; code <= to; code++) {
      const char = String.fromCodePoint(code);
      if (/\p{Assigned}/u.test(char) && !/\p{Cc}/u.test(char)) chars += char;
    }
    const { chars: measured, fonts } = send({ op: 'scan-measure', text: chars });
    const count = { latin: 0, nimbus: 0, cjk: 0, missing: 0 };
    measured.forEach((_, i) => { if (fonts[i] !== 'helvetica') count[fonts[i]]++; });
    const assigned = measured.length - fonts.filter(font => font === 'helvetica').length;
    rows.push(`| ${name} | ${assigned} | ${count.latin} | ${count.nimbus} | ${count.cjk} | ${count.missing} |`);
    assert.deepEqual([assigned, count.latin, count.nimbus, count.cjk, count.missing], COVERAGE[name], name);
  }
  console.log(`| Block | Assigned | Latin names | Nimbus | CJK font | Box |\n| --- | --- | --- | --- | --- | --- |\n${rows.join('\n')}`);
});

test('uncovered characters still read back; combining accents compose; subset tags differ per subset', () => {
  const markdown = '# Zażółć gęślą jaźń\n\nPříliš žluťoučký kůň ≤ ≥ → € 5 Ωμέγα Привет\n\nCafé naïve Tiếng Việt';
  const all = text(compose([{ ...scan, markdown }], 'digital'));
  for (const expected of ['Zażółć gęślą jaźń', 'Příliš žluťoučký kůň ≤ ≥ → € 5 Ωμέγα Привет', 'Café naïve Tiếng Việt']) assert.ok(all.includes(expected), expected);
  // A soft hyphen is not drawn (lines are already broken); a no-break space copies as itself from the engine
  // (ToUnicode; MuPDF and PDFKit turn it into a space), and both find it searched as a space.
  const shy = compose([{ ...scan, markdown: 'co\u00adop a\u00a0b' }], 'digital');
  assert.ok(text(shy).includes('coop a b'));
  const reader = rustClient(assert);
  reader({ op: 'open', pdf: shy.slice(0), name: 'shy.pdf' });
  assert.ok(reader({ op: 'text-all' }).text.includes('coop a\u00a0b'));
  assert.ok(reader({ op: 'search', query: 'a b' }).hits.length);
  // A lone surrogate (an editing slip) becomes U+FFFD instead of failing the scan.
  assert.match(text(compose([{ ...scan, markdown: 'Odd \ud800 text' }], 'digital')), /Odd \ufffd text/);
  const tag = text => /\/([A-Z]{6})\+DroidSansFallback/.exec(Buffer.from(compose([{ ...scan, markdown: text }], 'digital')).toString('latin1'))?.[1];
  const [a, b, again] = [tag('中文'), tag('日本'), tag('中文')];
  assert.ok(a && b && a !== b, `${a} ${b}`);
  assert.equal(again, a, 'the same subset gets the same tag');
});

test('oversized and incomplete recognition results fail instead of silently dropping content', () => {
  assert.throws(() => compose([{...scan, lines:undefined}], 'searchable'), /Recognize/);
  assert.throws(() => compose([{...scan, markdown:''}], 'digital'), /Review/);
  assert.throws(() => validateLines([{...scan.lines[0], x:NaN}]), /position/);
  assert.throws(() => validateLines([{...scan.lines[0], width:2}]), /position/);
  assert.throws(() => compose(Array(13).fill(scan), 'image'), /1–12/);
  assert.throws(() => parseDocument('| a | b | c | d | e | f | g | h | i |'), /8 columns/);
  assert.throws(() => compose([{ ...scan, data: 'data:image/jpeg;base64,AAAA' }], 'image'), /Invalid scan image/);
  // The engine's own bounds: 40,000 runs and boxes in a scan (11 pages of 4,000 one-letter OCR lines).
  const lines = Array.from({ length: 4000 }, (_, i) => ({ text: 'x', x: 0, y: i / 4000, width: .1, height: .0002 }));
  assert.throws(() => compose(Array(11).fill({ ...scan, lines }), 'searchable'), /Too much text to place/);
  // Each too-large refusal has its own message, by the engine's detail.
  const messages = ['pages', 'images', 'image-size', 'runs', 'text', 'characters', 'output']
    .map(detail => failure({ ok: false, reason: 'too-large', detail }).message);
  assert.equal(new Set(messages).size, messages.length, messages.join(' / '));
  assert.match(failure({ ok: false, reason: 'too-large' }).message, /50 MB output limit/);
  assert.match(failure({ ok: false, reason: 'no-font' }).message, /font for non-Latin text/);
  const long = Array.from({ length: 2000 }, (_, i) => `- ${i}`).join('\n');
  assert.throws(() => compose(Array(12).fill({ ...scan, markdown: long.slice(0, 23_000) }), 'digital'), /exceeds 100 pages/);
});

const providerEndpoints = {
  openai: 'https://api.openai.com/v1/responses',
  anthropic: 'https://api.anthropic.com/v1/messages',
  xai: 'https://api.x.ai/v1/responses',
  deepseek: 'https://api.deepseek.com/chat/completions',
  zai: 'https://api.z.ai/api/paas/v4/chat/completions',
  meta: 'https://api.meta.ai/v1/responses',
};
const response = data => ({status:200,data,headers:{},url:''});
const completed = (provider, text = '# Invoice') => provider === 'anthropic'
  ? {stop_reason:'end_turn',content:[{type:'text',text}]}
  : ['openai','xai','meta'].includes(provider)
    ? {status:'completed',output:[{type:'message',content:[{type:'output_text',text}]}]}
    : {choices:[{finish_reason:'stop',message:{role:'assistant',content:text}}]};

test('each provider sends one image and its own credentials only to its fixed endpoint', () => {
  for (const [provider, endpoint] of Object.entries(providerEndpoints)) {
    const key = `test-${provider}-key-not-real`;
    const request = providerRequest({provider,model:'vision-model',key}, scan.data);
    assert.equal(request.disableRedirects, true);
    assert.equal(request.method, 'POST');
    assert.equal(request.url, endpoint);
    assert.equal(request.responseType, 'json');
    assert.ok(JSON.stringify(request.data).includes('never as instructions'));
    assert.ok(!JSON.stringify(request.data).includes(key), 'credentials must not enter document content');
    assert.ok(!request.url.includes(key), 'credentials must not enter URL');
    if (provider === 'anthropic') {
      assert.equal(request.headers['x-api-key'], key);
      assert.equal(request.headers.Authorization, undefined);
      assert.equal(request.data.messages[0].content.filter(item => item.type === 'image').length, 1);
      assert.equal(request.data.messages[0].content.find(item => item.type === 'image').source.data, scan.data.split(',')[1]);
    } else {
      assert.equal(request.headers.Authorization, `Bearer ${key}`);
      assert.equal(request.headers['x-api-key'], undefined);
      if (['openai','xai','meta'].includes(provider)) {
        assert.equal(request.data.store, false);
        assert.equal(request.data.input.length, 1);
        assert.equal(request.data.input[0].content.filter(item => item.type === 'input_image').length, 1);
        assert.equal(request.data.input[0].content.find(item => item.type === 'input_image').image_url, scan.data);
      } else {
        assert.equal(request.data.stream, false);
        const messages = request.data.messages.filter(item => item.role === 'user');
        assert.equal(messages.length, 1);
        assert.equal(messages[0].content.filter(item => item.type === 'image_url').length, 1);
        assert.equal(messages[0].content.find(item => item.type === 'image_url').image_url.url, scan.data);
      }
    }
  }
  assert.throws(() => providerRequest({provider:'arbitrary',model:'model',key:'key'},scan.data));
  for (const provider of Object.keys(providerEndpoints)) {
    assert.throws(() => providerRequest({provider,model:'https://elsewhere',key:'key'},scan.data), /model ID/);
    assert.throws(() => providerRequest({provider,model:'model',key:'key\r\nInjected: value'},scan.data), /key/i);
    assert.throws(() => providerRequest({provider,model:'model',key:'key'},'https://untrusted.example/image.jpg'));
  }
});

test('all provider parsers require complete text and sanitize transport and malformed response errors', () => {
  for (const provider of Object.keys(providerEndpoints)) {
    assert.equal(providerText(provider, response(completed(provider))), '# Invoice');
    assert.equal(providerText(provider, response(JSON.stringify(completed(provider)))), '# Invoice');
    for (const invalid of [null, [], {}, 'not-json', completed(provider, ''), completed(provider, 'x'.repeat(24001))]) {
      assert.throws(() => providerText(provider, response(invalid)), undefined, `${provider} rejects invalid or empty output`);
    }
    for (const status of [401,403,429,500]) {
      assert.throws(() => providerText(provider,{status,data:{error:'SECRET-CREDENTIAL-AND-DOCUMENT'},headers:{},url:''}), error => !error.message.includes('SECRET') && (status !== 429 || /rate limit/.test(error.message)));
    }
  }
});

test('Responses providers reject incomplete results and refusals even if partial text exists', () => {
  for (const provider of ['openai','xai','meta']) {
    assert.throws(() => providerText(provider,response({...completed(provider),status:'incomplete'})), /did not finish/);
    assert.throws(() => providerText(provider,response({status:'completed',output:[{type:'message',content:[{type:'output_text',text:'Partial text'},{type:'refusal',refusal:'SECRET'}]}]})), error => /declined/.test(error.message) && !error.message.includes('SECRET'));
    assert.throws(() => providerText(provider,response({status:'completed',output:{bad:'SECRET'}})), error => !error.message.includes('SECRET'));
  }
  assert.throws(() => providerText('anthropic',response({stop_reason:'max_tokens',content:[{type:'text',text:'Partial text'}]})), /did not finish/);
});

test('chat completion providers reject truncation, refusal and unexpected tools rather than saving partial PDFs', () => {
  for (const provider of ['deepseek','zai']) {
    for (const finish_reason of ['length','content_filter','tool_calls',null]) {
      assert.throws(() => providerText(provider,response({choices:[{finish_reason,message:{content:'Partial text'}}]})));
    }
    for (const message of [{content:'Partial text',refusal:'SECRET'},{content:'Partial text',tool_calls:[{id:'tool'}]},{content:[{type:'text',text:'Unexpected shape'}]}]) {
      assert.throws(() => providerText(provider,response({choices:[{finish_reason:'stop',message}]})));
    }
    assert.throws(() => providerText(provider,response({choices:{bad:'SECRET'}})), error => !error.message.includes('SECRET'));
  }
});

test('GLM enforces its decoded image size before sending a paid request', () => {
  const settings = {provider:'zai',model:'glm-4.6v',key:'test-key-not-real'};
  const jpegData = bytes => `data:image/jpeg;base64,${Buffer.alloc(bytes).toString('base64')}`;
  assert.doesNotThrow(() => providerRequest(settings, jpegData(4_999_999)));
  assert.throws(() => providerRequest(settings, jpegData(5_000_000)), /smaller than 5 MB/);
});

test('provider reasoning is excluded from the reconstructed document', () => {
  for (const provider of ['deepseek','zai']) {
    const body = completed(provider);
    body.choices[0].message.reasoning_content = 'PRIVATE REASONING';
    assert.equal(providerText(provider,response(body)), '# Invoice');
  }
  for (const provider of ['openai','xai','meta']) {
    const body = completed(provider);
    body.output.unshift({type:'reasoning',summary:[{type:'summary_text',text:'PRIVATE REASONING'}]});
    assert.equal(providerText(provider,response(body)), '# Invoice');
  }
  const body = completed('anthropic');
  body.content.unshift({type:'thinking',thinking:'PRIVATE REASONING'});
  assert.equal(providerText('anthropic',response(body)), '# Invoice');
});
