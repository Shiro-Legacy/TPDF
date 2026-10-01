/**
 * P5 U2 main-side verification. Node only. Suite before this file: 379 pass + 1 RC2 skip.
 */
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createPrivateKey, X509Certificate } from 'node:crypto';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { cmsDetached } from '../build/sign/cms.js';
import { kids, parseTlv, pemDer } from '../build/sign/der.js';
import { createTrustStore } from '../build/sign/trust.js';
import { verifySignature } from '../build/sign/verify.js';

const root = path.resolve(import.meta.dirname, '..');
const caPem = path.join(root, 'fixtures/p0b/.ca/ca.pem');
const ca = new X509Certificate(readFileSync(caPem));
const other = new X509Certificate(readFileSync(path.join(root, 'fixtures/p0b/.ca/other-ca.pem')));
const leaf = new X509Certificate(readFileSync(path.join(root, 'fixtures/p0b/.ca/leaf.pem')));
const py = path.join(root, '.venv-qa/bin/python');

function pdf(rel) {
  return readFileSync(path.join(root, rel));
}

function entryFrom(buf, extra = {}) {
  const text = buf.toString('latin1');
  const m = /\/ByteRange\s*\[\s*(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s*\]/.exec(text);
  assert.ok(m, 'ByteRange');
  const [a, b, c, d] = m.slice(1).map(Number);
  const hex = buf.subarray(b + 1, c - 1).toString('ascii');
  const sub = /\/SubFilter\s*\/([^\s/<>\[]+)/.exec(text);
  const docmdp = /\/TransformMethod\s*\/DocMDP[\s\S]{0,240}?\/P\s+(\d+)/.exec(text);
  return {
    field: 'Sig1', page: 1, rect: null, empty: false,
    subFilter: sub?.[1] ?? '',
    byteRange: [a, b, c, d],
    contents: Buffer.from(hex, 'hex'),
    docmdp: docmdp ? Number(docmdp[1]) : null,
    ...extra,
  };
}

function pyLine(rel) {
  const run = spawnSync(py, ['scripts/verify-sign.py', path.join(root, rel), '--ca', caPem], { encoding: 'utf8', cwd: root });
  const line = run.stdout.split('\n').find(row => row.startsWith('Sig1 '));
  assert.ok(line, run.stderr);
  return line;
}

function reseat(src, entry, contents) {
  const [, b, c] = entry.byteRange;
  const hex = Buffer.from(contents).toString('hex').toUpperCase();
  assert.equal(hex.length, c - b - 2);
  const next = Buffer.from(src);
  next.write(hex, b + 1, hex.length, 'ascii');
  return { src: next, entry: { ...entry, contents } };
}

test('p0b fixtures match the contract table and pyHanko coverage', () => {
  const rows = [
    ['fixtures/p0b/signed-intact.pdf', 'valid', 'entire-file', 'trusted', 'INTACT ENTIRE_FILE trusted'],
    ['fixtures/p0b/signed-tampered.pdf', 'invalid', 'entire-file', 'none', 'INVALID ENTIRE_FILE'],
    ['fixtures/p0b/signed-untrusted.pdf', 'valid', 'entire-file', 'untrusted', 'INTACT ENTIRE_FILE untrusted'],
    ['fixtures/p0b/certified-p2.pdf', 'valid', 'entire-file', 'trusted', 'INTACT ENTIRE_FILE trusted'],
    ['fixtures/p0b/certified-p2-filled.pdf', 'valid', 'partial', 'trusted', 'INTACT ENTIRE_REVISION trusted'],
    ['fixtures/p0b/certified-p2-annot.pdf', 'valid', 'partial', 'trusted', 'INTACT ENTIRE_REVISION trusted'],
  ];
  for (const [rel, status, coverage, trust, expectPy] of rows) {
    const src = pdf(rel);
    const verdict = verifySignature(src, entryFrom(src), [ca]);
    assert.equal(verdict.status, status, rel);
    assert.equal(verdict.coverage, coverage, rel);
    assert.equal(verdict.trust, trust, rel);
    if (status === 'invalid') assert.equal(verdict.reason, 'digest');
    if (trust === 'trusted') assert.equal(verdict.issuerCn, 'P0B Test CA');
    assert.equal(pyLine(rel).includes(expectPy), true, pyLine(rel));
    if (rel.includes('certified-p2')) assert.equal(verdict.docmdp, 2);
  }
});

test('ec, sha1, and expired fixtures', () => {
  const ec = pdf('fixtures/p5/signed-ec.pdf');
  const ecVerdict = verifySignature(ec, entryFrom(ec), [ca]);
  assert.equal(ecVerdict.status, 'valid');
  assert.equal(ecVerdict.trust, 'trusted');
  assert.equal(ecVerdict.digest, 'sha256');
  assert.equal(ecVerdict.signerCn, 'P0B EC');

  const sha1 = pdf('fixtures/p5/signed-sha1.pdf');
  const sha1Verdict = verifySignature(sha1, entryFrom(sha1), [ca]);
  assert.equal(sha1Verdict.status, 'valid');
  assert.equal(sha1Verdict.reason, 'weak-digest');
  assert.equal(sha1Verdict.trust, 'trusted');
  assert.equal(sha1Verdict.digest, 'sha1');

  const expired = pdf('fixtures/p5/signed-expired.pdf');
  const expiredVerdict = verifySignature(expired, entryFrom(expired), [ca]);
  assert.equal(expiredVerdict.status, 'valid');
  assert.equal(expiredVerdict.trust, 'expired');
  assert.equal(expiredVerdict.issuerCn, undefined);
});

test('a CA:FALSE issuer does not extend trust', () => {
  const src = pdf('fixtures/p5/signed-false-ca.pdf');
  const entry = entryFrom(src);
  const alice = new X509Certificate(readFileSync(path.join(root, 'fixtures/p5/false-ca/alice.pem')));
  assert.equal(alice.ca, false);
  const viaCa = verifySignature(src, entry, [ca]);
  assert.equal(viaCa.status, 'valid');
  assert.equal(viaCa.trust, 'untrusted');
  assert.equal(viaCa.signerCn, 'Forged Bob');
  assert.equal(verifySignature(src, entry, [alice]).trust, 'untrusted');
  assert.equal(verifySignature(src, entry, [alice, ca]).trust, 'untrusted');
});

test('byte-range, digest, coverage, and subfilter failures', () => {
  const src = pdf('fixtures/p0b/signed-intact.pdf');
  const entry = entryFrom(src);
  const flipped = Buffer.from(src);
  flipped[80] ^= 0x01;
  const digest = verifySignature(flipped, entry, [ca]);
  assert.equal(digest.status, 'invalid');
  assert.equal(digest.reason, 'digest');

  const widened = { ...entry, byteRange: [0, entry.byteRange[1], entry.byteRange[2] + 2, entry.byteRange[3] - 2] };
  const hole = verifySignature(src, widened, [ca]);
  assert.equal(hole.status, 'invalid');
  assert.equal(hole.reason, 'byte-range');

  const spaced = Buffer.from(src);
  spaced[entry.byteRange[1] + 1] = 0x20;
  assert.equal(verifySignature(spaced, entry, [ca]).reason, 'byte-range');

  const dirty = Buffer.from(entry.contents);
  dirty[dirty.length - 1] = 1;
  const reseated = reseat(src, entry, dirty);
  assert.equal(verifySignature(reseated.src, reseated.entry, [ca]).reason, 'byte-range');

  const appended = Buffer.concat([src, Buffer.from('\ntrailer')]);
  const partial = verifySignature(appended, entry, [ca]);
  assert.equal(partial.status, 'valid');
  assert.equal(partial.coverage, 'partial');
  assert.equal(partial.covered, src.length);
  assert.equal(partial.total, appended.length);

  const cut = { ...entry, byteRange: [0, entry.byteRange[1], entry.byteRange[2], entry.byteRange[3] - 10] };
  assert.equal(verifySignature(src, cut, [ca]).reason, 'byte-range');

  assert.equal(verifySignature(src, { ...entry, subFilter: 'adbe.x509.rsa_sha1' }, [ca]).status, 'unsupported');
  assert.equal(verifySignature(src, { ...entry, subFilter: 'no.such.filter' }, [ca]).reason, 'no.such.filter');
  assert.equal(verifySignature(src, { field: 'Blank', page: 1, rect: null, empty: true }, []).status, 'empty');
  assert.equal(verifySignature(src, { ...entry, byteRange: [0, 1.5, 2, 3] }, [ca]).reason, 'malformed');
});

test('P=1 plus a later revision is changes-not-permitted, not a status change', () => {
  const src = pdf('fixtures/p5/certified-p1.pdf');
  const entry = entryFrom(src, { docmdp: 1 });
  const later = Buffer.concat([src, Buffer.from('%%later\n')]);
  const verdict = verifySignature(later, entry, [ca]);
  assert.equal(verdict.status, 'valid');
  assert.equal(verdict.trust, 'trusted');
  assert.equal(verdict.coverage, 'partial');
  assert.equal(verdict.reason, 'changes-not-permitted');
});

test('trust anchors: empty, other CA, the CA, and the leaf itself', () => {
  const src = pdf('fixtures/p0b/signed-intact.pdf');
  const entry = entryFrom(src);
  assert.equal(verifySignature(src, entry, []).trust, 'untrusted');
  assert.equal(verifySignature(src, entry, [other]).trust, 'untrusted');
  const trusted = verifySignature(src, entry, [ca]);
  assert.equal(trusted.trust, 'trusted');
  assert.equal(trusted.issuerCn, 'P0B Test CA');
  const self = verifySignature(src, entry, [leaf]);
  assert.equal(self.trust, 'trusted');
  assert.equal(self.issuerCn, 'P0B Signer');
  assert.equal(leaf.ca, false);
  assert.equal(verifySignature(pdf('fixtures/p0b/signed-untrusted.pdf'), entryFrom(pdf('fixtures/p0b/signed-untrusted.pdf')), [leaf]).trust, 'untrusted');
});

test('a not-yet-valid leaf is expired, and a SKI sid is unsupported', () => {
  const dir = mkdtempSync(path.join(tmpdir(), 'p5-future-'));
  try {
    const key = path.join(dir, 'future.key');
    const csr = path.join(dir, 'future.csr');
    const cert = path.join(dir, 'future.pem');
    assert.equal(spawnSync('openssl', ['genrsa', '-out', key, '2048']).status, 0);
    assert.equal(spawnSync('openssl', ['req', '-new', '-key', key, '-subj', '/CN=Future', '-out', csr]).status, 0);
    assert.equal(spawnSync('openssl', [
      'x509', '-req', '-in', csr, '-CA', caPem, '-CAkey', path.join(root, 'fixtures/p0b/.ca/ca.key'),
      '-set_serial', '0x5eed', '-sha256', '-not_before', '20300101000000Z', '-not_after', '20300102000000Z', '-out', cert,
    ]).status, 0);
    const head = Buffer.from('%PDF-1.4\n');
    const tail = Buffer.from('\n%%EOF\n');
    const content = Buffer.concat([head, tail]);
    const cms = cmsDetached({
      content, certs: [pemDer(readFileSync(cert)), pemDer(readFileSync(caPem))],
      key: createPrivateKey(readFileSync(key)), signingTime: new Date(),
    });
    const hex = cms.toString('hex').toUpperCase();
    const src = Buffer.concat([head, Buffer.from('<' + hex + '>'), tail]);
    const b = head.length;
    const c = b + hex.length + 2;
    const entry = { field: 'Sig1', page: 1, rect: null, empty: false, subFilter: 'adbe.pkcs7.detached', byteRange: [0, b, c, tail.length], contents: cms };
    const future = verifySignature(src, entry, [ca]);
    assert.equal(future.status, 'valid');
    assert.equal(future.trust, 'expired');

    const top = kids(cms, parseTlv(cms));
    const signed = kids(cms, top[1])[0];
    const info = kids(cms, kids(cms, kids(cms, signed).filter(part => part.tag === 0x31).pop())[0]);
    const ski = Buffer.from(cms);
    ski[info[1].start] = 0xa0;
    const skiHex = ski.toString('hex').toUpperCase();
    const skiSrc = Buffer.concat([head, Buffer.from('<' + skiHex + '>'), tail]);
    const skiEntry = { ...entry, contents: ski };
    assert.equal(verifySignature(skiSrc, skiEntry, [ca]).status, 'unsupported');
    assert.equal(verifySignature(skiSrc, skiEntry, [ca]).reason, 'signer-id');
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('trust store dedupes, skips garbage, and round-trips through a file', () => {
  const mem = createTrustStore(null);
  const pem = readFileSync(caPem);
  assert.deepEqual(mem.add(pem), { added: 1, total: 1 });
  assert.deepEqual(mem.add(Buffer.concat([Buffer.from('not a cert\n'), pem])), { added: 0, total: 1 });
  assert.equal(mem.list()[0].cn, 'P0B Test CA');
  mem.clear();
  assert.equal(mem.certs().length, 0);

  const dir = mkdtempSync(path.join(tmpdir(), 'p5-trust-'));
  try {
    const file = path.join(dir, 'trusted-certs.pem');
    const store = createTrustStore(file);
    assert.equal(store.add(pem).added, 1);
    const reopened = createTrustStore(file);
    assert.equal(reopened.list()[0].cn, 'P0B Test CA');
    assert.equal(reopened.certs()[0].fingerprint256, ca.fingerprint256);
    reopened.clear();
    assert.equal(readFileSync(file).length, 0);
    assert.equal(createTrustStore(file).list().length, 0);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
