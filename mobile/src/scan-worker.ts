// The scan worker composes the PDF (scan-pdf.ts) and reopens it with the Rust engine the app ships. The engine's
// glue, adapter and WASM are the desktop build's copies in engine/ (scripts/build.mjs keeps these imports external).
import init, { RustApp } from '../../src/renderer/vendor/tandem_rust_workflow.js';
import { createRustEngine } from '../../src/renderer/engine-rust.js';
import { checkScanPDF, createScanPDF, platformDeflate, type ScanEngine } from './scan-pdf.js';
import type { ScanOutput, ScanPage } from './scan-model.js';

const worker = self as unknown as Worker;
const base = new URL('./engine/', import.meta.url);
let engine: Promise<ScanEngine> | null = null;

/** The Rust engine, loaded on the first scan (each scan gets a new worker, so once per scan). */
function rustEngine(): Promise<ScanEngine> {
  engine ??= init({ module_or_path: new URL('vendor/tandem_rust_workflow_bg.wasm', base) })
    .then(() => createRustEngine({ RustApp }) as unknown as ScanEngine,
      () => { throw new Error('The PDF engine could not start. Your scans are still here.'); });
  // Composing may fail first; this failure is then reported by whoever awaits it, or not at all.
  engine.catch(() => {});
  return engine;
}

async function fallbackFont(): Promise<Uint8Array> {
  const response = await fetch(new URL('fonts/DroidSansFallback.ttf', base));
  if (!response.ok) throw new Error('The font for non-Latin text could not be loaded.');
  return new Uint8Array(await response.arrayBuffer());
}

worker.onmessage = async (event: MessageEvent<{ pages: ScanPage[]; output: ScanOutput }>) => {
  try {
    const { pages, output } = event.data;
    const loading = rustEngine();
    const { bytes, pageCount } = await createScanPDF(pages, output, { fallbackFont, deflate: platformDeflate() });
    checkScanPDF(await loading, bytes, pageCount);
    worker.postMessage({ bytes }, [bytes]);
  } catch (error) { worker.postMessage({ error: error instanceof Error ? error.message : 'Could not create the PDF.' }); }
};
worker.postMessage({ ready: true });

