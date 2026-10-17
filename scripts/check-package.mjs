// What a packaged or installed app must hold, the same on every platform. One checker: the release workflow runs
// it as a command on each installed app, and scripts/check-macos-package.mjs (the M10 macOS checklist) imports
// checkPackage() for its `package`, `attribution` and `wasm-paths` items.
//   node scripts/check-package.mjs <smoke report.json> <the app's resources folder>
// The report must say ok, packaged, sandboxed and the Rust engine (its worker round trip included). The
// resources folder (macOS: Tandem PDF.app/Contents/Resources; Windows and Linux: resources beside the binary)
// must hold an app.asar with the Rust engine and no MuPDF file, reference or code, a WASM that names no
// build-machine path (and is the engine job's, when TPDF_WASM_SHA256 is set), and the Rust build's notices,
// each the same file as in this checkout.
import { createHash } from 'node:crypto';
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { CJK_FONT } from './cjk-font.mjs';

const ROOT = fileURLToPath(new URL('..', import.meta.url));
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');

/** The files of an asar archive (its JSON header) and a reader for one of them. */
function asar(resources) {
  const bytes = readFileSync(join(resources, 'app.asar'));
  const header = JSON.parse(bytes.toString('utf8', 16, 16 + bytes.readUInt32LE(12)));
  const base = 8 + bytes.readUInt32LE(4);
  const files = new Map();
  (function walk(node, prefix) {
    for (const [name, entry] of Object.entries(node.files ?? {})) {
      const path = prefix ? `${prefix}/${name}` : name;
      if (entry.files) walk(entry, path);
      else if (!entry.link) files.set(path, entry);
    }
  })(header, '');
  const read = path => {
    const entry = files.get(path);
    if (!entry) return null;
    if (entry.unpacked) return readFileSync(join(resources, 'app.asar.unpacked', path));
    return bytes.subarray(base + Number(entry.offset), base + Number(entry.offset) + entry.size);
  };
  return { files: [...files.keys()], read };
}

/** Every file under `dir`, as paths relative to it ('/'-separated). */
const walk = dir => readdirSync(dir, { withFileTypes: true })
  .flatMap(entry => entry.isDirectory() ? walk(join(dir, entry.name)).map(path => `${entry.name}/${path}`) : [entry.name]);

/** Absolute build-machine paths in a WASM module: a home folder, the checkout, any /Users/ or /home/ path. */
export function wasmPathLeaks(bytes, { home = homedir(), checkout = ROOT } = {}) {
  const text = Buffer.from(bytes).toString('latin1');
  return [home, checkout.replace(/[\\/]$/, ''), '/Users/', '/home/'].filter(needle => needle && text.includes(needle));
}

// An import, dynamic import, URL or script tag naming MuPDF (policy.js may list MuPDF's assets as data; it serves
// only the built engine's).
const LOADS_MUPDF = /(?:\bfrom\s*|\bimport\s*\(\s*|\bnew URL\(\s*|\bsrc=)["'`][^"'`]*mupdf/i;
// MuPDF's code under any name: its WASM exports fz_/pdf_ C symbols and names itself mupdf-wasm, and its glue
// (mupdf.js, mupdf-wasm.js) calls into libmupdf. None of these occur in the Rust package.
const MUPDF_CODE = { wasm: /fz_new_context|pdf_new_document|mupdf-wasm/, js: /libmupdf/ };
const WASM = 'build/renderer/vendor/tandem_rust_workflow_bg.wasm';

/**
 * Checks the resources folder of a packaged or installed app against this checkout (`root`). `report` (the
 * packaged `--smoke` report) is optional; `platform` defaults to the report's. Returns `checks` (name -> true,
 * false, or null when it does not apply) in three groups, the `failures` among them, and facts for a report.
 */
export function checkPackage({ resources, report = null, root = ROOT, platform = report?.platform ?? process.platform, expectedWasmSha256 = null }) {
  const smoke = report ? {
    'the self-test passed': report.ok === true,
    'it ran packaged': report.packaged === true,
    'it ran sandboxed': report.sandbox === true,
    'the build is the Rust engine': report.engine === 'rust',
    'the worker round trip ran on the Rust engine': report.roundtrip?.engine === 'rust',
  } : {};

  const archive = asar(resources);
  const mupdfFiles = archive.files.filter(path => /mupdf/i.test(path));
  const mupdfLoaders = archive.files.filter(path => /\.(m?js|html)$/.test(path) && LOADS_MUPDF.test(archive.read(path).toString('utf8')));
  const mupdfCode = archive.files.filter(path => {
    const kind = path.endsWith('.wasm') ? 'wasm' : /\.m?js$/.test(path) ? 'js' : null;
    return kind && MUPDF_CODE[kind].test(archive.read(path).toString('latin1'));
  });
  const wasm = archive.read(WASM);
  const wasmSha256 = wasm && sha256(wasm);
  const leaks = wasm ? wasmPathLeaks(wasm, { checkout: root }) : [];
  const font = archive.read(`build/renderer/${CJK_FONT.served}`);
  const contents = {
    'app.asar holds no file named after MuPDF': mupdfFiles.length === 0,
    'no script in app.asar loads a MuPDF file': mupdfLoaders.length === 0,
    'no file in app.asar carries MuPDF code': mupdfCode.length === 0,
    ...Object.fromEntries(['build/renderer/vendor/tandem_rust_workflow.js', WASM, 'build/renderer/engine-rust.js', `build/renderer/${CJK_FONT.served}`]
      .map(path => [`app.asar holds ${path}`, archive.files.includes(path)])),
    'the CJK font is the pinned one': !!font && sha256(font) === CJK_FONT.sha256 && font.length === CJK_FONT.size,
    'build/engine-build.js names the Rust engine': /export const ENGINE = "rust"/.test(archive.read('build/engine-build.js')?.toString('utf8') ?? ''),
    'the WASM names no build-machine path (docs/building-rust-release.md)': !!wasm && leaks.length === 0,
    // One WASM for every platform: the engine job's hash, when the workflow passes it.
    "the WASM is the engine job's (TPDF_WASM_SHA256)": expectedWasmSha256 ? wasmSha256 === expectedWasmSha256 : null,
  };

  const licences = join(resources, 'licenses');
  const read = path => { try { return readFileSync(path); } catch { return null; } };
  const same = (shipped, source) => { const a = read(join(licences, shipped)), b = read(join(root, source)); return !!a && !!b && a.equals(b); };
  const text = path => read(join(licences, path))?.toString('utf8') ?? '';
  const notice = text('NOTICE.txt'), droid = text('third-party/droid-sans-fallback-NOTICE.txt');
  const engineLicences = walk(join(root, 'engine/reference/licenses'));
  const shippedEngineLicences = existsSync(join(licences, 'engine-licenses')) ? walk(join(licences, 'engine-licenses')) : [];
  // The licences inside app.asar (/licenses) are the same files as the readable folder, minus Electron's.
  const asarLicences = archive.files.filter(path => path.startsWith('licenses/'));
  const readable = existsSync(licences) ? walk(licences).filter(path => !path.startsWith('Electron-')) : [];
  // Electron's notices: inside the bundle on macOS (a DMG carries only the .app), beside the binary elsewhere.
  const electron = platform === 'darwin' ? ['Electron-LICENSE', 'Electron-LICENSES.chromium.html'].map(name => join(licences, name))
    : ['LICENSE', 'LICENSES.chromium.html'].map(name => resolve(resources, '..', name));
  const licenceChecks = {
    'Engine-NOTICE.txt is engine/NOTICE': same('Engine-NOTICE.txt', 'engine/NOTICE'),
    'Engine-AGPL-3.0.txt is engine/COPYING': same('Engine-AGPL-3.0.txt', 'engine/COPYING'),
    'PDF4All-MIT.txt is LICENSE': same('PDF4All-MIT.txt', 'LICENSE'),
    'engine-licenses is engine/reference/licenses': engineLicences.length > 0 && engineLicences.length === shippedEngineLicences.length
      && engineLicences.every(path => same(`engine-licenses/${path}`, `engine/reference/licenses/${path}`)),
    "engine-licenses holds wasm-bindgen's licence": existsSync(join(licences, 'engine-licenses/wasm-bindgen-0.2.129')),
    'lgpl-relink.md is docs/third-party/lgpl-relink.md': same('third-party/lgpl-relink.md', 'docs/third-party/lgpl-relink.md'),
    'droid-sans-fallback-NOTICE.txt is the committed notice': same('third-party/droid-sans-fallback-NOTICE.txt', CJK_FONT.notice),
    'Droid notice: Apache-2.0 text and the AOSP copyright': /Apache License/.test(droid) && /Version 2\.0/.test(droid) && /Android Open Source Project/.test(droid),
    'no MuPDF licence ships with a Rust build': !existsSync(join(licences, 'MuPDF-AGPL-3.0.txt')),
    "Electron's LICENSE and LICENSES.chromium.html ship": electron.every(path => existsSync(path)),
    'NOTICE.txt: Rust engine AGPL-3.0-or-later, informed by MuPDF': /Rust engine, AGPL-3\.0-or-later and informed by MuPDF/.test(notice),
    'NOTICE.txt: corresponding source at the tag, engine included': /Corresponding source: the app's source at this version's tag, the engine in its engine folder/.test(notice),
    'NOTICE.txt: Droid Sans Fallback, AOSP copyright, Apache-2.0': /Droid Sans Fallback/.test(notice) && /Android Open Source Project/.test(notice) && /Apache-2\.0/.test(notice),
    'NOTICE.txt: no IJG sentence (no libjpeg in a Rust build)': !/Independent JPEG Group/.test(notice),
    'NOTICE.txt: no MuPDF archive or MuPDF-AGPL file named': !/MuPDF-AGPL-3\.0\.txt|MuPDF source archive/.test(notice),
    'licences in app.asar match the readable folder': readable.length > 0 && asarLicences.length === readable.length
      && readable.every(path => archive.read(`licenses/${path}`)?.equals(readFileSync(join(licences, path))) === true),
  };

  const checks = { smoke, contents, licences: licenceChecks };
  const failures = Object.values(checks).flatMap(group => Object.entries(group).filter(([, ok]) => ok === false).map(([name]) => name));
  return {
    checks, failures,
    facts: { asarFiles: archive.files.length, wasmSha256, wasmBytes: wasm?.length ?? null, wasmPathLeaks: leaks, mupdfFiles, mupdfLoaders, mupdfCode },
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const [reportPath, resources] = process.argv.slice(2);
  if (!reportPath || !resources) throw new Error('usage: node scripts/check-package.mjs <smoke report.json> <resources folder>');
  const report = JSON.parse(readFileSync(reportPath, 'utf8'));
  console.log(report.platform, report.arch, 'ok', report.ok, 'packaged', report.packaged, 'sandbox', report.sandbox, 'engine', report.engine);
  const { failures, facts } = checkPackage({ resources, report, expectedWasmSha256: process.env.TPDF_WASM_SHA256 || null });
  console.log('tandem_rust_workflow_bg.wasm sha256', facts.wasmSha256);
  if (failures.length) {
    const named = { mupdfFiles: facts.mupdfFiles, mupdfLoaders: facts.mupdfLoaders, mupdfCode: facts.mupdfCode, wasmPathLeaks: facts.wasmPathLeaks };
    console.error(`The packaged app failed ${failures.length} check(s):\n- ${failures.join('\n- ')}\n${JSON.stringify(named)}`);
    process.exit(1);
  }
  console.log(`Packaged app checks passed (${facts.asarFiles} files in app.asar).`);
}
