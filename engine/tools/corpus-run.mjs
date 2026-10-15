#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
// Differential run of the Rust app engine against MuPDF over the external
// corpus listed by corpus-manifest.mjs (engine/reference/corpus-20261015).
//
// Each file runs in its own child process (node --max-old-space-size, a hard
// wall-clock timeout and a resident-memory ceiling enforced by the parent), so
// a hang, a WASM trap or an OOM is recorded and the run goes on. The child
// dispatches the requests the app sends to both engines: engine.ts on the real
// mupdf module (node_modules/mupdf) and the Rust WASM package behind the
// production adapter (build/renderer/engine-rust.js): open (the empty password
// when locked; for qpdf files also the passwords qpdf's test scripts state),
// render at scale 1 of the first, middle and last page, text-all, search, then
// on Rust save and validate, the save reopened in MuPDF and in Rust, and an
// annot-add Square on page 0 saved and reopened in MuPDF.
//
// Privacy: corpus files are untrusted data. Records hold only numbers,
// booleans, fixed engine tokens, error classes and hashes of error messages,
// never document text, metadata or bytes. Per-file JSONL goes to the run
// directory (outside the repository); the summary holds counts, distributions
// and failing files as {source, path, sha256, classes}.
//
// usage (from the app root, after `npm run build` and engine/tools/build-workflow.sh):
//   node engine/tools/corpus-run.mjs --runs DIR [--jobs 4] [--timeout 60] [--heap-mb 2048]
//        [--rss-mb 3072] [--source NAME] [--limit N] [--only SHA256,...] [--run-id ID] [--resume]
//        [--manifest FILE] [--corpus DIR] [--summary FILE | --no-summary]
//   node engine/tools/corpus-run.mjs --summarize RUN_DIR [--summary FILE]
//   node engine/tools/corpus-run.mjs --child FILE [--debug]   (one file; --debug prints raw
//        error messages to stderr for local inspection only, never into a record)
import { execFile, execFileSync, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const SELF = fileURLToPath(import.meta.url);
const args = process.argv.slice(2);
const flag = (name, fallback) => {
  const at = args.indexOf(name);
  return at >= 0 && args[at + 1] !== undefined ? args[at + 1] : fallback;
};
const has = name => args.includes(name);

/** Never read, whatever the manifest says (project rule). */
const FORBIDDEN = "SBI05131482.pdf";
/** Render parity: a compared page below this share of pixels within 16 levels is render-pixels or render-edges (below). */
const WITHIN16_MIN = 98;
/**
 * Ink parity: pixels more than 64 levels apart as a share of the larger non-white pixel count.
 * The within-16 share hides missing or recoloured content on a mostly white page, and it
 * flags pages whose only difference is glyph and edge rasterization (measured: Rust's text is
 * heavier than MuPDF's). A page at or above INK_MAX percent with at least INK_MIN_PX such
 * pixels differs in content: render-pixels below the within-16 threshold, render-ink above it;
 * below the threshold with less ink apart it is render-edges, recorded but not a failure.
 */
const INK_MAX = 25;
const INK_MIN_PX = 200;
/** The share of a page's ink that differs by more than 64 levels, or null without the counts. */
function inkPct(row, inkOf = row) {
  if (typeof row.over64Px !== "number") return null;
  const ink = Math.max(Number(inkOf.mupdf?.nonWhite) || 0, Number(inkOf.rust?.nonWhite) || 0, Number(row.nonWhite) || 0, 1);
  return r3((100 * row.over64Px) / ink);
}
const inkMismatch = (row, inkOf) => {
  const ink = inkPct(row, inkOf);
  return ink !== null && ink >= INK_MAX && row.over64Px >= INK_MIN_PX;
};
/** The app's default render scale and its search budget (engine.ts MAX_HITS_DEFAULT). */
const SCALE = 1;
const MAX_HITS = 500;
const FIXED_QUERY = "the";
/** The password a saved copy is reopened with: the one that unlocked the original. */
const SAME = "same-password";

// ---------------------------------------------------------------------------
// Shared helpers

const r1 = value => Math.round(value * 10) / 10;
const r3 = value => Math.round(value * 1000) / 1000;
const sha256 = data => createHash("sha256").update(data).digest("hex");

/** A message's hash with digits folded, so one template clusters; the text itself is never kept. */
const messageHash = message => sha256(String(message ?? "").replace(/\d+/g, "#")).slice(0, 12);

const ERROR_CLASSES = [
  ["engine-stopped", /the PDF engine stopped/i],
  ["trap", /RuntimeError|unreachable|memory access out of bounds|Aborted\(|call stack/i],
  ["oom", /out of memory|cannot allocate|allocation failed|malloc|memory exhausted/i],
  ["needs-password", /password is required|needs a password|until it is unlocked/i],
  ["no-pages", /no pages/i],
  ["too-large", /exceeds|ceiling|edge limit|too large|too big/i],
  ["too-complex", /too complex/i],
  ["read-only", /read-only|does not permit/i],
  ["out-of-range", /out of range|must be an integer/i],
  ["not-pdf", /not a PDF|too few bytes|no objects found|cannot recognize version|not a pdf/i],
  ["unreadable", /could not be read|cannot open|failed to open|cannot find|syntax|invalid|cannot parse|corrupt|damaged|repair|xref|trailer|object|unexpected|expected/i],
];

/** `op:class` from fixed patterns; the message is not kept. */
function errorClass(message) {
  const text = String(message ?? "");
  const op = text.match(/^([a-z][a-z-]*):/)?.[1] ?? "error";
  const found = ERROR_CLASSES.find(([, pattern]) => pattern.test(text));
  return `${op}:${found ? found[0] : "other"}`;
}

/** Reason tokens are fixed engine strings; anything else is not recorded. */
const token = value => (typeof value === "string" && /^[a-z][a-z0-9-]{0,40}$/.test(value) ? value : undefined);

function percentiles(values) {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const at = q => sorted[Math.min(sorted.length - 1, Math.max(0, Math.ceil((q / 100) * sorted.length) - 1))];
  const mean = sorted.reduce((sum, value) => sum + value, 0) / sorted.length;
  return {
    n: sorted.length, min: sorted[0], p1: at(1), p5: at(5), p10: at(10), p25: at(25), p50: at(50),
    p75: at(75), p90: at(90), p99: at(99), max: sorted[sorted.length - 1], mean: r3(mean),
  };
}

function defaultCorpus() {
  const candidates = [path.join(root, "../tpdf-corpus"), path.join(root, "../../tpdf-corpus")];
  return candidates.find(dir => fs.existsSync(dir)) ?? candidates[0];
}

// ---------------------------------------------------------------------------
// Child: one file, both engines

async function runChild(file) {
  const debug = has("--debug");
  const progress = process.env.TPDF_CORPUS_PROGRESS;
  const passwords = JSON.parse(process.env.TPDF_CORPUS_PASSWORDS || "[]");
  const record = { v: 1, stage: "load" };
  const emit = stage => {
    record.stage = stage;
    if (progress) fs.writeFileSync(progress, JSON.stringify(record));
  };
  const raw = (where, message) => {
    if (debug) process.stderr.write(`[debug] ${where}: ${String(message).slice(0, 400)}\n`);
  };
  emit("load");
  const started = performance.now();
  const mupdf = (await import(path.join(root, "node_modules/mupdf/dist/mupdf.js"))).default;
  const warnings = { warning: 0, error: 0 };
  mupdf.setLog({ warning: () => warnings.warning++, error: () => warnings.error++ });
  const { createEngine } = await import(path.join(root, "build/renderer/engine.js"));
  const { createRustEngine } = await import(path.join(root, "build/renderer/engine-rust.js"));
  const wasm = await import(path.join(root, "engine/web/pkg/tandem_rust_workflow.js"));
  wasm.initSync({ module: fs.readFileSync(path.join(root, "engine/web/pkg/tandem_rust_workflow_bg.wasm")) });
  const engines = { mupdf: createEngine(mupdf), rust: createRustEngine(wasm) };
  record.loadMs = r1(performance.now() - started);

  if (path.basename(file) === FORBIDDEN) throw new Error("refusing a forbidden file");
  const bytes = fs.readFileSync(file);
  record.bytes = bytes.byteLength;
  const copy = () => bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
  const asBuffer = data => (data instanceof ArrayBuffer ? data.slice(0) : new Uint8Array(data).slice().buffer);

  let nextId = 1;
  const trapped = { mupdf: null, rust: null };
  function call(engine, session, request) {
    const t0 = performance.now();
    let reply;
    try {
      reply = engines[engine].dispatch({ id: nextId++, ...request }, session).reply;
    } catch (err) {
      reply = { ok: false, error: `${request.op}: thrown ${err?.name ?? ""} ${err?.message ?? err}` };
    }
    const ms = r1(performance.now() - t0);
    if (!reply.ok) {
      raw(`${engine} ${request.op}`, reply.error);
      const cls = errorClass(reply.error);
      if (!trapped[engine] && (reply.reason === "worker" || /:(trap|engine-stopped)$/.test(cls))) trapped[engine] = record.stage;
    }
    return { reply, ms };
  }
  const outcome = ({ reply, ms }) => (reply.ok ? { ok: true, ms } : {
    ok: false, ms, error: errorClass(reply.error), hash: messageHash(reply.error),
    ...(token(reply.reason) ? { reason: token(reply.reason) } : {}),
  });

  const INFO_BOOLEANS = ["encrypted", "needsPassword", "repaired", "certified", "xfa", "openAction", "tagged", "printable"];
  const INFO_NUMBERS = ["signatures", "attachments", "versions"];
  function info(value) {
    if (!value || typeof value !== "object") return null;
    const out = {};
    for (const key of INFO_BOOLEANS) if (typeof value[key] === "boolean") out[key] = value[key];
    for (const key of INFO_NUMBERS) if (typeof value[key] === "number") out[key] = value[key];
    if (token(value.algorithm)) out.algorithm = value.algorithm;
    if (value.permissions && typeof value.permissions === "object") {
      out.permissions = {};
      for (const key of ["print", "copy", "edit", "annotate", "form"]) {
        if (typeof value.permissions[key] === "boolean") out.permissions[key] = value.permissions[key];
      }
    }
    return out;
  }

  const norm = text => (typeof text === "string" ? text.replace(/\s+/g, " ").trim() : "");
  const chars = text => [...text].length;
  function bagOverlap(a, b) {
    const count = new Map();
    let left = 0, right = 0, common = 0;
    for (const ch of a) if (ch !== " ") { count.set(ch, (count.get(ch) ?? 0) + 1); left++; }
    for (const ch of b) {
      if (ch === " ") continue;
      right++;
      const n = count.get(ch) ?? 0;
      if (n > 0) { common++; count.set(ch, n - 1); }
    }
    return Math.max(left, right) ? r3(common / Math.max(left, right)) : 1;
  }
  function compareText(a, b) {
    const [x, y] = [norm(a), norm(b)];
    const equal = x === y;
    return {
      equal,
      equalNoSpace: equal || x.replace(/ /g, "") === y.replace(/ /g, ""),
      chars: [chars(x), chars(y)],
      overlap: equal ? 1 : bagOverlap(x, y),
    };
  }
  function comparePixels(a, b, width, height) {
    const n = width * height;
    if (!(a instanceof Uint8Array) || !(b instanceof Uint8Array) || a.length < n * 4 || b.length < n * 4) return null;
    let exact = 0, near = 0, far = 0;
    for (let i = 0, p = 0; i < n; i++, p += 4) {
      const d = Math.max(Math.abs(a[p] - b[p]), Math.abs(a[p + 1] - b[p + 1]), Math.abs(a[p + 2] - b[p + 2]));
      if (d === 0) exact++;
      if (d <= 16) near++;
      if (d > 64) far++;
    }
    const pct = k => (n ? r3((100 * k) / n) : 100);
    return { exactPct: pct(exact), within16Pct: pct(near), over64Px: far };
  }
  const pixelsOf = reply => (reply.ok && reply.pixels instanceof ArrayBuffer ? new Uint8Array(reply.pixels) : null);

  /** Opens bytes on one engine and unlocks with the empty password, then (qpdf) the stated test passwords. */
  function open(engine, data, name) {
    const session = engines[engine].createSession();
    const opened = call(engine, session, { op: "open", pdf: asBuffer(data), name });
    const result = { ...outcome(opened) };
    let locked = false;
    let password = null;
    if (opened.reply.ok) {
      result.pageCount = opened.reply.pageCount;
      result.info = info(opened.reply.info);
      locked = !!opened.reply.info?.needsPassword;
      result.pages = Array.isArray(opened.reply.pages) ? opened.reply.pages : [];
      result.rotations = Array.isArray(opened.reply.rotations) ? opened.reply.rotations : [];
    }
    return { session, result, locked, password, reply: opened.reply };
  }
  function unlock(engine, state, candidates) {
    if (!state.locked) return;
    const tries = [{ password: "", script: null }, ...candidates];
    state.result.passwordTries = 0;
    for (const candidate of tries) {
      state.result.passwordTries++;
      const { reply } = call(engine, state.session, { op: "authenticate", password: candidate.password });
      if (reply.ok && reply.authenticated) {
        state.locked = false;
        state.password = candidate.password;
        state.result.unlocked = candidate.script === null ? "empty" : candidate.script === SAME ? "same-password" : "qpdf-test-script";
        if (candidate.script && candidate.script !== SAME) {
          state.result.unlockScript = candidate.script;
          state.result.unlockPasswordSha256 = sha256(candidate.password).slice(0, 16);
        }
        state.result.pageCount = reply.pageCount;
        state.result.info = info(reply.info);
        state.result.pages = Array.isArray(reply.pages) ? reply.pages : state.result.pages;
        state.result.rotations = Array.isArray(reply.rotations) ? reply.rotations : state.result.rotations;
        state.result.auth = token(reply.auth);
        return;
      }
      if (!reply.ok) break;
    }
    state.result.locked = true;
  }
  function geometry(a, b) {
    const n = Math.min(a.pages.length, b.pages.length);
    let sizes = 0, rotations = 0;
    for (let i = 0; i < n; i++) {
      const [p, q] = [a.pages[i], b.pages[i]];
      if (!(Math.abs(p.width - q.width) <= 0.01 && Math.abs(p.height - q.height) <= 0.01)) sizes++;
      if (a.rotations[i] !== b.rotations[i]) rotations++;
    }
    return { compared: n, sizesDiffer: sizes, rotationsDiffer: rotations };
  }
  const close = (engine, session) => call(engine, session, { op: "close" });
  const name = "document.pdf";

  // Open on both engines (Rust alone when a MuPDF stage hung or crashed in the first run).
  const skipMupdf = process.env.TPDF_CORPUS_SKIP_MUPDF === "1";
  if (skipMupdf) record.mupdfSkipped = true;
  emit("open-mupdf");
  const m = skipMupdf ? { reply: { ok: false }, result: { skipped: true }, locked: false } : open("mupdf", bytes, name);
  emit("open-rust");
  const r = open("rust", bytes, name);
  record.mupdf = { open: m.result };
  record.rust = { open: r.result };
  emit("authenticate-mupdf");
  unlock("mupdf", m, passwords);
  emit("authenticate-rust");
  unlock("rust", r, passwords);
  const mUsable = m.reply.ok && !m.locked;
  const rUsable = r.reply.ok && !r.locked;
  if (mUsable && rUsable) record.geometry = geometry(m.result, r.result);
  for (const state of [m, r]) {
    delete state.result.pages;
    delete state.result.rotations;
  }

  // Pages: first, middle, last of what both can show.
  const counts = [mUsable ? m.result.pageCount : null, rUsable ? r.result.pageCount : null].filter(n => Number.isInteger(n) && n > 0);
  const n = counts.length ? Math.min(...counts) : 0;
  const pages = n ? [...new Set([0, Math.floor((n - 1) / 2), n - 1])] : [];
  record.pages = [];
  const original = { mupdf: new Map(), rust: new Map() };
  for (const page of pages) {
    const row = { page };
    emit(`render-mupdf-${page}`);
    const a = mUsable ? call("mupdf", m.session, { op: "render", page, scale: SCALE }) : null;
    emit(`render-rust-${page}`);
    const b = rUsable ? call("rust", r.session, { op: "render", page, scale: SCALE }) : null;
    for (const [engine, result] of [["mupdf", a], ["rust", b]]) {
      if (!result) continue;
      const { reply } = result;
      row[engine] = outcome(result);
      if (reply.ok) {
        Object.assign(row[engine], {
          px: [reply.pixelWidth, reply.pixelHeight], renderMs: reply.renderMs, lines: Array.isArray(reply.lines) ? reply.lines.length : null,
          nonWhite: reply.nonWhite,
        });
        original[engine].set(page, { text: typeof reply.text === "string" ? reply.text : "", pixels: pixelsOf(reply), w: reply.pixelWidth, h: reply.pixelHeight });
      }
    }
    if (a?.reply.ok && b?.reply.ok) {
      row.dimsEqual = a.reply.pixelWidth === b.reply.pixelWidth && a.reply.pixelHeight === b.reply.pixelHeight;
      if (row.dimsEqual) Object.assign(row, comparePixels(pixelsOf(a.reply), pixelsOf(b.reply), a.reply.pixelWidth, a.reply.pixelHeight) ?? { pixelsUnreadable: true });
      row.text = compareText(a.reply.text, b.reply.text);
    }
    record.pages.push(row);
  }

  // text-all and search, as the app asks (each engine that can show the document; compared when both can).
  if (mUsable || rUsable) {
    emit("text-all-mupdf");
    const a = mUsable ? call("mupdf", m.session, { op: "text-all" }) : null;
    emit("text-all-rust");
    const b = rUsable ? call("rust", r.session, { op: "text-all" }) : null;
    record.textAll = { ...(a ? { mupdf: outcome(a) } : {}), ...(b ? { rust: outcome(b) } : {}) };
    if (a?.reply.ok && b?.reply.ok) {
      record.textAll.text = compareText(a.reply.text, b.reply.text);
      record.textAll.pagesRust = String(b.reply.text).split("\f").length;
      record.textAll.pagesMupdf = String(a.reply.text).split("\f").length;
    }
    const source = a?.reply.ok ? a.reply.text : b?.reply.ok ? b.reply.text : "";
    const derived = String(source).match(/\p{L}{4,}/u)?.[0]?.slice(0, 64);
    record.search = {};
    for (const [kind, query] of [["fixed", FIXED_QUERY], ["derived", derived]]) {
      if (!query) continue;
      emit(`search-${kind}-mupdf`);
      const x = mUsable ? call("mupdf", m.session, { op: "search", query, maxHits: MAX_HITS }) : null;
      emit(`search-${kind}-rust`);
      const y = rUsable ? call("rust", r.session, { op: "search", query, maxHits: MAX_HITS }) : null;
      const side = ({ reply, ms }) => (reply.ok ? { ok: true, ms, total: reply.total, truncated: !!reply.truncated } : outcome({ reply, ms }));
      record.search[kind] = { ...(x ? { mupdf: side(x) } : {}), ...(y ? { rust: side(y) } : {}) };
      if (x?.reply.ok && y?.reply.ok) {
        record.search[kind].equal = x.reply.total === y.reply.total && !!x.reply.truncated === !!y.reply.truncated;
        const pagesOf = reply => (Array.isArray(reply.hits) ? reply.hits.map(hit => hit.page).join(",") : "");
        record.search[kind].samePages = pagesOf(x.reply) === pagesOf(y.reply);
      }
    }
  }

  // Save on both; Rust's save validated, then reopened in MuPDF and in Rust.
  let rustSaved = null;
  if (rUsable) {
    emit("save-mupdf");
    const ms = mUsable ? call("mupdf", m.session, { op: "save" }) : null;
    if (ms) record.mupdf.save = { ...outcome(ms), ...(ms.reply.ok ? { bytes: ms.reply.bytes?.byteLength } : {}) };
    emit("save-rust");
    const rs = call("rust", r.session, { op: "save" });
    record.save = { rust: { ...outcome(rs) } };
    if (rs.reply.ok && rs.reply.bytes instanceof ArrayBuffer) {
      rustSaved = new Uint8Array(rs.reply.bytes).slice();
      record.save.rust.bytes = rustSaved.byteLength;
      emit("validate-rust");
      record.save.validate = outcome(call("rust", r.session, { op: "validate", pdf: asBuffer(rustSaved) }));

      emit("reopen-mupdf");
      const reopened = skipMupdf ? null : open("mupdf", rustSaved, name);
      if (reopened) {
      if (reopened.locked && r.password !== null) unlock("mupdf", reopened, [{ password: r.password, script: SAME }]);
      const back = { ...reopened.result };
      delete back.pages;
      delete back.rotations;
      record.save.reopenMupdf = back;
      const reference = mUsable ? m.result.pageCount : r.result.pageCount;
      if (reopened.reply.ok && !reopened.locked) {
        back.pageCountEqual = reopened.result.pageCount === reference;
        back.pagesCompared = [];
        for (const page of pages) {
          if (page >= reopened.result.pageCount) continue;
          emit(`reopen-mupdf-render-${page}`);
          const result = call("mupdf", reopened.session, { op: "render", page, scale: SCALE });
          const row = { page, ...outcome(result), ...(result.reply.ok ? { nonWhite: result.reply.nonWhite } : {}) };
          const before = original.mupdf.get(page);
          if (result.reply.ok && before) {
            row.text = compareText(before.text, result.reply.text).equal;
            row.dimsEqual = before.w === result.reply.pixelWidth && before.h === result.reply.pixelHeight;
            if (row.dimsEqual) Object.assign(row, comparePixels(before.pixels, pixelsOf(result.reply), before.w, before.h) ?? {});
          }
          back.pagesCompared.push(row);
        }
        back.repairedAfter = (() => {
          try {
            return !!reopened.session.doc?.asPDF()?.wasRepaired();
          } catch {
            return null;
          }
        })();
      }
      close("mupdf", reopened.session);
      }

      emit("reopen-rust");
      const again = open("rust", rustSaved, name);
      if (again.locked && r.password !== null) unlock("rust", again, [{ password: r.password, script: SAME }]);
      const rb = { ...again.result };
      delete rb.pages;
      delete rb.rotations;
      record.save.reopenRust = rb;
      if (again.reply.ok && !again.locked) {
        rb.pageCountEqual = again.result.pageCount === r.result.pageCount;
        rb.pagesCompared = [];
        for (const page of pages) {
          if (page >= again.result.pageCount) continue;
          emit(`reopen-rust-render-${page}`);
          const result = call("rust", again.session, { op: "render", page, scale: SCALE });
          const row = { page, ...outcome(result), ...(result.reply.ok ? { nonWhite: result.reply.nonWhite } : {}) };
          const before = original.rust.get(page);
          if (result.reply.ok && before) {
            row.text = compareText(before.text, result.reply.text).equal;
            row.dimsEqual = before.w === result.reply.pixelWidth && before.h === result.reply.pixelHeight;
            if (row.dimsEqual) Object.assign(row, comparePixels(before.pixels, pixelsOf(result.reply), before.w, before.h) ?? {});
          }
          rb.pagesCompared.push(row);
        }
      }
      close("rust", again.session);
    }
  }

  // Edit round trip: annot-add Square on page 0 (Rust; MuPDF for its refusal), save, reopen in MuPDF.
  if (rUsable && r.result.pageCount > 0) {
    emit("edit-mupdf");
    const size = r.reply.pages?.[0] ?? (m.reply.ok ? m.reply.pages?.[0] : null);
    const w = Number(size?.width) || 612, h = Number(size?.height) || 792;
    const rect = [r3(w * 0.3), r3(h * 0.3), r3(w * 0.6), r3(h * 0.45)];
    const request = { op: "annot-add", page: 0, type: "Square", rect, contents: "" };
    const squares = reply => (reply.ok && Array.isArray(reply.items) ? reply.items.filter(item => item.type === "Square").length : null);
    record.edit = {};
    let before = null;
    if (mUsable) {
      before = squares(call("mupdf", m.session, { op: "annots", page: 0 }).reply);
      record.edit.mupdf = outcome(call("mupdf", m.session, request));
    }
    emit("edit-rust");
    const rustBefore = squares(call("rust", r.session, { op: "annots", page: 0 }).reply);
    const added = call("rust", r.session, request);
    record.edit.rust = outcome(added);
    if (added.reply.ok) {
      record.edit.rustSquares = [rustBefore, squares(call("rust", r.session, { op: "annots", page: 0 }).reply)];
      emit("edit-save");
      const saved = call("rust", r.session, { op: "save" });
      record.edit.save = outcome(saved);
      if (saved.reply.ok && saved.reply.bytes instanceof ArrayBuffer) {
        const edited = new Uint8Array(saved.reply.bytes).slice();
        record.edit.save.bytes = edited.byteLength;
        emit("edit-validate");
        record.edit.validate = outcome(call("rust", r.session, { op: "validate", pdf: asBuffer(edited) }));
        emit("edit-reopen-mupdf");
        const reopened = skipMupdf ? null : open("mupdf", edited, name);
        if (reopened) {
        if (reopened.locked && r.password !== null) unlock("mupdf", reopened, [{ password: r.password, script: SAME }]);
        const back = { ...outcome({ reply: reopened.reply, ms: reopened.result.ms }) };
        if (reopened.reply.ok && !reopened.locked) {
          const after = squares(call("mupdf", reopened.session, { op: "annots", page: 0 }).reply);
          back.repaired = !!reopened.result.info?.repaired;
          back.squares = [before ?? rustBefore, after];
          back.present = after !== null && after === (before ?? rustBefore ?? 0) + 1;
          const shown = call("mupdf", reopened.session, { op: "render", page: 0, scale: SCALE });
          back.render = outcome(shown);
          try {
            back.repairedAfter = !!reopened.session.doc?.asPDF()?.wasRepaired();
          } catch {
            back.repairedAfter = null;
          }
        } else if (reopened.locked) {
          back.locked = true;
        }
        record.edit.reopenMupdf = back;
        close("mupdf", reopened.session);
        }
      }
    }
  }

  record.trapped = trapped;
  record.mupdfLog = warnings;
  record.ms = r1(performance.now() - started);
  emit("done");
}

// ---------------------------------------------------------------------------
// Classification of one JSONL entry

/** Classes that put a file on the failing list; the rest are recorded differences or shared refusals. */
const FAILURE_CLASSES = new Set([
  "timeout", "oom", "crash", "rust-trap", "open-differs", "password-differs", "pagecount-differs", "pagesize-differs",
  "render-dims", "render-error", "render-pixels", "render-ink", "text-differs", "text-space-differs", "textall-differs", "search-differs",
  "info-differs", "save-refused", "validate-refused", "save-unreadable", "save-repaired", "save-pagecount",
  "save-text-differs", "save-render-differs", "edit-failed",
  "save-mupdf-timeout", "save-mupdf-crash", "save-mupdf-oom", "rust-alone-timeout", "rust-alone-crash", "rust-alone-oom",
]);

/** A failed child whose last stage was not a Rust one gets a second run with Rust alone. */
const needsRustAlone = entry => entry.outcome !== "ok" && !entry.rustAlone && typeof entry.stage === "string"
  && entry.stage !== "load" && !/rust/.test(entry.stage);

/** What the Rust-alone run says about Rust by itself (no MuPDF comparison is possible there). */
function rustAloneClasses(alone, classes) {
  if (alone.outcome !== "ok") classes.add(`rust-alone-${alone.outcome}`);
  const rec = alone.record ?? {};
  if (rec.trapped?.rust) classes.add("rust-trap");
  if (rec.save?.validate && !rec.save.validate.ok) classes.add("validate-refused");
  const back = rec.save?.reopenRust;
  if (back && (!back.ok || back.locked)) classes.add("save-unreadable");
  if (back?.pageCountEqual === false) classes.add("save-pagecount");
  if ((back?.pagesCompared ?? []).some(row => row.text === false)) classes.add("save-text-differs");
}

export function classify(entry) {
  const classes = new Set();
  const details = {};
  if (entry.outcome !== "ok") {
    // A MuPDF stage that hung or crashed while Rust alone completes is MuPDF's, except on Rust's own save.
    const alone = entry.rustAlone;
    const stage = entry.stage ?? "";
    if (alone?.outcome === "ok" && !/rust/.test(stage)) {
      classes.add(/^(edit-)?reopen-mupdf/.test(stage) ? `save-mupdf-${entry.outcome}` : `mupdf-${entry.outcome}`);
    } else classes.add(entry.outcome);
    if (alone) rustAloneClasses(alone, classes);
  }
  const rec = entry.record ?? {};
  if (rec.trapped?.rust) classes.add("rust-trap");
  if (rec.trapped?.mupdf) classes.add("mupdf-trap");
  const mo = rec.mupdf?.open, ro = rec.rust?.open;
  if (mo && ro) {
    if (mo.ok !== ro.ok) {
      classes.add("open-differs");
      details.open = mo.ok ? "rust-refuses" : "rust-opens-mupdf-refuses";
    } else if (!mo.ok) {
      classes.add("both-refuse-open");
    } else {
      if (!!mo.locked !== !!ro.locked || !!mo.info?.needsPassword !== !!ro.info?.needsPassword) classes.add("password-differs");
      if (mo.locked && ro.locked) classes.add("locked");
      if (!mo.locked && !ro.locked) {
        if (mo.pageCount !== ro.pageCount) classes.add("pagecount-differs");
        if (!!mo.info?.repaired !== !!ro.info?.repaired) classes.add("repaired-differs");
        const fields = [];
        for (const key of ["encrypted", "signatures", "certified", "xfa", "attachments", "openAction", "tagged", "versions", "algorithm", "printable"]) {
          if (mo.info?.[key] !== ro.info?.[key]) fields.push(key);
        }
        for (const key of ["print", "copy", "edit", "annotate", "form"]) {
          if (mo.info?.permissions?.[key] !== ro.info?.permissions?.[key]) fields.push(`permissions.${key}`);
        }
        if (fields.length) details.info = fields;
        // `versions` (saved prior revisions) is a contract field the renderer never reads: recorded on its own.
        if (fields.includes("versions")) classes.add("versions-differs");
        if (fields.some(field => field !== "versions")) classes.add("info-differs");
      }
    }
  }
  if (rec.geometry && (rec.geometry.sizesDiffer || rec.geometry.rotationsDiffer)) classes.add("pagesize-differs");
  for (const row of rec.pages ?? []) {
    if (row.mupdf && row.rust) {
      if (row.mupdf.ok && !row.rust.ok) classes.add("render-error");
      if (!row.mupdf.ok && row.rust.ok) classes.add("render-error-mupdf-only");
      if (!row.mupdf.ok && !row.rust.ok) classes.add("render-error-both");
    }
    if (row.dimsEqual === false) classes.add("render-dims");
    if (typeof row.within16Pct === "number" && row.within16Pct < WITHIN16_MIN) classes.add(inkMismatch(row) ? "render-pixels" : "render-edges");
    else if (inkMismatch(row)) classes.add("render-ink");
    if (row.text && !row.text.equal) classes.add(row.text.equalNoSpace ? "text-space-differs" : "text-differs");
  }
  const ta = rec.textAll;
  if (ta) {
    if (ta.mupdf && ta.rust && ta.mupdf.ok !== ta.rust.ok) classes.add("textall-differs");
    else if (ta.text && !ta.text.equal) classes.add(ta.text.equalNoSpace ? "textall-space-differs" : "textall-differs");
  }
  for (const kind of ["fixed", "derived"]) {
    const s = rec.search?.[kind];
    if (s?.mupdf && s.rust && (s.mupdf.ok !== s.rust.ok || s.equal === false)) classes.add("search-differs");
  }
  const save = rec.save;
  // MuPDF refused or locked the source, or its WASM trapped on it (every later MuPDF call in the child then fails).
  const mupdfRefusedSource = !!mo && !mo.skipped && (!mo.ok || !!mo.locked || !!rec.trapped?.mupdf);
  if (save) {
    if (!save.rust.ok) {
      // A refusal MuPDF's engine.ts makes too (a signed document is read-only) is the contract, not a failure.
      if (rec.mupdf?.save && !rec.mupdf.save.ok && rec.mupdf.save.reason === save.rust.reason) classes.add("save-refused-both");
      else if (!rec.mupdf?.save && save.rust.reason === "read-only" && (ro?.info?.signatures > 0 || ro?.info?.certified)) classes.add("save-refused-read-only");
      else classes.add("save-refused");
    }
    if (save.validate && !save.validate.ok) classes.add("validate-refused");
    for (const [engine, back] of [["mupdf", save.reopenMupdf], ["rust", save.reopenRust]]) {
      if (!back) continue;
      // MuPDF failing on Rust's save of a source MuPDF could not read either keeps the source's damage.
      const asSource = engine === "mupdf" && mupdfRefusedSource;
      if (!back.ok || back.locked) {
        classes.add(asSource ? "save-unreadable-as-source" : "save-unreadable");
        (details.saveUnreadable ??= []).push(engine);
        continue;
      }
      if (engine === "mupdf" && (back.info?.repaired || back.repairedAfter)) classes.add(asSource ? "save-repaired-as-source" : "save-repaired");
      if (back.pageCountEqual === false) classes.add("save-pagecount");
      for (const row of back.pagesCompared ?? []) {
        if (row.text === false) classes.add("save-text-differs");
        const before = (rec.pages ?? []).find(page => page.page === row.page);
        // Only a page MuPDF drew from the source can show what the save changed.
        if (engine === "mupdf" && before?.mupdf?.ok && (row.ok === false || row.dimsEqual === false || (typeof row.within16Pct === "number" && row.within16Pct < WITHIN16_MIN) || inkMismatch(row, before))) {
          classes.add("save-render-differs");
        }
      }
    }
  }
  const edit = rec.edit;
  if (edit?.rust) {
    if (!edit.rust.ok) {
      const readOnly = edit.rust.reason === "read-only" && (ro?.info?.signatures > 0 || ro?.info?.certified || ro?.info?.permissions?.annotate === false);
      if (edit.mupdf && !edit.mupdf.ok) classes.add("edit-refused-both");
      // Without MuPDF's verdict, a read-only refusal Rust's own DocumentInfo explains is the contract.
      else if (!edit.mupdf && readOnly) classes.add("edit-refused-read-only");
      else {
        classes.add("edit-failed");
        details.edit = `add:${edit.rust.reason ?? edit.rust.error}`;
      }
    } else if (!edit.save?.ok) {
      classes.add("edit-failed");
      details.edit = "save";
    } else if ((!edit.reopenMupdf?.ok || edit.reopenMupdf.locked) && mupdfRefusedSource) {
      classes.add("edit-unverified-as-source");
    } else if (!edit.reopenMupdf?.ok || edit.reopenMupdf.locked) {
      classes.add("edit-failed");
      details.edit = "reopen";
    } else if (edit.reopenMupdf.repaired || edit.reopenMupdf.repairedAfter) {
      classes.add("edit-failed");
      details.edit = "repaired";
    } else if (!edit.reopenMupdf.present) {
      classes.add("edit-failed");
      details.edit = "annotation-missing";
    } else if (edit.validate && !edit.validate.ok) {
      classes.add("edit-failed");
      details.edit = "validate";
    }
  }
  const list = [...classes].sort();
  return { classes: list, failures: list.filter(name => FAILURE_CLASSES.has(name)), details };
}

// ---------------------------------------------------------------------------
// Summary

function readEntries(runDir) {
  const file = path.join(runDir, "results.jsonl");
  if (!fs.existsSync(file)) return [];
  const byHash = new Map();
  for (const line of fs.readFileSync(file, "utf8").split("\n")) {
    if (!line.trim()) continue;
    const entry = JSON.parse(line);
    byHash.set(entry.sha256, entry);
  }
  return [...byHash.values()];
}

function count(map, key, by = 1) {
  map[key] = (map[key] ?? 0) + by;
}

function summarize(runDir, meta) {
  const entries = readEntries(runDir);
  const out = {
    schema: "tpdf-corpus-summary/1",
    ...meta,
    thresholds: { renderWithin16PctMin: WITHIN16_MIN, renderInkOver64PctMax: INK_MAX, renderInkMinPx: INK_MIN_PX, scale: SCALE, pagesPerFile: "first, middle, last", searchMaxHits: MAX_HITS },
    notes: [
      "Each unique file (by sha256) runs once and is attributed to its first manifest location; duplicates are counted in the manifest.",
      "Text equality compares whitespace-normalised render text (Rust render reply text against engine.ts render text, MuPDF preserve-spans asText).",
      "Records hold only counts, booleans, fixed engine tokens, error classes and hashes of error messages; no document text or metadata.",
    ],
    files: entries.length,
    outcomes: {},
    classes: {},
    failureClasses: {},
    perSource: {},
  };
  const exact = [], within = [], inks = [], renderMs = { mupdf: [], rust: [] }, fileMs = [];
  const open = { mupdf: { ok: 0, locked: 0, refused: 0, repaired: 0, errors: {} }, rust: { ok: 0, locked: 0, refused: 0, repaired: 0, errors: {} }, bothOpen: 0, rustOnly: 0, mupdfOnly: 0, neither: 0 };
  const passwords = { emptyUnlocked: { mupdf: 0, rust: 0 }, qpdfScriptUnlocked: { mupdf: 0, rust: 0 }, qpdfFiles: [] };
  const render = { pagesCompared: 0, dimsEqual: 0, dimsDiffer: 0, errors: { rustOnly: 0, mupdfOnly: 0, both: 0 }, rustErrors: {}, buckets: {} };
  const text = { pagesCompared: 0, equal: 0, spaceOnly: 0, differs: 0, differsRustEmpty: 0, differsMupdfEmpty: 0, overlap: [], textAll: { compared: 0, equal: 0, spaceOnly: 0, differs: 0, refusedBoth: 0, statusDiffers: 0 }, search: { compared: 0, equal: 0, differs: 0 } };
  const save = { attempted: 0, ok: 0, refused: {}, mupdfRefusedToo: 0, validateOk: 0, validateRefused: {}, reopenMupdf: { ok: 0, failed: 0, repaired: 0, pageCountEqual: 0, pagesCompared: 0, textEqual: 0, within16Ok: 0 }, reopenRust: { ok: 0, failed: 0, pageCountEqual: 0, pagesCompared: 0, textEqual: 0 } };
  const edit = { attempted: 0, rustAdded: 0, refusedBoth: 0, refusedRustOnly: 0, saved: 0, reopened: 0, present: 0, repaired: 0, validateOk: 0, failed: 0, failures: {} };
  const info = {};
  const failing = [], everyFile = [], timeouts = [], crashes = [], slow = [];
  const errorHashes = { mupdf: {}, rust: {} };
  for (const entry of entries) {
    const { classes, failures, details } = classify(entry);
    const rec = entry.record ?? {};
    count(out.outcomes, entry.outcome);
    const source = (out.perSource[entry.source] ??= { files: 0, outcomes: {}, classes: {}, failing: 0, mupdfOpens: 0, rustOpens: 0, saveOk: 0, editPresent: 0, pagesCompared: 0, pagesWithin16Ok: 0, pagesTextEqual: 0 });
    source.files++;
    count(source.outcomes, entry.outcome);
    for (const name of classes) {
      count(out.classes, name);
      count(source.classes, name);
    }
    for (const name of failures) count(out.failureClasses, name);
    if (classes.length) everyFile.push({ source: entry.source, path: entry.path, sha256: entry.sha256, classes });
    if (failures.length) {
      source.failing++;
      failing.push({ source: entry.source, path: entry.path, sha256: entry.sha256, classes: failures, ...(Object.keys(details).length ? { details } : {}) });
    }
    const alone = entry.rustAlone ? { rustAlone: { outcome: entry.rustAlone.outcome, stage: entry.rustAlone.stage } } : {};
    if (entry.outcome === "timeout") timeouts.push({ source: entry.source, path: entry.path, sha256: entry.sha256, stage: entry.stage, ...alone });
    if (entry.outcome === "crash" || entry.outcome === "oom") crashes.push({ source: entry.source, path: entry.path, sha256: entry.sha256, outcome: entry.outcome, stage: entry.stage, exit: entry.exit, stderrClass: entry.stderrClass, ...alone });
    if (typeof rec.ms === "number") fileMs.push(rec.ms);
    for (const row of rec.pages ?? []) {
      if (row.rust?.ok && typeof row.rust.ms === "number") slow.push({ source: entry.source, path: entry.path, sha256: entry.sha256, page: row.page, rustMs: row.rust.ms, mupdfMs: row.mupdf?.ms ?? null });
    }
    for (const engine of ["mupdf", "rust"]) {
      const o = rec[engine]?.open;
      if (!o) continue;
      if (o.ok && !o.locked) open[engine].ok++;
      else if (o.ok) open[engine].locked++;
      else {
        open[engine].refused++;
        count(open[engine].errors, o.error);
        count(errorHashes[engine], `${o.error} ${o.hash}`);
      }
      if (o.info?.repaired) open[engine].repaired++;
      if (o.unlocked === "empty") passwords.emptyUnlocked[engine]++;
      if (o.unlocked === "qpdf-test-script") passwords.qpdfScriptUnlocked[engine]++;
      if (o.ok && !o.locked && engine === "mupdf") source.mupdfOpens++;
      if (o.ok && !o.locked && engine === "rust") source.rustOpens++;
    }
    const mOk = rec.mupdf?.open?.ok && !rec.mupdf.open.locked, rOk = rec.rust?.open?.ok && !rec.rust.open.locked;
    if (rec.mupdf?.open && rec.rust?.open) {
      if (mOk && rOk) open.bothOpen++;
      else if (rOk) open.rustOnly++;
      else if (mOk) open.mupdfOnly++;
      else open.neither++;
    }
    for (const engine of ["mupdf", "rust"]) {
      const o = rec[engine]?.open;
      if (o?.unlockScript) passwords.qpdfFiles.push({ path: entry.path, sha256: entry.sha256, engine, script: o.unlockScript, passwordSha256: o.unlockPasswordSha256 });
    }
    if (details.info) for (const field of details.info) count(info, field);
    for (const row of rec.pages ?? []) {
      if (row.mupdf?.ok && typeof row.mupdf.ms === "number") renderMs.mupdf.push(row.mupdf.ms);
      if (row.rust?.ok && typeof row.rust.ms === "number") renderMs.rust.push(row.rust.ms);
      if (row.mupdf && row.rust) {
        if (row.mupdf.ok && !row.rust.ok) {
          render.errors.rustOnly++;
          count(render.rustErrors, row.rust.error);
        } else if (!row.mupdf.ok && row.rust.ok) render.errors.mupdfOnly++;
        else if (!row.mupdf.ok && !row.rust.ok) render.errors.both++;
      }
      if (row.dimsEqual === true) render.dimsEqual++;
      if (row.dimsEqual === false) render.dimsDiffer++;
      if (typeof row.exactPct === "number") {
        render.pagesCompared++;
        source.pagesCompared++;
        exact.push(row.exactPct);
        within.push(row.within16Pct);
        const ink = inkPct(row);
        if (ink !== null) inks.push(ink);
        if (row.within16Pct >= WITHIN16_MIN) source.pagesWithin16Ok++;
        const bucket = row.within16Pct >= 99.9 ? ">=99.9" : row.within16Pct >= 99 ? "99-99.9" : row.within16Pct >= 98 ? "98-99" : row.within16Pct >= 95 ? "95-98" : row.within16Pct >= 90 ? "90-95" : row.within16Pct >= 50 ? "50-90" : "<50";
        count(render.buckets, bucket);
      }
      if (row.text) {
        text.pagesCompared++;
        if (row.text.equal) {
          text.equal++;
          source.pagesTextEqual++;
        } else if (row.text.equalNoSpace) text.spaceOnly++;
        else {
          text.differs++;
          if (row.text.chars?.[1] === 0) text.differsRustEmpty++;
          else if (row.text.chars?.[0] === 0) text.differsMupdfEmpty++;
          else text.overlap.push(row.text.overlap);
        }
      }
    }
    const ta = rec.textAll;
    if (ta?.mupdf && ta.rust) {
      if (ta.mupdf.ok && ta.rust.ok) {
        text.textAll.compared++;
        if (ta.text.equal) text.textAll.equal++;
        else if (ta.text.equalNoSpace) text.textAll.spaceOnly++;
        else text.textAll.differs++;
      } else if (!ta.mupdf.ok && !ta.rust.ok) text.textAll.refusedBoth++;
      else text.textAll.statusDiffers++;
    }
    for (const kind of ["fixed", "derived"]) {
      const s = rec.search?.[kind];
      if (!s?.mupdf || !s.rust) continue;
      text.search.compared++;
      if (s.equal) text.search.equal++;
      else text.search.differs++;
    }
    if (rec.save) {
      save.attempted++;
      if (rec.save.rust.ok) save.ok++;
      else {
        count(save.refused, rec.save.rust.reason ?? rec.save.rust.error);
        if (rec.mupdf?.save && !rec.mupdf.save.ok) save.mupdfRefusedToo++;
      }
      if (rec.save.validate?.ok) save.validateOk++;
      else if (rec.save.validate) count(save.validateRefused, rec.save.validate.reason ?? rec.save.validate.error);
      const bm = rec.save.reopenMupdf;
      if (bm) {
        if (bm.ok && !bm.locked) {
          save.reopenMupdf.ok++;
          if (bm.info?.repaired || bm.repairedAfter) save.reopenMupdf.repaired++;
          if (bm.pageCountEqual) save.reopenMupdf.pageCountEqual++;
          for (const row of bm.pagesCompared ?? []) {
            save.reopenMupdf.pagesCompared++;
            if (row.text) save.reopenMupdf.textEqual++;
            if (typeof row.within16Pct === "number" && row.within16Pct >= WITHIN16_MIN) save.reopenMupdf.within16Ok++;
          }
        } else save.reopenMupdf.failed++;
      }
      const br = rec.save.reopenRust;
      if (br) {
        if (br.ok && !br.locked) {
          save.reopenRust.ok++;
          if (br.pageCountEqual) save.reopenRust.pageCountEqual++;
          for (const row of br.pagesCompared ?? []) {
            save.reopenRust.pagesCompared++;
            if (row.text) save.reopenRust.textEqual++;
          }
        } else save.reopenRust.failed++;
      }
      if (rec.save.rust.ok && bm?.ok && !bm.locked && !(bm.info?.repaired || bm.repairedAfter) && bm.pageCountEqual) source.saveOk++;
    }
    if (rec.edit?.rust) {
      edit.attempted++;
      if (rec.edit.rust.ok) edit.rustAdded++;
      else if (classes.includes("edit-refused-both")) edit.refusedBoth++;
      else edit.refusedRustOnly++;
      if (rec.edit.save?.ok) edit.saved++;
      if (rec.edit.validate?.ok) edit.validateOk++;
      if (rec.edit.reopenMupdf?.ok && !rec.edit.reopenMupdf.locked) edit.reopened++;
      if (rec.edit.reopenMupdf?.present) {
        edit.present++;
        source.editPresent++;
      }
      if (rec.edit.reopenMupdf?.repaired || rec.edit.reopenMupdf?.repairedAfter) edit.repaired++;
      if (classes.includes("edit-failed")) {
        edit.failed++;
        count(edit.failures, details.edit);
      }
    }
  }
  const top = (map, n = 25) => Object.fromEntries(Object.entries(map).sort((a, b) => b[1] - a[1]).slice(0, n));
  // Clusters: failing files grouped by their exact set of failure classes, and examples per class
  // (smallest files first, so a case is easy to reproduce).
  const sizes = new Map(entries.map(entry => [entry.sha256, entry.bytes ?? 0]));
  const bySize = [...failing].sort((a, b) => sizes.get(a.sha256) - sizes.get(b.sha256));
  const example = file => ({ source: file.source, path: file.path, sha256: file.sha256, bytes: sizes.get(file.sha256) });
  const clusters = {};
  for (const file of bySize) {
    const key = file.classes.join("+");
    const cluster = (clusters[key] ??= { count: 0, sources: {}, examples: [] });
    cluster.count++;
    count(cluster.sources, file.source);
    if (cluster.examples.length < 5) cluster.examples.push(example(file));
  }
  const byClass = {};
  for (const file of everyFile.sort((a, b) => sizes.get(a.sha256) - sizes.get(b.sha256))) {
    for (const name of file.classes) {
      const group = (byClass[name] ??= { failure: FAILURE_CLASSES.has(name), count: 0, sources: {}, examples: [] });
      group.count++;
      count(group.sources, file.source);
      if (group.examples.length < 6) group.examples.push(example(file));
    }
  }
  Object.assign(out, {
    open,
    passwords,
    infoFieldsDiffering: top(info, 40),
    render: {
      ...render,
      exactPct: percentiles(exact),
      within16Pct: percentiles(within),
      inkOver64Pct: percentiles(inks),
      renderMs: { mupdf: percentiles(renderMs.mupdf), rust: percentiles(renderMs.rust) },
    },
    text: { ...text, overlap: percentiles(text.overlap) },
    save,
    edit,
    fileMs: percentiles(fileMs),
    openErrorHashes: { mupdf: top(errorHashes.mupdf, 15), rust: top(errorHashes.rust, 15) },
    slowestRustRenders: slow.sort((a, b) => b.rustMs - a.rustMs).slice(0, 15),
    timeouts,
    crashes,
    byClass: Object.fromEntries(Object.entries(byClass).sort((a, b) => b[1].count - a[1].count)),
    clusters: Object.fromEntries(Object.entries(clusters).sort((a, b) => b[1].count - a[1].count).slice(0, 40)),
    failing: failing.sort((a, b) => (a.source < b.source ? -1 : a.source > b.source ? 1 : a.path < b.path ? -1 : 1)),
  });
  return out;
}

// ---------------------------------------------------------------------------
// Parent: schedule children

/**
 * Passwords qpdf's test scripts state: literal --password= arguments, and the
 * (file, password) tables of encryption.test and check-encryption.test. Read
 * as text; nothing in the corpus runs. Passwords given by variables or kept in
 * password files are not followed.
 */
function qpdfPasswords(corpus) {
  const dir = path.join(corpus, "qpdf/qpdf/qtest");
  const found = new Map();
  if (!fs.existsSync(dir)) return [];
  for (const name of fs.readdirSync(dir).filter(name => name.endsWith(".test")).sort()) {
    const text = fs.readFileSync(path.join(dir, name), "utf8");
    for (const match of text.matchAll(/--password=(?:'([^']*)'|"([^"]*)"|([^\s"'\\]+))/g)) {
      const value = match[1] ?? match[2] ?? match[3];
      if (value && !value.includes("$") && !found.has(value)) found.set(value, name);
    }
    if (name === "encryption.test" || name === "check-encryption.test") {
      for (const match of text.matchAll(/\[\s*'([^']+)'\s*,\s*'([^']*)'/g)) {
        if (match[2] && !found.has(match[2])) found.set(match[2], name);
      }
      for (const match of text.matchAll(/\[\s*"([^"]+)"\s*,\s*"([^"]*)"/g)) {
        if (match[2] && !match[2].includes("$") && !found.has(match[2])) found.set(match[2], name);
      }
    }
  }
  return [...found].map(([password, script]) => ({ password, script }));
}

function git(...argv) {
  try {
    return execFileSync("git", ["-C", root, ...argv], { encoding: "utf8" }).trim();
  } catch {
    return null;
  }
}

function stderrClass(text) {
  if (/heap out of memory|Reached heap limit|Allocation failed/i.test(text)) return "v8-heap-oom";
  if (/RangeError: (WebAssembly\.Memory|Array buffer allocation)|Out of memory|OOM/i.test(text)) return "wasm-oom";
  if (/RuntimeError/.test(text)) return "wasm-trap";
  if (/Maximum call stack/.test(text)) return "stack-overflow";
  if (/Error/.test(text)) return "js-error";
  return text.trim() ? "other" : "none";
}

async function runParent() {
  const manifestPath = path.resolve(flag("--manifest", path.join(root, "engine/reference/corpus-20261015/manifest.json")));
  const corpus = path.resolve(flag("--corpus", process.env.TPDF_CORPUS ?? defaultCorpus()));
  const runsDir = flag("--runs", process.env.TPDF_CORPUS_RUNS);
  if (!runsDir) throw new Error("--runs DIR (outside the repository) is required");
  if (path.resolve(runsDir).startsWith(root + path.sep)) throw new Error("--runs must be outside the repository");
  const jobs = Math.max(1, Number(flag("--jobs", 4)));
  const timeoutS = Number(flag("--timeout", 60));
  const heapMb = Number(flag("--heap-mb", 2048));
  const rssMb = Number(flag("--rss-mb", 3072));
  const limit = Number(flag("--limit", Infinity));
  const sourceFilter = flag("--source", null);
  const only = flag("--only", null)?.split(",");
  const short = git("rev-parse", "--short", "HEAD") ?? "unknown";
  const runId = flag("--run-id", `${new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19)}-${short}`);
  const runDir = path.resolve(runsDir, runId);
  fs.mkdirSync(path.join(runDir, "progress"), { recursive: true });
  const resultsPath = path.join(runDir, "results.jsonl");

  const manifestText = fs.readFileSync(manifestPath, "utf8");
  const manifest = JSON.parse(manifestText);
  const dirs = new Map(manifest.sources.map(source => [source.name, source.dir]));
  let files = manifest.files.map(file => ({ ...file, ...file.locations[0] }));
  if (sourceFilter) files = files.filter(file => file.locations.some(location => location.source === sourceFilter));
  if (only) files = files.filter(file => only.some(prefix => file.sha256.startsWith(prefix)));
  files = files.filter(file => !file.locations.some(location => path.basename(location.path) === FORBIDDEN));
  const previous = has("--resume") ? readEntries(runDir) : [];
  const done = new Set(previous.map(entry => entry.sha256));
  files = files.filter(file => !done.has(file.sha256)).slice(0, limit);
  const byHash = new Map(manifest.files.map(file => [file.sha256, { ...file, ...file.locations[0] }]));
  const tasks = [
    ...files.map(file => ({ file, alone: false })),
    // A resumed run also gives earlier MuPDF-stage failures their Rust-alone run.
    ...previous.filter(needsRustAlone).filter(entry => byHash.has(entry.sha256)).map(entry => ({ file: byHash.get(entry.sha256), alone: true, entry })),
  ];
  const passwords = qpdfPasswords(corpus);
  const engineDirty = git("status", "--porcelain", "--", "engine/src", "engine/bridge", "engine/Cargo.toml", "engine/Cargo.lock", "src/renderer");
  const meta = {
    created: new Date().toISOString().slice(0, 10),
    engine: {
      commit: git("rev-parse", "HEAD"), short, branch: git("rev-parse", "--abbrev-ref", "HEAD"),
      engineSourcesDirty: !!engineDirty,
      wasmSha256: sha256(fs.readFileSync(path.join(root, "engine/web/pkg/tandem_rust_workflow_bg.wasm"))),
    },
    mupdf: JSON.parse(fs.readFileSync(path.join(root, "node_modules/mupdf/package.json"), "utf8")).version,
    node: process.version,
    manifest: { path: path.relative(root, manifestPath), sha256: sha256(manifestText), uniqueFiles: manifest.files.length },
    run: { id: runId, jobs, timeoutS, heapMb, rssMb, qpdfStatedPasswords: passwords.length, scripts: [...new Set(passwords.map(p => p.script))] },
  };
  fs.writeFileSync(path.join(runDir, "meta.json"), JSON.stringify(meta, null, 1));
  console.error(`run ${runId}: ${files.length} files (${tasks.length - files.length} Rust-alone reruns), ${jobs} jobs, ${timeoutS} s timeout -> ${runDir}`);

  const started = Date.now();
  const running = new Map();
  let next = 0, finished = 0;
  const results = fs.openSync(resultsPath, "a");

  const monitor = setInterval(() => {
    const pids = [...running.keys()];
    if (!pids.length) return;
    execFile("ps", ["-o", "pid=,rss=", "-p", pids.join(",")], (err, stdout) => {
      if (err && !stdout) return;
      for (const line of String(stdout).trim().split("\n")) {
        const [pid, rss] = line.trim().split(/\s+/).map(Number);
        const job = running.get(pid);
        if (!job) continue;
        job.peakRssMb = Math.max(job.peakRssMb, Math.round(rss / 1024));
        if (rss / 1024 > rssMb && !job.killedFor) {
          job.killedFor = "oom";
          job.child.kill("SIGKILL");
        }
      }
    });
  }, 1000);

  await new Promise(resolve => {
    const launch = () => {
      while (running.size < jobs && next < tasks.length) {
        const task = tasks[next++];
        const { file } = task;
        const absolute = path.join(corpus, dirs.get(file.source), file.path);
        const progress = path.join(runDir, "progress", `${file.sha256}${task.alone ? "-rust" : ""}.json`);
        fs.rmSync(progress, { force: true });
        const child = spawn(process.execPath, [`--max-old-space-size=${heapMb}`, SELF, "--child", absolute], {
          cwd: root,
          stdio: ["ignore", "ignore", "pipe"],
          env: {
            ...process.env,
            TPDF_CORPUS_PROGRESS: progress,
            TPDF_CORPUS_PASSWORDS: file.source === "qpdf" ? JSON.stringify(passwords) : "[]",
            TPDF_CORPUS_SKIP_MUPDF: task.alone ? "1" : "",
          },
        });
        const job = { child, file, progress, started: Date.now(), stderr: "", peakRssMb: 0, killedFor: null };
        running.set(child.pid, job);
        child.stderr.on("data", chunk => {
          job.stderr = (job.stderr + chunk).slice(-8192);
        });
        job.timer = setTimeout(() => {
          if (!job.killedFor) {
            job.killedFor = "timeout";
            child.kill("SIGKILL");
          }
        }, timeoutS * 1000);
        child.on("close", (code, signal) => {
          clearTimeout(job.timer);
          running.delete(child.pid);
          let record = null;
          try {
            record = JSON.parse(fs.readFileSync(progress, "utf8"));
          } catch {
            record = null;
          }
          fs.rmSync(progress, { force: true });
          const complete = code === 0 && record?.stage === "done";
          const klass = stderrClass(job.stderr);
          const outcome = job.killedFor ?? (complete ? "ok" : /oom/.test(klass) ? "oom" : "crash");
          const run = {
            outcome, stage: record?.stage ?? null, wallMs: Date.now() - job.started,
            peakRssMb: job.peakRssMb, ...(outcome === "ok" ? {} : { exit: code, signal, stderrClass: klass }), record,
          };
          const entry = task.alone ? { ...task.entry, rustAlone: run } : {
            sha256: file.sha256, source: file.source, path: file.path, locations: file.locations.length, bytes: file.bytes,
            signature: file.signature, ...run,
          };
          fs.writeSync(results, JSON.stringify(entry) + "\n");
          finished++;
          if (finished % 50 === 0 || outcome !== "ok" || task.alone) {
            const rate = finished / ((Date.now() - started) / 1000);
            const what = task.alone ? `rust-alone ${outcome}` : outcome === "ok" ? "" : outcome;
            console.error(`${finished}/${tasks.length} ${what ? `${what} ${file.source}/${file.path} (${run.stage})` : ""} ${rate.toFixed(2)} files/s`);
          }
          // A MuPDF-stage failure gets a Rust-alone run, so a MuPDF hang does not hide Rust's results.
          if (!task.alone && needsRustAlone(entry)) tasks.push({ file, alone: true, entry });
          if (running.size === 0 && next >= tasks.length) resolve();
          else launch();
        });
      }
      if (running.size === 0 && next >= tasks.length) resolve();
    };
    launch();
  });
  clearInterval(monitor);
  fs.closeSync(results);
  const wallS = Math.round((Date.now() - started) / 1000);
  const wall = fs.existsSync(path.join(runDir, "wall.json")) ? JSON.parse(fs.readFileSync(path.join(runDir, "wall.json"), "utf8")) : { segments: [] };
  wall.segments.push({ files: files.length, tasks: tasks.length, wallS, endedAt: new Date().toISOString() });
  fs.writeFileSync(path.join(runDir, "wall.json"), JSON.stringify(wall));
  console.error(`finished ${tasks.length} runs in ${wallS} s`);
  if (!has("--no-summary")) writeSummary(runDir);
}

function writeSummary(runDir) {
  const meta = JSON.parse(fs.readFileSync(path.join(runDir, "meta.json"), "utf8"));
  const wall = fs.existsSync(path.join(runDir, "wall.json")) ? JSON.parse(fs.readFileSync(path.join(runDir, "wall.json"), "utf8")) : { segments: [] };
  meta.run.wallS = wall.segments.reduce((sum, segment) => sum + segment.wallS, 0);
  meta.run.segments = wall.segments.length;
  const summary = summarize(runDir, meta);
  const target = path.resolve(flag("--summary", path.join(root, `engine/reference/corpus-20261015/summary-${meta.engine.short}.json`)));
  // One failing file per line keeps the summary diffable.
  const { failing, ...rest } = summary;
  const head = JSON.stringify(rest, null, 1).replace(/\n\}$/, "");
  fs.writeFileSync(target, `${head},\n "failing": [\n${failing.map(file => `  ${JSON.stringify(file)}`).join(",\n")}\n ]\n}\n`);
  JSON.parse(fs.readFileSync(target, "utf8"));
  console.error(`summary: ${path.relative(root, target)} (${summary.files} files)`);
}

if (has("--child")) {
  await runChild(path.resolve(flag("--child")));
  process.exit(0);
} else if (has("--summarize")) {
  writeSummary(path.resolve(flag("--summarize")));
} else if (process.argv[1] && path.resolve(process.argv[1]) === SELF) {
  await runParent();
}
