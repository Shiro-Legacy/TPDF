// SPDX-License-Identifier: AGPL-3.0-or-later
// Read-only PDF width declarations and pinned AFM evidence; no glyph/font loading.
import {readFile,writeFile} from 'node:fs/promises';
import {resolve,join,dirname} from 'node:path';
import {pathToFileURL} from 'node:url';
import {createHash} from 'node:crypto';
const [moduleArg,fixturesArg,sourceArg,reportArg]=process.argv.slice(2);
if(!moduleArg||!fixturesArg||!sourceArg||!reportArg)throw Error('usage: node font-metrics-inventory.mjs mupdf.js fixtures pinned-source report.json');
const modulePath=resolve(moduleArg),root=resolve(fixturesArg),source=resolve(sourceArg);
const pkg=JSON.parse(await readFile(join(dirname(modulePath),'..','package.json'),'utf8'));if(pkg.name!=='mupdf'||pkg.version!=='1.28.1')throw Error('pinned MuPDF1.28.1 required');
const mupdf=await import(pathToFileURL(modulePath).href);
const previous=JSON.parse(await readFile(new URL('../reference/simple-font-inventory.json',import.meta.url),'utf8'));
const fixtures=JSON.parse(await readFile(new URL('../reference/fixture-inventory.json',import.meta.url),'utf8'));
const sha=bytes=>createHash('sha256').update(bytes).digest('hex');
const pageKeys=new Set(previous.pageResources.flatMap(p=>p.fonts.map(f=>p.file+'|'+f.id)));
const selected=previous.fonts.filter(f=>pageKeys.has(f.file+'|'+f.id)&&['Type1','TrueType'].includes(f.subtype));
const diagnostics=[],errors=[],fonts=[];let activeFile=null;
mupdf.setLog({warning:message=>diagnostics.push({file:activeFile,level:'warning',message}),error:message=>diagnostics.push({file:activeFile,level:'error',message})});
const numeric=obj=>obj.isNumber()?obj.asNumber():null;
for(const file of [...new Set(selected.map(f=>f.file))]){
 activeFile=file;const bytes=await readFile(join(root,file));const fixture=fixtures.documents.find(f=>f.file===file);if(sha(bytes)!==fixture.sha256)throw Error('fixture changed');
 const doc=mupdf.Document.openDocument(bytes,'application/pdf').asPDF();
 try{doc.disableJS();for(const prior of selected.filter(f=>f.file===file)){
  const [number,generation]=prior.id.split(':').map(Number);if(generation!==0)throw Error('unexpected generation');const font=doc.newIndirect(number);const widths=font.get('Widths');const descriptor=font.get('FontDescriptor');const missing=descriptor.isDictionary()?descriptor.get('MissingWidth'):doc.newNull();
  const values=widths.isArray()?Array.from({length:widths.length},(_,i)=>numeric(widths.get(i))):null;
  const raw=widths.isArray()?Array.from({length:widths.length},(_,i)=>widths.get(i).toString(true,true)):null;
  const first=numeric(font.get('FirstChar')),last=numeric(font.get('LastChar'));
  fonts.push({file,id:prior.id,subtype:prior.subtype,baseFont:prior.baseFont,encoding:prior.encoding,hasToUnicode:prior.toUnicode!==null,embeddedPrograms:prior.embeddedPrograms,flags:prior.flags,firstChar:first,lastChar:last,widthsReference:widths.toString(true,true),widthsKind:widths.isNull()?'absent':widths.isArray()?'array':'other',widthCount:values?.length??0,widths:values,widthLiterals:raw,exactDeclaredCount:values?Number.isInteger(first)&&Number.isInteger(last)&&values.length===last-first+1:null,missingWidthKind:missing.isNull()?'absent':missing.isNumber()?'number':'other',missingWidth:numeric(missing),missingWidthLiteral:missing.toString(true,true)});
 }}catch(e){errors.push({file,error:String(e)});}finally{doc.destroy();}
}
const encodingTables=JSON.parse(await readFile(new URL('../reference/encoding-table-inventory.json',import.meta.url),'utf8')).tables;
const afm=[];for(const [baseFont,filename] of [['Helvetica','NimbusSans-Regular'],['Times-Roman','NimbusRoman-Regular']]){
 const path=`resources/fonts/urw/input/${filename}.afm`;const bytes=await readFile(join(source,path));const text=bytes.toString('utf8');const entries=[];
 for(const line of text.split(/\r?\n/)){const m=/^C (-?\d+) ; WX (-?[\d.]+) ; N ([^ ;]+) ;/.exec(line);if(m)entries.push({code:Number(m[1]),width:Number(m[2]),name:m[3]});}
 const map=new Map(entries.map(e=>[e.name,e.width]));if(map.size!==entries.length)throw Error('duplicate AFM name');
 afm.push({baseFont,path,sha256:sha(bytes),glyphs:entries.length,encodingCoverage:Object.fromEntries(Object.entries(encodingTables).map(([name,rows])=>[name,{namedSlots:rows.filter(r=>r.glyph!==null).length,undefinedSlots:rows.filter(r=>r.glyph===null).length,missingGlyphNames:[...new Set(rows.filter(r=>r.glyph!==null&&!map.has(r.glyph)).map(r=>r.glyph))].sort()}])),copyright:text.split(/\r?\n/).filter(l=>l.startsWith('Comment Copyright')||l.startsWith('Notice ')),examples:['space','A','B','i','fi','bullet','Euro','.notdef'].map(name=>({name,width:map.get(name)??null})),entries});
}
const explicit=fonts.filter(f=>f.widthsKind==='array');const absent=fonts.filter(f=>f.widthsKind==='absent');const values=explicit.flatMap(f=>f.widths);
const sourceSha256={};for(const file of ['source/pdf/pdf-font.c','source/pdf/pdf-metrics.c','source/fitz/noto.c','source/fitz/font-table.h','resources/fonts/urw/NimbusSans-Regular.cff','resources/fonts/urw/NimbusRoman-Regular.cff'])sourceSha256[file]=sha(await readFile(join(source,file)));
const report={reference:'MuPDF1.28.1 read-only page-bound simple-font declarations; pinned raw AFM measurement without font loading',sourceSha256,pageBoundFonts:fonts.length,explicitWidthFonts:explicit.length,widthEntries:values.length,absentWidthFonts:absent.length,absentWidthsBaseFonts:Object.fromEntries([...new Set(absent.map(f=>f.baseFont))].map(n=>[n,absent.filter(f=>f.baseFont===n).length])),explicitMissingWidths:fonts.filter(f=>f.missingWidthKind==='number').length,nonIntegerWidths:values.filter(v=>v!==null&&!Number.isInteger(v)).length,zeroWidths:values.filter(v=>v===0).length,negativeWidths:values.filter(v=>v<0).length,invalidWidths:values.filter(v=>v===null||!Number.isFinite(v)).length,minimumWidth:Math.min(...values),maximumWidth:Math.max(...values),allCountsExact:explicit.every(f=>f.exactDeclaredCount),fonts,afm,diagnostics,errors,limitations:['Only 53 simple-font identities in inherited page resources; one appearance/Form-only font excluded.','No embedded font program, native font substitution, glyph selection, metrics lookup or actual text execution performed.','AFM advances require an independent native glyph-name/CFF comparison before runtime tables are accepted.']};
await writeFile(reportArg,JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify({...report,fonts:undefined,afm:afm.map(({entries,...row})=>row)},null,2));if(errors.length)process.exitCode=1;
