// SPDX-License-Identifier: AGPL-3.0-or-later
// Read-only metadata inventory; does not load fonts or interpret glyphs.
import {readFile,writeFile} from 'node:fs/promises';
import {resolve,join,dirname} from 'node:path';
import {pathToFileURL} from 'node:url';
import {createHash} from 'node:crypto';
const [moduleArg,fixturesArg,reportArg]=process.argv.slice(2);
if(!moduleArg||!fixturesArg||!reportArg)throw Error('usage: node simple-font-inventory.mjs mupdf.js fixtures report.json');
const modulePath=resolve(moduleArg),root=resolve(fixturesArg);
const pkg=JSON.parse(await readFile(join(dirname(modulePath),'..','package.json'),'utf8'));
if(pkg.name!=='mupdf'||pkg.version!=='1.28.1')throw Error('pinned MuPDF1.28.1 required');
const mupdf=await import(pathToFileURL(modulePath).href);
const previous=JSON.parse(await readFile(new URL('../reference/cmap-inventory.json',import.meta.url),'utf8'));
const fixtures=JSON.parse(await readFile(new URL('../reference/fixture-inventory.json',import.meta.url),'utf8'));
const sha=data=>createHash('sha256').update(data).digest('hex');
const diagnostics=[];let activeFile=null;
mupdf.setLog({warning:message=>diagnostics.push({file:activeFile,level:'warning',message}),error:message=>diagnostics.push({file:activeFile,level:'error',message})});
const name=o=>o.isName()?o.asName():null;
const identity=o=>{const m=/^(\d+)\s+(\d+)\s+R$/.exec(o.toString(true,true));return m?`${m[1]}:${m[2]}`:null;};
const fonts=[],pageResources=[],errors=[];
for(const fixture of fixtures.documents){
 if(fixture.passwordRequired)continue;
 activeFile=fixture.file;const bytes=await readFile(join(root,fixture.file));if(sha(bytes)!==fixture.sha256)throw Error('fixture hash changed');
 const doc=mupdf.Document.openDocument(bytes,'application/pdf').asPDF();
 try{
  doc.disableJS();
  for(const prior of previous.fonts.filter(f=>f.file===fixture.file)){
   const match=/^(\d+):(\d+)$/.exec(prior.id);if(!match||match[2]!=='0')throw Error(`unhandled fontidentity ${prior.id}`);
   const font=doc.newIndirect(Number(match[1]));
   const enc=font.get('Encoding'),desc=font.get('FontDescriptor'),diff=enc.isDictionary()?enc.get('Differences'):doc.newNull();
   const row={...prior,isDescendant:['CIDFontType0','CIDFontType2'].includes(prior.subtype),encodingKind:enc.isNull()?'absent':enc.isName()?'name':enc.isDictionary()?'dictionary':'other',encodingName:name(enc),baseEncoding:enc.isDictionary()?name(enc.get('BaseEncoding')):null,differences:diff.isArray()?Array.from({length:diff.length},(_,i)=>diff.get(i).toString(true,true)):null,fontDescriptor:identity(desc),flags:desc.isDictionary()&&desc.get('Flags').isNumber()?desc.get('Flags').asNumber():null,embeddedPrograms:['FontFile','FontFile2','FontFile3'].filter(key=>desc.isDictionary()&&desc.get(key).isStream()),firstChar:font.get('FirstChar').isNumber()?font.get('FirstChar').asNumber():null,lastChar:font.get('LastChar').isNumber()?font.get('LastChar').asNumber():null};
   if(row.encoding!== (enc.isNull()?null:enc.toString(true,true)))throw Error('prior encoding differs');
   fonts.push(row);
  }
  for(let i=0;i<doc.countPages();i++){
   const page=doc.findPage(i),resources=page.getInheritable('Resources'),resource=resources.isDictionary()?resources.get('Font'):doc.newNull(),entries=[];
   if(resource.isDictionary())resource.forEach((font,key)=>entries.push({name:key,id:identity(font),subtype:name(font.get('Subtype')),baseFont:name(font.get('BaseFont')),encoding:font.get('Encoding').toString(true,true),hasToUnicode:!font.get('ToUnicode').isNull()}));
   pageResources.push({file:fixture.file,page:i+1,fonts:entries});
  }
 }catch(e){errors.push({file:fixture.file,error:String(e)});}finally{doc.destroy();}
}
const top=fonts.filter(f=>!f.isDescendant),missing=top.filter(f=>f.toUnicode===null);
const counts=(rows,key)=>Object.fromEntries([...new Set(rows.map(r=>r[key]??'(none)'))].sort().map(k=>[k,rows.filter(r=>(r[key]??'(none)')===k).length]));
const report={reference:'Pinned MuPDF1.28.1 WASM dictionary reads, no font loading',documents:fixtures.documents.filter(f=>!f.passwordRequired).length,fontDictionaries:fonts.length,topLevelFonts:top.length,topLevelWithoutToUnicode:missing.length,withoutToUnicodeEncodings:counts(missing,'encoding'),topLevelEncodingKinds:counts(top,'encodingKind'),topLevelSubtypes:counts(top,'subtype'),topLevelBaseFonts:counts(top,'baseFont'),fontsWithDifferences:top.filter(f=>f.differences!==null).length,missingExplicitEncoding:top.filter(f=>f.encodingKind==='absent').map(f=>({file:f.file,id:f.id,toUnicode:f.toUnicode})),topLevelEmbedded:top.filter(f=>f.embeddedPrograms.length).length,withoutToUnicodeEmbedded:missing.filter(f=>f.embeddedPrograms.length).length,pages:pageResources.length,pageFontResourceEntries:pageResources.reduce((n,r)=>n+r.fonts.length,0),pagesWithoutFontResources:pageResources.filter(r=>!r.fonts.length).length,diagnostics,errors,limitations:['Resource registration is not proof of executed text or glyph use.','Inherited page resources are included; prior dictionary contexts also record encountered Form/resources.','Font dictionaries are taken from the hash-verified existing inventory; no new malformed-xref enumeration.','Embedded-program presence does not inspect font cmap, built-in encoding, glyph outlines or metrics.'],fonts,pageResources};
await writeFile(reportArg,JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify({...report,fonts:undefined,pageResources:undefined},null,2));if(errors.length)process.exitCode=1;
