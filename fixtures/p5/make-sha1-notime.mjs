// signed-sha1-notime.pdf: signed-sha1.pdf with /M blanked and its hole re-signed by the p0b leaf,
// SHA-1 digest, no signingTime attribute. Expected verdict: valid, trusted (ca.pem), reason "weak-digest no-signing-time".
// Run from the repo root after `npm run build`: node fixtures/p5/make-sha1-notime.mjs
import { readFileSync, writeFileSync } from 'node:fs';
import { createHash, createSign, createPrivateKey } from 'node:crypto';
import { tlv, seq, derSet, oid, issuerAndSerial, pemDer } from '../../build/sign/der.js';

const src = Buffer.from(readFileSync('fixtures/p5/signed-sha1.pdf'));
const text = src.toString('latin1');
const m = /\/M\s*\([^)]*\)/.exec(text);
src.fill(0x20, m.index, m.index + m[0].length);
const [, , b, c, d] = /\/ByteRange\s*\[\s*(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s*\]/.exec(text).map(Number);
const content = Buffer.concat([src.subarray(0, b), src.subarray(c, c + d)]);
const alg = o => seq([oid(o), tlv(0x05, Buffer.alloc(0))]);
const SHA1 = '1.3.14.3.2.26';
const leaf = pemDer(readFileSync('fixtures/p0b/.ca/leaf.pem'));
const { serial, issuer } = issuerAndSerial(leaf);
const attrs = derSet([
  seq([oid('1.2.840.113549.1.9.3'), derSet([oid('1.2.840.113549.1.7.1')])]),
  seq([oid('1.2.840.113549.1.9.4'), derSet([tlv(0x04, createHash('sha1').update(content).digest())])]),
]);
const sig = createSign('SHA1').update(attrs).sign(createPrivateKey(readFileSync('fixtures/p0b/.ca/leaf.key')));
const info = seq([tlv(0x02, Buffer.from([1])), seq([issuer, serial]), alg(SHA1),
  Buffer.concat([Buffer.from([0xa0]), attrs.subarray(1)]), alg('1.2.840.113549.1.1.1'), tlv(0x04, sig)]);
const signed = seq([tlv(0x02, Buffer.from([1])), derSet([alg(SHA1)]), seq([oid('1.2.840.113549.1.7.1')]),
  Buffer.concat([Buffer.from([0xa0]), tlv(0x30, leaf).subarray(1)]), derSet([info])]);
const hex = seq([oid('1.2.840.113549.1.7.2'), tlv(0xa0, signed)]).toString('hex');
if (hex.length > c - b - 2) throw new Error('CMS does not fit the hole');
src.write(hex.padEnd(c - b - 2, '0'), b + 1, 'latin1');
writeFileSync('fixtures/p5/signed-sha1-notime.pdf', src);
