// SPDX-License-Identifier: AGPL-3.0-or-later
// Read-only evidence: native PDF dictionaries/streams plus bounded sfnt headers.
import {readFile,writeFile} from 'node:fs/promises';
import {resolve,join,dirname} from 'node:path';
import {pathToFileURL} from 'node:url';
import {createHash} from 'node:crypto';
const [moduleArg,fixturesArg,reportArg]=process.argv.slice(2);
if(!moduleArg||!fixturesArg||!reportArg)throw Error('usage: node embedded-outline-inventory.mjs mupdf.js fixtures report.json');
const modulePath=resolve(moduleArg),root=resolve(fixturesArg);
const pkg=JSON.parse(await readFile(join(dirname(modulePath),'..','package.json'),'utf8'));if(pkg.name!=='mupdf'||pkg.version!=='1.28.1')throw Error('pinned MuPDF1.28.1 required');
const mupdf=await import(pathToFileURL(modulePath).href);
const metricsBytes=await readFile(new URL('../reference/font-metrics-inventory.json',import.meta.url));
const fixtureBytes=await readFile(new URL('../reference/fixture-inventory.json',import.meta.url));
const selected=JSON.parse(metricsBytes).fonts.filter(f=>f.widthsKind==='array');if(selected.length!==5)throw Error('expected five explicit-width fonts');
const fixtures=JSON.parse(fixtureBytes).documents;
const sha=b=>createHash('sha256').update(b).digest('hex');
const identity=o=>o.toString(true,true);
const diagnostics=[];let active=null;mupdf.setLog({warning:m=>diagnostics.push({file:active,level:'warning',message:m}),error:m=>diagnostics.push({file:active,level:'error',message:m})});
function sfnt(data){
 if(data.length<12||data.length>32*1024*1024)throw Error('sfnt length limit');
 const version=data.subarray(0,4).toString('hex'),count=data.readUInt16BE(4);if(count>256||12+16*count>data.length)throw Error('sfnt directory bounds');
 const tables=[];for(let i=0;i<count;i++){const p=12+i*16,tag=data.toString('ascii',p,p+4),offset=data.readUInt32BE(p+8),length=data.readUInt32BE(p+12);if(offset>data.length||length>data.length-offset)throw Error('sfnt table bounds');tables.push({tag,offset,length,checksum:data.readUInt32BE(p+4).toString(16).padStart(8,'0')});}
 if(new Set(tables.map(t=>t.tag)).size!==tables.length)throw Error('duplicate sfnt table');
 const get=tag=>{const t=tables.find(t=>t.tag===tag);return t?data.subarray(t.offset,t.offset+t.length):null;};
 const head=get('head'),maxp=get('maxp'),loca=get('loca'),glyf=get('glyf'),cmap=get('cmap'),post=get('post');
 const result={sfntVersionHex:version,tables,outlineFormat:glyf&&loca?'TrueType glyf/loca':get('CFF ')?'CFF':get('CFF2')?'CFF2':'other',unitsPerEm:head&&head.length>=54?head.readUInt16BE(18):null,glyphCount:maxp&&maxp.length>=6?maxp.readUInt16BE(4):null,indexToLocFormat:head&&head.length>=54?head.readInt16BE(50):null,postVersionHex:post&&post.length>=4?post.subarray(0,4).toString('hex'):null,variableTables:tables.filter(t=>['fvar','gvar','avar','HVAR','MVAR'].includes(t.tag)).map(t=>t.tag),instructionTables:tables.filter(t=>['fpgm','prep','cvt '].includes(t.tag)).map(t=>t.tag),cmaps:[]};
 if(cmap){if(cmap.length<4)throw Error('cmap header');const n=cmap.readUInt16BE(2);if(n>256||4+n*8>cmap.length)throw Error('cmap directory');for(let i=0;i<n;i++){const p=4+i*8,off=cmap.readUInt32BE(p+4);if(off+2>cmap.length)throw Error('cmap offset');result.cmaps.push({platform:cmap.readUInt16BE(p),encoding:cmap.readUInt16BE(p+2),format:cmap.readUInt16BE(off),offset:off});}}
 if(glyf&&loca){const format=result.indexToLocFormat,n=result.glyphCount;if(n===null||![0,1].includes(format))throw Error('loca format');const bytes=format===0?2:4;if((n+1)*bytes>loca.length)throw Error('loca length');let prior=0,empty=0,simple=0,composite=0,maxContours=0;for(let i=0;i<n;i++){const start=format===0?loca.readUInt16BE(i*2)*2:loca.readUInt32BE(i*4),end=format===0?loca.readUInt16BE((i+1)*2)*2:loca.readUInt32BE((i+1)*4);if(start<prior||end<start||end>glyf.length)throw Error('loca range');prior=end;if(start===end){empty++;continue;}if(end-start<10)throw Error('glyf header');const contours=glyf.readInt16BE(start);if(contours<0)composite++;else{simple++;maxContours=Math.max(maxContours,contours);}}result.glyphHeaders={empty,simple,composite,maxSimpleContours:maxContours};}
 return result;
}
const fonts=[];
for(const row of selected){active=row.file;const bytes=await readFile(join(root,row.file)),fixture=fixtures.find(f=>f.file===row.file);if(!fixture||sha(bytes)!==fixture.sha256)throw Error('fixture hash changed');const doc=mupdf.Document.openDocument(bytes,'application/pdf').asPDF();try{doc.disableJS();const [number,generation]=row.id.split(':').map(Number);if(generation!==0)throw Error('generation');const font=doc.newIndirect(number),desc=font.get('FontDescriptor'),programs=[];for(const key of ['FontFile','FontFile2','FontFile3']){const stream=desc.get(key);if(stream.isStream()){const buffer=stream.readStream();let data;try{data=Buffer.from(buffer.asUint8Array());}finally{buffer.destroy();}programs.push({key,object:identity(stream),subtype:stream.get('Subtype').toString(true,true),decodedBytes:data.length,sha256:sha(data),...sfnt(data)});}}fonts.push({file:row.file,fixtureSha256:fixture.sha256,font:row.id,subtype:row.subtype,baseFont:row.baseFont,encoding:row.encoding,flags:row.flags,widthEntries:row.widthCount,descriptor:identity(desc),fontDictionary:font.resolve().toString(true,true),descriptorDictionary:desc.resolve().toString(true,true),programs});}finally{doc.destroy();}}
const report={reference:'MuPDF1.28.1 WASM dictionary and decoded-stream reads; standalone bounded sfnt table/header inventory',sourceReports:{'font-metrics-inventory.json':sha(metricsBytes),'fixture-inventory.json':sha(fixtureBytes)},fontCount:fonts.length,programCount:fonts.reduce((n,f)=>n+f.programs.length,0),diagnostics,fonts,limitations:['Dictionary/header evidence only: no font charmap selection, glyph identity, quadratic/composite outline expansion or hinting performed.','Checks table/loca boundaries for inventory; not a production font validator and does not verify table checksums.','Registered page font presence is not proof of executed glyph use.']};
await writeFile(reportArg,JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify({fonts:fonts.map(f=>({file:f.file,font:f.font,programs:f.programs.map(p=>({format:p.outlineFormat,glyphs:p.glyphCount,headers:p.glyphHeaders,cmaps:p.cmaps,variable:p.variableTables}))})),diagnostics},null,2));
if(diagnostics.length)process.exitCode=1;
