/** The worker's PDF engine in a TPDF_ENGINE=rust build: the Rust engine's WASM package from ./vendor (scripts/build.mjs installs it as worker-engine.js). */
import init, { RustApp } from "./vendor/tandem_rust_workflow.js";
import { createRustEngine } from "./engine-rust.js";
import type { WorkerEngine } from "./worker-engine.js";

export async function loadEngine(): Promise<WorkerEngine> {
  await init({ module_or_path: new URL("./vendor/tandem_rust_workflow_bg.wasm", import.meta.url) });
  const engine = createRustEngine({ RustApp });
  return { name: "rust", createSession: engine.createSession, dispatch: engine.dispatch as WorkerEngine["dispatch"] };
}
