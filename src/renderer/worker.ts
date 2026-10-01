import * as mupdf from "./vendor/mupdf.js";
import { createEngine } from "./engine.js";
import { copyBytes } from "./engine-core.js";

type Probe = { id: number; probe: true };
type Roundtrip = { id: number; roundtrip: true; pdf: ArrayBuffer };

/** The P0 round-trip probe's own bound, unrelated to the app ceiling of the same name. */
const PROBE_MAX_BYTES = 1_000_000;
const worker = self as unknown as Worker;
const engine = createEngine(mupdf);
const session = engine.createSession();

worker.onmessage = (event: MessageEvent<Probe | Roundtrip | Parameters<typeof engine.dispatch>[0]>) => {
  const data = event.data;
  if (!data) return;
  // P0 smoke paths — unchanged since the first spike.
  if ("probe" in data && data.probe) {
    void runProbe(data.id);
    return;
  }
  if ("roundtrip" in data && data.roundtrip) {
    try {
      const result = roundtrip(data.pdf);
      worker.postMessage({ id: data.id, roundtrip: true, ...result }, [result.savedBytes]);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      worker.postMessage({ id: data.id, roundtrip: true, ok: false, error: message });
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

function roundtrip(pdf: ArrayBuffer) {
  if (!(pdf instanceof ArrayBuffer) || pdf.byteLength < 5 || pdf.byteLength > PROBE_MAX_BYTES) {
    throw new Error("PDF exceeds size bound");
  }
  const doc = mupdf.Document.openDocument(pdf, "application/pdf").asPDF();
  if (!doc) throw new Error("not a PDF");
  try {
    doc.disableJS();
    const page = doc.loadPage(0) as mupdf.PDFPage;
    try {
      const annot = page.createAnnotation("Square");
      annot.setRect([100, 100, 300, 200]);
      annot.setInteriorColor([1, 0, 0]);
      annot.setContents("PDF4All P0 note");
      annot.update();
      const field = page.getWidgets().find(widget => widget.getName() === "name");
      if (!field) throw new Error("missing name field");
      field.setTextValue("PDF4All P0");
      page.update();
      const buffer = doc.saveToBuffer("incremental=no");
      try {
        const savedBytes = copyBytes(buffer.asUint8Array());
        return { ok: true, savedBytes, savedSize: savedBytes.byteLength, readback: reopen(savedBytes) };
      } finally {
        buffer.destroy();
      }
    } finally {
      page.destroy();
    }
  } finally {
    doc.destroy();
  }
}

function reopen(savedBytes: ArrayBuffer) {
  const doc = mupdf.Document.openDocument(savedBytes, "application/pdf").asPDF();
  if (!doc) throw new Error("reopen failed");
  try {
    doc.disableJS();
    const page = doc.loadPage(0) as mupdf.PDFPage;
    try {
      const annots = page.getAnnotations()
        .filter(annot => annot.getType() === "Square")
        .map(annot => ({ type: annot.getType(), rect: [...annot.getRect()], contents: annot.getContents() }));
      const fieldValue = page.getWidgets().find(widget => widget.getName() === "name")?.getValue() ?? "";
      return { annots, fieldValue };
    } finally {
      page.destroy();
    }
  } finally {
    doc.destroy();
  }
}

