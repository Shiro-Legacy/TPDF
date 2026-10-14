import { mkdtemp, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import assert from 'node:assert/strict';
const directory=fileURLToPath(new URL('.',import.meta.url));
let javaHome=process.env.JAVA_HOME;
if(!javaHome){const bundled=join(directory,'../../.tooling/jdk');const versions=await readdir(bundled);javaHome=join(bundled,versions.find(name=>name.startsWith('jdk-')),'Contents/Home');}
const temp=await mkdtemp(join(tmpdir(),'tandem-android-auth-security-'));
function run(command,args){const result=spawnSync(command,args,{encoding:'utf8',timeout:30000});process.stdout.write(result.stdout||'');process.stderr.write(result.stderr||'');assert.equal(result.status,0,`Native protocol test failed: ${result.error||''}`);}
try{
  run(join(javaHome,'bin/javac'),['-d',temp,join(directory,'../app/src/main/java/com/tandempdf/mobile/ChatGPTProtocol.java'),join(directory,'ChatGPTProtocolTest.java')]);
  run(join(javaHome,'bin/java'),['-cp',temp,'com.tandempdf.mobile.ChatGPTProtocolTest']);
}finally{await rm(temp,{recursive:true,force:true});}
