// SPDX-License-Identifier: AGPL-3.0-or-later
// Read-only workload inventory, not a production parser or operator validator.
// MuPDF supplies decoded bytes; this scanner only recognizes lexical boundaries
// to count top-level operator words. Inline-image payloads are never searched.
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {dirname,join,resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';

const [moduleArg,fixtureArg,reportArg,saveArg,rustArg]=process.argv.slice(2);
const rust=resolve(rustArg??'target/debug/pdf-inspect');
if(!moduleArg||!fixtureArg||!reportArg)throw Error('usage: node content-inventory.mjs mupdf.js fixtures report.json [save-dir] [rust-binary]');
const modulePath=resolve(moduleArg),root=resolve(fixtureArg);
const meta=JSON.parse(await readFile(join(dirname(modulePath),'..','package.json'),'utf8'));
if(meta.name!=='mupdf'||meta.version!=='1.28.1')throw Error('requires pinned MuPDF 1.28.1');
const mupdf=await import(pathToFileURL(modulePath).href);
const fixtureInventory=JSON.parse(await readFile(new URL('../reference/fixture-inventory.json',import.meta.url),'utf8'));
const streamInventory=JSON.parse(await readFile(new URL('../reference/stream-comparison-r4b.json',import.meta.url),'utf8'));
const known=new Set('w J j M d ri i gs q Q cm m l c v y h re S s f F f* B B* b b* n W W* BT ET Tc Tw Tz TL Tf Tr Ts Td TD Tm T* Tj TJ \' " d0 d1 CS cs SC SCN sc scn G g RG rg K k sh Do MP DP BMC BDC EMC BX EX BI ID EI'.split(' '));
const whitespace=b=>[0,9,10,12,13,32].includes(b);
const delimiter=b=>whitespace(b)||[40,41,60,62,91,93,123,125,47,37].includes(b);
function scan(data){
  const operators={},unknown={},stack=[];let i=0,tokens=0,complete=true,error=null;
  const fail=reason=>{complete=false;error={offset:i,reason};};
  while(i<data.length&&complete){
    const b=data[i];
    if(whitespace(b)){i++;continue;}
    if(b===37){while(i<data.length&&data[i]!==10&&data[i]!==13)i++;continue;}
    if(++tokens>2000000){fail('inventory token limit');break;}
    if(b===47){i++;while(i<data.length&&!delimiter(data[i]))i++;continue;}
    if(b===40){
      i++;let depth=1;
      while(i<data.length&&depth){const next=data[i++];if(next===92){if(i<data.length){const escaped=data[i++];if(escaped===13&&data[i]===10)i++;}}else if(next===40)depth++;else if(next===41)depth--;}
      if(depth)fail('unterminated literal string');continue;
    }
    if(b===60&&data[i+1]!==60){i++;while(i<data.length&&data[i]!==62)i++;if(i===data.length)fail('unterminated hex string');else i++;continue;}
    if(b===91||(b===60&&data[i+1]===60)){stack.push(b===91?'array':'dict');i+=b===91?1:2;if(stack.length>256)fail('inventory nesting limit');continue;}
    if(b===93||(b===62&&data[i+1]===62)){const kind=b===93?'array':'dict';if(stack.pop()!==kind){fail('mismatched container delimiter');break;}i+=b===93?1:2;continue;}
    if(delimiter(b)){fail('unexpected delimiter');break;}
    const start=i;while(i<data.length&&!delimiter(data[i]))i++;
    if(i-start>1024){fail('oversized inventory keyword');break;}
    if(stack.length)continue;
    const word=data.subarray(start,i).toString('latin1');
    if(/^[-+]?(?:\d+\.?\d*|\.\d+)$/.test(word)||['true','false','null'].includes(word))continue;
    operators[word]=(operators[word]??0)+1;
    if(!known.has(word))unknown[word]=(unknown[word]??0)+1;
    if(word==='BI'){complete=false;error={offset:start,reason:'inline image begins; payload and remaining operators intentionally not scanned'};break;}
  }
  if(complete&&stack.length)fail('unclosed operand container');
  return{complete,operators,unknown,error,scannedBytes:i};
}
function refId(object){const text=object.toString(true,true);const match=/^(\d+)\s+(\d+)\s+R$/.exec(text);return match?`${match[1]}:${match[2]}`:null;}
const results=[],pages=[],classificationErrors=[],lexerCrossChecks={matched:0,failures:[]};
let selectedBytes=0;
if(saveArg)await mkdir(saveArg,{recursive:true});
for(const expected of fixtureInventory.documents){
  if(expected.passwordRequired)continue;
  const allStreams=streamInventory.results.filter(row=>row.file===expected.file&&row.status==='matched');
  const bytes=await readFile(join(root,expected.file));
  if(createHash('sha256').update(bytes).digest('hex')!==expected.sha256)throw Error(`fixture changed ${expected.file}`);
  const doc=mupdf.Document.openDocument(bytes,'application/pdf').asPDF();
  const selected=new Map();
  const select=(stream,role,page=null)=>{const id=refId(stream);if(!id){classificationErrors.push({file:expected.file,reason:'content stream without indirect identity',role,page});return;}const value=selected.get(id)??{roles:new Set(),pages:new Set()};value.roles.add(role);if(page!==null)value.pages.add(page);selected.set(id,value);};
  try{
    doc.disableJS();
    for(let pageNumber=0;pageNumber<doc.countPages();pageNumber++){
      const page=doc.findPage(pageNumber),contents=page.get('Contents'),ids=[];
      const add=object=>{if(!object.isStream()){classificationErrors.push({file:expected.file,page:pageNumber,reason:'non-stream page Contents member'});return;}select(object,'page',pageNumber);ids.push(refId(object));};
      if(contents.isArray())for(let i=0;i<contents.length;i++)add(contents.get(i));else if(!contents.isNull())add(contents);
      pages.push({file:expected.file,page:pageNumber,contents:ids});
    }
    for(const stream of allStreams){const number=Number(stream.id.split(':')[0]);const object=doc.newIndirect(number);if(object.get('Subtype').asName()==='Form')select(object,'form');}
    for(const[id,selection]of selected){
      const baseline=allStreams.find(row=>row.id===id);if(!baseline){classificationErrors.push({file:expected.file,id,reason:'selected content missing from validated stream baseline'});continue;}
      const object=doc.newIndirect(Number(id.split(':')[0])),buffer=object.readStream();let data;
      try{data=Buffer.from(buffer.asUint8Array());}finally{buffer.destroy();}
      const hash=createHash('sha256').update(data).digest('hex');if(hash!==baseline.sha256)throw Error(`decoded stream changed ${expected.file} ${id}`);
      selectedBytes+=data.length;
      const row={file:expected.file,id,roles:[...selection.roles].sort(),pages:[...selection.pages].sort((a,b)=>a-b),bytes:data.length,sha256:hash,...scan(data)};
      const prefix=row.complete?data:data.subarray(0,row.error.offset);
      const tokenRun=spawnSync(rust,['--tokens','-'],{input:prefix,timeout:10000,maxBuffer:8*1024*1024});
      const tokenCounts={};let depth=0;
      if(tokenRun.status===0){
        for(const line of tokenRun.stdout.toString().trim().split('\n')){const[kind,value]=line.split('\t');if(kind==='array-start'||kind==='dict-start')depth++;else if(kind==='array-end'||kind==='dict-end')depth--;else if(kind==='keyword'&&depth===0){const word=Buffer.from(value,'hex').toString('latin1');if(!['true','false','null'].includes(word))tokenCounts[word]=(tokenCounts[word]??0)+1;}}
      }
      const scannerCounts={...row.operators};if(!row.complete&&row.error.reason.startsWith('inline image'))delete scannerCounts.BI;
      const normalized=counts=>JSON.stringify(Object.entries(counts).sort(([a],[b])=>a.localeCompare(b)));
      if(tokenRun.status!==0||normalized(tokenCounts)!==normalized(scannerCounts))lexerCrossChecks.failures.push({file:expected.file,id,reason:tokenRun.status!==0?tokenRun.stderr.toString().trim():'operator counts differ from accepted Rust lexer'});else lexerCrossChecks.matched++;
      if(saveArg){row.savedFile=`${results.length.toString().padStart(4,'0')}.bin`;await writeFile(join(saveArg,row.savedFile),data);}
      results.push(row);
    }
  }finally{doc.destroy();}
}
const operators={},unknown={},streamsPerOperator={};
for(const row of results)for(const[operator,count]of Object.entries(row.operators)){operators[operator]=(operators[operator]??0)+count;streamsPerOperator[operator]=(streamsPerOperator[operator]??0)+1;}
for(const row of results)for(const[word,count]of Object.entries(row.unknown))unknown[word]=(unknown[word]??0)+count;
const summary={reference:'MuPDF 1.28.1 WASM decoded bytes',baselineStreams:streamInventory.matched,selectedStreams:results.length,pageStreams:results.filter(row=>row.roles.includes('page')).length,formStreams:results.filter(row=>row.roles.includes('form')).length,selectedBytes,logicalPages:pages.length,pagesWithContent:pages.filter(page=>page.contents.length).length,contentArrays:pages.filter(page=>page.contents.length>1).length,pageStreamReferences:pages.reduce((sum,page)=>sum+page.contents.length,0),completeScans:results.filter(row=>row.complete).length,incompleteScans:results.filter(row=>!row.complete).length,operatorOccurrences:Object.values(operators).reduce((a,b)=>a+b,0),operators,streamsPerOperator,unknown,classificationErrors,lexerCrossChecks,limitations:['Lexical workload inventory only; no operand arity or graphics/text semantics validation.','Unique stream counts do not count repeat execution or Form resource invocation.','Forms include all Form-subtype streams in the baseline, including appearance streams; reachability is not claimed.','Each stream is scanned individually; a content-array semantic parser must concatenate with explicit source mapping.','Scanning stops at BI rather than searching binary inline-image data for EI.'],pages,results};
await writeFile(reportArg,JSON.stringify(summary,null,2)+'\n');
console.log(JSON.stringify({...summary,pages:undefined,results:undefined},null,2));
process.exitCode=classificationErrors.length||lexerCrossChecks.failures.length?1:0;
