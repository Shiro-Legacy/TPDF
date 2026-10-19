import { createRequire } from 'node:module';

/**
 * The tandem-sign WASM module (crates/tandem-sign, built by engine/tools/build-workflow.sh and copied to
 * build/sign/vendor by scripts/build.mjs). Electron's main process is its only user: private keys live inside
 * the module as `Key` handles and never reach the renderer. The module has no clock or entropy of its own, so
 * every call that needs them takes them from here.
 */

/** A private key inside the module. `dropKey` frees it, which zeroes it. */
export interface Key {
  readonly kind: string;
  readonly bits: number;
  readonly curve: string | undefined;
  free(): void;
}

interface Identity {
  certCount(): number;
  cert(index: number): Uint8Array | undefined;
  takeKey(): Key | undefined;
  free(): void;
}

/** The module reads the PDF through this, so a large file is never copied in whole. */
export interface Reader {
  read(offset: number, len: number): Uint8Array;
}

export interface SignModule {
  /** Zeroes `bytes` and `password` (UTF-8) in place before returning. */
  parseP12(bytes: Uint8Array, password: Uint8Array): Identity;
  parsePem(bytes: Uint8Array, password: Uint8Array): Identity;
  checkUsable(leaf: Uint8Array, key: Key, when: number): string;
  /** Zeroes `entropy`. */
  signDetached(key: Key, certs: Uint8Array, content: Uint8Array, entropy: Uint8Array): Uint8Array;
  planSignature(reader: Reader, len: number, contentsAt: number, contentsLen: number, byteRangeAt: number): string;
  signPrepared(reader: Reader, len: number, contentsAt: number, contentsLen: number, byteRangeAt: number,
    key: Key, certs: Uint8Array, entropy: Uint8Array): string;
  verifySignatures(reader: Reader, len: number, entries: string, trusted: Uint8Array, now: number): string;
  certInfo(der: Uint8Array): string | undefined;
  maxSigContentsBytes(): number;
}

let loaded: SignModule | undefined;

/** The module, loaded on first use (a missing build throws, which callers report as unreadable). */
export function signModule(): SignModule {
  loaded ??= createRequire(import.meta.url)('./vendor/tandem_sign.cjs') as SignModule;
  return loaded;
}

export function readerOf(bytes: Uint8Array): Reader {
  return { read: (offset, len) => bytes.subarray(offset, offset + len) };
}

/** `[u32 big-endian length][bytes]` per certificate, the module's list form. */
export function packDers(ders: Uint8Array[]): Uint8Array {
  const out = Buffer.alloc(ders.reduce((n, der) => n + 4 + der.byteLength, 0));
  let at = 0;
  for (const der of ders) {
    out.writeUInt32BE(der.byteLength, at);
    out.set(der, at + 4);
    at += 4 + der.byteLength;
  }
  return out;
}

/** 48 bytes from the platform CSPRNG (Web Crypto), the seed of one signature's RSA blinding. */
export function entropy(): Uint8Array {
  return globalThis.crypto.getRandomValues(new Uint8Array(48));
}

/** The reason token a module error carries (its message), else `unreadable`. */
export function reasonOf(err: unknown): string {
  const message = err instanceof Error ? err.message : '';
  return /^[a-z][a-z0-9-]{0,40}$/.test(message) ? message : 'unreadable';
}

const dropped = new WeakSet<object>();

/** Frees a key once; later calls do nothing. */
export function dropKey(key: Key | undefined | null): void {
  if (!key || typeof key !== 'object' || dropped.has(key)) return;
  dropped.add(key);
  try { key.free(); } catch { /* already gone */ }
}

/**
 * Reads a certificate file: the key handle and the certificates, leaf first. Throws the reason token. The
 * module zeroes `file` (the caller's buffer) and its copy of the password; the JavaScript string itself
 * cannot be wiped.
 */
export function loadIdentity(kind: 'p12' | 'pem', file: Uint8Array, password: string): { key: Key; certs: Buffer[] } {
  const mod = signModule();
  const secret = Buffer.from(password, 'utf8');
  let id: Identity;
  try {
    id = kind === 'p12' ? mod.parseP12(file, secret) : mod.parsePem(file, secret);
  } finally {
    secret.fill(0);
  }
  try {
    const certs: Buffer[] = [];
    for (let i = 0; i < id.certCount(); i++) certs.push(Buffer.from(id.cert(i) ?? new Uint8Array(0)));
    const key = id.takeKey();
    if (!key) throw new Error('unreadable');
    return { key, certs };
  } finally {
    id.free();
  }
}
