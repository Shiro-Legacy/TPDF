import { Worker } from 'node:worker_threads';
import type { SignOffsets } from './cms.js';

/**
 * Main's side of the signing worker (src/sign/sign-worker.ts): certificate files are read and documents
 * signed off the main thread, and the private key stays in the worker; main holds a handle number. If the
 * worker dies, its keys die with it and every pending call answers `unreadable`.
 */

export type OpenResult =
  | { ok: true; handle: number; cn: string; notAfter: string; certs: number }
  | { ok: false; reason: string; date?: string; status?: string };
export type SignResult = { ok: true; bytes: Uint8Array } | { ok: false; reason: string; date?: string; status?: string };

export function createSigner(workerUrl = new URL('./sign-worker.js', import.meta.url)) {
  let worker: Worker | null = null;
  let nextId = 1;
  const pending = new Map<number, (reply: Record<string, unknown>) => void>();
  function fail(): void {
    worker = null;
    for (const done of pending.values()) done({ ok: false, reason: 'unreadable' });
    pending.clear();
  }
  function start(): Worker {
    if (worker) return worker;
    const started = new Worker(workerUrl);
    started.unref();
    started.on('message', (reply: Record<string, unknown> & { id: number }) => {
      const done = pending.get(reply.id);
      pending.delete(reply.id);
      done?.(reply);
    });
    started.on('error', fail);
    started.on('exit', () => { if (worker === started) fail(); });
    worker = started;
    return started;
  }
  function call(message: Record<string, unknown>, transfer: ArrayBuffer[] = []): Promise<Record<string, unknown>> {
    return new Promise(resolve => {
      const id = nextId++;
      pending.set(id, resolve);
      try {
        start().postMessage({ id, ...message }, transfer);
      } catch {
        pending.delete(id);
        resolve({ ok: false, reason: 'unreadable' });
      }
    });
  }
  return {
    /** Reads a certificate file (its bytes are moved to the worker) and checks it can sign at `when`. */
    async open(kind: 'p12' | 'pem', file: Uint8Array, password: string, when: number): Promise<OpenResult> {
      // A copy of exactly these bytes (a Buffer may be a view into a larger, shared ArrayBuffer).
      const bytes = new Uint8Array(file.byteLength);
      bytes.set(file);
      const copy = bytes.buffer;
      file.fill(0);
      return await call({ op: 'open', kind, file: copy, password, when }, [copy]) as OpenResult;
    },
    /** Signs a prepared PDF with a held key (one shot: the key is dropped afterwards). `source` is moved. */
    async sign(handle: number, source: ArrayBuffer, offsets: SignOffsets, when: number): Promise<SignResult> {
      const reply = await call({ op: 'sign', handle, source, offsets, when }, [source]);
      if (reply.ok === true && reply.bytes instanceof ArrayBuffer) return { ok: true, bytes: new Uint8Array(reply.bytes) };
      return reply as SignResult;
    },
    /** Drops a held key; unknown handles are ignored. */
    drop(handle: number): void {
      if (worker) void call({ op: 'drop', handle });
    },
    /** Stops the worker (its keys go with it). */
    async close(): Promise<void> {
      const running = worker;
      worker = null;
      if (running) await running.terminate();
      fail();
    },
  };
}
