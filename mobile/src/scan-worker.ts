import * as mupdf from '../../src/renderer/vendor/mupdf.js';
import { createScanPDF } from './scan-pdf.js';
import type { ScanOutput, ScanPage } from './scan-model.js';
const worker = self as unknown as Worker;
worker.onmessage = (event: MessageEvent<{ pages: ScanPage[]; output: ScanOutput }>) => {
  try {
    const bytes = createScanPDF(mupdf, event.data.pages, event.data.output);
    worker.postMessage({ bytes }, [bytes]);
  } catch (error) { worker.postMessage({ error: error instanceof Error ? error.message : 'Could not create the PDF.' }); }
};
worker.postMessage({ ready: true });
