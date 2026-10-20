// The scan worker has the Rust engine the app ships compose the PDF (scan-compose.ts lays the pages out), then
// reopens it with the same engine. The engine is the editor worker's worker-engine.js (web/, served from engine/),
// with its glue, adapter and WASM (scripts/build.mjs keeps these imports external): the same CJK font loading as
// the editor's.
import { loadEngine, type WorkerEngine } from '../../web/worker-engine.js';
import { checkScanPDF, composeScan, type ScanEngine } from './scan-compose.js';
import type { ScanOutput, ScanPage } from './scan-model.js';

const worker = self as unknown as Worker;
let engine: Promise<WorkerEngine> | null = null;

/** The Rust engine, loaded on the first scan (each scan gets a new worker, so once per scan). */
function rustEngine(): Promise<WorkerEngine> {
  engine ??= loadEngine().catch(() => { throw new Error('The PDF engine could not start. Your scans are still here.'); });
  return engine;
}

worker.onmessage = async (event: MessageEvent<{ pages: ScanPage[]; output: ScanOutput }>) => {
  try {
    const { pages, output } = event.data;
    const scanner = await rustEngine() as unknown as ScanEngine;
    const { bytes, pageCount } = composeScan(scanner, pages, output);
    checkScanPDF(scanner, bytes, pageCount);
    worker.postMessage({ bytes }, [bytes]);
  } catch (error) { worker.postMessage({ error: error instanceof Error ? error.message : 'Could not create the PDF.' }); }
};
worker.postMessage({ ready: true });
