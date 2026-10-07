import { chmod, cp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { packager } from '@electron/packager';

const platform = process.argv[2] ?? process.platform;
const arch = process.argv[3] ?? process.arch;
if (!['darwin-arm64', 'win32-x64', 'linux-x64'].includes(`${platform}-${arch}`)) {
  throw new Error('Supported targets: darwin arm64, win32 x64 or linux x64.');
}
const manifest = JSON.parse(await readFile('package.json', 'utf8'));
await rm('.stage', { recursive: true, force: true });
await mkdir('.stage/licenses', { recursive: true });
await cp('build', '.stage/build', { recursive: true });
await cp('node_modules/mupdf/LICENSE', '.stage/licenses/MuPDF-AGPL-3.0.txt');
await cp('LICENSE', '.stage/licenses/PDF4All-MIT.txt');
await cp('docs/third-party', '.stage/licenses/third-party', { recursive: true });
await cp('fixtures/hello.provenance.json', '.stage/licenses/fixture-provenance.json');
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
  "Tandem PDF's own code is MIT (PDF4All-MIT.txt). This build bundles MuPDF and jbig2dec, both AGPL-3.0-or-later (MuPDF-AGPL-3.0.txt), so the build as a whole is distributed under the AGPL.",
  `Corresponding source: the app's source at this version's tag and the MuPDF source archive, both at ${source} (build steps: README.md; MuPDF rebuild recipe: third-party/corresponding-source.md).`,
  'Electron and Chromium notices: LICENSE and LICENSES.chromium.html (beside the app on Windows and Linux, in this folder on macOS).',
  ...(platform === 'win32' ? ['d3dcompiler_47.dll, dxcompiler.dll and dxil.dll are redistributed unmodified, as shipped by Electron; they are Microsoft components under their own terms.'] : []),
  'This software is based in part on the work of the Independent JPEG Group.', '',
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
