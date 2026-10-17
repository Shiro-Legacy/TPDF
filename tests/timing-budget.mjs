// Wall-clock bounds that hold on a busy machine. A test that guards against
// an operation turning quadratic or expanding work states a nominal bound
// (what the operation is allowed on an idle machine); the bound is scaled by
// how much slower the machine is when the test runs, measured right then on
// a fixed amount of native work (SHA-256 of 4 MiB, median of five), and
// given headroom. A regression of the kind these tests guard costs ten to a
// hundred times the nominal bound, so it still fails; a gate run sharing
// the machine with other builds no longer does. Read-only operations are
// timed best of several runs (`bestOf`).
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';

/** The calibration unit on an idle development machine (ms). */
export const NOMINAL_UNIT_MS = 2;
const block = Buffer.alloc(4 << 20, 7);

/** The machine's speed now: the median of five timings of the unit (ms). */
export function unitMs() {
  const times = [];
  for (let i = 0; i < 5; i++) {
    const started = performance.now();
    createHash('sha256').update(block).digest();
    times.push(performance.now() - started);
  }
  return times.sort((a, b) => a - b)[2];
}

/** The best of `runs` timings of `fn` (ms), and its last result. */
export function bestOf(runs, fn) {
  let ms = Infinity;
  let result;
  for (let i = 0; i < runs; i++) {
    const started = performance.now();
    result = fn();
    ms = Math.min(ms, performance.now() - started);
  }
  return { ms, result };
}

/** Asserts `ms` is within `nominalMs`, scaled by how much slower than
 * nominal the machine is now (never below nominal), times `headroom`. */
export function assertWithin(ms, nominalMs, message, headroom = 3) {
  const factor = Math.max(1, unitMs() / NOMINAL_UNIT_MS);
  const bound = nominalMs * factor * headroom;
  assert.ok(
    ms < bound,
    `${message}: ${ms.toFixed(1)} ms, bound ${bound.toFixed(0)} ms (nominal ${nominalMs} ms, machine ${factor.toFixed(1)}x slower than nominal)`,
  );
}
