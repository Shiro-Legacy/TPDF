const {app,BrowserWindow,protocol,ipcMain,dialog}=require('electron');
const fs=require('node:fs/promises');const path=require('node:path');const crypto=require('node:crypto');
const args=process.argv.slice(1);const option=name=>args.find(v=>v.startsWith(`--${name}=`))?.slice(name.length+3);
const report=option('workflow-test');const testInput=option('workflow-input');
if(report)app.setPath('userData',path.join(path.dirname(report),'electron-profile'));
protocol.registerSchemesAsPrivileged([{scheme:'tandem-rust',privileges:{standard:true,secure:true,supportFetchAPI:true}}]);
const {pathToFileURL}=require('node:url');
const appRoot=process.env.TPDF_APP_ROOT||process.cwd();
let safeWrite;let currentSource=null;const opened=new Map();let saveFault=null;
const origin='tandem-rust://engine';const root=path.resolve(__dirname,'../web');
const allowed=new Map([['/index.html','text/html'],['/app.js','text/javascript'],['/worker.js','text/javascript'],['/style.css','text/css'],['/pkg/tandem_rust_workflow.js','text/javascript'],['/pkg/tandem_rust_workflow_bg.wasm','application/wasm']]);
const csp="default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self'; connect-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; object-src 'none'; base-uri 'none'";
let win,lastSaved=null,saveIndex=0;
function sender(event){if(event.sender!==win.webContents||!event.senderFrame?.url.startsWith(`${origin}/`))throw new Error('Invalid IPC sender');}
async function read(file){
  const source=await safeWrite.inspectRegularFile(file);
  if(!source||source.size>16*1024*1024)throw new Error('PDF must be at most 16 MiB');
  const bytes=await safeWrite.collectCapped(file,source.size);
  const token=crypto.randomUUID();opened.clear();opened.set(token,source);
  return {name:path.basename(file),bytes,token};
}
app.whenReady().then(async()=>{
  safeWrite=await import(pathToFileURL(path.join(appRoot,'build/safe-write.js')).href);
  await safeWrite.cleanupOrphans(path.join(app.getPath('userData'),'pending-saves.json'));
  ipcMain.handle('workflow:opened',(event,token)=>{sender(event);const source=opened.get(token);if(!source)throw new Error('Stale file open');currentSource=source;opened.clear();});
  if(report)ipcMain.handle('workflow:test-save',(event,mode)=>{sender(event);if(!['cancel','disk-full'].includes(mode))throw new Error('Invalid fault');saveFault=mode;});
  protocol.handle('tandem-rust',async request=>{
    const url=new URL(request.url);const type=allowed.get(url.pathname);
    if(url.host!=='engine'||!type)return new Response('Not found',{status:404});
    return new Response(await fs.readFile(path.join(root,url.pathname)),{headers:{'Content-Type':type,'Content-Security-Policy':csp}});
  });
  win=new BrowserWindow({width:1150,height:900,show:!report,webPreferences:{preload:path.join(__dirname,'preload.cjs'),sandbox:true,contextIsolation:true,nodeIntegration:false,additionalArguments:report?['--workflow-test-mode']:[]}});
  win.webContents.setWindowOpenHandler(()=>({action:'deny'}));
  win.webContents.on('will-navigate',(event,url)=>{if(!url.startsWith(`${origin}/`))event.preventDefault();});
  ipcMain.handle('workflow:open',async event=>{sender(event);if(report)return read(testInput);const result=await dialog.showOpenDialog(win,{properties:['openFile'],filters:[{name:'PDF',extensions:['pdf']}]});return result.canceled?null:read(result.filePaths[0]);});
  ipcMain.handle('workflow:reopen',async event=>{sender(event);if(!lastSaved)throw new Error('No saved copy');return read(lastSaved);});
  ipcMain.handle('workflow:save',async(event,bytes)=>{
    sender(event);if(!(bytes instanceof Uint8Array)||bytes.byteLength<5||bytes.byteLength>16*1024*1024)throw new Error('Invalid PDF output');
    if(!currentSource)throw new Error('No confirmed source');
    const source=currentSource;const fault=saveFault;saveFault=null;if(fault==='cancel')return null;
    let destination;
    if(report)destination=path.join(path.dirname(report),`workflow-save-${saveIndex+1}.pdf`);
    else{const result=await dialog.showSaveDialog(win,{defaultPath:safeWrite.copyBasename(path.basename(source.path)),filters:[{name:'PDF',extensions:['pdf']}]});if(result.canceled)return null;destination=result.filePath;}
    const result=await safeWrite.saveCopy({dest:destination,bytes,source,
      isCurrent:()=>currentSource===source,journalPath:path.join(app.getPath('userData'),'pending-saves.json'),
      inject:fault==='disk-full'?{writeAll:async()=>{throw Object.assign(new Error('injected disk full'),{code:'ENOSPC'});}}:undefined});
    if(!result.ok)throw new Error(result.reason);
    const adopted=await safeWrite.inspectRegularFile(destination);
    if(!adopted)throw new Error('The copy was written but could not be reopened. Unsaved state is retained.');
    currentSource=adopted;lastSaved=destination;if(report)saveIndex+=1;return result;
  });
  await win.loadURL(`${origin}/index.html`);
  if(report){
    try{
      const result=await win.webContents.executeJavaScript(option('workflow-mode')==='view'?'window.workflowTest.view()':'window.workflowTest.run()');
      result.sourceSha256=crypto.createHash('sha256').update(await fs.readFile(testInput)).digest('hex');
      await fs.writeFile(report,JSON.stringify(result,null,2));
      await fs.writeFile(path.join(path.dirname(report),'workflow.png'),(await win.webContents.capturePage()).toPNG());app.exit(0);
    }catch(error){await fs.writeFile(report,JSON.stringify({error:String(error)},null,2));app.exit(1);}
  }
}).catch(error=>{console.error(error);app.exit(1);});
app.on('window-all-closed',()=>app.quit());
