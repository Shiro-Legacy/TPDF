// The Rust engine adapter (build/renderer/engine-rust.js). The fake-bridge tests
// always run; the rest need the WASM package and run under `npm run test:rust`
// (TPDF_ENGINE=rust), where engine.js is the Rust engine.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import mupdf from 'mupdf';
import { createEngine } from './rust-engine.mjs';
import { createRustEngine } from '../build/renderer/engine-rust.js';

const skip = process.env.TPDF_ENGINE === 'rust' ? false : 'runs under npm run test:rust';
const STATE = {
  revision: 3, dirty: true, canUndo: true, canRedo: false, generation: 2, savedPosition: 1,
  locked: false, auth: 'user', name: 'x.pdf', sourceBytes: 9,
};

/** A stand-in for the WASM `RustApp` with its queue semantics; `answer` scripts each dispatch. */
function fakeWasm(answer) {
  const log = { apps: 0, calls: [] };
  class FakeApp {
    inputs = [];
    outputs = [];
    panicked = false;
    constructor() { log.apps += 1; }
    guard() {
      // A trapped wasm-bindgen object refuses every later call.
      if (this.panicked) throw new Error('recursive use of an object detected which would lead to unsafe aliasing in rust');
    }
    create_session() { this.guard(); return 7; }
    drop_session() { this.guard(); }
    clear_inputs() { this.guard(); this.inputs = []; }
    push_input(bytes) { this.guard(); this.inputs.push(bytes); }
    dispatch(handle, json) {
      this.guard();
      const inputs = this.inputs;
      this.inputs = [];
      const request = JSON.parse(json);
      log.calls.push({ handle, request, inputs: inputs.map(bytes => [...bytes]) });
      const reply = answer(request, inputs);
      if (reply === 'panic') {
        this.panicked = true;
        throw new WebAssembly.RuntimeError('unreachable');
      }
      this.outputs = reply.outputs ?? [];
      return reply.json;
    }
    output_count() { this.guard(); return this.outputs.length; }
    take_output(index) { this.guard(); const bytes = this.outputs[index]; this.outputs[index] = new Uint8Array(0); return bytes; }
    session_state() { this.guard(); return JSON.stringify(STATE); }
    snapshot_bytes() { this.guard(); return undefined; }
  }
  return { wasm: { RustApp: FakeApp }, log };
}

test('rust adapter: ArrayBuffers cross as placeholders both ways and are transferred', () => {
  const { wasm, log } = fakeWasm(() => ({
    json: JSON.stringify({
      id: 9, ok: true, bytes: { $buffer: 0 }, list: [{ pixels: { $buffer: 1 } }], same: { $input: 0 }, again: { $input: 0 },
    }),
    outputs: [new Uint8Array([1, 2, 3]), new Uint8Array([4])],
  }));
  const engine = createRustEngine(wasm);
  const session = engine.createSession();
  assert.deepEqual(
    [session.handle, session.revision, session.generation, session.savedPosition, session.locked, session.auth, session.name, session.sourceBytes],
    [7, 3, 2, 1, false, 'user', 'x.pdf', 9],
  );
  assert.deepEqual(engine.journalState(session), { revision: 3, dirty: true, canUndo: true, canRedo: false });

  const pdf = new Uint8Array([5, 6]).buffer;
  const image = new Uint8Array([7]).buffer;
  const request = { id: 9, op: 'open', name: 'a\ud800b', pdf, images: [{ name: 'i', bytes: image }], view: new Uint8Array([1]) };
  const { reply, transfer } = engine.dispatch(request, session);
  assert.equal(log.calls.length, 1);
  assert.equal(log.calls[0].handle, 7);
  assert.deepEqual(log.calls[0].request, {
    id: 9, op: 'open', name: 'a�b', pdf: { $buffer: 0 }, images: [{ name: 'i', bytes: { $buffer: 1 } }], view: { 0: 1 },
  });
  assert.deepEqual(log.calls[0].inputs, [[5, 6], [7]]);
  assert.equal(request.pdf, null, 'open drops the request reference, as engine.ts does');
  assert.equal(request.images[0].bytes, image);
  assert.ok(reply.bytes instanceof ArrayBuffer && reply.list[0].pixels instanceof ArrayBuffer);
  assert.deepEqual([...new Uint8Array(reply.bytes)], [1, 2, 3]);
  assert.deepEqual([...new Uint8Array(reply.list[0].pixels)], [4]);
  assert.equal(reply.same, pdf, '$input answers with the caller\'s own buffer');
  assert.equal(reply.again, pdf);
  assert.deepEqual(transfer, [reply.bytes, reply.list[0].pixels, pdf], 'each buffer is transferred once');
});

test('rust adapter: data never reads as a wire marker, and non-finite numbers survive', () => {
  const { wasm, log } = fakeWasm(request => ({ json: JSON.stringify({ id: 1, ok: true, echo: request.data }) }));
  const engine = createRustEngine(wasm);
  const session = engine.createSession();
  const id = { $buffer: 0 };
  const data = { $buffer: 0, nested: [{ $input: 1 }, { $$x: 2 }], n: [NaN, Infinity, -Infinity, -0], ['k\udc00']: 1 };
  const { reply, transfer } = engine.dispatch({ id, op: 'echo', data, pdf: new ArrayBuffer(1) }, session);
  assert.deepEqual(log.calls[0].request.data, {
    $$buffer: 0, nested: [{ $$input: 1 }, { $$$x: 2 }],
    n: [{ $number: 'NaN' }, { $number: 'Infinity' }, { $number: '-Infinity' }, 0], 'k�': 1,
  });
  assert.equal(reply.id, id, 'the reply id is the request\'s own value');
  assert.deepEqual(reply.echo, { $buffer: 0, nested: [{ $input: 1 }, { $$x: 2 }], n: [NaN, Infinity, -Infinity, 0], 'k�': 1 });
  assert.equal(transfer, undefined);

  // A reply marker that names nothing is a Rust bug, reported as a refusal.
  const bad = fakeWasm(() => ({ json: '{"ok":true,"bytes":{"$buffer":0},"x":{"$other":1}}' }));
  const badEngine = createRustEngine(bad.wasm);
  const refused = badEngine.dispatch({ id: 4, op: 'save' }, badEngine.createSession()).reply;
  assert.equal(refused.ok, false);
  assert.equal(refused.id, 4);
  assert.match(refused.error, /names nothing/);
  assert.equal(refused.reason, undefined);
});

test('rust adapter: a Date crosses as its own properties, as engine.ts reads it, not as its toJSON string', () => {
  // engine.ts refuses a Date where it wants a string (search's query, open's name); its toJSON
  // string crossed before and Rust accepted it (integration acceptance review, the wire differential).
  const { wasm, log } = fakeWasm(() => ({ json: JSON.stringify({ id: 1, ok: true }) }));
  const engine = createRustEngine(wasm);
  const when = new Date(0);
  engine.dispatch({ id: 1, op: 'search', query: when, data: { at: [when], name: 'x' } }, engine.createSession());
  assert.deepEqual(log.calls[0].request, { id: 1, op: 'search', query: {}, data: { at: [{}], name: 'x' } });
});

test('rust adapter: request bytes are dropped where engine.ts drops them, and never leak into the next request', () => {
  const { wasm, log } = fakeWasm(() => ({ json: '{"ok":false,"error":"x"}' }));
  const engine = createRustEngine(wasm);
  const session = engine.createSession();
  const bytes = value => new Uint8Array([value]).buffer;

  const unnamed = { id: 1, op: 'open', name: '', pdf: bytes(1) };
  engine.dispatch(unnamed, session);
  assert.ok(unnamed.pdf instanceof ArrayBuffer, 'engine.ts refuses the name before it drops pdf');
  const compare = { id: 2, op: 'compare-open', pdf: bytes(2) };
  engine.dispatch(compare, session);
  assert.ok(compare.pdf instanceof ArrayBuffer);
  for (const [op, field] of [['validate', 'pdf'], ['batch-file', 'bytes'], ['compare-open', 'pdf']]) {
    const request = { id: 3, op, name: 'n.pdf', [field]: bytes(3) };
    engine.dispatch(request, session);
    assert.equal(request[field], null, op);
  }
  const other = { id: 4, op: 'save', pdf: bytes(4) };
  engine.dispatch(other, session);
  assert.ok(other.pdf instanceof ArrayBuffer);

  // A frozen request fails like engine.ts (the drop throws) and queues nothing.
  const calls = log.calls.length;
  const frozen = engine.dispatch(Object.freeze({ id: 5, op: 'open', name: 'a.pdf', pdf: bytes(0xaa) }), session).reply;
  assert.equal(frozen.ok, false);
  assert.equal(frozen.id, 5);
  assert.match(frozen.error, /read.only|Cannot assign/);
  assert.equal(log.calls.length, calls, 'nothing reached Rust');
  engine.dispatch({ id: 6, op: 'open', name: 'b.pdf', pdf: bytes(0xbb) }, session);
  assert.deepEqual(log.calls.at(-1).inputs, [[0xbb]], 'only this request\'s bytes are bound');
});

test('rust adapter: a Rust panic stops its app; its sessions answer reason worker, new sessions get a fresh app', () => {
  const { wasm, log } = fakeWasm(request => (request.op === 'boom' ? 'panic' : { json: JSON.stringify({ id: request.id, ok: true }) }));
  const engine = createRustEngine(wasm);
  const session = engine.createSession();
  assert.equal(engine.dispatch({ id: 1, op: 'close' }, session).reply.ok, true);
  assert.equal(session.revision, 3);

  const boom = engine.dispatch({ id: 2, op: 'boom' }, session).reply;
  assert.deepEqual(boom, {
    id: 2, ok: false, reason: 'worker', error: 'the PDF engine stopped after an internal error (unreachable)',
    revision: 3, dirty: true, canUndo: true, canRedo: false,
  });
  const after = engine.dispatch({ id: 3, op: 'close' }, session).reply;
  assert.equal(after.reason, 'worker');
  assert.match(after.error, /unreachable/, 'the first cause, not the poisoned-borrow error');
  assert.equal(log.calls.length, 2, 'a stopped app is never called again');
  // Getters and journalState keep the last state read and never throw.
  assert.equal(session.revision, 3);
  assert.deepEqual(engine.journalState(session), { revision: 3, dirty: true, canUndo: true, canRedo: false });
  assert.equal(engine.withSession(session, app => app.snapshot_bytes(1), 'stopped'), 'stopped');

  const fresh = engine.createSession();
  assert.equal(log.apps, 2);
  assert.equal(engine.dispatch({ id: 4, op: 'close' }, fresh).reply.ok, true);
  assert.equal(engine.dispatch({ id: 5, op: 'close' }, session).reply.reason, 'worker', 'the old session stays stopped');
});

test('rust adapter: a request JSON cannot carry is a refusal, not a throw', () => {
  const { wasm, log } = fakeWasm(() => ({ json: '{"ok":true}' }));
  const engine = createRustEngine(wasm);
  const session = engine.createSession();
  const cyclic = { id: 10, op: 'close' };
  cyclic.self = cyclic;
  const refused = engine.dispatch(cyclic, session);
  assert.equal(refused.transfer, undefined);
  assert.equal(refused.reply.id, 10);
  assert.equal(refused.reply.ok, false);
  assert.match(refused.reply.error, /circular/i);
  assert.equal(refused.reply.revision, 3);
  assert.match(engine.dispatch({ id: 11, op: 'close', n: 1n }, session).reply.error, /BigInt/);
  assert.equal(log.calls.length, 0);
  assert.equal(engine.dispatch({ op: 'close' }, session).reply.id, undefined);
  assert.ok(Object.hasOwn(engine.dispatch({ op: 'close' }, session).reply, 'id'), 'engine.ts always sets id');
});

test('rust engine: close and an unknown op through the WASM bridge', { skip }, () => {
  const engine = createEngine(mupdf);
  const session = engine.createSession();
  assert.equal(typeof session.handle, 'number', 'engine.js resolved to the Rust engine');
  assert.equal(session.doc, null);
  assert.deepEqual(engine.journalState(session), { revision: 0, dirty: false, canUndo: false, canRedo: false });
  assert.deepEqual(engine.dispatch({ id: 1, op: 'close' }, session), { reply: { id: 1, ok: true } });
  assert.equal(session.generation, 1);
  const other = engine.createSession();
  engine.dispatch({ id: 2, op: 'close' }, other);
  assert.equal(other.generation, 2, 'one generation counter per engine');

  const request = { id: 3, op: 'nope', pdf: new ArrayBuffer(8), images: [{ bytes: new ArrayBuffer(2) }], text: 'a\ud800' };
  assert.deepEqual(engine.dispatch(request, session), {
    reply: { id: 3, ok: false, error: 'unknown op: nope', revision: 0, dirty: false, canUndo: false, canRedo: false },
  });
  assert.ok(request.pdf instanceof ArrayBuffer, 'only open, compare-open and validate drop pdf');
  assert.equal(engine.dispatch({ op: 7 }, session).reply.error, 'unknown op: 7');
  assert.equal(engine.dispatch({ op: NaN }, session).reply.error, 'unknown op: NaN');
  assert.equal(engine.dispatch(null, session).reply.error, 'unknown op: undefined');

  const open = { id: 4, op: 'open', name: 'a.pdf', pdf: new ArrayBuffer(8) };
  assert.equal(engine.dispatch(open, session).reply.id, 4);
  assert.equal(open.pdf, null);
  assert.deepEqual(
    { name: session.name, locked: session.locked, auth: session.auth, sourceBytes: session.sourceBytes, savedPosition: session.savedPosition },
    { name: null, locked: false, auth: 'none', sourceBytes: 0, savedPosition: 0 },
  );
});

test('rust engine: the WASM wire keeps doubles, ids, markers and odd keys intact', { skip }, async () => {
  const pkg = await import('../engine/web/pkg/tandem_rust_workflow.js');
  pkg.initSync({ module: readFileSync(new URL('../engine/web/pkg/tandem_rust_workflow_bg.wasm', import.meta.url)) });
  const queue = [];
  class SpyApp extends pkg.RustApp {
    push_input(bytes) { queue.push([...bytes]); return super.push_input(bytes); }
    clear_inputs() { queue.push('clear'); return super.clear_inputs(); }
  }
  const engine = createRustEngine({ RustApp: SpyApp });
  const session = engine.createSession();

  // Rust prints the op it parsed: the same double JavaScript sent.
  assert.equal(engine.dispatch({ id: 1, op: 23.172468304866854 }, session).reply.error, 'unknown op: 23.172468304866854');
  assert.equal(engine.dispatch({ id: 1, op: 273535373942.828125 }, session).reply.error, `unknown op: ${273535373942.828125}`);
  const id = { $buffer: 0 };
  const echoed = engine.dispatch({ id, op: 'nope', pdf: new ArrayBuffer(1) }, session);
  assert.equal(echoed.reply.id, id);
  assert.equal(echoed.transfer, undefined);
  assert.equal(engine.dispatch({ op: { $number: 'NaN' } }, session).reply.error, 'unknown op: [object Object]');

  // A lone surrogate in a key no longer breaks the request; close runs.
  assert.deepEqual(engine.dispatch({ id: 7, op: 'close', sanitize: { ['k\ud800']: true } }, session).reply, { id: 7, ok: true });
  // serde_json's depth limit is a refusal for the same id.
  let deep = [];
  for (let i = 0; i < 130; i++) deep = [deep];
  const tooDeep = engine.dispatch({ id: 8, op: 'close', deep }, session).reply;
  assert.equal(tooDeep.id, 8);
  assert.match(tooDeep.error, /recursion limit/);

  // A frozen request queues nothing; the next request clears the queue first anyway.
  queue.length = 0;
  const frozen = Object.freeze({ id: 9, op: 'open', name: 'a.pdf', pdf: new Uint8Array([0xaa]).buffer });
  assert.equal(engine.dispatch(frozen, session).reply.ok, false);
  engine.dispatch({ id: 10, op: 'open', name: 'b.pdf', pdf: new Uint8Array([0xbb]).buffer }, session);
  assert.deepEqual(queue, ['clear', [0xbb]]);
});
