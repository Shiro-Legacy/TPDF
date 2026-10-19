/**
 * P3 U2 smoke: `--edit=content` ALPHA→BETA, BRAVO overflow, tail AHPLA.
 */
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { mkdtemp as makeTempDir, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { createRequire } from 'node:module';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test, { after } from 'node:test';
import { appBinary, appEntry } from './app-bin.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));

/** Every temp dir this file makes (housekeeping B): removed after all its tests, never before their asserts. */
const madeDirs = [];
const mkdtemp = async prefix => { const dir = await makeTempDir(prefix); madeDirs.push(dir); return dir; };
after(() => Promise.all(madeDirs.map(dir => rm(dir, { recursive: true, force: true, maxRetries: 5 }))));
const electron = appBinary;
const py = join(root, process.platform === 'win32' ? '.venv-qa/Scripts/python.exe' : '.venv-qa/bin/python');
const skip = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON'
  : !readFileSync(join(root, 'src/renderer/renderer.ts'), 'utf8').includes('P3_EDIT') ? 'no P3_EDIT'
  : false;

async function runSmoke(args, timeoutMs = 60_000) {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-p3-'));
  const reportPath = join(dir, 'report.json');
  const child = spawn(electron, [`--user-data-dir=${join(dir, 'udata')}`, ...appEntry, '--smoke', `--report=${reportPath}`, ...args], {
    cwd: root,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let timedOut = false;
  const timer = setTimeout(() => { timedOut = true; child.kill(); }, timeoutMs);
  const code = await new Promise(resolve => child.on('close', resolve));
  clearTimeout(timer);
  let report = null;
  try { report = JSON.parse(await readFile(reportPath, 'utf8')); } catch { /* */ }
  return { code, report, timedOut, dir };
}

function textHas(pdf, expected) {
  const r = spawnSync(py, [join(root, 'scripts/verify-roundtrip.py'), pdf, '--text-has', JSON.stringify(expected)], { encoding: 'utf8' });
  assert.equal(r.status, 0, r.stdout + r.stderr);
  assert.equal(JSON.parse(r.stdout).ok, true);
}

function recover(pdf, token) {
  const r = spawnSync(py, [join(root, 'scripts/p0b/recover.py'), pdf, token], { encoding: 'utf8' });
  return JSON.parse(r.stdout);
}

function editRegion(before, after, rect) {
  const r = spawnSync(py, [join(root, 'scripts/verify-edit-region.py'), before, after, '--rect', rect, '--margin', '2'], { encoding: 'utf8' });
  const body = r.stdout ? JSON.parse(r.stdout) : { ok: false, error: r.stderr };
  assert.equal(r.status, 0, JSON.stringify(body));
  assert.equal(body.ok, true, JSON.stringify(body));
  return body;
}

test('smoke --edit=content on edit-transformed ALPHA→BETA, BRAVO overflows', { skip, timeout: 60_000 }, async () => {
  const fixture = join(root, 'fixtures/p0b/edit-transformed.pdf');
  const dest = join(await mkdtemp(join(tmpdir(), 'pdf4all-p3-xf-')), 'out.pdf');
  const { code, report, timedOut } = await runSmoke([`--open=${fixture}`, '--edit=content', `--save-to=${dest}`]);
  assert.equal(timedOut, false);
  assert.equal(code, 0, JSON.stringify(report));
  assert.equal(report?.ok, true);
  const content = report?.content ?? report?.edit?.content ?? report?.edit;
  assert.equal(content?.edited?.from, 'ALPHA');
  assert.equal(content?.edited?.to, 'BETA');
  assert.ok((content?.refused ?? []).some(r => r.detail === 'overflow'), 'BRAVO growth beyond the page margin must be refused: ' + JSON.stringify(content?.refused));
  textHas(dest, { 0: ['BETA', 'NEIGHBOR'] });
  const rec = recover(dest, 'ALPHA');
  assert.equal(rec.ok, true, JSON.stringify({ raw: rec.raw_count, hits: rec.object_hit_count }));
  editRegion(fixture, dest, '72,300,160,330');
});

test('smoke --edit=content on edit-tagged keeps MCID and link rect', { skip, timeout: 60_000 }, async () => {
  const fixture = join(root, 'fixtures/p0b/edit-tagged.pdf');
  const dest = join(await mkdtemp(join(tmpdir(), 'pdf4all-p3-tag-')), 'out.pdf');
  const { code, report, timedOut } = await runSmoke([`--open=${fixture}`, '--edit=content', `--save-to=${dest}`]);
  assert.equal(timedOut, false);
  assert.equal(code, 0, JSON.stringify(report));
  assert.equal(report?.ok, true);
  const content = report?.content ?? report?.edit;
  assert.equal(content?.edited?.to, 'BRAVO');
  textHas(dest, { 0: ['BRAVO', 'NEIGHBOR'] });
  const rec = recover(dest, 'ALPHA');
  assert.equal(rec.ok, true, JSON.stringify({ raw: rec.raw_count, hits: rec.object_hit_count }));
  const pyCheck = spawnSync(py, ['-c', `
import json, sys
from pypdf import PdfReader
r = PdfReader(sys.argv[1])
root = r.trailer["/Root"]
elem = root["/StructTreeRoot"]["/K"][0].get_object()
actual = str(elem.get("/ActualText", ""))
page = r.pages[0].get_contents().get_data()
link = r.pages[0]["/Annots"][0].get_object()["/Rect"]
rect = [float(x) for x in link]
print(json.dumps({"actual": actual, "mcid": b"/MCID 0" in page, "rect": rect}))
`, dest], { encoding: 'utf8' });
  assert.equal(pyCheck.status, 0, pyCheck.stderr);
  const tagged = JSON.parse(pyCheck.stdout);
  assert.equal(tagged.actual, 'BRAVO');
  assert.equal(tagged.mcid, true);
  assert.deepEqual(tagged.rect, [200, 695, 280, 720]);
  editRegion(fixture, dest, '72,700,160,724');
});

test('smoke --edit=content on edit-tail: growth shifts tail and oversize remains refused', { skip, timeout: 60_000 }, async () => {
  const fixture = join(root, 'fixtures/p3/edit-tail.pdf');
  const dest = join(await mkdtemp(join(tmpdir(), 'pdf4all-p3-tail-')), 'out.pdf');
  const { code, report, timedOut } = await runSmoke([`--open=${fixture}`, '--edit=content', `--save-to=${dest}`]);
  assert.equal(timedOut, false);
  assert.equal(code, 0, JSON.stringify(report));
  const content = report?.content ?? report?.edit;
  assert.equal(content?.edited?.from, 'ALPHA');
  assert.equal(content?.edited?.to, 'BETA');
  assert.ok(Math.abs(content?.last?.shift + 8.01) < 0.01, 'the tail shift is reported');
  assert.ok((content?.refused ?? []).some(r => r.detail === 'overflow'), 'oversize text remains refused: ' + JSON.stringify(content?.refused));
});

test('smoke --edit=content on encrypted-noedit is read-only', { skip, timeout: 60_000 }, async () => {
  const fixture = join(root, 'fixtures/encrypted-noedit.pdf');
  // The user password unlocks the file; /P still forbids modify, so the edit itself must refuse.
  const { code, report, timedOut } = await runSmoke([`--open=${fixture}`, '--edit=content', '--password=pdf4all-user']);
  assert.equal(timedOut, false);
  assert.equal(code, 0, JSON.stringify(report));
  const content = report?.content ?? report?.edit;
  const refused = content?.refused ?? [];
  assert.ok((content?.candidates ?? 0) > 0, 'unlocked no-edit file still lists candidates (copy permitted): ' + JSON.stringify(content));
  assert.ok(refused.length > 0 || content?.edited == null, JSON.stringify(content));
  assert.ok(refused.some(item => item.detail === 'read-only'), JSON.stringify(content));
});
