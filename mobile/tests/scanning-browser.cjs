// Browser UI + real PDF worker; the phone OCR and paid provider boundaries are mocked.
const { app, BrowserWindow } = require('electron');
const { mkdtempSync, writeFileSync } = require('node:fs');
const { tmpdir } = require('node:os');
const { join } = require('node:path');
const assert = require('node:assert/strict');
app.setPath('userData', mkdtempSync(join(tmpdir(), 'tandem-scan-ui-')));
app.whenReady().then(async () => {
  const window = new BrowserWindow({ width: 390, height: 844, show: false, webPreferences: { contextIsolation: true, nodeIntegration: false, sandbox: true } });
  const web = window.webContents, js = code => web.executeJavaScript(code);
  let step = 'startup';
  const watchdog = setTimeout(() => { console.error('Timed out at', step); app.exit(1); }, 180_000);
  web.session.webRequest.onBeforeRequest((details, callback) => callback({ cancel: /^https?:/.test(details.url) && !details.url.startsWith('http://127.0.0.1:4173/') }));
  const errors = []; web.on('console-message', event => { if (event.level === 'error') errors.push(event.message); });
  async function wait(expression) {
    for (let i = 0; i < 250; i++) { if (await js(expression)) return; await new Promise(r => setTimeout(r, 100)); }
    throw new Error(`Timed out: ${expression}\n${await js('document.body.innerText')}`);
  }
  const idle = () => wait(`document.getElementById('scan-dialog').getAttribute('aria-busy') !== 'true' && !document.body.classList.contains('busy')`);
  async function click(selector) { step = selector; await js(`document.querySelector(${JSON.stringify(selector)}).click()`); await idle(); }
  async function select(value) { await js(`document.getElementById('scan-output').value=${JSON.stringify(value)}; document.getElementById('scan-output').dispatchEvent(new Event('change'))`); }
  async function connect(provider, model = 'vision-test') {
    await click('#scan-ai-connect');
    await js(`document.getElementById('ai-key').value='old-provider-key-not-real'; document.getElementById('ai-provider').value=${JSON.stringify(provider)}; document.getElementById('ai-provider').dispatchEvent(new Event('change'))`);
    if (provider === 'openai') await js(`document.getElementById('ai-openai-method').value='api';document.getElementById('ai-openai-method').dispatchEvent(new Event('change'))`);
    assert.equal(await js(`document.getElementById('ai-key').value`), '', 'changing provider clears the previous credential');
    assert.equal(await js(`document.getElementById('ai-model').value`), '', 'changing provider clears the previous model');
    if (provider === 'zai') {
      await js(`new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(resolve, 50))))`);
      writeFileSync(join(tmpdir(), 'tandem-multi-provider-settings.png'), (await web.capturePage()).toPNG());
    }
    assert.equal(await js(`document.getElementById('scan-consent').checked`), false);
    await js(`document.getElementById('ai-model').value=${JSON.stringify(model)}; document.getElementById('ai-key').value=${JSON.stringify(`test-${provider}-key-not-real`)}; document.getElementById('ai-form').requestSubmit()`);
    assert.equal(await js(`document.getElementById('ai-key').value`), '');
    assert.equal(await js(`document.getElementById('scan-consent').checked`), false);
  }
  try {
    await window.loadURL('about:blank');
    web.debugger.attach('1.3');
    await web.debugger.sendCommand('Page.enable');
    await web.debugger.sendCommand('Page.addScriptToEvaluateOnNewDocument', { source: `
      window.webkit={messageHandlers:{bridge:{}}};
      window.requestLog=[]; window.failSecond=false; window.ocrCount=0;
      window.Capacitor={PluginHeaders:[{name:'DocumentScanner',methods:[{name:'recognize',rtype:'promise'}]},{name:'CapacitorHttp',methods:[{name:'request',rtype:'promise'}]},{name:'ChatGPTAccount',methods:[{name:'getAccounts',rtype:'promise'}]}],nativePromise:async(plugin,method,options)=>{
        if(plugin==='ChatGPTAccount' && method==='getAccounts')return {accounts:[]};
        if(plugin==='DocumentScanner') { window.ocrCount++; return {lines:[{text:'Recognized page '+window.ocrCount,x:.1,y:.1,width:.6,height:.03},{text:'Amount 12.50',x:.1,y:.2,width:.4,height:.03}]}; }
        if(plugin==='CapacitorHttp') {
          window.requestLog.push({url:options.url,data:options.data,headers:options.headers});
          if(window.failSecond && window.requestLog.length===2) return {status:429,data:{error:'rate limited'}};
          const markdown='# Invoice '+window.requestLog.length+'\\n\\n| Item | Total |\\n| --- | --- |\\n| Paper | 12.50 |\\n\\n[unclear] reference';
          const data = options.url.endsWith('/responses')
            ? {status:'completed',output:[{type:'message',content:[{type:'output_text',text:markdown}]}]}
            : options.url.includes('anthropic')
              ? {stop_reason:'end_turn',content:[{type:'text',text:markdown}]}
              : {choices:[{finish_reason:'stop',message:{role:'assistant',content:markdown}}]};
          return {status:200,data};
        }
        throw new Error('Unexpected native call');
      }};
    ` });
    await window.loadURL('http://127.0.0.1:4173'); await idle();
    assert.deepEqual(await js(`Array.from(document.querySelectorAll('#ai-provider option')).map(option => option.value)`), ['openai','anthropic','xai','meta','deepseek','zai']);
    await click('#scan'); await click('#scan-sample');
    await click('#scan-rotate'); await click('#scan-rotate'); await click('#scan-rotate'); await click('#scan-rotate');
    await click('#scan-crop');
    await js(`(() => { const c=document.getElementById('crop-canvas'),r=c.getBoundingClientRect(); c.setPointerCapture=()=>{}; c.hasPointerCapture=()=>true; for (const [type,x,y] of [['pointerdown',.02,.02],['pointermove',.98,.98]]) c.dispatchEvent(new PointerEvent(type,{pointerId:1,clientX:r.left+r.width*x,clientY:r.top+r.height*y})); document.getElementById('crop-apply').click(); })()`);
    await idle(); assert.equal(await js(`document.getElementById('crop-dialog').open`), false);
    await select('image'); await click('#scan-save');
    assert.equal(await js(`document.getElementById('name').textContent`), 'Scanned document.pdf', await js(`document.getElementById('scan-status').textContent`));
    await click('#back'); await click('#scan'); await click('#scan-sample'); await click('#scan-sample');
    await select('searchable'); await click('#scan-process');
    assert.equal(await js(`window.ocrCount`), 2);
    await click('#scan-prev');
    await js(`const area=document.querySelector('#scan-text-fields textarea'); area.value='Corrected recognition'; area.dispatchEvent(new Event('input'))`);
    await click('#scan-save');
    assert.ok(await js(`document.getElementById('scan-status').textContent.includes('Review every page')`));
    await click('#scan-reviewed');
    await js(`void (()=>{window.realTransaction=IDBDatabase.prototype.transaction;IDBDatabase.prototype.transaction=function(...args){if(args[1]==='readwrite')throw new DOMException('Full','QuotaExceededError');return window.realTransaction.apply(this,args)}})()`);
    await click('#scan-save');
    assert.equal(await js(`document.getElementById('scan-dialog').open`), true, 'failed storage retains scan review');
    assert.equal(await js(`document.querySelector('#scan-text-fields textarea').value`), 'Corrected recognition');
    await js('void(IDBDatabase.prototype.transaction=window.realTransaction)');
    await click('#scan-save');
    assert.ok(await js(`document.getElementById('page-text').textContent.includes('Corrected recognition')`));
    await click('#menu'); await click('#copy-text');
    assert.ok(await js(`document.getElementById('copy-page-text').value.includes('Corrected recognition')`));
    await click('[data-close="copy-text-dialog"]'); await click('#back');
    // AI sends nothing before consent; a partial batch survives a rate limit and resumes.
    await click('#scan'); await click('#scan-sample'); await click('#scan-sample'); await select('digital'); await connect('openai');
    await click('#scan-process'); assert.equal(await js('window.requestLog.length'), 0);
    await click('#scan-consent'); await js('void(window.failSecond=true)'); await click('#scan-process');
    assert.equal(await js('window.requestLog.length'), 2);
    assert.ok(await js(`document.getElementById('scan-status').textContent.includes('rate limit')`));
    await click('#scan-process'); assert.equal(await js('window.requestLog.length'), 3, 'completed page is not billed again');
    await click('#scan-prev');
    await js(`const area2=document.querySelector('#scan-text-fields textarea');area2.value=area2.value.replace('[unclear] reference','Verified reference 7788');area2.dispatchEvent(new Event('input'))`);
    await js(`document.getElementById('scan-text-review').scrollIntoView({block:'start'})`);
    writeFileSync(join(tmpdir(), 'tandem-scan-ai-review.png'), (await web.capturePage()).toPNG());
    await click('#scan-reviewed'); await click('#scan-save');
    assert.ok(await js(`document.getElementById('page-text').textContent.includes('Verified reference 7788')`));
    await click('#back');
    assert.ok(await js(`Array.from(document.querySelectorAll('.file-open')).some(e=>e.textContent.includes('original scan.pdf'))`));
    await js(`Array.from(document.querySelectorAll('.file-open')).find(e=>e.textContent.includes('original scan.pdf')).click()`); await idle();
    assert.equal((await js(`document.getElementById('page-text').textContent`)).trim(), '');
    await click('#back');
    await click('#scan'); await click('#scan-sample'); await select('digital'); await connect('anthropic'); await click('#scan-consent'); await click('#scan-process');
    assert.equal(await js(`window.requestLog.at(-1).url`), 'https://api.anthropic.com/v1/messages');
    await click('#scan-reviewed'); await click('#scan-save');
    assert.ok(await js(`document.getElementById('page-text').textContent.includes('Invoice 4')`));
    assert.equal(await js(`window.requestLog.at(-1).headers['x-api-key']`), 'test-anthropic-key-not-real');
    // Every additional provider uses its selected endpoint and credential, requires fresh consent,
    // and exports the reviewed result through the same real PDF worker.
    for (const [provider, endpoint] of [
      ['xai','https://api.x.ai/v1/responses'],
      ['deepseek','https://api.deepseek.com/chat/completions'],
      ['zai','https://api.z.ai/api/paas/v4/chat/completions'],
      ['meta','https://api.meta.ai/v1/responses'],
    ]) {
      await click('#back'); await click('#scan'); await click('#scan-sample'); await select('digital');
      await click('#scan-consent');
      await connect(provider);
      if (provider === 'meta') assert.equal(await js(`/train/i.test(document.getElementById('scan-consent-text').textContent)`), false, 'standard Meta models do not show contributor training consent');
      const count = await js('window.requestLog.length');
      await click('#scan-process');
      assert.equal(await js('window.requestLog.length'), count, `${provider} requires new consent`);
      await click('#scan-consent'); await click('#scan-process');
      const request = await js('window.requestLog.at(-1)');
      assert.equal(request.url, endpoint);
      assert.equal(request.headers.Authorization, `Bearer test-${provider}-key-not-real`);
      assert.equal(request.headers['x-api-key'], undefined);
      await click('#scan-save');
      assert.ok(await js(`document.getElementById('scan-status').textContent.includes('Review every page')`));
      await click('#scan-reviewed'); await click('#scan-save');
      assert.ok(await js(`document.getElementById('page-text').textContent.includes(${JSON.stringify(`Invoice ${count + 1}`)})`));
    }
    await click('#back'); await click('#scan'); await click('#scan-sample'); await select('digital');
    await click('#scan-consent'); await click('#scan-ai-connect'); await click('#ai-disconnect');
    assert.equal(await js(`document.getElementById('scan-consent').checked`), false, 'disconnect clears upload consent');
    const disconnectedCount = await js('window.requestLog.length');
    await click('#scan-process');
    assert.equal(await js('window.requestLog.length'), disconnectedCount, 'disconnected key cannot send a request');
    assert.equal(await js(`localStorage.length`), 0, 'API keys never persisted in localStorage');
    assert.equal(await js(`sessionStorage.length`), 0, 'API keys never persisted in sessionStorage');
    const savedDocuments = await js(`new Promise((resolve,reject) => {
      const request=indexedDB.open('tandem-mobile');
      request.onsuccess=()=>{const db=request.result,tx=db.transaction('documents'),all=tx.objectStore('documents').getAll();
        all.onsuccess=()=>resolve(all.result.map(doc=>({...doc,bytes:new TextDecoder().decode(doc.bytes)})));
        tx.oncomplete=()=>db.close();tx.onerror=()=>reject(tx.error);};request.onerror=()=>reject(request.error);
    })`);
    assert.ok(!JSON.stringify(savedDocuments).includes('-key-not-real'), 'API keys never enter stored documents or PDFs');
    await connect('meta', 'muse-spark-1.3-contributor');
    assert.equal(await js(`document.getElementById('scan-consent').checked`), false, 'contributor model requires fresh consent');
    assert.ok(await js(`/train/i.test(document.getElementById('scan-consent-text').textContent)`), 'Meta contributor models disclose training before upload');
    assert.ok(await js(`document.getElementById('scan-ai-state').textContent.includes('connected for this session')`));
    await select('image'); await click('#scan-save');
    assert.equal(await js('window.requestLog.length'), disconnectedCount, 'saving the image-only PDF makes no AI request');
    await new Promise(resolve => { web.once('did-finish-load', resolve); window.reload(); }); await idle();
    await click('#scan'); await click('#scan-sample'); await select('digital'); await click('#scan-process');
    assert.equal(await js('window.requestLog.length'), 0, 'reloading drops the session credential');
    assert.equal(await js(`document.getElementById('scan-consent').disabled`), true, 'reload requires a new AI connection');
    assert.ok(await js(`document.getElementById('scan-status').textContent.includes('Connect')`));
    assert.equal(errors.length, 0, errors.join('\n'));
    console.log('PASS: crop/rotate, image PDF, OCR correction, review gate, storage recovery, six-provider consent and credential isolation, disconnect/reload, partial-batch retry, typeset PDF, original preservation. Paid APIs and native OCR mocked.');
    clearTimeout(watchdog); app.exit(0);
  } catch(error) { console.error(error, errors); app.exit(1); }
});
