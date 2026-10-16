// npm run test:rust: the unit suite with build/renderer/engine.js resolved to
// the Rust engine (engine-select.mjs), skipping the tests below by name. Each
// one drives MuPDF internals of the live engine, so no Rust engine can pass it
// as written. Usage: node tests/run-rust.mjs [test files or globs]
import { spawnSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';

export const RUST_SKIPS = [
  {
    test: 'save enforces the 50 MiB save ceiling on the source before serialization',
    reason: 'p1-worker: spies on session.doc.saveToBuffer to prove serialization started; needs a Rust fault hook',
  },
  {
    test: 'save rejects an oversized rewrite before copying it out of the heap',
    reason: 'p1-worker: replaces session.doc.saveToBuffer with a fake oversized result; needs a Rust fault hook',
  },
  ...[
    'a failure after a native write rolls back and leaves a clean baseline untouched',
    'a failure after a write on a dirty baseline keeps the earlier edits and the journal',
    'a written abandon after an undo restores content, keeps undo, and drops the redo branch',
    'an abandon with no write keeps the redo branch',
    'the saved state cannot be reported clean once its branch is cut (the P2 false-clean defect)',
  ].map(test => ({
    test,
    reason: 'p3-journal: injects failures by patching mupdf.PDFPage.prototype.createAnnotation; needs a Rust fault hook',
  })),
  {
    test: 'gate: the journal covers /Rotate put, deletePage and rearrangePages',
    reason: 'p2-pages: assigns a raw MuPDF document to session.doc (engineSession)',
  },
  {
    test: 'saved resets the dirty baseline, and undoing back to it is clean again',
    reason: 'p2-edit: asserts MuPDF\'s own hasUnsavedChanges() on the live document; session.doc is a per-revision snapshot',
  },
  {
    test: '§3.4 word-too-wide; stale ids; tagged paragraphs grow; TJ lines become Tj; undo restores',
    reason: 'p6-paragraph: reads a page of session.doc held across an undo; session.doc is a per-revision snapshot (undo exactness is covered by engine/tests/app_edit.rs)',
  },
  {
    test: 'batch (d): every call leaves no transient document alive',
    reason: 'p6-batch: counts MuPDF openDocument/destroy calls and patches saveToBuffer; Rust covers the same branches in engine/tests/app_misc.rs',
  },
];

const pattern = text => `^${text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`;

/** node's arguments for the Rust unit suite over `files` (engine/tools/op-evidence.mjs adds its own flags). */
export function rustTestArgs(files, flags = []) {
  return [
    '--import', './tests/engine-select.mjs', '--test', ...flags,
    ...RUST_SKIPS.map(skip => `--test-skip-pattern=${pattern(skip.test)}`),
    ...(files.length ? files : ['tests/*.test.mjs']),
  ];
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  // --test-skip-pattern drops these from the counts, so the list is printed instead.
  for (const skip of RUST_SKIPS) console.log(`# rust engine skips "${skip.test}": ${skip.reason}`);
  const files = process.argv.slice(2);
  const result = spawnSync(process.execPath, rustTestArgs(files), {
    stdio: 'inherit', env: { ...process.env, TPDF_ENGINE: 'rust', CI_NO_ELECTRON: '1' },
  });
  process.exitCode = result.status ?? 1;
}
