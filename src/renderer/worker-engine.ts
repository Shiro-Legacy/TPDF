/**
 * The worker's PDF engine. scripts/build.mjs installs the Rust engine's loader
 * (worker-engine-rust.ts) as this module; tsc's own emit of it exports nothing,
 * so a build that skipped that step fails to load rather than run without one.
 */
import type { DispatchResult, SessionRequest } from "./protocol.js";

/** What the worker needs from the engine. */
export type WorkerEngine = {
  name: "rust";
  createSession(): object;
  dispatch(request: SessionRequest, session: object): DispatchResult;
};

export declare function loadEngine(): Promise<WorkerEngine>;
