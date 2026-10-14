/**
 * The worker engine on the Rust application engine (engine/bridge `RustApp`).
 * Rust owns validation, refusals and reply shapes; this adapter only moves
 * requests and replies across as JSON. Wire format (engine/APP-ENGINE-DESIGN.md):
 * an `ArrayBuffer` crosses as a `{"$buffer": n}` placeholder into buffers passed
 * beside the JSON, a reply hands a request buffer back as `{"$input": n}`, a
 * non-finite number crosses as `{"$number": "NaN"}`, and data keys that begin
 * with `$` carry one more `$`, so data never reads as a marker.
 */
import { attempt, errorMessage } from "./engine-core.js";
import type { AuthLevel, DispatchResult, JournalState, SessionRequest } from "./engine.js";

/** An instance of the wasm-bindgen package's `RustApp`. */
export type RustApp = {
  create_session(): number;
  drop_session(session: number): void;
  clear_inputs(): void;
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

/**
 * One `RustApp`. A Rust panic traps, and wasm-bindgen then refuses every later
 * call on that object, so the first call that throws stops it for good.
 */
type Instance = { app: RustApp; stopped: string | null };
type Binding = { instance: Instance; handle: number; state: RustState };

/** Thrown for a call on a stopped `RustApp`; the reply says `reason: "worker"`. */
class EngineStopped extends Error {}

const INITIAL: RustState = {
  revision: 0, dirty: false, canUndo: false, canRedo: false,
  generation: 0, savedPosition: 0, locked: false, auth: "none", name: null, sourceBytes: 0,
};

/**
 * Rust's JSON reader refuses lone surrogates, so they cross as U+FFFD. MuPDF's
 * UTF-8 conversion merged them with the next code unit instead; U+FFFD is the
 * deliberate improvement.
 */
const LONE_SURROGATES = /\p{Cs}/gu;
const LONE_SURROGATE = /\p{Cs}/u;

function clean(text: string): string {
  return text.replace(LONE_SURROGATES, "�");
}

function plainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** The request as wire JSON; its `ArrayBuffer`s are appended to `inputs`. */
function encode(request: unknown, inputs: ArrayBuffer[]): string {
  return JSON.stringify(request, (_key, value: unknown) => {
    if (typeof value === "string") return clean(value);
    if (typeof value === "number") return Number.isFinite(value) ? value : { $number: String(value) };
    if (value instanceof ArrayBuffer) return { $buffer: inputs.push(value) - 1 };
    if (!plainObject(value)) return value;
    const keys = Object.keys(value);
    if (!keys.some(key => key.startsWith("$") || LONE_SURROGATE.test(key))) return value;
    return Object.fromEntries(keys.map(key => [clean(key.startsWith("$") ? `$${key}` : key), value[key]]));
  }) ?? "null";
}

/** The wire reply as a reply; `marker` resolves each `{"$...": payload}` marker. */
function decode(text: string, marker: (key: string, payload: unknown) => unknown): Record<string, unknown> {
  return JSON.parse(text, (_key, value: unknown) => {
    if (!plainObject(value)) return value;
    const keys = Object.keys(value);
    const single = keys.filter(key => key.startsWith("$") && !key.startsWith("$$"));
    if (single.length === 1 && keys.length === 1) return marker(single[0], value[single[0]]);
    if (single.length) throw new Error(`rust engine: reply key ${single[0]} is neither a marker nor escaped`);
    if (!keys.some(key => key.startsWith("$"))) return value;
    return Object.fromEntries(keys.map(key => [key.startsWith("$") ? key.slice(1) : key, value[key]]));
  }) as Record<string, unknown>;
}

/** engine.ts drops the request's reference to its input bytes at these points; Rust holds its own copy. */
function release(request: unknown): void {
  if (!plainObject(request)) return;
  const op = request.op;
  const named = typeof request.name === "string" && request.name !== "";
  if (op === "validate" || ((op === "open" || op === "compare-open") && named)) request.pdf = null;
  if (op === "batch-file") request.bytes = null;
}

export function createRustEngine(wasm: RustWasm) {
  let current: Instance = { app: new wasm.RustApp(), stopped: null };
  const bindings = new WeakMap<RustSession, Binding>();
  // A session the worker lets go of frees its Rust document too.
  const sessions = new FinalizationRegistry<{ instance: Instance; handle: number }>(({ instance, handle }) => {
    attempt(() => call(instance, app => app.drop_session(handle)), undefined);
  });

  function call<T>(instance: Instance, work: (app: RustApp) => T): T {
    if (instance.stopped !== null) throw new EngineStopped(instance.stopped);
    try {
      return work(instance.app);
    } catch (err) {
      instance.stopped = errorMessage(err);
      throw new EngineStopped(instance.stopped);
    }
  }

  /** The session's state from Rust, or the last state read once its app has stopped. */
  function state(binding: Binding): RustState {
    const text = attempt(() => call(binding.instance, app => app.session_state(binding.handle)), "null");
    const read = attempt(() => JSON.parse(text) as RustState | null, null);
    if (read) binding.state = read;
    return binding.state;
  }

  function createSession(): RustSession {
    // Sessions of a stopped app stay stopped; a new one gets a fresh app.
    if (current.stopped !== null) current = { app: new wasm.RustApp(), stopped: null };
    const instance = current;
    const handle = call(instance, app => app.create_session());
    const binding: Binding = { instance, handle, state: { ...INITIAL } };
    const read = () => state(binding);
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
    bindings.set(session, binding);
    sessions.register(session, { instance, handle });
    return session;
  }

  /** Never throws, like engine.ts `journalState`. */
  function journalState(session: RustSession): JournalState {
    const binding = bindings.get(session);
    const { revision, dirty, canUndo, canRedo } = binding ? state(binding) : INITIAL;
    return { revision, dirty, canUndo, canRedo };
  }

  /** Never throws, like engine.ts `dispatch`; the reply's `id` is the request's own. */
  function dispatch(request: SessionRequest, session: RustSession): DispatchResult {
    const id = (request as { id?: unknown } | null | undefined)?.id;
    try {
      const binding = bindings.get(session);
      if (!binding) throw new Error("dispatch: not a session of this engine");
      const { instance, handle } = binding;
      const inputs: ArrayBuffer[] = [];
      const json = encode(request, inputs);
      release(request);
      const text = call(instance, app => {
        // Nothing queued by an earlier request may bind to this one's placeholders.
        app.clear_inputs();
        // A detached buffer reads as empty, as engine.ts sees it.
        for (const buffer of inputs) app.push_input(buffer.byteLength ? new Uint8Array(buffer) : new Uint8Array(0));
        return app.dispatch(handle, json);
      });
      const outputs = call(instance, app => app.output_count());
      const taken = new Set<number>();
      const transfer: ArrayBuffer[] = [];
      const keep = (buffer: ArrayBuffer) => {
        if (!transfer.includes(buffer)) transfer.push(buffer);
        return buffer;
      };
      const reply = decode(text, (key, payload) => {
        if (key === "$number" && (payload === "NaN" || payload === "Infinity" || payload === "-Infinity")) return Number(payload);
        const index = typeof payload === "number" && Number.isInteger(payload) && payload >= 0 ? payload : -1;
        if (key === "$input" && index >= 0 && index < inputs.length) return keep(inputs[index]);
        if (key === "$buffer" && index >= 0 && index < outputs && !taken.has(index)) {
          taken.add(index);
          // wasm-bindgen has already copied the output out of WASM memory.
          const bytes = call(instance, app => app.take_output(index));
          return keep(bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength
            ? bytes.buffer as ArrayBuffer
            : bytes.slice().buffer);
        }
        throw new Error(`rust engine: reply marker ${key} ${JSON.stringify(payload)} names nothing`);
      });
      reply.id = id;
      return transfer.length ? { reply, transfer } : { reply };
    } catch (err) {
      // A request JSON cannot carry (a BigInt, a cycle), a frozen request, or a stopped app.
      const stopped = err instanceof EngineStopped;
      return {
        reply: {
          id, ok: false,
          error: stopped ? `the PDF engine stopped after an internal error (${errorMessage(err)})` : errorMessage(err),
          ...(stopped ? { reason: "worker" } : {}),
          ...journalState(session),
        },
      };
    }
  }

  /**
   * Test inspection: runs `use` against the session's `RustApp` and returns
   * `fallback` once that app has stopped. `use` should only call the app.
   */
  function withSession<T>(session: RustSession, use: (app: RustApp, handle: number) => T, fallback: T): T {
    const binding = bindings.get(session);
    if (!binding) return fallback;
    return attempt(() => call(binding.instance, app => use(app, binding.handle)), fallback);
  }

  return { createSession, dispatch, journalState, withSession };
}
