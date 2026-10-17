// Checks an installed or packaged app after its `--smoke` self-test, the same on every platform:
//   node scripts/check-package.mjs <smoke report.json> <the app's resources folder>
// The report must say ok, packaged, sandboxed and the Rust engine (its worker round trip included). The
// resources folder (macOS: Tandem PDF.app/Contents/Resources; Windows and Linux: resources beside the binary)
// must hold an app.asar with the Rust engine and no MuPDF file or reference, and the Rust build's notices.
import { readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';

const [reportPath, resources] = process.argv.slice(2);
if (!reportPath || !resources) throw new Error('usage: node scripts/check-package.mjs <smoke report.json> <resources folder>');
const failures = [];
const check = (ok, what) => { if (!ok) failures.push(what); };

const report = JSON.parse(readFileSync(reportPath, 'utf8'));
console.log(report.platform, report.arch, 'ok', report.ok, 'packaged', report.packaged, 'sandbox', report.sandbox, 'engine', report.engine);
check(report.ok === true, 'the self-test passed');
check(report.packaged === true, 'it ran packaged');
check(report.sandbox === true, 'it ran sandboxed');
check(report.engine === 'rust', 'the build is the Rust engine');
check(report.roundtrip?.engine === 'rust', 'the worker round trip ran on the Rust engine');

/** The files of an asar archive (its JSON header) and a reader for one of them. */
function asar(path) {
  const bytes = readFileSync(path);
  const header = JSON.parse(bytes.toString('utf8', 16, 16 + bytes.readUInt32LE(12)));
  const base = 8 + bytes.readUInt32LE(4);
  const files = new Map();
  (function walk(node, prefix) {
    for (const [name, entry] of Object.entries(node.files ?? {})) {
      const path = prefix ? `${prefix}/${name}` : name;
      if (entry.files) walk(entry, path);
      else files.set(path, entry);
    }
  })(header, '');
  const read = path => {
    const entry = files.get(path);
    if (entry.unpacked) return readFileSync(join(`${resources}/app.asar.unpacked`, path));
    return bytes.subarray(base + Number(entry.offset), base + Number(entry.offset) + entry.size);
  };
  return { files: [...files.keys()], read };
}

const archive = asar(join(resources, 'app.asar'));
const named = archive.files.filter(path => /mupdf/i.test(path));
check(!named.length, `app.asar holds no file named after MuPDF (${named.join(', ')})`);
for (const path of ['build/renderer/vendor/tandem_rust_workflow.js', 'build/renderer/vendor/tandem_rust_workflow_bg.wasm', 'build/renderer/engine-rust.js', 'build/renderer/fonts/DroidSansFallback.ttf']) {
  check(archive.files.includes(path), `app.asar holds ${path}`);
}
// An import, dynamic import, URL or script tag naming MuPDF (policy.js may list MuPDF's assets as data; it serves
// only the built engine's).
const LOADS_MUPDF = /(?:\bfrom\s*|\bimport\s*\(\s*|\bnew URL\(\s*|\bsrc=)["'`][^"'`]*mupdf/i;
const loaders = archive.files.filter(path => /\.(m?js|html)$/.test(path) && LOADS_MUPDF.test(archive.read(path).toString('utf8')));
check(!loaders.length, `no script in app.asar loads a MuPDF file (${loaders.join(', ')})`);
check(/export const ENGINE = "rust"/.test(archive.read('build/engine-build.js').toString('utf8')), 'build/engine-build.js names the Rust engine');

const licenses = join(resources, 'licenses');
for (const name of ['NOTICE.txt', 'Engine-NOTICE.txt', 'Engine-AGPL-3.0.txt', 'PDF4All-MIT.txt', 'third-party/droid-sans-fallback-NOTICE.txt']) {
  check(existsSync(join(licenses, name)), `licenses/${name} ships`);
}
check(!existsSync(join(licenses, 'MuPDF-AGPL-3.0.txt')), 'no MuPDF licence ships with a Rust build');
const notice = existsSync(join(licenses, 'NOTICE.txt')) ? readFileSync(join(licenses, 'NOTICE.txt'), 'utf8') : '';
check(/Rust engine, AGPL-3\.0-or-later and informed by MuPDF/.test(notice), 'NOTICE.txt names the Rust engine and its licence');
check(!/Independent JPEG Group/.test(notice), 'NOTICE.txt carries no IJG sentence (no libjpeg ships)');

if (failures.length) {
  console.error(`The packaged app failed ${failures.length} check(s):\n- ${failures.join('\n- ')}`);
  process.exit(1);
}
console.log(`Packaged app checks passed (${archive.files.length} files in app.asar).`);
