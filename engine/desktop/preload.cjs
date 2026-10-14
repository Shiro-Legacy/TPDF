const {contextBridge,ipcRenderer}=require('electron');
contextBridge.exposeInMainWorld('rustWorkflow',{
  open:()=>ipcRenderer.invoke('workflow:open'),
  opened:token=>ipcRenderer.invoke('workflow:opened',token),
  testSave:process.argv.includes('--workflow-test-mode')?mode=>ipcRenderer.invoke('workflow:test-save',mode):undefined,
  save:bytes=>ipcRenderer.invoke('workflow:save',bytes),
  reopen:()=>ipcRenderer.invoke('workflow:reopen'),
  testMode:process.argv.includes('--workflow-test-mode'),
});
