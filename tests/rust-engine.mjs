// SPDX-License-Identifier: MIT
// The app engine as the tests import it: the Rust engine behind the production
// adapter (build-test/engine-rust.js), the suite's request bounds and, as
// `session.doc`, a reader of the session's document over the engine's
// test-only `inspect` op (inspect.mjs). The reader opens the session's
// current bytes (RustApp snapshot_bytes: what a save would write) in a
// session of its own, so every inspection after an edit also proves the
// serialized bytes parse back to the same facts. No other PDF library is
// involved: the engine's output is read by the engine, and by the independent
// pypdf checks some tests run.
import { readFileSync } from 'node:fs';
import { createRustEngine } from '../build-test/engine-rust.js';
import { MAX_PDF_BYTES, MAX_SAVE_BYTES } from '../build-test/limits.js';
import { readCjkFont } from '../scripts/cjk-font.mjs';
import { Inspector } from './inspect.mjs';

// The request bounds the tests assert against: the app's own (web/, compiled
// into build-test/) and those only the Rust engine defines (engine-constants.mjs).
export * from '../build-test/engine-shared.js';
export * from '../build-test/limits.js';
export { OP_PERMISSION } from '../build-test/engine-core.js';
export * from './engine-constants.mjs';
export * from './inspect.mjs';

const pkg = new URL('../engine/web/pkg/', import.meta.url);
let wasm;
try {
  wasm = await import(new URL('tandem_rust_workflow.js', pkg).href);
  wasm.initSync({ module: readFileSync(new URL('tandem_rust_workflow_bg.wasm', pkg)) });
} catch (err) {
  throw new Error(`the tests need engine/web/pkg; build it with engine/tools/build-workflow.sh (${err.message})`);
}

/** An ArrayBuffer of a Buffer's or Uint8Array's bytes (a copy), as the `open` op takes. */
export function toArrayBuffer(bytes) {
  return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
}

export function createEngine() {
  const rust = createRustEngine(wasm);
  // The CJK fallback font the app's worker reads, from the same committed file.
  rust.useCjkFont(wasm, () => readCjkFont());
  // Each session's compare-slot check, run after the ops that replace or empty the slot.
  const compareChecks = new WeakMap();
  const COMPARE_OPS = new Set(['open', 'compare-open', 'compare-close']);

  function dispatch(request, session) {
    const op = attempt(() => request?.op);
    const result = rust.dispatch(request, session);
    if (COMPARE_OPS.has(op)) compareChecks.get(session)?.();
    return result;
  }

  /** An inspector of the document `session` holds; its reads dispatch `inspect` there. */
  function inspector(session, options = {}) {
    return new Inspector(request => rust.dispatch(request, session).reply, options);
  }

  /**
   * A reader of the session's current bytes (the compare slot's with `compare`),
   * reopened in a hidden session as a reader of a saved file would open them,
   * with the session's password when the bytes still carry its encryption.
   * Null while the session holds no such document.
   */
  function snapshotReader(session, { compare = false, password = null, name = null, layers = [] } = {}) {
    const bytes = rust.withSession(session, (app, handle) => (compare ? app.compare_bytes(handle) : app.snapshot_bytes(handle)), undefined);
    if (!bytes) return null;
    const reader = reopen(bytes, { password, name: name ?? (compare ? 'compare.pdf' : 'snapshot.pdf') });
    // The layers panel's visibility (layer-set) is view-only state the session keeps and
    // never writes; the reader's own view gets the same toggles, as the earlier harness
    // applied them to its copy, so its text reads as the session's render does.
    for (const item of layers) {
      if (item.visible === false) rust.dispatch({ op: 'layer-set', layerId: item.id, visible: false }, reader.owner);
    }
    return reader;
  }

  function createSession() {
    const session = rust.createSession();
    // The test-only bridge view and setters (RustApp session_debug, session_set).
    const debug = () => JSON.parse(rust.withSession(session, (app, handle) => app.session_debug(handle), 'null')) ?? {};
    const set = fields => rust.withSession(session, (app, handle) => app.session_set(handle, JSON.stringify(fields)), false);
    const sourceBytes = Object.getOwnPropertyDescriptor(session, 'sourceBytes').get;
    let cached = null;

    /**
     * The compare slot as the worker protocol describes it, {doc, name, pageCount,
     * sourceBytes}, one object per slot; `doc` reads the slot's bytes, reopened.
     * When the slot is replaced or emptied the reader is released and goes
     * stale, as the earlier engine destroyed the slot's document.
     */
    let compared = null;
    function compareSlot() {
      // An empty slot reports null.
      const slot = debug().compare ?? null;
      if (compared && compared.serial !== slot?.serial) {
        closeDocument(compared.value.doc);
        compared = null;
      }
      if (!slot) return null;
      compared ??= {
        serial: slot.serial,
        value: Object.freeze({
          name: slot.name,
          pageCount: slot.pageCount,
          sourceBytes: slot.sourceBytes,
          doc: snapshotReader(session, { compare: true, name: slot.name }),
        }),
      };
      return compared.value;
    }
    compareChecks.set(session, compareSlot);

    Object.defineProperties(session, {
      doc: {
        enumerable: true,
        get() {
          // One reader per document state keeps identity checks stable and its
          // snapshot current: generation covers open and close, revision every
          // journal step, locked and auth an unlock (which replaces the
          // document without a new revision), securityDirty a security-set, and
          // the layer visibility a layer-set (which changes no revision). A
          // superseded reader is released and goes stale. Null while nothing
          // is open.
          const { generation, revision, locked, auth, journal, password, name, securityDirty } = debug();
          if (!journal) {
            if (cached) closeDocument(cached.doc);
            cached = null;
            return null;
          }
          const layers = locked ? [] : rust.dispatch({ op: 'layers' }, session).reply.items ?? [];
          const key = `${generation}:${revision}:${locked}:${auth}:${securityDirty}:${layers.map(item => +item.visible).join('')}`;
          if (cached?.key === key) return cached.doc;
          if (cached) closeDocument(cached.doc);
          cached = { key, doc: snapshotReader(session, { password: password ?? null, name: name ?? undefined, layers }) };
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
      /** The journal as the engine records it: `{position, steps}`. */
      journal: { enumerable: true, get: () => debug().journal ?? { position: 0, steps: [] } },
      // Stand-ins: tests only compare these with null.
      lockedBytes: { enumerable: true, get: () => byteCount(debug().lockedBytes) },
      signedSource: {
        enumerable: true,
        get: () => byteCount(debug().signedSource),
        // Tests only drop the retained source.
        set(value) {
          if (value !== null || !set({ signedSource: null })) throw new TypeError('signedSource: only null can be assigned');
        },
      },
      batch: { enumerable: true, get: () => debug().batch ?? false },
      compare: { enumerable: true, get: compareSlot },
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

  /**
   * Bytes opened in a session of their own, the way a reader of a saved file
   * opens them: the `open` op and, when the file asks for one, `authenticate`
   * with `password`, refused as they refuse. The reader's hidden session is
   * released by `closeDocument`.
   */
  function reopen(bytes, { password = null, name = 'reopened.pdf' } = {}) {
    const pdf = bytes instanceof ArrayBuffer ? bytes : toArrayBuffer(bytes);
    const session = createSession();
    const opened = rust.dispatch({ id: 1, op: 'open', name, pdf }, session).reply;
    if (opened.ok !== true) {
      release(session);
      throw new Error(opened.error);
    }
    if (password !== null && opened.info?.needsPassword === true) {
      const unlocked = rust.dispatch({ id: 2, op: 'authenticate', password }, session).reply;
      if (unlocked.ok !== true || unlocked.authenticated !== true) {
        release(session);
        throw new Error(unlocked.ok === true ? 'reopen: the password did not unlock the document' : unlocked.error);
      }
    }
    return inspector(session, { owner: session });
  }

  function release(session) {
    rust.dispatch({ op: 'close' }, session);
    rust.withSession(session, (app, handle) => app.drop_session(handle), undefined);
  }

  /**
   * Lets a reader go: its hidden session is closed and dropped, and the
   * reader is stale from then on (every read throws). The session whose
   * `doc` it was keeps its own document.
   */
  function closeDocument(doc) {
    if (!doc || doc.stale) return;
    doc.stale = true;
    if (doc.owner) release(doc.owner);
  }

  return {
    createSession,
    dispatch,
    journalState: rust.journalState,
    withSession: rust.withSession,
    reopen,
    closeDocument,
    maxPdfBytes: MAX_PDF_BYTES,
    maxSaveBytes: MAX_SAVE_BYTES,
  };
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
