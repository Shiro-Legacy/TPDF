import { parentPort } from 'node:worker_threads';
import { signPdf, type SignOffsets } from './cms.js';
import { dropKey, type Key } from './native.js';
import { certUsable, parseP12, parsePem, SignError } from './pkcs12.js';

/**
 * The signing worker (src/sign/signer.ts starts it): reading a certificate file (its password hashing can
 * take seconds) and signing run here, so Electron's main process stays responsive. Private keys never
 * leave this thread: main holds only a handle number. It runs in main's process, as a worker thread.
 */

type Request =
  | { id: number; op: 'open'; kind: 'p12' | 'pem'; file: ArrayBuffer; password: string; when: number }
  | { id: number; op: 'sign'; handle: number; source: ArrayBuffer; offsets: SignOffsets; when: number }
  | { id: number; op: 'drop'; handle: number };

const held = new Map<number, { key: Key; certs: Buffer[] }>();
let next = 1;

function drop(handle: number): void {
  dropKey(held.get(handle)?.key);
  held.delete(handle);
}

function reasonOf(err: unknown): string {
  return err instanceof SignError ? err.reason : 'unreadable';
}

parentPort?.on('message', (request: Request) => {
  const reply = (body: Record<string, unknown>, transfer: ArrayBuffer[] = []) => {
    try {
      parentPort?.postMessage({ id: request.id, ...body }, transfer);
    } catch {
      // Memory that cannot be moved between threads is copied instead.
      parentPort?.postMessage({ id: request.id, ...body });
    }
  };
  try {
    if (request.op === 'open') {
      const file = Buffer.from(request.file);
      const parsed = request.kind === 'pem' ? parsePem(file, request.password) : parseP12(file, request.password);
      let kept = false;
      try {
        const facts = certUsable(parsed.certs, parsed.key, request.when);
        if (!facts.ok) {
          reply({ ok: false, reason: facts.reason, ...(facts.date ? { date: facts.date, status: facts.status } : {}) });
          return;
        }
        const handle = next++;
        held.set(handle, parsed);
        kept = true;
        reply({ ok: true, handle, cn: facts.cn, notAfter: facts.notAfter, certs: parsed.certs.length });
      } finally {
        if (!kept) dropKey(parsed.key);
      }
    } else if (request.op === 'sign') {
      const identity = held.get(request.handle);
      held.delete(request.handle);
      if (!identity) { reply({ ok: false, reason: 'no-certificate' }); return; }
      try {
        const facts = certUsable(identity.certs, identity.key, request.when);
        if (!facts.ok) {
          reply({ ok: false, reason: facts.reason, ...(facts.date ? { date: facts.date, status: facts.status } : {}) });
          return;
        }
        const patched = signPdf(new Uint8Array(request.source), request.offsets, identity.key, identity.certs);
        if (!patched.ok) { reply(patched); return; }
        const bytes = patched.bytes;
        const out = bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength
          ? bytes.buffer as ArrayBuffer
          : bytes.slice().buffer as ArrayBuffer;
        reply({ ok: true, bytes: out }, [out]);
      } finally {
        dropKey(identity.key);
      }
    } else if (request.op === 'drop') {
      drop(request.handle);
      reply({ ok: true });
    }
  } catch (err) {
    reply({ ok: false, reason: reasonOf(err) });
  }
});
