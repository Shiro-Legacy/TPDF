import { at, attempt, copyBytes, failReason } from "./engine-core.js";
import { editViewBox, scanEditTargets } from "./content-edit.js";
import { MAX_REDACT_SOURCE_BYTES } from "./limits.js";
import type { MupdfDocument, MupdfModule, MupdfObject, MupdfPage } from "./engine.js";

export const MAX_REDACT_MARKS = 2000;
export const MAX_REDACT_TEXT = 256;
export const MAX_REDACT_STRUCT_NODES = 50_000;
export const MAX_REDACT_IMAGE_PIXELS = 40_000_000;

export const REDACT_SANITIZE_KEYS = [
  "info", "xmp", "attachments", "outlines", "annots", "links", "fields", "layers", "struct",
] as const;
export type RedactSanitizeKey = (typeof REDACT_SANITIZE_KEYS)[number];
export type RedactSanitize = Record<RedactSanitizeKey, boolean>;
export type RedactInventory = {
  info: number; xmp: boolean; attachments: number; outlines: number; annots: number;
  links: number; fields: number; layers: number; struct: number;
};
export type RedactShared = { page: number; kind: "form" | "image"; otherPages: number[] };
export type RedactPreview = {
  marks: number; pages: number[]; pageCounts: { page: number; marks: number }[];
  shared: RedactShared[]; sanitize: RedactInventory; inlineAlt: number;
};
export type RedactMark = { page: number; rect: number[]; search?: string };
export type RedactPlan = { page: number; rect: number[]; quads?: number[][]; search?: string };

/** The measured apply-memory guard, checked on the intermediate rewrite itself. */
export function checkRedactSourceBytes(bytes: number): void {
  if (bytes > MAX_REDACT_SOURCE_BYTES) {
    failReason("too-large", "Redaction is limited to documents up to 12 MiB in this version");
  }
}

function objectId(object: MupdfObject): number {
  return attempt(() => object.isIndirect() ? object.asIndirect() : -1, -1);
}

function subtype(object: MupdfObject): string {
  const value = at(object, "Subtype");
  return value?.isName() ? value.asName() : "";
}

function intersects(left: number[], right: number[]): boolean {
  return left[0] < right[2] && left[2] > right[0] && left[1] < right[3] && left[3] > right[1];
}

function annotationRect(annotation: ReturnType<MupdfPage["getAnnotations"]>[number]): number[] {
  // Redact getRect() grows one point for its border; QuadPoints are the actual
  // search mark and therefore the post-check box. Area marks have no quads.
  const quads = attempt(() => annotation.getQuadPoints() ?? [], [] as number[][]);
  const xs = quads.flatMap(quad => [quad[0], quad[2], quad[4], quad[6]]);
  const ys = quads.flatMap(quad => [quad[1], quad[3], quad[5], quad[7]]);
  if (xs.length) return [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)];
  const rect = attempt(() => [...annotation.getRect()], [] as number[]);
  return rect.length === 4 ? rect : [];
}

export function redactMarks(doc: MupdfDocument): RedactMark[] {
  const marks: RedactMark[] = [];
  for (let pageIndex = 0; pageIndex < doc.countPages(); pageIndex++) {
    const page = doc.loadPage(pageIndex) as MupdfPage;
    try {
      for (const annotation of page.getAnnotations()) {
        if (annotation.getType() !== "Redact") continue;
        const rect = annotationRect(annotation);
        if (rect.length !== 4) continue;
        const tagged = at(annotation.getObject() as MupdfObject, "PDF4AllSearch");
        marks.push({ page: pageIndex, rect, ...(tagged?.isString() ? { search: tagged.asString() } : {}) });
      }
    } finally {
      page.destroy();
    }
  }
  return marks;
}

/** A user rectangle, normalized and clamped to the displayed page box. */
export function redactRect(doc: MupdfDocument, pageIndex: number, value: unknown, op = "redact-add"): number[] {
  if (!Array.isArray(value) || value.length !== 4 || value.some(item => typeof item !== "number" || !Number.isFinite(item))) {
    failReason("invalid", `${op}: rect must be four finite numbers`);
  }
  const page = doc.loadPage(pageIndex) as MupdfPage;
  let box: number[];
  try { box = [...page.getBounds()]; } finally { page.destroy(); }
  const raw = value as number[];
  const x0 = Math.max(box[0], Math.min(raw[0], raw[2]));
  const y0 = Math.max(box[1], Math.min(raw[1], raw[3]));
  const x1 = Math.min(box[2], Math.max(raw[0], raw[2]));
  const y1 = Math.min(box[3], Math.max(raw[1], raw[3]));
  if (x1 - x0 < 1 || y1 - y0 < 1) failReason("invalid", `${op}: the clamped rect must be at least 1 by 1 point`);
  return [x0, y0, x1, y1];
}

/**
 * P6 U3 §3.1/§9.4: a mark's new rect, inside the caller's one `mutate`. The quads must go
 * (MuPDF would redact and `annotationRect` would verify the old ones) and so does the
 * search tag: a moved search mark is an area mark. Measured: with no quads,
 * `setRect → clearQuadPoints → update` keeps the rect exactly (no border growth), so the
 * written rect is the requested one and no deflate is needed.
 */
export function setRedactRect(annotation: ReturnType<MupdfPage["getAnnotations"]>[number], rect: number[]): void {
  annotation.setRect(rect as [number, number, number, number]);
  annotation.clearQuadPoints();
  (annotation.getObject() as MupdfObject).delete("PDF4AllSearch");
  annotation.update();
}

function inflatedQuad(quad: number[], box: number[]): number[] {
  const xs = [quad[0], quad[2], quad[4], quad[6]];
  const ys = [quad[1], quad[3], quad[5], quad[7]];
  const x0 = Math.max(box[0], Math.min(...xs) - 2);
  const y0 = Math.max(box[1], Math.min(...ys) - 2);
  const x1 = Math.min(box[2], Math.max(...xs) + 2);
  const y1 = Math.min(box[3], Math.max(...ys) + 2);
  return [x0, y0, x1, y0, x0, y1, x1, y1];
}

function unionQuads(quads: number[][]): number[] {
  const xs = quads.flatMap(quad => [quad[0], quad[2], quad[4], quad[6]]);
  const ys = quads.flatMap(quad => [quad[1], quad[3], quad[5], quad[7]]);
  return [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)];
}

export function redactSearchPlan(doc: MupdfDocument, value: unknown): RedactPlan[] {
  if (typeof value !== "string" || value.length < 1 || value.length > MAX_REDACT_TEXT || /[\u0000-\u001f\u007f]/.test(value)) {
    failReason("invalid", `redact-search: text must be 1..${MAX_REDACT_TEXT} characters with no controls`);
  }
  const plans: RedactPlan[] = [];
  for (let pageIndex = 0; pageIndex < doc.countPages(); pageIndex++) {
    const page = doc.loadPage(pageIndex) as MupdfPage;
    try {
      const box = [...page.getBounds()];
      const hits = attempt(() => page.search(value, "") ?? [], [] as number[][][]);
      // MuPDF caps one page at 500 hits. Exactly 500 is therefore ambiguous: fail
      // closed rather than silently leaving a 501st occurrence live.
      if (hits.length === 500) failReason("too-many", "redact-search: a page reached the 500-hit search ceiling");
      for (const hit of hits) {
        if (!hit.length) continue;
        const quads = hit.map(quad => inflatedQuad(quad, box));
        plans.push({ page: pageIndex, rect: unionQuads(quads), quads, search: value });
      }
    } finally {
      page.destroy();
    }
  }
  if (!plans.length) failReason("no-match", "redact-search: no match");
  return plans;
}

export function addRedactPlans(doc: MupdfDocument, plans: RedactPlan[]): void {
  for (const plan of plans) {
    const page = doc.loadPage(plan.page) as MupdfPage;
    try {
      const annotation = page.createAnnotation("Redact");
      annotation.setRect(plan.rect as [number, number, number, number]);
      if (plan.quads?.length) annotation.setQuadPoints(plan.quads as [number, number, number, number, number, number, number, number][]);
      if (plan.search) (annotation.getObject() as MupdfObject).put("PDF4AllSearch", doc.asPDF()!.newString(plan.search));
      annotation.update();
    } finally {
      page.destroy();
    }
  }
}

function walkObject(root: MupdfObject | null, visit: (object: MupdfObject) => void, limit = MAX_REDACT_STRUCT_NODES): number {
  const seen = new Set<number>();
  let count = 0;
  const walk = (object: MupdfObject | null): void => {
    if (!object || object.isNull()) return;
    const id = objectId(object);
    if (id > 0) {
      if (seen.has(id)) return;
      seen.add(id);
    }
    count += 1;
    if (count > limit) failReason("too-large", `redact: object walk exceeds ${limit} nodes`);
    visit(object);
    if (object.isArray() || object.isDictionary() || object.isStream()) object.forEach(value => walk(value as MupdfObject));
  };
  walk(root);
  return count;
}

function fieldNodes(fields: MupdfObject | null): MupdfObject[] {
  const nodes: MupdfObject[] = [];
  if (!fields?.isArray()) return nodes;
  const seen = new Set<number>();
  const walk = (node: MupdfObject): void => {
    const id = objectId(node);
    if (id > 0 && seen.has(id)) return;
    if (id > 0) seen.add(id);
    nodes.push(node);
    const kids = at(node, "Kids");
    if (kids?.isArray()) for (let i = 0; i < kids.length; i++) walk(kids.get(i) as MupdfObject);
  };
  for (let i = 0; i < fields.length; i++) walk(fields.get(i) as MupdfObject);
  return nodes;
}

export function redactInventory(doc: MupdfDocument): RedactInventory {
  const pdf = doc.asPDF();
  if (!pdf) failReason("invalid", "redact-preview: the open document is not a PDF");
  const root = at(pdf.getTrailer() as MupdfObject, "Root");
  const info = at(pdf.getTrailer() as MupdfObject, "Info");
  let infoCount = 0;
  info?.forEach((value, key) => { if (!["Producer", "CreationDate", "ModDate"].includes(String(key)) && value.isString()) infoCount += 1; });
  let outlines = 0;
  walkObject(at(root, "Outlines"), object => { if (at(object, "Title")?.isString()) outlines += 1; });
  let annots = 0;
  let links = 0;
  for (let pageIndex = 0; pageIndex < doc.countPages(); pageIndex++) {
    const pageObject = pdf.findPage(pageIndex) as MupdfObject;
    const list = at(pageObject, "Annots");
    if (!list?.isArray()) continue;
    for (let i = 0; i < list.length; i++) {
      const kind = subtype(list.get(i) as MupdfObject);
      if (kind === "Link") links += 1;
      else if (kind !== "Widget" && kind !== "Redact" && kind !== "Popup") annots += 1;
    }
  }
  const fields = fieldNodes(at(at(root, "AcroForm"), "Fields"))
    .filter(node => !!at(node, "V") || !!at(node, "DV") || !!at(node, "AP")).length;
  let layers = 0;
  const ocgs = at(at(root, "OCProperties"), "OCGs");
  if (ocgs?.isArray()) for (let i = 0; i < ocgs.length; i++) if (at(ocgs.get(i) as MupdfObject, "Name")) layers += 1;
  let struct = 0;
  walkObject(at(root, "StructTreeRoot"), object => {
    for (const key of ["ActualText", "Alt", "E"]) if (at(object, key)) struct += 1;
  });
  return {
    info: infoCount,
    xmp: !!at(root, "Metadata"),
    attachments: Object.keys(attempt(() => pdf.getEmbeddedFiles() ?? {}, {} as Record<string, MupdfObject>)).length,
    outlines, annots, links, fields, layers, struct,
  };
}

function placements(doc: MupdfDocument): { page: number; kind: "form" | "image"; object: number; box: number[] }[] {
  const result: { page: number; kind: "form" | "image"; object: number; box: number[] }[] = [];
  for (let page = 0; page < doc.countPages(); page++) {
    const scan = scanEditTargets(doc, page);
    for (const record of scan.records.values()) {
      if (record.depth !== 0 || !record.placement || record.objectNumber < 1) continue;
      result.push({ page, kind: record.kind === "image" ? "image" : "form", object: record.objectNumber, box: editViewBox(record.placement.box, scan.transform) });
    }
  }
  return result;
}

function contentStreams(object: MupdfObject | null): MupdfObject[] {
  if (!object) return [];
  if (object.isStream()) return [object];
  if (!object.isArray()) return [];
  const streams: MupdfObject[] = [];
  for (let i = 0; i < object.length; i++) {
    const item = object.get(i) as MupdfObject;
    if (item.isStream()) streams.push(item);
  }
  return streams;
}

function streamBytes(stream: MupdfObject): Uint8Array {
  const buffer = stream.readStream();
  try { return buffer.asUint8Array().slice(); } finally { buffer.destroy(); }
}

function inlineAltCount(doc: MupdfDocument, pages: number[]): number {
  const pdf = doc.asPDF()!;
  let count = 0;
  const pattern = /\/(?:ActualText|Alt|E)\b/g;
  for (const page of pages) for (const stream of contentStreams(at(pdf.findPage(page) as MupdfObject, "Contents"))) {
    count += new TextDecoder("latin1").decode(streamBytes(stream)).match(pattern)?.length ?? 0;
  }
  return count;
}

export function redactPreview(doc: MupdfDocument): RedactPreview {
  const marks = redactMarks(doc);
  const pageCounts = [...new Set(marks.map(mark => mark.page))].sort((a, b) => a - b)
    .map(page => ({ page, marks: marks.filter(mark => mark.page === page).length }));
  let all: ReturnType<typeof placements>;
  try { all = placements(doc); }
  catch (error) {
    if ((error as { reason?: string } | null)?.reason !== "too-large") throw error;
    failReason("scan-budget", `redact-preview: page content is too complex to check for shared graphics`);
  }
  const shared: RedactShared[] = [];
  const seen = new Set<string>();
  for (const placement of all) {
    if (!marks.some(mark => mark.page === placement.page && intersects(mark.rect, placement.box))) continue;
    for (const other of all) {
      if (other === placement || other.object !== placement.object) continue;
      if (marks.some(mark => mark.page === other.page && intersects(mark.rect, other.box))) continue;
      const key = `${placement.page}:${placement.object}:${other.page}:${other.box.join(",")}`;
      if (seen.has(key)) continue;
      seen.add(key);
      shared.push({ page: placement.page, kind: placement.kind, otherPages: [other.page] });
    }
  }
  const pages = pageCounts.map(item => item.page);
  return { marks: marks.length, pages, pageCounts, shared, sanitize: redactInventory(doc), inlineAlt: inlineAltCount(doc, pages) };
}

function removePageAnnotations(doc: MupdfDocument, sanitize: RedactSanitize): void {
  const pdf = doc.asPDF()!;
  for (let pageIndex = 0; pageIndex < doc.countPages(); pageIndex++) {
    const page = pdf.findPage(pageIndex) as MupdfObject;
    const annots = at(page, "Annots");
    if (!annots?.isArray()) continue;
    const keep = pdf.newArray();
    for (let i = 0; i < annots.length; i++) {
      const item = annots.get(i) as MupdfObject;
      const kind = subtype(item);
      const remove = (kind === "FileAttachment" && sanitize.attachments)
        || (sanitize.annots && kind !== "Widget" && kind !== "Link" && kind !== "Redact");
      if (kind === "Link" && sanitize.links) {
        const action = at(item, "A");
        action?.delete("URI");
        action?.delete("F");
      }
      if (!remove) keep.push(item);
    }
    page.put("Annots", keep);
  }
}

function sanitizeFields(doc: MupdfDocument): void {
  const pdf = doc.asPDF()!;
  const root = at(pdf.getTrailer() as MupdfObject, "Root")!;
  const acro = at(root, "AcroForm");
  if (!acro) return;
  acro.delete("XFA");
  acro.delete("SigFlags");
  const fields = at(acro, "Fields");
  if (!fields?.isArray()) return;
  const signatures = new Set<number>();
  for (let i = 0; i < fields.length; i++) {
    const top = fields.get(i) as MupdfObject;
    const ft = at(top, "FT");
    if (ft?.isName() && ft.asName() === "Sig") for (const node of fieldNodes((() => { const a = pdf.newArray(); a.push(top); return a; })())) signatures.add(objectId(node));
  }
  const kept = pdf.newArray();
  for (let i = 0; i < fields.length; i++) {
    const top = fields.get(i) as MupdfObject;
    if (!signatures.has(objectId(top))) kept.push(top);
  }
  acro.put("Fields", kept);
  for (let pageIndex = 0; pageIndex < doc.countPages(); pageIndex++) {
    const pageObject = pdf.findPage(pageIndex) as MupdfObject;
    const annots = at(pageObject, "Annots");
    if (annots?.isArray() && signatures.size) {
      const next = pdf.newArray();
      for (let i = 0; i < annots.length; i++) {
        const item = annots.get(i) as MupdfObject;
        if (!signatures.has(objectId(item))) next.push(item);
      }
      pageObject.put("Annots", next);
    }
    const page = doc.loadPage(pageIndex) as MupdfPage;
    try {
      for (const widget of page.getWidgets()) {
        const own = widget.getObject() as MupdfObject;
        if (signatures.has(objectId(own))) continue;
        for (let node: MupdfObject | null = own, depth = 0; node && depth < 8; depth++, node = at(node, "Parent")) {
          node.delete("V"); node.delete("DV"); node.delete("AP");
          const ft = at(node, "FT");
          if (ft?.isName() && ft.asName() === "Btn") node.put("AS", pdf.newName("Off"));
        }
        attempt(() => widget.update(), false);
      }
    } finally { page.destroy(); }
  }
}

function sanitizeDocument(doc: MupdfDocument, sanitize: RedactSanitize): RedactInventory {
  const removed = redactInventory(doc);
  const pdf = doc.asPDF()!;
  const trailer = pdf.getTrailer() as MupdfObject;
  const root = at(trailer, "Root")!;
  if (sanitize.info) {
    const info = at(trailer, "Info");
    const keys: string[] = [];
    info?.forEach((_value, key) => { if (!["Producer", "CreationDate", "ModDate"].includes(String(key))) keys.push(String(key)); });
    for (const key of keys) info?.delete(key);
  }
  if (sanitize.xmp) root.delete("Metadata");
  if (sanitize.attachments) at(root, "Names")?.delete("EmbeddedFiles");
  if (sanitize.outlines) root.delete("Outlines");
  removePageAnnotations(doc, sanitize);
  if (sanitize.fields) sanitizeFields(doc);
  if (sanitize.layers) {
    const ocgs = at(at(root, "OCProperties"), "OCGs");
    if (ocgs?.isArray()) for (let i = 0; i < ocgs.length; i++) (ocgs.get(i) as MupdfObject).put("Name", pdf.newString(`Layer ${i + 1}`));
  }
  if (sanitize.struct) walkObject(at(root, "StructTreeRoot"), object => {
    if (object.isDictionary() || object.isStream()) {
      object.delete("ActualText"); object.delete("Alt"); object.delete("E");
    }
  });
  const result = { ...removed };
  for (const key of REDACT_SANITIZE_KEYS) if (!sanitize[key]) {
    if (key === "xmp") result.xmp = false;
    else result[key] = 0;
  }
  return result;
}

function verifyText(page: MupdfPage, rect: number[]): boolean {
  const inner = [rect[0] + 2, rect[1] + 2, rect[2] - 2, rect[3] - 2];
  if (inner[2] <= inner[0] || inner[3] <= inner[1]) return true;
  const structured = page.toStructuredText("preserve-spans");
  let found = false;
  try {
    structured.walk({
      onChar(_char: string, _origin: number[], _font: unknown, _size: number, quad: ArrayLike<number>) {
        const q = Array.from(quad, Number);
        if (q.length < 8) return;
        const xs = [q[0], q[2], q[4], q[6]];
        const ys = [q[1], q[3], q[5], q[7]];
        if (intersects(inner, [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)])) found = true;
      },
    });
  } finally { structured.destroy(); }
  return !found;
}

function verifyPixels(mupdf: MupdfModule, page: MupdfPage, rect: number[]): boolean {
  const pixmap = page.toPixmap(mupdf.Matrix.identity, mupdf.ColorSpace.DeviceRGB, false);
  try {
    const width = pixmap.getWidth();
    const height = pixmap.getHeight();
    const components = pixmap.getNumberOfComponents();
    const pixels = pixmap.getPixels();
    const x0 = Math.max(0, Math.floor(rect[0]) + 2), x1 = Math.min(width, Math.ceil(rect[2]) - 2);
    const y0 = Math.max(0, Math.floor(rect[1]) + 2), y1 = Math.min(height, Math.ceil(rect[3]) - 2);
    let first = "";
    for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) {
      const offset = (y * width + x) * components;
      const color = `${pixels[offset]},${pixels[offset + 1]},${pixels[offset + 2]}`;
      if (!first) first = color;
      else if (color !== first) return false;
    }
    return true;
  } finally { pixmap.destroy(); }
}

function contains(bytes: Uint8Array, needle: Uint8Array): boolean {
  outer: for (let i = 0; i <= bytes.length - needle.length; i++) {
    for (let j = 0; j < needle.length; j++) if (bytes[i + j] !== needle[j]) continue outer;
    return true;
  }
  return false;
}

function utf16be(text: string): Uint8Array {
  const bytes = new Uint8Array(2 + text.length * 2);
  bytes[0] = 0xfe; bytes[1] = 0xff;
  for (let i = 0; i < text.length; i++) { bytes[2 + i * 2] = text.charCodeAt(i) >> 8; bytes[3 + i * 2] = text.charCodeAt(i) & 255; }
  return bytes;
}

export function applyRedactions(
  mupdf: MupdfModule,
  source: ArrayBuffer,
  password: string | null,
  sanitize: RedactSanitize,
  finalOptions: string,
): { bytes: ArrayBuffer; marks: number; pages: number[]; sanitized: RedactInventory; kept: RedactInventory; verified: { text: number; pixels: number; raw: number } } {
  let scratch: MupdfDocument | null = null;
  try {
    scratch = mupdf.Document.openDocument(source, "application/pdf");
    source = new ArrayBuffer(0);
    if (scratch.needsPassword() && (!password || !scratch.authenticatePassword(password))) failReason("encrypted", "redact-apply: scratch document could not be unlocked");
    const marks = redactMarks(scratch);
    const markedPages = new Set(marks.map(mark => mark.page));
    for (let pageIndex = 0; pageIndex < scratch.countPages(); pageIndex++) {
      if (!markedPages.has(pageIndex)) continue;
      const scan = scanEditTargets(scratch, pageIndex);
      const pdf = scratch.asPDF()!;
      for (const record of scan.records.values()) {
        if (record.kind !== "image" || record.objectNumber < 1) continue;
        const image = pdf.newIndirect(record.objectNumber) as MupdfObject;
        const width = at(image, "Width"), height = at(image, "Height");
        const pixels = width?.isNumber() && height?.isNumber() ? width.asNumber() * height.asNumber() : 0;
        if (pixels > MAX_REDACT_IMAGE_PIXELS) failReason("too-large", `redact-apply: an image on page ${pageIndex + 1} exceeds ${MAX_REDACT_IMAGE_PIXELS} pixels`);
      }
    }
    const pages = [...new Set(marks.map(mark => mark.page))].sort((a, b) => a - b);
    for (const pageIndex of pages) {
      const page = scratch.loadPage(pageIndex) as MupdfPage;
      try {
        page.applyRedactions(true, mupdf.PDFPage.REDACT_IMAGE_PIXELS,
          mupdf.PDFPage.REDACT_LINE_ART_REMOVE_IF_TOUCHED, mupdf.PDFPage.REDACT_TEXT_REMOVE);
      } finally { page.destroy(); }
    }
    const beforeSanitize = redactInventory(scratch);
    const sanitized = sanitizeDocument(scratch, sanitize);
    const kept = { ...beforeSanitize };
    for (const key of REDACT_SANITIZE_KEYS) if (sanitize[key]) {
      if (key === "xmp") kept.xmp = false;
      else kept[key] = 0;
    }
    let text = 0, pixels = 0;
    const textFailed: RedactMark[] = [];
    const pixelFailed: RedactMark[] = [];
    for (const mark of marks) {
      const page = scratch.loadPage(mark.page) as MupdfPage;
      try {
        if (verifyText(page, mark.rect)) text += 1; else textFailed.push(mark);
        if (verifyPixels(mupdf, page, mark.rect)) pixels += 1; else pixelFailed.push(mark);
      } finally { page.destroy(); }
    }
    if (textFailed.length) failReason("verify-failed", `redact-apply: text check failed for ${JSON.stringify(textFailed)}`);
    if (pixelFailed.length) failReason("verify-failed", `redact-apply: pixels check failed for ${JSON.stringify(pixelFailed)}`);
    const buffer = scratch.asPDF()!.saveToBuffer(finalOptions);
    let bytes: ArrayBuffer;
    try { bytes = copyBytes(buffer.asUint8Array()); } finally { buffer.destroy(); }
    let raw = 0;
    const searches = new Set(marks.flatMap(mark => mark.search ? [mark.search] : []));
    const pdf = scratch.asPDF()!;
    const checked = new Set<number>();
    const reachable: MupdfObject[] = [];
    const collect = (object: MupdfObject | null): void => {
      if (!object) return;
      const id = objectId(object);
      if (id > 0 && checked.has(id)) return;
      if (id > 0) checked.add(id);
      if (object.isStream()) reachable.push(object);
      const xobjects = at(at(object, "Resources"), "XObject");
      xobjects?.forEach(value => collect(value as MupdfObject));
    };
    for (const page of pages) {
      const pageObject = pdf.findPage(page) as MupdfObject;
      for (const stream of contentStreams(at(pageObject, "Contents"))) reachable.push(stream);
      const xobjects = at(at(pageObject, "Resources"), "XObject");
      xobjects?.forEach(value => collect(value as MupdfObject));
    }
    for (const search of searches) {
      const plain = new TextEncoder().encode(search);
      const wide = utf16be(search);
      const hex = new TextEncoder().encode([...plain].map(byte => byte.toString(16).padStart(2, "0")).join(""));
      const wideHex = new TextEncoder().encode([...wide].map(byte => byte.toString(16).padStart(2, "0")).join(""));
      if (reachable.some(stream => {
        const data = streamBytes(stream);
        const lower = new TextEncoder().encode(new TextDecoder("latin1").decode(data).toLowerCase());
        return contains(data, plain) || contains(data, wide) || contains(lower, hex) || contains(lower, wideHex);
      })) failReason("verify-failed", `redact-apply: raw check failed for ${JSON.stringify(search)}`);
      raw += 1;
    }
    for (const pageIndex of pages) {
      const page = scratch.loadPage(pageIndex) as MupdfPage;
      try {
        if (page.getAnnotations().some(annotation => annotation.getType() === "Redact")) failReason("verify-failed", "redact-apply: raw check failed because a Redact mark remains");
      } finally { page.destroy(); }
    }
    const check = mupdf.Document.openDocument(bytes, "application/pdf");
    try {
      if (check.needsPassword() && (!password || !check.authenticatePassword(password))) failReason("verify-failed", "redact-apply: output could not be verified");
      // countVersions() is the xref-chain proof. A raw `/Prev` scan is not: an
      // outline with sibling bookmarks carries `/Prev` in its own dicts, and a
      // kept attachment can carry `%%EOF` — both refused a correct output.
      const out = check.asPDF();
      if (out?.countVersions() !== 1 || out.wasRepaired()) {
        failReason("verify-failed", "redact-apply: output is not one compact revision");
      }
    } finally { check.destroy(); }
    return { bytes, marks: marks.length, pages, sanitized, kept, verified: { text, pixels, raw } };
  } finally {
    scratch?.destroy();
  }
}

export function checkSanitize(value: unknown): RedactSanitize {
  if (!value || typeof value !== "object" || Array.isArray(value)) failReason("invalid", "redact-apply: sanitize must be an object");
  const object = value as Record<string, unknown>;
  const result = {} as RedactSanitize;
  for (const key of REDACT_SANITIZE_KEYS) {
    if (typeof object[key] !== "boolean") failReason("invalid", `redact-apply: sanitize.${key} must be true or false`);
    result[key] = object[key] as boolean;
  }
  return result;
}
