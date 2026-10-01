import { cp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { packager } from '@electron/packager';

const platform = process.argv[2] ?? process.platform;
const arch = process.argv[3] ?? process.arch;
if (!['darwin-arm64', 'win32-x64', 'linux-x64'].includes(`${platform}-${arch}`)) {
  throw new Error('P0 evaluation targets only: darwin arm64, win32 x64 or linux x64 (not a support commitment).');
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
await writeFile('.stage/PRIVATE-SPIKE.txt', 'Private unsigned feasibility experiment. Not a release. PDF4All code is MIT (licenses/PDF4All-MIT.txt); the bundled MuPDF is AGPL-3.0-or-later, so any distribution of this build must meet AGPL obligations including corresponding source; do not redistribute this spike build.\nThis software is based in part on the work of the Independent JPEG Group.\n');
const paths = await packager({
  dir: '.stage', out: 'out', name: manifest.productName, platform, arch,
  electronVersion: manifest.devDependencies.electron,
  download: { checksums: JSON.parse(await readFile('node_modules/electron/checksums.json', 'utf8')) },
  appBundleId: 'com.shiro-legacy.tandem-pdf', asar: true, overwrite: true,
  prune: false, osxSign: false,
  // Both from scripts/make-icon.sh. A Linux binary carries no icon: the window loads build/icon.png (src/main.ts).
  ...(platform === 'linux' ? {} : { icon: platform === 'win32' ? 'assets/icon.ico' : 'assets/icon.icns' }),
  appCategoryType: 'public.app-category.productivity',
  appCopyright: 'Copyright © 2026 Shiro-Legacy',
  // Windows: the exe's version resource. "Open with" for PDFs is a per-user registry entry there
  // (scripts/install-windows.ps1) and a .desktop file on Linux (scripts/install-linux.sh), not part of the bundle.
  win32metadata: { CompanyName: 'Shiro-Legacy', FileDescription: manifest.productName, ProductName: manifest.productName },
  ignore: [/^\/agent($|\/)/], extraResource: ['.stage/agent'],
  // "Open With" for PDFs; Alternate so Preview stays the default until the user picks otherwise.
  extendInfo: { CFBundleDocumentTypes: [{
    CFBundleTypeName: 'PDF document', CFBundleTypeRole: 'Editor', LSHandlerRank: 'Alternate',
    LSItemContentTypes: ['com.adobe.pdf'],
  }] },
});
console.log(paths.join('\n'));
