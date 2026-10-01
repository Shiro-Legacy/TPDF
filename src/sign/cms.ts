import { createHash, createSign, createVerify, verify, X509Certificate, type KeyObject } from 'node:crypto';
import { MAX_SIG_CONTENTS_BYTES } from '../renderer/limits.js';
import { cat, derSet, issuerAndSerial, kids, oid, oidOf, parseTlv, pemDer, seq, tlv, utcTime, type Tlv } from './der.js';

export { MAX_SIG_CONTENTS_BYTES };

const ID = {
  data: '1.2.840.113549.1.7.1',
  signedData: '1.2.840.113549.1.7.2',
  contentType: '1.2.840.113549.1.9.3',
  messageDigest: '1.2.840.113549.1.9.4',
  signingTime: '1.2.840.113549.1.9.5',
  sha256: '2.16.840.1.101.3.4.2.1',
  rsa: '1.2.840.113549.1.1.1',
  sha256Rsa: '1.2.840.113549.1.1.11',
};

/** mupdf writes this with no space after `/ByteRange`. Main checks these 36 bytes, never `"/ByteRange ["`. */
export const BYTE_RANGE_SENTINEL = '[0 1000000000 1000000000 1000000000]';
export const SIGNING_SKEW_MS = 10 * 60 * 1000;

export type SignOffsets = { contentsAt: number; contentsLen: number; byteRangeAt: number };
export type PatchResult =
  | { ok: true; bytes: Uint8Array }
  | { ok: false; reason: 'unreadable' | 'cms-too-large' };

function alg(dotted: string, withNull: boolean): Buffer {
  return seq(withNull ? [oid(dotted), Buffer.from([0x05, 0x00])] : [oid(dotted)]);
}

export function signingWhenOk(when: number, now = Date.now()): boolean {
  return Number.isFinite(when) && Math.abs(when - now) <= SIGNING_SKEW_MS;
}

/**
 * Detached SignedData. `certs[0]` is the leaf. Signature is RSA PKCS#1 v1.5
 * over the signed-attrs SET (tag 0x31), not the implicit [0] form.
 */
export function cmsDetached(input: {
  content: Buffer;
  certs: Buffer[];
  key: KeyObject;
  signingTime?: Date;
}): Buffer {
  const leaf = input.certs[0];
  const { serial, issuer } = issuerAndSerial(leaf);
  const digest = createHash('sha256').update(input.content).digest();
  const signed = derSet([
    seq([oid(ID.contentType), derSet([oid(ID.data)])]),
    seq([oid(ID.messageDigest), derSet([tlv(0x04, digest)])]),
    seq([oid(ID.signingTime), derSet([utcTime(input.signingTime ?? new Date())])]),
  ]);
  const signature = createSign('SHA256').update(signed).sign(input.key);
  const signerInfo = seq([
    tlv(0x02, Buffer.from([1])),
    seq([issuer, serial]),
    alg(ID.sha256, false),
    cat([Buffer.from([0xa0]), signed.subarray(1)]),
    alg(ID.rsa, true),
    tlv(0x04, signature),
  ]);
  const signedData = seq([
    tlv(0x02, Buffer.from([1])),
    derSet([alg(ID.sha256, false)]),
    seq([oid(ID.data)]),
    tlv(0xa0, cat(input.certs)),
    derSet([signerInfo]),
  ]);
  return seq([oid(ID.signedData), tlv(0xa0, signedData)]);
}

export type VerifyResult = {
  digestMatches: boolean;
  signatureValid: boolean;
  signerCn: string;
  /** Present only when `caPem` was passed. Leaf signature under the CA key — no dates, EKU, or revocation. */
  chainToCa?: boolean;
};

function attrValue(buf: Buffer, attr: Tlv): Buffer | undefined {
  const body = kids(buf, attr)[1];
  if (!body) return;
  return buf.subarray(kids(buf, body)[0].valueStart, kids(buf, body)[0].valueEnd);
}

function cnOf(der: Buffer): string {
  const subject = new X509Certificate(der).subject;
  return /(?:^|\n)CN=([^\n]*)/.exec(subject)?.[1] ?? '';
}

/** Leaf cert signature under `caKey`. SHA-256 only; dates are not checked. */
function leafSignedBy(certDer: Buffer, caKey: KeyObject): boolean {
  const top = parseTlv(certDer);
  const [tbs, alg, bit] = kids(certDer, top);
  if (oidOf(certDer, kids(certDer, alg)[0]) !== ID.sha256Rsa) return false;
  const unused = certDer[bit.valueStart];
  if (unused !== 0) return false;
  return verify('sha256', certDer.subarray(tbs.start, tbs.end), caKey, certDer.subarray(bit.valueStart + 1, bit.valueEnd));
}

/**
 * Test oracle and U2 seed. Not reachable from IPC. Checks the message-digest
 * attribute, the RSA signature over signed attrs re-tagged as SET, and
 * optionally the leaf's signature under the CA key.
 */
export function verifyDetached(cms: Buffer, content: Buffer, caPem?: Buffer): VerifyResult {
  const out: VerifyResult = { digestMatches: false, signatureValid: false, signerCn: '' };
  if (caPem) out.chainToCa = false;
  try {
    const top = kids(cms, parseTlv(cms));
    if (oidOf(cms, top[0]) !== ID.signedData) return out;
    const signedData = kids(cms, top[1])[0];
    const parts = kids(cms, signedData);
    const certSet = parts.find(part => part.tag === 0xa0);
    const signerSet = parts.filter(part => part.tag === 0x31).pop();
    if (!certSet || !signerSet) return out;
    const certs = kids(cms, certSet).map(node => cms.subarray(node.start, node.end));
    const info = kids(cms, kids(cms, signerSet)[0]);
    const sid = kids(cms, info[1]);
    const wantIssuer = cms.subarray(sid[0].start, sid[0].end);
    const wantSerial = cms.subarray(sid[1].start, sid[1].end);
    const leaf = certs.find(der => {
      const ids = issuerAndSerial(der);
      return ids.issuer.equals(wantIssuer) && ids.serial.equals(wantSerial);
    }) ?? certs[0];
    if (!leaf) return out;
    out.signerCn = cnOf(leaf);
    const signedAttrs = info.find(part => part.tag === 0xa0);
    const signature = info.filter(part => part.tag === 0x04).pop();
    if (!signedAttrs || !signature) return out;
    for (const attr of kids(cms, signedAttrs)) {
      if (oidOf(cms, kids(cms, attr)[0]) !== ID.messageDigest) continue;
      const digest = attrValue(cms, attr);
      out.digestMatches = !!digest && digest.equals(createHash('sha256').update(content).digest());
    }
    const setTagged = Buffer.from(cms.subarray(signedAttrs.start, signedAttrs.end));
    setTagged[0] = 0x31;
    const check = createVerify('SHA256');
    check.update(setTagged);
    out.signatureValid = check.verify(new X509Certificate(leaf).publicKey, cms.subarray(signature.valueStart, signature.valueEnd));
    if (caPem) out.chainToCa = leafSignedBy(leaf, new X509Certificate(caPem).publicKey);
  } catch {
    return out;
  }
  return out;
}

function put10(buf: Buffer, at: number, n: number): boolean {
  if (!Number.isInteger(n) || n < 0 || n > 9_999_999_999) return false;
  buf.write(String(n).padStart(10, '0'), at, 10, 'ascii');
  return true;
}

/**
 * Check the 36-byte sentinel and the zero Contents hole, write ByteRange in
 * place, then ask `cmsFor` for the CMS over the two ranges and patch the hex.
 */
export function patchSignature(
  source: Uint8Array,
  offsets: SignOffsets,
  cmsFor: (content: Buffer) => Buffer,
): PatchResult {
  const { contentsAt, contentsLen, byteRangeAt } = offsets;
  if (contentsLen !== MAX_SIG_CONTENTS_BYTES * 2) return { ok: false, reason: 'unreadable' };
  if (byteRangeAt < 0 || contentsAt < 1) return { ok: false, reason: 'unreadable' };
  if (byteRangeAt + BYTE_RANGE_SENTINEL.length > source.length) return { ok: false, reason: 'unreadable' };
  if (contentsAt + contentsLen >= source.length) return { ok: false, reason: 'unreadable' };
  const buf = Buffer.from(source);
  if (buf.toString('ascii', byteRangeAt, byteRangeAt + BYTE_RANGE_SENTINEL.length) !== BYTE_RANGE_SENTINEL) {
    return { ok: false, reason: 'unreadable' };
  }
  const gapStart = contentsAt - 1;
  const gapEnd = contentsAt + contentsLen + 1;
  if (buf[gapStart] !== 0x3c || buf[contentsAt + contentsLen] !== 0x3e) return { ok: false, reason: 'unreadable' };
  for (let i = contentsAt; i < contentsAt + contentsLen; i++) {
    if (buf[i] !== 0x30) return { ok: false, reason: 'unreadable' };
  }
  const second = gapEnd;
  const tail = buf.length - second;
  if (!put10(buf, byteRangeAt + 3, gapStart) || !put10(buf, byteRangeAt + 14, second) || !put10(buf, byteRangeAt + 25, tail)) {
    return { ok: false, reason: 'unreadable' };
  }
  const content = cat([buf.subarray(0, gapStart), buf.subarray(second)]);
  const cms = cmsFor(content);
  const hex = cms.toString('hex').toUpperCase();
  if (hex.length > contentsLen) return { ok: false, reason: 'cms-too-large' };
  buf.write(hex, contentsAt, hex.length, 'ascii');
  return { ok: true, bytes: buf };
}

export { pemDer };
