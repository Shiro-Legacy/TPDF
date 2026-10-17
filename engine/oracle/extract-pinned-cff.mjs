// SPDX-License-Identifier: AGPL-3.0-or-later
// Development-only: export the font assets bundled with pinned MuPDF 1.28.1.
// The Rust runtime uses generated immutable outlines and never imports MuPDF.
import fs from 'node:fs';
import crypto from 'node:crypto';
import mupdf from 'mupdf';
const output = process.argv[2];
if (!output) throw new Error('usage: extract-pinned-cff.mjs OUTPUT_DIRECTORY');
const expected = {
  Helvetica: '366720e054067fa49d030f58005c739fc787dcc9727a9327a28767777791d706',
  'Helvetica-Bold': 'd4ea80d1d0e29a1e950808009ca8f96fe07a0670b60b556f6b2cf0705337fa9f',
  'Helvetica-BoldOblique': '9cdfc2d0947339dce76f99bdf9e2d9fe01e472a384cf683007e6a0bb35634623',
};
fs.mkdirSync(output, {recursive:true});
const report=[];
for(const [name,sha256] of Object.entries(expected)){
  const doc=new mupdf.PDFDocument(), font=new mupdf.Font(name);
  const object=doc.addFont(font);
  const bytes=object.get('DescendantFonts').get(0).get('FontDescriptor').get('FontFile3').readStream().asUint8Array();
  if(crypto.createHash('sha256').update(bytes).digest('hex')!==sha256) throw new Error(`font asset mismatch: ${name}`);
  fs.writeFileSync(`${output}/${name}.cff`,bytes);
  report.push({name,bytes:bytes.length,sha256}); font.destroy();doc.destroy();
}
fs.writeFileSync(`${output}/font-assets.json`,JSON.stringify(report,null,2)+'\n');
