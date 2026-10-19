/**
 * sign-merge (contract-sign-merge.md D7): (a) the engine's image signature (`annot-add` Stamp + image +
 * `signature`) and `annot-move` on signature-appearance Stamps; (b) the store's image form is a Rust test now
 * (crates/tandem-app/tests/sign_store.rs);
 * (c) the drive: image and drawn signatures placed, moved, resized, deleted the same way, the bare `sign` op
 * arming placement; (d) static: the old Sign… dialog and its menu item are gone. Engine harness and
 * drive()/red()/brief()/header() copied from tests/draw-signature.test.mjs (not shared on purpose).
 */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { copyFile, mkdtemp as makeTempDir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test, { after } from 'node:test';
import mupdf from 'mupdf';
import { createEngine } from './rust-engine.mjs';
import { blankPdf } from './blank-pdf.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const engine = createEngine(mupdf);
const CI_NO_ELECTRON = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON' : false;
/** Windows has no POSIX permission bits (the store sits under the user's own profile there) and no FIFO files. */
const WIN = process.platform === 'win32';
const buffer = bytes => bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);

const madeDirs = [];
const mkdtemp = async prefix => { const dir = await makeTempDir(prefix); madeDirs.push(dir); return dir; };
after(() => Promise.all(madeDirs.map(dir => rm(dir, { recursive: true, force: true, maxRetries: 5 }))));

function openBytes(bytes, name = 'b.pdf') {
  const s = engine.createSession();
  const reply = engine.dispatch({ id: 1, op: 'open', name, pdf: buffer(bytes) }, s).reply;
  assert.equal(reply.ok, true, reply.error);
  return s;
}
const request = (s, req) => engine.dispatch({ id: 2, ...req }, s).reply;
const blank = () => openBytes(blankPdf(612, 792));
const near = (actual, expected, label, tolerance = 0.01) => assert.ok(Math.abs(actual - expected) <= tolerance, `${label}: ${actual} vs ${expected}`);
const GRAY = await readFile(join(root, 'fixtures/img-gray.png'));
const image = () => ({ name: 'signature', bytes: buffer(GRAY) });
const stamps = s => request(s, { op: 'annots', page: 0 }).items.filter(item => item.type === 'Stamp');
const addImageSig = (s, rect = [100, 100, 300, 160], extra = {}) => request(s, { op: 'annot-add', page: 0, type: 'Stamp', rect, image: image(), signature: true, contents: '', ...extra });
function savedStamps(s) {
  const saved = request(s, { op: 'save' });
  assert.equal(saved.ok, true, saved.error);
  const doc = mupdf.Document.openDocument(new Uint8Array(saved.bytes), 'application/pdf');
  return doc.loadPage(0).getAnnotations().filter(annot => annot.getType() === 'Stamp');
}
const today = () => { const now = new Date(), pad = v => String(v).padStart(2, '0'); return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`; };

test('(a) annot-add Stamp + image + signature: the signature subject, no /Contents, no /T, the rect asked for, an image appearance; refusals; the old sign path unchanged', () => {
  const s = blank();
  const added = addImageSig(s);
  assert.equal(added.ok, true, added.error);
  assert.equal(added.revision, 1);
  const [listed] = stamps(s);
  assert.equal(listed.subject, 'Signature appearance');
  assert.equal(listed.contents, '');
  assert.deepEqual(listed.rect.map(v => Math.round(v * 100) / 100), [100, 100, 300, 160]);
  const [stamp] = savedStamps(s);
  assert.equal(stamp.getSubject(), 'Signature appearance');
  assert.ok(stamp.getObject().get('Contents').isNull(), 'no /Contents');
  assert.ok(stamp.getObject().get('T').isNull(), 'no /T');
  assert.deepEqual([...stamp.getRect()].map(v => Math.round(v * 100) / 100), [100, 100, 300, 160]);
  const ap = stamp.getObject().get('AP').get('N');
  assert.equal(ap.isNull(), false, 'an appearance stream');
  assert.equal(ap.get('Resources').get('XObject').isNull(), false, 'the appearance draws an image');

  // signature with sign, on a Stamp without an image, with an icon, or non-boolean: refused before any write.
  for (const extra of [
    { type: 'Stamp', rect: [100, 100, 300, 160], image: image(), sign: { name: 'Ada' }, signature: true },
    { type: 'Stamp', rect: [100, 100, 300, 160], signature: true },
    { type: 'Stamp', rect: [100, 100, 300, 160], icon: 'Draft', signature: true },
    { type: 'Stamp', rect: [100, 100, 300, 160], image: image(), signature: 'yes' },
    { type: 'Square', rect: [100, 100, 200, 200], signature: true },
  ]) {
    const t = blank();
    const refused = request(t, { op: 'annot-add', page: 0, contents: '', ...extra });
    assert.equal(refused.ok, false, JSON.stringify(Object.keys(extra)));
    assert.equal(refused.dirty, false);
    assert.equal(refused.revision, 0);
    assert.equal(request(t, { op: 'annots', page: 0 }).items.length, 0);
  }
  const square = request(blank(), { op: 'annot-add', page: 0, type: 'Square', rect: [100, 100, 200, 200], signature: true, contents: '' });
  assert.equal(square.error, 'annot-add: signature applies to Ink only and must be true or false', 'the other types keep draw-signature\'s sentence');

  // The old path: Stamp + image + sign:{name} still writes the dated line.
  const old = blank();
  assert.equal(request(old, { op: 'annot-add', page: 0, type: 'Stamp', rect: [72, 72, 272, 132], image: image(), sign: { name: 'Ada Lovelace' }, contents: '' }).ok, true);
  assert.equal(stamps(old)[0].contents, `Signed by Ada Lovelace on ${today()}`);
});

test('(a) annot-move on an image signature: point moves, width resizes keeping the aspect, clamps, no-ops; icon stamps refused; old appearances move; undo reverts each', () => {
  const s = blank();
  assert.equal(addImageSig(s).ok, true);
  const [sig] = stamps(s);
  const moved = request(s, { op: 'annot-move', page: 0, annotId: sig.id, point: [150, 200] });
  assert.equal(moved.ok, true, moved.error);
  assert.equal(moved.revision, 2);
  assert.deepEqual(stamps(s)[0].rect.map(v => Math.round(v * 100) / 100), [150, 200, 350, 260], 'shifted, size kept');

  const resized = request(s, { op: 'annot-move', page: 0, annotId: sig.id, point: [150, 200], width: 100 });
  assert.equal(resized.ok, true, resized.error);
  const [small] = stamps(s);
  near(small.rect[0], 150, 'left kept');
  near(small.rect[1], 200, 'top kept');
  near(small.rect[2] - small.rect[0], 100, 'width 100');
  near(small.rect[3] - small.rect[1], 30, 'aspect kept (200x60 -> 100x30)');

  assert.equal(request(s, { op: 'annot-move', page: 0, annotId: sig.id, point: [150, 200], width: 5 }).ok, true);
  near(stamps(s)[0].rect[2] - stamps(s)[0].rect[0], 24, 'width clamped to 24');
  near(stamps(s)[0].rect[3] - stamps(s)[0].rect[1], 7.2, 'aspect kept at 24');

  assert.equal(request(s, { op: 'annot-move', page: 0, annotId: sig.id, point: [600, 790] }).ok, true);
  const [edge] = stamps(s);
  assert.ok(edge.rect[0] >= 0 && edge.rect[1] >= 0 && edge.rect[2] <= 612.01 && edge.rect[3] <= 792.01, `inside the page: ${edge.rect}`);

  const same = request(s, { op: 'annot-move', page: 0, annotId: sig.id, point: [edge.rect[0], edge.rect[1]], width: edge.rect[2] - edge.rect[0] });
  assert.equal(same.ok, true, same.error);
  assert.equal(same.noop, true);
  assert.equal(same.revision, 5, 'a no-op journals nothing');

  // An icon stamp keeps today's refusal.
  assert.equal(request(s, { op: 'annot-add', page: 0, type: 'Stamp', rect: [300, 300, 400, 350], icon: 'Draft', contents: '' }).ok, true);
  const icon = stamps(s).find(item => item.subject !== 'Signature appearance');
  const refused = request(s, { op: 'annot-move', page: 0, annotId: icon.id, point: [10, 10] });
  assert.equal(refused.ok, false);
  assert.equal(refused.error, 'annot-move: only FreeText annotations can be moved');
  assert.equal(refused.revision, 6);

  // An appearance from the old Sign… dialog is a signature too: it moves.
  assert.equal(request(s, { op: 'annot-add', page: 0, type: 'Stamp', rect: [72, 400, 272, 460], image: image(), sign: { name: 'Ada' }, contents: '' }).ok, true);
  const old = stamps(s).find(item => item.contents.startsWith('Signed by'));
  const oldMoved = request(s, { op: 'annot-move', page: 0, annotId: old.id, point: [100, 500], width: 100 });
  assert.equal(oldMoved.ok, true, oldMoved.error);
  assert.deepEqual(stamps(s).find(item => item.id === old.id).rect.map(v => Math.round(v * 100) / 100), [100, 500, 200, 530]);

  assert.equal(request(s, { op: 'undo' }).ok, true); // the old appearance's move
  assert.deepEqual(stamps(s).find(item => item.id === old.id).rect.map(v => Math.round(v * 100) / 100), [72, 400, 272, 460]);
  for (let i = 0; i < 2; i++) assert.equal(request(s, { op: 'undo' }).ok, true); // the old add, the icon
  assert.equal(request(s, { op: 'undo' }).ok, true); // the edge move
  near(stamps(s)[0].rect[0], 150, 'undo reverts the edge move');
  assert.equal(request(s, { op: 'undo' }).ok, true); // the clamp to 24
  near(stamps(s)[0].rect[2] - stamps(s)[0].rect[0], 100, 'undo reverts the 24 resize');
  assert.equal(request(s, { op: 'undo' }).ok, true); // the 100 resize
  near(stamps(s)[0].rect[2] - stamps(s)[0].rect[0], 200, 'undo reverts the resize');
  assert.equal(request(s, { op: 'undo' }).ok, true); // the move
  assert.deepEqual(stamps(s)[0].rect.map(v => Math.round(v * 100) / 100), [100, 100, 300, 160], 'undo reverts the move');
});

test('(review) annot-move on a foreign signature Stamp larger than the page brings it inside the page, aspect kept', () => {
  // A document from elsewhere: an image Stamp with /Subj (Signature appearance) whose rect overhangs the page.
  const pdf = new mupdf.PDFDocument(blankPdf(612, 792));
  const annot = pdf.loadPage(0).createAnnotation('Stamp');
  annot.setRect([100, 100, 300, 160]); annot.setStampImage(new mupdf.Image(GRAY)); annot.setSubject('Signature appearance'); annot.update();
  annot.setRect([-50, -50, 700, 900]);
  const s = openBytes(pdf.saveToBuffer('').asUint8Array());
  const [sig] = stamps(s);
  const moved = request(s, { op: 'annot-move', page: 0, annotId: sig.id, point: [10, 10] });
  assert.equal(moved.ok, true, moved.error);
  const [x0, y0, x1, y1] = stamps(s)[0].rect;
  assert.ok(x0 >= -0.01 && y0 >= -0.01 && x1 <= 612.01 && y1 <= 792.01, `inside the page: ${[x0, y0, x1, y1]}`);
  near((x1 - x0) / (y1 - y0), 750 / 950, 'aspect kept');
  assert.equal(request(s, { op: 'undo' }).ok, true);
  assert.deepEqual(stamps(s)[0].rect.map(v => Math.round(v * 100) / 100), [-50, -50, 700, 900], 'undo restores it');
});

async function drive(script, stage, timeoutMs = 90_000) {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-drive-test-'));
  await stage(dir);
  const report = join(dir, 'report.json');
  const child = spawn(process.execPath, [join(root, 'scripts/drive.mjs'), script, `--out=${dir}`, `--report=${report}`], { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
  let stdout = '';
  child.stdout.setEncoding('utf8');
  child.stdout.on('data', chunk => { stdout += chunk; });
  const timer = setTimeout(() => child.kill(), timeoutMs);
  const code = await new Promise(resolve => child.on('close', resolve));
  clearTimeout(timer);
  return { code, report: JSON.parse(stdout), dir, stdout };
}
const red = steps => steps.filter(step => step.expect !== undefined && !step.ok);
const brief = step => ({ cmd: step.cmd, save: step.state?.save, revision: step.state?.revision, dirty: step.state?.dirty, error: step.state?.error ?? step.error, announcement: step.state?.announcement, fill: step.state?.fill });
async function header(script, steps, clicks) {
  const text = await readFile(join(root, script), 'utf8');
  const found = text.match(/(\d+) steps, (\d+) click lines/);
  assert.ok(found, 'script header pins the step and click counts');
  assert.deepEqual([Number(found[1]), Number(found[2])], [steps, clicks]);
}

test('(c) drive: an image signature from the pad, placed, moved and resized like a drawing; a drawing replaces it; the bare sign op arms; merged.pdf keeps both', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async () => {
  const SCRIPT = 'tests/drive/sign-merge.jsonl';
  await header(SCRIPT, 18, 0);
  const { code, report, dir } = await drive(SCRIPT, out => copyFile(join(root, 'fixtures/img-rgb.png'), join(out, 'signature.png')), 150_000);
  const steps = report.steps;
  assert.equal(steps.length, 18, report.error ?? '');
  assert.equal(red(steps).length, 0, JSON.stringify(red(steps).map(brief)));
  assert.equal(code, 0);

  const RGB = await readFile(join(root, 'fixtures/img-rgb.png'));
  const all = JSON.stringify(report);
  assert.ok(!all.includes(RGB.toString('base64').slice(0, 40)), 'no image base64 in the report');
  for (const step of steps) assert.ok(!JSON.stringify(step.state ?? {}).includes('signature.png'), 'no file name in STATE');
  for (const step of steps.filter(item => item.cmd === 'sign-draw')) assert.equal(step.contents, '[redacted]');

  assert.deepEqual(steps[3].state.fill.signature, { saved: true, kind: 'image', strokes: 0, points: 0, armed: false, pad: false });
  const placed = steps[4].state.fill.signatures;
  assert.equal(placed.length, 1);
  assert.equal(placed[0].kind, 'image');
  const [x0, y0, x1, y1] = placed[0].rect;
  near((x0 + x1) / 2, 300, 'centred x', 0.05);
  near((y0 + y1) / 2, 650, 'centred y', 0.05);
  assert.ok(x1 - x0 <= 200.01 && y1 - y0 <= 60.01, `inside 200x60: ${placed[0].rect}`);
  near((x1 - x0) / (y1 - y0), 1.5, 'the image aspect (300x200)', 0.01);
  near(y1 - y0, 60, 'fills the 60 pt height', 0.05);
  const dragged = steps[5].state.fill.signatures[0].rect;
  near(dragged[0], x0 + 40, 'dragged x', 1.5);
  near(dragged[1], y0 - 100, 'dragged y', 1.5);
  near(dragged[2] - dragged[0], x1 - x0, 'a drag keeps the width', 0.05);
  const grown = steps[6].state.fill.signatures[0].rect;
  near(grown[2] - grown[0], (dragged[2] - dragged[0]) * 1.1, '+ grows the width by 10 %', 0.05);
  near((grown[2] - grown[0]) / (grown[3] - grown[1]), 1.5, '+ keeps the aspect', 0.01);

  assert.deepEqual(steps[7].state.fill.signature, { saved: true, kind: 'drawn', strokes: 2, points: 7, armed: false, pad: false });
  assert.deepEqual(steps[8].state.fill.signatures.map(item => item.kind).sort(), ['drawn', 'image']);
  assert.deepEqual(steps[9].state.fill.signatures.map(item => item.kind), ['image'], 'Delete removed the focused drawing');
  assert.equal(steps[10].state.fill.signatures.length, 2);

  const doc = mupdf.Document.openDocument(await readFile(join(dir, 'merged.pdf')), 'application/pdf');
  const annots = doc.loadPage(0).getAnnotations();
  const stampList = annots.filter(annot => annot.getType() === 'Stamp');
  const inkList = annots.filter(annot => annot.getType() === 'Ink');
  assert.equal(stampList.length, 1);
  assert.equal(stampList[0].getSubject(), 'Signature appearance');
  assert.equal(stampList[0].getContents(), '');
  assert.ok(stampList[0].getObject().get('T').isNull(), 'no /T on the image signature');
  assert.equal(inkList.length, 1);
  assert.deepEqual(inkList[0].getColor(), [0, 0, 0]);
  assert.equal(inkList[0].getSubject(), 'Drawn signature');
});

test('(c) drive, review F1: a phone JPEG tagged EXIF Orientation 6 is placed upright and its EXIF never reaches the PDF', { skip: CI_NO_ELECTRON, timeout: 120_000 }, async () => {
  const SCRIPT = 'tests/drive/sign-merge-exif.jsonl';
  await header(SCRIPT, 6, 0);
  // 300x100 as stored, left third black; Orientation 6 = shown rotated 90° clockwise: 100x300 with the top third black.
  const W = 300, H = 100, pix = new mupdf.Pixmap(mupdf.ColorSpace.DeviceRGB, [0, 0, W, H], false);
  pix.clear(255);
  const px = pix.getPixels();
  for (let y = 0; y < H; y++) for (let x = 0; x < 100; x++) { const i = (y * W + x) * 3; px[i] = px[i + 1] = px[i + 2] = 0; }
  const jpeg = Buffer.from(pix.asJPEG(90, false));
  // APP1 Exif, big-endian TIFF, one IFD entry: Orientation (0x0112) SHORT 1 = 6.
  const body = Buffer.concat([Buffer.from('Exif\0\0', 'latin1'), Buffer.from([0x4d, 0x4d, 0, 0x2a, 0, 0, 0, 8, 0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0, 0, 0, 0, 0])]);
  const phone = Buffer.concat([jpeg.subarray(0, 2), Buffer.from([0xff, 0xe1, (body.length + 2) >> 8, (body.length + 2) & 0xff]), body, jpeg.subarray(2)]);
  const { code, report, dir } = await drive(SCRIPT, out => writeFile(join(out, 'phone.jpg'), phone));
  const steps = report.steps;
  assert.equal(steps.length, 6, report.error ?? '');
  assert.equal(red(steps).length, 0, JSON.stringify(red(steps).map(brief)));
  assert.equal(code, 0);

  const [x0, y0, x1, y1] = steps[4].state.fill.signatures[0].rect;
  near(x1 - x0, 20, 'upright: 20 pt wide', 0.05);
  near(y1 - y0, 60, 'upright: 60 pt tall', 0.05);
  const saved = await readFile(join(dir, 'exif.pdf'));
  assert.ok(!saved.includes('Exif'), 'no EXIF block in the saved PDF');
  const stamp = mupdf.Document.openDocument(saved, 'application/pdf').loadPage(0).getAnnotations().find(annot => annot.getType() === 'Stamp');
  const [sx0, sy0, sx1, sy1] = stamp.getRect().map(Math.round);
  const page = mupdf.Document.openDocument(saved, 'application/pdf').loadPage(0).toPixmap(mupdf.Matrix.identity, mupdf.ColorSpace.DeviceRGB, false);
  const pw = page.getWidth(), pp = page.getPixels();
  const dark = (ya, yb) => { let n = 0, t = 0; for (let y = ya; y < yb; y++) for (let x = sx0 + 2; x < sx1 - 2; x++) { t++; if (pp[(y * pw + x) * 3] < 128) n++; } return n / t; };
  assert.ok(dark(sy0 + 2, sy0 + 18) > 0.9, `the top third is the black end: ${dark(sy0 + 2, sy0 + 18)}`);
  assert.ok(dark(sy1 - 18, sy1 - 2) < 0.1, `the bottom third is white: ${dark(sy1 - 18, sy1 - 2)}`);
});
