import { MAX_SIG_CONTENTS_BYTES } from '../renderer/limits.js';
import { entropy, packDers, readerOf, signModule, type Key } from './native.js';

/**
 * Detached CMS signing and patching it into a prepared PDF, in the tandem-sign module (crates/tandem-sign).
 * The CMS carries content-type, message-digest, signing-time and ESS signing-certificate-v2 attributes.
 */

export { MAX_SIG_CONTENTS_BYTES };

/** mupdf writes this with no space after `/ByteRange`. Main checks these 36 bytes, never `"/ByteRange ["`. */
export const BYTE_RANGE_SENTINEL = '[0 1000000000 1000000000 1000000000]';
export const SIGNING_SKEW_MS = 10 * 60 * 1000;

export type SignOffsets = { contentsAt: number; contentsLen: number; byteRangeAt: number };
export type PatchResult =
  | { ok: true; bytes: Uint8Array }
  | { ok: false; reason: 'unreadable' | 'cms-too-large' };

export function signingWhenOk(when: number, now = Date.now()): boolean {
  return Number.isFinite(when) && Math.abs(when - now) <= SIGNING_SKEW_MS;
}

/**
 * Detached SignedData over `content`. `certs[0]` is the leaf; RSA signs PKCS#1 v1.5 SHA-256, P-256/P-384
 * ECDSA. PAdES B-B: no signing-time attribute (the PDF's /M carries the time).
 */
export function cmsDetached(input: { content: Buffer; certs: Buffer[]; key: Key }): Buffer {
  return Buffer.from(signModule().signDetached(input.key, packDers(input.certs), input.content, entropy()));
}

type Plan = { ok: true; gapStart: number; second: number; byteRange: string } | { ok: false; reason: 'unreadable' };

function plan(source: Uint8Array, offsets: SignOffsets): Plan {
  const { contentsAt, contentsLen, byteRangeAt } = offsets;
  return JSON.parse(signModule().planSignature(readerOf(source), source.byteLength, contentsAt, contentsLen, byteRangeAt));
}

/**
 * Check the 36-byte sentinel and the zero Contents hole, write ByteRange in place, then ask `cmsFor` for the
 * CMS over the two ranges and patch the hex. (Tests use this with their own `cmsFor`; main uses `signPdf`.)
 */
export function patchSignature(
  source: Uint8Array,
  offsets: SignOffsets,
  cmsFor: (content: Buffer) => Buffer,
): PatchResult {
  const checked = plan(source, offsets);
  if (!checked.ok) return { ok: false, reason: 'unreadable' };
  const buf = Buffer.from(source);
  buf.write(checked.byteRange, offsets.byteRangeAt, 36, 'ascii');
  const content = Buffer.concat([buf.subarray(0, checked.gapStart), buf.subarray(checked.second)]);
  const hex = cmsFor(content).toString('hex').toUpperCase();
  if (hex.length > offsets.contentsLen) return { ok: false, reason: 'cms-too-large' };
  buf.write(hex, offsets.contentsAt, hex.length, 'ascii');
  return { ok: true, bytes: buf };
}

/**
 * Sign a prepared PDF with a held key: the module checks the hole, hashes the two ranges by reading them from
 * `source` (so the file is not copied into it) and returns the two writes, applied here to a copy.
 */
export function signPdf(source: Uint8Array, offsets: SignOffsets, key: Key, certs: Buffer[]): PatchResult {
  const { contentsAt, contentsLen, byteRangeAt } = offsets;
  const result = JSON.parse(signModule().signPrepared(
    readerOf(source), source.byteLength, contentsAt, contentsLen, byteRangeAt, key, packDers(certs), entropy(),
  )) as { ok: true; byteRangeAt: number; byteRange: string; contentsAt: number; hex: string } | { ok: false; reason: string };
  if (!result.ok) return { ok: false, reason: result.reason === 'cms-too-large' ? 'cms-too-large' : 'unreadable' };
  const buf = Buffer.from(source);
  buf.write(result.byteRange, result.byteRangeAt, 36, 'ascii');
  buf.write(result.hex, result.contentsAt, result.hex.length, 'ascii');
  return { ok: true, bytes: buf };
}

/** The DER inside PEM text (the first block's base64). */
export function pemDer(pem: string | Buffer): Buffer {
  return Buffer.from(String(pem).replace(/-----[^-]+-----/g, '').replace(/\s/g, ''), 'base64');
}
