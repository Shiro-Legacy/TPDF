// SPDX-License-Identifier: AGPL-3.0-or-later
// Independent MuPDF consumer of files written by the Rust engine.
import {readFile,writeFile} from 'node:fs/promises';
import {pathToFileURL} from 'node:url';
import {createHash} from 'node:crypto';
import assert from 'node:assert/strict';
const [appRoot,evidenceDir]=process.argv.slice(2);
const mupdf=await import(pathToFileURL(`${appRoot}/node_modules/mupdf/dist/mupdf.js`).href);
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
async function inspect(file){
  const bytes=await readFile(file);const doc=mupdf.Document.openDocument(bytes,'application/pdf').asPDF();
  assert(doc);doc.disableJS();const pages=[];
  try{
    for(let page=0;page<doc.countPages();page++){
      const item=doc.loadPage(page);const text=item.toStructuredText('preserve-whitespace');
      const pixmap=item.toPixmap(mupdf.Matrix.identity,mupdf.ColorSpace.DeviceRGB,false,true);
      pages.push({text:text.asText(),bounds:item.getBounds(),width:pixmap.getWidth(),height:pixmap.getHeight(),components:pixmap.getNumberOfComponents(),pixels:Uint8Array.from(pixmap.getPixels())});
      pixmap.destroy();text.destroy();item.destroy();
    }
    assert.equal(doc.wasRepaired(),false,`${file} required repair`);
    return {sha256:hash(bytes),pages};
  }finally{doc.destroy();}
}
const first=await inspect(`${evidenceDir}/workflow-save-1.pdf`);
const second=await inspect(`${evidenceDir}/workflow-save-2.pdf`);
assert.match(first.pages[0].text,/Hello world/);assert.doesNotMatch(first.pages[0].text,/PDF4All/);
assert.match(second.pages[0].text,/Edited again/);assert.doesNotMatch(second.pages[0].text,/Hello world/);
const original=await inspect(`${appRoot}/fixtures/p0b/edit-transformed.pdf`);
const edited=await inspect(`${evidenceDir}/transformed-edited.pdf`);
assert.match(edited.pages[0].text,/BETA/);assert.match(edited.pages[0].text,/NEIGHBOR/);assert.doesNotMatch(edited.pages[0].text,/ALPHA/);
const a=original.pages[0],b=edited.pages[0];assert.deepEqual(a.bounds,b.bounds);assert.equal(a.pixels.length,b.pixels.length);
let inside=0,outside=0;
for(let y=0;y<a.height;y++)for(let x=0;x<a.width;x++){
  const offset=(y*a.width+x)*a.components;
  const different=a.pixels.slice(offset,offset+a.components).some((v,c)=>v!==b.pixels[offset+c]);
  if(different){if(x>=69&&x<=154&&y>=471&&y<=496)inside++;else outside++;}
}
assert(inside>0,'edit did not change native render');assert.equal(outside,0,'unrelated pixels changed');
const report={oracle:'MuPDF 1.28.1 WASM (test only)',first:{sha256:first.sha256,text:first.pages[0].text},second:{sha256:second.sha256,text:second.pages[0].text},transformed:{sha256:edited.sha256,text:b.text,changedInside:inside,changedOutside:outside},allReopenWithoutRepair:true};
await writeFile(`${evidenceDir}/independent-oracle.json`,JSON.stringify(report,null,2));console.log(JSON.stringify(report));
