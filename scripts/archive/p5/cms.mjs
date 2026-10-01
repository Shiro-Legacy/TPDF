#!/usr/bin/env node
/**
 * Detached CMS SignedData (SHA-256, RSA PKCS#1 v1.5) with node:crypto.
 * Signed attrs: contentType, messageDigest, signingTime. No dependency.
 * QA only — not the app. Self-check: `node scripts/p5/cms.mjs`
 */
import { spawnSync } from 'node:child_process';
import { createHash, createSign } from 'node:crypto';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

const ID = {
  data: '1.2.840.113549.1.7.1',
  signedData: '1.2.840.113549.1.7.2',
  contentType: '1.2.840.113549.1.9.3',
  messageDigest: '1.2.840.113549.1.9.4',
  signingTime: '1.2.840.113549.1.9.5',
  sha256: '2.16.840.1.101.3.4.2.1',
  rsa: '1.2.840.113549.1.1.1',
};

const cat = (parts) => Buffer.concat(parts);

function derLen(n) {
  if (n < 0x80) return Buffer.from([n]);
  const bytes = [];
  let x = n;
  while (x) { bytes.unshift(x & 0xff); x >>= 8; }
  return Buffer.from([0x80 | bytes.length, ...bytes]);
}

function tlv(tag, content) {
  const body = Buffer.isBuffer(content) ? content : Buffer.from(content);
  return cat([Buffer.from([tag]), derLen(body.length), body]);
}

function seq(parts) { return tlv(0x30, cat(parts)); }

function derSet(parts) {
  return tlv(0x31, cat(parts.slice().sort(Buffer.compare)));
}

function oid(dotted) {
  const arcs = dotted.split('.').map(Number);
  const body = [arcs[0] * 40 + arcs[1]];
  for (const arc of arcs.slice(2)) {
    const chunk = [arc & 0x7f];
    let rest = arc >> 7;
    while (rest) { chunk.push((rest & 0x7f) | 0x80); rest >>= 7; }
    body.push(...chunk.reverse());
  }
  return tlv(0x06, Buffer.from(body));
}

function alg(dotted, withNull) {
  return seq(withNull ? [oid(dotted), Buffer.from([0x05, 0x00])] : [oid(dotted)]);
}

function utcTime(date) {
  const p = (n) => String(n).padStart(2, '0');
  const text = `${p(date.getUTCFullYear() % 100)}${p(date.getUTCMonth() + 1)}${p(date.getUTCDate())}`
    + `${p(date.getUTCHours())}${p(date.getUTCMinutes())}${p(date.getUTCSeconds())}Z`;
  return tlv(0x17, text);
}

function parseTlv(buf, offset = 0) {
  const tag = buf[offset];
  let length = buf[offset + 1];
  let cursor = offset + 2;
  if (length & 0x80) {
    const count = length & 0x7f;
    length = 0;
    for (let i = 0; i < count; i++) length = (length << 8) | buf[cursor++];
  }
  return { tag, start: offset, valueStart: cursor, valueEnd: cursor + length, end: cursor + length };
}

function kids(buf, node) {
  const out = [];
  for (let cursor = node.valueStart; cursor < node.valueEnd;) {
    const child = parseTlv(buf, cursor);
    out.push(child);
    cursor = child.end;
  }
  return out;
}

/** Issuer Name and serial INTEGER, copied from the leaf so the DN bytes match the cert. */
export function issuerAndSerial(certDer) {
  const tbs = kids(certDer, parseTlv(certDer))[0];
  const parts = kids(certDer, tbs);
  const skipVersion = parts[0].tag === 0xa0 ? 1 : 0;
  const serial = certDer.subarray(parts[skipVersion].start, parts[skipVersion].end);
  const issuer = certDer.subarray(parts[skipVersion + 2].start, parts[skipVersion + 2].end);
  return { serial, issuer };
}

export function pemDer(pem) {
  return Buffer.from(String(pem).replace(/-----[^-]+-----/g, '').replace(/\s/g, ''), 'base64');
}

/**
 * Detached SignedData. `content` is the byte-range concatenation (not embedded).
 * `keyPem` is a PEM private key. Certs are embedded; CA is optional.
 */
export function cmsDetached({ content, leafDer, caDer, keyPem, signingTime = new Date() }) {
  const { serial, issuer } = issuerAndSerial(leafDer);
  const digest = createHash('sha256').update(content).digest();
  const signed = derSet([
    seq([oid(ID.contentType), derSet([oid(ID.data)])]),
    seq([oid(ID.messageDigest), derSet([tlv(0x04, digest)])]),
    seq([oid(ID.signingTime), derSet([utcTime(signingTime)])]),
  ]);
  const signature = createSign('SHA256').update(signed).sign(keyPem);
  const signerInfo = seq([
    tlv(0x02, Buffer.from([1])),
    seq([issuer, serial]),
    alg(ID.sha256, false),
    cat([Buffer.from([0xa0]), signed.subarray(1)]),
    alg(ID.rsa, true),
    tlv(0x04, signature),
  ]);
  const certs = [leafDer, ...(caDer ? [caDer] : [])].sort(Buffer.compare);
  const signedData = seq([
    tlv(0x02, Buffer.from([1])),
    derSet([alg(ID.sha256, false)]),
    seq([oid(ID.data)]),
    tlv(0xa0, cat(certs)),
    derSet([signerInfo]),
  ]);
  return seq([oid(ID.signedData), tlv(0xa0, signedData)]);
}

function selfCheck() {
  const root = path.resolve(path.dirname(new URL(import.meta.url).pathname), '../..');
  const ca = path.join(root, 'fixtures/p0b/.ca');
  const content = Buffer.from('pdf4all-cms-self-check');
  const cms = cmsDetached({
    content,
    leafDer: pemDer(readFileSync(path.join(ca, 'leaf.pem'))),
    caDer: pemDer(readFileSync(path.join(ca, 'ca.pem'))),
    keyPem: readFileSync(path.join(ca, 'leaf.key')),
    signingTime: new Date('2026-09-23T16:00:00Z'),
  });
  const dir = mkdtempSync(path.join(tmpdir(), 'p5-cms-'));
  try {
    writeFileSync(path.join(dir, 'content.bin'), content);
    writeFileSync(path.join(dir, 'sig.der'), cms);
    const run = spawnSync('openssl', [
      'cms', '-verify', '-inform', 'DER', '-in', path.join(dir, 'sig.der'),
      '-content', path.join(dir, 'content.bin'), '-CAfile', path.join(ca, 'ca.pem'),
      '-binary', '-out', '/dev/null',
    ], { encoding: 'utf8' });
    if (run.status !== 0) {
      console.error(run.stderr || run.stdout);
      throw new Error(`openssl cms -verify failed (${run.status})`);
    }
    console.log(`cms self-check ok (${cms.length} bytes)`);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

if (import.meta.url === `file://${process.argv[1]}`) selfCheck();
