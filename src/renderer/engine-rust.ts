/**
 * The worker engine on the Rust application engine (engine/bridge `RustApp`).
 * Rust owns validation, refusals and reply shapes; this adapter only moves
 * requests and replies across as JSON. Every `ArrayBuffer` crosses as a
 * `{"$buffer": n}` placeholder into buffers passed beside the JSON, both ways.
 */
import { attempt } from "./engine-core.js";
import type { AuthLevel, DispatchResult, JournalState, SessionRequest } from "./engine.js";

/** An instance of the wasm-bindgen package's `RustApp`. */
export type RustApp = {
  create_session(): number;
  drop_session(session: number): void;
  push_input(bytes: Uint8Array): void;
  dispatch(session: number, request: string): string;
  output_count(): number;
  take_output(index: number): Uint8Array;
  session_state(session: number): string;
  snapshot_bytes(session: number): Uint8Array | undefined;
};

/** The initialised wasm-bindgen package (`tandem_rust_workflow.js`). */
export type RustWasm = { RustApp: new () => RustApp };

type RustState = JournalState & {
  generation: number;
  savedPosition: number;
  locked: boolean;
  auth: AuthLevel;
  name: string | null;
  sourceBytes: number;
};

/** A Rust session handle; each field is read from Rust when accessed. */
export type RustSession = {
  readonly handle: number;
  readonly revision: number;
  readonly generation: number;
  readonly savedPosition: number;
  readonly locked: boolean;
  readonly auth: AuthLevel;
  readonly name: string | null;
  readonly sourceBytes: number;
};

/** Rust's JSON reader refuses lone surrogates; MuPDF's UTF-8 conversion turned them into U+FFFD too. */
const LONE_SURROGATE = /\p{Cs}/gu;

/** The index of a `{"$buffer": n}` placeholder, or null. */
function bufferIndex(value: unknown): number | null {
  if (typeof value !== "object" || value === null || Array.isArray(value) || Object.keys(value).length !== 1) return null;
  const index = (value as { $buffer?: unknown }).$buffer;
  return typeof index === "number" && Number.isInteger(index) && index >= 0 ? index : null;
}

export function createRustEngine(wasm: RustWasm) {
  const app = new wasm.RustApp();
  // A session the worker lets go of frees its Rust document too.
  const sessions = new FinalizationRegistry<number>(handle => app.drop_session(handle));

  function createSession(): RustSession {
    const handle = app.create_session();
    const read = () => JSON.parse(app.session_state(handle)) as RustState;
    const session: RustSession = {
      handle,
      get revision() { return read().revision; },
      get generation() { return read().generation; },
      get savedPosition() { return read().savedPosition; },
      get locked() { return read().locked; },
      get auth() { return read().auth; },
      get name() { return read().name; },
      get sourceBytes() { return read().sourceBytes; },
    };
    sessions.register(session, handle);
    return session;
  }

  function journalState(session: RustSession): JournalState {
    const { revision, dirty, canUndo, canRedo } = JSON.parse(app.session_state(session.handle)) as RustState;
    return { revision, dirty, canUndo, canRedo };
  }

  /** Never throws, like engine.ts `dispatch`. */
  function dispatch(request: SessionRequest, session: RustSession): DispatchResult {
    const fields = request as unknown as Record<string, unknown> | null;
    try {
      const inputs: ArrayBuffer[] = [];
      const json = JSON.stringify(request, (_key, value: unknown) => {
        if (value instanceof ArrayBuffer) return { $buffer: inputs.push(value) - 1 };
        return typeof value === "string" ? value.replace(LONE_SURROGATE, "�") : value;
      }) ?? "null";
      // A detached buffer reads as empty, as engine.ts sees it.
      for (const buffer of inputs) app.push_input(buffer.byteLength ? new Uint8Array(buffer) : new Uint8Array(0));
      // engine.ts drops the request's reference to the input bytes; Rust has its own copy.
      const op = fields?.op;
      if (fields && (op === "open" || op === "compare-open" || op === "validate")) fields.pdf = null;
      if (fields && op === "batch-file") fields.bytes = null;
      const transfer: ArrayBuffer[] = [];
      const reply = JSON.parse(app.dispatch(session.handle, json), (_key, value: unknown) => {
        const index = bufferIndex(value);
        if (index === null) return value;
        // wasm-bindgen has already copied the output out of WASM memory.
        const bytes = app.take_output(index);
        const buffer = bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength
          ? bytes.buffer as ArrayBuffer
          : bytes.slice().buffer;
        transfer.push(buffer);
        return buffer;
      }) as Record<string, unknown>;
      return transfer.length ? { reply, transfer } : { reply };
    } catch (err) {
      // Only a request JSON cannot carry (a BigInt, a cycle) or a WASM trap gets here.
      return {
        reply: {
          id: fields?.id, ok: false, error: err instanceof Error ? err.message : String(err),
          ...attempt<Partial<JournalState>>(() => journalState(session), {}),
        },
      };
    }
  }

  // `app` is exposed for inspection; the test harness reads `snapshot_bytes` through it.
  return { app, createSession, dispatch, journalState };
}
