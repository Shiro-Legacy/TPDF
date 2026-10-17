#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
// Reruns the M10 fixed benchmark on the Rust engine and fails (exit 1) when a
// Rust p90 exceeds its frozen budget (engine/reference/bench-20261017/budgets.json).
// Timing-dependent, so it is not one of the default gates: run it by hand on
// the named machine (or one at least as fast), idle, after
// engine/tools/build-workflow.sh and `npm run build`:
//
//   node engine/tools/bench/check-budgets.mjs --out DIR [--budgets FILE] [--docs id,...] [any bench.mjs option]
//   node engine/tools/bench/check-budgets.mjs --check RESULTS [--budgets FILE]     (no rerun)
//
// DIR is outside the repository; its results.json is what was checked.
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { DEFAULT_BUDGETS, check } from "./budgets.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../../..");
const args = process.argv.slice(2);
const take = name => {
  const at = args.indexOf(name);
  if (at < 0) return undefined;
  const [, value] = args.splice(at, 2);
  return value;
};
const budgetsFile = take("--budgets") ?? DEFAULT_BUDGETS;
let resultsFile = take("--check");
const budgets = JSON.parse(fs.readFileSync(budgetsFile, "utf8"));

if (!resultsFile) {
  const out = args[args.indexOf("--out") + 1];
  if (!args.includes("--out") || !out) {
    console.error("usage: check-budgets.mjs --out DIR [--budgets FILE] [bench.mjs options] | --check RESULTS [--budgets FILE]");
    process.exit(2);
  }
  // Rust only; the documents that have budgets, or those --docs names (and only their budgets are checked).
  const named = args.includes("--docs") ? args[args.indexOf("--docs") + 1].split(",") : null;
  if (named) budgets.budgets = Object.fromEntries(Object.entries(budgets.budgets).filter(([doc]) => named.includes(doc)));
  const docs = named ? [] : ["--docs", Object.keys(budgets.budgets).join(",")];
  const run = spawnSync(process.execPath, [path.join(here, "bench.mjs"), "--engines", "rust", ...docs, ...args], { cwd: root, stdio: "inherit" });
  if (run.status !== 0) process.exit(run.status ?? 1);
  resultsFile = path.join(out, "results.json");
}

const results = JSON.parse(fs.readFileSync(resultsFile, "utf8"));
const { over, missing } = check(results, budgets);
for (const doc of missing) console.log(`MISSING ${doc}: no Rust results`);
for (const row of over) console.log(`OVER ${row.doc} ${row.metric} ${row.kind}: p90 ${row.p90} > budget ${row.budget}`);
const errors = Object.entries(results.results).flatMap(([doc, engines]) => (engines.rust?.errors ?? []).map(error => `${doc}: ${error}`));
for (const error of errors) console.log(`ERROR ${error}`);
const failed = over.length + missing.length + errors.length;
console.log(failed
  ? `FAIL: ${over.length} over budget, ${missing.length} missing, ${errors.length} errors (${resultsFile})`
  : `PASS: every Rust p90 within budget (${resultsFile})`);
process.exitCode = failed ? 1 : 0;
