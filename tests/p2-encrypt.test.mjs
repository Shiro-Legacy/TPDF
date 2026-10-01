/**
 * P2 unit 10 §A — encryption and permissions, engine direct.
 *
 * Gate: `.stage/gate-u10.mjs`, table in `.quad/shared/p2/p2-encrypt.md`. The three
 * facts the tests below lean on: `authenticatePassword` returns 0 / 2 / 4 (fail /
 * user / owner); `encrypt=keep` re-encrypts an authenticated document with its own
 * algorithm and both passwords; `encrypt=aes-256,permissions=<int>` writes a V5 R6
 * file whose `/P` matches the flag sum.
 *
 * Every reply this file makes is also scanned for the password strings: a password
 * may exist in the worker's session and in the request that carried it, nowhere else.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import mupdf from 'mupdf';
import {
  createEngine, MAX_PASSWORD_CHARS, MAX_SAVE_BYTES,
} from '../build/renderer/engine.js';

const engine = createEngine(mupdf);
const USER = 'pdf4all-user';
const OWNER = 'pdf4all-owner';
const NEW_USER = 'gate-user-9f3';
const NEW_OWNER = 'gate-owner-9f3';

function toArrayBuffer(buffer) {
  return buffer.buffer.slice(buffer.byteOffset, buffer.byteOffset + buffer.byteLength);
}

const fixture = async name => toArrayBuffer(await readFile(`fixtures/${name}.pdf`));

function op(session, request) {
  const result = engine.dispatch({ id: 700 + (request.id ?? 0), ...request }, session);
  return { ...result.reply, transfer: result.transfer };
}

/** An op that must fail with a reason, so a regression does not pass as a new success. */
function refused(session, request, reason) {
  const reply = op(session, request);
  assert.equal(reply.ok, false, `${request.op} should have been refused`);
  if (reason) assert.equal(reply.reason, reason, reply.error);
  return reply;
}

async function open(name, session) {
  const reply = op(session, { op: 'open', name: `${name}.pdf`, pdf: await fixture(name) });
  assert.equal(reply.ok, true, reply.error);
  return reply;
}

function openBytes(name, bytes, session) {
  const reply = op(session, { op: 'open', name, pdf: bytes });
  assert.equal(reply.ok, true, reply.error);
  return reply;
}

/** Reopen written bytes the way a reader would: fresh document, no session state. */
function reopen(bytes) {
  const doc = mupdf.Document.openDocument(bytes, 'application/pdf');
  doc.asPDF()?.disableJS();
  return doc;
}

function auth(bytes, password) {
  const doc = reopen(bytes);
  try {
    return doc.authenticatePassword(password);
  } finally {
    doc.destroy();
  }
}

test('the gate: keep-save preserves RC4, both passwords, permissions and the new note', async () => {
  const session = engine.createSession();
  const opened = await open('encrypted', session);
  assert.equal(opened.info.encrypted, true);
  assert.equal(opened.info.needsPassword, true);
  assert.equal(opened.info.algorithm, 'rc4-128', 'read from /Encrypt without a password');
  assert.equal(opened.auth, 'none');

  // A locked session has no password to re-encrypt with, so the save is refused.
  refused(session, { op: 'save' }, 'encrypted');

  const unlock = op(session, { op: 'authenticate', password: USER });
  assert.equal(unlock.authenticated, true);
  assert.equal(unlock.auth, 'user', 'mupdf returns 2 for the user password');
  assert.equal(session.locked, false);
  assert.equal(session.password, USER, 'held in worker memory for the next save');

  const add = op(session, { op: 'annot-add', page: 0, type: 'Text', point: [72, 600], contents: 'gate note' });
  assert.equal(add.ok, true, add.error);

  const saved = op(session, { op: 'save' });
  assert.equal(saved.ok, true, saved.error);
  assert.ok(saved.bytes.byteLength > 100);

  // The copy needs the ORIGINAL password pair, and the note survived.
  assert.equal(auth(saved.bytes, USER), 2, 'the user password still opens it');
  assert.equal(auth(saved.bytes, OWNER), 4, 'the owner password still opens it');
  assert.equal(auth(saved.bytes, 'nope'), 0, 'a wrong password is refused');
  assert.equal(auth(saved.bytes, ''), 0, 'the empty password is refused');
  const copy = reopen(saved.bytes);
  try {
    copy.authenticatePassword(USER);
    assert.equal(copy.getMetaData('encryption'), 'Standard V2 R3 128-bit RC4', 'algorithm preserved');
    const page = copy.loadPage(0);
    try {
      assert.equal(page.getAnnotations().filter(a => a.getContents() === 'gate note').length, 1);
    } finally {
      page.destroy();
    }
  } finally {
    copy.destroy();
  }
  assert.equal(saved.security.algorithm, 'rc4-128', 'the next save keeps the source algorithm');
  session.doc.destroy();
});

test('security-set AES-256 stores a pending change, dirties the document, and validate reopens it', async () => {
  const session = engine.createSession();
  await open('hello', session);
  assert.equal(engine.detect(session.doc).algorithm, 'none');
  assert.equal(op(session, { op: 'save' }).ok, true, 'a plain document saves unchanged');

  const set = op(session, {
    op: 'security-set', mode: 'aes-256',
    userPassword: NEW_USER, ownerPassword: NEW_OWNER,
    permissions: { print: true, copy: false, edit: false, annotate: true, form: false },
  });
  assert.equal(set.ok, true, set.error);
  assert.equal(set.dirty, true, 'the pending change is unsaved');
  assert.ok(set.revision >= 1, 'the revision bumps so caches refresh');
  assert.deepEqual(set.security, {
    mode: 'aes-256', algorithm: 'aes-256', permissions: ['print', 'annotate'],
    auth: 'none', pending: true,
  });

  const saved = op(session, { op: 'save' });
  assert.equal(saved.ok, true, saved.error);
  const validated = op(session, { op: 'validate', pdf: saved.bytes });
  assert.equal(validated.ok, true, validated.error);
  assert.equal(validated.bytes, saved.bytes, 'validate hands the caller bytes straight back');

  assert.equal(auth(saved.bytes, NEW_USER), 2);
  assert.equal(auth(saved.bytes, NEW_OWNER), 4);
  assert.equal(auth(saved.bytes, 'wrong'), 0);
  assert.equal(auth(saved.bytes, USER), 0, 'the fixture password never opens it');
  const copy = reopen(saved.bytes);
  try {
    copy.authenticatePassword(NEW_USER);
    assert.equal(copy.getMetaData('encryption'), 'Standard V5 R6 256-bit AES');
    assert.equal(copy.hasPermission('print'), true);
    assert.equal(copy.hasPermission('annotate'), true);
    assert.equal(copy.hasPermission('copy'), false);
    assert.equal(copy.hasPermission('edit'), false);
    assert.equal(copy.hasPermission('form'), false);
  } finally {
    copy.destroy();
  }

  // The write landed: clean, and the password is still held for the next save.
  const marked = op(session, { op: 'saved' });
  assert.equal(marked.dirty, false, 'saved clears the pending-change dirt');
  assert.equal(session.password, null, 'a plain document never held one');
  assert.equal(session.security.userPassword, NEW_USER, 'still needed by the next save');
  session.doc.destroy();
});

test('a second save after an AES-256 change writes AES-256 again, not the source encryption', async () => {
  const session = engine.createSession();
  await open('encrypted', session);
  const unlocked = op(session, { op: 'authenticate', password: OWNER });
  assert.equal(unlocked.auth, 'owner', 'owner password');
  assert.equal(unlocked.info.permissions.edit, true, 'the owner is not bound by /P');

  const set = op(session, { op: 'security-set', mode: 'aes-256', userPassword: NEW_USER, ownerPassword: NEW_OWNER, permissions: { print: true } });
  assert.equal(set.ok, true, set.error);
  const first = op(session, { op: 'save' });
  assert.equal(first.ok, true, first.error);
  op(session, { op: 'saved' });
  const second = op(session, { op: 'save' });
  assert.equal(second.ok, true, second.error);
  assert.equal(op(session, { op: 'validate', pdf: second.bytes }).ok, true);
  const copy = reopen(second.bytes);
  try {
    assert.equal(copy.authenticatePassword(NEW_USER), 2, 'the new password still applies');
    assert.equal(copy.getMetaData('encryption'), 'Standard V5 R6 256-bit AES', 'not re-encrypted back to RC4');
  } finally {
    copy.destroy();
  }
  session.doc.destroy();
});

test('removing encryption drops /Encrypt and the held password after the write', async () => {
  const session = engine.createSession();
  await open('encrypted', session);
  assert.equal(op(session, { op: 'authenticate', password: OWNER }).auth, 'owner');
  const set = op(session, { op: 'security-set', mode: 'none' });
  assert.equal(set.ok, true, set.error);
  assert.equal(set.security.mode, 'none');
  assert.equal(set.security.algorithm, 'none');
  const saved = op(session, { op: 'save' });
  assert.equal(saved.ok, true, saved.error);
  assert.equal(op(session, { op: 'validate', pdf: saved.bytes }).ok, true);

  const copy = reopen(saved.bytes);
  try {
    assert.equal(copy.needsPassword(), false, 'no password is needed any more');
    assert.equal(copy.getMetaData('encryption'), 'None');
    assert.equal(copy.asPDF().getTrailer().get('Encrypt').isNull(), true, 'no /Encrypt');
    assert.equal(copy.hasPermission('print'), true);
    assert.equal(copy.hasPermission('edit'), true);
  } finally {
    copy.destroy();
  }
  const marked = op(session, { op: 'saved' });
  assert.equal(marked.dirty, false);
  assert.equal(session.password, null, 'removing encryption drops the held password');
  session.doc.destroy();
});

test('a user-password session cannot change security or exceed /P', async () => {
  // print only: no copy, no edit, no annotate, no form.
  const session = engine.createSession();
  await open('hello', session);
  op(session, { op: 'security-set', mode: 'aes-256', userPassword: USER, ownerPassword: OWNER, permissions: { print: true } });
  const locked = op(session, { op: 'save' });
  assert.equal(locked.ok, true, locked.error);
  session.doc.destroy();

  const reader = engine.createSession();
  openBytes('restricted.pdf', locked.bytes, reader);
  const asUser = op(reader, { op: 'authenticate', password: USER });
  assert.equal(asUser.auth, 'user');
  assert.deepEqual(asUser.info.permissions, {
    print: true, copy: false, edit: false, annotate: false, form: false,
  });

  refused(reader, { op: 'security-set', mode: 'none' }, undefined);
  const annotate = refused(reader, { op: 'annot-add', page: 0, type: 'Text', point: [72, 600], contents: 'x' }, 'read-only');
  assert.match(annotate.error, /annotations/, 'the reason names the missing permission');
  refused(reader, { op: 'page-rotate', page: 0, by: 90 }, 'read-only');
  refused(reader, { op: 'field-set', page: 0, widgetId: 0, value: 'x' }, 'read-only');
  refused(reader, { op: 'undo' }, 'read-only');
  // Reading and saving are still allowed: the file keeps its own encryption.
  assert.equal(op(reader, { op: 'render', page: 0, scale: 1 }).ok, true);
  const kept = op(reader, { op: 'save' });
  assert.equal(kept.ok, true, kept.error);
  assert.equal(auth(kept.bytes, USER), 2, 'the kept copy still opens with the user password');

  // The owner may change it.
  const owner = engine.createSession();
  openBytes('restricted.pdf', locked.bytes, owner);
  const asOwner = op(owner, { op: 'authenticate', password: OWNER });
  assert.equal(asOwner.auth, 'owner');
  assert.equal(asOwner.info.permissions.edit, true, 'owner auth reports every permission');
  assert.equal(op(owner, { op: 'security-set', mode: 'none' }).ok, true);
  owner.doc.destroy();
  reader.doc.destroy();
});

test('security-set validates mode, password shape and permissions before storing anything', async () => {
  const session = engine.createSession();
  await open('hello', session);
  const cases = [
    [{ mode: 'rc4-128', userPassword: 'u' }, /mode must be one of/],
    [{ mode: 'aes-256', userPassword: '', ownerPassword: 'o' }, /non-empty user password/],
    [{ mode: 'aes-256', userPassword: 42 }, /user password must be a string/],
    [{ mode: 'aes-256', userPassword: 'u', ownerPassword: 'a,b' }, /cannot contain a comma/],
    [{ mode: 'aes-256', userPassword: 'x'.repeat(MAX_PASSWORD_CHARS + 1) }, /at most 1024/],
    [{ mode: 'aes-256', userPassword: 'u', permissions: { print: 'yes' } }, /must be true or false/],
  ];
  for (const [request, match] of cases) {
    const reply = op(session, { op: 'security-set', ...request });
    assert.equal(reply.ok, false, JSON.stringify(request));
    assert.match(reply.error, match, reply.error);
    assert.equal(session.security, null, 'a refused request stores nothing');
  }
  // A blank owner password means "the same as the user password".
  const set = op(session, { op: 'security-set', mode: 'aes-256', userPassword: 'solo' });
  assert.equal(set.ok, true, set.error);
  const saved = op(session, { op: 'save' });
  assert.equal(auth(saved.bytes, 'solo'), 6, 'user and owner are the same when the owner is blank');
  session.doc.destroy();
});

test('a source over the save ceiling is refused before any serialization work', async () => {
  const session = engine.createSession();
  await open('hello', session);
  assert.equal(op(session, { op: 'security-set', mode: 'aes-256', userPassword: 'u' }).ok, true);
  // A source over the save ceiling is refused before any serialization work.
  session.sourceBytes = MAX_SAVE_BYTES + 1;
  const big = refused(session, { op: 'save' }, 'too-large');
  assert.match(big.error, /save ceiling/);
  session.doc.destroy();
});

test('passwords never appear in a reply or in a renderer log line', async () => {
  const seen = [];
  const session = engine.createSession();
  const record = request => {
    const result = engine.dispatch({ id: 800 + seen.length, ...request }, session);
    seen.push(result.reply);
    return result.reply;
  };
  const opened = record({ op: 'open', name: 'hello.pdf', pdf: await fixture('hello') });
  assert.equal(opened.ok, true);
  record({ op: 'security-set', mode: 'aes-256', userPassword: NEW_USER, ownerPassword: NEW_OWNER, permissions: { print: true } });
  record({ op: 'annot-add', page: 0, type: 'Text', point: [72, 600], contents: 'x' });
  const saved = record({ op: 'save' });
  record({ op: 'validate', pdf: saved.bytes });
  record({ op: 'saved' });
  record({ op: 'close' });

  const sealed = engine.createSession();
  seen.push(engine.dispatch({ id: 899, op: 'open', name: 'encrypted.pdf', pdf: await fixture('encrypted') }, sealed).reply);
  seen.push(engine.dispatch({ id: 900, op: 'authenticate', password: USER }, sealed).reply);

  const wire = JSON.stringify(seen, (key, value) => (value instanceof ArrayBuffer ? `<${value.byteLength} bytes>` : value));
  for (const secret of [NEW_USER, NEW_OWNER, USER, OWNER]) {
    assert.equal(wire.includes(secret), false, `a reply carried ${secret}`);
  }
  assert.equal(/\bpassword\b/i.test(wire), false, 'no reply has a password field at all');

  // The renderer must not log a password either: every console.log line is checked,
  // and the P2_EDIT security slice names only mode/algorithm/permissions/auth.
  const renderer = await readFile(new URL('../src/renderer/renderer.ts', import.meta.url), 'utf8');
  const logs = renderer.split('\n').filter(line => line.includes('console.log('));
  assert.ok(logs.length > 0);
  for (const line of logs) assert.equal(/password/i.test(line), false, line.trim());
  assert.match(renderer, /security: \{ mode: securityState\.mode, algorithm: securityState\.algorithm, permissions: securityState\.permissions, auth: securityState\.auth \}/);
});

test('garbage=compact drops the deleted page\'s stream from the saved copy (addendum A)', async () => {
  const session = engine.createSession();
  await open('nav', session);
  const deleted = op(session, { op: 'page-delete', page: 0 });
  assert.equal(deleted.ok, true, deleted.error);
  const saved = op(session, { op: 'save' });
  assert.equal(saved.ok, true, saved.error);
  const raw = Buffer.from(saved.bytes).toString('latin1');
  assert.equal(raw.includes('one'), false, "the deleted page's content stream is not in the copy");
  assert.equal(raw.includes('two'), true, 'a surviving page is still there');
  const copy = reopen(saved.bytes);
  try {
    assert.equal(copy.asPDF().countVersions(), 1, 'the copy has one revision');
  } finally {
    copy.destroy();
  }
  session.doc.destroy();
});

test('detect() reports algorithm and permissions for every state', async () => {
  const session = engine.createSession();
  await open('hello', session);
  assert.deepEqual(engine.detect(session.doc), {
    encrypted: false, needsPassword: false, algorithm: 'none', repaired: false, signatures: 0,
    certified: false, xfa: false, attachments: 0, openAction: false, tagged: false, versions: 0,
    printable: true, permissions: { print: true, copy: true, edit: true, annotate: true, form: true },
  });
  session.doc.destroy();

  const withheld = engine.createSession();
  const opened = await open('encrypted-noprint', withheld);
  assert.equal(opened.info.algorithm, 'rc4-128');
  assert.equal(opened.info.permissions.print, false, 'read from /P while locked');
  assert.equal(opened.info.permissions.copy, true);
  assert.equal(opened.auth, 'none');
  withheld.doc.destroy();
});
