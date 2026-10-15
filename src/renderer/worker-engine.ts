/**
 * The worker's PDF engine. scripts/build.mjs installs the selected engine's
 * loader (worker-engine-rust.ts or worker-engine-mupdf.ts) as this module;
 * tsc's own emit of it exports nothing, so a build that skipped that step
 * fails to load rather than run an engine it did not select.
 */
import type { DispatchResult, SessionRequest } from "./engine.js";

/** What the worker needs from either engine. */
export type WorkerEngine = {
  name: "rust" | "mupdf";
  createSession(): object;
  dispatch(request: SessionRequest, session: object): DispatchResult;
};

export declare function loadEngine(): Promise<WorkerEngine>;
