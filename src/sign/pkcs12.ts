import { dropKey, loadIdentity, reasonOf, signModule, type Key } from './native.js';

/** PKCS#12 and PEM reading and the before-signing check, in the tandem-sign module (crates/tandem-sign). */

export type SignReason = 'bad-password' | 'unsupported-key' | 'unreadable' | 'certificate-expired' | 'too-costly';

export class SignError extends Error {
  readonly reason: SignReason;
  readonly date?: string;
  readonly status?: string;
  constructor(reason: SignReason, extra?: { date?: string; status?: string }) {
    super(extra?.status ?? reason);
    this.reason = reason;
    this.date = extra?.date;
    this.status = extra?.status;
  }
}

export type { Key as SigningKey } from './native.js';
export { dropKey };

const REASONS = new Set<string>(['bad-password', 'unsupported-key', 'unreadable', 'certificate-expired', 'too-costly']);

function load(kind: 'p12' | 'pem', file: Buffer, password: string): { key: Key; certs: Buffer[] } {
  try {
    return loadIdentity(kind, file, password);
  } catch (err) {
    const reason = reasonOf(err);
    throw new SignError((REASONS.has(reason) ? reason : 'unreadable') as SignReason);
  }
}

/** A PKCS#12 file: the key (RSA of 2048 bits or more, P-256 or P-384) and its certificates, leaf first. */
export function parseP12(file: Buffer, password: string): { key: Key; certs: Buffer[] } {
  return load('p12', file, password);
}

/** PEM text with a private key (plain or encrypted) and its certificates. */
export function parsePem(file: Buffer, password: string): { key: Key; certs: Buffer[] } {
  return load('pem', file, password);
}

/** Whether the leaf can sign at `when`: notBefore <= when < notAfter. */
export function certUsable(certs: Buffer[], key: Key, when: number):
  { ok: true; cn: string; notAfter: string } | { ok: false; reason: 'certificate-expired' | 'unsupported-key' | 'unreadable'; date?: string; status?: string } {
  if (!certs[0]) return { ok: false, reason: 'unreadable' };
  return JSON.parse(signModule().checkUsable(certs[0], key, when));
}
