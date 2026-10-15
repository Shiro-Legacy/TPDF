import { loadEngine, type WorkerEngine } from "./worker-engine.js";
import type { SessionRequest } from "./engine.js";

type Probe = { id: number; probe: true };
type Roundtrip = { id: number; roundtrip: true; pdf: ArrayBuffer };

/** The P0 round-trip probe's own bound, unrelated to the app ceiling of the same name. */
const PROBE_MAX_BYTES = 1_000_000;
const worker = self as unknown as Worker;
// A build ships one engine (scripts/build.mjs). A failed load fails the worker at once through
// its error event (a rejected top-level await raises none), with no fallback.
const engine = await loadEngine().catch((err: unknown) => {
  reportError(err);
  return new Promise<never>(() => {});
});
const session = engine.createSession();

worker.onmessage = (event: MessageEvent<Probe | Roundtrip | SessionRequest>) => {
  const data = event.data;
  if (!data) return;
  // P0 smoke paths.
  if ("probe" in data && data.probe) {
    void runProbe(data.id);
    return;
  }
  if ("roundtrip" in data && data.roundtrip) {
    try {
      const result = roundtrip(engine, data.pdf);
      worker.postMessage({ id: data.id, roundtrip: true, ...result }, [result.savedBytes]);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      worker.postMessage({ id: data.id, roundtrip: true, engine: engine.name, ok: false, error: message });
    }
    return;
  }
  if (!("op" in data)) return;
  const { reply, transfer } = engine.dispatch(data, session);
  worker.postMessage(reply, transfer ?? []);
};
worker.postMessage({ ready: true });

async function runProbe(id: number) {
  const nodeAbsent = typeof process === "undefined" && typeof require === "undefined";
  let violation: { effectiveDirective: string; violatedDirective: string; blockedURI: string } | null = null;
  const matched = new Promise<void>(resolve => {
    const timer = setTimeout(finish, 1000);
    function onViolation(event: Event) {
      const v = event as SecurityPolicyViolationEvent;
      if (v.effectiveDirective !== "connect-src" || !String(v.blockedURI).includes("example.invalid")) return;
      violation = { effectiveDirective: v.effectiveDirective, violatedDirective: v.violatedDirective, blockedURI: v.blockedURI };
      finish();
    }
    function finish() {
      clearTimeout(timer);
      self.removeEventListener("securitypolicyviolation", onViolation);
      resolve();
    }
    self.addEventListener("securitypolicyviolation", onViolation);
  });
  let rejected = false;
  let rejection: string | undefined;
  try {
    await fetch("https://example.invalid/pdf4all-worker-probe");
  } catch (err) {
    rejected = true;
    rejection = err instanceof Error ? err.name : "rejected";
  }
  await matched;
  worker.postMessage({
    id, probe: true, nodeAbsent, cspBlocked: !!violation && rejected, rejected, rejection, violation,
  });
}

/**
 * The P0 round trip through engine ops, in sessions of its own: a Square
 * annotation and the `name` field's value on form.pdf, saved and read back
 * from a fresh open of the saved bytes.
 */
function roundtrip(engine: WorkerEngine, pdf: ArrayBuffer) {
  if (!(pdf instanceof ArrayBuffer) || pdf.byteLength < 5 || pdf.byteLength > PROBE_MAX_BYTES) {
    throw new Error("PDF exceeds size bound");
  }
  const edited = engine.createSession();
  const again = engine.createSession();
  const call = (session: object, request: Record<string, unknown>) => {
    const { reply } = engine.dispatch({ id: 0, ...request } as SessionRequest, session);
    if (reply.ok !== true) throw new Error(`${String(request.op)}: ${String(reply.error)}`);
    return reply;
  };
  type Items<T> = { items: T[] };
  try {
    call(edited, { op: "open", pdf, name: "form.pdf" });
    const widget = (call(edited, { op: "fields", page: 0 }) as Items<{ id: number; name: string }>).items
      .find(item => item.name === "name");
    if (!widget) throw new Error("missing name field");
    call(edited, { op: "annot-add", page: 0, type: "Square", rect: [100, 100, 300, 200], contents: "PDF4All P0 note" });
    call(edited, { op: "field-set", page: 0, widgetId: widget.id, value: "PDF4All P0" });
    const savedBytes = call(edited, { op: "save" }).bytes;
    if (!(savedBytes instanceof ArrayBuffer)) throw new Error("save: no bytes");
    call(again, { op: "open", pdf: savedBytes.slice(0), name: "roundtrip.pdf" });
    const annots = (call(again, { op: "annots", page: 0 }) as Items<{ type: string; rect: number[]; contents: string }>).items
      .filter(annot => annot.type === "Square")
      .map(annot => ({ type: annot.type, rect: [...annot.rect], contents: annot.contents }));
    const fieldValue = (call(again, { op: "fields", page: 0 }) as Items<{ name: string; value: unknown }>).items
      .find(item => item.name === "name")?.value ?? "";
    return { engine: engine.name, ok: true, savedBytes, savedSize: savedBytes.byteLength, readback: { annots, fieldValue } };
  } finally {
    engine.dispatch({ id: 0, op: "close" }, edited);
    engine.dispatch({ id: 0, op: "close" }, again);
  }
}
