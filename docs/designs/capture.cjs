// Run: node_modules/.bin/electron docs/designs/capture.cjs
// Static mockups only. No app code, PDF engine, or network access.
const { app, BrowserWindow, session } = require('electron');
const { writeFileSync } = require('node:fs');
const { join } = require('node:path');
const assert = require('node:assert/strict');

app.commandLine.appendSwitch('force-device-scale-factor', '1');
app.whenReady().then(async () => {
  session.defaultSession.webRequest.onBeforeRequest(
    { urls: ['http://*/*', 'https://*/*'] },
    (_details, callback) => callback({ cancel: true }),
  );
  const win = new BrowserWindow({
    width: 1440, height: 1000, useContentSize: true, show: false,
    webPreferences: { sandbox: true, contextIsolation: true, nodeIntegration: false },
  });
  for (const name of ['03-focus']) {
    await win.loadFile(join(__dirname, `${name}.html`));
    const dimensions = await win.webContents.executeJavaScript(`
      document.fonts.ready.then(() => ({
        width: innerWidth, height: innerHeight,
        remoteAssets: [...document.querySelectorAll('[src], link[href]')]
          .some(el => /^https?:/.test(el.src || el.href)),
        text: document.body.innerText.length
      }))
    `);
    assert.equal(dimensions.width, 1440);
    assert.equal(dimensions.height, 1000);
    assert.equal(dimensions.remoteAssets, false);
    assert.ok(dimensions.text > 500, 'Mockup should include readable document and UI text');
    await new Promise(resolve => setTimeout(resolve, 150));
    const image = (await win.webContents.capturePage()).resize({ width: 1440, height: 1000 });
    assert.deepEqual(image.getSize(), { width: 1440, height: 1000 });
    writeFileSync(join(__dirname, `${name}.png`), image.toPNG());
    console.log(`Captured ${name}.png — 1440×1000; static checks passed`);
  }
  app.quit();
}).catch(error => { console.error(error); app.exit(1); });
