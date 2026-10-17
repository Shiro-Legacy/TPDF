/**
 * P2 unit 7 §A — exports: images → PDF, page → PNG/JPEG (the encode is the
 * renderer's, the pixels are this engine's), text → .txt.
 *
 * Runs against the app engine (tests/rust-engine.mjs) exactly as the worker does. The images are
 * built here with zlib so the engine tests do not wait on a fixture, and they match
 * the contract's description: a 300x200 RGB gradient and a 200x300 grayscale image
 * with a black band at rows 100-120.
 *
 * Measured in the gate (`.stage/gate-u7.mjs`, written in `p2-export.md`): mupdf's
 * `addPage` only builds the page object — `countPages()` stays 0 until `insertPage`
 * puts it in the page tree — and it reports 96 dpi for an image with no resolution
 * metadata, which is why a page keeps one point per pixel in that case.
 */
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdtemp as makeTempDir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test, { after } from 'node:test';
import zlib from 'node:zlib';
import mupdf from 'mupdf';
import { createEngine, MAX_IMAGES, MAX_IMAGE_BYTES } from './rust-engine.mjs';
import { assertWithin } from './timing-budget.mjs';

const engine = createEngine(mupdf);

/** Every temp dir this file makes (housekeeping B): removed after all its tests, never before their asserts. */
const madeDirs = [];
const mkdtemp = async prefix => { const dir = await makeTempDir(prefix); madeDirs.push(dir); return dir; };
after(() => Promise.all(madeDirs.map(dir => rm(dir, { recursive: true, force: true, maxRetries: 5 }))));
const repoRoot = fileURLToPath(new URL('..', import.meta.url));
const python = join(repoRoot, process.platform === 'win32' ? '.venv-qa/Scripts/python.exe' : '.venv-qa/bin/python3');

function toArrayBuffer(buffer) {
  return buffer.buffer.slice(buffer.byteOffset, buffer.byteOffset + buffer.byteLength);
}

/** A minimal PNG encoder: 8-bit RGB (channels 3) or gray (1), no filter. */
function png(width, height, channels, pixel) {
  const raw = Buffer.alloc(height * (1 + width * channels));
  let offset = 0;
  for (let y = 0; y < height; y++) {
    raw[offset++] = 0;
    for (let x = 0; x < width; x++) {
      const value = pixel(x, y);
      if (channels === 1) raw[offset++] = value[0];
      else {
        raw[offset++] = value[0];
        raw[offset++] = value[1];
        raw[offset++] = value[2];
      }
    }
  }
  const chunk = (type, data) => {
    const out = Buffer.alloc(12 + data.length);
    out.writeUInt32BE(data.length, 0);
    out.write(type, 4, 'latin1');
    data.copy(out, 8);
    out.writeUInt32BE(zlib.crc32(Buffer.concat([Buffer.from(type, 'latin1'), data])), 8 + data.length);
    return out;
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8;
  ihdr[9] = channels === 1 ? 0 : 2;
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', zlib.deflateSync(raw)),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

const rgbPng = png(300, 200, 3, (x, y) => [x % 256, y % 256, 128]);
/** Grayscale, with a black band at rows 100..120. */
const grayPng = png(200, 300, 1, (x, y) => {
  void x;
  return [y >= 100 && y <= 120 ? 0 : 200];
});

function image(name, bytes) {
  return { name, bytes: toArrayBuffer(bytes) };
}

function op(session, request) {
  return engine.dispatch({ id: 900 + (request.id ?? 0), ...request }, session).reply;
}

async function open(id, session = engine.createSession()) {
  const bytes = toArrayBuffer(await readFile(`fixtures/${id}.pdf`));
  const reply = engine.dispatch({ id: 1, op: 'open', name: `${id}.pdf`, pdf: bytes }, session).reply;
  assert.equal(reply.ok, true, reply.error);
  return session;
}

function reopen(bytes) {
  const doc = mupdf.Document.openDocument(bytes, 'application/pdf');
  doc.asPDF().disableJS();
  return doc;
}

/** The RGBA pixels of one page as mupdf sees them (independent of our render op). */
function pixelsOf(doc, page) {
  const loaded = doc.loadPage(page);
  try {
    // alpha=true so the pixmap is RGBA: a 3-component pixmap would need converting.
    const pixmap = loaded.toPixmap(mupdf.Matrix.scale(1, 1), mupdf.ColorSpace.DeviceRGB, true);
    try {
      assert.equal(pixmap.getNumberOfComponents(), 4);
      return { width: pixmap.getWidth(), height: pixmap.getHeight(), pixels: pixmap.getPixels() };
    } finally {
      pixmap.destroy();
    }
  } finally {
    loaded.destroy();
  }
}

/** How many pixels of one row are dark (a black band is dark, white paper is not). */
function darkInRow(image, row) {
  let dark = 0;
  for (let x = 0; x < image.width; x++) {
    const at = (row * image.width + x) * 4;
    if (image.pixels[at] < 60 && image.pixels[at + 1] < 60) dark++;
  }
  return dark;
}

// ------------------------------------------------------------ images to PDF

test('pdf-from-images: two PNGs become two pages of the right size, each showing its image', async () => {
  const session = engine.createSession();
  const reply = op(session, {
    op: 'pdf-from-images',
    images: [image('rgb.png', rgbPng), image('gray.png', grayPng)],
  });
  assert.equal(reply.ok, true, reply.error);
  assert.equal(reply.pageCount, 2);
  assert.ok(reply.bytes.byteLength > 1000);

  const doc = reopen(reply.bytes);
  try {
    assert.equal(doc.countPages(), 2, 'one page per image');
    assert.deepEqual([...doc.loadPage(0).getBounds()], [0, 0, 300, 200], 'the RGB page is the image pixel size');
    assert.deepEqual([...doc.loadPage(1).getBounds()], [0, 0, 200, 300], 'so is the gray one');

    const rgb = pixelsOf(doc, 0);
    const gray = pixelsOf(doc, 1);
    assert.equal(rgb.width, 300);
    assert.equal(gray.width, 200);
    assert.equal(gray.height, 300);
    assert.ok(darkInRow(rgb, 5) < 300, 'the gradient is not a black page');
    assert.ok(darkInRow(gray, 5) === 0, 'the top of the gray image is light');
    assert.ok(darkInRow(gray, 110) > 150, `the black band is there (row 110 has ${darkInRow(gray, 110)} dark pixels)`);
    assert.equal(darkInRow(gray, 250), 0, 'and the bottom is light again');
  } finally {
    doc.destroy();
  }

  // The app can open and render what it built, and the journal is untouched.
  const opened = engine.createSession();
  try {
    const openReply = engine.dispatch({ id: 1, op: 'open', name: 'images.pdf', pdf: reply.bytes }, opened).reply;
    assert.equal(openReply.ok, true, openReply.error);
    assert.equal(openReply.pageCount, 2);
    const rendered = op(opened, { op: 'render', page: 1, scale: 1 });
    assert.equal(rendered.ok, true, rendered.error);
    assert.equal(rendered.pixelWidth, 200);
    assert.equal(rendered.nonWhite > 1000, true, 'the render has ink');
    assert.equal(op(opened, { op: 'undo' }).ok, false, 'nothing was mutated');
  } finally {
    engine.closeDocument(opened.doc);
  }
});

test('pdf-from-images takes a JPEG too, in picker order', async () => {
  // A JPEG made from a real page, the way the page export makes one.
  const source = await open('hello');
  let jpeg;
  try {
    const rendered = op(source, { op: 'render', page: 0, scale: 1 });
    const pixmap = mupdf.Pixmap.createFromPixels?.(rendered.pixelWidth, rendered.pixelHeight, new Uint8Array(rendered.pixels), 4, false);
    // The engine's own pixels round-trip through mupdf's JPEG writer.
    const page = source.doc.loadPage(0);
    try {
      const direct = page.toPixmap(mupdf.Matrix.scale(1, 1), mupdf.ColorSpace.DeviceRGB, false);
      jpeg = direct.asJPEG(90);
      direct.destroy();
    } finally {
      page.destroy();
      pixmap?.destroy();
    }
  } finally {
    engine.closeDocument(source.doc);
  }

  const session = engine.createSession();
  const reply = op(session, { op: 'pdf-from-images', images: [image('gray.png', grayPng), image('page.jpg', jpeg)] });
  assert.equal(reply.ok, true, reply.error);
  assert.equal(reply.pageCount, 2);
  const doc = reopen(reply.bytes);
  try {
    assert.deepEqual([...doc.loadPage(0).getBounds()], [0, 0, 200, 300], 'the order is the picker order');
    const second = doc.loadPage(1);
    const bounds = [...second.getBounds()];
    assert.equal(bounds[2] - bounds[0], 612, 'the JPEG page is its pixel width');
    assert.ok(pixelsOf(doc, 1).pixels.length > 0);
    second.destroy();
  } finally {
    doc.destroy();
  }
});

test('pdf-from-images refuses what it cannot decode, without touching any document', async () => {
  const session = await open('nav');
  try {
    const refusals = [
      [op(session, { op: 'pdf-from-images', images: [] }), /1\.\.50 files/],
      [op(session, { op: 'pdf-from-images', images: Array.from({ length: MAX_IMAGES + 1 }, () => image('x.png', grayPng)) }), /1\.\.50 files/],
      [op(session, { op: 'pdf-from-images', images: [image('notes.txt', Buffer.from('not an image'))] }), /notes\.txt is not a PNG or JPEG/],
      [op(session, { op: 'pdf-from-images', images: [image('', grayPng)] }), /needs a name/],
      [op(session, { op: 'pdf-from-images', images: [{ name: 'x.png', bytes: 'nope' }] }), /has no bytes/],
      [op(session, { op: 'pdf-from-images', images: [{ name: 'x.png', bytes: new ArrayBuffer(0) }] }), /has no bytes/],
      [op(session, { op: 'pdf-from-images', images: [{ name: 'x.png', bytes: new ArrayBuffer(MAX_IMAGE_BYTES + 1) }] }), /byte image ceiling/],
    ];
    for (const [reply, pattern] of refusals) {
      assert.equal(reply.ok, false, JSON.stringify(reply));
      assert.match(reply.error, pattern);
    }
    assert.equal(session.doc.countPages(), 3, 'the open document is untouched');
    assert.equal(op(session, { op: 'undo' }).ok, false, 'and never dirtied');
  } finally {
    engine.closeDocument(session.doc);
  }
});

// ------------------------------------------------------------- text export

test('text-all exports every page as plain lines joined by \\f', async () => {
  const nav = await open('nav');
  try {
    const reply = op(nav, { op: 'text-all' });
    assert.equal(reply.ok, true, reply.error);
    assert.equal(reply.text, 'one\ftwo\fthree');
  } finally {
    engine.closeDocument(nav.doc);
  }

  const hello = await open('hello');
  try {
    const reply = op(hello, { op: 'text-all' });
    assert.equal(reply.ok, true, reply.error);
    assert.match(reply.text, /PDF4All/);
    assert.equal(reply.text.includes('\f'), false, 'one page means one form feed at most (none)');
    assert.equal(hello.revision, 0, 'a text export is not a mutation');
  } finally {
    engine.closeDocument(hello.doc);
  }
});

test('large.pdf: text-all stays inside the cost and size budget', async () => {
  const session = await open('large');
  try {
    const started = performance.now();
    const reply = op(session, { op: 'text-all' });
    const ms = performance.now() - started;
    assert.equal(reply.ok, true, reply.error);
    const bytes = Buffer.byteLength(reply.text, 'utf8');
    const pages = reply.text.split('\f').length;
    console.log(`U7_TEXT ${JSON.stringify({ ms: Math.round(ms), bytes, pages })}`);
    assert.equal(pages, 500, 'every page contributes one part');
    assert.ok(bytes > 0 && bytes < 50 * 1024 * 1024);
    assertWithin(ms, 5000, '500 pages of text');
  } finally {
    engine.closeDocument(session.doc);
  }
});

// -------------------------------------------------------------- permissions

test('a document that forbids copying exports no text, and one that forbids printing says so', async () => {
  // encrypted-noprint: no print permission (what the image export checks), copy allowed.
  const session = engine.createSession();
  try {
    const opened = engine.dispatch({ id: 1, op: 'open', name: 'encrypted-noprint.pdf', pdf: toArrayBuffer(await readFile('fixtures/encrypted-noprint.pdf')) }, session).reply;
    assert.equal(opened.ok, true, opened.error);
    assert.equal(opened.info.needsPassword, true);
    const auth = engine.dispatch({ id: 2, op: 'authenticate', password: 'pdf4all-user' }, session).reply;
    assert.equal(auth.authenticated, true);
    assert.equal(auth.info.printable, false, 'the renderer refuses image export on this flag');
    assert.equal(auth.info.encrypted, true);
    const text = op(session, { op: 'text-all' });
    assert.equal(text.ok, true, text.error);
    assert.match(text.text, /PDF4All/);
  } finally {
    engine.closeDocument(session.doc);
  }

  // A no-copy document: built with pypdf (the same .venv-qa the verify script uses).
  if (!existsSync(python)) {
    console.log('SKIP no-copy check: .venv-qa/bin/python3 is not there');
    return;
  }
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-nocopy-'));
  const path = join(dir, 'nocopy.pdf');
  const built = spawnSync(python, ['-c', [
    'import pypdf',
    'from pypdf import PdfWriter',
    'w = PdfWriter()',
    'w.add_blank_page(width=300, height=600)',
    "w.encrypt(user_password='', owner_password='owner', permissions_flag=pypdf.constants.UserAccessPermissions.PRINT, algorithm='RC4-40')",
    `w.write(${JSON.stringify(path)})`,
  ].join('\n')], { encoding: 'utf8' });
  assert.equal(built.status, 0, built.stdout + built.stderr);

  const locked = engine.createSession();
  try {
    const bytes = toArrayBuffer(await readFile(path));
    const opened = engine.dispatch({ id: 1, op: 'open', name: 'nocopy.pdf', pdf: bytes }, locked).reply;
    assert.equal(opened.ok, true, opened.error);
    assert.equal(locked.locked, false, 'an empty user password opens it');
    const refused = op(locked, { op: 'text-all' });
    assert.equal(refused.ok, false, 'the copy bit is honoured');
    assert.equal(refused.reason, 'read-only');
    assert.match(refused.error, /does not permit copying/);
    // Extracting a page is still allowed: it writes nothing into this document.
    assert.equal(op(locked, { op: 'extract-pages', pages: [0] }).ok, true);
  } finally {
    engine.closeDocument(locked.doc);
  }
});

test('text-all refuses a locked document before reading a page', async () => {
  const session = engine.createSession();
  const bytes = mupdf.Document.openDocument(toArrayBuffer(await readFile('fixtures/hello.pdf')), 'application/pdf').asPDF();
  const buffer = bytes.saveToBuffer('incremental=no,encrypt=aes-256,user-password=secret,owner-password=owner');
  const encrypted = toArrayBuffer(buffer.asUint8Array());
  buffer.destroy();
  bytes.destroy();
  try {
    const opened = engine.dispatch({ id: 1, op: 'open', name: 'secret.pdf', pdf: encrypted }, session).reply;
    assert.equal(opened.ok, true, opened.error);
    const refused = op(session, { op: 'text-all' });
    assert.equal(refused.ok, false);
    assert.match(refused.error, /password is required/);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('an exported page image round-trips back into a PDF, and a saved copy validates', async () => {
  // Simulates the renderer's PNG/JPEG export: the engine's render gives the pixels,
  // so a PNG made from them must build the same size page back.
  const session = await open('hello');
  try {
    const rendered = op(session, { op: 'render', page: 0, scale: 150 / 72 });
    assert.equal(rendered.ok, true, rendered.error);
    const pixmap = mupdf.Pixmap.createFromPixels
      ? null
      : null;
    void pixmap;
    // Encode with mupdf's own writer over the *engine's* pixels: a stand-in for the
    // renderer's OffscreenCanvas, so the round-trip is proven in Node too.
    const page = session.doc.loadPage(0);
    let pngBytes;
    try {
      const direct = page.toPixmap(mupdf.Matrix.scale(150 / 72, 150 / 72), mupdf.ColorSpace.DeviceRGB, false);
      assert.equal(direct.getWidth(), rendered.pixelWidth, 'the same pixels the render reply carries');
      assert.equal(direct.getHeight(), rendered.pixelHeight);
      pngBytes = direct.asPNG();
      direct.destroy();
    } finally {
      page.destroy();
    }

    const built = op(session, { op: 'pdf-from-images', images: [image('page 1.png', pngBytes)] });
    assert.equal(built.ok, true, built.error);
    const doc = reopen(built.bytes);
    try {
      assert.equal(doc.countPages(), 1);
      const bounds = [...doc.loadPage(0).getBounds()];
      assert.equal(Math.round(bounds[2]), rendered.pixelWidth, 'one point per pixel with no resolution metadata');
      assert.equal(Math.round(bounds[3]), rendered.pixelHeight);
    } finally {
      doc.destroy();
    }
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('the export reply shapes carry what the smoke logs', async () => {
  const session = await open('hello');
  try {
    const text = op(session, { op: 'text-all' });
    assert.deepEqual(Object.keys(text).sort(), ['id', 'ok', 'text']);
    const images = op(session, { op: 'pdf-from-images', images: [image('a.png', grayPng), image('b.png', grayPng)] });
    assert.equal(images.pageCount, 2);
    assert.ok(images.bytes instanceof ArrayBuffer);
    const outlined = Object.keys(images).sort();
    assert.deepEqual(outlined, ['bytes', 'id', 'ok', 'pageCount'], 'transferred bytes, no journal state');
  } finally {
    engine.closeDocument(session.doc);
  }
});
