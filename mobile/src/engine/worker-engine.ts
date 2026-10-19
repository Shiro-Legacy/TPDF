/**
 * The worker's PDF engine: the Rust engine's WASM package from ./vendor, with its CJK fallback font from ./fonts
 * (mobile/scripts/build.mjs puts both beside this module in dist/engine). The editor's worker (worker.ts) and the
 * scan worker (../scan-worker.ts) load it.
 *
 * The desktop app built this module from two files (src/renderer/worker-engine.ts declared the type and
 * worker-engine-rust.ts was copied over its emit); with one engine, mobile keeps them as one file.
 */
import init, { RustApp, cjk_font_wanted, has_cjk_font, set_cjk_font } from "./vendor/tandem_rust_workflow.js";
import { errorMessage } from "./engine-core.js";
import { createRustEngine } from "./engine-rust.js";
import type { DispatchResult, SessionRequest } from "../protocol.js";

/** What the worker needs from the engine. */
export type WorkerEngine = {
  name: "rust";
  createSession(): object;
  dispatch(request: SessionRequest, session: object): DispatchResult;
};

/** The CJK fallback font mobile/scripts/build.mjs puts beside the worker (assets/fonts/DroidSansFallback.ttf). */
const CJK_FONT = new URL("./fonts/DroidSansFallback.ttf", import.meta.url);

/**
 * The CJK font, read the first time a render draws CJK text without it. Synchronous like the dispatch
 * that asks for it, so the render's reply already has the text (a worker may block on a request).
 */
function cjkFont(): Uint8Array | null {
  try {
    const request = new XMLHttpRequest();
    request.open("GET", CJK_FONT, false);
    request.responseType = "arraybuffer";
    request.send();
    if (request.status === 200 && request.response instanceof ArrayBuffer) return new Uint8Array(request.response);
    throw new Error(`HTTP ${request.status}`);
  } catch (err) {
    console.warn(`the CJK font could not be read (${errorMessage(err)}); CJK text in fonts a PDF does not embed is not drawn`);
    return null;
  }
}

export async function loadEngine(): Promise<WorkerEngine> {
  await init({ module_or_path: new URL("./vendor/tandem_rust_workflow_bg.wasm", import.meta.url) });
  const engine = createRustEngine({ RustApp });
  engine.useCjkFont({ set_cjk_font, cjk_font_wanted, has_cjk_font }, cjkFont);
  return { name: "rust", createSession: engine.createSession, dispatch: engine.dispatch as WorkerEngine["dispatch"] };
}
