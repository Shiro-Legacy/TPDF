#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
// The form ops, Rust against engine.ts on MuPDF: the `fields` listing of
// every page (30 at most) of every fixture, compared as values, and the op
// sequences of reference/app-forms-20261015/sequences.json, whose replies
// (errors included) and listings after each step must agree, object numbers
// aside (they are mapped by first appearance).
// usage (repo root, after npm run build, npm run build:oracle and tools/build-workflow.sh):
//   node engine/oracle/app-forms-diff.mjs [result.json]
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const mupdf = (await import(path.join(root, "node_modules/mupdf/dist/mupdf.js"))).default;
const { createEngine: createMupdf } = await import(path.join(root, "build-oracle/engine/oracle/app-engine/engine.js"));
const { createEngine: createRust } = await import(path.join(root, "tests/rust-engine.mjs"));
const ab = (b) => b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength);
const canon = (v) => JSON.stringify(v, (_k, x) => (x && typeof x === "object" && !Array.isArray(x)
  ? Object.fromEntries(Object.keys(x).sort().map((key) => [key, x[key]])) : x));
const engines = () => [createMupdf(mupdf), createRust(mupdf)];

function fixtures(dir) {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) return entry.name.startsWith(".") ? [] : fixtures(full);
    return entry.name.endsWith(".pdf") && entry.name !== "SBI05131482.pdf" ? [full] : [];
  });
}

const listing = { same: [], different: [] };
for (const file of fixtures(path.join(root, "fixtures")).sort()) {
  const results = engines().map((engine) => {
    const session = engine.createSession();
    const dispatch = (request) => engine.dispatch({ id: 1, ...request }, session).reply;
    const opened = dispatch({ op: "open", name: "x.pdf", pdf: ab(fs.readFileSync(file)) });
    if (!opened.ok) return { error: opened.error };
    if (session.locked) dispatch({ op: "authenticate", password: "pdf4all-user" });
    const count = dispatch({ op: "page-ids" }).ids?.length ?? 1;
    return Array.from({ length: Math.min(count, 30) }, (_v, page) => {
      const reply = dispatch({ op: "fields", page });
      return reply.ok ? { tabs: reply.tabs, items: reply.items } : { error: reply.error };
    });
  });
  (canon(results[0]) === canon(results[1]) ? listing.same : listing.different).push(path.relative(root, file));
}

const { sequences } = JSON.parse(fs.readFileSync(path.join(root, "engine/reference/app-forms-20261015/sequences.json"), "utf8"));
const ops = { same: [], different: [] };
for (const sequence of sequences) {
  const logs = engines().map((engine) => {
    const session = engine.createSession();
    const dispatch = (request) => engine.dispatch({ id: 1, ...request }, session).reply;
    const log = [dispatch({ op: "open", name: "x.pdf", pdf: ab(fs.readFileSync(path.join(root, "fixtures", sequence.file))) }).ok];
    for (const step of sequence.steps) {
      const request = { ...step };
      for (const [key, value] of Object.entries(request)) {
        if (typeof value !== "string" || !value.startsWith("@")) continue;
        const [name, on] = value.slice(1).split(":");
        const items = dispatch({ op: "fields", page: request.page ?? 0 }).items ?? [];
        request[key] = items.find((item) => item.name === name && (on === undefined || item.on === on))?.id ?? 999999;
      }
      const { pages: _pages, rotations: _rotations, pageCount: _count, ...reply } = dispatch(request);
      log.push([step.op, reply]);
      for (const page of sequence.pages ?? [0]) log.push(["fields", dispatch({ op: "fields", page }).items]);
    }
    // Object numbers differ (MuPDF allocates more objects): map them by first appearance.
    const numbers = new Map();
    const number = (n) => (numbers.has(n) ? numbers.get(n) : (numbers.set(n, `#${numbers.size}`), numbers.get(n)));
    const walk = (x) => {
      if (Array.isArray(x)) return x.forEach(walk);
      if (!x || typeof x !== "object") return;
      for (const [key, value] of Object.entries(x)) {
        if ((key === "id" || key === "widgetId") && typeof value === "number" && value !== 1) x[key] = number(value);
        else if (key === "widgetIds") x[key] = value.map(number);
        else walk(value);
      }
    };
    walk(log);
    return log.map(canon);
  });
  const step = logs[0].findIndex((entry, index) => entry !== logs[1][index]);
  if (step < 0 && logs[0].length === logs[1].length) ops.same.push(sequence.name);
  else ops.different.push({ sequence: sequence.name, step, mupdf: logs[0][step]?.slice(0, 600), rust: logs[1][step]?.slice(0, 600) });
}

const result = { date: "2026-10-15", fixturesListing: listing, sequences: ops };
if (process.argv[2]) fs.writeFileSync(process.argv[2], `${JSON.stringify(result, null, 1)}\n`);
console.log(`listing: ${listing.same.length} same, ${listing.different.length} different; sequences: ${ops.same.length} same, ${ops.different.length} different`);
