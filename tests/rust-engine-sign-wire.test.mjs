// The sign ops' wire contract on the Rust adapter (build/renderer/engine-rust.js):
// a `{"$bytes": n}` reply marker is a Uint8Array (verify.ts wants one), and
// sign-prepare carries the worker's UTC offset at `when`, since Rust has no time
// zone. The fake-bridge tests always run; the last one runs on whichever engine
// engine.js resolves to (MuPDF, or Rust under npm run test:rust).
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import mupdf from 'mupdf';
import { createEngine } from './rust-engine.mjs';
import { createRustEngine } from '../build/renderer/engine-rust.js';

// A zone with daylight saving and a negative UTC offset, so the stamp differs
// from UTC in hour, date and offset sign.
process.env.TZ = 'America/Chicago';

const STATE = {
  revision: 0, dirty: false, canUndo: false, canRedo: false, generation: 1, savedPosition: 0,
  locked: false, auth: 'none', name: null, sourceBytes: 0,
};

/** A stand-in for the WASM `RustApp`; `answer` scripts each dispatch. */
function fakeWasm(answer) {
  const log = { calls: [] };
  class FakeApp {
    inputs = [];
    outputs = [];
    create_session() { return 1; }
    drop_session() {}
    clear_inputs() { this.inputs = []; }
    push_input(bytes) { this.inputs.push(bytes); }
    dispatch(handle, json) {
      const request = JSON.parse(json);
      log.calls.push(request);
      const reply = answer(request);
      this.outputs = reply.outputs ?? [];
      return reply.json;
    }
    output_count() { return this.outputs.length; }
    take_output(index) { const bytes = this.outputs[index]; this.outputs[index] = new Uint8Array(0); return bytes; }
    session_state() { return JSON.stringify(STATE); }
    snapshot_bytes() { return undefined; }
  }
  return { wasm: { RustApp: FakeApp }, log };
}

test('rust adapter: $bytes is a transferred Uint8Array and shares the $buffer index space', () => {
  const { wasm } = fakeWasm(() => ({
    json: JSON.stringify({ ok: true, bytes: { $buffer: 0 }, signatures: [{ contents: { $bytes: 1 } }, { contents: { $bytes: 2 } }] }),
    outputs: [new Uint8Array([1]), new Uint8Array([0x30, 0x82]), new Uint8Array(new ArrayBuffer(8), 2, 3).fill(9)],
  }));
  const engine = createRustEngine(wasm);
  const { reply, transfer } = engine.dispatch({ id: 1, op: 'sign-list' }, engine.createSession());
  assert.equal(reply.ok, true);
  assert.ok(reply.bytes instanceof ArrayBuffer);
  const [first, second] = reply.signatures.map(entry => entry.contents);
  assert.ok(first instanceof Uint8Array && second instanceof Uint8Array);
  assert.deepEqual([...first], [0x30, 0x82]);
  // An output that is a view into a larger buffer is copied to its own.
  assert.deepEqual([...second], [9, 9, 9]);
  assert.equal(second.byteOffset, 0);
  assert.equal(second.buffer.byteLength, 3);
  assert.deepEqual(transfer, [reply.bytes, first.buffer, second.buffer]);

  // Each output is taken once, under either marker.
  for (const json of ['{"ok":true,"a":{"$bytes":0},"b":{"$buffer":0}}', '{"ok":true,"a":{"$buffer":0},"b":{"$bytes":0}}', '{"ok":true,"a":{"$bytes":1}}']) {
    const twice = createRustEngine(fakeWasm(() => ({ json, outputs: [new Uint8Array([1])] })).wasm);
    const refused = twice.dispatch({ id: 2, op: 'sign-list' }, twice.createSession());
    assert.equal(refused.reply.ok, false, json);
    assert.match(refused.reply.error, /names nothing/);
    assert.equal(refused.transfer, undefined);
  }
});

test('rust adapter: sign-prepare alone carries the UTC offset at its own time', () => {
  const { wasm, log } = fakeWasm(request => ({ json: JSON.stringify({ id: request.id, ok: true }) }));
  const engine = createRustEngine(wasm);
  const session = engine.createSession();
  const summer = Date.UTC(2026, 9, 14, 12);
  const winter = Date.UTC(2026, 0, 15, 3, 30);
  const request = { id: 1, op: 'sign-prepare', name: 'A', reason: '', page: 0, rect: null, certify: false, when: summer };
  engine.dispatch(request, session);
  engine.dispatch({ ...request, id: 2, when: winter }, session);
  assert.deepEqual(log.calls.map(call => call.timezoneOffset), [300, 360], 'CDT and CST: UTC minus local, in minutes');
  assert.deepEqual(log.calls.map(call => call.timezoneOffset), [summer, winter].map(when => new Date(when).getTimezoneOffset()));
  assert.equal('timezoneOffset' in request, false, 'the caller\'s request is not changed');

  // No other op, and no sign-prepare without a numeric time, gets one.
  log.calls.length = 0;
  engine.dispatch({ ...request, id: 3, when: String(summer) }, session);
  engine.dispatch({ ...request, id: 4, when: undefined }, session);
  for (const op of ['sign-list', 'save', 'open', 'stamp']) engine.dispatch({ id: 5, op, when: summer }, session);
  assert.equal(log.calls.length, 6);
  for (const call of log.calls) assert.equal('timezoneOffset' in call, false, call.op);
  // A caller's own value is replaced by the worker's.
  log.calls.length = 0;
  engine.dispatch({ ...request, id: 6, timezoneOffset: -9999 }, session);
  assert.equal(log.calls[0].timezoneOffset, 300);
});

/** sign-prepare.ts `localStamp`. */
function localStamp(ms) {
  const d = new Date(ms), p = n => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

test('sign-prepare: the appearance shows the signer\'s local date and time, the /M date UTC', async () => {
  const engine = createEngine(mupdf);
  const session = engine.createSession();
  const bytes = await readFile(new URL('../fixtures/hello.pdf', import.meta.url));
  const opened = engine.dispatch({ id: 1, op: 'open', name: 'hello.pdf', pdf: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) }, session).reply;
  assert.equal(opened.ok, true, opened.error);
  // 03:30 UTC is the previous evening in Chicago.
  for (const [when, stamp, utc] of [
    [Date.UTC(2026, 9, 14, 12), '2026-10-14 07:00', 'D:20261014120000Z'],
    [Date.UTC(2026, 0, 15, 3, 30), '2026-01-14 21:30', 'D:20260115033000Z'],
  ]) {
    assert.equal(localStamp(when), stamp, 'the test runs in America/Chicago');
    const reply = engine.dispatch({ id: 2, op: 'sign-prepare', name: 'Alice', reason: 'Approve', page: 0, rect: [100, 100, 300, 160], certify: false, when }, session).reply;
    assert.equal(reply.ok, true, reply.error);
    const text = new TextDecoder('latin1').decode(new Uint8Array(reply.bytes));
    assert.ok(text.includes(`(Digitally signed by Alice) Tj 0 -11 Td (${stamp}) Tj 0 -11 Td (Reason: Approve) Tj`), stamp);
    assert.ok(text.includes(`/M(${utc})`), utc);
  }
});
