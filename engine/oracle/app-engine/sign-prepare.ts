import type { MupdfModule } from "./engine.js";
import { winAnsiBytes } from "../../../web/engine-core.js";
import { MAX_SAVE_BYTES, MAX_SIG_CONTENTS_BYTES } from "../../../web/limits.js";

// The options are part of the worker protocol (web/protocol.ts).
import type { SignPrepareOptions } from "../../../web/protocol.js";
export type { SignPrepareOptions };

const SENTINEL = "[0 1000000000 1000000000 1000000000]";
const reject = (reason: string): never => { throw Object.assign(new Error(reason), { reason }); };

export function prepareSignature(mupdf: MupdfModule, source: ArrayBuffer, opts: SignPrepareOptions, incremental = false) {
  const opened = mupdf.Document.openDocument(source, "application/pdf");
  const doc = opened.asPDF();
  if (!doc) { opened.destroy(); throw Object.assign(new Error("invalid"), { reason: "invalid" }); }
  try {
    const page = doc.loadPage(opts.page) as InstanceType<MupdfModule["PDFPage"]>;
    try {
      const root = doc.getTrailer().get("Root");
      const acro = root.get("AcroForm");
      const fields = acro.isNull() ? doc.newArray() : acro.get("Fields");
      if (typeof opts.field === "string") {
        // §1.4 (D2): the field keeps its object number; the signed appearance REPLACES its /AP.
        const name = opts.field;
        const found = page.getWidgets().find(w => w.getFieldType() === "signature" && w.getName() === name);
        if (!found) return reject("invalid");
        const widget = found.getObject();
        const rect = [...found.getRect()];
        widget.delete("AP");
        widget.put("AP", { N: appearance(doc, opts, rect) });
        widget.put("V", signatureDict(doc, opts));
        if (acro.isNull()) { fields.push(widget); root.put("AcroForm", doc.addObject({ Fields: fields, SigFlags: 3 })); }
        else acro.put("SigFlags", 3);
        if (opts.certify) root.put("Perms", { DocMDP: widget.get("V") });
        return save(doc, source, opts.page, rect, incremental);
      }
      const annot = page.createAnnotation("Widget");
      const widget = annot.getObject();
      widget.put("FT", "Sig");
      let n = 1;
      const names = new Set<string>();
      for (let i = 0; i < fields.length; i++) names.add(fields.get(i).get("T").asString());
      while (names.has(`Signature${n}`)) n++;
      widget.put("T", doc.newString(`Signature${n}`));
      widget.put("F", 4);
      const box = page.getBounds();
      let rect: number[] = [0, 0, 0, 0];
      if (opts.rect) {
        const [x0, y0, x1, y1] = opts.rect;
        const [bx0, by0, bx1, by1] = box;
        rect = [Math.max(bx0, Math.min(bx1, x0)), Math.max(by0, Math.min(by1, y0)),
          Math.max(bx0, Math.min(bx1, x1)), Math.max(by0, Math.min(by1, y1))];
        if (rect[2] - rect[0] < 40 || rect[3] - rect[1] < 20) reject("invalid");
        annot.setRect(rect as [number, number, number, number]);
        widget.put("AP", { N: appearance(doc, opts, rect) });
      }
      const sigObj = signatureDict(doc, opts);
      widget.put("V", sigObj);
      fields.push(widget);
      if (acro.isNull()) root.put("AcroForm", doc.addObject({ Fields: fields, SigFlags: 3 }));
      else acro.put("SigFlags", 3);
      if (opts.certify) root.put("Perms", { DocMDP: sigObj });
      return save(doc, source, opts.page, rect, incremental);
    } finally { page.destroy(); }
  } finally { doc.destroy(); }
}

type PdfDoc = NonNullable<ReturnType<InstanceType<MupdfModule["Document"]>["asPDF"]>>;

/** U1's text appearance for a page-space `rect` (w × h), turned by the page's `/Rotate`. */
function appearance(doc: PdfDoc, opts: SignPrepareOptions, rect: number[]) {
  const font = doc.addObject({ Type: "Font", Subtype: "Type1", BaseFont: "Helvetica", Encoding: "WinAnsiEncoding" });
  const w = rect[2] - rect[0], h = rect[3] - rect[1];
  const lines = [`Digitally signed by ${opts.name}`, localStamp(opts.when), ...(opts.reason ? [`Reason: ${opts.reason}`] : [])];
  // winAnsiBytes already escapes ( ) and \.
  const safe = (s: string) => String.fromCharCode(...winAnsiBytes(s));
  const commands = [`q 0.93 0.95 1 rg 0 0 ${w} ${h} re f 0.1 0.2 0.5 RG 1 w 0.5 0.5 ${w - 1} ${h - 1} re S BT /Helv 9 Tf 0 0 0.15 rg`];
  // Td is relative: place the first line, then step down 11 pt per line.
  lines.forEach((line, i) => commands.push(`${i ? "0 -11" : `8 ${h - 13}`} Td (${safe(line)}) Tj`));
  commands.push("ET Q\n");
  const rotation = ((doc.findPage(opts.page).getInheritable("Rotate").asNumber() % 360) + 360) % 360;
  const matrix = rotation === 90 ? [0, 1, -1, 0, 0, 0] : rotation === 180 ? [-1, 0, 0, -1, w, h] : rotation === 270 ? [0, -1, 1, 0, 0, w] : [1, 0, 0, 1, 0, 0];
  return doc.addStream(Uint8Array.from(commands.join(" "), ch => ch.charCodeAt(0)), { Type: "XObject", Subtype: "Form", BBox: [0, 0, w, h], Matrix: matrix, Resources: { Font: { Helv: font } } });
}

/** The U1 `/Sig` dict: the 8192-byte hole, the ByteRange sentinel, and the DocMDP reference when certifying. */
function signatureDict(doc: PdfDoc, opts: SignPrepareOptions) {
  const sig: Record<string, unknown> = { Type: "Sig", Filter: "Adobe.PPKLite", SubFilter: "adbe.pkcs7.detached", Contents: doc.newByteString(new Uint8Array(MAX_SIG_CONTENTS_BYTES)), ByteRange: [0, 1000000000, 1000000000, 1000000000], Name: doc.newString(opts.name), M: doc.newString(pdfDate(opts.when)) };
  if (opts.reason) sig.Reason = doc.newString(opts.reason);
  if (opts.certify) sig.Reference = [{ Type: "SigRef", TransformMethod: "DocMDP", TransformParams: { Type: "TransformParams", P: 2, V: "1.2" } }];
  return doc.addObject(sig);
}

function save(doc: PdfDoc, source: ArrayBuffer, page: number, rect: number[], incremental: boolean) {
  const saved = doc.saveToBuffer(incremental ? "incremental" : "incremental=no,garbage=yes");
  try {
    const bytes = saved.asUint8Array().slice();
    if (bytes.byteLength > MAX_SAVE_BYTES) reject("too-large");
    return scanSignature(bytes.buffer, page, rect, incremental ? source.byteLength : 0);
  } finally { saved.destroy(); }
}

/** YYYY-MM-DD HH:MM in the signer's local time (contract §2). */
function localStamp(ms: number) {
  const d = new Date(ms), p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

function pdfDate(date: number) {
  const d = new Date(date), p = (n: number) => String(n).padStart(2, "0");
  return `D:${d.getUTCFullYear()}${p(d.getUTCMonth() + 1)}${p(d.getUTCDate())}${p(d.getUTCHours())}${p(d.getUTCMinutes())}${p(d.getUTCSeconds())}Z`;
}

export function scanSignature(bytes: ArrayBuffer, page: number, rect: number[], sourceLength: number) {
  const data = new Uint8Array(bytes), text = new TextDecoder("latin1").decode(data);
  const contents = `/Contents<${"0".repeat(MAX_SIG_CONTENTS_BYTES * 2)}>`;
  const hits = (needle: string) => { const found: number[] = []; let from = 0, at; while ((at = text.indexOf(needle, from)) >= 0) { found.push(at); from = at + 1; } return found; };
  const rangeHits = hits(SENTINEL).filter(at => at >= (sourceLength && text.startsWith("%PDF") ? sourceLength : 0));
  const contentsHits = hits(contents).filter(at => at >= (sourceLength && text.startsWith("%PDF") ? sourceLength : 0));
  if (rangeHits.length !== 1 || contentsHits.length !== 1) reject("invalid");
  const at = contentsHits[0] + "/Contents".length + 1, len = MAX_SIG_CONTENTS_BYTES * 2;
  const byteRangeAt = rangeHits[0];
  if (data[at - 1] !== 60 || data[at + len] !== 62) reject("invalid");
  return { bytes, contentsAt: at, contentsLen: len, byteRangeAt, page, rect, revisions: sourceLength ? hits("/ByteRange").length : 1 }; // one per signature dict: the prior ones plus this one
}
