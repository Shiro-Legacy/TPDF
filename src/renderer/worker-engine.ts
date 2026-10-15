/**
 * The worker's PDF engine: the Rust engine's WASM package from ./vendor. A
 * TPDF_ENGINE=mupdf build replaces this module with worker-engine-mupdf.ts
 * (scripts/build.mjs); a build never loads the other engine.
 */
import init, { RustApp } from "./vendor/tandem_rust_workflow.js";
import { createRustEngine } from "./engine-rust.js";
import type { DispatchResult, SessionRequest } from "./engine.js";

/** What the worker needs from either engine. */
export type WorkerEngine = {
  name: "rust" | "mupdf";
  createSession(): object;
  dispatch(request: SessionRequest, session: object): DispatchResult;
};

export async function loadEngine(): Promise<WorkerEngine> {
  await init({ module_or_path: new URL("./vendor/tandem_rust_workflow_bg.wasm", import.meta.url) });
  const engine = createRustEngine({ RustApp });
  return { name: "rust", createSession: engine.createSession, dispatch: engine.dispatch as WorkerEngine["dispatch"] };
}
