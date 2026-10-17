// SPDX-License-Identifier: AGPL-3.0-or-later
// Runs the same placement edits, pastes and batches on engine.ts (MuPDF)
// and on the Rust app engine, saves both, renders every edited page of both
// saves with MuPDF at scale 1 and reports differing pixels and whether the
// page's content streams decode to the same text; then the median time of
// the writing op over five runs on each engine. Usage, from the app root
// after `npm run build`, `npm run build:oracle` and engine/tools/build-workflow.sh:
//   node engine/oracle/compare-app-edit-objects.mjs [summary.json]
import { readFileSync, writeFileSync } from 'node:fs';
import mupdf from 'mupdf';
import { createEngine as createMupdf } from '../../build-oracle/engine/oracle/app-engine/engine.js';
import { createEngine as createRust } from '../../tests/rust-engine.mjs';

const engines = { mupdf: createMupdf(mupdf), rust: createRust(mupdf) };
const LATIN = 'latin1';
const OBJECTS6 = 'fixtures/p6/objects6.pdf';
const DIRECT3 = 'fixtures/p6/direct3.pdf';
const OBJECTS = 'fixtures/p3/edit-objects.pdf';
const STREAMS = 'fixtures/p3/targets-streams.pdf';
const image = name => ({ name, bytes: buffer(readFileSync(`fixtures/${name}`)) });

const object = (name, action, extra = {}, nth = 0) => ({ op: 'object', name, nth, action, ...extra });
const SCENARIOS = [
  { name: 'move', file: OBJECTS6, steps: [object('Im1', 'move', { box: [80, 600, 180, 650] })] },
  { name: 'resize', file: OBJECTS6, steps: [object('Im1', 'resize', { box: [72, 600, 272, 700] })] },
  { name: 'rotate a plain placement', file: OBJECTS6, steps: [object('Im1', 'transform', { transform: 'rotate-cw' })] },
  { name: 'flip a turned placement', file: OBJECTS6, steps: [object('Im2', 'transform', { transform: 'flip-h' })] },
  { name: 'crop: rewrite a clip', file: OBJECTS6, steps: [object('Im4', 'crop', { box: [80, 405, 120, 430] })] },
  { name: 'crop: insert a clip', file: OBJECTS6, steps: [object('Im1', 'crop', { box: [92, 610, 152, 640] })] },
  { name: 'duplicate a turned placement', file: OBJECTS6, steps: [object('Im2', 'duplicate', { box: [410, 660, 460, 760] })] },
  { name: 'delete a Form', file: OBJECTS6, steps: [object('Fm1', 'delete')] },
  { name: 'replace with a PNG', file: STREAMS, steps: [object('Im1', 'replace', { image: 'img-rgb.png' })] },
  { name: 'copy-on-write: inherited resources', file: OBJECTS, page: 1, steps: [object('Fm1', 'move', { box: [100, 500, 220, 520] })] },
  { name: 'edit-objects', file: DIRECT3, steps: [{ op: 'objects', items: [['Im1', 'move', [80, 600, 180, 650]], ['Im2', 'delete'], ['Fm1', 'duplicate', 'shift']] }] },
  { name: 'paste on the same page', file: OBJECTS6, steps: [{ op: 'copy', targets: [['image', 'Im1'], ['image', 'Im2']] }, { op: 'paste' }] },
  { name: 'paste a Form into another document', file: OBJECTS, steps: [{ op: 'copy', targets: [['form', 'Fm2']] }, { op: 'open', file: OBJECTS6 }, { op: 'paste' }] },
  { name: 'paste a run on another page', file: DIRECT3, page: 2, steps: [{ op: 'copy', page: 0, targets: [['text', 'Move me']] }, { op: 'paste' }] },
  { name: 'edit-batch: a run and an image', file: DIRECT3, steps: [{ op: 'batch', text: 'Move me', image: 'Im1' }] },
];

function buffer(bytes) {
  return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
}

let nextId = 1;
const call = (engine, session, request) => engine.dispatch({ id: nextId++, ...request }, session).reply;
const objects = scan => scan.items.filter(item => item.kind !== 'text');
const named = (scan, name, nth = 0) => objects(scan).filter(item => item.objectName === name)[nth];

/** One step's writing request on `page`, built from a fresh scan. */
function request(engine, session, step, page) {
  const scan = call(engine, session, { op: 'edit-targets', page: step.page ?? page });
  const base = { page: step.page ?? page, generation: scan.generation, revision: scan.revision };
  switch (step.op) {
    case 'object': {
      const target = named(scan, step.name, step.nth);
      const extra = step.image ? { image: image(step.image) } : {};
      return { op: 'edit-object', ...base, targetId: target.targetId, action: step.action, box: step.box, transform: step.transform, ...extra };
    }
    case 'objects': {
      const items = step.items.map(([name, action, box]) => {
        const target = named(scan, name);
        const shifted = box === 'shift' ? target.placement.box.map(v => v + 10) : box;
        return { targetId: target.targetId, action, ...(shifted ? { box: shifted } : {}) };
      });
      return { op: 'edit-objects', ...base, items };
    }
    case 'copy': {
      const targets = step.targets.map(([kind, name]) => ({
        kind, id: kind === 'text' ? scan.items.find(item => item.text === name).targetId : named(scan, name).targetId,
      }));
      return { op: 'clip-copy', ...base, targets };
    }
    case 'paste':
      return { op: 'clip-paste', page };
    case 'batch': {
      const run = scan.items.find(item => item.kind === 'text' && item.text === step.text);
      const box = named(scan, step.image).placement.box;
      return { op: 'edit-batch', ...base, items: [
        { op: 'edit-text', targetId: run.targetId, move: { dx: 0, dy: 20 } },
        { op: 'edit-object', targetId: named(scan, step.image).targetId, action: 'move', box: box.map((v, k) => k % 2 ? v - 10 : v + 10) },
      ] };
    }
    default:
      return null;
  }
}

/** Runs one scenario on one engine: the saved bytes and the writing op's time. */
function run(engine, scenario) {
  const session = engine.createSession();
  const page = scenario.page ?? 0;
  const open = file => call(engine, session, { op: 'open', name: 'x.pdf', pdf: buffer(readFileSync(file)) });
  if (!open(scenario.file).ok) return { error: 'open' };
  let ms = 0;
  for (const step of scenario.steps) {
    if (step.op === 'open') {
      if (!open(step.file).ok) return { error: 'open' };
      continue;
    }
    const built = request(engine, session, step, page);
    const started = performance.now();
    const reply = call(engine, session, built);
    if (built.op !== 'clip-copy') ms += performance.now() - started;
    if (!reply.ok) return { error: `${built.op}: ${reply.error} ${reply.detail ?? ''}` };
  }
  const saved = call(engine, session, { op: 'save' });
  if (!saved.ok) return { error: `save: ${saved.error}` };
  return { bytes: new Uint8Array(saved.bytes), page, ms };
}

function contentText(doc, page) {
  const contents = doc.loadPage(page).getObject().get('Contents');
  const list = contents.isArray() ? Array.from({ length: contents.length }, (_, i) => contents.get(i)) : [contents];
  return list.map(item => Buffer.from(item.readStream().asUint8Array()).toString(LATIN)).join('\n');
}

function pixels(doc, page) {
  const pixmap = doc.loadPage(page).toPixmap(mupdf.Matrix.identity, mupdf.ColorSpace.DeviceRGB, false, true);
  return { width: pixmap.getWidth(), height: pixmap.getHeight(), samples: pixmap.getPixels().slice() };
}

const median = values => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];
const summary = [];
for (const scenario of SCENARIOS) {
  const results = { mupdf: run(engines.mupdf, scenario), rust: run(engines.rust, scenario) };
  const row = { name: scenario.name };
  if (results.mupdf.error || results.rust.error) {
    Object.assign(row, { mupdf: results.mupdf.error ?? 'ok', rust: results.rust.error ?? 'ok' });
    summary.push(row);
    console.log(JSON.stringify(row));
    continue;
  }
  const docs = { mupdf: mupdf.Document.openDocument(results.mupdf.bytes, 'application/pdf'), rust: mupdf.Document.openDocument(results.rust.bytes, 'application/pdf') };
  const page = results.mupdf.page;
  const [a, b] = [pixels(docs.mupdf, page), pixels(docs.rust, page)];
  let differing = 0;
  let worst = 0;
  if (a.width === b.width && a.height === b.height) {
    for (let i = 0; i < a.samples.length; i += 3) {
      const d = Math.max(Math.abs(a.samples[i] - b.samples[i]), Math.abs(a.samples[i + 1] - b.samples[i + 1]), Math.abs(a.samples[i + 2] - b.samples[i + 2]));
      if (d) differing++;
      worst = Math.max(worst, d);
    }
  } else differing = -1;
  Object.assign(row, { page, pixels: a.width * a.height, differing, worst, sameContent: contentText(docs.mupdf, page) === contentText(docs.rust, page) });
  const times = { mupdf: [], rust: [] };
  for (let k = 0; k < 5; k++) for (const name of ['mupdf', 'rust']) times[name].push(run(engines[name], scenario).ms);
  row.ms = { mupdf: Number(median(times.mupdf).toFixed(2)), rust: Number(median(times.rust).toFixed(2)) };
  summary.push(row);
  console.log(JSON.stringify(row));
}
if (process.argv[2]) writeFileSync(process.argv[2], `${JSON.stringify(summary, null, 2)}\n`);
