#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
// M10 fixed benchmark: the app's engine layer, as the renderer's worker drives
// it, on a fixed document set (docs.json), for the Rust engine (the production
// adapter build/renderer/engine-rust.js over engine/web/pkg) and for MuPDF
// (build-oracle/engine/oracle/app-engine/engine.js over node_modules/mupdf) as the comparison.
//
// Per document and engine, one child process per run (node
// --max-old-space-size, a wall-clock timeout and a resident-memory ceiling the
// parent enforces by polling `ps`). A pass, on a fresh session:
//   open (+ authenticate when the document has a user password), first page
//   (render page 0 at scale 1), zoom (page 0 at 0.5, 1, 2 and 4), thumbnails
//   (maxEdge 256, the first min(pages, 50) pages), search (the fixed query,
//   500 hits, the app's budget), an edit sequence (20 Square annot-adds on
//   page 0, page-rotate page 0, and field-set of the first fillable text field
//   on page 0 for form documents), save (+ saved), reopen (the saved bytes in a
//   new session, and its first page), and session growth (200 cycles of
//   annot-add then undo on the edited session). A document whose permissions
//   refuse annotations (docs.json `readOnly`) gets no edits and no cycles.
// Cold: --cold fresh processes, one pass each (engine load included in
// `engineLoad`, not in the pass). Warm: one process, one discarded pass, then
// --warm passes. Each metric: median, p90 (nearest rank) and max.
//
// Memory: peak RSS is the child's own maximum resident size
// (process.resourceUsage().maxRSS). WASM memory is the engine's linear memory
// size (the high-water mark; it never shrinks). Retained history is how much
// WASM linear memory and the JS heap (heapUsed, after a GC) grew over
// the edit sequence; session growth is the same over the 200 cycles. The
// allocators reuse freed blocks, so a WASM figure shows only what outgrew the
// memory earlier steps freed; a steady growth across cycles is a leak.
//
// Privacy: corpus files are untrusted. They are named only by source, path
// within the source and SHA-256, checked before use; records hold only timings,
// sizes, counts and fixed error classes, never document text or bytes.
//
// usage (from the app root, after engine/tools/build-workflow.sh and `npm run build`, `npm run build:oracle`):
//   node engine/oracle/bench/bench.mjs --out DIR [--engines rust,mupdf] [--docs id,...] [--cold 3] [--warm 10]
//        [--cycles 200] [--heap-mb 2048] [--rss-mb 2500] [--timeout 900] [--load-max 2] [--load-wait 900] [--child-wait 60]
//        [--corpus DIR] [--results FILE] [--resume]
//   node engine/oracle/bench/bench.mjs --child --engine rust|mupdf --doc ID [--passes N] [--cycles 200]
// DIR (outside the repository) gets one JSON file per child (--resume reuses those already there);
// --results (default DIR/results.json) gets the aggregate.
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");
const SELF = fileURLToPath(import.meta.url);
const DOCS_FILE = path.join(path.dirname(SELF), "docs.json");
const args = process.argv.slice(2);
const flag = (name, fallback) => {
  const at = args.indexOf(name);
  return at >= 0 && args[at + 1] !== undefined ? args[at + 1] : fallback;
};
const has = name => args.includes(name);

/** Never read (project rule). */
const FORBIDDEN = "SBI05131482.pdf";
export const SCALES = [0.5, 1, 2, 4];
export const THUMB_CAP = 50;
export const THUMB_MAX_EDGE = 256;
export const MAX_HITS = 500;
export const ANNOTS = 20;

const now = () => performance.now();
const r2 = value => Math.round(value * 100) / 100;
const sha256 = data => createHash("sha256").update(data).digest("hex");
const MB = 1024 * 1024;

export function loadDocs() {
  return JSON.parse(fs.readFileSync(DOCS_FILE, "utf8"));
}

function defaultCorpus() {
  const candidates = [path.join(root, "../tpdf-corpus"), path.join(root, "../../tpdf-corpus")];
  return candidates.find(dir => fs.existsSync(dir)) ?? candidates[0];
}

/** The document's bytes after the checks (regular file, no symlink, size and SHA-256), or throws a fixed reason. */
function readDoc(doc, corpus) {
  let file;
  let base;
  if (doc.repo) {
    file = path.join(root, doc.repo);
    base = root;
  } else {
    base = path.join(corpus, doc.corpus.source);
    file = path.join(base, doc.corpus.path);
  }
  if (path.basename(file) === FORBIDDEN) throw new Error("doc: forbidden file");
  const real = fs.realpathSync(file);
  if (real !== path.join(fs.realpathSync(base), path.relative(base, file))) throw new Error("doc: symlink");
  const fd = fs.openSync(file, fs.constants.O_RDONLY | (fs.constants.O_NOFOLLOW ?? 0));
  let bytes;
  try {
    if (!fs.fstatSync(fd).isFile()) throw new Error("doc: not a regular file");
    bytes = fs.readFileSync(fd);
  } finally {
    fs.closeSync(fd);
  }
  const expect = doc.repo ? doc : doc.corpus;
  if (bytes.byteLength !== expect.bytes) throw new Error("doc: size differs from docs.json");
  if (sha256(bytes) !== expect.sha256) throw new Error("doc: sha256 differs from docs.json");
  return bytes;
}

// ---------------------------------------------------------------------------
// Statistics

/** Nearest-rank percentile of sorted values. */
const rank = (sorted, q) => sorted[Math.min(sorted.length - 1, Math.max(0, Math.ceil((q / 100) * sorted.length) - 1))];

export function stats(values) {
  const list = values.filter(value => typeof value === "number" && Number.isFinite(value));
  if (!list.length) return null;
  const sorted = [...list].sort((a, b) => a - b);
  const mid = sorted.length >> 1;
  const median = sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
  return { n: sorted.length, median: r2(median), p90: r2(rank(sorted, 90)), max: r2(sorted[sorted.length - 1]) };
}

// ---------------------------------------------------------------------------
// Child: one engine, one document, N passes

class Refused extends Error {
  constructor(op, reply) {
    const reason = typeof reply?.reason === "string" && /^[a-z][a-z0-9-]{0,40}$/.test(reply.reason) ? reply.reason : null;
    super(`${op}: refused${reason ? ` (${reason})` : ""}`);
    this.op = op;
    this.reply = reply;
  }
}

async function runChild() {
  const engineName = flag("--engine");
  const docId = flag("--doc");
  const passes = Math.max(1, Number(flag("--passes", 1)) | 0);
  const cycles = Math.max(0, Number(flag("--cycles", 200)) | 0);
  const debug = has("--debug");
  const { docs, query } = loadDocs();
  const doc = docs.find(entry => entry.id === docId);
  if (!doc) throw new Error(`unknown doc ${docId}`);
  const bytes = readDoc(doc, path.resolve(flag("--corpus", process.env.TPDF_CORPUS ?? defaultCorpus())));

  const loadStarted = now();
  let engine;
  let wasmBytes;
  if (engineName === "mupdf") {
    // The emscripten module object the factory fills in: its HEAPU8 tracks the current linear memory.
    globalThis.$libmupdf_wasm_Module = {};
    const mupdf = (await import(path.join(root, "node_modules/mupdf/dist/mupdf.js"))).default;
    mupdf.setLog({ warning: () => {}, error: () => {} });
    const module = globalThis.$libmupdf_wasm_Module;
    wasmBytes = () => module.HEAPU8.buffer.byteLength;
    const { createEngine } = await import(path.join(root, "build-oracle/engine/oracle/app-engine/engine.js"));
    engine = createEngine(mupdf);
  } else if (engineName === "rust") {
    const wasm = await import(path.join(root, "engine/web/pkg/tandem_rust_workflow.js"));
    const exports = wasm.initSync({ module: fs.readFileSync(path.join(root, "engine/web/pkg/tandem_rust_workflow_bg.wasm")) });
    wasmBytes = () => exports.memory.buffer.byteLength;
    const { createRustEngine } = await import(path.join(root, "build/renderer/engine-rust.js"));
    engine = createRustEngine(wasm);
    // The app's CJK fallback font, read as the worker reads it: when a render first asks for it.
    const { readCjkFont } = await import(path.join(root, "scripts/cjk-font.mjs"));
    engine.useCjkFont(wasm, () => readCjkFont(root));
  } else {
    throw new Error(`unknown engine ${engineName}`);
  }
  const engineLoad = now() - loadStarted;

  let nextId = 1;
  function call(session, request) {
    const started = now();
    const { reply } = engine.dispatch({ id: nextId++, ...request }, session);
    const ms = now() - started;
    if (!reply.ok) {
      if (debug) process.stderr.write(`${request.op}: ${reply.error}\n`);
      throw new Refused(request.op, reply);
    }
    return { reply, ms };
  }
  // Two full collections with a turn of the event loop after each, so array buffers the
  // previous step dropped are released before memory is read.
  const turn = () => new Promise(resolve => setImmediate(resolve));
  const gc = async () => {
    for (let i = 0; i < 2; i++) {
      globalThis.gc?.();
      await turn();
    }
  };
  // The JS heap only: reply pixel buffers (external memory) are released lazily and would swamp it.
  const jsBytes = () => process.memoryUsage().heapUsed;
  const copy = data => (data instanceof ArrayBuffer ? data.slice(0) : new Uint8Array(data).slice().buffer);

  function open(session, data) {
    let { reply, ms } = call(session, { op: "open", pdf: copy(data), name: "document.pdf" });
    if (reply.info?.needsPassword) {
      if (typeof doc.password !== "string") throw new Error("open: locked and docs.json has no password");
      const unlocked = call(session, { op: "authenticate", password: doc.password });
      if (!unlocked.reply.authenticated) throw new Error("authenticate: not authenticated");
      ms += unlocked.ms;
      reply = unlocked.reply;
    }
    return { reply, ms };
  }

  const info = {};
  async function pass() {
    const m = {};
    const session = engine.createSession();
    const opened = open(session, bytes);
    m.open = opened.ms;
    const pageCount = opened.reply.pageCount;
    const size = opened.reply.pages?.[0] ?? { width: 612, height: 792 };
    info.pages = pageCount;
    m.firstPage = call(session, { op: "render", page: 0, scale: 1 }).ms;
    m.openFirstPage = m.open + m.firstPage;
    for (const scale of SCALES) m[`render${scale}`] = call(session, { op: "render", page: 0, scale }).ms;

    const thumbs = Math.min(pageCount, THUMB_CAP);
    let started = now();
    const each = [];
    for (let page = 0; page < thumbs; page++) each.push(call(session, { op: "thumbnail", page, maxEdge: THUMB_MAX_EDGE }).ms);
    m.thumbnails = now() - started;
    m.thumbnailMax = Math.max(...each);
    info.thumbnails = thumbs;

    const searched = call(session, { op: "search", query, maxHits: MAX_HITS });
    m.search = searched.ms;
    info.hits = Array.isArray(searched.reply.hits) ? searched.reply.hits.length : null;

    // A document whose permissions refuse annotations (docs.json `readOnly`) gets no edits
    // and no growth cycles, as in the app; it is still saved and reopened.
    const box = (i, w, h) => {
      const x = 20 + (i % 5) * Math.max(10, (w - 60) / 5);
      const y = 20 + Math.floor(i / 5) * Math.max(10, (h - 60) / 5);
      return [x, y, x + 30, y + 20];
    };
    let turned = size;
    if (!doc.readOnly) {
      await gc();
      const wasmBefore = wasmBytes();
      const jsBefore = jsBytes();
      started = now();
      const adds = [];
      for (let i = 0; i < ANNOTS; i++) {
        adds.push(call(session, { op: "annot-add", page: 0, type: "Square", rect: box(i, size.width, size.height), contents: "" }).ms);
      }
      m.annotAdd = adds.reduce((sum, ms) => sum + ms, 0);
      m.annotAddMax = Math.max(...adds);
      const rotated = call(session, { op: "page-rotate", page: 0, by: 90 });
      m.rotate = rotated.ms;
      // The growth cycles add to the rotated page, in its own (rotated) size.
      turned = rotated.reply.pages?.[0] ?? { width: size.height, height: size.width };
      if (doc.forms) {
        const fields = call(session, { op: "fields", page: 0 });
        m.fields = fields.ms;
        // The first fillable text field, given a value within its /MaxLen.
        const target = (fields.reply.items ?? []).find(item => item.kind === "text" && !item.readOnly);
        const value = "bench 1".slice(0, target?.maxLen || undefined);
        if (target) m.fieldSet = call(session, { op: "field-set", page: 0, widgetId: target.id, value }).ms;
        info.fieldSet = !!target;
      }
      m.edit = now() - started;
      await gc();
      m.historyWasmMb = (wasmBytes() - wasmBefore) / MB;
      m.historyJsMb = (jsBytes() - jsBefore) / MB;
    }

    const saved = call(session, { op: "save" });
    m.save = saved.ms;
    const out = saved.reply.bytes;
    info.savedBytes = out?.byteLength ?? null;
    call(session, { op: "saved", ...(typeof saved.reply.ticket === "string" ? { ticket: saved.reply.ticket } : {}) });

    const second = engine.createSession();
    const reopened = open(second, out);
    m.reopen = reopened.ms;
    m.reopenFirstPage = call(second, { op: "render", page: 0, scale: 1 }).ms;
    call(second, { op: "close" });

    if (cycles && !doc.readOnly) {
      await gc();
      const wasm0 = wasmBytes();
      const js0 = jsBytes();
      const rss0 = process.memoryUsage().rss;
      const times = [];
      for (let i = 0; i < cycles; i++) {
        started = now();
        call(session, { op: "annot-add", page: 0, type: "Square", rect: box(i % 25, turned.width, turned.height), contents: "" });
        call(session, { op: "undo" });
        times.push(now() - started);
      }
      await gc();
      m.growthWasmMb = (wasmBytes() - wasm0) / MB;
      m.growthJsMb = (jsBytes() - js0) / MB;
      m.growthRssMb = (process.memoryUsage().rss - rss0) / MB;
      const sorted = times.sort((a, b) => a - b);
      m.cycleMedian = sorted[sorted.length >> 1];
      m.cycleMax = sorted[sorted.length - 1];
      m.cycles = times.reduce((sum, ms) => sum + ms, 0);
    }
    call(session, { op: "close" });
    for (const key of Object.keys(m)) m[key] = r2(m[key]);
    return m;
  }

  const results = [];
  let error = null;
  try {
    for (let i = 0; i < passes; i++) results.push(await pass());
  } catch (err) {
    error = err instanceof Refused ? err.message : `${String(err?.name ?? "Error")}: ${err instanceof Error && /^(doc|open|authenticate):/.test(err.message) ? err.message : "thrown"}`;
    if (debug) process.stderr.write(`${err?.stack ?? err}\n`);
  }
  const record = {
    engine: engineName, doc: docId, passes: results, error,
    engineLoad: r2(engineLoad),
    peakRssMb: r2((process.resourceUsage().maxRSS * 1024) / MB),
    wasmMb: r2(wasmBytes() / MB),
    info,
  };
  process.stdout.write(`${JSON.stringify(record)}\n`);
}

// ---------------------------------------------------------------------------
// Parent: load wait, children, aggregate

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const load1 = () => r2(os.loadavg()[0]);

/** Waits (polling every `pollS`, up to `waitS`) until the 1-minute load average is below `max`; the last load seen. */
async function waitForQuiet(max, waitS, pollS) {
  const seen = [load1()];
  const deadline = Date.now() + waitS * 1000;
  while (seen[seen.length - 1] >= max && Date.now() < deadline) {
    await sleep(pollS * 1000);
    seen.push(load1());
  }
  return { load: seen[seen.length - 1], waited: seen.length - 1, quiet: seen[seen.length - 1] < max };
}

function rssMbOf(pid) {
  try {
    return Number(execFileSync("ps", ["-o", "rss=", "-p", String(pid)], { encoding: "utf8" }).trim()) / 1024;
  } catch {
    return 0;
  }
}

function runOne({ engine, doc, passes, cycles, heapMb, rssMb, timeoutS, corpus }) {
  return new Promise(resolve => {
    const child = spawn(process.execPath, [
      `--max-old-space-size=${heapMb}`, "--expose-gc", SELF, "--child", "--engine", engine, "--doc", doc,
      "--passes", String(passes), "--cycles", String(cycles), "--corpus", corpus,
    ], { cwd: root, stdio: ["ignore", "pipe", "pipe"] });
    let out = "";
    let err = "";
    let killed = null;
    let polledPeak = 0;
    child.stdout.on("data", chunk => { out += chunk; });
    child.stderr.on("data", chunk => { err += chunk; });
    const timer = setTimeout(() => { killed = "timeout"; child.kill("SIGKILL"); }, timeoutS * 1000);
    const poll = setInterval(() => {
      const mb = rssMbOf(child.pid);
      polledPeak = Math.max(polledPeak, mb);
      if (mb > rssMb) { killed = "rss"; child.kill("SIGKILL"); }
    }, 250);
    child.on("close", code => {
      clearTimeout(timer);
      clearInterval(poll);
      let record = null;
      try {
        record = JSON.parse(out.trim().split("\n").pop());
      } catch {
        record = null;
      }
      if (!record) {
        const oom = /heap out of memory|RangeError: (WebAssembly\.Memory|Array buffer allocation)/i.test(err);
        record = { engine, doc, passes: [], error: killed ?? (oom ? "oom" : `exit ${code}`) };
      }
      record.polledPeakRssMb = r2(polledPeak);
      resolve(record);
    });
  });
}

export const METRICS = [
  "open", "firstPage", "openFirstPage", ...SCALES.map(scale => `render${scale}`), "thumbnails", "thumbnailMax", "search",
  "annotAdd", "annotAddMax", "rotate", "fields", "fieldSet", "edit", "save", "reopen", "reopenFirstPage",
  "cycleMedian", "cycleMax", "cycles", "historyWasmMb", "historyJsMb", "growthWasmMb", "growthJsMb", "growthRssMb",
];
/** Per-process figures (one sample per process). */
export const PROCESS_METRICS = ["engineLoad", "peakRssMb", "wasmMb"];

/** cold/warm stats per metric from the raw child records of one engine and document. */
export function aggregate(cold, warm) {
  const out = {};
  const coldPasses = cold.map(record => record.passes[0]).filter(Boolean);
  const warmPasses = warm ? warm.passes.slice(1) : [];
  for (const metric of METRICS) {
    const entry = { cold: stats(coldPasses.map(m => m[metric])), warm: stats(warmPasses.map(m => m[metric])) };
    if (entry.cold || entry.warm) out[metric] = entry;
  }
  for (const metric of PROCESS_METRICS) {
    out[metric] = { cold: stats(cold.map(record => record[metric])), warm: stats(warm ? [warm[metric]] : []) };
  }
  return out;
}

function machine() {
  const sys = key => {
    try {
      return execFileSync("sysctl", ["-n", key], { encoding: "utf8" }).trim();
    } catch {
      return null;
    }
  };
  const sw = key => {
    try {
      return execFileSync("sw_vers", [key], { encoding: "utf8" }).trim();
    } catch {
      return null;
    }
  };
  return {
    model: sys("hw.model"), cpu: sys("machdep.cpu.brand_string"), cores: os.cpus().length,
    memoryGb: Math.round(os.totalmem() / 1024 ** 3), os: `macOS ${sw("-productVersion")} (${sw("-buildVersion")})`,
    arch: process.arch,
  };
}

function versions() {
  const read = file => {
    try {
      return JSON.parse(fs.readFileSync(path.join(root, file), "utf8")).version;
    } catch {
      return null;
    }
  };
  let commit = null;
  let modified = null;
  try {
    commit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
    modified = execFileSync("git", ["status", "--porcelain", "--", "engine/src", "engine/bridge/src", "src/renderer"], { cwd: root, encoding: "utf8" }).trim() !== "";
  } catch {
    // not a checkout
  }
  return {
    node: process.version, electron: read("node_modules/electron/package.json"), mupdf: read("node_modules/mupdf/package.json"),
    wasmSha256: sha256(fs.readFileSync(path.join(root, "engine/web/pkg/tandem_rust_workflow_bg.wasm"))),
    commit, engineSourcesModified: modified,
  };
}

async function runParent() {
  const outDir = flag("--out");
  if (!outDir) throw new Error("--out DIR is required (outside the repository)");
  fs.mkdirSync(outDir, { recursive: true });
  const engines = flag("--engines", "rust,mupdf").split(",");
  const { docs } = loadDocs();
  const only = flag("--docs", null)?.split(",");
  const selected = only ? docs.filter(doc => only.includes(doc.id)) : docs;
  const cold = Number(flag("--cold", 3));
  const warm = Number(flag("--warm", 10));
  const cycles = Number(flag("--cycles", 200));
  const heapMb = Number(flag("--heap-mb", 2048));
  const rssMb = Number(flag("--rss-mb", 2500));
  const timeoutS = Number(flag("--timeout", 900));
  const loadMax = Number(flag("--load-max", 2));
  const loadWait = Number(flag("--load-wait", 900));
  const corpus = path.resolve(flag("--corpus", process.env.TPDF_CORPUS ?? defaultCorpus()));
  const log = text => process.stderr.write(`${new Date().toISOString()} ${text}\n`);

  // Before the first measurement: up to --load-wait s (polling every 30 s) for the 1-minute load
  // average to fall below --load-max; before each child, up to --child-wait s more (polling every 5 s).
  const childWait = Number(flag("--child-wait", 60));
  const initial = await waitForQuiet(loadMax, loadWait, 30);
  log(`initial load ${initial.load} after ${initial.waited} polls${initial.quiet ? "" : " (still above the limit; measuring anyway)"}`);
  const runs = [];
  for (const doc of selected) {
    for (const engine of engines) {
      const plan = [...Array.from({ length: cold }, (_, i) => ({ kind: "cold", index: i, passes: 1 })),
        ...(warm > 0 ? [{ kind: "warm", index: 0, passes: warm + 1 }] : [])];
      for (const step of plan) {
        const file = path.join(outDir, `${doc.id}.${engine}.${step.kind}${step.index}.json`);
        if (has("--resume") && fs.existsSync(file)) {
          runs.push(JSON.parse(fs.readFileSync(file, "utf8")));
          continue;
        }
        const quiet = await waitForQuiet(loadMax, childWait, 5);
        const started = Date.now();
        const record = await runOne({ engine, doc: doc.id, passes: step.passes, cycles, heapMb, rssMb, timeoutS, corpus });
        Object.assign(record, { kind: step.kind, index: step.index, loadBefore: quiet.load, loadQuiet: quiet.quiet, loadWaitedPolls: quiet.waited, loadAfter: load1(), wallMs: Date.now() - started });
        fs.writeFileSync(file, `${JSON.stringify(record)}\n`);
        runs.push(record);
        log(`${doc.id} ${engine} ${step.kind}${step.index}: ${record.error ?? "ok"} load ${quiet.load}->${record.loadAfter} ${(record.wallMs / 1000).toFixed(1)} s`);
      }
    }
  }

  const results = {};
  for (const doc of selected) {
    results[doc.id] = {};
    for (const engine of engines) {
      const mine = runs.filter(run => run.doc === doc.id && run.engine === engine);
      const coldRuns = mine.filter(run => run.kind === "cold");
      const warmRun = mine.find(run => run.kind === "warm") ?? null;
      results[doc.id][engine] = {
        errors: mine.filter(run => run.error).map(run => `${run.kind}${run.index}: ${run.error}`),
        info: mine.find(run => run.info && Object.keys(run.info).length)?.info ?? null,
        metrics: aggregate(coldRuns, warmRun),
      };
    }
  }
  const loads = runs.map(run => run.loadBefore).filter(Number.isFinite);
  const aggregateRecord = {
    schema: "tpdf-bench-results/1",
    created: new Date().toISOString(),
    machine: machine(),
    versions: versions(),
    method: {
      cold, warm, cycles, scales: SCALES, thumbnailCap: THUMB_CAP, thumbnailMaxEdge: THUMB_MAX_EDGE, maxHits: MAX_HITS,
      annotAdds: ANNOTS, heapMb, rssMb, loadMax, units: "ms unless the metric name ends in Mb (MiB)",
      percentile: "nearest rank; cold n = processes, warm n = passes after one discarded pass in one process; per-process metrics (engineLoad, peakRssMb, wasmMb) have warm n = 1",
    },
    load: { initial, min: Math.min(...loads), max: Math.max(...loads), runsAboveMax: runs.filter(run => run.loadQuiet === false).length, perRun: runs.map(run => ({ doc: run.doc, engine: run.engine, run: `${run.kind}${run.index}`, before: run.loadBefore, after: run.loadAfter })) },
    docs: selected.map(doc => ({ id: doc.id, kind: doc.kind, ...(doc.repo ? { repo: doc.repo, sha256: doc.sha256, bytes: doc.bytes } : { source: doc.corpus.source, path: doc.corpus.path, sha256: doc.corpus.sha256, bytes: doc.corpus.bytes }) })),
    results,
  };
  const target = flag("--results", path.join(outDir, "results.json"));
  fs.writeFileSync(target, `${JSON.stringify(aggregateRecord, null, 1)}\n`);
  log(`wrote ${target}`);
}

if (import.meta.url === `file://${process.argv[1]}` || process.argv[1] === SELF) {
  (has("--child") ? runChild() : runParent()).catch(err => {
    process.stderr.write(`${err?.stack ?? err}\n`);
    process.exitCode = 1;
  });
}
