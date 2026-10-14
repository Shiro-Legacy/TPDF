// build/renderer/engine.js under TPDF_ENGINE=rust (see engine-select.mjs). It
// re-exports the real module's constants and helpers and replaces createEngine
// with the Rust engine behind the production adapter. MuPDF stays test-only:
// session.doc is the Rust session's current bytes reopened with real MuPDF, so
// tests that inspect the live document inspect Rust's output.
import { readFileSync } from 'node:fs';
import { createEngine as createMupdfEngine } from '../build/renderer/engine.js?real';
import { createRustEngine } from '../build/renderer/engine-rust.js';

export * from '../build/renderer/engine.js?real';

const pkg = new URL('../engine/web/pkg/', import.meta.url);
let wasm;
try {
  wasm = await import(new URL('tandem_rust_workflow.js', pkg).href);
  wasm.initSync({ module: readFileSync(new URL('tandem_rust_workflow_bg.wasm', pkg)) });
} catch (err) {
  throw new Error(`TPDF_ENGINE=rust needs engine/web/pkg; build it with engine/tools/build-workflow.sh (${err.message})`);
}

export function createEngine(mupdf) {
  // Members that act on MuPDF documents (detect, openDocument, closeDocument, ...)
  // come from the real engine and work on the session.doc copies.
  const reference = createMupdfEngine(mupdf);
  const rust = createRustEngine(wasm);

  function createSession() {
    const session = rust.createSession();
    let cached = null;
    Object.defineProperty(session, 'doc', {
      enumerable: true,
      get() {
        // One copy per (generation, revision) keeps identity checks stable;
        // closeDocument destroys it (pointer 0) and the next read reopens.
        const key = `${session.generation}:${session.revision}`;
        if (cached?.key === key && cached.doc?.pointer !== 0) return cached.doc;
        const bytes = rust.app.snapshot_bytes(session.handle);
        cached = { key, doc: bytes ? mupdf.Document.openDocument(bytes, 'application/pdf') : null };
        return cached.doc;
      },
    });
    return session;
  }

  return { ...reference, createSession, dispatch: rust.dispatch, journalState: rust.journalState };
}
