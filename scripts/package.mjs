import { chmod, cp, mkdir, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { isAbsolute, relative, resolve, sep } from 'node:path';
import { packager } from '@electron/packager';
import { CJK_FONT } from './cjk-font.mjs';

const platform = process.argv[2] ?? process.platform;
const arch = process.argv[3] ?? process.arch;
if (!['darwin-arm64', 'win32-x64', 'linux-x64'].includes(`${platform}-${arch}`)) {
  throw new Error('Supported targets: darwin arm64, win32 x64 or linux x64.');
}
// Packages ship the Rust engine (`npm run package` builds it) and its notices. MuPDF is a test-only oracle:
// a `npm run build:mupdf` build is for tests and is refused here.
const { ENGINE } = await import(new URL('../build/engine-build.js', import.meta.url).href);
const { localAsset } = await import(new URL('../build/policy.js', import.meta.url).href);
if (ENGINE !== 'rust') throw new Error(`packages ship the Rust engine, but build/engine-build.js names ${ENGINE}; run npm run package (a MuPDF build is for tests only)`);
const manifest = JSON.parse(await readFile('package.json', 'utf8'));
await rm('.stage', { recursive: true, force: true });
await mkdir('.stage/licenses', { recursive: true });
// Of build/renderer only what the app serves (src/policy.ts): the package holds no MuPDF engine module.
const renderer = resolve('build/renderer');
await cp('build', '.stage/build', { recursive: true, filter: async source => {
  const path = relative(renderer, resolve(source));
  if (path === '' || path === '..' || path.startsWith(`..${sep}`) || isAbsolute(path)) return true;
  return (await stat(source)).isDirectory() || !!localAsset(`app://pdf4all/${path.split(sep).join('/')}`);
} });
await cp('LICENSE', '.stage/licenses/PDF4All-MIT.txt');
await cp('fixtures/hello.provenance.json', '.stage/licenses/fixture-provenance.json');
// docs/third-party describes the MuPDF WASM; of it only Electron's LGPL record and the CJK font's notice apply.
await cp('engine/NOTICE', '.stage/licenses/Engine-NOTICE.txt');
await cp('engine/COPYING', '.stage/licenses/Engine-AGPL-3.0.txt');
await cp('engine/reference/licenses', '.stage/licenses/engine-licenses', { recursive: true });
await mkdir('.stage/licenses/third-party');
await cp('docs/third-party/lgpl-relink.md', '.stage/licenses/third-party/lgpl-relink.md');
// The CJK fallback font the Rust build ships (scripts/cjk-font.mjs): Apache-2.0 text and the AOSP copyright.
await cp(CJK_FONT.notice, '.stage/licenses/third-party/droid-sans-fallback-NOTICE.txt');
await writeFile('.stage/package.json', JSON.stringify({
  name: manifest.name, productName: manifest.productName, version: manifest.version, type: 'module',
  main: manifest.main, private: true, license: manifest.license,
  // Linux: the Wayland app id, so the shell pairs the window with tandem-pdf.desktop (scripts/install-linux.sh).
  desktopName: 'tandem-pdf.desktop',
}));
// The agent side ships beside app.asar (Contents/Resources/agent/; Windows resources\agent\, Linux resources/agent/) so an MCP client needs only the app:
// scripts/session.mjs finds the app's own binary from there. Notice file names below keep "PDF4All"
// because the dated records in docs/third-party cite them.
await mkdir('.stage/agent/scripts', { recursive: true });
await mkdir('.stage/agent/docs', { recursive: true });
for (const file of ['scripts/tandem-mcp.mjs', 'scripts/session.mjs', 'docs/agent.md']) await cp(file, `.stage/agent/${file}`);
await cp('.stage/package.json', '.stage/agent/package.json');
// What a recipient reads first: the licence of the whole build and where its source is (AGPL section 6). It ships
// inside app.asar and, with every other notice, in the readable resources/licenses folder beside it.
const source = `https://github.com/Shiro-Legacy/TPDF/releases/tag/v${manifest.version}`;
await writeFile('.stage/licenses/NOTICE.txt', [
  `${manifest.productName} ${manifest.version}`,
  // The wording below is engine/design/app-worker.md's, for the owner to confirm before a release
  // (engine/design/app-m9-mobile-release.md lists it).
  "Tandem PDF's own code is MIT (PDF4All-MIT.txt). This build bundles the Tandem PDF Rust engine, AGPL-3.0-or-later and informed by MuPDF (Engine-NOTICE.txt, Engine-AGPL-3.0.txt; its dependencies' licence files are in engine-licenses), so the build as a whole is distributed under the AGPL.",
  `Corresponding source: the app's source at this version's tag, the engine in its engine folder, at ${source} (build steps: README.md and engine/tools/build-workflow.sh).`,
  `The CJK fallback font Droid Sans Fallback (${CJK_FONT.served} in the app), Copyright (c) 2005-2008 The Android Open Source Project, is Apache-2.0 (third-party/droid-sans-fallback-NOTICE.txt).`,
  'Electron and Chromium notices: LICENSE and LICENSES.chromium.html (beside the app on Windows and Linux, in this folder on macOS).',
  ...(platform === 'win32' ? ['d3dcompiler_47.dll, dxcompiler.dll and dxil.dll are redistributed unmodified, as shipped by Electron; they are Microsoft components under their own terms.'] : []),
  // No IJG sentence: that was owed for MuPDF's libjpeg; the Rust engine decodes JPEG with zune-jpeg (engine/NOTICE).
  '',
].join('\n'));
// macOS: Electron's two notice files sit beside the .app, which a DMG does not carry, so they go inside it.
if (platform === 'darwin') for (const file of ['LICENSE', 'LICENSES.chromium.html']) await cp(`node_modules/electron/dist/${file}`, `.stage/licenses/Electron-${file}`);
const sign = platform === 'darwin' ? process.env.APPLE_SIGN_IDENTITY : undefined;
const paths = await packager({
  dir: '.stage', out: 'out', name: manifest.productName, platform, arch,
  electronVersion: manifest.devDependencies.electron,
  download: { checksums: JSON.parse(await readFile('node_modules/electron/checksums.json', 'utf8')) },
  appBundleId: 'com.shiro-legacy.tandem-pdf', asar: true, overwrite: true,
  prune: false,
  // A release build signs with a Developer ID certificate from the keychain and notarizes (hardened runtime and
  // Electron's default entitlements come from @electron/osx-sign); without the variables the bundle stays unsigned.
  osxSign: sign ? { identity: sign } : false,
  ...(sign && process.env.APPLE_ID ? { osxNotarize: { appleId: process.env.APPLE_ID, appleIdPassword: process.env.APPLE_APP_PASSWORD, teamId: process.env.APPLE_TEAM_ID } } : {}),
  // Both from scripts/make-icon.mjs. A Linux binary carries no icon: the window loads build/icon.png (src/main.ts).
  ...(platform === 'linux' ? {} : { icon: platform === 'win32' ? 'assets/icon.ico' : 'assets/icon.icns' }),
  appCategoryType: 'public.app-category.productivity',
  appCopyright: 'Copyright © 2026 Shiro-Legacy',
  // Windows: the exe's version resource. "Open with" for PDFs is a per-user registry entry there
  // (scripts/install-windows.ps1) and a .desktop file on Linux (scripts/install-linux.sh), not part of the bundle.
  win32metadata: { CompanyName: 'Shiro-Legacy', FileDescription: manifest.productName, ProductName: manifest.productName },
  // The licences folder is in app.asar and, readable, beside it; Electron's 20 MB notice file only beside it.
  ignore: [/^\/agent($|\/)/, /^\/licenses\/Electron-/], extraResource: ['.stage/agent', '.stage/licenses'],
  // "Open With" for PDFs; Alternate so Preview stays the default until the user picks otherwise.
  extendInfo: { CFBundleDocumentTypes: [{
    CFBundleTypeName: 'PDF document', CFBundleTypeRole: 'Editor', LSHandlerRank: 'Alternate',
    LSItemContentTypes: ['com.adobe.pdf'],
  }] },
});
// @electron/packager makes the app folder with mkdtemp (mode 0700). A per-user install never notices; a .deb installed
// by root for everyone does (/opt/tandem-pdf unreadable to the user: "Permission denied" at launch). Open it up.
if (platform !== 'win32') for (const dir of paths) await chmod(dir, 0o755);
console.log(paths.join('\n'));
