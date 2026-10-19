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
    assert.deepEqual({ name: listed[0].name, folder: listed[0].folder }, { name: 'f3.pdf', folder: dir.split(/[\\/]/).pop() });
    assert.equal(listed[1].name, `f${MAX_RECENT + 1}.pdf`);
    assert.ok(listed.every(entry => !entry.name.includes('/') && !entry.folder.includes('/') && !entry.id.includes('/')));
    await store.remove(join(dir, 'f3.pdf'));
    const again = createRecentStore(file); // read back from disk
    assert.equal(again.list().length, MAX_RECENT - 1);
    assert.equal(again.path(again.list()[0].id), join(dir, `f${MAX_RECENT + 1}.pdf`));
    assert.equal(again.path('no-such-id'), undefined);
    assert.equal(createRecentStore(null).list().length, 0);
  } finally { await rm(dir, { recursive: true, force: true }); }
});

test('review fix 3: a recent entry is opened by its id, so a list drawn before an entry was dropped never opens another file', async () => {
  const { createRecentStore } = await import('../build/recent.js');
  const store = createRecentStore(null);
  for (const name of ['c.pdf', 'b.pdf', 'a.pdf']) await store.add(`/docs/${name}`);
  const drawn = store.list(); // a.pdf, b.pdf, c.pdf as the start screen shows them
  await store.remove('/docs/a.pdf'); // a.pdf was missing when opened: main dropped it
  assert.equal(store.path(drawn[0].id), undefined, 'the dropped entry opens nothing');
  assert.equal(store.path(drawn[1].id), '/docs/b.pdf', 'an index would now have named c.pdf');
  assert.equal(store.path(drawn[2].id), '/docs/c.pdf');
  await store.add('/docs/b.pdf'); // reopened: moves to the front and keeps its id
  assert.equal(store.list()[0].id, drawn[1].id);
});
test('X7: userText cleans internal text only: prefixes, error tokens, byte counts and jargon', async () => {
  const { userText } = await import('../build/renderer/messages.js');
  assert.equal(userText('sign-place: no signature has been drawn'), 'No signature has been drawn');
  assert.equal(userText('fill-text: text is empty'), 'Text is empty');
  assert.equal(userText('Text cannot be added on a rotated page (unsupported-transform)'), 'Text cannot be added on a rotated page');
  assert.equal(userText('save: 130000000 bytes is over the 125829120 byte save ceiling'), '124 MB is over the 120 MB save limit');
  assert.equal(userText('open: PDF exceeds the 125829120 byte ceiling'), 'PDF exceeds the 120 MB limit');
  const signPrepare = userText('sign-prepare: incremental save changed source bytes');
  assert.doesNotMatch(signPrepare, /sign-prepare|incremental/);
  assert.equal(userText('That edit was refused.'), 'That change could not be made. Undo or try a different change.');
  assert.equal(userText('edit-text: longer text is refused per run'), 'Longer text is not possible per run');
  assert.equal(userText('Saving needs the desktop app (window.pdf4all is unavailable).'), 'Saving needs the desktop app.');
  assert.equal(userText(''), '');
});

test('X7: userText never rewrites what a person wrote or named, nor catalogue text', async () => {
  const { userText, MSG } = await import('../build/renderer/messages.js');
  for (const text of [
    'Saved refused-claims.pdf.', 'invoice.pdf could not be opened. It may be damaged or not a PDF. Check the file and try again.',
    're: contract.pdf is larger than 120 MB, the most Tandem PDF can open. Try a smaller file.',
    'Highlighted “the claim was refused (see p. 2)”', 'Note: 130000000 bytes per MiB, Identity-H, copy-on-write',
    MSG.open.tooLarge('refused-claims.pdf'), MSG.save.savedAs('re: contract.pdf'), MSG.sign.signedOpened('incremental update.pdf'),
  ]) assert.equal(userText(text), text);
});

test('X7: no catalogue message carries an internal name, a byte count or jargon', async () => {
  const { MSG } = await import('../build/renderer/messages.js');
  const texts = [];
  const walk = value => {
    if (typeof value === 'string') texts.push(value);
    else if (typeof value === 'function') for (const args of [['form.pdf'], ['form.pdf', 1], [1], [2], [3], [true, ['printing'], true], [false, [], false], ['bad-password'], ['busy'], ['no-signing-time weak-digest digest'], ['cancelled'], ['permission']]) { try { const out = value(...args); if (typeof out === 'string' && !args.some(arg => typeof arg === 'string' && arg.includes('-') && out.includes(arg))) texts.push(out); } catch { /* not this shape */ } }
    else if (value && typeof value === 'object') Object.values(value).forEach(walk);
  };
  walk(MSG);
  assert.ok(texts.length > 60, `walked ${texts.length} texts`);
  for (const text of texts) {
    assert.doesNotMatch(text, /\bMiB\b|\bbytes?\b|refused|Identity-H|copy-on-write|incremental|show-text|XFA|DocMDP|P=\d|window\.pdf4all|fixture/, text);
    assert.doesNotMatch(text, /^[a-z]+(?:-[a-z]+)*: /, `an operation prefix: ${text}`);
    assert.doesNotMatch(text, /\b[a-z]+-[a-z]+-[a-z]+\b/, `an error token: ${text}`);
  }
  // Every error that is a full sentence says what to do next, not only what happened.
  for (const [reason, text] of Object.entries(MSG.save.reason)) {
    if (reason === 'cancelled') continue;
    assert.match(text, /[.]\s.+[.]$|Choose|Use |Wait|Remove|Save |try again|Try again/i, `${reason}: ${text}`);
  }
});

test('X8: success and information are never drawn red; STATE error never holds a success', async () => {
  const text = await css();
  assert.match(text, /#error\[data-kind="success"\]:not\(\[hidden\]\) \{\n  color: #0b2414;\n  background: #6fcf8f;/);
  assert.match(text, /#error\[data-kind="info"\]/);
  const renderer = await readFile(join(root, 'src/renderer/renderer.ts'), 'utf8');
  assert.match(renderer, /error: errorEl\.hidden \|\| noticeKind === "success" \|\| noticeKind === "info" \? "" : noticeRaw,/);
  assert.match(renderer, /showMessage\(notice, "success"\);/, 'the late save that reopened the file is a success');
  assert.doesNotMatch(renderer, /showError\(`Saved /, 'no "Saved …" success goes through showError');
});

test('item 9: one Sign entry point opens a chooser; the certificate button is "Sign and save as…" and starts disabled', async () => {
  const html = await readFile(join(root, 'src/renderer/index.html'), 'utf8');
  assert.match(html, /<button id="cert-sign-go" class="primary" type="button" disabled>Sign and save as…<\/button>/);
  assert.match(html, /<button id="cert-sign-open" [^>]*>Sign…<\/button>/);
  for (const id of ['sign-choice-dialog', 'sign-choice-picture', 'sign-choice-certificate', 'sign-choice-cancel']) assert.match(html, new RegExp(`id="${id}"`), id);
  const renderer = await readFile(join(root, 'src/renderer/renderer.ts'), 'utf8');
  assert.match(renderer, /signButton\.addEventListener\("click", \(\) => openSignChoice\(\)\);/);
  assert.match(renderer, /certSignOpenButton\.addEventListener\("click", \(\) => openSignChoice\(\)\);/);
  assert.match(renderer, /if \(!certSignPath\) \{ certSignStatus\.textContent = MSG\.sign\.chooseCertificate; return; \}/, 'never a silent no-op');
  const main = await readFile(join(root, 'src/main.ts'), 'utf8');
  assert.match(main, /return \{ ok: true, name: result\.name, signer: held\.cn, opened: true \};/, 'D6: the signed copy opens (GUI)');
});

test('X5: the search arrows sit inside the search field, apart from the page arrows; X3: Organize keeps the inspector tab', async () => {
  const html = await readFile(join(root, 'src/renderer/index.html'), 'utf8');
  const field = /<div class="search-field">([\s\S]*?)<\/div>/.exec(html)?.[1] ?? '';
  for (const id of ['search-input', 'search-status', 'search-prev', 'search-next']) assert.match(field, new RegExp(`id="${id}"`), id);
  assert.doesNotMatch(field, /id="prev"|id="next"/);
  const renderer = await readFile(join(root, 'src/renderer/renderer.ts'), 'utf8');
  assert.match(renderer, /organize: null,/);
});

test('X7: a wrapped engine message loses every operation prefix', async () => {
  const { userText } = await import('../build/renderer/messages.js');
  assert.equal(userText('content: edit-text: the text is refused'), 'The text is not possible');
});

test('review fix 5: the filled-form question defaults to a new file; Esc cancels', async () => {
  const { MSG } = await import('../build/renderer/messages.js');
  const box = MSG.filled.box('form.pdf');
  assert.equal(box.buttons[box.defaultId], 'Save as New File…');
  assert.equal(box.buttons[box.cancelId], 'Cancel');
  assert.equal(box.buttons[0], 'Save Over', 'main reads answer 0 as Save Over');
  assert.equal(box.message, 'Save over “form.pdf” or as a new file?');
});

test('review fix 11: a failed signing says why in plain words, by reason', async () => {
  const { MSG } = await import('../build/renderer/messages.js');
  const written = 'The signed copy could not be written. Try again or choose another folder.';
  for (const reason of ['invalid', 'certificate-expired', 'unreadable', 'unsupported-key']) assert.notEqual(MSG.sign.failed(reason), written, reason);
  assert.match(MSG.sign.failed('invalid'), /date and time/);
  assert.match(MSG.sign.failed('certificate-expired', 'this certificate expired on 2026-01-01'), /expired on 2026-01-01.*Choose another certificate/);
  assert.match(MSG.sign.failed('unreadable'), /certificate/);
  assert.match(MSG.sign.failed('permission'), /read-only/);
  assert.equal(MSG.sign.failed('something-new'), written);
});

test('review fix 12: Help does not understate how long the private key and password stay in memory', async () => {
  const html = await readFile(join(root, 'src/renderer/index.html'), 'utf8');
  const row = /<th scope="row">Certificate signing<\/th>([\s\S]*?)<\/tr>/.exec(html)?.[1] ?? '';
  assert.match(row, /cannot erase it or the certificate password from memory, so copies of both may stay in the app's memory until it quits/);
  // What Help says the hold does is what src/sign/hold.ts does: one-shot, and dropped after 5 minutes idle.
  const { createCertHold } = await import('../build/sign/hold.js');
  const hold = createCertHold(20);
  hold.put(1, { key: null, certs: [], cn: 'x' });
  assert.ok(hold.take(1));
  assert.equal(hold.take(1), undefined, 'let go right after signing');
  hold.put(2, { key: null, certs: [], cn: 'y' });
  await new Promise(resolve => setTimeout(resolve, 40));
  assert.equal(hold.take(2), undefined, 'let go after the idle time');
});

test('re-review fix 5: catalogue text is never rewritten, even when a value looks internal; small byte counts stay', async () => {
  const { userText, MSG } = await import('../build/renderer/messages.js');
  const named = MSG.open.tooLarge('open: claims.pdf');
  assert.equal(userText(named), named, 'a file named like an operation keeps its name');
  assert.equal(userText(MSG.save.savedAs('save: 2026.pdf')), 'Saved as save: 2026.pdf. The window now edits save: 2026.pdf.');
  assert.equal(userText('open: claims.pdf is gone'), 'Claims.pdf is gone', 'the same words from the engine are internal');
  assert.equal(userText('batch-file: too few bytes for a PDF (1500 bytes)'), 'Too few bytes for a PDF (1500 bytes)');
  assert.equal(userText('save: 130000000 bytes is over the 125829120 byte save ceiling'), '124 MB is over the 120 MB save limit');
});
