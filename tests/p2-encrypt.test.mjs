/**
 * P2 unit 10 §A — encryption and permissions, engine direct.
 *
 * Gate: `.stage/gate-u10.mjs`, table in `.quad/shared/p2/p2-encrypt.md`. The three
 * facts the tests below lean on: `authenticate` unlocks as 'user' or 'owner' or
 * refuses; a keep-save re-encrypts an authenticated document with its own
 * algorithm and both passwords; an AES-256 `security-set` writes a V5 R6 file whose
 * `/P` matches the permissions asked for.
 *
 * Every reply this file makes is also scanned for the password strings: a password
 * may exist in the worker's session and in the request that carried it, nowhere else.
 *
 * The "independent reader" tests hand the engine's saved bytes to pypdf + cryptography
 * (scripts/verify-save.py --encryption, run from .venv-qa) so the encrypted output is
 * not judged by the engine's own decryptor alone; they skip when .venv-qa is absent.
 */
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  createEngine, MAX_PASSWORD_CHARS, MAX_SAVE_BYTES,
} from './rust-engine.mjs';

const engine = createEngine();
const USER = 'pdf4all-user';
const OWNER = 'pdf4all-owner';
const NEW_USER = 'gate-user-9f3';
const NEW_OWNER = 'gate-owner-9f3';

const root = fileURLToPath(new URL('..', import.meta.url));
const python = join(root, process.platform === 'win32' ? '.venv-qa/Scripts/python.exe' : '.venv-qa/bin/python3');
const NO_PY = existsSync(python) ? false : 'no .venv-qa (pypdf + cryptography)';

/** pypdf's report on saved bytes: verify-save.py --encryption, both passwords tried on fresh readers. */
async function pypdfReport(bytes, user, owner) {
  const dir = await mkdtemp(join(tmpdir(), 'tandem-p2-encrypt-'));
  try {
    const pdf = join(dir, 'saved.pdf');
    await writeFile(pdf, Buffer.from(bytes));
    const args = [join(root, 'scripts/verify-save.py'), '--encryption', pdf, '--password', user];
    if (owner !== undefined) args.push('--owner-password', owner);
    const checked = spawnSync(python, args, { encoding: 'utf8' });
    assert.equal(checked.status, 0, checked.stdout + checked.stderr);
    const report = JSON.parse(checked.stdout);
    assert.equal(report.ok, true, checked.stdout);
    return report;
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}

/** The engine's permission names against the /P bits pypdf decoded. */
const P_BIT = { print: 'print', edit: 'modify', copy: 'copy', annotate: 'annotate', form: 'fillForms' };

/** An R6 file's /Perms must decrypt (file key, AES-256-ECB) to the same /P, "adb" and all. */
function assertPermsConsistent(report) {
  assert.equal(report.perms?.checked, true, JSON.stringify(report.perms));
  assert.equal(report.perms.consistent, true, JSON.stringify(report.perms));
  assert.equal(report.perms.pypdfPermsValid, true, "pypdf's own /Perms verdict agrees");
}

/** hello.pdf, security-set AES-256 with these passwords and permissions, saved: the bytes and the set reply. */
async function aesSaved(userPassword, ownerPassword, permissions) {
  const session = engine.createSession();
  await open('hello', session);
  const request = { op: 'security-set', mode: 'aes-256', userPassword };
  if (ownerPassword !== undefined) request.ownerPassword = ownerPassword;
  if (permissions !== undefined) request.permissions = permissions;
  const set = op(session, request);
  assert.equal(set.ok, true, set.error);
  const saved = op(session, { op: 'save' });
  assert.equal(saved.ok, true, saved.error);
  engine.closeDocument(session.doc);
  return { bytes: saved.bytes, set };
}

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
function reopen(bytes, password = null) {
  return engine.reopen(bytes, { password });
}

/** How `password` unlocks fresh bytes: 'user', 'owner', or null when refused. */
function auth(bytes, password) {
  const session = engine.createSession();
  try {
    assert.equal(op(session, { op: 'open', name: 'reopened.pdf', pdf: bytes }).ok, true);
    const reply = op(session, { op: 'authenticate', password });
    return reply.ok === true && reply.authenticated === true ? reply.auth : null;
  } finally {
    op(session, { op: 'close' });
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
  assert.equal(unlock.auth, 'user', 'the user password unlocks as the user');
  assert.equal(session.locked, false);
  assert.equal(session.password, USER, 'held in worker memory for the next save');

  const add = op(session, { op: 'annot-add', page: 0, type: 'Text', point: [72, 600], contents: 'gate note' });
  assert.equal(add.ok, true, add.error);

  const saved = op(session, { op: 'save' });
  assert.equal(saved.ok, true, saved.error);
  assert.ok(saved.bytes.byteLength > 100);

  // The copy needs the ORIGINAL password pair, and the note survived.
  assert.equal(auth(saved.bytes, USER), 'user', 'the user password still opens it');
  assert.equal(auth(saved.bytes, OWNER), 'owner', 'the owner password still opens it');
  assert.equal(auth(saved.bytes, 'nope'), null, 'a wrong password is refused');
  assert.equal(auth(saved.bytes, ''), null, 'the empty password is refused');
  const copy = reopen(saved.bytes, USER);
  try {
    assert.equal(copy.info.algorithm, 'rc4-128', 'algorithm preserved');
    const encrypt = copy.trailer.get('Encrypt');
    assert.deepEqual([encrypt.number('V'), encrypt.number('R'), encrypt.number('Length')], [2, 3, 128], 'Standard V2 R3 128-bit');
    assert.equal(copy.annots(0).filter(a => a.string('Contents') === 'gate note').length, 1);
  } finally {
    engine.closeDocument(copy);
  }
  assert.equal(saved.security.algorithm, 'rc4-128', 'the next save keeps the source algorithm');
  engine.closeDocument(session.doc);
});

test('security-set AES-256 stores a pending change, dirties the document, and validate reopens it', async () => {
  const session = engine.createSession();
  await open('hello', session);
  assert.equal(session.doc.info.algorithm, 'none');
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

  assert.equal(auth(saved.bytes, NEW_USER), 'user');
  assert.equal(auth(saved.bytes, NEW_OWNER), 'owner');
  assert.equal(auth(saved.bytes, 'wrong'), null);
  assert.equal(auth(saved.bytes, USER), null, 'the fixture password never opens it');
  const copy = reopen(saved.bytes, NEW_USER);
  try {
    assert.equal(copy.info.algorithm, 'aes-256');
    const encrypt = copy.trailer.get('Encrypt');
    assert.deepEqual([encrypt.number('V'), encrypt.number('R')], [5, 6], 'Standard V5 R6');
    assert.equal(copy.info.permissions.print, true);
    assert.equal(copy.info.permissions.annotate, true);
    assert.equal(copy.info.permissions.copy, false);
    assert.equal(copy.info.permissions.edit, false);
    assert.equal(copy.info.permissions.form, false);
  } finally {
    engine.closeDocument(copy);
  }

  // The write landed: clean, and the password is still held for the next save.
  const marked = op(session, { op: 'saved' });
  assert.equal(marked.dirty, false, 'saved clears the pending-change dirt');
  assert.equal(session.password, null, 'a plain document never held one');
  assert.equal(session.security.userPassword, NEW_USER, 'still needed by the next save');
  engine.closeDocument(session.doc);
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
  assert.equal(auth(second.bytes, NEW_USER), 'user', 'the new password still applies');
  const copy = reopen(second.bytes, NEW_USER);
  try {
    assert.equal(copy.info.algorithm, 'aes-256', 'not re-encrypted back to RC4');
    const encrypt = copy.trailer.get('Encrypt');
    assert.deepEqual([encrypt.number('V'), encrypt.number('R')], [5, 6], 'Standard V5 R6');
  } finally {
    engine.closeDocument(copy);
  }
  engine.closeDocument(session.doc);
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
    assert.equal(copy.info.needsPassword, false, 'no password is needed any more');
    assert.equal(copy.info.algorithm, 'none');
    assert.equal(copy.trailer.get('Encrypt'), null, 'no /Encrypt');
    assert.equal(copy.info.permissions.print, true);
    assert.equal(copy.info.permissions.edit, true);
  } finally {
    engine.closeDocument(copy);
  }
  const marked = op(session, { op: 'saved' });
  assert.equal(marked.dirty, false);
  assert.equal(session.password, null, 'removing encryption drops the held password');
  engine.closeDocument(session.doc);
});

test('a user-password session cannot change security or exceed /P', async () => {
  // print only: no copy, no edit, no annotate, no form.
  const session = engine.createSession();
  await open('hello', session);
  op(session, { op: 'security-set', mode: 'aes-256', userPassword: USER, ownerPassword: OWNER, permissions: { print: true } });
  const locked = op(session, { op: 'save' });
  assert.equal(locked.ok, true, locked.error);
  engine.closeDocument(session.doc);

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
  assert.equal(auth(kept.bytes, USER), 'user', 'the kept copy still opens with the user password');

  // The owner may change it.
  const owner = engine.createSession();
  openBytes('restricted.pdf', locked.bytes, owner);
  const asOwner = op(owner, { op: 'authenticate', password: OWNER });
  assert.equal(asOwner.auth, 'owner');
  assert.equal(asOwner.info.permissions.edit, true, 'owner auth reports every permission');
  assert.equal(op(owner, { op: 'security-set', mode: 'none' }).ok, true);
  engine.closeDocument(owner.doc);
  engine.closeDocument(reader.doc);
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
  // One password that is both unlocks with the owner's rights.
  assert.equal(auth(saved.bytes, 'solo'), 'owner', 'user and owner are the same when the owner is blank');
  engine.closeDocument(session.doc);
});

test('independent reader: pypdf opens the RC4 keep-save with the user password as user and the owner password as owner', { skip: NO_PY }, async () => {
  const session = engine.createSession();
  await open('encrypted', session);
  assert.equal(op(session, { op: 'authenticate', password: USER }).auth, 'user');
  assert.equal(op(session, { op: 'annot-add', page: 0, type: 'Text', point: [72, 600], contents: 'gate note' }).ok, true);
  const saved = op(session, { op: 'save' });
  assert.equal(saved.ok, true, saved.error);
  engine.closeDocument(session.doc);

  const report = await pypdfReport(saved.bytes, USER, OWNER);
  assert.equal(report.algorithm, 'RC4-128');
  assert.deepEqual([report.encrypt.V, report.encrypt.R, report.encrypt.Length], [2, 3, 128]);
  assert.equal(report.passwords.user.level, 'user', 'the user password decrypts at user level');
  assert.equal(report.passwords.owner.level, 'owner', 'the owner password decrypts at owner level');
  assert.equal(report.emptyOpens, false);
  assert.equal(report.perms, null, 'no /Perms before R6');
  // The source's /P came through unchanged.
  const source = await pypdfReport(await fixture('encrypted'), USER, OWNER);
  assert.equal(report.PUnsigned, source.PUnsigned, '/P kept from the source');
});

test('independent reader: the AES-256 owner password unlocks as owner, the user password as user, /Perms agrees with /P', { skip: NO_PY }, async () => {
  const { bytes } = await aesSaved(NEW_USER, NEW_OWNER, { print: true, copy: false, edit: false, annotate: true, form: false });
  const report = await pypdfReport(bytes, NEW_USER, NEW_OWNER);
  assert.equal(report.algorithm, 'AES-256');
  assert.deepEqual([report.encrypt.V, report.encrypt.R, report.encrypt.Length], [5, 6, 256]);
  assert.equal(report.passwords.owner.decrypted, true);
  assert.equal(report.passwords.owner.level, 'owner');
  assert.equal(report.passwords.user.decrypted, true);
  assert.equal(report.passwords.user.level, 'user');
  assert.equal(report.emptyOpens, false);
  assertPermsConsistent(report);
  const wrong = await pypdfReport(bytes, 'wrong', USER);
  assert.equal(wrong.passwords.user.decrypted, false, 'a wrong password is refused');
  assert.equal(wrong.passwords.owner.decrypted, false, 'the fixture password is refused');
});

test('independent reader: the /P the engine writes matches the permissions asked for, bit by bit', { skip: NO_PY }, async () => {
  const sets = [
    { print: true, copy: false, edit: false, annotate: true, form: false },
    { print: true, copy: false, edit: false, annotate: false, form: false },
    { print: false, copy: true, edit: false, annotate: false, form: true },
    { print: false, copy: false, edit: true, annotate: false, form: false },
    { print: true, copy: true, edit: true, annotate: true, form: true },
    { print: false, copy: false, edit: false, annotate: false, form: false },
  ];
  for (const permissions of sets) {
    const { bytes, set } = await aesSaved(NEW_USER, NEW_OWNER, permissions);
    const report = await pypdfReport(bytes, NEW_USER, NEW_OWNER);
    const label = JSON.stringify(permissions);
    for (const [name, bit] of Object.entries(P_BIT)) {
      assert.equal(report.permissionBits[bit], permissions[name], `${label}: /P ${bit} bit`);
      assert.equal(set.security.permissions.includes(name), permissions[name], `${label}: engine reply ${name}`);
    }
    assert.equal(report.PUnsigned & 3, 0, `${label}: bits 1-2 of /P must be 0`);
    assertPermsConsistent(report);
  }
});

test('independent reader: a blank owner password makes the user password the owner password too', { skip: NO_PY }, async () => {
  const { bytes } = await aesSaved('solo');
  const report = await pypdfReport(bytes, 'solo', 'solo');
  assert.equal(report.algorithm, 'AES-256');
  // pypdf tries the owner password first, so one password that is both reports owner twice.
  assert.equal(report.passwords.user.level, 'owner');
  assert.equal(report.passwords.owner.level, 'owner');
  assert.equal(report.emptyOpens, false);
  assertPermsConsistent(report);
});

test('a source over the save ceiling is refused before any serialization work', async () => {
  const session = engine.createSession();
  await open('hello', session);
  assert.equal(op(session, { op: 'security-set', mode: 'aes-256', userPassword: 'u' }).ok, true);
  // A source over the save ceiling is refused before any serialization work.
  session.sourceBytes = MAX_SAVE_BYTES + 1;
  const big = refused(session, { op: 'save' }, 'too-large');
  assert.match(big.error, /save ceiling/);
  engine.closeDocument(session.doc);
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

  // That no log line or STATE carries a password is a Rust test: crates/tandem-app/tests/words.rs.
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
    assert.equal(copy.info.versions, 0, 'the copy has one revision (no update after the first)');
  } finally {
    engine.closeDocument(copy);
  }
  engine.closeDocument(session.doc);
});

test('detect() reports algorithm and permissions for every state', async () => {
  const session = engine.createSession();
  await open('hello', session);
  assert.deepEqual(session.doc.info, {
    encrypted: false, needsPassword: false, algorithm: 'none', repaired: false, signatures: 0,
    certified: false, xfa: false, xfaRendered: false, attachments: 0, openAction: false, tagged: false, versions: 0,
    printable: true, permissions: { print: true, copy: true, edit: true, annotate: true, form: true },
  });
  engine.closeDocument(session.doc);

  const withheld = engine.createSession();
  const opened = await open('encrypted-noprint', withheld);
  assert.equal(opened.info.algorithm, 'rc4-128');
  assert.equal(opened.info.permissions.print, false, 'read from /P while locked');
  assert.equal(opened.info.permissions.copy, true);
  assert.equal(opened.auth, 'none');
  engine.closeDocument(withheld.doc);
});
