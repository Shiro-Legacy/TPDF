// The test build (`npm run build:test`): the shared TypeScript in web/ (the worker protocol, the Rust engine's
// adapter and the worker) compiled by tsc into build-test/, with the engine's WASM glue in build-test/vendor and the
// pinned CJK font in build-test/fonts, laid out as the worker loads them. The tests (tests/rust-engine.mjs and
// the files that import build-test/) and engine/oracle's tools load the adapter from here; nothing ships it (the
// desktop app links the engine natively, mobile builds web/ itself). A missing or stale engine/web/pkg, or a CJK
// font that is not the pinned one, fails here, before build-test/ changes.
import { spawnSync } from 'node:child_process';
import { copyFile, mkdir, rm, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { CJK_FONT, readCjkFont } from './cjk-font.mjs';
import { checkRustPackage, RUST_ASSETS, RUST_PKG } from './engine-package.mjs';

const ROOT = fileURLToPath(new URL('..', import.meta.url));
const OUT = join(ROOT, 'build-test');
const cjkFont = readCjkFont(ROOT);
await checkRustPackage();
// A previous build must not leave other files behind.
await rm(OUT, { recursive: true, force: true });
// tsc from the root's typescript (a dev dependency); its package exports no bin path, so it is found by its package.json.
const tsc = join(dirname(createRequire(import.meta.url).resolve('typescript/package.json')), 'bin/tsc');
const compiled = spawnSync(process.execPath, [tsc, '-p', join(ROOT, 'web/tsconfig.json')], { stdio: 'inherit' });
if (compiled.status !== 0) throw new Error(`tsc -p web/tsconfig.json failed (exit ${compiled.status ?? compiled.signal})`);
await mkdir(`${OUT}/vendor`, { recursive: true });
for (const name of RUST_ASSETS) await copyFile(`${RUST_PKG}${name}`, `${OUT}/vendor/${name}`);
await mkdir(`${OUT}/fonts`, { recursive: true });
await writeFile(`${OUT}/${CJK_FONT.served}`, cjkFont);
await copyFile(join(ROOT, CJK_FONT.notice), `${OUT}/fonts/DroidSansFallback-NOTICE.txt`);
console.log('Test build written to build-test/.');
