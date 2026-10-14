// SPDX-License-Identifier: AGPL-3.0-or-later
// Read-only ToUnicode/font workload inventory, not a production CMap parser.
// Interpretation references pinned MuPDF pdf-cmap-parse.c (Artifex 2004-2021).
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {dirname,join,resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';

const [moduleArg,fixtureArg,reportArg,saveArg,rustArg]=process.argv.slice(2);
if(!moduleArg||!fixtureArg||!reportArg)throw Error('usage: node cmap-inventory.mjs mupdf.js fixtures report.json [save-dir] [rust-binary]');
const modulePath=resolve(moduleArg),root=resolve(fixtureArg),rust=resolve(rustArg??'target/debug/pdf-inspect');
const meta=JSON.parse(await readFile(join(dirname(modulePath),'..','package.json'),'utf8'));
if(meta.name!=='mupdf'||meta.version!=='1.28.1')throw Error('requires pinned MuPDF1.28.1');
const mupdf=await import(pathToFileURL(modulePath).href);
const nativeDiagnostics=[];let activeFile=null,activeObject=null;
mupdf.setLog({warning:message=>nativeDiagnostics.push({level:'warning',file:activeFile,object:activeObject,message}),error:message=>nativeDiagnostics.push({level:'error',file:activeFile,object:activeObject,message})});
const fixtures=JSON.parse(await readFile(new URL('../reference/fixture-inventory.json',import.meta.url),'utf8'));
const baseline=JSON.parse(await readFile(new URL('../reference/stream-comparison-r4b.json',import.meta.url),'utf8'));
const hash=data=>createHash('sha256').update(data).digest('hex');
const refId=object=>{const match=/^(\d+)\s+(\d+)\s+R$/.exec(object.toString(true,true));return match?`${match[1]}:${match[2]}`:null;};
const name=object=>object.isName()?object.asName():null;
function utf16(hex){
 const data=Buffer.from(hex,'hex');let scalars=0,nonBmp=false,invalid=data.length===0||data.length%2!==0;
 for(let i=0;i+1<data.length;i+=2){const value=data.readUInt16BE(i);scalars++;if(value>=0xd800&&value<=0xdbff){if(i+3<data.length){const low=data.readUInt16BE(i+2);if(low>=0xdc00&&low<=0xdfff){nonBmp=true;i+=2;continue;}}invalid=true;}else if(value>=0xdc00&&value<=0xdfff)invalid=true;}
 return {units:Math.floor(data.length/2),scalars,nonBmp,invalid};
}
function inspect(data){
 const run=spawnSync(rust,['--tokens','-'],{input:data,timeout:10000,maxBuffer:16*1024*1024});
 if(run.error||run.signal)throw run.error??Error(`token signal ${run.signal}`);
 if(run.status!==0)return{tokenized:false,error:run.stderr.toString().trim()};
 const tokens=run.stdout.toString().trim().split('\n').filter(Boolean).map(line=>{const[kind,value]=line.split('\t');return{kind,value,word:kind==='keyword'||kind==='name'?Buffer.from(value,'hex').toString('latin1'):null};});
 const result={tokenized:true,tokens:tokens.length,codespaces:[],bfchar:0,bfrange:0,sequentialRanges:0,sequentialRangeOverflow:0,sequentialRangeSurrogateCrossings:0,arrayRanges:0,mappedSourceEntries:0,sourceByteWidths:[],destinationMaxUnits:0,multiScalarDestinations:0,nonBmpDestinations:0,invalidUtf16Destinations:0,invalidDestinationExamples:[],usecmapNames:[],otherMappingOperators:[],countMismatches:[],syntaxNotes:[]};
 const widths=new Set(),ranges=[];
 const destination=token=>{if(token?.kind!=='string'){result.syntaxNotes.push('non-string Unicode destination');return;}const text=utf16(token.value);result.destinationMaxUnits=Math.max(result.destinationMaxUnits,text.units);if(text.scalars>1)result.multiScalarDestinations++;if(text.nonBmp)result.nonBmpDestinations++;if(text.invalid){result.invalidUtf16Destinations++;result.invalidDestinationExamples.push({source:ranges.at(-1)??null,destinationHex:token.value,decodedBytes:token.value.length/2});}};
 const source=(lo,hi)=>{if(lo?.kind!=='string'||hi?.kind!=='string'){result.syntaxNotes.push('non-string source');return 0;}const width=lo.value.length/2;widths.add(width);if(width!==hi.value.length/2||width<1||width>4){result.syntaxNotes.push('invalid source byte width');return 0;}const a=BigInt(`0x${lo.value}`),b=BigInt(`0x${hi.value}`);if(a>b){result.syntaxNotes.push('reversed source range');return 0;}ranges.push({width,lo:Number(a),hi:Number(b)});return Number(b-a+1n);};
 for(let i=0;i<tokens.length;i++){
  const token=tokens[i];if(token.kind!=='keyword')continue;
  if(token.word==='usecmap'){result.usecmapNames.push(tokens[i-1]?.word??null);continue;}
  if(['begincidchar','begincidrange','beginnotdefchar','beginnotdefrange'].includes(token.word))result.otherMappingOperators.push(token.word);
  if(!['begincodespacerange','beginbfchar','beginbfrange'].includes(token.word))continue;
  const end=token.word.replace('begin','end');const declared=tokens[i-1]?.kind==='integer'?Number(tokens[i-1].value):null;let actual=0;
  while(++i<tokens.length&&tokens[i].word!==end){
   const lo=tokens[i],hi=token.word==='beginbfchar'?lo:tokens[++i];actual++;
   if(token.word==='begincodespacerange'){
    if(lo?.kind==='string'&&hi?.kind==='string')result.codespaces.push({low:lo.value,high:hi.value,bytes:lo.value.length/2});else result.syntaxNotes.push('invalid codespace entry');
   }else if(token.word==='beginbfchar'){
    result.bfchar++;result.mappedSourceEntries+=source(lo,lo);destination(tokens[++i]);
   }else{
    result.bfrange++;result.mappedSourceEntries+=source(lo,hi);const target=tokens[++i];
    if(target?.kind==='array-start'){result.arrayRanges++;let arrayCount=0;while(++i<tokens.length&&tokens[i].kind!=='array-end'){destination(tokens[i]);arrayCount++;}if(lo?.kind==='string'&&hi?.kind==='string'&&arrayCount!==Number(BigInt(`0x${hi.value}`)-BigInt(`0x${lo.value}`)+1n))result.syntaxNotes.push('bfrange array count mismatch');}
    else{result.sequentialRanges++;destination(target);const range=ranges.at(-1);if(target?.kind==='string'&&target.value.length===4&&range){const start=Number.parseInt(target.value,16),end=start+range.hi-range.lo;if(end>65535)result.sequentialRangeOverflow++;if(start<=0xdfff&&end>=0xd800)result.sequentialRangeSurrogateCrossings++;}}
   }
   if(i>=tokens.length){result.syntaxNotes.push(`missing ${end}`);break;}
  }
  if(actual!==declared)result.countMismatches.push({operator:token.word,declared,actual});
 }
 result.sourceByteWidths=[...widths].sort((a,b)=>a-b);
 const ordered=ranges.sort((a,b)=>a.width-b.width||a.lo-b.lo||a.hi-b.hi);let last=null;result.overlappingMappings=0;
 for(const range of ordered){if(last&&last.width===range.width&&range.lo<=last.hi)result.overlappingMappings++;if(!last||last.width!==range.width||range.hi>last.hi)last=range;}
 return result;
}
const fonts=[],cmaps=[],errors=[],documents=[];
if(saveArg)await mkdir(saveArg,{recursive:true});
for(const fixture of fixtures.documents){
 if(fixture.passwordRequired)continue;
 activeFile=fixture.file;activeObject=null;
 const bytes=await readFile(join(root,fixture.file));if(hash(bytes)!==fixture.sha256)throw Error(`fixture changed ${fixture.file}`);
 const doc=mupdf.Document.openDocument(bytes,'application/pdf').asPDF(),found=new Map();
 const remember=(object,context)=>{if(!object.isDictionary())return;const id=refId(object)??context;const previous=found.get(id);if(previous){previous.contexts.add(context);return;}found.set(id,{object,id,contexts:new Set([context])});};
 try{
  doc.disableJS();const objectCount=doc.countObjects();if(objectCount>1_000_000)throw Error('inventory object limit');
  for(let i=1;i<objectCount;i++){
   activeObject=i;const object=doc.newIndirect(i);if(!object.isDictionary())continue;
   if(name(object.get('Type'))==='Font')remember(object,`object:${i}`);
   const resources=object.get('Resources','Font');
   if(resources.isDictionary())resources.forEach((font,key)=>remember(font,`object:${i}.Resources.Font.${key}`));
  }
  for(const{id,object,contexts}of found.values()){
   const encoding=object.get('Encoding'),unicode=object.get('ToUnicode');
   const row={file:fixture.file,id,contexts:[...contexts].sort(),subtype:name(object.get('Subtype')),baseFont:name(object.get('BaseFont')),encoding:encoding.isNull()?null:encoding.toString(true,true),toUnicode:unicode.isNull()?null:unicode.toString(true,true),descendantFonts:object.get('DescendantFonts').isArray()?object.get('DescendantFonts').length:0};
   fonts.push(row);
   if(unicode.isNull())continue;
   if(!unicode.isStream()){row.toUnicodeKind=unicode.isName()?'predefined-name':'other';continue;}
   row.toUnicodeKind='stream';const cmapId=refId(unicode);row.toUnicodeId=cmapId;
   let existing=cmaps.find(cmap=>cmap.file===fixture.file&&cmap.id===cmapId);
   if(existing){existing.fontIds.push(id);continue;}
   const buffer=unicode.readStream();let data;try{data=Buffer.from(buffer.asUint8Array());}finally{buffer.destroy();}
   if(data.length>16*1024*1024)throw Error('inventory CMap byte limit');
   const reference=baseline.results.find(stream=>stream.file===fixture.file&&stream.id===cmapId&&stream.status==='matched');
   if(!reference||reference.sha256!==hash(data))throw Error(`ToUnicode missing or differs from validated baseline ${fixture.file} ${cmapId}`);
   existing={file:fixture.file,id:cmapId,fontIds:[id],bytes:data.length,sha256:hash(data),streamUseCMap:unicode.get('UseCMap').toString(true,true),...inspect(data)};
   if(saveArg){existing.savedFile=`${cmaps.length.toString().padStart(4,'0')}.cmap`;await writeFile(join(saveArg,existing.savedFile),data);}
   cmaps.push(existing);
  }
  documents.push({file:fixture.file,objects:objectCount,fonts:found.size});
 }catch(error){errors.push({file:fixture.file,error:String(error)});}finally{doc.destroy();}
}
const counts=field=>{const out={};for(const row of fonts){const value=row[field]??'(none)';out[value]=(out[value]??0)+1;}return out;};
const report={reference:'MuPDF1.28.1 WASM font dictionaries and decoded ToUnicode bytes',documents:documents.length,fontDictionaries:fonts.length,descendantFontDictionaries:fonts.filter(row=>['CIDFontType0','CIDFontType2'].includes(row.subtype)).length,uniqueCMapPayloads:new Set(cmaps.map(row=>row.sha256)).size,cMapsWithInvalidDestinations:cmaps.filter(row=>row.invalidUtf16Destinations>0).length,fontSubtypes:counts('subtype'),fontEncodings:counts('encoding'),nativeDiagnostics,nativeDiagnosticCount:nativeDiagnostics.length,fontsWithToUnicode:fonts.filter(row=>row.toUnicode!==null).length,fontsWithoutToUnicode:fonts.filter(row=>row.toUnicode===null).length,uniqueToUnicodeStreams:cmaps.length,decodedCMapBytes:cmaps.reduce((sum,row)=>sum+row.bytes,0),tokenizedCMaps:cmaps.filter(row=>row.tokenized).length,bfcharEntries:cmaps.reduce((sum,row)=>sum+(row.bfchar??0),0),bfrangeEntries:cmaps.reduce((sum,row)=>sum+(row.bfrange??0),0),mappedSourceEntries:cmaps.reduce((sum,row)=>sum+(row.mappedSourceEntries??0),0),cmapSourceByteWidths:[...new Set(cmaps.flatMap(row=>row.sourceByteWidths??[]))].sort(),rangeArrayCount:cmaps.reduce((sum,row)=>sum+(row.arrayRanges??0),0),multiScalarDestinations:cmaps.reduce((sum,row)=>sum+(row.multiScalarDestinations??0),0),nonBmpDestinations:cmaps.reduce((sum,row)=>sum+(row.nonBmpDestinations??0),0),invalidUtf16Destinations:cmaps.reduce((sum,row)=>sum+(row.invalidUtf16Destinations??0),0),overlappingMappings:cmaps.reduce((sum,row)=>sum+(row.overlappingMappings??0),0),usecmapCount:cmaps.reduce((sum,row)=>sum+(row.usecmapNames?.length??0),0),errors,limitations:['Inventory includes all Font dictionaries and directly encountered Resources/Font dictionaries, not only executed text fonts.','Font dictionaries with no Type outside encountered resources may be absent.','CMap features are measured by a bounded token scanner, not an accepted production CMap interpreter.','ToUnicode coverage does not imply glyph selection, font metrics, Unicode fallback, text positioning, or extraction support.','Encrypted/password-required fixtures are skipped.'],fixtureDocuments:documents,fonts,cmaps};
await writeFile(reportArg,JSON.stringify(report,null,2)+'\n');
console.log(JSON.stringify({...report,fixtureDocuments:undefined,fonts:undefined,cmaps:undefined,nativeDiagnostics:undefined},null,2));
process.exitCode=errors.length||report.tokenizedCMaps!==cmaps.length?1:0;
