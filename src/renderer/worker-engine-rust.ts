/** The worker's PDF engine: the Rust engine's WASM package from ./vendor (scripts/build.mjs installs it as worker-engine.js). */
import init, { RustApp, cjk_font_wanted, has_cjk_font, set_cjk_font } from "./vendor/tandem_rust_workflow.js";
import { errorMessage } from "./engine-core.js";
import { createRustEngine } from "./engine-rust.js";
import type { WorkerEngine } from "./worker-engine.js";

/** The CJK fallback font scripts/build.mjs puts beside the worker (scripts/cjk-font.mjs). */
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
