/**
 * The editor's engine worker (mobile/src/app.ts starts it as engine/worker.js): one session on the Rust engine,
 * each request answered with the engine's reply. The Electron renderer's worker without its P0 smoke paths (the
 * `probe` and `roundtrip` messages only Electron's `--smoke` run sent).
 */
import { loadEngine } from "./worker-engine.js";
import type { SessionRequest } from "./protocol.js";

const worker = self as unknown as Worker;
// One engine. A failed load fails the worker at once through its error event (a rejected top-level await
// raises none), with no fallback.
const engine = await loadEngine().catch((err: unknown) => {
  reportError(err);
  return new Promise<never>(() => {});
});
const session = engine.createSession();

worker.onmessage = (event: MessageEvent<SessionRequest>) => {
  const data = event.data;
  if (!data || !("op" in data)) return;
  const { reply, transfer } = engine.dispatch(data, session);
  worker.postMessage(reply, transfer ?? []);
};
worker.postMessage({ ready: true });
