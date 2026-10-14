import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { build } from 'esbuild';
import mupdf from 'mupdf';

async function module(path) {
  const result = await build({ entryPoints: [path], bundle: true, format: 'esm', write: false });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}`);
}
const { createScanPDF } = await module('src/scan-pdf.ts');
const { providerRequest, providerText } = await module('src/scan-ai.ts');
const { validateLines, parseDocument } = await module('src/scan-model.ts');
const scan = { id: 'scan', data: `data:image/jpeg;base64,${(await readFile('dist/sample-scan.jpg')).toString('base64')}`, width: 1224, height: 1584,
  lines: [{ text: 'Hello scan résumé 中文', x: .1, y: .2, width: .5, height: .03 }] };

test('searchable scan retains identical pixels and searchable Unicode text after reopen', () => {
  const imageDoc = mupdf.Document.openDocument(createScanPDF(mupdf, [scan], 'image'), 'pdf');
  const textDoc = mupdf.Document.openDocument(createScanPDF(mupdf, [scan], 'searchable'), 'pdf');
  const imagePage = imageDoc.loadPage(0), textPage = textDoc.loadPage(0);
  const a = imagePage.toPixmap(mupdf.Matrix.identity, mupdf.ColorSpace.DeviceRGB, false);
  const b = textPage.toPixmap(mupdf.Matrix.identity, mupdf.ColorSpace.DeviceRGB, false);
  assert.deepEqual(a.getPixels(), b.getPixels(), 'OCR must not change scan appearance');
  const text = textPage.toStructuredText();
  assert.match(text.asText(), /Hello scan résumé 中文/);
  assert.ok(textPage.search('résumé').length);
  const line = JSON.parse(text.asJSON()).blocks.find(block => block.type === 'text').lines[0];
  assert.ok(line.bbox.x >= 60 && line.bbox.x < 63, JSON.stringify(line));
  text.destroy(); a.destroy(); b.destroy(); imagePage.destroy(); textPage.destroy(); imageDoc.destroy(); textDoc.destroy();
});

test('digital reconstruction produces real text, tables and overflow pages without losing the tail', () => {
  const content = '# Project notes\n\nRésumé 中文\n\n| Item | Quantity |\n| --- | --- |\n| Paper | 17 |\n\n' + Array.from({length:150}, (_,i) => `- Line ${i} carries exact words and figures 123.45.`).join('\n') + '\n\nEND-OF-DOCUMENT';
  const doc = mupdf.Document.openDocument(createScanPDF(mupdf, [{...scan, markdown:content}], 'digital'), 'pdf');
  assert.ok(doc.countPages() > 2);
  let text = '';
  for (let i = 0; i < doc.countPages(); i++) { const page = doc.loadPage(i), st = page.toStructuredText(); text += st.asText(); st.destroy(); page.destroy(); }
  for (const expected of ['Project notes', 'Résumé 中文', 'Quantity', 'Paper', '17', 'Line 149', 'END-OF-DOCUMENT']) assert.ok(text.includes(expected), expected);
  doc.destroy();
});

test('oversized and incomplete recognition results fail instead of silently dropping content', () => {
  assert.throws(() => createScanPDF(mupdf, [{...scan, lines:undefined}], 'searchable'), /Recognize/);
  assert.throws(() => createScanPDF(mupdf, [{...scan, markdown:''}], 'digital'), /Review/);
  assert.throws(() => validateLines([{...scan.lines[0], x:NaN}]), /position/);
  assert.throws(() => validateLines([{...scan.lines[0], width:2}]), /position/);
  assert.throws(() => createScanPDF(mupdf, Array(13).fill(scan), 'image'), /1–12/);
  assert.throws(() => parseDocument('| a | b | c | d | e | f | g | h | i |'), /8 columns/);
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
