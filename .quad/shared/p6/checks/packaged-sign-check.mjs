// Drives the MCP server inside the packaged app through draw-signature: sign-draw, sign-place, save; then checks
// the saved Ink with mupdf. usage: node packaged-sign-check.mjs <server.mjs> <outdir> [image]
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { readFileSync } from 'node:fs';
import * as mupdf from 'mupdf';
const [server, out, image] = process.argv.slice(2);
const child = spawn(process.execPath, [server, `--out=${out}`], { stdio: ['pipe', 'pipe', 'inherit'] });
const waiting = new Map(); let id = 0;
createInterface({ input: child.stdout }).on('line', l => { const m = JSON.parse(l); waiting.get(m.id)?.(m); });
const rpc = (method, params) => new Promise(r => { waiting.set(++id, r); child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n'); });
const call = async (name, args = {}) => { const r = (await rpc('tools/call', { name, arguments: args })).result; return { isError: r.isError, state: JSON.parse(r.content[0].text).state }; };
await rpc('initialize', { protocolVersion: '2025-06-18' });
await call('tandem_new', { size: 'letter' });
const before = await call('tandem_state');
console.log('agent session starts with no saved signature:', before.state?.fill?.signature?.saved === false);
const strokes = [Array.from({ length: 200 }, (_, i) => [i * 2, 40 + 30 * Math.sin(i / 9)]), [[150, 10], [150, 10]]];
const drawn = await call('tandem_do', { cmd: 'sign-draw', contents: { strokes } });
console.log('sign-draw', JSON.stringify(drawn.state?.fill?.signature), 'error', JSON.stringify(drawn.state?.error));
const placed = await call('tandem_do', { cmd: 'sign-place', contents: { page: 0, x: 300, y: 650 } });
console.log('sign-place', JSON.stringify(placed.state?.fill?.signatures), 'error', JSON.stringify(placed.state?.error));
// sign-merge: an image replaces the drawing as the saved signature (argv[4] = an absolute PNG/JPEG path) and places as a Stamp.
if (image) {
  const used = await call('tandem_do', { cmd: 'sign-draw', images: [image], contents: { image: true } });
  console.log('sign-draw image', JSON.stringify(used.state?.fill?.signature), 'error', JSON.stringify(used.state?.error));
  const put = await call('tandem_do', { cmd: 'sign-place', contents: { page: 0, x: 300, y: 400 } });
  console.log('sign-place image', JSON.stringify(put.state?.fill?.signatures), 'error', JSON.stringify(put.state?.error));
}
const saved = await call('tandem_save', { path: 'signed.pdf' }); console.log('save', JSON.stringify(saved.state?.save));
child.stdin.end(); await new Promise(r => child.on('close', r));
const doc = mupdf.Document.openDocument(readFileSync(`${out}/signed.pdf`), 'application/pdf');
for (const a of doc.loadPage(0).getAnnotations()) console.log('annot', a.getType(), 'color', JSON.stringify(a.getColor()), 'subject', a.getSubject(), 'T', String(a.getObject().get('T')), 'contents', JSON.stringify(a.getContents()), 'strokes', a.getType() === 'Ink' ? a.getInkList().length : '-');
