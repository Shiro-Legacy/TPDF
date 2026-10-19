/**
 * Phase 1 UX quick wins (docs/rust-and-ux-plan.md §3, §4 row 1): static checks of the shell.
 * The drive and real-window checks live in tests/p1-ux-drive.test.mjs and docs/ux-audit/after/.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';

const root = fileURLToPath(new URL('..', import.meta.url));
const css = () => readFile(join(root, 'src/renderer/styles.css'), 'utf8');

/** The body of the first rule whose selector line is exactly `selector`. */
function rule(text, selector) {
  const at = text.indexOf(`\n${selector} {`);
  assert.ok(at >= 0, `no rule ${selector}`);
  return text.slice(at, text.indexOf('}', at));
}

test('X1: a tool row wraps instead of scrolling its tools out of sight', async () => {
  const text = await css();
  const ctx = rule(text, '.ctx');
  assert.match(ctx, /flex-wrap: wrap;/);
  assert.doesNotMatch(ctx, /overflow/);
  assert.match(rule(text, '#page-tools'), /flex-wrap: wrap;/);
  // The spacer before the Inspector button must not take half of the row's width.
  assert.match(rule(text, '.bar-mode > .grow'), /flex: 0 0 0;/);
});

test('X9: a hint is never squeezed into a narrow column, and the status bar keeps its row', async () => {
  const text = await css();
  assert.match(rule(text, '.ctx .hint'), /min-width: 14rem;/);
  for (const [selector, row] of [['#titlebar', 1], ['.toolbar', 2], ['#field-bar', 3], ['.notices', 4], ['.app', 5], ['.statusbar', 6]]) {
    assert.match(text, new RegExp(`\\n${selector.replace('.', '\\.')} \\{ grid-row: ${row}; \\}`), selector);
  }
});
