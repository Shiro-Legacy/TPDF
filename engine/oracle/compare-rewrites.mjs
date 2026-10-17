// SPDX-License-Identifier: AGPL-3.0-or-later
// Independent native reopen + WASM raw/decoded streams and raster comparison.
// Existing fixtures are read only and SHA256 checked before every rewrite.
import { readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
const [moduleArg, fixtureArg, rustArg, oracleArg, reportArg, inventoryArg] = process.argv.slice(2);
if (!reportArg) throw new Error('usage: node compare-rewrites.mjs mupdf.js fixtures rust-binary native-document-oracle report.json [inventory.json]');
const modulePath=resolve(moduleArg), root=resolve(fixtureArg), rust=resolve(rustArg), oracle=resolve(oracleArg);
const meta=JSON.parse(await readFile(join(dirname(modulePath),'..','package.json'),'utf8'));
if (meta.name!=='mupdf'||meta.version!=='1.28.1') throw new Error('requires MuPDF 1.28.1');
const mupdf=await import(pathToFileURL(modulePath).href);
const inventory=JSON.parse(await readFile(inventoryArg ? resolve(inventoryArg) : new URL('../reference/fixture-inventory.json',import.meta.url),'utf8'));
const sha=bytes=>createHash('sha256').update(bytes).digest('hex');
const run=(bin,args,input)=>{
  const result=spawnSync(bin,args,{input,timeout:30000,maxBuffer:129*1024*1024});
  if(result.error||result.signal) throw result.error??new Error(`process signal ${result.signal}`);
  return result;
};
const native=bytes=>{
  const result=run(oracle,[],bytes);
  if(result.status!==0) throw new Error(`native oracle: ${result.stderr}`);
  const value=JSON.parse(result.stdout);
  if(value.repaired) throw new Error('native MuPDF repaired PDF');
  return value;
};
const raster=(doc,index)=>{
  const page=doc.loadPage(index);
  try {
    const bounds=page.getBounds();
    if(bounds.some(x=>!Number.isFinite(x))||Math.abs((bounds[2]-bounds[0])*(bounds[3]-bounds[1]))>40_000_000) throw new Error('raster bounds too large');
    const pix=page.toPixmap(mupdf.Matrix.scale(0.5,0.5),mupdf.ColorSpace.DeviceRGB,false,true);
    try { return {width:pix.getWidth(),height:pix.getHeight(),stride:pix.getStride(),sha256:sha(pix.getPixels())}; }
    finally {pix.destroy();}
  } finally {page.destroy();}
};
let matched=0,unsupported=0,failures=0,matchedPages=0,matchedStreams=0,rawBytes=0,decodedBytes=0;
const rows=[];
for(const entry of inventory.documents){
  const bytes=await readFile(join(root,entry.file));
  if(sha(bytes)!==entry.sha256) throw new Error(`changed fixture ${entry.file}`);
  const row={file:entry.file,inputSha256:entry.sha256};
  try{
    const written=run(rust,['--rewrite'],bytes);
    if(written.status!==0){
      const reason=written.stderr.toString().trim();
      if(written.status===2&&reason.includes('unsupported')){
        if(written.stdout.length) throw new Error('partial stdout on refusal');
        unsupported++;rows.push({...row,status:'unsupported',reason});continue;
      }
      throw new Error(`Rust writer: ${reason}`);
    }
    const output=written.stdout;
    const originalGeometry=native(bytes), resultGeometry=native(output);
    if(JSON.stringify(originalGeometry.pages)!==JSON.stringify(resultGeometry.pages)) throw new Error('native page geometry changed');
    const first=run(rust,['--document'],bytes), second=run(rust,['--document'],output);
    if(first.status!==0||second.status!==0) throw new Error(`Rust reopen: ${second.stderr}`);
    const info=JSON.parse(first.stdout), actual=JSON.parse(second.stdout);
    if(JSON.stringify(info)!==JSON.stringify(actual)) throw new Error('Rust inventory changed');
    const again=run(rust,['--rewrite'],output);
    if(again.status!==0||!again.stdout.equals(output)) throw new Error('rewrite not canonical/idempotent');
    const before=mupdf.Document.openDocument(bytes,'application/pdf').asPDF();
    const after=mupdf.Document.openDocument(output,'application/pdf').asPDF();
    const streams=[],pages=[];
    try{
      before.disableJS();after.disableJS();
      for(const stream of info.streams){
        const oldObject=before.newIndirect(stream.object),newObject=after.newIndirect(stream.object);
        try{
          const result={object:stream.object,generation:stream.generation};
          for(const [method,key] of [['readRawStream','raw'],['readStream','decoded']]){
            const a=oldObject[method](),b=newObject[method]();
            try{
              const left=Buffer.from(a.asUint8Array()),right=Buffer.from(b.asUint8Array());
              if(!left.equals(right)) throw new Error(`${key} stream ${stream.object} changed`);
              result[key]={bytes:left.length,sha256:sha(left)};
            }finally{a.destroy();b.destroy();}
          }
          streams.push(result);
        }finally{oldObject.destroy();newObject.destroy();}
      }
      for(let index=0;index<info.pages.length;index++){
        const a=raster(before,index),b=raster(after,index);
        if(JSON.stringify(a)!==JSON.stringify(b)) throw new Error(`rendered page ${index+1} changed`);
        pages.push({page:index+1,...a});
      }
    }finally{before.destroy();after.destroy();}
    matched++;matchedPages+=pages.length;matchedStreams+=streams.length;
    for(const stream of streams){rawBytes+=stream.raw.bytes;decodedBytes+=stream.decoded.bytes;}
    rows.push({...row,status:'matched',outputSha256:sha(output),outputBytes:output.length,pages,streams});
  }catch(error){failures++;rows.push({...row,status:'failure',reason:String(error)});console.error(entry.file,String(error));}
}
const result={reference:'MuPDF 1.28.1 native + WASM',rustSha256:sha(await readFile(rust)),oracleSha256:sha(await readFile(oracle)),rasterDpi:36,matched,unsupported,failures,matchedPages,matchedStreams,rawBytes,decodedBytes,passed:failures===0&&matched>=30,results:rows};
await writeFile(reportArg,JSON.stringify(result,null,2)+'\n');
console.log(JSON.stringify({...result,results:undefined},null,2));
process.exitCode=result.passed?0:1;
