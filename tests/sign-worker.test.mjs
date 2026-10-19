/**
 * The signing worker main uses (src/sign/signer.ts, src/sign/sign-worker.ts): certificate files are read
 * and documents signed off the main thread, keys stay in the worker behind one-shot handles, and the main
 * event loop keeps running while a slow (many-iteration) file is opened. Node only, no Electron.
 */
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { X509Certificate } from 'node:crypto';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test, { after } from 'node:test';
import mupdf from 'mupdf';
import { createEngine } from './rust-engine.mjs';
import { createSigner } from '../build/sign/signer.js';
import { verifySignature } from '../build/sign/verify.js';

const root = path.resolve(import.meta.dirname, '..');
const ca = new X509Certificate(readFileSync(path.join(root, 'fixtures/p0b/.ca/ca.pem')));
let failures = 0;
const signer = createSigner(new URL('../build/sign/sign-worker.js', import.meta.url), () => { failures++; });
const work = mkdtempSync(path.join(tmpdir(), 'sign-worker-'));
after(async () => { await signer.close(); rmSync(work, { recursive: true, force: true }); });

const engine = createEngine(mupdf);
let id = 1;
async function prepared() {
  const s = engine.createSession();
  const bytes = await readFile(path.join(root, 'fixtures/edit-tj.pdf'));
  assert.equal(engine.dispatch({ id: id++, op: 'open', name: 'edit-tj.pdf', pdf: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) }, s).reply.ok, true);
  const reply = engine.dispatch({ id: id++, op: 'sign-prepare', name: 'Worker', reason: 'worker test', page: 0, rect: null, certify: false, when: Date.now() }, s).reply;
  assert.equal(reply.ok, true, reply.error);
  return reply;
}
const offsetsOf = r => ({ contentsAt: r.contentsAt, contentsLen: r.contentsLen, byteRangeAt: r.byteRangeAt });

function entryFrom(pdf) {
  const text = pdf.toString('latin1');
  const [a, b, c, d] = /\/ByteRange\s*\[\s*(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s*\]/.exec(text).slice(1).map(Number);
  return { field: 'Sig1', page: 0, rect: null, empty: false, subFilter: /\/SubFilter\s*\/([^\s/<>[]+)/.exec(text)[1],
    m: /\/M\s*\((D:[^)]*)\)/.exec(text)?.[1], byteRange: [a, b, c, d], contents: Buffer.from(pdf.subarray(b + 1, c - 1).toString('ascii'), 'hex') };
}

test('open, sign once, and the handle is spent; a wrong password and a dropped handle are refused', async () => {
  const file = readFileSync(path.join(root, 'fixtures/p5/leaf.p12'));
  assert.equal((await signer.open('p12', Buffer.from(file), 'nope', Date.now())).reason, 'bad-password');
  const opened = await signer.open('p12', Buffer.from(file), 'pdf4all', Date.now());
  assert.equal(opened.ok, true, JSON.stringify(opened));
  assert.equal(opened.cn, 'P0B Signer');
  const copy = Buffer.from(file);
  const again = await signer.open('p12', copy, 'pdf4all', Date.now());
  assert.ok(copy.every(byte => byte === 0), 'the caller\'s copy of the file is zeroed');

  const prep = await prepared();
  const signed = await signer.sign(opened.handle, prep.bytes, offsetsOf(prep), Date.now());
  assert.equal(signed.ok, true, JSON.stringify(signed));
  const pdf = Buffer.from(signed.bytes);
  const v = verifySignature(pdf, entryFrom(pdf), [ca]);
  assert.deepEqual([v.status, v.trust, v.signerCn], ['valid', 'trusted', 'P0B Signer']);

  const prep2 = await prepared();
  assert.equal((await signer.sign(opened.handle, prep2.bytes, offsetsOf(prep2), Date.now())).reason, 'no-certificate', 'one shot');
  signer.drop(again.handle);
  const prep3 = await prepared();
  assert.equal((await signer.sign(again.handle, prep3.bytes, offsetsOf(prep3), Date.now())).reason, 'no-certificate');
});

test('an out-of-date time is refused at open and at signing', async () => {
  const file = readFileSync(path.join(root, 'fixtures/p5/leaf.p12'));
  const late = Date.parse(new X509Certificate(readFileSync(path.join(root, 'fixtures/p0b/.ca/leaf.pem'))).validTo) + 86_400_000;
  const refused = await signer.open('p12', Buffer.from(file), 'pdf4all', late);
  assert.equal(refused.reason, 'certificate-expired');
  assert.match(refused.status, /^this certificate expired on /);
  const opened = await signer.open('p12', Buffer.from(file), 'pdf4all', Date.now());
  const prep = await prepared();
  assert.equal((await signer.sign(opened.handle, prep.bytes, offsetsOf(prep), late)).reason, 'certificate-expired');
});

test('the main event loop keeps running while a slow certificate file is opened', async () => {
  const key = path.join(caDirPath('leaf.key'));
  const out = path.join(work, 'slow.p12');
  const run = spawnSync('openssl', ['pkcs12', '-export', '-inkey', key, '-in', caDirPath('leaf.pem'), '-iter', '300000', '-passout', 'pass:pw', '-out', out], { encoding: 'utf8' });
  assert.equal(run.status, 0, run.stderr);
  let ticks = 0;
  const timer = setInterval(() => { ticks++; }, 10);
  const started = Date.now();
  const opened = await signer.open('p12', readFileSync(out), 'pw', Date.now());
  const took = Date.now() - started;
  clearInterval(timer);
  assert.equal(opened.ok, true, JSON.stringify(opened));
  signer.drop(opened.handle);
  // At ~1 ms of work per 1,000 iterations the open takes a good fraction of a second; the timer kept firing.
  assert.ok(ticks >= Math.floor(took / 10 / 4), `ticks ${ticks} in ${took} ms`);
});

function caDirPath(name) {
  return path.join(root, 'fixtures/p0b/.ca', name);
}

test('a handle from a stopped worker never selects a key in the next one (review p2b L1)', async () => {
  const file = readFileSync(path.join(root, 'fixtures/p5/leaf.p12'));
  await signer.close();
  const first = await signer.open('p12', Buffer.from(file), 'pdf4all', Date.now());
  assert.match(first.handle, /^\d+:1$/, 'a fresh worker numbers from 1');
  const before = failures;
  await signer.close();
  assert.equal(failures, before + 1, 'onFailure ran (main clears its hold)');
  const second = await signer.open('p12', Buffer.from(file), 'pdf4all', Date.now());
  assert.equal(second.ok, true);
  assert.notEqual(second.handle, first.handle);
  assert.equal(first.handle.split(':')[1], second.handle.split(':')[1], 'the new worker reuses the number');
  const prep = await prepared();
  assert.equal((await signer.sign(first.handle, prep.bytes, offsetsOf(prep), Date.now())).reason, 'no-certificate');
  const prep2 = await prepared();
  assert.equal((await signer.sign(second.handle, prep2.bytes, offsetsOf(prep2), Date.now())).ok, true);
});

test('a certificate not meant for signing documents opens with forSigning false (review p2b L4)', async () => {
  const key = path.join(work, 'tls.key');
  const cert = path.join(work, 'tls.pem');
  const ext = path.join(work, 'tls.cnf');
  writeFileSync(ext, '[v3]\nbasicConstraints=CA:FALSE\nkeyUsage=critical,keyEncipherment\nextendedKeyUsage=serverAuth\n');
  for (const args of [
    ['genpkey', '-algorithm', 'EC', '-pkeyopt', 'ec_paramgen_curve:P-256', '-out', key],
    ['req', '-new', '-key', key, '-subj', '/CN=TLS Only', '-out', `${cert}.csr`],
    ['x509', '-req', '-in', `${cert}.csr`, '-CA', caDirPath('ca.pem'), '-CAkey', caDirPath('ca.key'), '-set_serial', '4242', '-days', '2', '-extfile', ext, '-extensions', 'v3', '-out', cert],
  ]) assert.equal(spawnSync('openssl', args, { encoding: 'utf8' }).status, 0, args.join(' '));
  const opened = await signer.open('pem', Buffer.concat([readFileSync(key), readFileSync(cert)]), '', Date.now());
  assert.deepEqual([opened.ok, opened.forSigning], [true, false]);
  const leaf = await signer.open('p12', readFileSync(path.join(root, 'fixtures/p5/leaf.p12')), 'pdf4all', Date.now());
  assert.equal(leaf.forSigning, true);
  signer.drop(opened.handle);
  signer.drop(leaf.handle);
});
