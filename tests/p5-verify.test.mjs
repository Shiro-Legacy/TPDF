import assert from 'node:assert/strict';
import { X509Certificate } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import mupdf from 'mupdf';
import { createEngine } from '../build/renderer/engine.js';
import { verifySignature } from '../build/sign/verify.js';
import { parseP12 } from '../build/sign/pkcs12.js';
import { cmsDetached, patchSignature } from '../build/sign/cms.js';

const engine = createEngine(mupdf);
let id = 1;
const bytesOf = b => b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength);
const fixture = async name => bytesOf(await readFile(new URL(`../fixtures/p0b/${name}.pdf`, import.meta.url)));
const dispatch = (s, request) => engine.dispatch({ id: id++, ...request }, s).reply;

test('sign-list reports signed fields and retained source without changing the session', async () => {
  for (const [name, certified, expectedRange] of [
    ['signed-intact', null, [0, 1355, 8065, 636]],
    ['signed-tampered', null, [0, 1355, 8065, 636]],
    ['signed-untrusted', null, [0, 1355, 8109, 636]],
    ['certified-p2', 2, [0, 1383, 8093, 755]],
    ['certified-p2-filled', 2, [0, 1383, 8093, 755]],
  ]) {
    const source = await fixture(name);
    const session = engine.createSession();
    assert.equal(dispatch(session, { op: 'open', name: `${name}.pdf`, pdf: source }).ok, true);
    const before = engine.journalState(session);
    const result = dispatch(session, { op: 'sign-list' });
    assert.equal(result.ok, true, result.error);
    assert.equal(result.signatures.length, 1);
    const entry = result.signatures[0];
    assert.equal(entry.field, 'Sig1');
    assert.equal(entry.page, 1);
    assert.deepEqual(entry.byteRange, expectedRange);
    assert.equal(entry.contents.length, name === 'signed-untrusted' ? 3376 : 3354);
    assert.equal(entry.docmdp ?? null, certified);
    assert.equal(result.certified, certified);
    assert.equal(result.versions, name === 'certified-p2-filled' ? 3 : 2);
    assert.deepEqual(new Uint8Array(result.bytes), new Uint8Array(source));
    assert.deepEqual(engine.journalState(session), before);
  }
});

test('sign-list entries verify against intact, tampered, and a freshly signed document', async () => {
  const ca = new X509Certificate(await readFile(new URL('../fixtures/p0b/.ca/ca.pem', import.meta.url)));
  for (const [name, status] of [['signed-intact', 'valid'], ['signed-tampered', 'invalid']]) {
    const source = await fixture(name);
    const session = engine.createSession();
    assert.equal(dispatch(session, { op: 'open', name: `${name}.pdf`, pdf: source }).ok, true);
    const listed = dispatch(session, { op: 'sign-list' });
    const verdict = verifySignature(Buffer.from(listed.bytes), listed.signatures[0], [ca]);
    assert.equal(verdict.status, status);
    if (status === 'valid') assert.equal(verdict.trust, 'trusted');
  }

  const session = engine.createSession();
  const unsigned = bytesOf(await readFile(new URL('../fixtures/edit-tj.pdf', import.meta.url)));
  assert.equal(dispatch(session, { op: 'open', name: 'edit-tj.pdf', pdf: unsigned }).ok, true);
  const prepared = dispatch(session, { op: 'sign-prepare', name: 'P0B Signer', reason: 'verify round trip', page: 0, rect: null, certify: false, when: Date.now() });
  assert.equal(prepared.ok, true, prepared.error);
  const { key, certs } = parseP12(await readFile(new URL('../fixtures/p5/leaf.p12', import.meta.url)), 'pdf4all');
  let cms;
  const patched = patchSignature(new Uint8Array(prepared.bytes), prepared, content => {
    cms = cmsDetached({ content, certs, key, signingTime: new Date() });
    return cms;
  });
  assert.equal(patched.ok, true, patched.reason);
  const reopened = engine.createSession();
  const signedBytes = patched.bytes.buffer.slice(patched.bytes.byteOffset, patched.bytes.byteOffset + patched.bytes.byteLength);
  assert.equal(dispatch(reopened, { op: 'open', name: 'round-trip.pdf', pdf: signedBytes }).ok, true);
  const before = engine.journalState(reopened);
  const listed = dispatch(reopened, { op: 'sign-list' });
  const verdict = verifySignature(Buffer.from(listed.bytes), listed.signatures[0], [ca]);
  assert.equal(verdict.status, 'valid');
  assert.equal(verdict.trust, 'trusted');
  assert.deepEqual(engine.journalState(reopened), before);
});

test('sign-list distinguishes no document, unsigned documents, and an unavailable retained source', async () => {
  assert.equal(dispatch(engine.createSession(), { op: 'sign-list' }).reason, 'no-document');
  const source = bytesOf(await readFile(new URL('../fixtures/edit-tj.pdf', import.meta.url)));
  const session = engine.createSession();
  assert.equal(dispatch(session, { op: 'open', name: 'unsigned.pdf', pdf: source }).ok, true);
  const result = dispatch(session, { op: 'sign-list' });
  assert.equal(result.ok, true, result.error);
  assert.deepEqual(result.signatures, []);
  assert.equal(result.bytes.byteLength, 0);

  const signed = engine.createSession();
  assert.equal(dispatch(signed, { op: 'open', name: 'signed.pdf', pdf: await fixture('signed-intact') }).ok, true);
  signed.signedSource = null;
  signed.sourceBytes = 60 * 1024 * 1024;
  assert.equal(dispatch(signed, { op: 'sign-list' }).reason, 'too-large');
});
