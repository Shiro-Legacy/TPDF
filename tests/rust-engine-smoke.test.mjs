// The Rust engine adapter (build/renderer/engine-rust.js). The first test drives a
// fake bridge and always runs; the rest need the WASM package and run under
// `npm run test:rust` (TPDF_ENGINE=rust), where engine.js is the Rust engine.
import test from 'node:test';
import assert from 'node:assert/strict';
import mupdf from 'mupdf';
import { createEngine } from '../build/renderer/engine.js';
import { createRustEngine } from '../build/renderer/engine-rust.js';

const skip = process.env.TPDF_ENGINE === 'rust' ? false : 'runs under npm run test:rust';

test('rust adapter: ArrayBuffers cross as placeholders both ways and are transferred', () => {
  const calls = [];
  class FakeApp {
    inputs = [];
    outputs = [];
    create_session() { return 7; }
    drop_session() {}
    push_input(bytes) { this.inputs.push(bytes); }
    dispatch(handle, json) {
      calls.push({ handle, request: JSON.parse(json), inputs: this.inputs });
      this.inputs = [];
      this.outputs = [new Uint8Array([1, 2, 3]), new Uint8Array([4])];
      return JSON.stringify({
        id: 9, ok: true, bytes: { $buffer: 0 }, list: [{ pixels: { $buffer: 1 } }], other: { $buffer: 0, x: 1 },
      });
    }
    output_count() { return this.outputs.length; }
    take_output(index) { return this.outputs[index]; }
    session_state() {
      return JSON.stringify({
        revision: 3, dirty: true, canUndo: true, canRedo: false, generation: 2, savedPosition: 1,
        locked: false, auth: 'user', name: 'x.pdf', sourceBytes: 9,
      });
    }
    snapshot_bytes() { return undefined; }
  }
  const engine = createRustEngine({ RustApp: FakeApp });
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
  assert.equal(calls.length, 1);
  assert.equal(calls[0].handle, 7);
  assert.deepEqual(calls[0].request, {
    id: 9, op: 'open', name: 'a�b', pdf: { $buffer: 0 }, images: [{ name: 'i', bytes: { $buffer: 1 } }], view: { 0: 1 },
  });
  assert.deepEqual(calls[0].inputs.map(bytes => [...bytes]), [[5, 6], [7]]);
  assert.equal(request.pdf, null, 'open drops the request reference, as engine.ts does');
  assert.equal(request.images[0].bytes, image);
  assert.ok(reply.bytes instanceof ArrayBuffer && reply.list[0].pixels instanceof ArrayBuffer);
  assert.deepEqual([...new Uint8Array(reply.bytes)], [1, 2, 3]);
  assert.deepEqual([...new Uint8Array(reply.list[0].pixels)], [4]);
  assert.deepEqual(reply.other, { $buffer: 0, x: 1 }, 'only an exact placeholder is a buffer');
  assert.deepEqual(transfer, [reply.bytes, reply.list[0].pixels]);

  // A request JSON cannot carry is a refusal reply, not a throw.
  const cyclic = { id: 10, op: 'close' };
  cyclic.self = cyclic;
  const refused = engine.dispatch(cyclic, session);
  assert.equal(refused.transfer, undefined);
  assert.equal(refused.reply.id, 10);
  assert.equal(refused.reply.ok, false);
  assert.match(refused.reply.error, /circular/i);
  assert.equal(refused.reply.revision, 3);
  assert.equal(calls.length, 1);
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
  assert.equal(engine.dispatch(null, session).reply.error, 'unknown op: undefined');

  const open = { id: 4, op: 'open', name: 'a.pdf', pdf: new ArrayBuffer(8) };
  assert.equal(engine.dispatch(open, session).reply.id, 4);
  assert.equal(open.pdf, null);
  assert.deepEqual(
    { name: session.name, locked: session.locked, auth: session.auth, sourceBytes: session.sourceBytes, savedPosition: session.savedPosition },
    { name: null, locked: false, auth: 'none', sourceBytes: 0, savedPosition: 0 },
  );
});
