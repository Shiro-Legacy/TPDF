// SPDX-License-Identifier: AGPL-3.0-or-later
// node:test reporter for engine/tools/op-evidence.mjs: one JSON line per test
// result (file relative to the repository, name, nesting, outcome, skip or todo
// reason, duration, the error's message) and one per run diagnostic (the
// tests/pass/fail/skipped totals). Use beside the spec reporter:
//   node --test --test-reporter=spec --test-reporter-destination=stdout \
//     --test-reporter=./engine/tools/op-evidence-reporter.mjs --test-reporter-destination=<file> ...
import { relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../..', import.meta.url));

function where(file) {
  if (typeof file !== 'string') return null;
  const path = file.startsWith('file:') ? fileURLToPath(file) : file;
  return relative(root, path).split('\\').join('/');
}

function message(error) {
  if (!error) return undefined;
  const parts = [];
  for (let at = error, depth = 0; at && depth < 4; at = at.cause, depth++) {
    const text = typeof at === 'object' ? at.message ?? String(at) : String(at);
    if (text && !parts.includes(text)) parts.push(text);
  }
  return parts.join(' | ').slice(0, 4000) || undefined;
}

export default async function* opEvidenceReporter(source) {
  for await (const event of source) {
    const { type, data } = event;
    if (type === 'test:pass' || type === 'test:fail') {
      const details = data.details ?? {};
      const outcome = type === 'test:fail' ? 'fail' : data.skip !== undefined ? 'skip' : data.todo !== undefined ? 'todo' : 'pass';
      yield JSON.stringify({
        kind: 'test',
        file: where(data.file),
        name: data.name,
        nesting: data.nesting,
        line: data.line,
        type: details.type ?? 'test',
        outcome,
        reason: typeof data.skip === 'string' ? data.skip : typeof data.todo === 'string' ? data.todo : undefined,
        durationMs: typeof details.duration_ms === 'number' ? Math.round(details.duration_ms) : undefined,
        failureType: details.error?.failureType,
        error: type === 'test:fail' ? message(details.error) : undefined,
      }) + '\n';
    } else if (type === 'test:diagnostic' && data.nesting === 0 && !data.file) {
      yield JSON.stringify({ kind: 'diagnostic', message: data.message }) + '\n';
    }
  }
}
