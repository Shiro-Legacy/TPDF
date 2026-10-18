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
import type { AuthLevel, DispatchResult, JournalState, SessionRequest } from "./protocol.js";

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

/** The package's CJK font functions: module-wide, one font for every `RustApp` (engine/design/app-view.md). */
export type CjkFontRoute = {
  set_cjk_font(bytes: Uint8Array): boolean;
  cjk_font_wanted(): boolean;
  has_cjk_font(): boolean;
};

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

/**
 * The request as wire JSON; its `ArrayBuffer`s are appended to `inputs`. An
 * object with `toJSON` (a `Date`, the only one structured clone carries)
 * crosses as its own properties, as engine.ts reads it, not as the string
 * `toJSON` makes: engine.ts refuses a `Date` where it wants a string.
 */
function encode(request: unknown, inputs: ArrayBuffer[]): string {
  return JSON.stringify(request, function (this: unknown, key: string, converted: unknown) {
    const original = (this as Record<string, unknown>)[key];
    const value = original !== converted && typeof original === "object" && original !== null
      && typeof (original as { toJSON?: unknown }).toJSON === "function"
      ? { ...original }
      : converted;
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

/**
 * Reply fields engine.ts times. `std::time::Instant` panics on
 * wasm32-unknown-unknown, so Rust reports 0 and the adapter times the dispatch
 * (render's `paintMs` too, which then includes its text extraction).
 */
const TIMED = new Map<unknown, string[]>([
  ["open", ["openMs"]], ["save", ["saveMs"]], ["render", ["paintMs", "renderMs"]], ["search", ["searchMs"]],
  ["compare-render", ["paintMs", "renderMs"]],
]);

/**
 * The bytes Rust gets for a request buffer. engine.ts sizes a buffer by its
 * `byteLength`; one whose contents cannot be read at that length (a Proxy)
 * crosses as that many zero bytes, so Rust's size checks see the same size.
 */
function readable(buffer: ArrayBuffer): Uint8Array {
  if (!buffer.byteLength) return new Uint8Array(0);
  const bytes = new Uint8Array(buffer);
  return bytes.byteLength === buffer.byteLength ? bytes : new Uint8Array(buffer.byteLength);
}

/** Ops whose output may be encrypted get 32 fresh random bytes (writer.rs `Entropy`). */
const ENTROPY_OPS = new Set<unknown>(["save", "redact-apply"]);

/** The worker's local date, `YYYY-MM-DD`: engine.ts dates a signature appearance with it; Rust has no time zone. */
function localDate(now = new Date()): string {
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

/** A caller's own `today` (M9: the renderer records it so a replay dates as the original did). */
const DATE = /^\d{4}-\d{2}-\d{2}$/;

/**
 * The request Rust sees: the caller's, plus `entropy` where the op needs it,
 * `today` for annot-add, and for sign-prepare the worker's UTC offset at
 * `when` (Rust has no time zone; appearances show the signer's local time).
 * A caller that sends its own `today` (a `YYYY-MM-DD` string) or `entropy`
 * (32 bytes) keeps them: the renderer injects both into the requests it logs
 * for recovery, so a replay after a worker restart writes the same bytes
 * (engine/design/app-m9-recovery.md).
 */
function wire(request: unknown): unknown {
  if (plainObject(request) && request.op === "sign-prepare" && typeof request.when === "number") {
    return { ...request, timezoneOffset: new Date(request.when).getTimezoneOffset() };
  }
  if (plainObject(request) && request.op === "annot-add") {
    return { ...request, today: typeof request.today === "string" && DATE.test(request.today) ? request.today : localDate() };
  }
  if (!plainObject(request) || !ENTROPY_OPS.has(request.op)) return request;
  if (request.entropy instanceof ArrayBuffer && request.entropy.byteLength === 32) return request;
  return { ...request, entropy: crypto.getRandomValues(new Uint8Array(32)).buffer };
}

/** Ops that answer with pixels: drawn again once the CJK font a render asked for is installed. */
const PAINT_OPS = new Set<unknown>(["render", "thumbnail", "compare-render"]);
/** Ops whose own checks draw pages: the CJK font is installed before them, as their pixels are not returned. */
const PRELOAD_OPS = new Set<unknown>(["redact-apply"]);
/**
 * engine/src/app/ops/scan.rs `MAX_SCAN_ITEMS`, checked here as well: refusing past it in Rust came only after the
 * request was encoded and parsed, which took 33 s for 400,000 boxes.
 */
const MAX_SCAN_ITEMS = 40_000;

/** A scan-compose request's runs and boxes, counted without encoding it. */
function scanItems(request: unknown): number {
  const pages = plainObject(request) && Array.isArray(request.pages) ? request.pages : [];
  let items = 0;
  for (const page of pages) {
    if (!plainObject(page)) continue;
    for (const key of ["runs", "boxes"]) {
      const list = page[key];
      if (Array.isArray(list)) items += list.length;
    }
  }
  return items;
}

/** Ops refused (`no-font`) while text they lay out needs the CJK font: dispatched again once it is installed. */
const FONT_OPS = new Set<unknown>(["scan-compose", "scan-measure"]);

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
  let cjk: { route: CjkFontRoute; read: () => Uint8Array | null; settled: boolean } | null = null;
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

  /**
   * The CJK fallback font (Droid Sans Fallback, scripts/cjk-font.mjs) for text whose font a PDF does not
   * embed: `read` gives its bytes, synchronously, the first time a render draws CJK text without it (or
   * before an op whose own checks draw pages), or null when it cannot be read. It is read at most once.
   */
  function useCjkFont(route: CjkFontRoute, read: () => Uint8Array | null): void {
    cjk = { route, read, settled: false };
  }

  /**
   * Installs the CJK font when a render asked for it (`cjk_font_wanted`) or, with `preload`, when none is
   * installed. True when the engine took it now: a render that drew without it then runs again. A failed
   * read or a refused font is not retried, and such text stays undrawn.
   */
  function installCjkFont(instance: Instance, preload: boolean): boolean {
    if (!cjk || cjk.settled) return false;
    const { route, read } = cjk;
    if (!call(instance, () => (preload ? !route.has_cjk_font() : route.cjk_font_wanted()))) return false;
    cjk.settled = true;
    const bytes = attempt(read, null);
    return !!bytes && call(instance, () => route.set_cjk_font(bytes));
  }

  /** One dispatch of the wire request: its reply (without `id`) and the buffers to transfer. */
  function exchange(instance: Instance, handle: number, json: string, inputs: ArrayBuffer[]): DispatchResult {
    const text = call(instance, app => {
      // Nothing queued by an earlier request may bind to this one's placeholders.
      app.clear_inputs();
      // A detached buffer reads as empty, as engine.ts sees it.
      for (const buffer of inputs) app.push_input(readable(buffer));
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
      // `$bytes` is an output a reply holds as a Uint8Array (a MuPDF byte string in engine.ts).
      if ((key === "$buffer" || key === "$bytes") && index >= 0 && index < outputs && !taken.has(index)) {
        taken.add(index);
        // wasm-bindgen has already copied the output out of WASM memory.
        const bytes = call(instance, app => app.take_output(index));
        const buffer = keep(bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength
          ? bytes.buffer as ArrayBuffer
          : bytes.slice().buffer);
        return key === "$bytes" ? new Uint8Array(buffer) : buffer;
      }
      throw new Error(`rust engine: reply marker ${key} ${JSON.stringify(payload)} names nothing`);
    });
    return transfer.length ? { reply, transfer } : { reply };
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
      const op = (request as { op?: unknown } | null | undefined)?.op;
      if (op === "scan-compose" && scanItems(request) > MAX_SCAN_ITEMS) {
        // The engine's own refusal, without the cost of sending the request.
        const error = `scan-compose: a scan has at most ${MAX_SCAN_ITEMS} runs and boxes in all`;
        return { reply: { id, ok: false, error, reason: "too-large", detail: "runs", ...journalState(session) } };
      }
      const inputs: ArrayBuffer[] = [];
      const json = encode(wire(request), inputs);
      release(request);
      const started = performance.now();
      if (PRELOAD_OPS.has(op)) installCjkFont(instance, true);
      let result = exchange(instance, handle, json, inputs);
      // Renders are not journaled, so drawing again with the font is the reply the request should have had;
      // the scan ops change nothing either, so their second dispatch is the reply too.
      if (installCjkFont(instance, false) && ((PAINT_OPS.has(op) && result.reply.ok === true) || FONT_OPS.has(op))) {
        result = exchange(instance, handle, json, inputs);
      }
      // The time includes the font's install and the first draw: what the caller waited.
      const elapsed = Math.round(performance.now() - started);
      const { reply } = result;
      reply.id = id;
      for (const timed of TIMED.get(op) ?? []) {
        if (reply.ok === true && typeof reply[timed] === "number") reply[timed] = elapsed;
      }
      return result;
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

  return { createSession, dispatch, journalState, withSession, useCjkFont };
}
