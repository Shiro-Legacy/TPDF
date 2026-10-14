const $ = id => document.getElementById(id);
const api = window.rustWorkflow;
const worker = new Worker('./worker.js', { type: 'module' });
let sequence = 0, state = null, selected = null, items = [], page = 0, busy = false, lastSave = false, failed = false;
const pending = new Map();
let resolveReady, rejectReady;
const ready = new Promise((resolve,reject)=>{resolveReady=resolve;rejectReady=reject;});
worker.onmessage = ({data}) => {
  if(data.ready){resolveReady();status('Ready. Open a PDF.');return;}
  if(data.fatal){rejectReady(new Error(data.fatal));status(data.fatal,true);return;}
  const entry=pending.get(data.id); if(!entry)return;pending.delete(data.id);clearTimeout(entry.timer);
  data.ok ? entry.resolve(data) : entry.reject(new Error(data.error));
};
function failWorker(message){
  failed=true;worker.terminate();state=null;selected=null;
  for(const entry of pending.values()){clearTimeout(entry.timer);entry.reject(new Error(message));}pending.clear();
  rejectReady(new Error(message));status(message,true);sync();
}
worker.onerror = event => failWorker(event.message);
function request(op, values={}, transfer=[]){
  if(failed)return Promise.reject(new Error('The engine stopped. Restart the preview and reopen a saved file.'));
  const id=++sequence;
  return new Promise((resolve,reject)=>{
    const timer=setTimeout(()=>failWorker('Engine stopped after a 60-second timeout. Restart the preview; unsaved edits are unavailable.'),60000);
    pending.set(id,{resolve,reject,timer});worker.postMessage({id,op,...values},transfer);
  });
}
function status(text,error=false){$('status').textContent=text;$('status').classList.toggle('error',error);}
function sync(){
  $('open').disabled=busy||failed; $('pages').disabled=busy||!state;
  $('undo').disabled=busy||!state?.canUndo;$('redo').disabled=busy||!state?.canRedo;
  $('save').disabled=busy||!state?.capabilities?.canSave;$('reopen').disabled=busy||!lastSave||failed;
  $('save').title=state?.capabilities?.saveError?.detail||'';
  $('apply').disabled=busy||!selected;$('replacement').disabled=busy||!selected;
  $('dirty').textContent=state?.dirty?'Unsaved edits':'';
  for(const button of document.querySelectorAll('#runs button,.target'))button.disabled=busy;
}
async function perform(label, action){
  if(busy)return;
  busy=true;sync();status(label);
  try{await ready;await action();}catch(error){status(error.message,true);throw error;}finally{busy=false;sync();}
}
function clickAction(id,label,action){$(id).onclick=()=>{perform(label,action).catch(()=>{});};}
function select(item){
  selected=item;$('replacement').value=item.text;
  for(const button of document.querySelectorAll('[data-target]'))button.classList.toggle('selected',Number(button.dataset.target)===item.targetId);
  $('hint').textContent='Edit this text, then choose Apply edit.';sync();$('replacement').focus();
}
async function refresh(){
  selected=null;items=[];$('replacement').value='';$('runs').replaceChildren();$('targets').replaceChildren();
  // Never leave the previous page's pixels visible under a new page number or
  // filename when rendering the newly selected page fails.
  $('paper').hidden=true;$('empty').hidden=false;
  $('canvas').width=0;$('canvas').height=0;
  const capability=state?.capabilities?.pages[page];
  if(capability&&!capability.canRender){
    $('empty').textContent=`Page ${page+1} cannot be displayed: ${capability.renderError.detail}`;
    $('hint').textContent=capability.editError?.detail||'Editing is unavailable on this page.';
    sync();return;
  }
  $('empty').textContent='Loading page…';
  const rendered=await request('render',{page,dpi:72});state={...state,...rendered};delete state.pixels;
  const canvas=$('canvas');canvas.width=rendered.width;canvas.height=rendered.height;
  canvas.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(rendered.pixels),rendered.width,rendered.height),0,0);
  $('paper').hidden=false;$('empty').hidden=true;
  if(capability&&!capability.canEditText){
    $('hint').textContent=capability.editError?.detail||'No supported text targets on this page.';
    sync();return;
  }
  try{
    const scan=await request('edit-targets',{page});items=scan.items;
    for(const item of items){
      const button=document.createElement('button');button.textContent=item.text;button.dataset.target=item.targetId;button.onclick=()=>select(item);$('runs').append(button);
      const overlay=document.createElement('button');overlay.className='target';overlay.dataset.target=item.targetId;overlay.setAttribute('aria-label',`Edit ${item.text}`);
      const [x0,y0,x1,y1]=item.bounds;Object.assign(overlay.style,{left:`${x0-2}px`,top:`${y0-2}px`,width:`${x1-x0+4}px`,height:`${y1-y0+4}px`});overlay.onclick=()=>select(item);$('targets').append(overlay);
    }
    $('hint').textContent=items.length?'Select text on the page or from the list below.':'No supported text targets on this page.';
  }catch(error){$('hint').textContent=error.message;}
  sync();
}
async function openFile(reopen=false){
  const file=await (reopen?api.reopen():api.open());if(!file)return;
  const bytes=new Uint8Array(file.bytes).buffer;
  const next=await request('open',{bytes},[bytes]);await api.opened(file.token);state=next;page=0;$('name').textContent=file.name;
  $('pages').replaceChildren(...Array.from({length:state.pageCount},(_,i)=>{const option=document.createElement('option');option.value=i;option.textContent=i+1;return option;}));
  await refresh();status(`Opened ${file.name}.`);
}
clickAction('open','Opening PDF…',async()=>{if(state?.dirty&&!confirm('Discard unsaved edits and open another PDF?'))return;await openFile();});
clickAction('undo','Undoing edit…',async()=>{state=await request('undo');await refresh();status('Edit undone.');});
clickAction('redo','Redoing edit…',async()=>{state=await request('redo');await refresh();status('Edit redone.');});
clickAction('reopen','Reopening saved PDF…',async()=>{if(state?.dirty&&!confirm('Discard unsaved edits and reopen the saved copy?'))return;await openFile(true);});
$('pages').onchange=()=>perform('Loading page…',async()=>{page=Number($('pages').value);await refresh();status(`Page ${page+1}.`);}).catch(()=>{});
$('editor').onsubmit=event=>{
  event.preventDefault();if(!selected||busy)return;
  const target=selected,text=$('replacement').value;
  perform('Applying edit…',async()=>{state=await request('edit-text',{page,targetId:target.targetId,text,generation:state.generation,revision:state.revision});await refresh();status('Text updated.');}).catch(()=>{});
};
clickAction('save','Saving PDF as a new copy…',async()=>{
  let pendingSave=false,ticket=null;
  try{
    const saved=await request('save');pendingSave=true;ticket=saved.ticket;
    const validated=await request('validate',{bytes:saved.bytes},[saved.bytes]);
    const result=await api.save(new Uint8Array(validated.bytes));
    if(!result){status('Save cancelled.');return;}
    state=await request('saved',{ticket});pendingSave=false;lastSave=true;$('name').textContent=result.name;status(`Saved ${result.name}.${result.warning ? ' Durability warning: '+result.warning : ''}`);
  }finally{if(pendingSave)await request('cancel-save',{ticket});}
});
if(api.testMode){
  const hash=async()=>Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',$('canvas').getContext('2d').getImageData(0,0,$('canvas').width,$('canvas').height).data))).map(v=>v.toString(16).padStart(2,'0')).join('');
  async function idle(allowError=false){const end=Date.now()+60000;while(busy&&Date.now()<end)await new Promise(resolve=>setTimeout(resolve,20));if(busy)throw new Error('UI stuck busy');if(!allowError&&$('status').classList.contains('error'))throw new Error($('status').textContent);}
  function assert(condition,message){if(!condition)throw new Error(message);}
  window.workflowTest={async view(){
    await ready;$('open').click();await idle();
    const pages=[];
    for(let index=0;index<state.pageCount;index++){
      $('pages').value=index;$('pages').dispatchEvent(new Event('change'));await idle();
      pages.push({page:index,displayed:!$('paper').hidden,pixelSha256:$('paper').hidden?null:await hash(),
        capability:state.capabilities.pages[index],hint:$('hint').textContent,
        width:$('canvas').width,height:$('canvas').height});
    }
    const workerState=await request('state');
    return {backend:'rust-wasm',pages,canSave:state.capabilities.canSave,workerNodeAbsent:workerState.nodeAbsent,
      nodeAbsent:typeof process==='undefined'&&typeof require==='undefined'};
  },async run(){
    await ready;$('open').click();await idle();const before=await hash();
    document.querySelector('#runs button').click();$('replacement').value='Hello world';$('apply').click();await idle();
    assert(state.dirty,'edit did not dirty document');const edited=await hash();assert(before!==edited,'pixels did not change');
    $('undo').click();await idle();assert((await hash())===before,'undo pixels differ');assert(!state.dirty,'undo should restore clean state');
    $('redo').click();await idle();assert((await hash())===edited,'redo pixels differ');
    await api.testSave('cancel');$('save').click();await idle();assert(state.dirty,'cancel cleared dirty state');
    await api.testSave('disk-full');$('save').click();await idle(true);assert(state.dirty,'failed save cleared dirty state');
    assert($('status').textContent.includes('disk-full'),'missing disk failure');
    $('save').click();await idle();assert(!state.dirty,'save was not acknowledged');assert($('name').textContent==='workflow-save-1.pdf','Save As did not adopt the saved filename');
    $('reopen').click();await idle();assert((await hash())===edited,'fresh reopen pixels differ');
    assert(items.some(t=>t.text==='Hello world'),'saved text missing');
    document.querySelector('#runs button').click();$('replacement').value='Edited again';$('apply').click();await idle();
    $('save').click();await idle();$('reopen').click();await idle();assert(items.some(t=>t.text==='Edited again'),'second edit did not survive');
    const scan = await request('edit-targets',{page:0});
    const pendingSave = await request('save');
    await request('edit-text',{page:0,targetId:0,text:'Late edit',generation:scan.generation,revision:scan.revision});
    const lateAck = await request('saved',{ticket:pendingSave.ticket}); assert(lateAck.dirty,'late save marked newer edit clean');
    const restored = await request('undo'); assert(!restored.dirty,'saved node identity was lost');
    let staleRejected=false;
    try{await request('edit-text',{page:0,targetId:0,text:'Stale',generation:scan.generation,revision:scan.revision});}catch{staleRejected=true;}
    assert(staleRejected,'stale worker target accepted');
    const cancelled = await request('save');await request('cancel-save',{ticket:cancelled.ticket});
    let cancelledRejected=false;try{await request('saved',{ticket:cancelled.ticket});}catch{cancelledRejected=true;}
    assert(cancelledRejected,'cancelled save acknowledged');
    let invalidRejected=false;try{await request('open',{bytes:new Uint8Array([1,2,3]).buffer});}catch{invalidRejected=true;}
    assert(invalidRejected,'invalid document opened');
    const kept=await request('state');assert(kept.generation===restored.generation,'failed open replaced live document');
    assert(kept.nodeAbsent,'Node exposed in worker');
    // Synthetic capability checks execute through the real WASM worker and UI.
    const pdf = objects => {
      let value='%PDF-1.7\n';const offsets=[];
      objects.forEach((object,i)=>{offsets.push(value.length);value+=`${i+1} 0 obj\n${object}\nendobj\n`;});
      const start=value.length;value+=`xref\n0 ${objects.length+1}\n0000000000 65535 f \n`;
      offsets.forEach(offset=>value+=`${String(offset).padStart(10,'0')} 00000 n \n`);
      value+=`trailer\n<< /Size ${objects.length+1} /Root 1 0 R >>\nstartxref\n${start}\n%%EOF\n`;
      return new TextEncoder().encode(value).buffer;
    };
    const make = content => `<< /Length ${content.length} >>\nstream\n${content}\nendstream`;
    const signed=pdf(['<< /Type /Catalog /Pages 2 0 R /Perms << /DocMDP 5 0 R >> >>',
      '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
      '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R >>',
      make('0 0 50 50 re f'),'<< /Type /Sig /ByteRange [0 1 2 3] /Contents <00> >>']);
    state=await request('open',{bytes:signed});page=0;await refresh();
    assert(!state.capabilities.canSave&&state.capabilities.pages[0].canRender,'signed view capabilities incorrect');
    assert($('save').disabled&&!$('paper').hidden,'signed view controls incorrect');
    let signedSaveRejected=false;try{await request('save');}catch{signedSaveRejected=true;}
    assert(signedSaveRejected,'signed save accepted');
    let signedValidationRejected=false;try{await request('validate',{bytes:signed});}catch{signedValidationRejected=true;}
    assert(signedValidationRejected,'save validator inherited permissive view admission');
    const partial=pdf(['<< /Type /Catalog /Pages 2 0 R >>','<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>',
      '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R >>',make('0 0 50 50 re f'),
      '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 6 0 R >>',make('/Unsupported cs')]);
    state=await request('open',{bytes:partial});page=0;await refresh();assert(!$('paper').hidden,'supported neighbor hidden');
    page=1;await refresh();assert($('paper').hidden&&$('canvas').width===0,'unsupported page showed stale pixels');
    assert($('empty').textContent.includes('cannot be displayed'),'missing page refusal');
    const capabilityChecks={signedSaveRejected,signedValidationRejected,unsupportedPageClearsPixels:true};
    await request('close');await openFile(true);
    return {backend:'rust-wasm',before,edited,reopened:true,secondEdit:true,state,capabilityChecks,
      workerFailures:{staleRejected,cancelledRejected,invalidRejected,lateSaveStayedDirty:lateAck.dirty,cancelAndDiskFailureKeptDirty:true},
      workerNodeAbsent:kept.nodeAbsent,nodeAbsent:typeof process==='undefined'&&typeof require==='undefined'};
  }};
}
