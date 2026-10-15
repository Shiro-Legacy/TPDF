/** The worker's PDF engine in a TPDF_ENGINE=mupdf build (scripts/build.mjs installs it as worker-engine.js). */
import * as mupdf from "./vendor/mupdf.js";
import { createEngine } from "./engine.js";
import type { WorkerEngine } from "./worker-engine.js";

export async function loadEngine(): Promise<WorkerEngine> {
  const engine = createEngine(mupdf);
  return { name: "mupdf", createSession: engine.createSession, dispatch: engine.dispatch as WorkerEngine["dispatch"] };
}
