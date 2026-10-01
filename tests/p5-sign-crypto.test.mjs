/**
 * P5 U1 main-side crypto. Node only — no Electron. Legacy RC2 skips when the
 * cipher is absent (repo Node 22). Every other fixture is required.
 */
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createDecipheriv, createPrivateKey, X509Certificate } from 'node:crypto';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { derSet, oid, parseTlv, tlv } from '../build/sign/der.js';
import {
  BYTE_RANGE_SENTINEL, cmsDetached, MAX_SIG_CONTENTS_BYTES, patchSignature, signingWhenOk, verifyDetached,
} from '../build/sign/cms.js';
import { createCertHold } from '../build/sign/hold.js';
import { certUsable, parseP12, parsePem, SignError } from '../build/sign/pkcs12.js';

const root = path.resolve(import.meta.dirname, '..');
const ca = path.join(root, 'fixtures/p0b/.ca');
const p5 = path.join(root, 'fixtures/p5');
const password = 'pdf4all';

function read(name) {
  return readFileSync(path.join(p5, name));
}

function hole(contentsAt = 80) {
  const contentsLen = MAX_SIG_CONTENTS_BYTES * 2;
  const byteRangeAt = contentsAt + contentsLen + 4;
  const bytes = Buffer.alloc(byteRangeAt + BYTE_RANGE_SENTINEL.length + 8, 0x20);
  bytes[contentsAt - 1] = 0x3c;
  bytes.fill(0x30, contentsAt, contentsAt + contentsLen);
  bytes[contentsAt + contentsLen] = 0x3e;
  bytes.write(BYTE_RANGE_SENTINEL, byteRangeAt, 'ascii');
  return { bytes, offsets: { contentsAt, contentsLen, byteRangeAt } };
}

test('DER round-trip: an OID parses back to the same bytes', () => {
  const encoded = oid('1.2.840.113549.1.9.4');
  const node = parseTlv(encoded);
  assert.equal(node.tag, 0x06);
  assert.equal(node.end, encoded.length);
  assert.equal(tlv(node.tag, encoded.subarray(node.valueStart, node.valueEnd)).equals(encoded), true);
  const set = derSet([tlv(0x02, Buffer.from([2])), tlv(0x02, Buffer.from([1]))]);
  assert.equal(set[0], 0x31);
  assert.equal(set[4], 1);
  assert.equal(set[7], 2);
});

test('parseP12 PBES2 matches leaf.key and orders the leaf first', () => {
  const parsed = parseP12(read('leaf.p12'), password);
  const leaf = createPrivateKey(readFileSync(path.join(ca, 'leaf.key')));
  assert.equal(parsed.key.asymmetricKeyType, 'rsa');
  assert.equal(parsed.key.asymmetricKeyDetails.modulusLength, leaf.asymmetricKeyDetails.modulusLength);
  assert.equal(parsed.certs.length, 2);
  assert.equal(new X509Certificate(parsed.certs[0]).subject, 'CN=P0B Signer');
  assert.equal(certUsable(parsed.certs, parsed.key, Date.now()).cn, 'P0B Signer');
});

test('wrong PKCS#12 password is bad-password', () => {
  assert.throws(() => parseP12(read('leaf.p12'), 'nope'), (err) => err instanceof SignError && err.reason === 'bad-password');
});

// Electron's BoringSSL decrypts rc2-40-cbc but leaves it out of getCiphers(), so probe it directly.
const hasRc2 = (() => { try { createDecipheriv('rc2-40-cbc', Buffer.alloc(5), Buffer.alloc(8)); return true; } catch { return false; } })();

test('legacy p12 parses when rc2-40-cbc exists', { skip: !hasRc2 }, () => {
  const parsed = parseP12(read('leaf-legacy.p12'), password);
  assert.equal(new X509Certificate(parsed.certs[0]).subject, 'CN=P0B Signer');
});

test('P-256 p12 is unsupported-key', () => {
  assert.throws(() => parseP12(read('leaf-ec.p12'), password), (err) => err instanceof SignError && err.reason === 'unsupported-key');
});

test('RSA under 2048 bits is unsupported-key', () => {
  const dir = mkdtempSync(path.join(tmpdir(), 'p5-rsa-'));
  try {
    const key = path.join(dir, 'small.key');
    const cert = path.join(dir, 'small.pem');
    const csr = path.join(dir, 'small.csr');
    assert.equal(spawnSync('openssl', ['genrsa', '-out', key, '1024'], { encoding: 'utf8' }).status, 0);
    assert.equal(spawnSync('openssl', ['req', '-new', '-key', key, '-subj', '/CN=Small', '-out', csr], { encoding: 'utf8' }).status, 0);
    assert.equal(spawnSync('openssl', ['x509', '-req', '-in', csr, '-signkey', key, '-sha256', '-days', '1', '-out', cert], { encoding: 'utf8' }).status, 0);
    const pem = Buffer.concat([readFileSync(key), readFileSync(cert)]);
    assert.throws(() => parsePem(pem, ''), (err) => err instanceof SignError && err.reason === 'unsupported-key');
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('parsePem plain and encrypted', () => {
  const plain = parsePem(read('leaf.pem'), '');
  const enc = parsePem(read('leaf-enc.pem'), password);
  assert.equal(new X509Certificate(plain.certs[0]).subject, 'CN=P0B Signer');
  assert.equal(plain.certs.length, 2);
  assert.equal(enc.certs.length, 2);
  assert.throws(() => parsePem(read('leaf-enc.pem'), 'nope'), (err) => err instanceof SignError && err.reason === 'bad-password');
});

test('expired and not-yet-valid leaves are certificate-expired', () => {
  const parsed = parseP12(read('leaf.p12'), password);
  const cert = new X509Certificate(parsed.certs[0]);
  const after = Date.parse(cert.validTo) + 86_400_000;
  const expired = certUsable(parsed.certs, parsed.key, after);
  assert.equal(expired.ok, false);
  assert.equal(expired.reason, 'certificate-expired');
  assert.match(expired.status, /^this certificate expired on \d{4}-\d{2}-\d{2}$/);
  const early = certUsable(parsed.certs, parsed.key, 0);
  assert.equal(early.reason, 'certificate-expired');
  assert.match(early.status, /^this certificate is not valid until \d{4}-\d{2}-\d{2}$/);
});

test('cmsDetached verifies here and with openssl', () => {
  const parsed = parseP12(read('leaf.p12'), password);
  const content = Buffer.from('pdf4all-cms');
  const cms = cmsDetached({ content, certs: parsed.certs, key: parsed.key, signingTime: new Date('2026-09-23T16:00:00Z') });
  const own = verifyDetached(cms, content, readFileSync(path.join(ca, 'ca.pem')));
  assert.equal(own.digestMatches, true);
  assert.equal(own.signatureValid, true);
  assert.equal(own.signerCn, 'P0B Signer');
  assert.equal(own.chainToCa, true);
  assert.equal(verifyDetached(cms, Buffer.from('other')).digestMatches, false);
  const dir = mkdtempSync(path.join(tmpdir(), 'p5-cms-'));
  try {
    writeFileSync(path.join(dir, 'content.bin'), content);
    writeFileSync(path.join(dir, 'sig.der'), cms);
    const run = spawnSync('openssl', [
      'cms', '-verify', '-inform', 'DER', '-in', path.join(dir, 'sig.der'),
      '-content', path.join(dir, 'content.bin'), '-CAfile', path.join(ca, 'ca.pem'),
      '-binary', '-out', '/dev/null',
    ], { encoding: 'utf8' });
    assert.equal(run.status, 0, run.stderr);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('sign-file hole: a flipped Contents byte is unreadable, and an oversized CMS is cms-too-large', () => {
  const { bytes, offsets } = hole();
  bytes[offsets.contentsAt] = 0x31;
  const flipped = patchSignature(bytes, offsets, () => Buffer.from([0x30]));
  assert.equal(flipped.ok, false);
  assert.equal(flipped.reason, 'unreadable');
  const fresh = hole();
  const overflow = patchSignature(fresh.bytes, fresh.offsets, () => Buffer.alloc(MAX_SIG_CONTENTS_BYTES + 1, 1));
  assert.equal(overflow.reason, 'cms-too-large');
  const parsed = parseP12(read('leaf.p12'), password);
  const signed = patchSignature(fresh.bytes, fresh.offsets, content => cmsDetached({
    content, certs: parsed.certs, key: parsed.key, signingTime: new Date(),
  }));
  assert.equal(signed.ok, true);
  assert.notEqual(signed.bytes.toString('ascii', fresh.offsets.byteRangeAt, fresh.offsets.byteRangeAt + 36), BYTE_RANGE_SENTINEL);
});

test('main owns the clock: more than 10 minutes off is invalid', () => {
  assert.equal(signingWhenOk(Date.now()), true);
  assert.equal(signingWhenOk(Date.now() - 11 * 60 * 1000), false);
  assert.equal(signingWhenOk(Date.now() + 11 * 60 * 1000), false);
});

test('certificate-info then certificate-clear then sign-file is no-certificate', () => {
  const hold = createCertHold(40);
  const parsed = parseP12(read('leaf.p12'), password);
  hold.put(1, { key: parsed.key, certs: parsed.certs, cn: 'P0B Signer' });
  hold.clear(1);
  assert.equal(hold.take(1), undefined);
  hold.put(1, { key: parsed.key, certs: parsed.certs, cn: 'P0B Signer' });
  assert.equal(hold.take(1).cn, 'P0B Signer');
  assert.equal(hold.take(1), undefined);
});

test('a held key is dropped after the idle timer', async () => {
  const hold = createCertHold(30);
  const parsed = parseP12(read('leaf.p12'), password);
  hold.put(7, { key: parsed.key, certs: parsed.certs, cn: 'P0B Signer' });
  await new Promise(resolve => setTimeout(resolve, 50));
  assert.equal(hold.take(7), undefined);
});
