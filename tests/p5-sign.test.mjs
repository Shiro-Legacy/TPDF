import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import mupdf from 'mupdf';
import { createEngine, MAX_SAVE_BYTES, MAX_SIG_CONTENTS_BYTES } from './rust-engine.mjs';
import { readFile as readBytes } from 'node:fs/promises';
import { parseP12 } from '../build/sign/pkcs12.js';
import { cmsDetached, patchSignature } from '../build/sign/cms.js';
import { verifyDetached } from './sign-oracle.mjs';

const engine = createEngine(mupdf);
let id = 1;
const bytesOf = b => b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength);
const fixture = async name => bytesOf(await readFile(new URL(`../fixtures/${name}`, import.meta.url)));
const dispatch = (s, request) => engine.dispatch({ id: id++, ...request }, s).reply;
const open = async name => {
  const s = engine.createSession();
  const r = dispatch(s, { op: 'open', name, pdf: await fixture(name) });
  assert.equal(r.ok, true, r.error);
  return s;
};
const request = (overrides = {}) => ({ op: 'sign-prepare', name: 'P0B Signer', reason: 'unit test', page: 0, rect: [72, 620, 340, 680], certify: false, when: Date.now(), ...overrides });

test('P5 signature placeholder is exact-once, visible/invisible, and non-mutating', async () => {
  assert.equal(MAX_SIG_CONTENTS_BYTES, 8192);
  const s = await open('edit-tj.pdf');
  const before = engine.journalState(s);
  const visible = dispatch(s, request());
  assert.equal(visible.ok, true, visible.error);
  const bytes = new Uint8Array(visible.bytes);
  const text = new TextDecoder('latin1').decode(bytes);
  assert.match(text, /\/SubFilter\/adbe\.pkcs7\.detached/);
  assert.match(text, /\/SigFlags 3/);
  assert.equal(text.includes('/AP'), true);
  assert.equal(text.slice(visible.byteRangeAt, visible.byteRangeAt + 36), '[0 1000000000 1000000000 1000000000]');
  assert.equal(bytes[visible.contentsAt - 1], 60);
  assert.equal(bytes[visible.contentsAt + visible.contentsLen], 62);
  assert.equal(new TextDecoder().decode(bytes.slice(visible.contentsAt, visible.contentsAt + visible.contentsLen)), '0'.repeat(MAX_SIG_CONTENTS_BYTES * 2));
  assert.equal(visible.revisions, 1);
  assert.deepEqual(engine.journalState(s), before);
  const parsed = mupdf.Document.openDocument(visible.bytes, 'application/pdf');
  try {
    assert.equal(parsed.asPDF().countVersions(), 1);
    const reopened = engine.createSession();
    assert.equal(dispatch(reopened, { op: 'open', name: 'prepared.pdf', pdf: visible.bytes }).info.signatures, 1);
  }
  finally { parsed.destroy(); }

  const rotated = dispatch(s, request({ page: 1, rect: [72, 500, 300, 560] }));
  assert.equal(rotated.ok, true, rotated.error);
  assert.match(new TextDecoder('latin1').decode(rotated.bytes), /\/Matrix\[0 1 -1 0 0 0\]/);
  const rotatedPdf = mupdf.Document.openDocument(rotated.bytes, 'application/pdf');
  try {
    const page = rotatedPdf.loadPage(1);
    const pixmap = page.toPixmap(mupdf.Matrix.scale(2, 2), mupdf.ColorSpace.DeviceRGB, false);
    try {
      const pixels = pixmap.getPixels();
      let signatureFill = 0;
      for (let i = 0; i < pixels.length; i += pixmap.getNumberOfComponents()) {
        if (pixels[i + 2] - pixels[i] > 8 && pixels[i + 2] - pixels[i + 1] > 8) signatureFill++;
      }
      assert.ok(signatureFill > 500, `rotated widget AP rendered (${signatureFill} blue pixels)`);
    } finally { pixmap.destroy(); page.destroy(); }
  } finally { rotatedPdf.destroy(); }

  const invisible = dispatch(s, request({ rect: null }));
  assert.equal(invisible.ok, true, invisible.error);
  const invisibleText = new TextDecoder('latin1').decode(invisible.bytes);
  const invisibleWidget = /\/FT\/Sig\/T\(Signature1\)\/Rect\[0 0 0 0\](.*?)\/V/.exec(invisibleText)?.[1] ?? '';
  assert.equal(invisibleWidget.includes('/AP'), false);
});

test('P5 certification and signed-source incremental preparation', async () => {
  const unsigned = await open('edit-tj.pdf');
  const certified = dispatch(unsigned, request({ certify: true, rect: null }));
  assert.equal(certified.ok, true, certified.error);
  assert.match(new TextDecoder('latin1').decode(certified.bytes), /\/DocMDP/);
  const certifiedSession = engine.createSession();
  assert.equal(dispatch(certifiedSession, { op: 'open', name: 'certified.pdf', pdf: certified.bytes }).ok, true);
  const already = dispatch(certifiedSession, request({ certify: true }));
  assert.equal(already.reason, 'already-signed');

  const signed = await open('p0b/signed-intact.pdf');
  const source = await fixture('p0b/signed-intact.pdf');
  const next = dispatch(signed, request({ rect: null }));
  assert.equal(next.ok, true, next.error);
  assert.deepEqual(new Uint8Array(next.bytes).slice(0, source.byteLength), new Uint8Array(source));
  assert.equal(next.revisions, 2);
  const reopenedTwice = engine.createSession();
  assert.equal(dispatch(reopenedTwice, { op: 'open', name: 'twice.pdf', pdf: next.bytes }).info.signatures, 2);
});

test('P5 prepare → CMS patch → verify → reopen preserves read-only signed source', async () => {
  const s = await open('edit-tj.pdf');
  const prepared = dispatch(s, request({ rect: null, reason: 'end-to-end' }));
  assert.equal(prepared.ok, true, prepared.error);
  const p12 = await readBytes(new URL('../fixtures/p5/leaf.p12', import.meta.url));
  const { key, certs } = parseP12(p12, 'pdf4all');
  let cms;
  const patched = patchSignature(new Uint8Array(prepared.bytes), prepared, content => {
    cms = cmsDetached({ content, certs, key, signingTime: new Date() });
    return cms;
  });
  assert.equal(patched.ok, true, patched.reason);
  const start = prepared.contentsAt - 1, end = prepared.contentsAt + prepared.contentsLen + 1;
  const content = Buffer.concat([patched.bytes.subarray(0, start), patched.bytes.subarray(end)]);
  const verified = verifyDetached(cms, content, await readBytes(new URL('../fixtures/p0b/.ca/ca.pem', import.meta.url)));
  assert.equal(verified.digestMatches, true);
  assert.equal(verified.signatureValid, true);
  assert.equal(verified.signerCn, 'P0B Signer');
  const reopened = engine.createSession();
  assert.equal(dispatch(reopened, { op: 'open', name: 'signed.pdf', pdf: patched.bytes.buffer.slice(patched.bytes.byteOffset, patched.bytes.byteOffset + patched.bytes.byteLength) }).info.signatures, 1);
  assert.equal(dispatch(reopened, { op: 'save' }).reason, 'read-only');
});

test('P5 locked and too-large sessions refuse before preparing', async () => {
  const locked = await open('encrypted-aes256.pdf');
  assert.equal(dispatch(locked, request()).reason, 'encrypted');
  const large = await open('edit-tj.pdf');
  large.sourceBytes = MAX_SAVE_BYTES + 1;
  assert.equal(dispatch(large, request()).reason, 'too-large');
});

test('P5 certified P=1 source refuses without dirtying', async () => {
  const s = await open('p5/certified-p1.pdf');
  const refusal = dispatch(s, request());
  assert.equal(refusal.reason, 'certified');
  assert.equal(refusal.dirty, false);
});

test('P5 invalid geometry and encrypted source refuse without dirtying', async () => {
  const s = await open('edit-tj.pdf');
  const bad = dispatch(s, request({ rect: [1, 1, 2, 2] }));
  assert.equal(bad.reason, 'invalid');
  assert.equal(bad.dirty, false);
  const encrypted = await open('encrypted-aes256.pdf').catch(async () => open('encrypted-user.pdf'));
  const refusal = dispatch(encrypted, request());
  assert.equal(refusal.reason, 'encrypted');
});

test('P5 appearance shows every line unescaped, /Name keeps Unicode, certified accepts only invisible (review fix)', async () => {
  const s = await open('edit-tj.pdf');
  const name = 'Zoë (QA) — €';
  const prepared = dispatch(s, request({ name, reason: 'approved (final)' }));
  assert.equal(prepared.ok, true, prepared.error);
  const doc = mupdf.PDFDocument.openDocument(prepared.bytes, 'application/pdf');
  const widget = doc.getTrailer().get('Root').get('AcroForm').get('Fields').get(0);
  // Structured text skips annotation appearances, so read the AP stream: one escape per paren, lines stepped by 0 -11 Td.
  // The stream is WinAnsi: — is byte 0x97 and € is 0x80 (read as latin1; Node 23+ decodes "windows-1252" as latin1 anyway).
  const ap = Buffer.from(widget.get('AP').get('N').readStream().asUint8Array()).toString('latin1');
  assert.match(ap, /8 47 Td \(Digitally signed by Zoë \\\(QA\\\) \x97 \x80\) Tj 0 -11 Td \(\d{4}-\d{2}-\d{2} \d{2}:\d{2}\) Tj 0 -11 Td \(Reason: approved \\\(final\\\)\) Tj/);
  const sig = widget.get('V');
  assert.equal(sig.get('Name').asString(), name);

  const certified = dispatch(s, request({ certify: true, rect: null }));
  assert.equal(certified.ok, true, certified.error);
  const c = engine.createSession();
  assert.equal(dispatch(c, { op: 'open', name: 'certified.pdf', pdf: certified.bytes }).ok, true);
  const visible = dispatch(c, request());
  assert.equal(visible.reason, 'certified');
  assert.equal(dispatch(c, request({ rect: null })).ok, true);
});

test('P5 U3: the new-widget path is unchanged beside empty fields (free name Signature4, field null)', async () => {
  const s = await open('p5/empty-sigfield.pdf');
  for (const field of [undefined, null]) {
    const prepared = dispatch(s, request({ rect: null, field }));
    assert.equal(prepared.ok, true, prepared.error);
    assert.equal('field' in prepared, false);
    assert.match(new TextDecoder('latin1').decode(prepared.bytes), /\/FT\/Sig\/T\(Signature4\)/);
  }
  const mixed = dispatch(s, request({ field: 'Signature1' }));
  assert.equal(mixed.reason, 'invalid', 'a field signature takes rect null');
  assert.equal(mixed.error, "sign-prepare: a field signature takes rect null and the field's page");
});
