import { readFile, writeFile, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import assert from 'node:assert/strict';
const directory = fileURLToPath(new URL('.', import.meta.url));
const source = await readFile(join(directory,'../App/App/ChatGPTAccountPlugin.swift'),'utf8');
function declaration(start, end) {
  const from=source.indexOf(start), to=end ? source.indexOf(end,from+start.length) : source.length;
  assert.ok(from>=0 && to>from, `Production declaration unavailable: ${start}`);
  return source.slice(from,to);
}
const parts = [
  'import Foundation\nimport Security\n',
  declaration('private enum ChatGPTFailure:', 'private struct ChatGPTRegistration:'),
  declaration('private enum ChatGPTOAuthCallback {', 'private final class ChatGPTHTTP:'),
  declaration('private enum ChatGPTJWT {', 'private enum ChatGPTStream {'),
  declaration('private enum ChatGPTStream {', 'private extension Data {'),
  declaration('private extension Data {'),
  'private enum ChatGPTHTTP { static func json(url: String) async throws -> [String: Any] { fatalError("Unexpected network call") } }\n',
  await readFile(join(directory,'chatgpt-security.swift'),'utf8'),
];
const temporary = await mkdtemp(join(tmpdir(),'tandem-ios-auth-security-'));
try {
  const script=join(temporary,'security.swift'); await writeFile(script,parts.join('\n'));
  const result=spawnSync('xcrun',['swift','-module-cache-path',join(temporary,'modules'),script],{encoding:'utf8',timeout:120000,maxBuffer:2_000_000});
  process.stdout.write(result.stdout || ''); process.stderr.write(result.stderr || '');
  assert.equal(result.status,0,`Native security harness failed: ${result.error || ''}`);
} finally { await rm(temporary,{recursive:true,force:true}); }
