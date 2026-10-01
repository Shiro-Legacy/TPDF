#!/usr/bin/env node
// Tandem PDF as an MCP server (contract-agent-session.md D3): stdio, newline-delimited JSON-RPC 2.0,
// no dependency. One live drive session (scripts/session.mjs) starts on the first tool call that
// needs it and closes at stdin EOF. Register: `claude mcp add tandem-pdf -- node <abs>/scripts/tandem-mcp.mjs`.
// Options: --out=<dir> (where relative tandem_save paths and ${OUT} land; default a new temp dir).
// In the repo it needs `npm run build` first; the packaged app ships this file under
// Contents/Resources/agent/scripts/ and drives itself. The agent reference is docs/agent.md (tool tandem_help).
import { mkdir, mkdtemp, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, isAbsolute, join, normalize, resolve, sep } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { createSession } from './session.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const { version } = JSON.parse(await readFile(join(root, 'package.json'), 'utf8'));
const outArg = process.argv.find(arg => arg.startsWith('--out='))?.slice('--out='.length);
const PROTOCOL = '2025-06-18';

let session; // Promise of the live session, made on first use and remade if the app exited.
let out;
async function live() {
  if (session) {
    try { if (!(await session).ended) return session; } catch { /* it never started; try again */ }
    session = undefined;
  }
  session ??= (async () => {
    out ??= resolve(outArg ?? await mkdtemp(join(tmpdir(), 'tandem-mcp-')));
    return createSession({ out });
  })();
  return session;
}
/** Relative paths are the agent's: resolved against this server's cwd (`${OUT}/…` is left to the app). */
const here = path => typeof path === 'string' && !path.startsWith('${OUT}') ? resolve(path) : path;

async function help(topic) {
  const text = await readFile(join(root, 'docs/agent.md'), 'utf8');
  if (!['create', 'modify', 'fill', 'ops'].includes(topic)) return text; // the schema's enum; never a RegExp from input
  const at = text.search(new RegExp(`^## ${topic}\\b`, 'im'));
  if (at < 0) return text;
  const next = text.slice(at + 3).search(/^## /m);
  return next < 0 ? text.slice(at) : text.slice(at, at + 3 + next);
}

const READ_BACK = 'The result is the drive STEP record {cmd, …, state}; state is the app\'s STATE after the step (doc.name, doc.pageCount, revision, dirty, error, announcement, content.edited/refused, fields.values/last, save).';
const TOOLS = [
  {
    name: 'tandem_open',
    description: `Open a PDF from disk (relative paths resolve against this server's working directory). Read state.doc {name, pageCount} and state.error ("" on success). ${READ_BACK}`,
    inputSchema: { type: 'object', properties: { path: { type: 'string', description: 'Path of the PDF to open' } }, required: ['path'] },
    step: args => ({ cmd: 'open', path: here(args.path) }),
  },
  {
    name: 'tandem_new',
    description: `Create a blank one-page document named Untitled.pdf (letter 612x792 pt by default, "a4", or [width, height] in points, each 72-14400) and make it the open document. Add text with tandem_do text-add and fields with tandem_do fields, then tandem_save. Read state.doc.name ("Untitled.pdf") and state.error. ${READ_BACK}`,
    inputSchema: { type: 'object', properties: { size: { oneOf: [{ type: 'string', enum: ['letter', 'a4'] }, { type: 'array', items: { type: 'number' }, minItems: 2, maxItems: 2 }] } } },
    step: args => ({ cmd: 'new', ...(args.size !== undefined ? { contents: { size: args.size } } : {}) }),
  },
  {
    name: 'tandem_do',
    description: `Run one drive step on the open document: {cmd, contents?, path?, to?, from?, range?, ms?} exactly as a drive script line (e.g. {"cmd":"text-add","contents":{"page":0,"point":[72,700],"text":"Hi","font":"Helvetica","size":12,"color":[0,0,0]}}, {"cmd":"fields","contents":{"action":"fill","name":"x","value":"y"}}, {"cmd":"content","contents":{"target":"old text","text":"new text"}}, {"cmd":"undo"}). Call tandem_help with topic "ops" for the op list, refusal tokens and ceilings. Check state.error, state.revision and the op's own slice. ${READ_BACK}`,
    inputSchema: {
      type: 'object',
      properties: {
        cmd: { type: 'string' }, contents: { type: ['object', 'string'] }, path: { type: 'string' }, to: { type: 'string' },
        from: { type: 'string' }, range: { type: 'string' }, ms: { type: 'number' },
      },
      required: ['cmd'],
    },
    step: args => ({ ...args, ...['path', 'to', 'from'].reduce((acc, key) => args[key] === undefined ? acc : { ...acc, [key]: here(args[key]) }, {}) }),
  },
  {
    name: 'tandem_state',
    description: `Read the current STATE without changing anything. ${READ_BACK}`,
    inputSchema: { type: 'object', properties: {} },
    step: () => ({ cmd: 'state' }),
  },
  {
    name: 'tandem_save',
    description: `Save a copy of the open document to a path (a relative path lands in this server's output folder, --out, and missing folders under it are created; it must stay inside --out; an absolute path needs an existing folder). Read state.save {ok, name} (ok false carries a reason). ${READ_BACK}`,
    inputSchema: { type: 'object', properties: { path: { type: 'string' } }, required: ['path'] },
    step: args => ({ cmd: 'save-to', path: isAbsolute(args.path) ? args.path : join(out, args.path) }),
  },
  {
    name: 'tandem_help',
    description: 'The agent reference (docs/agent.md): topic "create", "modify", "fill" or "ops"; the whole file without a topic. Read it before the first edit.',
    inputSchema: { type: 'object', properties: { topic: { type: 'string', enum: ['create', 'modify', 'fill', 'ops'] } } },
  },
];

const text = (value, isError) => ({ content: [{ type: 'text', text: typeof value === 'string' ? value : JSON.stringify(value, null, 1) }], isError });
async function callTool(tool, args) {
  if (tool.name === 'tandem_help') return text(await help(typeof args.topic === 'string' ? args.topic : undefined), false);
  for (const key of tool.inputSchema.required ?? []) if (typeof args[key] !== 'string' || !args[key]) return text(`${tool.name}: ${key} is required`, true);
  if (tool.name === 'tandem_do' && args.cmd === 'close') return text('tandem_do: close is refused; the session belongs to the server and ends when it stops', true);
  // Bugfix 1 D4: a relative save stays inside --out and gets its missing folders.
  const relativeSave = tool.name === 'tandem_save' && !isAbsolute(args.path);
  if (relativeSave && normalize(args.path).split(sep)[0] === '..') return text('tandem_save: path must stay inside the output folder', true);
  let step;
  try {
    const running = await live();
    const line = tool.step(args);
    if (relativeSave) await mkdir(dirname(line.path), { recursive: true });
    step = await running.step(line);
  } catch (err) {
    return text({ error: err.message }, true);
  }
  return text(step, Boolean(step.error) || (typeof step.state?.error === 'string' && step.state.error !== ''));
}

async function handle(message) {
  const { id, method, params } = message;
  const reply = body => { if (id !== undefined) process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id, ...body }) + '\n'); };
  if (method === 'initialize') {
    const protocolVersion = typeof params?.protocolVersion === 'string' ? params.protocolVersion : PROTOCOL;
    return reply({ result: { protocolVersion, capabilities: { tools: {} }, serverInfo: { name: 'tandem-pdf', version } } });
  }
  if (typeof method === 'string' && method.startsWith('notifications/')) return;
  if (method === 'ping') return reply({ result: {} });
  if (method === 'tools/list') return reply({ result: { tools: TOOLS.map(({ name, description, inputSchema }) => ({ name, description, inputSchema })) } });
  if (method === 'tools/call') {
    const tool = TOOLS.find(item => item.name === params?.name);
    if (!tool) return reply({ error: { code: -32602, message: `Unknown tool: ${params?.name}` } });
    return reply({ result: await callTool(tool, params.arguments && typeof params.arguments === 'object' ? params.arguments : {}) });
  }
  reply({ error: { code: -32601, message: `Method not found: ${method}` } });
}

const pending = new Set();
const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
lines.on('line', line => {
  if (!line.trim()) return;
  let message;
  try { message = JSON.parse(line); } catch {
    process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: null, error: { code: -32700, message: 'Parse error' } }) + '\n');
    return;
  }
  if (!message || typeof message !== 'object' || Array.isArray(message)) {
    process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: null, error: { code: -32600, message: 'Invalid Request' } }) + '\n');
    return;
  }
  const job = handle(message).catch(err => {
    if (message.id !== undefined) process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: message.id, error: { code: -32603, message: err.message } }) + '\n');
  });
  pending.add(job);
  job.finally(() => pending.delete(job));
});
lines.on('close', async () => {
  await Promise.all(pending);
  if (session) { try { await (await session).close(); } catch { /* the app already exited */ } }
  process.exitCode = 0;
});
