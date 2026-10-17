#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
// Budgets for the M10 fixed benchmark (bench.mjs).
//
//   node engine/tools/bench/budgets.mjs freeze RESULTS BUDGETS   derive budgets from a results.json
//   node engine/tools/bench/budgets.mjs check RESULTS [BUDGETS]   exit 1 when a Rust p90 exceeds its budget
//   node engine/tools/bench/budgets.mjs compare RESULTS           Rust against MuPDF medians, as Markdown
//
// A budget is 1.5 x the Rust p90, cold and warm separately, rounded up to two
// significant figures, and never below the metric's floor: timings under a few
// milliseconds and memory growth near zero are noise, not signal.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");
export const DEFAULT_BUDGETS = path.join(root, "engine/reference/bench-20261017/budgets.json");
export const FACTOR = 1.5;

/** The budgeted metrics and their floors (ms, or MiB for names ending in Mb). */
export const FLOORS = {
  open: 10, firstPage: 10, openFirstPage: 10, "render0.5": 10, render1: 10, render2: 10, render4: 10,
  thumbnails: 10, search: 10, edit: 10, save: 10, reopen: 10, reopenFirstPage: 10, cycles: 20,
  engineLoad: 50, peakRssMb: 64, wasmMb: 32, historyWasmMb: 16, historyJsMb: 16, growthWasmMb: 16, growthJsMb: 16,
  growthRssMb: 32,
};

/** Rounded up to two significant figures. */
export function roundUp(value) {
  if (!(value > 0)) return 0;
  const step = 10 ** (Math.floor(Math.log10(value)) - 1);
  return Math.ceil(value / step - 1e-9) * step;
}

export function freeze(results) {
  const budgets = {};
  for (const [doc, engines] of Object.entries(results.results)) {
    const metrics = engines.rust?.metrics;
    if (!metrics) continue;
    budgets[doc] = {};
    for (const [metric, floor] of Object.entries(FLOORS)) {
      const entry = metrics[metric];
      if (!entry) continue;
      const out = {};
      for (const kind of ["cold", "warm"]) {
        if (entry[kind]) out[kind] = Math.max(floor, roundUp(FACTOR * entry[kind].p90));
      }
      if (Object.keys(out).length) budgets[doc][metric] = out;
    }
  }
  return {
    schema: "tpdf-bench-budgets/1",
    created: new Date().toISOString().slice(0, 10),
    rule: `budget = ${FACTOR} x the Rust p90 (cold and warm separately), rounded up to two significant figures, at least the metric's floor; ms, or MiB for names ending in Mb`,
    measuredOn: { machine: results.machine, versions: results.versions, results: "results.json" },
    floors: FLOORS,
    budgets,
  };
}

/** Every Rust p90 above its budget: [{doc, metric, kind, p90, budget}]. */
export function check(results, budgets) {
  const over = [];
  const missing = [];
  for (const [doc, metrics] of Object.entries(budgets.budgets)) {
    const measured = results.results[doc]?.rust?.metrics;
    if (!measured) {
      missing.push(doc);
      continue;
    }
    for (const [metric, kinds] of Object.entries(metrics)) {
      for (const [kind, budget] of Object.entries(kinds)) {
        const p90 = measured[metric]?.[kind]?.p90;
        if (typeof p90 === "number" && p90 > budget) over.push({ doc, metric, kind, p90, budget });
      }
    }
  }
  return { over, missing };
}

const COMPARED = ["open", "firstPage", "openFirstPage", "render1", "render4", "thumbnails", "search", "edit", "save", "reopen", "cycleMedian", "peakRssMb", "wasmMb"];

/** Markdown: Rust/MuPDF medians per document (warm, and cold for open/first page and peak memory) and the slow list. */
export function compare(results, factor = FACTOR) {
  const lines = [];
  const slow = [];
  for (const kind of ["cold", "warm"]) {
    lines.push(`\n${kind} medians, Rust / MuPDF (ms, MiB for memory)\n`);
    lines.push(`| doc | ${COMPARED.join(" | ")} |`);
    lines.push(`|---|${COMPARED.map(() => "---").join("|")}|`);
    for (const [doc, engines] of Object.entries(results.results)) {
      const cells = COMPARED.map(metric => {
        const rust = engines.rust?.metrics?.[metric]?.[kind]?.median;
        const mupdf = engines.mupdf?.metrics?.[metric]?.[kind]?.median;
        if (typeof rust !== "number") return "-";
        if (typeof mupdf !== "number") return `${rust}`;
        // Sub-millisecond medians are timer noise; they are listed, never flagged.
        const flagged = rust > factor * mupdf && (metric.endsWith("Mb") ? rust - mupdf >= 8 : rust >= 2);
        if (flagged) slow.push({ doc, metric, kind, rust, mupdf, ratio: Math.round((rust / mupdf) * 10) / 10 });
        return `${rust} / ${mupdf}${flagged ? " **" : ""}`;
      });
      lines.push(`| ${doc} | ${cells.join(" | ")} |`);
    }
  }
  lines.push(`\nRust slower than MuPDF by more than ${factor}x (median, Rust at least 2 ms, or 8 MiB more memory):\n`);
  for (const row of slow.sort((a, b) => b.ratio - a.ratio)) {
    lines.push(`- ${row.doc} ${row.metric} (${row.kind}): ${row.rust} vs ${row.mupdf}, ${row.ratio}x`);
  }
  return { markdown: lines.join("\n"), slow };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [command, resultsFile, other] = process.argv.slice(2);
  const read = file => JSON.parse(fs.readFileSync(file, "utf8"));
  if (command === "freeze" && resultsFile && other) {
    fs.writeFileSync(other, `${JSON.stringify(freeze(read(resultsFile)), null, 1)}\n`);
    console.log(`wrote ${other}`);
  } else if (command === "check" && resultsFile) {
    const { over, missing } = check(read(resultsFile), read(other ?? DEFAULT_BUDGETS));
    for (const doc of missing) console.log(`MISSING ${doc}: no Rust results`);
    for (const row of over) console.log(`OVER ${row.doc} ${row.metric} ${row.kind}: p90 ${row.p90} > budget ${row.budget}`);
    console.log(over.length || missing.length ? `FAIL: ${over.length} over budget, ${missing.length} missing` : "PASS: every Rust p90 within budget");
    process.exitCode = over.length || missing.length ? 1 : 0;
  } else if (command === "compare" && resultsFile) {
    console.log(compare(read(resultsFile)).markdown);
  } else {
    console.error("usage: budgets.mjs freeze RESULTS BUDGETS | check RESULTS [BUDGETS] | compare RESULTS");
    process.exitCode = 2;
  }
}
