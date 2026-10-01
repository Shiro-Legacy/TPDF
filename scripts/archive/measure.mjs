#!/usr/bin/env node
/**
 * P0-a measurement harness: hello.pdf (small) and large.pdf (500 pages), with
 * the pinned Node MuPDF build only.
 *
 *   node scripts/measure.mjs        # 3 fresh processes per fixture, median
 *   node scripts/measure.mjs --once # one run, JSON on stdout (used internally)
 *
 * Every run happens in its own process so the RSS numbers are not polluted by
 * the previous run. Renders use Matrix.identity (scale 1.0 = 72 dpi) and the
 * full-document text pass renders nothing.
 *
 * There is deliberately no pass/fail field: the thresholds are the user's
 * decision and are not set yet (progress.md, P0-a).
 */
import { execFileSync } from 'node:child_process';
import { readFile, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import mupdf from 'mupdf';

const FIXTURES = ['hello', 'large'];
const RUNS = 3;
const OUT = '.quad/shared/p0/p0-measure.json';

const ms = () => performance.now();
const median = (values) => [...values].sort((a, b) => a - b)[values.length >> 1];

async function measure(id) {
  const bytes = await readFile(`fixtures/${id}.pdf`);
  const rss0 = process.memoryUsage().rss;
  let peak = rss0;
  const sample = () => { peak = Math.max(peak, process.memoryUsage().rss); };

  let t = ms();
  const doc = mupdf.Document.openDocument(bytes, 'application/pdf');
  const openMs = ms() - t;
  const pages = doc.countPages();
  sample();

  const render = (index) => {
    const page = doc.loadPage(index);
    const pixmap = page.toPixmap(mupdf.Matrix.identity, mupdf.ColorSpace.DeviceRGB, false);
    const size = [pixmap.getWidth(), pixmap.getHeight()];
    pixmap.destroy();
    page.destroy();
    return size;
  };

  t = ms();
  const firstPageSize = render(0);
  const firstPageRenderMs = ms() - t;
  sample();

  t = ms();
  render(pages - 1);
  const lastPageRenderMs = ms() - t;
  sample();

  t = ms();
  let textChars = 0;
  for (let index = 0; index < pages; index++) {
    const page = doc.loadPage(index);
    const text = page.toStructuredText();
    textChars += text.asText().length;
    text.destroy();
    page.destroy();
  }
  const textExtractMs = ms() - t;
  sample();

  const rssDeltaBytes = peak - rss0;
  doc.destroy();
  return {
    id,
    file: `fixtures/${id}.pdf`,
    bytes: bytes.length,
    pages,
    textChars,
    firstPageSizePt: firstPageSize,
    openMs: +openMs.toFixed(2),
    firstPageRenderMs: +firstPageRenderMs.toFixed(2),
    lastPageRenderMs: +lastPageRenderMs.toFixed(2),
    textExtractMs: +textExtractMs.toFixed(2),
    rssDeltaBytes,
    rssDeltaMiB: +(rssDeltaBytes / 1024 / 1024).toFixed(1),
  };
}

if (process.argv.includes('--once')) {
  // Warm up the wasm engine first: font/ICC stores and the page cache are
  // lazily initialised and would otherwise land in whichever fixture happens
  // to be measured first.
  const warmup = mupdf.Document.openDocument(await readFile('fixtures/hello.pdf'), 'application/pdf');
  const warmupPage = warmup.loadPage(0);
  warmupPage.toPixmap(mupdf.Matrix.identity, mupdf.ColorSpace.DeviceRGB, false).destroy();
  warmupPage.toStructuredText().destroy();
  warmupPage.destroy();
  warmup.destroy();

  const results = [];
  for (const id of FIXTURES) results.push(await measure(id));
  process.stdout.write(`${JSON.stringify(results)}\n`);
} else {
  const runs = [];
  for (let i = 0; i < RUNS; i++) {
    const stdout = execFileSync(process.execPath, [fileURLToPath(import.meta.url), '--once'], {
      encoding: 'utf8',
      maxBuffer: 1 << 20,
    });
    runs.push(JSON.parse(stdout.trim().split('\n').pop()));
  }

  const metrics = ['openMs', 'firstPageRenderMs', 'lastPageRenderMs', 'textExtractMs', 'rssDeltaBytes'];
  const fixtures = FIXTURES.map((id, index) => {
    const perRun = runs.map((run) => run[index]);
    return {
      id,
      file: perRun[0].file,
      bytes: perRun[0].bytes,
      pages: perRun[0].pages,
      textChars: perRun[0].textChars,
      firstPageSizePt: perRun[0].firstPageSizePt,
      runs: perRun.map((run) => Object.fromEntries(metrics.map((key) => [key, run[key]]))),
      median: Object.fromEntries(metrics.map((key) => [key, median(perRun.map((run) => run[key]))])),
      medianRssDeltaMiB: Number((median(perRun.map((run) => run.rssDeltaBytes)) / 1024 / 1024).toFixed(1)),
    };
  });

  const report = {
    measuredAt: new Date().toISOString(),
    harness: 'scripts/measure.mjs',
    thresholds: null,
    passFail: 'not evaluated - thresholds are the user decision and are not set yet (progress.md P0-a)',
    method: {
      runsPerFixture: RUNS,
      isolation: 'one fresh Node process per run; medians over the runs',
      renderScale: 'Matrix.identity (1.0 = 72 dpi), DeviceRGB, no alpha',
      textExtraction: 'loadPage + toStructuredText + asText for every page',
      memory: 'process.memoryUsage().rss sampled after every phase, reported as peak minus baseline of that run',
      warmup: 'each process renders and extracts one hello.pdf page before measuring, so wasm font/ICC init is not charged to the first fixture',
      caveat: 'mupdf caches pages, so the “last page” render of a one-page document (hello.pdf) is a cache hit; for large.pdf it is a real page',
    },
    engine: { package: 'mupdf', wrapper: 'mupdf@1.28.1 (npm), AGPL-3.0-or-later', wasm: 'mupdf-wasm.wasm' },
    node: process.version,
    host: {
      model: sysctl('hw.model'),
      cpu: sysctl('machdep.cpu.brand_string'),
      memoryBytes: Number(sysctl('hw.memsize')) || null,
      macOS: swVers(),
    },
    fixtures,
  };

  await writeFile(OUT, `${JSON.stringify(report, null, 2)}\n`);
  console.log(`wrote ${OUT}`);
  for (const fixture of fixtures) console.log(fixture.id, JSON.stringify(fixture.median));
}

function sysctl(key) {
  try {
    return execFileSync('sysctl', ['-n', key], { encoding: 'utf8' }).trim() || null;
  } catch {
    return null;
  }
}

function swVers() {
  try {
    const version = execFileSync('sw_vers', ['-productVersion'], { encoding: 'utf8' }).trim();
    const build = execFileSync('sw_vers', ['-buildVersion'], { encoding: 'utf8' }).trim();
    return `${version} (${build})`;
  } catch {
    return null;
  }
}
