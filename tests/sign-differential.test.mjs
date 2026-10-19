/**
 * Phase 2 differential proof (docs/rust-and-ux-plan.md §1.5): the tandem-sign module (crates/tandem-sign)
 * against OpenSSL (openssl CLI, node:crypto) and pyHanko (scripts/verify-sign.py), which stay test-only
 * oracles. Rust-signed output verifies under OpenSSL and pyHanko; OpenSSL-made signatures, PKCS#12 files and
 * PEM keys read and verify under Rust; tampered bytes fail in both. Node only, no Electron.
 */
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash, createPrivateKey, X509Certificate } from 'node:crypto';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import { devNull, tmpdir } from 'node:os';
import path from 'node:path';
import test, { after } from 'node:test';
import mupdf from 'mupdf';
import { createEngine } from './rust-engine.mjs';
import { cmsDetached, MAX_SIG_CONTENTS_BYTES, signPdf } from '../build/sign/cms.js';
import { createCertHold } from '../build/sign/hold.js';
import { signModule } from '../build/sign/native.js';
import { certUsable, parseP12, parsePem, SignError } from '../build/sign/pkcs12.js';
import { createTrustStore } from '../build/sign/trust.js';
import { verifySignature, verifySignatures } from '../build/sign/verify.js';
import { kids, oidOf, parseTlv, signedAttributes, verifyDetached } from './sign-oracle.mjs';

const root = path.resolve(import.meta.dirname, '..');
const caDir = path.join(root, 'fixtures/p0b/.ca');
const caPem = path.join(caDir, 'ca.pem');
const ca = new X509Certificate(readFileSync(caPem));
const py = path.join(root, process.platform === 'win32' ? '.venv-qa/Scripts/python.exe' : '.venv-qa/bin/python');
const hasPy = existsSync(py);
/** pyHanko legs run as subtests that report why they skip. */
const NO_PY = hasPy ? false : `no pyHanko: ${py} is missing (python3 -m venv .venv-qa && .venv-qa/bin/pip install pyhanko==0.37.0)`;
const work = mkdtempSync(path.join(tmpdir(), 'sign-diff-'));
after(() => rmSync(work, { recursive: true, force: true }));

const engine = createEngine(mupdf);
let id = 1;
const dispatch = (s, request) => engine.dispatch({ id: id++, ...request }, s).reply;
const bytesOf = b => b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength);

function openssl(args, input) {
  const run = spawnSync('openssl', args, { encoding: 'utf8', input });
  assert.equal(run.status, 0, `openssl ${args.join(' ')}: ${run.stderr}`);
  return run;
}

let serial = 0x7e57_0000;
/** A leaf issued by the fixture CA: `keygen` are openssl genpkey arguments. Returns paths and the CA-signed PEM. */
function issue(name, keygen, { keyUsage = 'critical,digitalSignature,nonRepudiation' } = {}) {
  const base = `${name.replace(/[^\w]+/g, '-')}-${serial}`;
  const key = path.join(work, `${base}.key`);
  const csr = path.join(work, `${base}.csr`);
  const cert = path.join(work, `${base}.pem`);
  const ext = path.join(work, `${base}.cnf`);
  writeFileSync(ext, `[v3]\nbasicConstraints=CA:FALSE\nkeyUsage=${keyUsage}\nsubjectKeyIdentifier=hash\nauthorityKeyIdentifier=keyid\n`);
  openssl(['genpkey', ...keygen, '-out', key]);
  openssl(['req', '-new', '-utf8', '-key', key, '-subj', `/CN=${name}`, '-out', csr]);
  openssl(['x509', '-req', '-in', csr, '-CA', caPem, '-CAkey', path.join(caDir, 'ca.key'), '-set_serial', String(serial++),
    '-sha256', '-days', '30', '-extfile', ext, '-extensions', 'v3', '-out', cert]);
  return { key, cert };
}

/** The engine's sign-list-shaped entry for the first signature in `pdf`. */
function entryFrom(pdf, extra = {}) {
  const text = pdf.toString('latin1');
  const [a, b, c, d] = /\/ByteRange\s*\[\s*(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s*\]/.exec(text).slice(1).map(Number);
  const sub = /\/SubFilter\s*\/([^\s/<>[]+)/.exec(text);
  const m = /\/M\s*\((D:[^)]*)\)/.exec(text);
  return {
    field: 'Sig1', page: 1, rect: null, empty: false, subFilter: sub?.[1] ?? '', ...(m ? { m: m[1] } : {}),
    byteRange: [a, b, c, d], contents: Buffer.from(pdf.subarray(b + 1, c - 1).toString('ascii'), 'hex'), docmdp: null, ...extra,
  };
}

/** A prepared PDF from the engine (edit-tj.pdf), signed through main's path (signPdf). */
async function signedPdf(identity) {
  const s = engine.createSession();
  const source = bytesOf(await readFile(path.join(root, 'fixtures/edit-tj.pdf')));
  assert.equal(dispatch(s, { op: 'open', name: 'edit-tj.pdf', pdf: source }).ok, true);
  const prepared = dispatch(s, { op: 'sign-prepare', name: 'Differential', reason: 'differential test', page: 0, rect: null, certify: false, when: Date.now() });
  assert.equal(prepared.ok, true, prepared.error);
  const patched = signPdf(new Uint8Array(prepared.bytes), prepared, identity.key, identity.certs);
  assert.equal(patched.ok, true, patched.reason);
  return Buffer.from(patched.bytes);
}

function pyhanko(pdf, name, trusted) {
  const file = path.join(work, name);
  writeFileSync(file, pdf);
  return spawnSync(py, ['scripts/verify-sign.py', file, '--form', '--ca', trusted ? caPem : path.join(work, 'no-such-ca.pem')], { cwd: root, encoding: 'utf8' });
}

function opensslVerify(cms, content, name) {
  const sig = path.join(work, `${name}.der`);
  const data = path.join(work, `${name}.bin`);
  writeFileSync(sig, cms);
  writeFileSync(data, content);
  return spawnSync('openssl', ['cms', '-verify', '-inform', 'DER', '-in', sig, '-content', data, '-CAfile', caPem,
    '-binary', '-purpose', 'any', '-out', devNull], { encoding: 'utf8' });
}

function signedContent(pdf, entry) {
  const [, b, c, d] = entry.byteRange;
  return Buffer.concat([pdf.subarray(0, b), pdf.subarray(c, c + d)]);
}

test('the module and limits.ts agree on the reserved CMS size', () => {
  assert.equal(signModule().maxSigContentsBytes(), MAX_SIG_CONTENTS_BYTES);
});

const p384 = () => issue('P384 Signer', ['-algorithm', 'EC', '-pkeyopt', 'ec_paramgen_curve:P-384']);

for (const [label, load, trusted] of [
  ['RSA PKCS#12', () => parseP12(readFileSync(path.join(root, 'fixtures/p5/leaf.p12')), 'pdf4all'), true],
  // ec.pem has no keyUsage, which pyHanko does not trust (docs/gotchas.md); its integrity is still checked.
  ['P-256 PKCS#12', () => parseP12(readFileSync(path.join(root, 'fixtures/p5/leaf-ec.p12')), 'pdf4all'), false],
  ['P-384 PEM', () => { const { key, cert } = p384(); return parsePem(Buffer.concat([readFileSync(key), readFileSync(cert)]), ''); }, true],
]) {
  test(`Rust-signed (${label}) is PAdES B-B and verifies under openssl, node:crypto and pyHanko; tampering fails in all`, async t => {
    const identity = load();
    const pdf = await signedPdf(identity);
    const entry = entryFrom(pdf);
    const content = signedContent(pdf, entry);
    const cms = entry.contents.subarray(0, parseTlv(entry.contents).end);

    assert.equal(entry.subFilter, 'ETSI.CAdES.detached');
    const rust = verifySignature(pdf, entry, [ca]);
    assert.equal(rust.status, 'valid');
    assert.equal(rust.coverage, 'entire-file');
    assert.equal(rust.trust, 'trusted');
    assert.equal(rust.reason, undefined, 'the claimed time is /M: no no-signing-time note');
    const [, y, mo, d, h, mi, sec] = /^D:(\d{4})(\d\d)(\d\d)(\d\d)(\d\d)(\d\d)Z$/.exec(entry.m);
    assert.equal(rust.signingTime, new Date(Date.UTC(+y, +mo - 1, +d, +h, +mi, +sec)).toISOString());
    const oracle = verifyDetached(cms, content);
    assert.deepEqual([oracle.digestMatches, oracle.signatureValid], [true, true]);
    assert.equal(rust.signerCn, oracle.signerCn);
    const ossl = opensslVerify(cms, content, `ok-${label.replace(/\W/g, '')}`);
    assert.equal(ossl.status, 0, ossl.stderr);

    // PAdES B-B: ESS signing-certificate-v2 names the leaf by its SHA-256.
    const { map } = signedAttributes(cms);
    const ess = map['1.2.840.113549.1.9.16.2.47'];
    assert.ok(ess, 'signing-certificate-v2 attribute');
    const certIdV2 = kids(cms, kids(cms, kids(cms, ess.value)[0])[0]);
    const hash = certIdV2[0];
    assert.equal(hash.tag, 0x04, 'default hash (SHA-256): no AlgorithmIdentifier');
    assert.equal(cms.subarray(hash.valueStart, hash.valueEnd).toString('hex'), createHash('sha256').update(identity.certs[0]).digest('hex'));
    assert.equal(map['1.2.840.113549.1.9.5'], undefined, 'PAdES: no signing-time attribute');
    const contentType = kids(cms, map['1.2.840.113549.1.9.3'].node)[1];
    assert.equal(oidOf(cms, kids(cms, contentType)[0]), '1.2.840.113549.1.7.1', 'content-type id-data');

    await t.test('pyHanko: intact, and PAdES B-B', { skip: NO_PY }, () => {
      const run = pyhanko(pdf, `ok-${label.replace(/\W/g, '')}.pdf`, trusted);
      assert.equal(run.status, 0, run.stdout + run.stderr);
      assert.match(run.stdout, new RegExp(`INTACT ENTIRE_FILE ${trusted ? 'trusted' : 'untrusted'} docmdp-ok PAdES-B-B`));
    });

    // A byte of the binary header comment: every reader still parses the file, but its bytes changed.
    const tampered = Buffer.from(pdf);
    assert.equal(tampered[9], 0x25);
    tampered[11] ^= 0x01;
    const bad = verifySignature(tampered, entry, [ca]);
    assert.deepEqual([bad.status, bad.reason], ['invalid', 'digest']);
    assert.equal(verifyDetached(cms, signedContent(tampered, entry)).digestMatches, false);
    assert.notEqual(opensslVerify(cms, signedContent(tampered, entry), `bad-${label.replace(/\W/g, '')}`).status, 0);
    await t.test('pyHanko: the tampered copy is invalid', { skip: NO_PY }, () => {
      const run = pyhanko(tampered, `bad-${label.replace(/\W/g, '')}.pdf`, trusted);
      assert.equal(run.status, 1);
      assert.match(run.stdout, /INVALID/);
    });
  });
}

/** Wraps an OpenSSL-made detached CMS into the smallest file the verifier accepts. */
function wrap(cms, content) {
  const hex = cms.toString('hex').toUpperCase();
  const head = content.subarray(0, 9);
  const tail = content.subarray(9);
  const src = Buffer.concat([head, Buffer.from(`<${hex}>`), tail]);
  const b = head.length;
  const c = b + hex.length + 2;
  return { src, entry: { field: 'Sig1', page: 0, rect: null, empty: false, subFilter: 'adbe.pkcs7.detached', byteRange: [0, b, c, tail.length], contents: cms } };
}

for (const [label, keygen, md, keyopt, digest] of [
  ['RSA SHA-256', ['-algorithm', 'RSA', '-pkeyopt', 'rsa_keygen_bits:2048'], 'sha256', [], 'sha256'],
  ['RSA SHA-384', ['-algorithm', 'RSA', '-pkeyopt', 'rsa_keygen_bits:2048'], 'sha384', [], 'sha384'],
  ['RSA SHA-512', ['-algorithm', 'RSA', '-pkeyopt', 'rsa_keygen_bits:3072'], 'sha512', [], 'sha512'],
  ['RSA SHA-1', ['-algorithm', 'RSA', '-pkeyopt', 'rsa_keygen_bits:2048'], 'sha1', [], 'sha1'],
  ['RSA-PSS SHA-256', ['-algorithm', 'RSA', '-pkeyopt', 'rsa_keygen_bits:2048'], 'sha256', ['-keyopt', 'rsa_padding_mode:pss', '-keyopt', 'rsa_pss_saltlen:32'], 'sha256'],
  ['RSA-PSS SHA-384 max salt', ['-algorithm', 'RSA', '-pkeyopt', 'rsa_keygen_bits:2048'], 'sha384', ['-keyopt', 'rsa_padding_mode:pss', '-keyopt', 'rsa_pss_saltlen:max'], 'sha384'],
  ['ECDSA P-256 SHA-256', ['-algorithm', 'EC', '-pkeyopt', 'ec_paramgen_curve:P-256'], 'sha256', [], 'sha256'],
  ['ECDSA P-384 SHA-384', ['-algorithm', 'EC', '-pkeyopt', 'ec_paramgen_curve:P-384'], 'sha384', [], 'sha384'],
  ['ECDSA P-521 SHA-512', ['-algorithm', 'EC', '-pkeyopt', 'ec_paramgen_curve:P-521'], 'sha512', [], 'sha512'],
  ['ECDSA P-521 SHA-256', ['-algorithm', 'EC', '-pkeyopt', 'ec_paramgen_curve:P-521'], 'sha256', [], 'sha256'],
]) {
  test(`OpenSSL-made ${label} signature verifies under Rust; a flipped byte fails in both`, () => {
    const name = `OSSL ${label.replace(/\W+/g, ' ')}`;
    const { key, cert } = issue(name, keygen);
    const content = Buffer.from(`%PDF-1.7\n1 0 obj\n<<>>\nendobj\n% ${label}\n%%EOF\n`);
    const data = path.join(work, `${name}.bin`);
    const out = path.join(work, `${name}.der`);
    writeFileSync(data, content);
    openssl(['cms', '-sign', '-binary', '-in', data, '-signer', cert, '-inkey', key, '-md', md, ...keyopt, '-outform', 'DER', '-out', out]);
    const cms = readFileSync(out);
    const { src, entry } = wrap(cms, content);
    const v = verifySignature(src, entry, [ca]);
    assert.equal(v.status, 'valid', JSON.stringify(v));
    assert.equal(v.trust, 'trusted');
    assert.equal(v.digest, digest);
    assert.equal(v.signerCn, name);
    assert.equal(v.reason, digest === 'sha1' ? 'weak-digest' : undefined);

    const flipped = Buffer.from(src);
    flipped[flipped.length - 8] ^= 0x01;
    assert.deepEqual([verifySignature(flipped, entry, [ca]).status, verifySignature(flipped, entry, [ca]).reason], ['invalid', 'digest']);
    const tamperedContent = signedContent(flipped, entry);
    assert.notEqual(opensslVerify(cms, tamperedContent, `${name}-bad`).status, 0);
    assert.equal(opensslVerify(cms, content, `${name}-ok`).status, 0);

    // A corrupted signature value is invalid in Rust and in OpenSSL.
    const sigNode = signedAttributes(cms).info.filter(part => part.tag === 0x04).pop();
    const broken = Buffer.from(cms);
    broken[sigNode.valueEnd - 2] ^= 0x01;
    const wrapped = wrap(broken, content);
    assert.equal(verifySignature(wrapped.src, wrapped.entry, [ca]).status, 'invalid');
    assert.notEqual(opensslVerify(broken, content, `${name}-sig`).status, 0);
  });
}

test('OpenSSL-made PKCS#12 and PEM variants read under Rust like node:crypto reads their keys', () => {
  const { key, cert } = issue('Container Signer', ['-algorithm', 'RSA', '-pkeyopt', 'rsa_keygen_bits:2048']);
  const ec = issue('Container EC', ['-algorithm', 'EC', '-pkeyopt', 'ec_paramgen_curve:P-384']);
  const nodeKey = createPrivateKey(readFileSync(key));
  const cases = [
    ['p12 AES-128 + SHA-256 MAC', ['-keypbe', 'AES-128-CBC', '-certpbe', 'AES-128-CBC', '-macalg', 'sha256'], key, cert],
    ['p12 3DES/RC2 (legacy)', ['-legacy'], key, cert],
    ['p12 3DES both + SHA-1 MAC', ['-keypbe', 'PBE-SHA1-3DES', '-certpbe', 'PBE-SHA1-3DES', '-macalg', 'sha1'], key, cert],
    ['p12 SHA-512 MAC, CA chain', ['-macalg', 'sha512', '-certfile', caPem], key, cert],
    ['p12 P-384', [], ec.key, ec.cert],
  ];
  for (const [label, args, k, c] of cases) {
    const out = path.join(work, `${label.replace(/\W+/g, '-')}.p12`);
    openssl(['pkcs12', '-export', '-inkey', k, '-in', c, ...args, '-passout', 'pass:pässwörd', '-out', out]);
    const parsed = parseP12(readFileSync(out), 'pässwörd');
    const leaf = new X509Certificate(parsed.certs[0]);
    assert.equal(leaf.checkPrivateKey(createPrivateKey(readFileSync(k))), true, label);
    assert.equal(parsed.key.kind, c === cert ? 'rsa' : 'ec', label);
    if (c === cert) assert.equal(parsed.key.bits, nodeKey.asymmetricKeyDetails.modulusLength);
    if (label.includes('CA chain')) assert.equal(parsed.certs.length, 2);
    assert.throws(() => parseP12(readFileSync(out), 'wrong'), err => err instanceof SignError && err.reason === 'bad-password', label);
  }
  const pems = [
    ['PKCS#8 PBES2 AES-128 HMAC-SHA1', ['pkcs8', '-topk8', '-in', key, '-v2', 'aes-128-cbc', '-v2prf', 'hmacWithSHA1', '-passout', 'pass:pw'], cert],
    ['PKCS#8 PBE-SHA1-3DES', ['pkcs8', '-topk8', '-in', key, '-v1', 'PBE-SHA1-3DES', '-passout', 'pass:pw'], cert],
    ['traditional RSA AES-256', ['rsa', '-in', key, '-aes256', '-traditional', '-passout', 'pass:pw'], cert],
    ['traditional RSA 3DES', ['rsa', '-in', key, '-des3', '-traditional', '-passout', 'pass:pw'], cert],
    ['traditional EC AES-128', ['ec', '-in', ec.key, '-aes128', '-passout', 'pass:pw'], ec.cert],
    ['plain PKCS#1', ['rsa', '-in', key, '-traditional'], cert],
    ['plain SEC1', ['ec', '-in', ec.key], ec.cert],
  ];
  for (const [label, args, c] of pems) {
    const out = path.join(work, `${label.replace(/\W+/g, '-')}.key.pem`);
    openssl([...args, '-out', out]);
    const parsed = parsePem(Buffer.concat([readFileSync(out), readFileSync(c)]), label.startsWith('plain') ? '' : 'pw');
    assert.equal(new X509Certificate(parsed.certs[0]).subject, new X509Certificate(readFileSync(c)).subject, label);
    if (!label.startsWith('plain')) {
      assert.throws(() => parsePem(Buffer.concat([readFileSync(out), readFileSync(c)]), 'nope'), err => err instanceof SignError && err.reason === 'bad-password', label);
    }
  }
});

test('the trust store reads certificates like node:crypto does', () => {
  const store = createTrustStore(null);
  const files = [caPem, path.join(caDir, 'leaf.pem'), path.join(caDir, 'ec.pem'), path.join(root, 'fixtures/p5/false-ca/alice.pem')];
  assert.deepEqual(store.add(Buffer.concat(files.map(file => readFileSync(file)))), { added: 4, total: 4 });
  const listed = store.list();
  files.forEach((file, i) => {
    const x509 = new X509Certificate(readFileSync(file));
    assert.equal(listed[i].fingerprint256, x509.fingerprint256);
    assert.equal(listed[i].cn, /(?:^|\n)CN=([^\n]*)/.exec(x509.subject)[1]);
    assert.equal(listed[i].notAfter, x509.validToDate.toISOString());
    assert.equal(JSON.parse(signModule().certInfo(x509.raw)).ca, x509.ca);
  });
  assert.equal(signModule().certInfo(Buffer.from('not a certificate')), undefined);
});

test('verdicts carry a plain-language sentence', () => {
  const read = rel => readFileSync(path.join(root, rel));
  const intact = read('fixtures/p0b/signed-intact.pdf');
  const [trusted] = verifySignatures(intact, [entryFrom(intact)], [ca]);
  assert.match(trusted.plain, /^Signed by “P0B Signer”, dated \d{1,2} [A-Z][a-z]{2} \d{4}\. The document has not changed since it was signed\. The signer's certificate is trusted: it was issued through “P0B Test CA”, which is in your trusted list\.$/);
  const [untrusted] = verifySignatures(intact, [entryFrom(intact)], []);
  assert.match(untrusted.plain, /^The signer's certificate is not in your trusted list, so the name is not confirmed\. Signed by “P0B Signer”, dated /);
  const tampered = read('fixtures/p0b/signed-tampered.pdf');
  assert.match(verifySignatures(tampered, [entryFrom(tampered)], [ca])[0].plain, /has changed since it was signed/);
  const certified = read('fixtures/p0b/certified-p2-filled.pdf');
  const [p2] = verifySignatures(certified, [entryFrom(certified, { docmdp: 2 })], [ca]);
  assert.match(p2.plain, /Later changes were added after signing.*allows only filling in forms and signing/);
  for (const v of [trusted, untrusted, p2]) assert.doesNotMatch(v.plain, /DocMDP|P=|bytes|digest|CMS/);
});

test('an over-long or broken string in one entry never costs the other entries their verdicts', () => {
  // Review p2 #1: a string cut in the middle of a surrogate pair made the whole batch unreadable.
  const intact = readFileSync(path.join(root, 'fixtures/p0b/signed-intact.pdf'));
  const good = entryFrom(intact);
  const cases = [
    { ...good, field: 'Long', reason: 'a'.repeat(1001) },
    { ...good, field: 'Emoji', reason: 'a'.repeat(1000) + '\u{1F600}' },
    { ...good, field: 'Lone', reason: 'x\ud800y' },
    { ...good, field: 'Big', page: 10n },
    null,
  ];
  const verdicts = verifySignatures(intact, [good, ...cases, good], [ca]);
  assert.equal(verdicts.length, cases.length + 2);
  assert.deepEqual([verdicts[0].status, verdicts.at(-1).status], ['valid', 'valid']);
  assert.deepEqual(verdicts.slice(1, 3).map(v => [v.field, v.status, v.reason]), [['Long', 'unsupported', 'malformed'], ['Emoji', 'unsupported', 'malformed']]);
  assert.deepEqual([verdicts[3].field, verdicts[3].status], ['Lone', 'valid'], 'a lone surrogate is replaced, not fatal');
  assert.deepEqual([verdicts[4].field, verdicts[4].page, verdicts[4].status], ['Big', 0, 'valid']);
  assert.deepEqual([verdicts[5].status, verdicts[5].reason], ['unsupported', 'malformed']);
});

test('a PKCS#12 file asking for too many KDF iterations is refused before the work (too-costly)', () => {
  const { key, cert } = issue('Costly Signer', ['-algorithm', 'EC', '-pkeyopt', 'ec_paramgen_curve:P-256']);
  const out = path.join(work, 'costly.p12');
  // No MAC (-nomac); the key bag asks for 2,000,001 PBKDF2 iterations, one over the per-derivation cap.
  openssl(['pkcs12', '-export', '-inkey', key, '-in', cert, '-nomac', '-keypbe', 'AES-128-CBC', '-certpbe', 'NONE', '-iter', '2000001', '-passout', 'pass:pw', '-out', out]);
  const started = Date.now();
  assert.throws(() => parseP12(readFileSync(out), 'pw'), err => err instanceof SignError && err.reason === 'too-costly');
  assert.ok(Date.now() - started < 2000, 'refused without doing the work');
});

test('a later revision that rewrites /M cannot change the signing date (review p2b M1)', async () => {
  const identity = parseP12(readFileSync(path.join(root, 'fixtures/p5/leaf.p12')), 'pdf4all');
  const pdf = await signedPdf(identity);
  const entry = entryFrom(pdf);
  const signed = verifySignature(pdf, entry, [ca]);
  assert.equal(signed.status, 'valid');
  const later = Buffer.concat([pdf, Buffer.from('\n% later revision\n%%EOF\n')]);
  for (const m of ['D:20000101000000Z', 'D:20990101000000Z']) {
    // The engine's sign-list reads /M from the newest version of the dictionary; this one claims another date.
    const v = verifySignature(later, { ...entry, m }, [ca]);
    assert.deepEqual([v.status, v.trust, v.coverage], ['valid', 'trusted', 'partial'], m);
    assert.equal(v.signingTime, signed.signingTime, m);
    assert.match(v.reason, /time-changed/);
    assert.match(v.plain, /A later change edited the signature's date; the date shown is the one that was signed\./);
    assert.match(v.reasonWords, /date edited after signing/);
  }
});

test('verdicts carry cleaned names and reasons in words for the Verify table', () => {
  const name = 'Eve\u202Egnp.exe\u202C Doe';
  const { key, cert } = issue(name, ['-algorithm', 'EC', '-pkeyopt', 'ec_paramgen_curve:P-256']);
  const identity = parsePem(Buffer.concat([readFileSync(key), readFileSync(cert)]), '');
  const content = Buffer.from('%PDF-1.7\n1 0 obj\n<<>>\nendobj\n%%EOF\n');
  const { src, entry } = wrap(cmsDetached({ content, certs: identity.certs, key: identity.key }), content);
  const [v] = verifySignatures(src, [entry], [ca]);
  assert.equal(v.status, 'valid');
  assert.equal(v.signerCn, name, 'STATE keeps the certificate name as it is');
  assert.equal(v.signerName, 'Evegnp.exe Doe');
  assert.equal(v.issuerName, 'P0B Test CA');
  assert.equal(v.reasonWords, 'no date given');
  assert.ok(v.plain.includes('“Evegnp.exe Doe”'));
});

test('a dropped key cannot sign again, and the hold drops keys it clears', () => {
  const parsed = parseP12(readFileSync(path.join(root, 'fixtures/p5/leaf.p12')), 'pdf4all');
  assert.equal(certUsable(parsed.certs, parsed.key, Date.now()).ok, true);
  const hold = createCertHold(1000);
  hold.put(3, { key: parsed.key, certs: parsed.certs, cn: 'P0B Signer' });
  hold.clear(3);
  assert.throws(() => cmsDetached({ content: Buffer.from('x'), certs: parsed.certs, key: parsed.key }));
  hold.clear(3);
});
