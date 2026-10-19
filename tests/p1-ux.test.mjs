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

test('X4: a normal launch shows the start screen; a smoke, a drive or --sample opens the sample', async () => {
  const main = await readFile(join(root, 'src/main.ts'), 'utf8');
  assert.match(main, /const startWithSample = smoke \|\| sampleArg;/);
  assert.match(main, /content="\$\{startWithSample \? fixture : ''\}"/);
  const renderer = await readFile(join(root, 'src/renderer/renderer.ts'), 'utf8');
  assert.match(renderer, /if \(fixture === ""\) \{ await showStartScreen\(\); return; \}/);
  const html = await readFile(join(root, 'src/renderer/index.html'), 'utf8');
  for (const id of ['start-screen', 'start-open', 'start-new', 'start-images', 'start-recent-list', 'start-drop']) assert.match(html, new RegExp(`id="${id}"`), id);
});

test('X4: recent files keep the newest eight, drop a missing file, and persist without paths reaching the renderer', async () => {
  const { mkdtemp, rm } = await import('node:fs/promises');
  const { tmpdir } = await import('node:os');
  const { createRecentStore, MAX_RECENT } = await import('../build/recent.js');
  const dir = await mkdtemp(join(tmpdir(), 'tpdf-recent-'));
  try {
    const file = join(dir, 'recent-files.json');
    const store = createRecentStore(file);
    for (let i = 0; i < MAX_RECENT + 2; i++) await store.add(join(dir, `f${i}.pdf`));
    await store.add(join(dir, 'f3.pdf'));
    await store.add('relative.pdf'); // never a relative path
    const listed = store.list();
    assert.equal(listed.length, MAX_RECENT);
    assert.deepEqual(listed[0], { name: 'f3.pdf', folder: dir.split(/[\\/]/).pop() });
    assert.equal(listed[1].name, `f${MAX_RECENT + 1}.pdf`);
    assert.ok(listed.every(entry => !entry.name.includes('/') && !entry.folder.includes('/')));
    await store.remove(join(dir, 'f3.pdf'));
    const again = createRecentStore(file); // read back from disk
    assert.equal(again.list().length, MAX_RECENT - 1);
    assert.equal(again.path(0), join(dir, `f${MAX_RECENT + 1}.pdf`));
    assert.equal(again.path(99), undefined);
    assert.equal(createRecentStore(null).list().length, 0);
  } finally { await rm(dir, { recursive: true, force: true }); }
});
