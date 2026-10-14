// SPDX-License-Identifier: AGPL-3.0-or-later
import {readFile,writeFile} from 'node:fs/promises';
import {pathToFileURL} from 'node:url';
import {createHash} from 'node:crypto';
import assert from 'node:assert/strict';
const [appRoot,directory]=process.argv.slice(2);
const mupdf=await import(pathToFileURL(`${appRoot}/node_modules/mupdf/dist/mupdf.js`).href);
const manifest=JSON.parse(await readFile(`${directory}/edits.json`));const results=[];
for(const row of manifest.rows.filter(r=>r.accepted)){
  const originalBytes=await readFile(row.input),editedBytes=await readFile(row.output);
  assert.equal(createHash('sha256').update(originalBytes).digest('hex'),row.sourceSha256);
  assert.equal(createHash('sha256').update(editedBytes).digest('hex'),row.outputSha256);
  const original=mupdf.Document.openDocument(originalBytes,'application/pdf').asPDF();
  const edited=mupdf.Document.openDocument(editedBytes,'application/pdf').asPDF();
  original.disableJS();edited.disableJS();assert.equal(edited.countPages(),original.countPages());
  let inside=0,outside=0,editText='';
  for(let page=0;page<original.countPages();page++){
    const a=original.loadPage(page),b=edited.loadPage(page);assert.deepEqual(a.getBounds(),b.getBounds());
    const ap=a.toPixmap(mupdf.Matrix.identity,mupdf.ColorSpace.DeviceRGB,false,true);
    const bp=b.toPixmap(mupdf.Matrix.identity,mupdf.ColorSpace.DeviceRGB,false,true);
    const at=a.toStructuredText('preserve-whitespace'),bt=b.toStructuredText('preserve-whitespace');
    if(page===row.page)editText=bt.asText();else assert.equal(at.asText(),bt.asText(),'unaffected page text changed');
    const x0=Math.min(row.oldBounds[0],row.newBounds[0])-3,y0=Math.min(row.oldBounds[1],row.newBounds[1])-3;
    const x1=Math.max(row.oldBounds[2],row.newBounds[2])+3,y1=Math.max(row.oldBounds[3],row.newBounds[3])+3;
    const ac=ap.getPixels(),bc=bp.getPixels();assert.equal(ac.length,bc.length);
    const components=ap.getNumberOfComponents(),width=ap.getWidth();
    for(let index=0;index<ac.length;index+=components){
      let differs=false;for(let c=0;c<components;c++)if(ac[index+c]!==bc[index+c])differs=true;
      if(differs){const x=(index/components)%width,y=Math.floor(index/components/width);
        if(page===row.page&&x>=x0&&x<=x1&&y>=y0&&y<=y1)inside++;else outside++;}
    }
    at.destroy();bt.destroy();ap.destroy();bp.destroy();a.destroy();b.destroy();
  }
  assert.equal(original.wasRepaired(),false);assert.equal(edited.wasRepaired(),false);
  assert.equal(outside,0,`${row.input}: pixels changed outside edit`);
  assert(inside>0,`${row.input}: no visible edit`);
  assert(editText.includes(row.newText),`${row.input}: new text missing from independent extraction: ${editText}`);
  results.push({input:row.input,outputSha256:row.outputSha256,changedInside:inside,changedOutside:outside,newText:row.newText,repaired:false});
  original.destroy();edited.destroy();
}
await writeFile(`${directory}/independent-oracle.json`,JSON.stringify({oracle:'MuPDF 1.28.1 WASM',documents:results.length,results},null,2));
console.log(JSON.stringify({verified:results.length,allOutsidePixelsUnchanged:true,allReopenWithoutRepair:true}));
