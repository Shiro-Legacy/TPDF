import mupdf from 'mupdf';
import { writeFile } from 'node:fs/promises';
import { createEngine } from '../../build/renderer/engine.js';

export async function writeSample(path) {
  const doc = new mupdf.PDFDocument();
  const regular = doc.addSimpleFont(new mupdf.Font('Helvetica'));
  const bold = doc.addSimpleFont(new mupdf.Font('Helvetica-Bold'));
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
  const page = doc.addPage([0, 0, 612, 792], 0, { Font: { F1: regular, F2: bold } }, lines.join('\n'));
  doc.insertPage(-1, page);
  const initial = doc.saveToBuffer('incremental=no');
  const source = new Uint8Array(initial.asUint8Array()).buffer;
  initial.destroy(); doc.destroy();
  const engine = createEngine(mupdf), session = engine.createSession();
  let id = 0;
  const send = request => {
    const { reply } = engine.dispatch({ id: ++id, ...request }, session);
    if (!reply.ok) throw new Error(reply.error);
    return reply;
  };
  send({ op: 'open', pdf: source, name: 'Welcome to Tandem.pdf' });
  send({ op: 'field-add', page: 0, kind: 'text', name: 'your_name', label: 'Your name', rect: [44, 459, 568, 491], maxLen: 100 });
  await writeFile(path, new Uint8Array(send({ op: 'save' }).bytes));
  send({ op: 'close' });
}
