// Real scan/PDF workflow with the native ChatGPT account boundary mocked.
// Never uses a real account, OAuth token, or paid API request.
const { app, BrowserWindow } = require('electron');
const { mkdtempSync, writeFileSync } = require('node:fs');
const { tmpdir } = require('node:os');
const { join } = require('node:path');
const assert = require('node:assert/strict');
app.setPath('userData', mkdtempSync(join(tmpdir(), 'tandem-chatgpt-ui-')));
app.whenReady().then(async () => {
  const window = new BrowserWindow({ width: 390, height: 844, show: false, webPreferences: { contextIsolation: true, nodeIntegration: false, sandbox: true } });
  const web = window.webContents, js = code => web.executeJavaScript(code);
  let step = 'startup';
  const watchdog = setTimeout(() => { console.error('Timed out at', step); app.exit(1); }, 180_000);
  const errors = [];
  web.on('console-message', event => { if (event.level === 'error') errors.push(event.message); });
  web.session.webRequest.onBeforeRequest((details, callback) => callback({ cancel: /^https?:/.test(details.url) && !details.url.startsWith('http://127.0.0.1:4173/') }));
  async function wait(expression) {
    for (let i = 0; i < 250; i++) { if (await js(expression)) return; await new Promise(resolve => setTimeout(resolve, 100)); }
    throw new Error(`Timed out at ${step}: ${expression}\n${await js('document.body.innerText')}`);
  }
  const idle = () => wait(`document.getElementById('scan-dialog').getAttribute('aria-busy') !== 'true' && !document.body.classList.contains('busy')`);
  async function click(selector) { step = selector; await js(`document.querySelector(${JSON.stringify(selector)}).click()`); await idle(); }
  async function select(selector, value) { await js(`document.querySelector(${JSON.stringify(selector)}).value=${JSON.stringify(value)};document.querySelector(${JSON.stringify(selector)}).dispatchEvent(new Event('change'))`); }
  const calls = method => js(`window.chatgptCalls.filter(call=>call.method===${JSON.stringify(method)})`);
  try {
    await window.loadURL('about:blank');
    web.debugger.attach('1.3'); await web.debugger.sendCommand('Page.enable');
    await web.debugger.sendCommand('Page.addScriptToEvaluateOnNewDocument', { source: `
      window.webkit={messageHandlers:{bridge:{}}};
      window.chatgptCalls=[];window.apiCalls=[];window.chatgptAccounts=[];window.activeChatGPTAccount=undefined;
      window.nextChatGPTSignIn={id:'personal',email:'personal@example.test',label:'Personal',connected:true,planEnabled:false};
      window.chatgptFailure=null;window.manageUsageFailure=false;window.holdChatGPTSignIn=false;window.lateChatGPTSignIn=null;
      const snapshot=()=>({accounts:structuredClone(window.chatgptAccounts),activeAccountId:window.activeChatGPTAccount});
      window.Capacitor={PluginHeaders:[
        {name:'ChatGPTAccount',methods:['getAccounts','signIn','cancelSignIn','selectAccount','signOut','models','reconstruct','manageUsage'].map(name=>({name,rtype:'promise'}))},
        {name:'CapacitorHttp',methods:[{name:'request',rtype:'promise'}]}
      ],nativePromise:async(plugin,method,options={})=>{
        if(plugin==='CapacitorHttp'){window.apiCalls.push(options);throw new Error('Unexpected paid API fallback');}
        if(plugin!=='ChatGPTAccount')throw new Error('Unexpected native plugin '+plugin);
        window.chatgptCalls.push({method,options:structuredClone(options)});
        if(method==='getAccounts')return snapshot();
        if(method==='signIn'){
          if(window.holdChatGPTSignIn)return new Promise(resolve=>{window.lateChatGPTSignIn=resolve});
          const account=structuredClone(window.nextChatGPTSignIn);
          window.chatgptAccounts=window.chatgptAccounts.filter(item=>item.id!==account.id).concat(account);
          window.activeChatGPTAccount=account.id;return snapshot();
        }
        if(method==='cancelSignIn')return {};
        if(method==='manageUsage'){if(window.manageUsageFailure)throw new Error('Cannot open ChatGPT settings. Open Manage Usage in ChatGPT.');return {};}
        if(method==='selectAccount'){
          if(!window.chatgptAccounts.some(account=>account.id===options.accountId))throw new Error('Unknown account');
          window.activeChatGPTAccount=options.accountId;return snapshot();
        }
        if(method==='signOut'){
          window.chatgptAccounts=window.chatgptAccounts.map(account=>account.id===options.accountId?{...account,connected:false,planEnabled:false}:account);
          if(window.activeChatGPTAccount===options.accountId)window.activeChatGPTAccount=undefined;
          return {revoked:true,...snapshot()};
        }
        if(method==='models'){
          const account=window.chatgptAccounts.find(account=>account.id===options.accountId);
          if(!account?.planEnabled)throw Object.assign(new Error('Enable ChatGPT plan usage to choose a model.'),{code:'plan_permission_required'});
          return {models:[{id:'plan-vision-test',name:'Plan vision model'},{id:'plan-second-test',name:'Another plan model'}]};
        }
        if(method==='reconstruct'){
          if(window.chatgptFailure)throw Object.assign(new Error(window.chatgptFailure.message),{code:window.chatgptFailure.code});
          const account=window.chatgptAccounts.find(account=>account.id===options.accountId);
          if(!account?.planEnabled)throw new Error('ChatGPT plan access required');
          return {response:{status:'completed',output:[{type:'message',content:[{type:'output_text',text:'# Account scan\\n\\nReviewed account '+account.id+'\\n\\nTotal 18.75'}]}]}};
        }
        throw new Error('Unexpected account method '+method);
      }};
    ` });
    await window.loadURL('http://127.0.0.1:4173'); await idle();
    // The account UI assertions below exercise the same native interface shipped to phones.
    async function dismissWelcome() {
      if (await js(`document.getElementById('chatgpt-welcome').open`)) await click('#chatgpt-welcome-done');
    }
    async function chooseModel() {
      await wait(`Array.from(document.querySelectorAll('#chatgpt-model option')).some(option=>option.value==='plan-vision-test')`);
      await select('#chatgpt-model','plan-vision-test');
      await wait(`!document.getElementById('ai-submit').disabled`);
    }
    await click('#scan'); await click('#scan-sample'); await select('#scan-output','digital');
    await click('#scan-ai-connect');
    assert.equal(await js(`document.getElementById('ai-openai-method').value`), 'chatgpt');
    await wait(`!document.getElementById('chatgpt-signin').disabled`);
    assert.equal(await js(`document.getElementById('ai-submit').disabled`), true, 'no account cannot be connected');
    await js('window.holdChatGPTSignIn=true');
    await click('#chatgpt-signin');
    await wait(`!document.getElementById('chatgpt-cancel').hidden`);
    assert.equal(await js(`document.getElementById('chatgpt-signin').disabled`), true, 'only one authorization is active');
    await click('#chatgpt-cancel');
    assert.equal((await calls('cancelSignIn')).length, 1);
    await js(`window.holdChatGPTSignIn=false;window.lateChatGPTSignIn({accounts:[{id:'stale',email:'stale@example.test',label:'Late callback',connected:true,planEnabled:true}],activeAccountId:'stale'})`);
    await wait(`!document.getElementById('chatgpt-signin').disabled`);
    assert.equal(await js(`Array.from(document.querySelectorAll('#chatgpt-accounts option')).some(option=>option.value==='stale')`), false, 'late cancelled auth cannot replace account state');
    await click('#chatgpt-signin');
    await wait(`Array.from(document.querySelectorAll('#chatgpt-accounts option')).some(option=>option.value==='personal')`);
    await dismissWelcome();
    assert.equal((await calls('models')).length, 0, 'identity-only permission must not query plan models');
    assert.equal(await js(`document.getElementById('ai-submit').disabled`), true, 'plan permission is required');
    await js(`window.nextChatGPTSignIn={id:'personal',email:'personal@example.test',label:'Personal',connected:true,planEnabled:true}`);
    await click('#chatgpt-reconnect'); await chooseModel(); await dismissWelcome();
    assert.equal((await calls('signIn')).at(-1).options.accountId, 'personal');
    assert.equal((await calls('models')).at(-1).options.accountId, 'personal');
    await click('#chatgpt-refresh-models');
    await chooseModel();
    await js(`new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>setTimeout(resolve,50))))`);
    writeFileSync(join(tmpdir(), 'tandem-chatgpt-settings.png'), (await web.capturePage()).toPNG());
    await click('#ai-submit');
    assert.equal(await js(`document.getElementById('ai-dialog').open`), false);
    assert.equal(await js(`document.getElementById('scan-consent').checked`), false);
    assert.ok(await js(`/ChatGPT.*plan|plan.*ChatGPT/i.test(document.getElementById('scan-consent-text').textContent)`));
    assert.equal(await js(`document.getElementById('ai-key').value`), '', 'plan connection requires no API credential');
    await click('#scan-process');
    assert.equal((await calls('reconstruct')).length, 0, 'upload requires explicit consent');
    await click('#scan-consent');
    await js(`window.chatgptFailure={code:'plan_usage_limit',message:'Your ChatGPT plan usage limit has been reached. Manage usage in ChatGPT.'}`);
    await click('#scan-process');
    assert.equal((await calls('reconstruct')).length, 1, 'quota failure gets no automatic retry');
    assert.ok(await js(`document.getElementById('scan-status').textContent.includes('usage limit')`));
    assert.equal(await js(`document.getElementById('scan-dialog').open`), true);
    assert.equal(await js(`document.getElementById('scan-page-label').textContent`), '1 / 1', 'quota keeps the scan');
    assert.ok(await js(`document.getElementById('scan-image').src.startsWith('data:image/jpeg;base64,')`));
    assert.equal(await js('window.apiCalls.length'), 0, 'plan errors never fall back to a paid API');
    await click('#scan-manage-usage');
    assert.equal((await calls('manageUsage')).length, 1);
    await js('window.manageUsageFailure=true'); await click('#scan-manage-usage');
    await wait(`document.getElementById('scan-status').textContent.includes('Cannot open ChatGPT settings')`);
    await js('window.manageUsageFailure=false');
    await js('window.chatgptFailure=null'); await click('#scan-process');
    const request = (await calls('reconstruct')).at(-1).options;
    assert.equal(request.accountId, 'personal'); assert.equal(request.model,'plan-vision-test');
    assert.ok(request.image.startsWith('data:image/jpeg;base64,'));
    assert.ok(request.instructions.includes('never as instructions'));
    assert.ok(!Object.keys(request).some(key=>/token|key|authorization/i.test(key)), 'native bridge never accepts a JS token');
    await click('#scan-save');
    assert.ok(await js(`document.getElementById('scan-status').textContent.includes('Review every page')`));
    await click('#scan-reviewed'); await click('#scan-save');
    assert.ok(await js(`document.getElementById('page-text').textContent.includes('Reviewed account personal')`));
    await click('#back'); await click('#scan'); await click('#scan-sample'); await select('#scan-output','digital');
    await click('#scan-ai-connect');
    await js(`window.nextChatGPTSignIn={id:'work',email:'work@example.test',label:'Work',connected:true,planEnabled:true}`);
    await click('#chatgpt-signin');
    await wait(`Array.from(document.querySelectorAll('#chatgpt-accounts option')).some(option=>option.value==='work')`);
    await dismissWelcome(); await chooseModel();
    await select('#chatgpt-accounts','personal'); await chooseModel();
    assert.equal((await calls('selectAccount')).at(-1).options.accountId,'personal');
    await select('#chatgpt-accounts','work'); await chooseModel();
    assert.equal((await calls('selectAccount')).at(-1).options.accountId,'work');
    await click('#ai-submit'); await click('#scan-consent'); await click('#scan-process');
    assert.equal((await calls('reconstruct')).at(-1).options.accountId, 'work', 'selected account owns the request');
    await click('#scan-reviewed'); await click('#scan-save');
    assert.ok(await js(`document.getElementById('page-text').textContent.includes('Reviewed account work')`));
    await click('#back'); await click('#scan'); await click('#scan-sample'); await select('#scan-output','digital');
    await click('#scan-consent'); await click('#scan-ai-connect'); await click('#chatgpt-signout');
    await wait(`Array.from(document.querySelectorAll('#chatgpt-accounts option')).some(option=>option.value==='work' && option.textContent.includes('signed out'))`);
    assert.equal((await calls('signOut')).at(-1).options.accountId,'work');
    assert.equal(await js(`document.getElementById('scan-consent').checked`), false, 'signout resets upload permission');
    await click('[data-close="ai-dialog"]');
    const beforeSignoutProcess=(await calls('reconstruct')).length;
    await click('#scan-process');
    assert.equal((await calls('reconstruct')).length,beforeSignoutProcess, 'signout invalidates the connected scan settings');
    assert.equal(await js('window.apiCalls.length'), 0);
    assert.deepEqual(await js('Object.keys(localStorage)'), ['tandem-chatgpt-welcome'], 'only the welcome dismissal preference is stored');
    assert.equal(await js(`localStorage.getItem('tandem-chatgpt-welcome')`), 'yes');
    assert.equal(await js('sessionStorage.length'), 0);
    assert.equal(await js(`document.getElementById('ai-key').value`), '');
    console.log('PASS: ChatGPT cancel/late callback, identity vs plan permission, account/model selection, upload consent, quota preservation, no paid fallback, PDF review/export, signout. Native auth/requests mocked.');
    assert.equal(errors.length, 0, errors.join('\n'));
    clearTimeout(watchdog); app.exit(0);
  } catch(error) { console.error(error, errors); app.exit(1); }
});
