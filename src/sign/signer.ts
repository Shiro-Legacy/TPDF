import { Worker } from 'node:worker_threads';
import type { SignOffsets } from './cms.js';

/**
 * Main's side of the signing worker (src/sign/sign-worker.ts): certificate files are read and documents
 * signed off the main thread, and the private key stays in the worker; main holds a handle. A handle names
 * the worker it came from (`<generation>:<number>`), so after the worker dies and a new one starts, an old
 * handle can never select a key the new worker holds. When a worker dies its keys die with it, every
 * pending call answers `unreadable`, and `onFailure` runs (main clears its hold).
 */

export type OpenResult =
  | { ok: true; handle: string; cn: string; notAfter: string; certs: number; forSigning: boolean }
  | { ok: false; reason: string; date?: string; status?: string };
export type SignResult = { ok: true; bytes: Uint8Array } | { ok: false; reason: string; date?: string; status?: string };

export function createSigner(workerUrl = new URL('./sign-worker.js', import.meta.url), onFailure: () => void = () => {}) {
  let worker: Worker | null = null;
  let generation = 0;
  let nextId = 1;
  const pending = new Map<number, (reply: Record<string, unknown>) => void>();
  function fail(dead: Worker): void {
    if (worker !== dead) return;
    worker = null;
    for (const done of pending.values()) done({ ok: false, reason: 'unreadable' });
    pending.clear();
    onFailure();
  }
  function start(): Worker {
    if (worker) return worker;
    const started = new Worker(workerUrl);
    generation++;
    started.unref();
    started.on('message', (reply: Record<string, unknown> & { id: number }) => {
      const done = pending.get(reply.id);
      pending.delete(reply.id);
      done?.(reply);
    });
    started.on('error', () => fail(started));
    started.on('exit', () => fail(started));
    worker = started;
    return started;
  }
  /** The worker's own handle number, when `handle` belongs to the running worker. */
  function local(handle: string): number | null {
    const match = /^(\d+):(\d+)$/.exec(handle);
    if (!match || !worker || Number(match[1]) !== generation) return null;
    return Number(match[2]);
  }
  /** Posts to the running worker; the reply comes with that worker's generation (a handle's prefix). */
  function call(message: Record<string, unknown>, transfer: ArrayBuffer[] = []): Promise<Record<string, unknown> & { generation: number }> {
    return new Promise(resolve => {
      const id = nextId++;
      let posted = 0;
      pending.set(id, reply => resolve({ ...reply, generation: posted }));
      try {
        const running = start();
        posted = generation;
        try {
          running.postMessage({ id, ...message }, transfer);
        } catch {
          // A buffer that cannot be moved (IPC hands main external memory) is copied instead;
          // the original is then zeroed, as a moved one would be gone.
          running.postMessage({ id, ...message });
          for (const buffer of transfer) new Uint8Array(buffer).fill(0);
        }
      } catch {
        pending.delete(id);
        resolve({ ok: false, reason: 'unreadable', generation: posted });
      }
    });
  }
  return {
    /** Reads a certificate file (its bytes are moved to the worker and zeroed here) and checks it can sign at `when`. */
    async open(kind: 'p12' | 'pem', file: Uint8Array, password: string, when: number): Promise<OpenResult> {
      // A copy of exactly these bytes (a Buffer may be a view into a larger, shared ArrayBuffer).
      const bytes = new Uint8Array(file.byteLength);
      bytes.set(file);
      file.fill(0);
      const { generation: from, ...reply } = await call({ op: 'open', kind, file: bytes.buffer, password, when }, [bytes.buffer]);
      if (reply.ok !== true) return reply as OpenResult;
      // The generation of the worker that answered, not of whichever runs now.
      return { ...reply, ok: true, handle: `${from}:${Number(reply.handle)}` } as OpenResult;
    },
    /** Signs a prepared PDF with a held key (one shot: the key is dropped afterwards). `source` is moved. */
    async sign(handle: string, source: ArrayBuffer, offsets: SignOffsets, when: number): Promise<SignResult> {
      const own = local(handle);
      if (own === null) return { ok: false, reason: 'no-certificate' };
      const { generation: _from, ...reply } = await call({ op: 'sign', handle: own, source, offsets, when }, [source]);
      if (reply.ok === true && reply.bytes instanceof ArrayBuffer) return { ok: true, bytes: new Uint8Array(reply.bytes) };
      return reply as SignResult;
    },
    /** Drops a held key; a handle of an earlier worker, or an unknown one, is ignored. */
    drop(handle: string): void {
      const own = local(handle);
      if (own !== null) void call({ op: 'drop', handle: own });
    },
    /** Stops the worker (its keys go with it). */
    async close(): Promise<void> {
      const running = worker;
      if (running) {
        fail(running);
        await running.terminate();
      }
    },
  };
}
