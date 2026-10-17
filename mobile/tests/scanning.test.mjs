import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { build } from 'esbuild';
import mupdf from 'mupdf';
import { readCjkFont } from '../../scripts/cjk-font.mjs';
import { rustClient, rustEngine } from './rust.mjs';

// The scanner composes its PDFs itself and checks them with the Rust engine the app ships (scan-worker.ts).
// MuPDF reads them here only as an independent oracle; it is not part of the mobile app.
async function module(path) {
  const result = await build({ entryPoints: [path], bundle: true, format: 'esm', write: false });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}`);
}
const { createScanPDF, checkScanPDF, platformDeflate } = await module('src/scan-pdf.ts');
const { providerRequest, providerText } = await module('src/scan-ai.ts');
const { parseTrueType } = await module('src/scan-font.ts');
const { winAnsiByte } = await module('../src/renderer/engine-core.ts');
const { validateLines, parseDocument } = await module('src/scan-model.ts');
let fontReads = 0;
const host = { fallbackFont: async () => { fontReads++; return readCjkFont(); }, deflate: platformDeflate() };
const compose = async (pages, output, using = host) => (await createScanPDF(pages, output, using)).bytes;

// The bundled sample page as a JPEG like the scanner's canvas output (the scanner's input is always JPEG),
// rendered at 2x and encoded by the oracle.
const welcome = mupdf.Document.openDocument(await readFile('dist/welcome.pdf'), 'application/pdf');
const pixmap = welcome.loadPage(0).toPixmap(mupdf.Matrix.scale(2, 2), mupdf.ColorSpace.DeviceRGB, false);
const jpeg = pixmap.asJPEG(90);
const scan = { id: 'scan', data: `data:image/jpeg;base64,${Buffer.from(jpeg).toString('base64')}`, width: pixmap.getWidth(), height: pixmap.getHeight(),
  lines: [{ text: 'Hello scan résumé 中文', x: .1, y: .2, width: .5, height: .03 }] };
const latin = [{ ...scan, lines: [{ text: 'Hello scan résumé', x: .1, y: .2, width: .5, height: .03 }] }];
const open = bytes => mupdf.Document.openDocument(new Uint8Array(bytes), 'application/pdf');

test('searchable scan retains identical pixels and searchable Unicode text after reopen', async () => {
  const imageDoc = open(await compose([scan], 'image'));
  const textBytes = await compose([scan], 'searchable');
  const textDoc = open(textBytes);
  const imagePage = imageDoc.loadPage(0), textPage = textDoc.loadPage(0);
  const a = imagePage.toPixmap(mupdf.Matrix.identity, mupdf.ColorSpace.DeviceRGB, false);
  const b = textPage.toPixmap(mupdf.Matrix.identity, mupdf.ColorSpace.DeviceRGB, false);
  assert.deepEqual(a.getPixels(), b.getPixels(), 'OCR must not change scan appearance');
  const text = textPage.toStructuredText();
  assert.match(text.asText(), /Hello scan résumé 中文/);
  assert.ok(textPage.search('résumé').length);
  const line = JSON.parse(text.asJSON()).blocks.find(block => block.type === 'text').lines[0];
  assert.ok(line.bbox.x >= 60 && line.bbox.x < 63, JSON.stringify(line));
  // The OCR line spans the recognized width (0.5 of the page): the run is scaled to fit.
  assert.ok(Math.abs(line.bbox.w - 306) < 4, JSON.stringify(line.bbox));
  text.destroy(); a.destroy(); b.destroy(); imagePage.destroy(); textPage.destroy(); imageDoc.destroy(); textDoc.destroy();
  // The Rust engine reads the same text and draws the page the same with or without it.
  const send = rustClient(assert);
  send({ op: 'open', pdf: textBytes.slice(0), name: 'scan.pdf' });
  assert.match(send({ op: 'text-all' }).text, /Hello scan résumé 中文/);
  assert.ok(send({ op: 'search', query: 'résumé' }).hits.length);
  assert.ok(send({ op: 'search', query: '中文' }).hits.length);
  const withText = new Uint8Array(send({ op: 'render', page: 0, scale: 1 }).pixels);
  send({ op: 'open', pdf: (await compose([scan], 'image')).slice(0), name: 'image.pdf' });
  assert.deepEqual(new Uint8Array(send({ op: 'render', page: 0, scale: 1 }).pixels), withText, 'OCR must not change scan appearance (Rust)');
  send({ op: 'close' });
});

test('digital reconstruction produces real text, tables and overflow pages without losing the tail', async () => {
  const content = '# Project notes\n\nRésumé 中文\n\n| Item | Quantity |\n| --- | --- |\n| Paper | 17 |\n\n' + Array.from({length:150}, (_,i) => `- Line ${i} carries exact words and figures 123.45.`).join('\n') + '\n\nEND-OF-DOCUMENT';
  const { bytes, pageCount } = await createScanPDF([{...scan, markdown:content}], 'digital', host);
  const doc = open(bytes);
  assert.ok(doc.countPages() > 2);
  assert.equal(doc.countPages(), pageCount);
  let text = '';
  for (let i = 0; i < doc.countPages(); i++) { const page = doc.loadPage(i), st = page.toStructuredText(); text += st.asText(); st.destroy(); page.destroy(); }
  for (const expected of ['Project notes', 'Résumé 中文', 'Quantity', 'Paper', '17', 'Line 149', 'END-OF-DOCUMENT']) assert.ok(text.includes(expected), expected);
  doc.destroy();
  const send = rustClient(assert);
  assert.equal(send({ op: 'open', pdf: bytes.slice(0), name: 'rebuilt.pdf' }).pageCount, pageCount);
  const rust = send({ op: 'text-all' }).text;
  for (const expected of ['Project notes', 'Résumé 中文', 'Quantity', 'Paper', 'Line 149', 'END-OF-DOCUMENT']) assert.ok(rust.includes(expected), expected);
  send({ op: 'close' });
});

test('scan output keeps the MuPDF scanner page size, untouched JPEG, embedded profiles and multi-page order', async () => {
  const wide = { ...scan, id: 'wide', width: scan.height, height: scan.width };
  const { bytes, pageCount } = await createScanPDF([scan, scan, scan], 'image', host);
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
  const mixedBytes = await compose([scan, { ...scan, data: scan.data, width: wide.width, height: wide.height }], 'image');
  const mixed = open(mixedBytes);
  assert.deepEqual(mixed.loadPage(1).getBounds().map(Math.round), [0, 0, 792, 612]);
  mixed.destroy();
  // A profile the JPEG carries (APP2 ICC_PROFILE chunks, here split in two) becomes its ICCBased colour space.
  const profile = Buffer.from('a stand-in ICC profile, split across two chunks');
  const app2 = (part, count, data) => Buffer.concat([Buffer.from([0xff, 0xe2, 0, 16 + data.length]), Buffer.from('ICC_PROFILE\0'), Buffer.from([part, count]), data]);
  const tagged = Buffer.concat([jpeg.subarray(0, 2), app2(2, 2, profile.subarray(20)), app2(1, 2, profile.subarray(0, 20)), jpeg.subarray(2)]);
  const icc = open(await compose([{ ...scan, data: `data:image/jpeg;base64,${tagged.toString('base64')}` }], 'image'));
  const space = icc.asPDF().findPage(0).get('Resources').get('XObject').get('Im0').get('ColorSpace');
  assert.equal(space.get(0).asName(), 'ICCBased');
  assert.equal(space.get(1).get('N').asNumber(), 3);
  assert.deepEqual(Buffer.from(space.get(1).readStream().asUint8Array()), profile);
  icc.destroy();
  // The engine the app ships accepts what it composes.
  checkScanPDF(rustEngine(), bytes, 3);
  assert.throws(() => checkScanPDF(rustEngine(), bytes, 2), /did not pass validation/);
  assert.throws(() => checkScanPDF(rustEngine(), bytes.slice(0, bytes.byteLength - 200), 3), /did not pass validation/);
});

test('only text outside WinAnsi reads the fallback font, and only the glyphs used are embedded', async () => {
  fontReads = 0;
  const plain = await compose(latin, 'searchable');
  assert.equal(fontReads, 0, 'Latin text needs no fallback font');
  assert.ok(plain.byteLength < jpeg.length + 12_000, `${plain.byteLength} bytes`);
  const cjk = await compose([scan], 'searchable');
  assert.equal(fontReads, 1);
  // MuPDF embedded its whole fallback font (about 1.7 MB) for the same page; the subset holds 3 glyphs.
  assert.ok(cjk.byteLength < jpeg.length + 40_000, `${cjk.byteLength} bytes`);
  // A character the fallback font lacks still reads back (ToUnicode), drawn as the font's .notdef.
  const rare = await compose([{ ...scan, lines: [{ text: 'Rare 𝔘 ✓ ภาษา', x: .1, y: .2, width: .5, height: .03 }] }], 'searchable');
  const doc = open(rare), page = doc.loadPage(0), st = page.toStructuredText();
  assert.match(st.asText(), /Rare 𝔘 ✓ ภาษา/);
  st.destroy(); page.destroy(); doc.destroy();
  checkScanPDF(rustEngine(), rare, 1);
  // The host may hand over the same font bytes every time (a Node Buffer here): composing leaves them intact.
  const font = readCjkFont(), before = Buffer.from(font);
  const shared = { fallbackFont: async () => font, deflate: host.deflate };
  assert.deepEqual(Buffer.from(await compose([scan], 'searchable', shared)), Buffer.from(await compose([scan], 'searchable', shared)));
  assert.ok(before.equals(font), 'the fallback font bytes are unchanged');
});

// Which characters outside WinAnsi the bundled fallback font draws, by Unicode block. Helvetica covers WinAnsi;
// everything else goes to Droid Sans Fallback, and what that lacks draws as its missing-glyph box (the text still
// extracts). MuPDF drew the Latin, Greek, Cyrillic, arrow, maths and currency rows with its embedded Nimbus Sans:
// no font file with that coverage ships in the Rust build (engine/design/app-m9-mobile-release.md, owner decision).
test('fallback font coverage per script (outside WinAnsi): CJK and Hangul yes, Latin Extended, Greek, Cyrillic, symbols no', () => {
  const font = parseTrueType(readCjkFont());
  const blocks = {
    'Latin Extended-A': [0x100, 0x17f, 0], 'Latin Extended-B': [0x180, 0x24f, 0], 'IPA': [0x250, 0x2af, 0],
    'Combining marks': [0x300, 0x36f, 0], 'Greek': [0x370, 0x3ff, 0], 'Cyrillic': [0x400, 0x4ff, 0],
    'Hebrew': [0x590, 0x5ff, 0], 'Arabic': [0x600, 0x6ff, 0], 'Thai': [0xe00, 0xe7f, 1],
    'Latin Extended Additional (Vietnamese)': [0x1e00, 0x1eff, 0], 'General Punctuation': [0x2000, 0x206f, 4],
    'Currency': [0x20a0, 0x20cf, 0], 'Letterlike': [0x2100, 0x214f, 0], 'Arrows': [0x2190, 0x21ff, 0],
    'Math Operators': [0x2200, 0x22ff, 0], 'Box Drawing': [0x2500, 0x257f, 0], 'Misc Symbols': [0x2600, 0x26ff, 0],
    'CJK Symbols': [0x3000, 0x303f, 36], 'Hiragana': [0x3040, 0x309f, 90], 'Katakana': [0x30a0, 0x30ff, 91],
    'CJK Unified (BMP)': [0x4e00, 0x9fff, 20902], 'Hangul Syllables': [0xac00, 0xd7a3, 11172], 'Fullwidth forms': [0xff00, 0xffef, 164],
    'Emoji': [0x1f300, 0x1faff, 0],
  };
  const rows = [];
  for (const [name, [from, to, expected]] of Object.entries(blocks)) {
    let assigned = 0, drawn = 0;
    for (let code = from; code <= to; code++) {
      const char = String.fromCodePoint(code);
      if (!/\p{Assigned}/u.test(char) || /\p{Cc}/u.test(char) || winAnsiByte(code) >= 0) continue;
      assigned++;
      if (font.glyph(code)) drawn++;
    }
    rows.push(`${name}: ${drawn} of ${assigned}`);
    assert.equal(drawn, expected, name);
  }
  console.log(`fallback font coverage outside WinAnsi:\n  ${rows.join('\n  ')}`);
});

test('uncovered characters still read back; combining accents compose; subset tags differ per subset', async () => {
  const markdown = '# Zażółć gęślą jaźń\n\nPříliš žluťoučký kůň ≤ ≥ → € 5\n\nCafe\u0301 nai\u0308ve';
  const { bytes } = await createScanPDF([{ ...scan, markdown }], 'digital', host);
  const doc = open(bytes), page = doc.loadPage(0), st = page.toStructuredText();
  const text = st.asText();
  for (const expected of ['Zażółć gęślą jaźń', 'Příliš žluťoučký kůň ≤ ≥ →', 'Café naïve']) assert.ok(text.includes(expected), expected);
  st.destroy(); page.destroy(); doc.destroy();
  // NFC: é and ï are WinAnsi once composed, so Latin text with separate accents needs no fallback font.
  fontReads = 0;
  await compose([{ ...scan, markdown: 'Cafe\u0301 nai\u0308ve' }], 'digital');
  assert.equal(fontReads, 0);
  const tag = async text => /\/([A-Z]{6})\+DroidSansFallback/.exec(Buffer.from(await compose([{ ...scan, markdown: text }], 'digital')).toString('latin1'))?.[1];
  const [a, b, again] = [await tag('中文'), await tag('日本'), await tag('中文')];
  assert.ok(a && b && a !== b, `${a} ${b}`);
  assert.equal(again, a, 'the same subset gets the same tag');
});

test('without CompressionStream (Safari 15 to 16.3) the scan is written uncompressed and still checks', async () => {
  const content = '# Notes\n\nRésumé 中文\n\n| A | B |\n| --- | --- |\n| 1 | 2 |';
  const { bytes, pageCount } = await createScanPDF([{ ...scan, markdown: content }], 'digital', { fallbackFont: host.fallbackFont });
  assert.ok(!Buffer.from(bytes).includes('FlateDecode'));
  checkScanPDF(rustEngine(), bytes, pageCount);
  const doc = open(bytes), page = doc.loadPage(0), st = page.toStructuredText();
  assert.match(st.asText(), /Résumé 中文/);
  st.destroy(); page.destroy(); doc.destroy();
});

test('oversized and incomplete recognition results fail instead of silently dropping content', async () => {
  await assert.rejects(compose([{...scan, lines:undefined}], 'searchable'), /Recognize/);
  await assert.rejects(compose([{...scan, markdown:''}], 'digital'), /Review/);
  assert.throws(() => validateLines([{...scan.lines[0], x:NaN}]), /position/);
  assert.throws(() => validateLines([{...scan.lines[0], width:2}]), /position/);
  await assert.rejects(compose(Array(13).fill(scan), 'image'), /1–12/);
  assert.throws(() => parseDocument('| a | b | c | d | e | f | g | h | i |'), /8 columns/);
  await assert.rejects(compose([{ ...scan, data: 'data:image/jpeg;base64,AAAA' }], 'image'), /Invalid scan image/);
  const long = Array.from({ length: 2000 }, (_, i) => `- ${i}`).join('\n');
  await assert.rejects(compose(Array(12).fill({ ...scan, markdown: long.slice(0, 23_000) }), 'digital'), /exceeds 100 pages/);
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
