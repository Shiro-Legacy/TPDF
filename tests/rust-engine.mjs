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
    // The test-only bridge view and setters (RustApp session_debug, session_set).
    const debug = () => JSON.parse(rust.withSession(session, (app, handle) => app.session_debug(handle), 'null')) ?? {};
    const set = fields => rust.withSession(session, (app, handle) => app.session_set(handle, JSON.stringify(fields)), false);
    const sourceBytes = Object.getOwnPropertyDescriptor(session, 'sourceBytes').get;
    let cached = null;

    /** The copy answers journal queries from the Rust journal; its own has no history. */
    function journaled(doc, password) {
      if (password && attempt(() => doc.needsPassword())) attempt(() => doc.authenticatePassword(password));
      const journal = () => debug().journal ?? { position: 0, steps: [] };
      return Object.assign(doc, {
        canUndo: () => rust.journalState(session).canUndo,
        canRedo: () => rust.journalState(session).canRedo,
        getJournal: () => journal(),
      });
    }

    /**
     * The layers panel's visibility (layer-set) is view-only state that MuPDF
     * keeps on its live document; the copy gets it the same way. Rows pair by
     * name and occurrence (named as engine.ts lists them), since Rust lists
     * each group once where MuPDF also lists duplicates and non-groups; a
     * MuPDF row without a Rust partner keeps its own state. The copy serves
     * tests that inspect session.doc; Rust's own render, search and text-all
     * after layer-set are checked in engine/tests/app_view.rs.
     */
    function layered(doc, items) {
      const pdf = doc && attempt(() => doc.asPDF());
      const count = pdf ? attempt(() => pdf.countLayers()) ?? 0 : 0;
      const wanted = new Map();
      for (const item of items) wanted.set(item.name, [...(wanted.get(item.name) ?? []), item.visible]);
      const seen = new Map();
      for (let index = 0; index < count; index++) {
        const name = attempt(() => pdf.getLayerName(index)) || `layer ${index + 1}`;
        const occurrence = seen.get(name) ?? 0;
        seen.set(name, occurrence + 1);
        const visible = wanted.get(name)?.[occurrence];
        if (visible !== undefined) attempt(() => pdf.setLayerVisible(index, visible));
      }
      return doc;
    }

    Object.defineProperties(session, {
      doc: {
        enumerable: true,
        get() {
          // One copy per document state keeps identity checks stable: generation
          // covers open and close, revision every journal step, locked and auth an
          // unlock (which replaces the document without a new revision), and the
          // layer visibility a layer-set (which changes no revision).
          const { generation, revision, locked, auth, password } = debug();
          const layers = locked ? [] : rust.dispatch({ op: 'layers' }, session).reply.items ?? [];
          const key = `${generation}:${revision}:${locked}:${auth}:${layers.map(item => +item.visible).join('')}`;
          // closeDocument destroys a copy (pointer 0); the next read reopens.
          if (cached?.key === key && cached.doc?.pointer !== 0) return cached.doc;
          const bytes = rust.withSession(session, (app, handle) => app.snapshot_bytes(handle), undefined);
          const doc = bytes ? journaled(mupdf.Document.openDocument(bytes, 'application/pdf'), password) : null;
          cached = { key, doc: layered(doc, layers) };
          return cached.doc;
        },
      },
      sourceBytes: {
        enumerable: true,
        get: sourceBytes,
        set(value) {
          if (!set({ sourceBytes: value })) throw new TypeError(`sourceBytes: ${value} is not a byte count`);
        },
      },
      password: { enumerable: true, get: () => debug().password ?? null },
      security: { enumerable: true, get: () => debug().security ?? null },
      securityDirty: { enumerable: true, get: () => debug().securityDirty ?? false },
      // Stand-ins: tests only compare these with null.
      lockedBytes: { enumerable: true, get: () => byteCount(debug().lockedBytes) },
      signedSource: { enumerable: true, get: () => byteCount(debug().signedSource) },
      batch: { enumerable: true, get: () => debug().batch ?? false },
      // The compare slot's fields as Rust reports them; no MuPDF copy of it yet.
      compare: { enumerable: true, get: () => debug().compare ?? null },
      // A token naming this session's clipboard; assigning it to another session
      // of the same engine copies the clipboard in Rust.
      clipboard: {
        enumerable: true,
        get: () => (debug().clipboard ? Object.freeze({ clipboardOf: session.handle }) : null),
        set(value) {
          const from = value === null ? null : value?.clipboardOf;
          if (from === undefined || !set({ clipboardFrom: from })) throw new TypeError('clipboard: not a clipboard of this engine');
        },
      },
    });
    return session;
  }

  return { ...reference, createSession, dispatch: rust.dispatch, journalState: rust.journalState };
}

function byteCount(length) {
  return typeof length === 'number' ? { byteLength: length } : null;
}

function attempt(read) {
  try {
    return read();
  } catch {
    return undefined;
  }
}
