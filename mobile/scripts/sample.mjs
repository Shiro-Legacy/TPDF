// The bundled samples, made with the Rust engine the app ships (no MuPDF): welcome.pdf, a page in base-14
// fonts with one text field, and sample-scan.png, that page rendered at 2x for the scanner's "Try a sample".
import { readFileSync } from 'node:fs';
import { writeFile } from 'node:fs/promises';
import { deflateSync } from 'node:zlib';
// The adapter scripts/build.mjs has just written to dist/engine (build.mjs imports this module after it).
import { createRustEngine } from '../dist/engine/engine-rust.js';

const pkg = new URL('../../engine/web/pkg/', import.meta.url);
const wasm = await import(new URL('tandem_rust_workflow.js', pkg).href);
wasm.initSync({ module: readFileSync(new URL('tandem_rust_workflow_bg.wasm', pkg)) });

/** The welcome page before its field: a minimal PDF with the text and rules the mobile tour points at. */
function welcomePage() {
  const lines = [];
  const text = (value, y, size = 12, font = 'F1', color = '.15 .24 .20') => {
    lines.push(`BT /${font} ${size} Tf ${color} rg 1 0 0 1 44 ${y} Tm (${value.replace(/[()\\]/g, '\\$&')}) Tj ET`);
  };
  text('TANDEM PDF', 743, 12, 'F2');
  text('Make this page yours.', 690, 30, 'F2');
  text('A small playground for your everyday paperwork.', 656, 13);
  lines.push('.87 .90 .85 RG 44 627 m 568 627 l S');
  text('01   MAKE YOUR MARK', 590, 11, 'F2');
  text('Try highlighting this sentence.', 559, 17);
  text('Choose Highlight below, then drag across the words.', 533, 11);
  text('02   LEAVE A THOUGHT', 478, 11, 'F2');
  text('Choose Note and tap anywhere to leave yourself a reminder.', 450, 12);
  text('Find your notes again in Document options.', 429, 11);
  text('03   FILL SOMETHING IN', 373, 11, 'F2');
  text('Your name', 345, 12);
  text('Tap Fill to complete this field.', 280, 10);
  text('04   MAKE IT OFFICIAL', 236, 11, 'F2');
  text('Tap Sign, draw your signature, and place it in the space below.', 210, 12);
  lines.push('.95 .95 .91 rg 44 104 524 85 re f');
  lines.push('.75 .80 .73 RG 58 126 m 554 126 l S');
  text('Your edits save on this device. Share exports a PDF copy.', 62, 10);
  const content = lines.join('\n');
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R /F2 5 0 R >> >> /Contents 6 0 R >>',
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>',
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>',
    `<< /Length ${Buffer.byteLength(content, 'latin1')} >>\nstream\n${content}\nendstream`,
  ];
  let body = '%PDF-1.7\n';
  const offsets = objects.map((object, i) => { const at = Buffer.byteLength(body, 'latin1'); body += `${i + 1} 0 obj\n${object}\nendobj\n`; return at; });
  const xref = Buffer.byteLength(body, 'latin1');
  body += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.map(at => `${String(at).padStart(10, '0')} 00000 n \n`).join('')}`;
  body += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  const bytes = Buffer.from(body, 'latin1');
  return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
}

/** An 8-bit RGB PNG from opaque RGBA pixels. */
function png(rgba, width, height) {
  const raw = Buffer.alloc((width * 3 + 1) * height);
  for (let y = 0; y < height; y++) {
    const row = y * (width * 3 + 1);
    for (let x = 0; x < width; x++) {
      const from = (y * width + x) * 4, to = row + 1 + x * 3;
      raw[to] = rgba[from]; raw[to + 1] = rgba[from + 1]; raw[to + 2] = rgba[from + 2];
    }
  }
  const table = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
  const crc = bytes => { let c = 0xffffffff; for (const byte of bytes) c = table[(c ^ byte) & 0xff] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };
  const chunk = (type, data) => {
    const out = Buffer.alloc(12 + data.length);
    out.writeUInt32BE(data.length, 0); out.write(type, 4, 'latin1'); data.copy(out, 8);
    out.writeUInt32BE(crc(out.subarray(4, 8 + data.length)), 8 + data.length);
    return out;
  };
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0); header.writeUInt32BE(height, 4); header[8] = 8; header[9] = 2;
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk('IHDR', header), chunk('IDAT', deflateSync(raw, { level: 9 })), chunk('IEND', Buffer.alloc(0))]);
}

export async function writeSamples(pdfPath, scanPath) {
  const engine = createRustEngine(wasm), session = engine.createSession();
  let id = 0;
  const send = request => {
    const { reply } = engine.dispatch({ id: ++id, ...request }, session);
    if (!reply.ok) throw new Error(`${request.op}: ${reply.error}`);
    return reply;
  };
  send({ op: 'open', pdf: welcomePage(), name: 'Welcome to Tandem.pdf' });
  send({ op: 'field-add', page: 0, kind: 'text', name: 'your_name', label: 'Your name', rect: [44, 459, 568, 491], maxLen: 100 });
  const saved = send({ op: 'save' }).bytes;
  send({ op: 'close' });
  send({ op: 'open', pdf: saved.slice(0), name: 'Welcome to Tandem.pdf' });
  const render = send({ op: 'render', page: 0, scale: 2 });
  send({ op: 'close' });
  await writeFile(pdfPath, new Uint8Array(saved));
  await writeFile(scanPath, png(new Uint8Array(render.pixels), render.pixelWidth, render.pixelHeight));
}
