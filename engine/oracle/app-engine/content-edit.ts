/**
 * P3 content editing: the read-only scanner (`scanEditTargets`, `walkEditContent`) and
 * the writer that follows it (A1 step 2). Moved byte-for-byte out of `engine.ts`; the
 * only textual change is the added `export `. Runtime imports come from
 * `./engine-core.js` only, so the compiled `content-edit.js` never imports
 * `./engine.js` (contract §5.4); the shared types come in as `import type`, erased.
 */
import {
  MAX_IMAGE_PAGE_POINTS,
  attempt,
  errorMessage,
  failEditText,
  failEditObject,
  failReason,
  winAnsiChar,
  winAnsiByte,
  concatBytes,
  winAnsiBytes,
  BASE14_FONTS,
} from "../../../web/engine-core.js";
import { at } from "./mupdf-core.js";
import type { EditOrient } from "../../../web/protocol.js";
import type { EditObjectAction, EditObjectImage, EditTarget, EditTextDetail, EditTargetFont, EditTargetKind, EditTargetSupport, MupdfDocument, MupdfObject, MupdfPage } from "./engine.js";

// ---------------------------------------------------------------------------
// The scanner's cost model, moved with the code that enforces it (contract §5.4).
// ---------------------------------------------------------------------------

/**
 * P3 U1 (read-only edit targets): the whole cost model of one discovery request,
 * in one place. Every bound is enforced by the scanner before it grows; hitting
 * one refuses the request rather than returning a partial target list.
 */
export const MAX_EDIT_SCAN_BYTES = 2 * 1024 * 1024;

/** U1b: how many identical placements of one run the association tolerates. */
export const MAX_EDIT_EQUAL_MATCHES = 8;

export const MAX_EDIT_TRAVERSALS = 128;

export const MAX_EDIT_FORM_DEPTH = 16;

export const MAX_EDIT_NESTING = 32;

export const MAX_EDIT_TOKENS = 200_000;

/** print-edit D3: the walker's raw ceiling (per-glyph blocks); `MAX_EDIT_TARGETS` binds after the glyph-line collapse. */
export const MAX_EDIT_RAW_TARGETS = 4096;

export const MAX_EDIT_TARGETS = 512;

export const MAX_EDIT_TEXT_CHARS = 4096;

export const MAX_EDIT_REPLY_TEXT_CHARS = 65_536;

export const MAX_EDIT_OPERANDS = 256;

// ---------------------------------------------------------------------------
// P3 U1: read-only edit-target discovery.
//
// One request, one page, one bounded pass over that page's /Contents streams and
// whatever Form XObjects they invoke. Nothing is written: `readStream()` and
// `toStructuredText()` are the only native calls the scanner makes. Identity is
// the operator's byte span plus a hash of its payload, never a text match, an
// object number or a bounding box, and `resolveEditTarget` re-enumerates before
// it trusts a handle.
//
// Known ceiling (recorded in the U1 evidence): `PDFObject.readStream()`
// decompresses inside WASM before JS sees a length, so MAX_EDIT_SCAN_BYTES
// bounds scanning and retention, not native decompression allocation.
// ---------------------------------------------------------------------------

/**
 * U3: an Identity-H CID font this build can prove: the code -> BMP character map
 * from a parsed /ToUnicode CMap, and the /W advances (absent code -> `/DW`),
 * all in 1000-unit text space.
 */
export type EditCid = {
  toUnicode: Map<number, string>;
  widths: Map<number, number>;
  dw: number;
  /** Coded: 1 for a symbolic simple font with no /Encoding, 2 for Identity-H. */
  bytesPerCode: 1 | 2;
};

/** U3: the write's reverse map (character -> code) and the width table for one run. */
export type EditCidWrite = { chars: Map<string, number>; widths: Map<number, number>; dw: number; bytesPerCode: 1 | 2 };

/**
 * U4: one simple placement. `rect` is what the placement's own `cm` maps to `box`:
 * the unit square for an image, or the `/Matrix`-mapped `/BBox` for a Form. Resize
 * recovers `a d e f` from `rect` and the new box, one formula for both kinds.
 */
export type EditPlacement = {
  cmStart: number;
  cmEnd: number;
  qStart: number;
  qEnd: number;
  box: number[];
  rect: number[];
  /** P6 U3 §9.1 / P6 U6 §9.8: the VISIBLE rect in the page view space (y down), for obstacle tests; never sent in a reply. */
  view: number[];
  /** P6 U6 §1.1: the `cm` matrix (six numbers) and its axis orientation. `box` is the full (uncropped) box. */
  matrix: number[];
  orient: EditOrient;
  /** P6 U6 §9.3: the single `x y w h re W n` directly after the block's `q`, or null. */
  clip: EditClip | null;
  /** P6 U6 §9.8: `box ∩ clip.rect` (= `box` without a clip), in page points. */
  visible: number[];
};

/** P6 U6 §1.1: one of the eight axis-aligned orientations of a placement `cm` (web/protocol.ts). */
export type { EditOrient };

/**
 * P6 U6 §9.3: a placement clip. `start` = the `q` token's end and `end` = the `n` token's
 * end (the removal span); `opsStart`/`opsEnd` span the four `re` operands (the rewrite span).
 */
export type EditClip = { rect: number[]; start: number; end: number; opsStart: number; opsEnd: number };

/** One resolved font dictionary, as the scanner needs it and as the reply reports it. */
export type EditFont = {
  summary: EditTargetFont;
  name: string | null;
  subtype: string | null;
  subset: boolean | null;
  simple: boolean;
  encodingUnknown: boolean;
  winAnsiEncoding: boolean;
  writingMode: number | null;
  /** U3: the Identity-H map and widths, or null when this build cannot prove them. */
  cid: EditCid | null;
  /** The font dictionary's indirect object number: the repertoire's identity (-1 when direct). */
  objectNumber: number;
};

/** The live structured-text font handle, used to measure a run's advance. */
/** Set once by the engine: the standard-14 font of that name (edit-text's WinAnsi fallback). */
let base14Font: ((name: string) => EditFontHandle) | null = null;
export function editSetBase14Font(factory: (name: string) => EditFontHandle): void { base14Font = factory; }

export type EditFontHandle = {
  getName?: () => string;
  encodeCharacter?: (char: number | string) => number;
  advanceGlyph?: (gid: number, wmode?: number) => number;
};

/** One structured-text character: association, its quad, and the font/size the width gate needs. */
export type EditRunChar = { ch: string; font: EditFontHandle | null; size: number; quad: number[]; origin?: number[] };

/** One structured-text line, for association only; the font name is a cross-check, never the font fact. */
export type EditRun = { text: string; fontName: string | null; quads: number[][]; chars: EditRunChar[] };

/** One `/ActualText` span in the owning stream, with its decoded bytes. */
export type EditActualText = { start: number; end: number; bytes: Uint8Array };

/** One open marked-content scope: its properties decide hidden content and tag rewriting. */
export type EditMarkedScope = {
  optional: boolean;
  actualText: EditActualText | null;
  /** An ActualText reached through a shared `/Properties` entry cannot be attributed. */
  sharedActualText: boolean;
  mcid: number | null;
};

/**
 * The worker-side record a `targetId` names. Never sent in a reply and never
 * built from a request: only the scanner creates these, and only a re-scan
 * proves a handle still names the same operator.
 */
export type EditTargetRecord = {
  targetId: string;
  kind: EditTargetKind;
  page: number;
  /** `c0` for a page /Contents element, `c0/Fm1@41` inside a Form invocation. */
  path: string;
  /** The stream object that holds the operator (the Form XObject for a descendant). */
  objectNumber: number;
  opStart: number;
  opEnd: number;
  operandStart: number;
  operandEnd: number;
  depth: number;
  support: EditTargetSupport;
  reason?: string;
  text?: string;
  font?: EditTargetFont;
  bounds?: number[];
  /** Another text-show follows before a positioning operator; that tail shifts by Δ. */
  tailFollows?: boolean;
  tailTargetId?: string;
  /** P3b: the text-state inputs used by the full advance formula. */
  horizontalScale?: number;
  characterSpacing?: number;
  wordSpacing?: number;
  tjAdjustment?: number;
  /** Debt sweep 2 §9.3: the run's text render mode (Tr); only 0|1|2 runs are candidates. Worker-only. */
  renderMode?: number;
  /** P3b: computed after the page scan from overlapping targets/placements. */
  growthLimit?: number;
  /** P6 U2: the nearest obstacle's left edge − 1 on this line, or null (no text-margin fallback). */
  obstacleLimit?: number | null;
  tailRight?: number;
  leftToRightTail?: boolean;
  /** `TJ` is rewritten as one `Tj`; the reply/UI names the dropped kerning. */
  kerned?: boolean;
  /** This record's simple font explicitly uses WinAnsiEncoding with no Differences. */
  winAnsiEncoding?: boolean;
  /** U2: the `/ActualText` span that must be rewritten with the operand, when it matches. */
  actualText?: EditActualText | null;
  mcid?: number | null;
  /** U2: how many optional-content scopes enclose this operator. */
  ocDepth?: number;
  /** U2: the operator's own decoded string bytes, for the ActualText equality check. */
  payloadBytes?: Uint8Array;
  /** U2: a StructElem `/ActualText` that equals the payload and must be rewritten with it. */
  structActualText?: string;
  /** U1b: the privatisations an edit makes first, or absent for an unshared target. */
  cow?: EditCow;
  /** P6 U2: linear part of Tm × CTM × page transform at the show (text space → `bounds` space), or null. */
  textLinear?: number[] | null;
  /** P6 U2: no show since the last positioning op, so Tm equals Tlm here and a `Td` moves only this run. */
  firstOnLine?: boolean;
  /** P6 U3 fix S1: the run's text-space origin in `bounds` space, or null when the pen is not proven there. */
  origin?: number[] | null;
  /** P6 U2: the `Tf` in force at the show, for the size rewrite and its restore. */
  tfName?: string | null;
  tfSize?: number | null;
  /** P6 U2: TL in force at the show (paragraph rule). */
  leading?: number | null;
  /** P6 U2 §3.1: the show before this one in the same BT, and the text-space translation
   * the ops between them compose to (null when anything else sits between). */
  prevShowId?: string | null;
  between?: number[] | null;
  /** U3: the run's own `Tf` size, the CID width rule's multiplier. */
  fontSize?: number | null;
  /** U3: the font object whose repertoire this run draws from (Identity-H runs only). */
  cidKey?: number;
  /** U3: the characters this run's font can draw on this page (sorted, unambiguous). */
  repertoire?: string;
  /** U3: the write's reverse map and widths, for an Identity-H run. */
  cidWrite?: EditCidWrite;
  /** U4: the object placement's spans and geometry. */
  placement?: EditPlacement;
  /** U4: what `edit-object` may do to this placement. */
  objectOps?: EditObjectAction[];
  /** U4: the `/Resources/XObject` name this placement draws. */
  objectName?: string;
  /** U4: image facts, read from the image XObject. */
  image?: EditObjectImage;
  /** print-edit D2: a print-to-PDF glyph line (worker-only): one block per glyph, x in user space. */
  glyphLine?: EditGlyphLine;
  /** U4 (§7.4): this placement is the only `Do` on the page naming its resource. */
  /** The scan's byte budget (shared with every record of that scan): a copy counts toward it. */
  budget?: { bytes: number };
};

/** print-edit D2: `colour` is the block's colour group ("" when none), `space` the word gap in em. */
export type EditGlyphLine = { glyphs: { code: number; x: number; adv: number }[]; y: number; scale: number; colour: string; space: number };

/**
 * U1b: every obstacle between the page and the run is one of the three proven shapes
 * (contract-p3-unit1b.md rule 1, archived in .quad/backup/history-20260924.tgz),
 * applied in this order before the write (rule 2).
 */
export type EditCow = {
  inheritedResources: boolean;
  /** The page /Contents element index to copy because another page references it. */
  sharedContents: number | null;
  /** A depth-1 Form invoked from that element: graft, register under a fresh name, rename this `Do`. */
  form: { name: string; reference: number; elementIndex: number; operandStart: number; operandEnd: number } | null;
};

/** The graphics/text state a content stream walk carries; forms always start fresh. */
/** U4: one `cm` seen since its own `q`, with the operand span a move/resize rewrites. */
export type EditCm = { start: number; end: number; matrix: number[] };

/**
 * The graphics/text state a content stream walk carries; forms always start fresh.
 * U4: `cms` holds the `cm`s since this state's own `q`, `clean` says nothing else was
 * painted or clipped in that block, and `qToken`/`qPath` name the block's `q` and the
 * `/Contents` element it sits in (a block may not cross elements).
 */
export type EditTextState = {
  fontName: string | null;
  fontSize: number | null;
  horizontalScale: number;
  characterSpacing: number;
  wordSpacing: number;
  inText: boolean;
  renderMode: number;
  cms: EditCm[];
  clean: boolean;
  qToken: { start: number; end: number } | null;
  qPath: string | null;
  /** P6 U2: the linear part [a b c d] of the CTM (page user space at the page walk's start), or null when unproven. */
  lin: number[] | null;
  /** P6 U2: the linear part of the last `Tm` in this BT (identity at BT), or null when unproven. */
  tm: number[] | null;
  /** P6 U2: `TL`, set by `TL` and `TD`; null when an operand is not a number. */
  leading: number | null;
  /** P6 U3 fix S1: the whole CTM [a b c d e f] in page user space, or null when unproven. */
  ctm?: number[] | null;
  /** P6 U3 fix S1: the whole text line matrix Tlm, or null when unproven. */
  tlm?: number[] | null;
  /**
   * P6 U6 §1.1: the clip-recognition step since this state's `q` (0 = right after `q`,
   * 1 = after `re`, 2 = after `W`, 3 = after `n`; absent/-1 = no clip may start), the
   * pending `re`, and the finished clip.
   */
  clipStage?: number;
  clipPending?: EditClip | null;
  clipRe?: (EditClip & { path: string }) | null;
};

/** Everything a `/Contents` array walk carries between streams: text state and marked-content scopes. */
export type EditCarried = { current: EditTextState; saved: EditTextState[]; scopes: EditMarkedScope[] };

export type EditScanState = {
  page: MupdfPage;
  pageObject: MupdfObject;
  pdf: EditPdf;
  pageIndex: number;
  items: EditTarget[];
  records: Map<string, EditTargetRecord>;
  budget: { bytes: number; traversals: number; tokens: number; replyText: number };
  contentsRefs: Map<number, number>;
  xobjectRefs: Map<number, number>;
  runs: EditRun[] | null;
  activeForms: Set<number>;
  /** U3: one resolved font per font object, so a CMap is parsed once per page walk. */
  fontCache: Map<number, EditFont | null>;
  /** U3: every code drawn with a font object on this page, by that object's number. */
  repertoire: Map<number, Set<number>>;
  /** U3: candidate CID runs, patched with their page-scoped repertoire after the walk. */
  cidPending: { record: EditTargetRecord; item: EditTarget; cid: EditCid }[];
  /** U4: true when the page's `/Rotate` or `/UserUnit` makes every placement uneditable. */
  geometryBlocked: boolean;
  /** P6 U2: the linear part of MuPDF's page transform (user space → the `bounds` space). */
  pageLinear: number[];
  /** P6 U3 §9.1: MuPDF's whole page transform; `pageLinear` is its first four numbers. */
  pageTransform: number[];
  /** print-edit D2: the decoded bytes of every depth-0 stream, by path. */
  streams: Map<string, Uint8Array>;
  /** print-edit D1: each depth-0 stream's glyph blocks (by operand start), parsed once. */
  glyphBlocks: Map<string, Map<number, EditGlyphBlock>>;
  /** print-edit D3: emitted targets that can never join a glyph line (the 512 cap, before matching more). */
  ordinaryTargets: number;
  /** Review fix 3: a first walk that only counts targets (no structured-text matching). */
  countOnly?: boolean;
};

/** P6 U3 §9.1: a user-space box (y up) through a page transform: its 4 corners, min/max (y down). */
export function editViewBox(box: number[], m: number[]): number[] {
  const [a, b, c, d, e, f] = m;
  const points = [[box[0], box[1]], [box[2], box[1]], [box[0], box[3]], [box[2], box[3]]]
    .map(([x, y]) => [a * x + c * y + e, b * x + d * y + f]);
  const xs = points.map(point => point[0]), ys = points.map(point => point[1]);
  return [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)];
}

export type EditOperand =
  | { t: "num"; n: number; start: number; end: number }
  | { t: "name"; name: string; start: number; end: number }
  | { t: "str"; bytes: Uint8Array; start: number; end: number }
  | { t: "arr"; items: EditOperand[]; start: number; end: number }
  | { t: "dict"; start: number; end: number };

export type EditToken =
  | { k: "operand"; value: EditOperand }
  | { k: "open"; container: "array" | "dict"; start: number; end: number }
  | { k: "close"; container: "array" | "dict"; start: number; end: number }
  | { k: "op"; op: string; start: number; end: number };

/** The six-hex-digit subset prefix; a hint about a font, never proof of a usable encoding. */
export const EDIT_SUBSET_PREFIX = /^[A-Z]{6}\+/;

export function editIsWhite(byte: number | undefined): boolean {
  return byte === 0 || byte === 9 || byte === 10 || byte === 12 || byte === 13 || byte === 32;
}

export function editIsDelimiter(byte: number): boolean {
  return byte === 0x28 || byte === 0x29 || byte === 0x3c || byte === 0x3e || byte === 0x5b
    || byte === 0x5d || byte === 0x7b || byte === 0x7d || byte === 0x2f || byte === 0x25;
}

export function editIsRegular(byte: number): boolean {
  return !editIsWhite(byte) && !editIsDelimiter(byte);
}

export function editHexDigit(byte: number | undefined): number {
  if (byte === undefined) return -1;
  if (byte >= 0x30 && byte <= 0x39) return byte - 0x30;
  if (byte >= 0x41 && byte <= 0x46) return byte - 0x41 + 10;
  if (byte >= 0x61 && byte <= 0x66) return byte - 0x61 + 10;
  return -1;
}

export function editBytesToLatin1(bytes: Uint8Array): string {
  let text = "";
  for (let start = 0; start < bytes.length; start += 4096) {
    text += String.fromCharCode(...bytes.subarray(start, start + 4096));
  }
  return text;
}

export function editBytesEqual(left: Uint8Array, right: Uint8Array): boolean {
  if (left.length !== right.length) return false;
  for (let index = 0; index < left.length; index++) if (left[index] !== right[index]) return false;
  return true;
}

/** U3: split an Identity-H payload into its 2-byte codes (an odd tail has no code); coded: one code per byte. */
export function editCodesOf(bytes: Uint8Array, bytesPerCode: 1 | 2 = 2): number[] {
  if (bytesPerCode === 1) return [...bytes];
  const codes: number[] = [];
  for (let index = 0; index + 1 < bytes.length; index += 2) codes.push((bytes[index] << 8) | bytes[index + 1]);
  return codes;
}

/** U3: the decoded text of an Identity-H payload, or null for an odd length or an unmapped code. */
export function editDecodeCid(bytes: Uint8Array, cid: EditCid): { text: string; codes: number[] } | null {
  if (cid.bytesPerCode === 2 && bytes.length % 2 !== 0) return null;
  const codes = editCodesOf(bytes, cid.bytesPerCode);
  let text = "";
  for (const code of codes) {
    const char = cid.toUnicode.get(code);
    if (char === undefined) return null;
    text += char;
  }
  return { text, codes };
}

/** U3: the code-point ranges that map to read-only text (contract §2.2). */
export function editIsRtlOrShaping(text: string): boolean {
  for (const char of text) {
    const code = char.codePointAt(0) ?? 0;
    if ((code >= 0x0300 && code <= 0x036f) || (code >= 0x0590 && code <= 0x08ff)
      || (code >= 0x1ab0 && code <= 0x1aff) || (code >= 0x1dc0 && code <= 0x1dff)
      || (code >= 0x20d0 && code <= 0x20ff) || (code >= 0xfb1d && code <= 0xfdff)
      || (code >= 0xfe20 && code <= 0xfe2f) || (code >= 0xfe70 && code <= 0xfeff)) return true;
  }
  return false;
}

/**
 * The text of an `/ActualText` string operand: a `FE FF` BOM means UTF-16BE,
 * anything else is PDFDocEncoding as Latin-1 (the U2 rule). Null when it cannot
 * be decoded (an odd number of UTF-16 bytes).
 */
export function editDecodeWinAnsi(bytes: Uint8Array): string | null {
  let text = "";
  for (const byte of bytes) {
    const char = winAnsiChar(byte);
    if (char === null) return null;
    text += char;
  }
  return text;
}

export function editDecodeActualText(bytes: Uint8Array): string | null {
  if (bytes.length >= 2 && bytes[0] === 0xfe && bytes[1] === 0xff) {
    if ((bytes.length - 2) % 2 !== 0) return null;
    let text = "";
    for (let index = 2; index < bytes.length; index += 2) text += String.fromCharCode((bytes[index] << 8) | bytes[index + 1]);
    return text;
  }
  return editBytesToLatin1(bytes);
}

/**
 * U3: the page-scoped repertoire of one font object, in code order. A character
 * drawn by two different codes is ambiguous and left out of both the displayed
 * repertoire and the write's reverse map; the first 512 characters by code win.
 */
export function editCidRepertoire(cid: EditCid, codes: Set<number> | undefined): { text: string; chars: Map<string, number> } | null {
  if (!codes || codes.size === 0) return null;
  const first = new Map<string, number | null>();
  for (const code of [...codes].sort((left, right) => left - right)) {
    const char = cid.toUnicode.get(code);
    if (char === undefined) continue;
    if (!first.has(char)) first.set(char, code);
    else if (first.get(char) !== code) first.set(char, null);
  }
  const chars = new Map<string, number>();
  for (const [char, code] of first) {
    if (code === null) continue;
    if (chars.size >= 512) break;
    chars.set(char, code);
  }
  if (chars.size === 0) return null;
  return { text: [...chars.keys()].join(""), chars };
}

/**
 * U3: after the page walk, every candidate CID run gets the repertoire of its
 * own font object (contract §2.2). The walk is sequential, so this is the only
 * place that can see every code drawn with that font on this page.
 */
export function editAttachRepertoires(state: EditScanState): void {
  for (const { record, item, cid } of state.cidPending) {
    const repertoire = editCidRepertoire(cid, record.cidKey === undefined ? undefined : state.repertoire.get(record.cidKey));
    // No proven repertoire (a direct font dictionary has no object identity, or every
    // drawn character is ambiguous): nothing can be written, so it is not a candidate.
    if (!repertoire) {
      record.support = item.support = "unsupported";
      record.reason = item.reason = "unsupported-encoding";
      delete record.cow;
      delete item.cowable;
      continue;
    }
    record.repertoire = repertoire.text;
    record.cidWrite = { chars: repertoire.chars, widths: cid.widths, dw: cid.dw, bytesPerCode: cid.bytesPerCode };
    item.repertoire = repertoire.text;
  }
}

/** U3: the CID operand: 4 lowercase hex digits per code (2 for a coded simple font), `<>` when empty. */
export function editHexOperand(codes: number[], bytesPerCode: 1 | 2 = 2): Uint8Array {
  if (codes.length === 0) return new TextEncoder().encode("<>");
  return new TextEncoder().encode(`<${codes.map(code => code.toString(16).padStart(bytesPerCode * 2, "0")).join("")}>`);
}

/**
 * U3: the `/ActualText` replacement. Printable ASCII keeps the U2 literal; any
 * other text is a UTF-16BE hex string with a BOM, so extraction stays honest.
 */
export function editActualTextBytes(text: string): Uint8Array {
  let ascii = true;
  for (const char of text) {
    const code = char.codePointAt(0) ?? 0;
    if (code < 0x20 || code > 0x7e) { ascii = false; break; }
  }
  if (ascii) return editLiteral(text);
  let hex = "feff";
  for (let index = 0; index < text.length; index++) hex += text.charCodeAt(index).toString(16).padStart(4, "0");
  return new TextEncoder().encode(`<${hex}>`);
}

/**
 * U3: the width gate for an Identity-H run, from the recorded `/W`/`/DW` and the
 * run's own `Tf` size. Null when this build has no size, which is
 * `unmeasurable-width`.
 */
export function editMeasureCid(record: EditTargetRecord, next: string, ratio = 1): { oldWidth: number; newWidth: number; missing: string[] } | null {
  const write = record.cidWrite;
  const size = record.fontSize;
  if (!write || size === null || size === undefined || !Number.isFinite(size)) return null;
  const scale = record.horizontalScale ?? 1;
  const spacing = record.characterSpacing ?? 0;
  // Tw applies to the single-byte code 32 of a simple font only (never to a 2-byte code), whatever it draws.
  const words = write.bytesPerCode === 1 ? record.wordSpacing ?? 0 : 0;
  const widthOf = (codes: number[], adjustment = 0, grow = 1): number => (
    codes.reduce((total, code) => total + ((write.widths.get(code) ?? write.dw) / 1000) * size * grow, 0)
      - adjustment * size / 1000 + spacing * codes.length + words * codes.filter(code => code === 32).length
  ) * scale;
  const missing: string[] = [];
  const codes: number[] = [];
  for (const char of next) {
    const code = write.chars.get(char);
    if (code === undefined) missing.push(char);
    else codes.push(code);
  }
  const oldCodes = editCodesOf(record.payloadBytes ?? new Uint8Array(0), write.bytesPerCode);
  return { oldWidth: widthOf(oldCodes, record.tjAdjustment), newWidth: missing.length ? 0 : widthOf(codes, 0, ratio), missing };
}

/**
 * U4 (amended §7.6): a target box in page points. It must have positive sides of at
 * least 1 pt, stay inside MediaBox ∩ CropBox, and stay under the 14400 pt ceiling.
 */
export function editObjectBox(value: unknown, clip: number[]): number[] {
  if (!Array.isArray(value) || value.length !== 4 || value.some(item => typeof item !== "number" || !Number.isFinite(item))) {
    failEditObject("out-of-bounds", "edit-object: box must be four finite numbers [x0, y0, x1, y1]");
  }
  const [x0, y0, x1, y1] = value as number[];
  if (!(x1 - x0 >= 1) || !(y1 - y0 >= 1)) failEditObject("out-of-bounds", "edit-object: each side of the box must be at least 1 pt");
  if (x1 - x0 > MAX_IMAGE_PAGE_POINTS || y1 - y0 > MAX_IMAGE_PAGE_POINTS) {
    failEditObject("too-large", `edit-object: a side over the ${MAX_IMAGE_PAGE_POINTS} pt ceiling`);
  }
  if (x0 < clip[0] || y0 < clip[1] || x1 > clip[2] || y1 > clip[3]) {
    failEditObject("out-of-bounds", "edit-object: the box must stay inside the page");
  }
  return [x0, y0, x1, y1];
}

const IDENTITY_ORIENT: EditOrient = { swap: false, sx: 1, sy: 1 };

/**
 * U4: the `cm` operand that maps `rect` onto `box`: `a 0 0 d e f` (the operator stays).
 * P6 U6 §1.1: `0 b c 0 e f` for a swapped orientation; `e`/`f` put the mapped rect's
 * bounding box exactly on `box`. The default orientation writes today's bytes.
 */
export function editPlacementMatrix(box: number[], rect: number[], orient: EditOrient = IDENTITY_ORIENT): string {
  if (!orient.swap) {
    const a = orient.sx * (box[2] - box[0]) / (rect[2] - rect[0]);
    const d = orient.sy * (box[3] - box[1]) / (rect[3] - rect[1]);
    const e = box[0] - Math.min(a * rect[0], a * rect[2]);
    const f = box[1] - Math.min(d * rect[1], d * rect[3]);
    return `${editNumber(a)} 0 0 ${editNumber(d)} ${editNumber(e)} ${editNumber(f)}`;
  }
  const c = orient.sx * (box[2] - box[0]) / (rect[3] - rect[1]);
  const b = orient.sy * (box[3] - box[1]) / (rect[2] - rect[0]);
  const e = box[0] - Math.min(c * rect[1], c * rect[3]);
  const f = box[1] - Math.min(b * rect[0], b * rect[2]);
  return `0 ${editNumber(b)} ${editNumber(c)} 0 ${editNumber(e)} ${editNumber(f)}`;
}

/** P6 U6 §1.1: the orientation of a `cm` (plain `a 0 0 d` or swapped `0 b c 0`), else null. */
export function editOrientOf(matrix: number[]): EditOrient | null {
  if (matrix.length < 4 || matrix.slice(0, 6).some(value => !Number.isFinite(value))) return null;
  const [a, b, c, d] = matrix;
  if (b === 0 && c === 0 && a !== 0 && d !== 0) return { swap: false, sx: a > 0 ? 1 : -1, sy: d > 0 ? 1 : -1 };
  if (a === 0 && d === 0 && b !== 0 && c !== 0) return { swap: true, sx: c > 0 ? 1 : -1, sy: b > 0 ? 1 : -1 };
  return null;
}

/** P6 U6: the normalised bounding box of `rect`'s four corners under `matrix` (row-vector). */
export function editMapRect(matrix: number[], rect: number[]): number[] {
  const xs: number[] = [];
  const ys: number[] = [];
  for (const [x, y] of [[rect[0], rect[1]], [rect[2], rect[1]], [rect[0], rect[3]], [rect[2], rect[3]]]) {
    xs.push(matrix[0] * x + matrix[2] * y + matrix[4]);
    ys.push(matrix[1] * x + matrix[3] * y + matrix[5]);
  }
  return [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)];
}

/** P6 U6 §1.2: the axis-aligned affine that maps rect `from` onto rect `to`. */
export function editRectAffine(from: number[], to: number[]): number[] {
  const sx = (to[2] - to[0]) / (from[2] - from[0]);
  const sy = (to[3] - to[1]) / (from[3] - from[1]);
  return [sx, 0, 0, sy, to[0] - sx * from[0], to[1] - sy * from[1]];
}

/** P6 U6 §1.2/§9.12: the rotation or flip about `rect`'s centre, as a page-space affine. */
export function editTransformAbout(transform: string, rect: number[]): number[] | null {
  const linear = transform === "rotate-cw" ? [0, -1, 1, 0] : transform === "rotate-ccw" ? [0, 1, -1, 0]
    : transform === "flip-h" ? [-1, 0, 0, 1] : transform === "flip-v" ? [1, 0, 0, -1] : null;
  if (!linear) return null;
  const cx = (rect[0] + rect[2]) / 2;
  const cy = (rect[1] + rect[3]) / 2;
  return editAffine(editAffine([1, 0, 0, 1, -cx, -cy], [...linear, 0, 0]), [1, 0, 0, 1, cx, cy]);
}

/** P6 U6 §9.3: the four `re` operands of a clip rect, `x y w h`. */
export function editClipOperands(rect: number[]): string {
  return `${editNumber(rect[0])} ${editNumber(rect[1])} ${editNumber(rect[2] - rect[0])} ${editNumber(rect[3] - rect[1])}`;
}

/**
 * P6 U6 §1.2: one placement write as a page-space affine `g` applied to the `cm` and,
 * when present, the clip rect. Returns the new full box, orientation, clip rect, visible
 * rect and the `cm`/clip text; `null` clip text means "no clip".
 */
export function editPlacementAfter(placement: EditPlacement, g: number[], box?: number[]): {
  full: number[]; orient: EditOrient; clip: number[] | null; visible: number[]; cm: string; clipText: string | null;
} {
  const orient = editOrientOf(editAffine(placement.matrix, g)) ?? placement.orient;
  // `box` is what `g` maps the visible rect onto: taken verbatim so an unclipped write is
  // byte-identical to the pre-U6 `editPlacementMatrix(box, rect)`.
  const clip = placement.clip ? editMapRect(g, placement.clip.rect) : null;
  const full = box && !clip ? box : editMapRect(g, placement.box);
  const visible = box ?? editMapRect(g, placement.visible);
  return { full, orient, clip, visible, cm: editPlacementMatrix(full, placement.rect, orient), clipText: clip ? editClipOperands(clip) : null };
}

/** P6 U6: one planned placement write: the `cm` text (null = untouched) and the clip text (null = no clip). */
export type EditPlacementWrite = { cm: string | null; clipText: string | null; noop: boolean };

/**
 * P6 U6 §1.2/§9.9/§9.10: plan a `move`/`resize`/`transform`/`crop` of one placement. Every
 * check runs here, before any write: `box` against the page (`editObjectBox`), the
 * transform token, crop containment, the uncropped-box ceiling. `noop` = the computed
 * `cm` and clip text equal today's (a `transform` is never a no-op).
 */
export function editPlanPlacement(placement: EditPlacement, action: string, rawBox: unknown, transform: unknown, pageClip: number[]): EditPlacementWrite {
  const currentCm = editPlacementMatrix(placement.box, placement.rect, placement.orient);
  const currentClip = placement.clip ? editClipOperands(placement.clip.rect) : null;
  if (action === "crop") {
    const box = editObjectBox(rawBox, pageClip);
    const full = placement.box;
    if (box[0] < full[0] - 0.01 || box[1] < full[1] - 0.01 || box[2] > full[2] + 0.01 || box[3] > full[3] + 0.01) {
      failEditObject("out-of-bounds", "edit-object: a crop must stay inside the picture");
    }
    const whole = box.every((value, index) => Math.abs(value - full[index]) <= 0.01);
    const clipText = whole ? null : editClipOperands(box);
    return { cm: null, clipText, noop: clipText === currentClip };
  }
  let g: number[];
  let box: number[];
  if (action === "transform") {
    const t = editTransformAbout(String(transform), placement.visible);
    if (!t) failEditObject("invalid-transform", `edit-object: transform must be rotate-cw, rotate-ccw, flip-h or flip-v`);
    const turned = editMapRect(t, placement.visible);
    box = editObjectBox(rawBox === undefined ? turned : rawBox, pageClip);
    g = editAffine(t, editRectAffine(turned, box));
  } else {
    box = editObjectBox(rawBox, pageClip);
    g = editRectAffine(placement.visible, box);
  }
  const after = editPlacementAfter(placement, g, box);
  editCheckFull(after.full);
  return { cm: after.cm, clipText: after.clipText, noop: action !== "transform" && after.cm === currentCm && after.clipText === currentClip };
}

/**
 * P6 U6 §9.9: a written full box must be finite and within the side ceiling (it may
 * leave the page when a clip hides that part).
 */
export function editCheckFull(full: number[]): void {
  if (full.some(value => !Number.isFinite(value)) || full[2] - full[0] > MAX_IMAGE_PAGE_POINTS || full[3] - full[1] > MAX_IMAGE_PAGE_POINTS) {
    failEditObject("too-large", `edit-object: the uncropped box would be over the ${MAX_IMAGE_PAGE_POINTS} pt ceiling`);
  }
}

/**
 * P6 U6 §1.2: the splice spans of one placement write: the `cm` and, when the clip
 * changes, its operands (or the insert/removal of the whole clip).
 */
export function editPlacementSpans(placement: EditPlacement, cm: string | null, clipText: string | null): { start: number; end: number; replacement: Uint8Array }[] {
  const encoder = new TextEncoder();
  const spans = cm === null ? [] : [{ start: placement.cmStart, end: placement.cmEnd, replacement: encoder.encode(cm) }];
  const clip = placement.clip;
  if (clip && clipText !== null) spans.push({ start: clip.opsStart, end: clip.opsEnd, replacement: encoder.encode(clipText) });
  else if (clip) spans.push({ start: clip.start, end: clip.end, replacement: new Uint8Array(0) });
  else if (clipText !== null) {
    const qEnd = placement.qStart + 1;
    spans.push({ start: qEnd, end: qEnd, replacement: encoder.encode(`\n${clipText} re W n`) });
  }
  return spans;
}

/**
 * P6 U5: a copy of one simple placement block at `box`, inserted right after its `Q`
 * (start = end = `qEnd`), so the copy is the next top-level block and paints above it.
 * The copy keeps the same resource name: the XObject is shared, never duplicated.
 */
export function editDuplicateEdit(bytes: Uint8Array, placement: EditPlacement, box: number[]): { start: number; end: number; replacement: Uint8Array } {
  // P6 U6 §9.4: the copy's clip and `cm` both follow the affine old visible → `box`.
  const after = editPlacementAfter(placement, editRectAffine(placement.visible, box), box);
  const clip = placement.clip;
  const head = clip && after.clipText !== null
    ? concatBytes(bytes.subarray(placement.qStart, clip.opsStart), new TextEncoder().encode(after.clipText), bytes.subarray(clip.opsEnd, placement.cmStart))
    : bytes.subarray(placement.qStart, placement.cmStart);
  const cm = new TextEncoder().encode(after.cm);
  const tail = bytes.subarray(placement.cmEnd, placement.qEnd);
  const replacement = new Uint8Array(1 + head.length + cm.length + tail.length);
  replacement[0] = 10;
  replacement.set(head, 1);
  replacement.set(cm, 1 + head.length);
  replacement.set(tail, 1 + head.length + cm.length);
  return { start: placement.qEnd, end: placement.qEnd, replacement };
}

/**
 * U4: a PDF number: at most four decimals, no exponent, `-0` as `0`, trailing zeros
 * stripped. Every placement rewrite goes through this, so a saved stream stays small.
 */
export function editNumber(value: number): string {
  const rounded = Math.round(value * 10000) / 10000;
  return Object.is(rounded, -0) ? "0" : String(rounded);
}

/**
 * U4: the end of the `Q` that follows a `Do` with only whitespace and comments
 * between them, or null. A simple block is `q … cm … Do Q`, so the delete span can
 * reach `qEnd` without a second look at the stream.
 */
export function editNextQEnd(bytes: Uint8Array, from: number): number | null {
  let position = from;
  for (;;) {
    while (position < bytes.length && editIsWhite(bytes[position])) position++;
    if (bytes[position] !== 0x25) break;
    while (position < bytes.length && bytes[position] !== 10 && bytes[position] !== 13) position++;
  }
  if (bytes[position] !== 0x51) return null;
  const end = position + 1;
  if (end < bytes.length && !editIsWhite(bytes[end]) && !editIsDelimiter(bytes[end])) return null;
  return end;
}

/** One `/BBox`-shaped rect: four finite numbers, normalized, or null. */
/** P6 U2: `Td`/`TD` operands added to a running text-space translation, or null. */
export function editTranslate(between: number[] | null, operands: EditOperand[]): number[] | null {
  if (!between || operands.length !== 2 || operands[0].t !== "num" || operands[1].t !== "num") return null;
  return [between[0] + operands[0].n, between[1] + operands[1].n];
}

/** P6 U2: a PDF array of exactly `count` finite numbers, or null. */
export function editNumbersOf(object: MupdfObject, count: number): number[] | null {
  if (!object.isArray() || object.length !== count) return null;
  const numbers: number[] = [];
  for (let index = 0; index < count; index++) {
    const value = object.get(index) as MupdfObject;
    if (!value.isNumber() || !Number.isFinite(value.asNumber())) return null;
    numbers.push(value.asNumber());
  }
  return numbers;
}

/** P6 U2: `left × right` for linear parts [a b c d] (PDF row-vector order). */
export function editMultiply(left: number[], right: number[]): number[] {
  const [a, b, c, d] = left;
  const [e, f, g, h] = right;
  return [a * e + b * g, a * f + b * h, c * e + d * g, c * f + d * h];
}

/** P6 U3 fix S1: `left × right` for whole matrices [a b c d e f] (PDF row-vector order). */
export function editAffine(left: number[], right: number[]): number[] {
  return [...editMultiply(left, right), left[4] * right[0] + left[5] * right[2] + right[4], left[4] * right[1] + left[5] * right[3] + right[5]];
}

/** P6 U3 fix S1: `[tx ty] Td` on the line matrix, or null when either is unproven. */
export function editLineMove(tlm: number[] | null | undefined, move: number[] | null): number[] | null {
  return tlm && move ? editAffine([1, 0, 0, 1, move[0], move[1]], tlm) : null;
}

/** P6 U2: the inverse of a linear part, or null when it is not invertible. */
export function editInvert(matrix: number[]): number[] | null {
  const [a, b, c, d] = matrix;
  const det = a * d - b * c;
  if (!Number.isFinite(det) || Math.abs(det) < 1e-9) return null;
  return [d / det, -b / det, -c / det, a / det];
}

export function editRectOf(object: MupdfObject | null): number[] | null {
  if (!object || !object.isArray() || object.length !== 4) return null;
  const numbers: number[] = [];
  for (let index = 0; index < 4; index++) {
    const value = object.get(index) as MupdfObject;
    if (!value.isNumber()) return null;
    numbers.push(value.asNumber());
  }
  if (numbers.some(number => !Number.isFinite(number))) return null;
  return [
    Math.min(numbers[0], numbers[2]), Math.min(numbers[1], numbers[3]),
    Math.max(numbers[0], numbers[2]), Math.max(numbers[1], numbers[3]),
  ];
}

/**
 * U4: the page box a placement must stay inside: MediaBox intersected with CropBox
 * (the page's own box, unrotated), never the displayed box.
 */
export function editPageClipBox(page: MupdfObject | null): number[] {
  const media = editRectOf(at(page, "MediaBox")) ?? [0, 0, 612, 792];
  const crop = editRectOf(at(page, "CropBox"));
  if (!crop) return media;
  const box = [
    Math.max(media[0], crop[0]), Math.max(media[1], crop[1]),
    Math.min(media[2], crop[2]), Math.min(media[3], crop[3]),
  ];
  return box[2] > box[0] && box[3] > box[1] ? box : media;
}

/**
 * U4: a `/Matrix` for a Form: absent means identity, and only scale+translate with
 * positive factors can be resized. Anything else is not a simple placement.
 */
export function editFormMatrix(object: MupdfObject): number[] | null {
  const matrix = at(object, "Matrix");
  if (!matrix) return [1, 0, 0, 1, 0, 0];
  if (!matrix.isArray() || matrix.length !== 6) return null;
  const numbers: number[] = [];
  for (let index = 0; index < 6; index++) {
    const value = matrix.get(index) as MupdfObject;
    if (!value.isNumber()) return null;
    numbers.push(value.asNumber());
  }
  if (numbers.some(number => !Number.isFinite(number))) return null;
  if (numbers[1] !== 0 || numbers[2] !== 0 || !(numbers[0] > 0) || !(numbers[3] > 0)) return null;
  return numbers;
}

/** U5: one preview match and one rejected run, as the reply reports them. */
export type EditReplaceMatch = {
  targetId: string;
  text: string;
  next: string;
  count: number;
  oldWidth: number;
  newWidth: number;
  tailFollows: boolean;
};
export type EditReplaceRejected = { targetId: string; text: string; count: number; detail: string; [key: string]: unknown };

/** U2/U3/U5: one validated text replacement, ready to write inside a mutate. */
export type EditTextPlan = {
  record: EditTargetRecord;
  next: string;
  elementIndex: number;
  operand: Uint8Array;
  tag: Uint8Array;
  oldWidth: number;
  newWidth: number;
  limit: number;
  shift: number;
  /** P6 U2 §2.2/§2.3: bytes written before the operand and after the operator (move, size). */
  prefix: string;
  suffix: string;
  /** print-edit D4: a glyph line whose bytes change (text or rounded geometry), so a pure move is no no-op. */
  rewrite?: boolean;
};

/** P6 U2 §3.5: one raw span rewrite in a /Contents element (the paragraph span). */
export type EditSpanEdit = { record: EditTargetRecord; start: number; end: number; replacement: Uint8Array };

/** P6 U2 §2.1: the optional geometry of one `edit-text`. */
export type EditGeometry = { move?: { dx: number; dy: number }; size?: number };

/** P6 U2: a resource name as PDF source, `#xx`-escaping anything not regular. */
export function editNameSource(name: string): string {
  let out = "/";
  for (const byte of new TextEncoder().encode(name)) {
    out += byte > 0x20 && byte < 0x7f && byte !== 0x23 && editIsRegular(byte) ? String.fromCharCode(byte) : `#${byte.toString(16).padStart(2, "0")}`;
  }
  return out;
}

/**
 * P6 U2 §2.2/§2.3/§9.1–6: `tx ty Td` / `size Tf` before the show and their exact inverses
 * after it. Returns the ratio the width rule scales by and the moved/resized box.
 */
export function editPlanGeometry(record: EditTargetRecord, geometry: EditGeometry | undefined): { prefix: string; suffix: string; ratio: number; offset: number[] } {
  const none = { prefix: "", suffix: "", ratio: 1, offset: [0, 0] };
  if (!geometry || (geometry.move === undefined && geometry.size === undefined)) return none;
  let prefix = "";
  let suffix = "";
  let ratio = 1;
  let offset = [0, 0];
  if (geometry.size !== undefined) {
    const size = geometry.size;
    if (typeof size !== "number" || !Number.isFinite(size) || size < 1 || size > 400) {
      failEditText("invalid", "edit-text: size is a number of page points, 1..400");
    }
    const scale = editTextScale(record);
    if (scale === null) failEditText("not-candidate", "edit-text: this run's text matrix is not proven", { reason: "matrix" });
    const old = record.tfSize!;
    const next = editNumber(Math.sign(old) * size / scale);
    if (Number(next) === 0) failEditText("invalid", "edit-text: size rounds to zero");
    if (next !== editNumber(old)) {
      ratio = Number(next) / old;
      const font = editNameSource(record.tfName!);
      prefix = `${font} ${next} Tf `;
      suffix = ` ${font} ${editNumber(old)} Tf`;
    }
  }
  if (geometry.move !== undefined) {
    const move = geometry.move;
    if (!move || typeof move !== "object" || !Number.isFinite(move.dx) || !Number.isFinite(move.dy)) {
      failEditText("invalid", "edit-text: move is { dx, dy } in page points");
    }
    const refusal = editMoveRefusal(record);
    if (refusal) failEditText(refusal.detail, `edit-text: this run cannot be moved (${refusal.reason})`, { reason: refusal.reason });
    const inverse = editInvert(record.textLinear!)!;
    const tx = editNumber(move.dx * inverse[0] + move.dy * inverse[2]);
    const ty = editNumber(move.dx * inverse[1] + move.dy * inverse[3]);
    if (tx !== "0" || ty !== "0") {
      // §9.5: the restore is the textual negation of the same strings, so Tlm is exact.
      const negate = (value: string) => value === "0" ? "0" : value.startsWith("-") ? value.slice(1) : `-${value}`;
      prefix = `${tx} ${ty} Td ${prefix}`;
      suffix = `${suffix} ${negate(tx)} ${negate(ty)} Td`;
      const linear = record.textLinear!;
      offset = [Number(tx) * linear[0] + Number(ty) * linear[2], Number(tx) * linear[1] + Number(ty) * linear[3]];
    }
  }
  return { prefix, suffix, ratio, offset };
}

/**
 * The one text validator: the U2 encoding cut, the U3 CID repertoire, the width/tail
 * rules and the tag form. `edit-text`, `edit-replace-preview` and `edit-replace-apply`
 * all go through it, so a replacement is never judged by two different rules.
 * `borrowed` is an already loaded page the caller owns; otherwise one is loaded here.
 */
export function editPlanText(doc: MupdfDocument, record: EditTargetRecord, next: string, borrowed: MupdfPage | null = null, runs?: EditRun[], geometry?: EditGeometry): EditTextPlan {
  if (record.glyphLine) return editPlanGlyphLine(doc, record, record.glyphLine, next, borrowed, geometry);
  const placed = editPlanGeometry(record, geometry);
  if (next.length > MAX_EDIT_TEXT_CHARS) {
    failEditText("too-large", `edit-text: text must be at most ${MAX_EDIT_TEXT_CHARS} characters`);
  }
  // A proven Identity-H run is written as hex over its own repertoire; a simple font
  // keeps the U2 cut (printable ASCII only, a literal operand).
  const cid = record.cidWrite !== undefined;
  if (!cid && record.font?.subtype === "Type0") {
    failEditText("not-candidate", "edit-text: this run's font has no proven repertoire", { reason: "unsupported-encoding" });
  }
  const winAnsi = record.winAnsiEncoding === true;
  if (!cid) {
    const bad: string[] = [];
    for (const char of next) {
      const code = char.codePointAt(0) ?? 0;
      if (code < 0x20 || (winAnsi ? winAnsiByte(code) < 0 : code > 0x7e)) bad.push(char);
    }
    if (bad.length) {
      failEditText("unsupported-encoding", winAnsi ? "edit-text: text is not WinAnsi-encodable" : "edit-text: only printable ASCII can be written", { missing: bad.slice(0, 20) });
    }
  }
  const page = borrowed ?? (doc.loadPage(record.page) as MupdfPage);
  let measured: { oldWidth: number; newWidth: number; missing: string[] } | null;
  let clip: number[] | null = null;
  try {
    measured = cid ? editMeasureCid(record, next, placed.ratio) : editMeasureRun(page, record, next, runs, placed.ratio);
    if (placed.prefix) clip = attempt(() => [...page.getBounds()] as number[], null);
  } finally {
    if (!borrowed) page.destroy();
  }
  if (!measured) {
    failEditText("unmeasurable-width", "edit-text: this run's advance cannot be measured in this build");
  }
  if (measured.missing.length) {
    failEditText("unsupported-encoding", "edit-text: the run's font has no glyph for some characters", { missing: measured.missing.slice(0, 20) });
  }
  const { oldWidth, newWidth, limit, shift, elementIndex } = editPlanWidth(record, measured, placed.prefix !== "", placed.offset, placed.ratio, clip);
  // The operand and the tag are the same bytes for U2, and differ for a CID run: the
  // operand is hex over the codes, the ActualText may be UTF-16BE.
  let operand: Uint8Array;
  if (cid) {
    const chars = record.cidWrite?.chars ?? new Map<string, number>();
    const codes: number[] = [];
    for (const char of next) {
      const code = chars.get(char);
      if (code !== undefined) codes.push(code);
    }
    operand = editHexOperand(codes, record.cidWrite?.bytesPerCode);
  } else if (winAnsi) {
    operand = concatBytes(new Uint8Array([0x28]), winAnsiBytes(next), new Uint8Array([0x29]));
  } else {
    operand = editLiteral(next);
  }
  return { record, next, elementIndex, operand, tag: cid ? editActualTextBytes(next) : operand, oldWidth, newWidth, limit, shift, prefix: placed.prefix, suffix: placed.suffix };
}

/**
 * print-edit D4: a glyph line is re-emitted one unclipped block per glyph. Unchanged text keeps
 * its spacing (scaled about x0); changed text is laid out at natural advances plus the line's
 * word space. The whole chain is the operand, so `prefix`/`suffix` stay empty.
 */
function editPlanGlyphLine(doc: MupdfDocument, record: EditTargetRecord, line: EditGlyphLine, raw: string, borrowed: MupdfPage | null, geometry?: EditGeometry): EditTextPlan {
  if (raw.length > MAX_EDIT_TEXT_CHARS) {
    failEditText("too-large", `edit-text: text must be at most ${MAX_EDIT_TEXT_CHARS} characters`);
  }
  const next = raw.replace(/\s+/g, " ").trim();
  let scale = line.scale;
  let dx = 0;
  let dy = 0;
  if (geometry?.size !== undefined) {
    const size = geometry.size;
    if (typeof size !== "number" || !Number.isFinite(size) || size < 1 || size > 400) {
      failEditText("invalid", "edit-text: size is a number of page points, 1..400");
    }
    scale = size;
  }
  if (geometry?.move !== undefined) {
    const move = geometry.move;
    if (!move || typeof move !== "object" || !Number.isFinite(move.dx) || !Number.isFinite(move.dy)) {
      failEditText("invalid", "edit-text: move is { dx, dy } in page points");
    }
    dx = move.dx;
    dy = move.dy;
  }
  const write = record.cidWrite!;
  const missing = [...next].filter(char => char !== " " && !write.chars.has(char));
  if (missing.length) {
    failEditText("unsupported-encoding", "edit-text: the run's font has no glyph for some characters", { missing: missing.slice(0, 20) });
  }
  const ratio = scale / line.scale;
  const x0 = line.glyphs[0].x;
  const left = x0 + dx;
  const y = line.y - dy;
  const same = next === record.text;
  const placed: { code: number; x: number; adv: number }[] = [];
  if (same) {
    for (const glyph of line.glyphs) placed.push({ code: glyph.code, x: left + (glyph.x - x0) * ratio, adv: glyph.adv * ratio });
  } else {
    let pen = left;
    let gap = false;
    for (const char of next) {
      if (char === " ") { gap = true; continue; }
      if (gap) pen += line.space * scale;
      gap = false;
      const code = write.chars.get(char)!;
      const adv = (write.widths.get(code) ?? write.dw) / 1000 * scale;
      placed.push({ code, x: pen, adv });
      pen += adv;
    }
  }
  // Rounded as written, so a move below the number grain is still the source's bytes.
  const geometric = editNumber(scale) !== editNumber(line.scale) || editNumber(dx) !== "0" || editNumber(dy) !== "0";
  let clip: number[] | null = null;
  if (geometric) {
    const page = borrowed ?? (doc.loadPage(record.page) as MupdfPage);
    try {
      clip = attempt(() => [...page.getBounds()] as number[], null);
    } finally {
      if (!borrowed) page.destroy();
    }
  }
  const bounds = record.bounds ?? [x0, 0, x0, 0];
  const last = placed[placed.length - 1];
  const measured = { oldWidth: bounds[2] - bounds[0], newWidth: last ? last.x + last.adv - left : 0 };
  const { oldWidth, newWidth, limit, shift, elementIndex } = editPlanWidth(record, measured, geometric, [dx, dy], ratio, clip);
  const font = editNameSource(record.tfName!);
  const colour = line.colour ? `${line.colour} ` : "";
  const size = editNumber(scale);
  const blocks = placed.map(glyph => `q ${colour}q ${size} 0 0 ${size} ${editNumber(glyph.x)} ${editNumber(y)} cm BT 1 0 0 1 0 0 Tm ${font} 1 Tf <${glyph.code.toString(16).padStart(2, "0")}> Tj ET Q Q`);
  return {
    record, next, elementIndex, operand: new TextEncoder().encode(blocks.join("\n")), tag: editActualTextBytes(next),
    oldWidth, newWidth, limit, shift, prefix: "", suffix: "", rewrite: !same || geometric,
  };
}

/**
 * The width, growth, page and element rules every text plan runs (print-edit D4.5: the ordinary
 * run and the glyph line share these lines). `geometric` = a move or size is written, `offset`
 * the move in `bounds` space, `ratio` the size change, `clip` the page box (geometric only).
 */
function editPlanWidth(
  record: EditTargetRecord, measured: { oldWidth: number; newWidth: number }, geometric: boolean,
  offset: number[], ratio: number, clip: number[] | null,
): { oldWidth: number; newWidth: number; limit: number; shift: number; elementIndex: number } {
  const clipRight = clip ? clip[2] - (offset[0]) : null;
  const oldWidth = Math.round(measured.oldWidth * 100) / 100;
  const newWidth = Math.round(measured.newWidth * 100) / 100;
  const delta = newWidth - oldWidth;
  // P6 U2: a size change is an explicit gesture, so the text-margin fallback does not bind it:
  // the nearest obstacle on the line, else the page's right edge; a move checks the page only.
  const geometryLimit = offset[0] !== 0 || offset[1] !== 0
    ? clipRight
    : record.obstacleLimit ?? clipRight;
  const limit = Math.round(((geometric ? geometryLimit : null) ?? record.growthLimit ?? record.bounds?.[2] ?? oldWidth) * 100) / 100;
  if (delta > 0.5) {
    if (record.tailFollows && record.leftToRightTail === false) {
      failEditText("tail-would-move", "edit-text: following text is not left-to-right", { oldWidth, newWidth, limit });
    }
    const right = record.tailFollows
      ? (record.tailRight ?? record.bounds?.[2] ?? oldWidth) + delta
      : (record.bounds?.[0] ?? 0) + newWidth;
    if (right > limit) {
      const detail = record.tailFollows ? "tail-would-move" : "overflow";
      failEditText(detail, `edit-text: the replacement would cross the ${limit} pt free-space limit`, { oldWidth, newWidth, limit });
    }
  }
  const shift = record.tailFollows ? Math.round(delta * 100) / 100 : 0;
  // §2.2/§9.6: the moved, resized box stays on the page (scaled about its bottom-left, the baseline side).
  if (geometric && record.bounds && clip) {
    const [x0, y0, , y1] = record.bounds;
    const box = [
      x0 + offset[0], y1 - (y1 - y0) * Math.abs(ratio) + offset[1],
      x0 + newWidth + offset[0], y1 + offset[1],
    ];
    if (box[0] < clip[0] - 0.5 || box[1] < clip[1] - 0.5 || box[2] > clip[2] + 0.5 || box[3] > clip[3] + 0.5) {
      failEditText("outside-page", "edit-text: the moved text would leave the page", { bounds: box.map(value => Math.round(value * 100) / 100) });
    }
  }
  // A depth-1 Form descendant names the page element that invokes it (U1b).
  const elementIndex = record.cow?.form ? record.cow.form.elementIndex : Number(/^c(\d+)$/.exec(record.path)?.[1] ?? NaN);
  if (!Number.isInteger(elementIndex)) failEditText("not-candidate", "edit-text: only top-level page content is editable");
  return { oldWidth, newWidth, limit, shift, elementIndex };
}

/**
 * The one text writer, called inside a mutate. Plans are grouped by /Contents element;
 * each element is privatised at most once (§7.2: U1b copy-on-write is not idempotent)
 * and spliced exactly once (§7.4), so a batch is one atomic rewrite. StructElem
 * ActualText puts stay per plan.
 */
export function editWritePlans(
  pdf: EditPdf, pageView: MupdfPage, plans: EditTextPlan[], privatised: Set<number>,
  fail: (error: unknown, targetId: string) => never,
): void {
  const groups = new Map<number, EditTextPlan[]>();
  for (const plan of plans) {
    // Review fix 4: a glyph line whose text and geometry are unchanged keeps its bytes (and clips).
    if (plan.record.glyphLine && !plan.rewrite) continue;
    const group = groups.get(plan.elementIndex);
    if (group) group.push(plan);
    else groups.set(plan.elementIndex, [plan]);
  }
  for (const [elementIndex, group] of [...groups.entries()].sort((left, right) => left[0] - right[0])) {
    const ordered = [...group].sort((left, right) => right.record.opStart - left.record.opStart);
    try {
      editWriteElement(pdf, pageView, elementIndex, ordered, privatised);
    } catch (err) {
      fail(err, ordered[0].record.targetId);
    }
  }
}

/** One /Contents element's plans: privatise once, splice once, then the tag puts. */
export function editWriteElement(pdf: EditPdf, pageView: MupdfPage, elementIndex: number, ordered: EditTextPlan[], privatised: Set<number>, spans: EditSpanEdit[] = []): void {
  const pageObject = pageView.getObject() as MupdfObject;
  const contents = at(pageObject, "Contents");
  const element = contents?.isArray()
    ? contents.get(elementIndex) as MupdfObject
    : elementIndex === 0 ? contents : null;
  if (!element || !element.isStream()) {
    failReason("invalid", "edit-text: the content stream is no longer where it was");
  }
  // U1b: privatise this placement first, in the same operation; the record's spans
  // hold because every copy is proven byte-identical to its source.
  let stream = element;
  const cowRecord = ordered.find(plan => plan.record.cow)?.record ?? spans.find(span => span.record.cow)?.record;
  if (cowRecord && !privatised.has(elementIndex)) {
    stream = editPrivatise(pdf, pageObject, contents as MupdfObject, element, elementIndex, cowRecord);
    privatised.add(elementIndex);
  }
  const bytes = editReadStream(stream, "edit-text");
  const edits: { start: number; end: number; replacement: Uint8Array }[] = spans.map(span => ({ start: span.start, end: span.end, replacement: span.replacement }));
  for (const plan of ordered) {
    const encoder = new TextEncoder();
    edits.push({ start: plan.record.operandStart, end: plan.record.operandEnd, replacement: plan.prefix ? concatBytes(encoder.encode(plan.prefix), plan.operand) : plan.operand });
    // A kerned array becomes one ordinary text-show; only this run loses kerning.
    // P6 U2: the move/size restore follows the operator.
    if (plan.record.kerned || plan.suffix) {
      const op = plan.record.kerned ? "Tj" : editBytesToLatin1(bytes.subarray(plan.record.opStart, plan.record.opEnd));
      edits.push({ start: plan.record.opStart, end: plan.record.opEnd, replacement: encoder.encode(op + plan.suffix) });
    }
    // The tag travels with the operand so extraction stays honest.
    if (plan.record.actualText) {
      edits.push({ start: plan.record.actualText.start, end: plan.record.actualText.end, replacement: plan.tag });
    }
  }
  // One splice for the element: any overlapping span refuses the whole operation.
  stream.writeStream(editSplice(bytes, edits));
  for (const plan of ordered) {
    const record = plan.record;
    if (record.mcid !== null && record.mcid !== undefined && record.structActualText !== undefined) {
      const structElement = editStructElement(pdf, pageObject, record.mcid);
      if (!structElement) {
        failReason("invalid", "edit-text: the tagged element is no longer reachable");
      }
      structElement.put("ActualText", pdf.newString(plan.next));
    }
  }
}

/** One plan through the same writer (a single `edit-text`). */
export function editWriteText(pdf: EditPdf, pageView: MupdfPage, plan: EditTextPlan): void {
  editWritePlans(pdf, pageView, [plan], new Set(), (error) => {
    // A single edit keeps the writer's own refusal token (too-large, invalid, ...).
    const tagged = error as { reason?: string; detail?: string } | null;
    if (tagged?.reason !== undefined || tagged?.detail !== undefined) throw error;
    failReason("invalid", errorMessage(error));
  });
}

/**
 * The replacement bytes for a text operand: a literal string, escaped, never hex,
 * so the next discovery finds the same operator shape.
 */
export function editLiteral(text: string): Uint8Array {
  return new TextEncoder().encode(`(${text.replace(/[\\()]/g, "\\$&")})`);
}

/** The decoded bytes of one stream, or a reason-tagged failure. */
export function editReadStream(stream: MupdfObject, op: string): Uint8Array {
  try {
    const buffer = stream.readStream();
    try {
      return buffer.asUint8Array().slice();
    } finally {
      buffer.destroy();
    }
  } catch (err) {
    if ((err as { reason?: string } | null)?.reason) throw err;
    return failReason("invalid", `${op}: a content stream cannot be read (${errorMessage(err)})`);
  }
}

/**
 * U1b copy-on-write, inside the caller's journal operation: inherited /Resources
 * are copied onto the page, a shared /Contents element is replaced by a private
 * copy, and a Form invocation is grafted, registered under a fresh name and this
 * one `Do` renamed. Returns the stream the edit writes into. The originals and
 * every other placement are never touched; each copy is proven byte-identical
 * to its source so the record's spans still name the same operator.
 */
export function editPrivatise(pdf: EditPdf, pageObject: MupdfObject, contents: MupdfObject, element: MupdfObject, elementIndex: number, record: EditTargetRecord): MupdfObject {
  const cow = record.cow as EditCow;
  let budget = record.budget?.bytes ?? 0;
  const charge = (source: MupdfObject): void => {
    const raw = source.readRawStream();
    try {
      budget += raw.getLength();
    } finally {
      raw.destroy();
    }
    if (budget > MAX_EDIT_SCAN_BYTES) failEditText("too-large", "edit-text: copying this placement would exceed the scan ceiling");
  };
  if (cow.inheritedResources && !at(pageObject, "Resources")) {
    const inherited = attempt(() => pageObject.getInheritable("Resources") as MupdfObject, null);
    if (!inherited) failReason("invalid", "edit-text: the inherited resources are no longer reachable");
    const copy = pdf.newDictionary() as MupdfObject;
    // A direct sub-dictionary (/XObject, /Font …) is copied too: put as-is it would be the
    // parent's own dictionary, so a later write here would land in the inherited
    // resources, outside this operation's undo (review U4).
    inherited.forEach((value: MupdfObject, key: string | number) => {
      if (value.isDictionary() && !value.isIndirect()) {
        const inner = pdf.newDictionary() as MupdfObject;
        value.forEach((item: MupdfObject, name: string | number) => inner.put(name, item));
        copy.put(key, inner);
      } else {
        copy.put(key, value);
      }
    });
    pageObject.put("Resources", copy);
  }
  let owned = element;
  if (cow.sharedContents !== null) {
    charge(element);
    const before = editReadStream(element, "edit-text");
    const raw = element.readRawStream();
    try {
      const extra: Record<string, MupdfObject> = {};
      element.forEach((value: MupdfObject, key: string | number) => { if (key !== "Length") extra[String(key)] = value; });
      owned = pdf.addRawStream(raw, extra) as MupdfObject;
    } finally {
      raw.destroy();
    }
    if (!editBytesEqual(before, editReadStream(owned, "edit-text"))) failReason("invalid", "edit-text: the copied content stream differs from its source");
    if (!contents.isArray()) pageObject.put("Contents", owned);
    else if (!contents.isIndirect()) contents.put(elementIndex, owned);
    else {
      // U5 review E-1: an indirect /Contents array can be shared by several pages, so
      // this page gets its own direct copy before the owned stream goes in.
      const copy = pdf.newArray() as MupdfObject;
      for (let index = 0; index < contents.length; index++) copy.push(contents.get(index) as MupdfObject);
      copy.put(elementIndex, owned);
      pageObject.put("Contents", copy);
    }
  }
  const form = cow.form;
  if (!form) return owned;
  const xobjects = at(at(pageObject, "Resources"), "XObject");
  const original = xobjects ? at(xobjects, form.name) : null;
  if (!xobjects || !original || attempt(() => original.asIndirect(), -1) !== form.reference) {
    failReason("invalid", "edit-text: the Form XObject is no longer where it was");
  }
  charge(original);
  const copy = pdf.newGraftMap().graftObject(original) as MupdfObject;
  if (!editBytesEqual(editReadStream(original, "edit-text"), editReadStream(copy, "edit-text"))) {
    failReason("invalid", "edit-text: the copied Form differs from its source");
  }
  let name = "";
  for (let n = 1; ; n++) {
    name = `${form.name}c${n}`;
    if (!at(xobjects, name)) break;
  }
  xobjects.put(name, copy);
  const bytes = editReadStream(owned, "edit-text");
  owned.writeStream(editSplice(bytes, [{ start: form.operandStart, end: form.operandEnd, replacement: new TextEncoder().encode(`/${name}`) }]));
  return copy;
}

/**
 * Apply replacements to one stream, highest offset first so earlier spans stay
 * valid. Every span is checked against the stream before anything is written.
 */
export function editSplice(bytes: Uint8Array, edits: { start: number; end: number; replacement: Uint8Array }[]): Uint8Array {
  const ordered = [...edits].sort((a, b) => b.start - a.start);
  let result = bytes;
  let previousStart = bytes.length;
  for (const edit of ordered) {
    if (edit.start < 0 || edit.end > previousStart || edit.end < edit.start) {
      failReason("invalid", "edit-text: the recorded spans no longer fit the stream");
    }
    previousStart = edit.start;
    const out = new Uint8Array(result.length - (edit.end - edit.start) + edit.replacement.length);
    out.set(result.subarray(0, edit.start), 0);
    out.set(edit.replacement, edit.start);
    out.set(result.subarray(edit.end), edit.start + edit.replacement.length);
    result = out;
  }
  return result;
}

/**
 * The width gate's measurement, in its own structured-text walk because the font
 * handles are only valid while that object is alive. Both strings are measured
 * with the matched run's own font and size (`advanceGlyph` is a fraction of an em
 * in this build, measured against the stext character quads).
 */
export function editMeasureRun(page: MupdfPage, record: EditTargetRecord, next: string, runs?: EditRun[], ratio = 1): { oldWidth: number; newWidth: number; missing: string[] } | null {
  const text = record.text ?? "";
  const measure = (walk: EditRun[]) => {
    const match = editMatchRuns(walk, text, record.font?.name ?? null, record.cow !== undefined, record.kerned === true, record.origin ?? null, record.depth);
    if (!match) return null;
    const old = editAdvance(match.chars, text);
    // A loaded base-14 WinAnsi font reaches `—`/`€` by glyph name only, so its
    // Unicode lookup misses them; the same standard font by name measures them.
    const name = record.font?.name ?? "";
    const fallback = record.winAnsiEncoding && record.font?.embedded === false && BASE14_FONTS.includes(name) && base14Font
      ? base14Font(name) : null;
    const fresh = editAdvance(match.chars, next, fallback);
    if (!old || !fresh) return null;
    const scale = record.horizontalScale ?? 1;
    const spacing = record.characterSpacing ?? 0;
    const words = record.wordSpacing ?? 0;
    const full = (width: number, value: string, adjustment = 0): number => (
      width - adjustment * (match.chars[0]?.size ?? 0) / 1000
        + spacing * [...value].length + words * [...value].filter(char => char === " ").length
    ) * scale;
    return {
      oldWidth: full(old.width, text, record.tjAdjustment),
      // §9.6: glyph advances scale with the size; Tc/Tw do not.
      newWidth: full(fresh.width * ratio, next),
      missing: fresh.missing,
    };
  };
  if (runs) return measure(runs);
  const structured = page.toStructuredText("preserve-spans");
  try {
    return measure(editRunsOf(structured));
  } catch {
    return null;
  } finally {
    structured.destroy();
  }
}

export function editHash(text: string): string {
  let hash = 0x811c9dc5;
  for (let index = 0; index < text.length; index++) {
    hash ^= text.charCodeAt(index) & 0xff;
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(16).padStart(8, "0");
}

export function editTargetId(page: number, path: string, opStart: number, payload: string): string {
  return `p${page}.${path}.${opStart}.${editHash(payload)}`;
}

/** A font-file extension, which the structured-text font name may drop (`.ttf`). */
export const EDIT_FONT_EXTENSION = /\.(ttf|otf|ttc|pfb|pfa)$/i;

export function editSameFont(left: string, right: string): boolean {
  // The structured-text name of a variable font is truncated at `[` (the axis
  // suffix), so the family name is the part before it.
  const normalise = (name: string): string => name.replace(EDIT_SUBSET_PREFIX, "").split("[")[0]
    .replace(EDIT_FONT_EXTENSION, "").toLowerCase();
  return normalise(left) === normalise(right);
}

export function editBumpReference(map: Map<number, number>, object: MupdfObject): void {
  const number = attempt(() => (object.isIndirect() ? object.asIndirect() : -1), -1);
  if (number > 0) map.set(number, (map.get(number) ?? 0) + 1);
}

/** Per-page structured text, built once per scan; association only, never a font fact. */
export function editStructuredRuns(page: MupdfPage): EditRun[] {
  const structured = page.toStructuredText("preserve-spans");
  try {
    return editRunsOf(structured);
  } catch {
    // A page whose structured text cannot be walked has no association; that is
    // reported as ambiguity for the affected targets, not as a whole-page failure.
    return [];
  } finally {
    structured.destroy();
  }
}

/** The live structured-text object a width measurement walks. */
export type EditStructuredText = ReturnType<MupdfPage["toStructuredText"]>;

/**
 * The walk itself, on a live structured-text object: the per-character font
 * handles are only valid while it is alive, which is why the width gate measures
 * inside its own walk rather than from a cached run.
 */
export function editRunsOf(structured: EditStructuredText): EditRun[] {
  const runs: EditRun[] = [];
  let current: EditRun | null = null;
  try {
    structured.walk({
      beginLine() {
        current = { text: "", fontName: null, quads: [], chars: [] };
      },
      onChar(char: string, origin: number[], font: EditFontHandle | null, size: number, quad: ArrayLike<number>) {
        if (!current) return;
        current.text += char;
        const handle = font && typeof font.getName === "function" ? font : null;
        const getName = handle?.getName?.bind(handle) ?? null;
        if (current.fontName === null && getName) current.fontName = attempt(getName, null);
        current.quads.push(Array.from(quad ?? [], Number));
        current.chars.push({ ch: char, font: handle, size: Number.isFinite(size) ? size : 0, quad: Array.from(quad ?? [], Number), origin: Array.from(origin ?? [], Number) });
      },
      endLine() {
        if (current && current.text) runs.push(current);
        current = null;
      },
    });
  } catch {
    return [];
  }
  return runs;
}

export function editUnionQuads(quads: number[][]): number[] | null {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const quad of quads) {
    for (let index = 0; index + 1 < quad.length; index += 2) {
      const x = quad[index];
      const y = quad[index + 1];
      if (!Number.isFinite(x) || !Number.isFinite(y)) continue;
      minX = Math.min(minX, x);
      maxX = Math.max(maxX, x);
      minY = Math.min(minY, y);
      maxY = Math.max(maxY, y);
    }
  }
  if (!Number.isFinite(minX) || !Number.isFinite(minY)) return null;
  return [minX, minY, maxX, maxY];
}

/**
 * The one occurrence of `text` in the page's structured text, or null for zero
 * or more than one. Bounds come from the matched characters' own quads (never a
 * guessed rectangle) and the characters carry the live font handles the width
 * gate needs.
 */
/**
 * The one run on the page that shows `text` in `baseFont`. With `anyOfEqual`
 * (U1b: the same Form drawn more than once) identical runs may repeat: any one
 * measures the same advance, but no single placement can claim the bounds.
 */
export function editMatchRuns(runs: EditRun[], text: string, baseFont: string | null, anyOfEqual: boolean, kerned: boolean, origin: number[] | null, depth: number): { bounds: number[] | null; chars: EditRunChar[] } | null {
  const matches: { chars: EditRunChar[]; fontName: string | null }[] = [];
  const limit = anyOfEqual ? MAX_EDIT_EQUAL_MATCHES + 1 : 2;
  // S1/P6 U8: the one occurrence starting at the run's own origin is the run, whether the same
  // text sits inside a longer line or repeats as another whole line elsewhere on the page. Only
  // direct page content (depth 0) gets that: an operator inside a Form (depth > 0) may be painted
  // at several places, so there a whole-line duplicate keeps the P3 rule (U1b: repeated Forms
  // stay bounds-less). Two runs at one origin (overprint, near.length > 1) stay ambiguous.
  const near: EditRunChar[][] = [];
  let wholeLines = 0;
  let nearWhole = false;
  const add = (chars: EditRunChar[], fontName: string | null, line: string): void => {
    const whole = line.trim() === text.trim();
    if (whole) wholeLines++;
    const at = chars[0]?.origin;
    const tolerance = Math.max(1, (chars[0]?.size ?? 0) / 2);
    if (origin && at && at.length >= 2 && Math.hypot(at[0] - origin[0], at[1] - origin[1]) <= tolerance) {
      near.push(chars);
      nearWhole = whole;
    }
    if (matches.length < limit) matches.push({ chars, fontName });
  };
  let overflow = false;
  for (const run of runs) {
    // One quad per character only holds while the run's text and quads line up;
    // a surrogate pair would shift every later index, so that run is not used.
    if (run.quads.length !== run.text.length) continue;
    if (baseFont && run.fontName && !editSameFont(run.fontName, baseFont)) continue;
    let from = 0;
    for (;;) {
      const found = run.text.indexOf(text, from);
      if (found >= 0 && run.chars.length >= found + text.length) {
        add(run.chars.slice(found, found + text.length), run.fontName, run.text);
        from = found + 1;
        if (matches.length >= limit && !origin) break;
        continue;
      }
      if (!kerned || matches.length > 0) break;
      const chars: EditRunChar[] = [];
      let cursor = 0;
      for (const character of text) {
        if (character !== " ") while (run.text[cursor] === " ") cursor++;
        if (run.text[cursor] !== character || !run.chars[cursor]) { chars.length = 0; break; }
        chars.push(run.chars[cursor++]);
      }
      if (chars.length === text.length) add(chars, run.fontName, run.text);
      break;
    }
    if (matches.length >= limit) {
      overflow = true;
      if (!origin) return null;
    }
  }
  if (near.length === 1 && (depth === 0 || wholeLines === (nearWhole ? 1 : 0))) return { bounds: editUnionQuads(near[0].map(char => char.quad)), chars: near[0] };
  if (overflow || matches.length === 0) return null;
  if (matches.length === 1) return { bounds: editUnionQuads(matches[0].chars.map(char => char.quad)), chars: matches[0].chars };
  if (!anyOfEqual) return null;
  const first = matches[0];
  const size = first.chars[0]?.size;
  if (!matches.every(match => match.fontName === first.fontName && match.chars[0]?.size === size)) return null;
  return { bounds: null, chars: first.chars };
}

/**
 * The measured advance of `text` in the run's own font, using `size` for every
 * character (the matched run's own sizes are used for the old string). Null when
 * this build cannot measure, which the caller refuses as `unmeasurable-width`.
 * `advanceGlyph` returns a fraction of an em here (measured against the stext
 * character quads), so the point width is advance x size.
 */
export function editAdvance(chars: EditRunChar[], text: string, fallback: EditFontHandle | null = null): { width: number; missing: string[] } | null {
  const font = chars[0]?.font;
  if (!font || typeof font.encodeCharacter !== "function" || typeof font.advanceGlyph !== "function") return null;
  const missing: string[] = [];
  let width = 0;
  for (const char of text) {
    let source = font;
    let gid = attempt(() => font.encodeCharacter!(char), 0);
    if (!gid && fallback?.encodeCharacter && fallback.advanceGlyph) {
      source = fallback;
      gid = attempt(() => fallback.encodeCharacter!(char), 0);
    }
    if (!gid) {
      missing.push(char);
      continue;
    }
    const advance = attempt(() => source.advanceGlyph!(gid, 0), NaN);
    if (!Number.isFinite(advance)) return null;
    width += advance * (chars[0].size || 0);
  }
  return { width, missing };
}

/** A document known to be a PDF; the scanner and the tag rewrite both need it. */
export type EditPdf = NonNullable<ReturnType<MupdfDocument["asPDF"]>>;

/**
 * The StructElem a page MCID maps to, via `/StructParents` and the ParentTree
 * (`/Nums`, then `/Kids` ranges). This is the struct-tree half of the tagged
 * `/ActualText` path; the inline `BDC` dictionary is the other place one can live.
 */
export function editStructElement(pdf: EditPdf, pageObject: MupdfObject, mcid: number): MupdfObject | null {
  const parents = at(pageObject, "StructParents");
  if (!parents || !parents.isNumber()) return null;
  const key = Math.trunc(parents.asNumber());
  const tree = at(at(pdf.getTrailer() as MupdfObject, "Root"), "StructTreeRoot");
  const nums = at(at(tree, "ParentTree"), "Nums");
  if (!nums || !nums.isArray()) return null;
  for (let index = 0; index + 1 < nums.length; index += 2) {
    const numKey = nums.get(index) as MupdfObject;
    if (!numKey.isNumber() || Math.trunc(numKey.asNumber()) !== key) continue;
    return editParentTreeNode(nums.get(index + 1) as MupdfObject, mcid, 0);
  }
  return null;
}

export function editParentTreeNode(node: MupdfObject | null, mcid: number, depth: number): MupdfObject | null {
  if (!node || depth > 8) return null;
  if (node.isArray()) return mcid < node.length ? (node.get(mcid) as MupdfObject) : null;
  if (node.isDictionary()) {
    const limits = at(node, "Limits");
    if (limits && limits.isArray() && limits.length >= 2) {
      const low = limits.get(0) as MupdfObject;
      const high = limits.get(1) as MupdfObject;
      if (low.isNumber() && high.isNumber() && (mcid < low.asNumber() || mcid > high.asNumber())) return null;
    }
    return editParentTreeNode(at(node, "Kids"), mcid, depth + 1);
  }
  return null;
}

/**
 * U3: read one auxiliary stream (a /ToUnicode CMap) inside the scan budget. A
 * stream that cannot be read, or that would push the scan over its ceiling, is
 * not an error: it just leaves the font unproven (`unsupported-encoding`).
 */
export function editReadBounded(stream: MupdfObject, state: EditScanState | null): Uint8Array | null {
  try {
    const buffer = stream.readStream();
    try {
      const view = buffer.asUint8Array();
      if (state) {
        if (state.budget.bytes + view.length > MAX_EDIT_SCAN_BYTES) return null;
        state.budget.bytes += view.length;
      }
      return view.slice();
    } finally {
      buffer.destroy();
    }
  } catch {
    return null;
  }
}

export const EDIT_HEX4 = /^[0-9A-Fa-f]{4}$/;
/** Every code this CMap maps; null when this build cannot prove it parsed the map. */
export const EDIT_MAX_CMAP_CODES = 1 << 20;

/**
 * Parse a /ToUnicode CMap (contract §2.1): 2-byte codes only, a destination of
 * exactly four hex digits is one BMP code point, any other destination length
 * (surrogate pairs, ligature sequences) leaves that code unmappable. Anything
 * this parser cannot follow returns null rather than a partial map.
 */
export function editParseToUnicode(text: string, bytesPerCode: 1 | 2 = 2): Map<number, string> | null {
  if (/\busecmap\b/.test(text)) return null;
  // Coded: every source is one byte (2 hex digits); destinations stay 4-hex BMP.
  const source = bytesPerCode === 1 ? /^[0-9A-Fa-f]{2}$/ : EDIT_HEX4;
  // Every codespace range must be bytesPerCode wide, or this build cannot split the payload.
  let block: RegExpExecArray | null;
  const ranges = /begincodespacerange([\s\S]*?)endcodespacerange/g;
  let seenSpace = false;
  while ((block = ranges.exec(text))) {
    for (const pair of block[1].matchAll(/<([0-9A-Fa-f]+)>\s*<([0-9A-Fa-f]+)>/g)) {
      seenSpace = true;
      if (!source.test(pair[1]) || !source.test(pair[2])) return null;
    }
  }
  if (!seenSpace) return null;
  const map = new Map<number, string>();
  let generated = 0;
  const add = (code: number, destination: string): void => {
    if (destination.length !== 4 || !/^[0-9A-Fa-f]{4}$/.test(destination)) return;
    map.set(code, String.fromCharCode(parseInt(destination, 16)));
  };
  const chars = /beginbfchar([\s\S]*?)endbfchar/g;
  while ((block = chars.exec(text))) {
    for (const pair of block[1].matchAll(/<([0-9A-Fa-f]+)>\s*<([0-9A-Fa-f]+)>/g)) {
      if (!source.test(pair[1])) return null;
      add(parseInt(pair[1], 16), pair[2]);
    }
  }
  const bfranges = /beginbfrange([\s\S]*?)endbfrange/g;
  while ((block = bfranges.exec(text))) {
    const body = block[1];
    const entries = /<([0-9A-Fa-f]+)>\s*<([0-9A-Fa-f]+)>\s*(<([0-9A-Fa-f]+)>|\[([\s\S]*?)\])/g;
    let entry: RegExpExecArray | null;
    let consumed = 0;
    while ((entry = entries.exec(body))) {
      consumed = entry.index + entry[0].length;
      const low = parseInt(entry[1], 16);
      const high = parseInt(entry[2], 16);
      if (!source.test(entry[1]) || !source.test(entry[2]) || high < low) return null;
      if (generated + (high - low + 1) > EDIT_MAX_CMAP_CODES) return null;
      generated += high - low + 1;
      if (entry[4] !== undefined) {
        if (!EDIT_HEX4.test(entry[4])) continue;
        const base = parseInt(entry[4], 16);
        if (base + (high - low) > 0xffff) return null;
        for (let code = low; code <= high; code++) map.set(code, String.fromCharCode(base + code - low));
      } else {
        const destinations = [...(entry[5] ?? "").matchAll(/<([0-9A-Fa-f]+)>/g)].map(match => match[1]);
        for (let index = 0; index < high - low + 1; index++) {
          if (index >= destinations.length) return null;
          if (EDIT_HEX4.test(destinations[index])) add(low + index, destinations[index]);
        }
      }
    }
    // A trailing fragment inside the block means this parser did not follow it.
    if (body.slice(consumed).match(/<[0-9A-Fa-f]+>/)) return null;
  }
  return map;
}

/**
 * Parse `/W` and `/DW` from a descendant font (contract §2.1). Both the
 * `c [w1 w2 …]` and the `cfirst clast w` forms are accepted; anything this
 * parser cannot follow returns null, and the font stays unproven.
 */
export function editParseWidths(descendant: MupdfObject): { widths: Map<number, number>; dw: number } | null {
  const list = at(descendant, "W");
  if (!list || !list.isArray()) return null;
  const widths = new Map<number, number>();
  let generated = 0;
  let index = 0;
  while (index < list.length) {
    const first = list.get(index) as MupdfObject;
    if (!first.isNumber()) return null;
    index += 1;
    if (index >= list.length) return null;
    const next = list.get(index) as MupdfObject;
    if (next.isArray()) {
      for (let item = 0; item < next.length; item++) {
        const width = next.get(item) as MupdfObject;
        if (!width.isNumber()) return null;
        const code = Math.trunc(first.asNumber()) + item;
        if (code >= 0 && code <= 0xffff) widths.set(code, width.asNumber());
      }
      index += 1;
      continue;
    }
    if (!next.isNumber() || index + 1 >= list.length) return null;
    const width = list.get(index + 1) as MupdfObject;
    if (!width.isNumber()) return null;
    const low = Math.trunc(first.asNumber());
    const high = Math.trunc(next.asNumber());
    if (high < low || high - low > 0xffff) return null;
    // Only 2-byte codes exist, and the expansion is charged like a CMap's, so a
    // small /W cannot make the walk allocate or loop without bound.
    generated += Math.max(0, Math.min(high, 0xffff) - Math.max(low, 0) + 1);
    if (generated > EDIT_MAX_CMAP_CODES) return null;
    for (let code = Math.max(low, 0); code <= Math.min(high, 0xffff); code++) widths.set(code, width.asNumber());
    index += 2;
  }
  const dwObject = at(descendant, "DW");
  if (!dwObject) return { widths, dw: 1000 };
  if (!dwObject.isNumber()) return null;
  return { widths, dw: dwObject.asNumber() };
}

/**
 * The `/cid` half of `editResolveFont`: only a fully proven Identity-H CID font
 * gets a map. Every other shape (Identity-V, a predefined or embedded CMap, a
 * non-Identity /CIDToGIDMap, no font program) stays `cid: null`.
 */
export function editResolveCid(font: MupdfObject, state: EditScanState | null): EditCid | null {
  const encoding = at(font, "Encoding");
  if (!encoding || !encoding.isName() || encoding.asName() !== "Identity-H") return null;
  const descendants = at(font, "DescendantFonts");
  if (!descendants || !descendants.isArray() || descendants.length !== 1) return null;
  const descendant = descendants.get(0) as MupdfObject;
  const subtype = at(descendant, "Subtype");
  if (!subtype || !subtype.isName() || (subtype.asName() !== "CIDFontType2" && subtype.asName() !== "CIDFontType0")) return null;
  const cidToGid = at(descendant, "CIDToGIDMap");
  if (cidToGid && !(cidToGid.isName() && cidToGid.asName() === "Identity")) return null;
  const descriptor = at(descendant, "FontDescriptor") ?? at(font, "FontDescriptor");
  if (!descriptor || (!at(descriptor, "FontFile2") && !at(descriptor, "FontFile3"))) return null;
  const cmapObject = at(font, "ToUnicode");
  if (!cmapObject || !cmapObject.isStream()) return null;
  const bytes = editReadBounded(cmapObject, state);
  if (!bytes) return null;
  const toUnicode = editParseToUnicode(editBytesToLatin1(bytes));
  if (!toUnicode) return null;
  const parsed = editParseWidths(descendant);
  if (!parsed) return null;
  return { toUnicode, widths: parsed.widths, dw: parsed.dw, bytesPerCode: 2 };
}

/**
 * Coded (contract-coded.md D1): a symbolic simple font with no /Encoding (LibreOffice
 * and Word exports) whose 1-byte /ToUnicode, /FirstChar + /Widths and embedded program
 * this build can prove. Any other shape stays `cid: null` and keeps the WinAnsi path.
 */
export function editResolveCoded(font: MupdfObject, descriptor: MupdfObject | null, state: EditScanState | null): EditCid | null {
  if (at(font, "Encoding") || !descriptor) return null;
  const flagsObject = at(descriptor, "Flags");
  if (!flagsObject || !flagsObject.isNumber()) return null;
  const flags = Math.trunc(flagsObject.asNumber());
  if ((flags & 4) === 0 || (flags & 32) !== 0) return null;
  if (!["FontFile", "FontFile2", "FontFile3"].some(key => !!at(descriptor, key))) return null;
  const cmapObject = at(font, "ToUnicode");
  if (!cmapObject || !cmapObject.isStream()) return null;
  const first = at(font, "FirstChar");
  const list = at(font, "Widths");
  if (!first || !first.isNumber() || !Number.isInteger(first.asNumber()) || !list || !list.isArray()) return null;
  const firstChar = first.asNumber();
  const widths = new Map<number, number>();
  // Only 1-byte codes exist, so at most 256 entries are ever read.
  for (let index = 0; index < list.length && firstChar + index <= 0xff; index++) {
    const width = list.get(index) as MupdfObject;
    if (!width.isNumber()) return null;
    if (firstChar + index >= 0) widths.set(firstChar + index, width.asNumber());
  }
  const bytes = editReadBounded(cmapObject, state);
  if (!bytes) return null;
  const toUnicode = editParseToUnicode(editBytesToLatin1(bytes), 1);
  if (!toUnicode) return null;
  const missing = at(descriptor, "MissingWidth");
  return { toUnicode, widths, dw: missing && missing.isNumber() ? missing.asNumber() : 0, bytesPerCode: 1 };
}

export function editResolveFont(resources: MupdfObject | null, name: string | null, state: EditScanState | null = null): EditFont | null {
  if (!resources || !name) return null;
  const fonts = at(resources, "Font");
  const font = fonts ? at(fonts, name) : null;
  if (!font) return null;
  const objectNumber = attempt(() => (font.isIndirect() ? font.asIndirect() : -1), -1);
  if (state && objectNumber > 0) {
    const cached = state.fontCache.get(objectNumber);
    if (cached !== undefined) return cached;
  }
  const subtypeObject = at(font, "Subtype");
  const subtype = subtypeObject && subtypeObject.isName() ? subtypeObject.asName() : null;
  const baseObject = at(font, "BaseFont");
  const baseFont = baseObject && baseObject.isName() ? baseObject.asName() : null;
  const encoding = at(font, "Encoding");
  const encodingUnknown = !!encoding && encoding.isDictionary();
  const winAnsiEncoding = !!encoding && encoding.isName() && encoding.asName() === "WinAnsiEncoding";
  const simple = subtype === "Type1" || subtype === "TrueType" || subtype === "MMType1";
  let writingMode: number | null = null;
  let descriptor = at(font, "FontDescriptor");
  if (subtype === "Type0") {
    const descendants = at(font, "DescendantFonts");
    const first = descendants && descendants.length > 0 ? (descendants.get(0) as MupdfObject) : null;
    if (first) {
      const mode = at(first, "WMode");
      writingMode = mode && mode.isNumber() ? mode.asNumber() : 0;
      descriptor = at(first, "FontDescriptor") ?? descriptor;
    }
  }
  const embedded = descriptor
    ? ["FontFile", "FontFile2", "FontFile3"].some(key => !!at(descriptor, key))
    : simple ? false : null;
  const cid = subtype === "Type0" ? editResolveCid(font, state) : simple ? editResolveCoded(font, descriptor, state) : null;
  const resolved: EditFont = {
    summary: {
      name: baseFont ? baseFont.replace(EDIT_SUBSET_PREFIX, "") : null,
      subtype,
      embedded,
      subset: baseFont ? EDIT_SUBSET_PREFIX.test(baseFont) : null,
      writingMode,
    },
    name: baseFont ? baseFont.replace(EDIT_SUBSET_PREFIX, "") : null,
    subtype,
    subset: baseFont ? EDIT_SUBSET_PREFIX.test(baseFont) : null,
    simple,
    encodingUnknown,
    winAnsiEncoding,
    writingMode,
    cid,
    objectNumber,
  };
  if (state && objectNumber > 0) state.fontCache.set(objectNumber, resolved);
  return resolved;
}

/**
 * Discover the edit targets of one page. Deterministic content order, one item
 * per text-show or Image/Form invocation occurrence; throws a reason-tagged
 * error (`too-large`, `invalid`) instead of ever returning a partial list.
 */
/** P6 U2 §2.4/§9.2–3: the run's page-pt font size and whether §2.2 can move it. */
export function editTextScale(record: EditTargetRecord): number | null {
  const linear = record.textLinear;
  if (!linear || !editInvert(linear) || !record.tfName || !record.tfSize || !Number.isFinite(record.tfSize)) return null;
  // The text-space unit up (0, 1) maps to (c, d): its length is the page-pt height of 1 Tf unit.
  return Math.hypot(linear[2], linear[3]);
}

/** Why §2.2 cannot move this run, or null when it can. */
export function editMoveRefusal(record: EditTargetRecord): { detail: "not-candidate" | "tail-would-move"; reason: string } | null {
  if (record.support !== "candidate") return { detail: "not-candidate", reason: record.reason ?? "not-candidate" };
  if (editTextScale(record) === null) return { detail: "not-candidate", reason: "matrix" };
  if (!record.firstOnLine) return { detail: "not-candidate", reason: "tail" };
  if (record.tailFollows) return { detail: "tail-would-move", reason: "tail-would-move" };
  return null;
}

export function editAttachGeometry(state: EditScanState): void {
  for (const item of state.items) {
    if (item.kind !== "text") continue;
    const record = state.records.get(item.targetId);
    if (!record) continue;
    const scale = editTextScale(record);
    if (scale !== null) item.fontSize = Math.round(Math.abs(record.tfSize!) * scale * 100) / 100;
    const refusal = editMoveRefusal(record);
    item.moveable = refusal === null;
    if (refusal && record.support === "candidate") item.moveReason = refusal.reason;
  }
}

export function editAttachGrowthLimits(state: EditScanState): void {
  const records = [...state.records.values()];
  const text = records.filter(record => record.kind === "text" && record.bounds?.length === 4);
  const pageRight = Math.max(...text.map(record => record.bounds![2]), 0);
  // Bugfix 1 D2: a run this app wrote (text-add's /PDF4AllF<n>) grows to the page box's right edge.
  // Not on a rotated or scaled page: there the run's advance is not the view's x axis (review fix).
  const pageBoxRight = state.geometryBlocked ? pageRight : attempt(() => state.page.getBounds()[2], pageRight);
  const overlaps = (left: number[], right: number[]): boolean => left[1] < right[3] && right[1] < left[3];
  for (const record of text) {
    const box = record.bounds!;
    const tailIds = new Set<string>();
    let tailRight = box[2];
    let leftToRight = true;
    let cursor = record;
    while (cursor.tailTargetId) {
      const next = state.records.get(cursor.tailTargetId);
      if (!next?.bounds || tailIds.has(next.targetId)) break;
      tailIds.add(next.targetId);
      if (next.bounds[0] + 0.5 < box[2]) leftToRight = false;
      tailRight = Math.max(tailRight, next.bounds[2]);
      cursor = next;
    }
    const obstacleLefts: number[] = [];
    for (const other of records) {
      if (other === record || tailIds.has(other.targetId)) continue;
      const otherBox = other.kind === "text" ? other.bounds : other.placement?.view;
      if (!otherBox || !overlaps(box, otherBox) || otherBox[0] < tailRight - 0.5) continue;
      obstacleLefts.push(otherBox[0]);
    }
    record.tailRight = tailRight;
    record.leftToRightTail = leftToRight;
    record.growthLimit = obstacleLefts.length ? Math.min(...obstacleLefts) - 1
      : /^PDF4AllF\d*$/.test(record.tfName ?? "") ? pageBoxRight : pageRight;
    record.obstacleLimit = obstacleLefts.length ? Math.min(...obstacleLefts) - 1 : null;
  }
}

/** print-edit D1: a Quartz per-glyph block, by the string operand's start, in one depth-0 stream. */
type EditGlyphBlock = { start: number; end: number; colour: string; scale: number; d: number; x: number; y: number; font: string };
const GLYPH_WS = String.raw`[\0\t\n\f\r ]`;
// Review fix 1: one way to split a digit run (`\d+\.?\d*` backtracked quadratically on long runs).
const GLYPH_NUM = String.raw`[+-]?(?:\d+(?:\.\d*)?|\.\d+)`;
const GLYPH_NAME = String.raw`/[^\0\t\n\f\r ()<>\[\]{}/%]+`;
const GLYPH_BLOCK = (() => {
  const w = `${GLYPH_WS}+`, n = GLYPH_NUM;
  const nums = (names: string[]) => names.map(name => `(?<${name}>${n})`).join(w);
  const head = `(?<=^|[\\0\\t\\n\\f\\r ()<>\\[\\]{}])q(?:${w}${n}${w}${n}${w}${n}${w}${n}${w}re${w}W${w}n)?`
    + `(?:${w}(?<colour>(?:${GLYPH_NAME}${w}cs${w})?(?:${n}${w}){1,4}(?:scn|sc|g|rg|k)))?`
    + `${w}q${w}${nums(["a", "b", "c", "d", "e", "f"])}${w}cm${w}BT${w}${nums(["t0", "t1", "t2", "t3", "t4", "t5"])}${w}Tm`
    + `${w}(?<font>${GLYPH_NAME})${w}(?<size>${n})${w}Tf${w}`;
  const operand = String.raw`\((?:\\[\s\S]|[^\\()])*\)|<[0-9a-fA-F\0\t\n\f\r ]*>`;
  return new RegExp(`(?<head>${head})(?<str>${operand})${GLYPH_WS}*Tj${w}ET${w}Q${w}Q(?![^\\0\\t\\n\\f\\r ()<>\\[\\]{}/%])`, "g");
})();

function editGlyphBlocksOf(state: EditScanState, path: string): Map<number, EditGlyphBlock> {
  let blocks = state.glyphBlocks.get(path);
  if (!blocks) {
    const bytes = state.streams.get(path);
    blocks = bytes ? editGlyphBlocks(bytes) : new Map();
    state.glyphBlocks.set(path, blocks);
  }
  return blocks;
}

function editGlyphBlocks(bytes: Uint8Array): Map<number, EditGlyphBlock> {
  const blocks = new Map<number, EditGlyphBlock>();
  for (const match of editBytesToLatin1(bytes).matchAll(GLYPH_BLOCK)) {
    const g = match.groups!;
    const v = (name: string) => Number(g[name]);
    // D12: a and d count as equal within 1e-3·a (Quartz writes 11.9625 / 11.96249); the scale is a.
    const scale = v("a");
    if (!(scale > 0) || Math.abs(v("d") - scale) > 1e-3 * scale || v("b") !== 0 || v("c") !== 0 || v("size") !== 1) continue;
    if (v("t0") !== 1 || v("t1") !== 0 || v("t2") !== 0 || v("t3") !== 1 || v("t4") !== 0 || v("t5") !== 0) continue;
    blocks.set(match.index! + g.head.length, {
      start: match.index!, end: match.index! + match[0].length, colour: g.colour ?? "",
      scale, d: v("d"), x: v("e"), y: v("f"), font: g.font.slice(1),
    });
  }
  return blocks;
}

/**
 * print-edit D2: chain consecutive Quartz glyph blocks (D1) into one ordinary text record per
 * line. Each line replaces its members in `state.items`/`state.records`; the whole chain is
 * the operand span, so D4 rewrites it in one splice.
 */
export function editCollapseGlyphLines(state: EditScanState, resources: MupdfObject | null): void {
  if (state.geometryBlocked) return;
  const near = (left: number, right: number) => Math.abs(left - right) <= 0.01;
  type Member = {
    item: EditTarget; record: EditTargetRecord; block: EditGlyphBlock; code: number; adv: number; colour: string;
    simple?: { objectNumber: number; widths: Map<number, number> };
    /** D12: an explicit 0x20 block of a D11 font: one word gap, not a glyph. */
    space?: boolean;
  };
  const glyphOf = (item: EditTarget): Member | null => {
    const record = state.records.get(item.targetId);
    if (!record || record.kind !== "text" || record.depth !== 0 || !/^c\d+$/.test(record.path)) return null;
    const ambiguous = record.support === "unsupported" && record.reason === "ambiguous-text-association";
    if (record.support !== "candidate" && !ambiguous) return null;
    if (record.payloadBytes?.length !== 1 || typeof record.text !== "string") return null;
    const block = editGlyphBlocksOf(state, record.path).get(record.operandStart);
    if (!block || block.font !== record.tfName) return null;
    const chars = [...record.text];
    if (chars.length !== 1 || (/\s/.test(chars[0]) && chars[0] !== " ")) return null;
    if (record.renderMode !== 0 || (record.ocDepth ?? 0) !== 0 || (record.mcid !== null && record.mcid !== undefined)) return null;
    if (record.actualText || record.structActualText !== undefined) return null;
    if ((record.characterSpacing ?? 0) !== 0 || (record.wordSpacing ?? 0) !== 0 || (record.horizontalScale ?? 1) !== 1) return null;
    const linear = record.textLinear, origin = record.origin, m = state.pageTransform;
    if (!linear || !origin) return null;
    if (!near(linear[0], block.scale) || !near(linear[1], 0) || !near(linear[2], 0) || !near(linear[3], -block.d)) return null;
    if (!near(origin[0], m[0] * block.x + m[2] * block.y + m[4]) || !near(origin[1], m[1] * block.x + m[3] * block.y + m[5])) return null;
    const font = editResolveFont(resources, record.tfName ?? null, state);
    if (!font || font.subtype === "Type0") return null;
    const code = record.payloadBytes[0];
    let adv: number;
    let simple: Member["simple"];
    if (font.cid) {
      if (font.cid.bytesPerCode !== 1 || chars[0] === " ") return null;
      adv = (font.cid.widths.get(code) ?? font.cid.dw) / 1000 * block.scale;
    } else {
      // D11: a simple WinAnsi/MacRoman font drawing printable ASCII, character = code.
      const widths = simpleWidths(font, record.tfName!);
      const width = widths?.get(code);
      if (width === undefined || code < 0x20 || code > 0x7e || record.text !== String.fromCharCode(code)) return null;
      adv = width / 1000 * block.scale;
      simple = { objectNumber: font.objectNumber, widths: widths! };
    }
    return { item, record, block, code, adv, colour: block.colour.replace(new RegExp(`${GLYPH_WS}+`, "g"), " "), simple, space: code === 0x20 };
  };
  const widthCache = new Map<number, Map<number, number> | null>();
  const simpleWidths = (font: EditFont, tfName: string): Map<number, number> | null => {
    if (!font.simple || font.encodingUnknown || font.objectNumber <= 0) return null;
    const cached = widthCache.get(font.objectNumber);
    if (cached !== undefined) return cached;
    const dict = at(at(resources, "Font"), tfName);
    const encoding = at(dict, "Encoding");
    const first = at(dict, "FirstChar");
    const list = at(dict, "Widths");
    let widths: Map<number, number> | null = null;
    if (encoding?.isName() && ["WinAnsiEncoding", "MacRomanEncoding"].includes(encoding.asName())
      && first?.isNumber() && list?.isArray()) {
      widths = new Map();
      for (let index = 0; index < list.length; index++) {
        const width = list.get(index) as MupdfObject;
        if (width.isNumber()) widths.set(first.asNumber() + index, width.asNumber());
      }
    }
    widthCache.set(font.objectNumber, widths);
    return widths;
  };

  const out: EditTarget[] = [];
  let chain: Member[] = [];
  let gaps: number[] = [];
  const flush = () => {
    const members = chain, flags = gaps;
    chain = [];
    gaps = [];
    // D12: a space block never ends a line; trailing ones stay ordinary items after it.
    const trailing: Member[] = [];
    while (members.length && members[members.length - 1].space) { trailing.unshift(members.pop()!); flags.pop(); }
    collapse(members, flags);
    out.push(...trailing.map(member => member.item));
  };
  const collapse = (members: Member[], flags: number[]) => {
    if (!members.length) return;
    const lender = members.find(member => !member.space && member.record.support === "candidate");
    const bounded = members.filter(member => member.record.bounds?.length === 4);
    // D11: the writable set of a simple font is the codes its glyph blocks draw on this page.
    const drawnChars = lender?.simple ? drawn.get(lender.simple.objectNumber)! : null;
    const write: EditCidWrite | undefined = lender?.simple
      ? { chars: drawnChars!, widths: lender.simple.widths, dw: 0, bytesPerCode: 1 }
      : lender?.record.cidWrite;
    if (!lender || !write || !bounded.length) {
      out.push(...members.map(member => member.item));
      return;
    }
    const first = members[0], last = members[members.length - 1];
    const top = Math.min(...bounded.map(member => member.record.bounds![1]));
    const bottom = Math.max(...bounded.map(member => member.record.bounds![3]));
    const boxes = members.map(member => member.record.bounds ?? [member.record.origin![0], top, member.record.origin![0] + member.adv, bottom]);
    const bounds = [Math.min(...boxes.map(box => box[0])), Math.min(...boxes.map(box => box[1])), Math.max(...boxes.map(box => box[2])), Math.max(...boxes.map(box => box[3]))];
    // D12: a space block is the word gap itself, so the gap rule adds no second space beside it.
    const wordGap = (index: number) => index > 0 && flags[index] > 0.12 && !members[index].space && !members[index - 1].space;
    const words = flags.filter((_, index) => wordGap(index)).sort((a, b) => a - b);
    const spaceWidth = members.some(member => member.space) ? lender.simple?.widths.get(0x20) : undefined;
    const space = spaceWidth !== undefined ? spaceWidth / 1000
      : words.length ? (words.length % 2 ? words[(words.length - 1) / 2] : (words[words.length / 2 - 1] + words[words.length / 2]) / 2) : 0.28;
    const glyphs = members.filter(member => !member.space);
    // Review fix 2: a run of adjacent space blocks is one word gap (one space), so the text stays normalised.
    const repeat = (index: number) => members[index].space === true && index > 0 && members[index - 1].space === true;
    const text = members.map((member, index) => (repeat(index) ? "" : (wordGap(index) ? " " : "") + member.record.text)).join("");
    if (text.length > MAX_EDIT_TEXT_CHARS) failReason("too-large", `edit-targets: a text target is over the ${MAX_EDIT_TEXT_CHARS} character ceiling`);
    const key = members.map((member, index) => (repeat(index) ? "" : `${wordGap(index) ? "+" : ""}${member.code.toString(16)}`)).filter(Boolean).join(".");
    const lend = lender.record;
    const own = drawnChars ? [...drawnChars].sort((a, b) => a[1] - b[1]).map(entry => entry[0]).join("") : lend.repertoire;
    const repertoire = own?.includes(" ") ? own : ` ${own ?? ""}`;
    const record: EditTargetRecord = {
      targetId: editTargetId(state.pageIndex, first.record.path, first.block.start, `GL|${key}`),
      kind: "text", page: state.pageIndex, path: first.record.path, objectNumber: first.record.objectNumber,
      opStart: last.block.end, opEnd: last.block.end, operandStart: first.block.start, operandEnd: last.block.end,
      depth: 0, support: "candidate", text, font: lend.font, bounds,
      tailFollows: false, actualText: null, mcid: null, ocDepth: 0,
      horizontalScale: 1, characterSpacing: 0, wordSpacing: 0, renderMode: 0, tjAdjustment: 0,
      winAnsiEncoding: lend.winAnsiEncoding, textLinear: lend.textLinear, firstOnLine: true, origin: first.record.origin,
      tfName: lend.tfName, tfSize: lend.tfSize, leading: lend.leading, prevShowId: null, between: null,
      fontSize: lend.fontSize, cidKey: lender.simple?.objectNumber ?? lend.cidKey, repertoire, cidWrite: write, budget: lend.budget,
      glyphLine: { glyphs: glyphs.map(member => ({ code: member.code, x: member.block.x, adv: member.adv })), y: first.block.y, scale: first.block.scale, colour: first.block.colour.trim(), space },
    };
    // Deviation (build note): an ambiguous first member has no `cow`, so the lender's is used.
    if (lend.cow) record.cow = lend.cow;
    const item: EditTarget = { targetId: record.targetId, kind: "text", bounds, support: "candidate", tailFollows: false, text, font: lend.font, repertoire, glyphs: glyphs.length };
    if (record.cow) item.cowable = true;
    for (const member of members) state.records.delete(member.record.targetId);
    state.records.set(record.targetId, record);
    out.push(item);
  };
  const glyphs = state.items.map(item => (item.kind === "text" ? glyphOf(item) : null));
  const drawn = new Map<number, Map<string, number>>();
  for (const glyph of glyphs) {
    if (!glyph?.simple || glyph.space) continue;
    if (!drawn.has(glyph.simple.objectNumber)) drawn.set(glyph.simple.objectNumber, new Map());
    drawn.get(glyph.simple.objectNumber)!.set(String.fromCharCode(glyph.code), glyph.code);
  }
  for (const [index, item] of state.items.entries()) {
    const glyph = glyphs[index];
    if (!glyph) { flush(); out.push(item); continue; }
    const prev = chain[chain.length - 1];
    let gap = 0;
    if (prev) {
      const bytes = state.streams.get(prev.record.path)!;
      let between = prev.record.path === glyph.record.path && prev.block.end <= glyph.block.start;
      for (let index = prev.block.end; between && index < glyph.block.start; index++) between = [0, 9, 10, 12, 13, 32].includes(bytes[index]);
      gap = (glyph.block.x - (prev.block.x + prev.adv)) / glyph.block.scale;
      // Review fix 5: scale and baseline compare with the chain's first block, so they cannot drift.
      const head = chain[0];
      const joins = between && glyph.record.tfName === prev.record.tfName && Math.abs(glyph.block.scale - head.block.scale) <= 1e-3 * head.block.scale
        && glyph.colour === prev.colour && Math.abs(glyph.block.y - head.block.y) < 0.005 && gap > -0.2 && gap <= 1.0;
      if (!joins) { flush(); gap = 0; }
    }
    // D12: a space block never starts a line.
    if (glyph.space && !chain.length) { out.push(item); continue; }
    chain.push(glyph);
    gaps.push(gap);
  }
  flush();
  state.items = out;
}

export function scanEditTargets(doc: MupdfDocument, pageIndex: number): { items: EditTarget[]; records: Map<string, EditTargetRecord>; paragraphs: Map<string, EditParagraph>; balancedContent: boolean; transform: number[] } {
  const pdf = doc.asPDF();
  if (!pdf) failReason("invalid", "edit-targets: the open document is not a PDF");
  const page = doc.loadPage(pageIndex) as MupdfPage;
  try {
    const pageObject = page.getObject() as MupdfObject;
    const ownResources = at(pageObject, "Resources");
    const resources = ownResources ?? attempt(() => pageObject.getInheritable("Resources") as MupdfObject, null);
    // U4: a rotated page or a /UserUnit other than 1 shifts every placement's own
    // `cm`, which this unit does not model, so no placement is editable there.
    const pageDict = attempt(() => pdf.findPage(pageIndex) as MupdfObject, null);
    // Bugfix 1 D6: /Rotate is inheritable (engine.ts text-add reads it the same way); /UserUnit is not.
    const rotate = attempt(() => (pageDict!.getInheritable("Rotate") as MupdfObject | null) ?? null, null);
    const userUnit = at(pageDict, "UserUnit");
    const geometryBlocked = (rotate !== null && Math.trunc(rotate.asNumber()) % 360 !== 0)
      || (userUnit !== null && userUnit.asNumber() !== 1);
    const pageTransform = attempt(() => [...page.getTransform()] as number[], [1, 0, 0, -1, 0, 0]);
    const contentsRefs = new Map<number, number>(), xobjectRefs = new Map<number, number>();
    const fresh = (countOnly: boolean): EditScanState => ({
      page, pageObject, pdf, pageIndex, items: [], records: new Map(),
      budget: { bytes: 0, traversals: 0, tokens: 0, replyText: 0 },
      contentsRefs, xobjectRefs, runs: null, activeForms: new Set(),
      fontCache: new Map(), repertoire: new Map(), cidPending: [],
      geometryBlocked,
      pageLinear: pageTransform.slice(0, 4),
      pageTransform,
      streams: new Map(),
      glyphBlocks: new Map(),
      ordinaryTargets: 0,
      countOnly,
    });

    // Sharing is counted from page dictionaries only: no stream is decoded to
    // prove a /Contents element or an XObject is private, and a page whose
    // resources are inherited never claims isolation.
    const pageCount = doc.countPages();
    for (let index = 0; index < pageCount; index++) {
      const dict = attempt(() => pdf.findPage(index) as MupdfObject, null);
      if (!dict) continue;
      const contents = at(dict, "Contents");
      if (contents?.isArray()) {
        for (let element = 0; element < contents.length; element++) {
          editBumpReference(contentsRefs, contents.get(element) as MupdfObject);
        }
      } else if (contents) {
        editBumpReference(contentsRefs, contents);
      }
      const xobjects = at(at(dict, "Resources") ?? null, "XObject");
      if (xobjects && xobjects.isDictionary()) xobjects.forEach((value: MupdfObject) => editBumpReference(xobjectRefs, value));
    }

    const elements: MupdfObject[] = [];
    const contents = at(pageObject, "Contents");
    if (contents?.isArray()) {
      for (let index = 0; index < contents.length; index++) elements.push(contents.get(index) as MupdfObject);
    } else if (contents) {
      elements.push(contents);
    }

    const walkPage = (state: EditScanState): EditCarried => {
      const carried: EditCarried = { current: { fontName: null, fontSize: null, horizontalScale: 1, characterSpacing: 0, wordSpacing: 0, inText: false, renderMode: 0, cms: [], clean: true, qToken: null, qPath: null, lin: [1, 0, 0, 1], tm: [1, 0, 0, 1], leading: 0, ctm: [1, 0, 0, 1, 0, 0], tlm: [1, 0, 0, 1, 0, 0] }, saved: [], scopes: [] };
      for (let index = 0; index < elements.length; index++) {
        const stream = elements[index];
        const objectNumber = attempt(() => (stream.isIndirect() ? stream.asIndirect() : -1), -1);
        const shared = objectNumber > 0 && (contentsRefs.get(objectNumber) ?? 0) > 1;
        const cow: EditCow = { inheritedResources: !ownResources, sharedContents: shared ? index : null, form: null };
        walkEditContent(state, stream, `c${index}`, 0, resources, !ownResources || shared, cow, carried);
      }
      return carried;
    };
    // Review fix 3: count first without structured-text matching, so a page whose targets can
    // never collapse to 512 refuses in linear time (the emit check throws at the 513th).
    walkPage(fresh(true));
    const state = fresh(false);
    const carried = walkPage(state);
    editAttachRepertoires(state);
    editCollapseGlyphLines(state, resources);
    if (state.items.length > MAX_EDIT_TARGETS) {
      failReason("too-large", `edit-targets: more than ${MAX_EDIT_TARGETS} targets on this page`);
    }
    editAttachGrowthLimits(state);
    editAttachGeometry(state);
    const paragraphs = editFindParagraphs(state);
    return {
      items: state.items,
      records: state.records,
      paragraphs,
      balancedContent: carried.saved.length === 0 && carried.current.cms.length === 0,
      transform: pageTransform,
    };
  } finally {
    page.destroy();
  }
}

/**
 * Walk one content stream (a page /Contents element or a Form XObject) with the
 * carried graphics/text state. The lexer is deliberately partial: it knows the
 * tokens and operators this unit needs and refuses a stream it cannot follow
 * safely, rather than guessing.
 */
export function walkEditContent(
  state: EditScanState,
  stream: MupdfObject,
  path: string,
  depth: number,
  resources: MupdfObject | null,
  structuralUnproven: boolean,
  cow: EditCow | null,
  carried: EditCarried,
): void {
  state.budget.traversals += 1;
  if (state.budget.traversals > MAX_EDIT_TRAVERSALS) {
    failReason("too-large", `edit-targets: more than ${MAX_EDIT_TRAVERSALS} content streams on this page`);
  }
  if (!stream.isStream()) failReason("invalid", "edit-targets: a /Contents entry is not a stream");
  let bytes: Uint8Array;
  try {
    const buffer = stream.readStream();
    try {
      const view = buffer.asUint8Array();
      if (state.budget.bytes + view.length > MAX_EDIT_SCAN_BYTES) {
        failReason("too-large", `edit-targets: the decoded streams exceed the ${MAX_EDIT_SCAN_BYTES} byte scan ceiling`);
      }
      bytes = view.slice();
    } finally {
      buffer.destroy();
    }
  } catch (err) {
    if ((err as { reason?: string } | null)?.reason) throw err;
    failReason("invalid", `edit-targets: a content stream cannot be read (${errorMessage(err)})`);
  }
  state.budget.bytes += bytes.length;
  if (depth === 0) state.streams.set(path, bytes);
  const objectNumber = attempt(() => (stream.isIndirect() ? stream.asIndirect() : -1), -1);

  type TextState = EditTextState;
  // Content streams of one page are concatenated with a whitespace, so graphics
  // and text state (including the q/Q stack) carries across the /Contents array.
  // A Form XObject gets a fresh state: its descendants are unproven targets, and
  // this scanner does not claim to inherit a font into a form.
  let current: TextState = { ...carried.current };
  const savedStates: TextState[] = [...carried.saved];
  const stack: EditOperand[][] = [[]];
  // The opener token of each open container, so an operand records the whole span
  // (a `<< … >>` property list is re-lexed later from its own bytes).
  const openers: { container: "array" | "dict"; start: number; end: number }[] = [];
  let pendingOperands = 0;
  let position = 0;
  let pendingImage = false;
  // U2 tail tracking: within one BT block, a text-show with no repositioning
  // between it and the next text-show means the next origin depends on this run's
  // advance, so the width may not change.
  let lastShow: { record: EditTargetRecord; item: EditTarget } | null = null;
  let positionedSinceShow = false;
  // P6 U2 §3.1/§9.10–11: the previous show in this BT and the exact text-space translation
  // since it, or null once anything but `Td`/`TD`/`T*` sits between them.
  let prevShow: EditTargetRecord | null = null;
  let between: number[] | null = [0, 0];

  const emit = (record: EditTargetRecord): EditTarget => {
    if (state.items.length >= MAX_EDIT_RAW_TARGETS) {
      failReason("too-large", `edit-targets: more than ${MAX_EDIT_RAW_TARGETS} targets on this page`);
    }
    // Review fix 3: a target that is not a one-byte show in a glyph block survives the collapse,
    // so the 513th of those refuses here, before any more records pay for structured-text matching.
    const joinable = record.kind === "text" && record.depth === 0 && record.payloadBytes?.length === 1
      && /^c\d+$/.test(record.path) && editGlyphBlocksOf(state, record.path).has(record.operandStart);
    if (!joinable && ++state.ordinaryTargets > MAX_EDIT_TARGETS) {
      failReason("too-large", `edit-targets: more than ${MAX_EDIT_TARGETS} targets on this page`);
    }
    const item: EditTarget = { targetId: record.targetId, kind: record.kind, bounds: record.bounds ?? null, support: record.support };
    if (record.kind === "text") {
      item.tailFollows = record.tailFollows === true;
      if (record.kerned) item.kerned = true;
    }
    if (record.cow) item.cowable = true;
    if (record.reason !== undefined) item.reason = record.reason;
    if (record.text !== undefined) {
      if (record.text.length > MAX_EDIT_TEXT_CHARS) {
        failReason("too-large", `edit-targets: a text target is over the ${MAX_EDIT_TEXT_CHARS} character ceiling`);
      }
      if (state.budget.replyText + record.text.length > MAX_EDIT_REPLY_TEXT_CHARS) {
        failReason("too-large", `edit-targets: the reply text is over the ${MAX_EDIT_REPLY_TEXT_CHARS} character ceiling`);
      }
      state.budget.replyText += record.text.length;
      item.text = record.text;
    }
    if (record.font !== undefined) item.font = record.font;
    if (record.placement) {
      const { box, visible, orient, clip } = record.placement;
      item.placement = clip ? { box: visible, orient, full: box } : { box: visible, orient };
    }
    if (record.objectOps) item.objectOps = record.objectOps;
    if (record.objectName !== undefined) item.objectName = record.objectName;
    if (record.image) item.image = record.image;
    state.items.push(item);
    state.records.set(record.targetId, record);
    return item;
  };

  const pushOperand = (value: EditOperand): void => {
    pendingOperands += 1;
    if (pendingOperands > MAX_EDIT_OPERANDS) {
      failReason("too-large", `edit-targets: more than ${MAX_EDIT_OPERANDS} pending operands`);
    }
    stack[0].push(value);
  };

  /**
   * The declared length of an inline image, or null when this build will not guess:
   * `/L`//`Length` wins; otherwise an unfiltered image's size is computed from
   * `/W /H /BPC /CS` (and `/IM true` = 1 bit per pixel). A filtered image stays
   * refused: its data boundary is not computable here.
   */
  const declaredInlineLength = (operands: EditOperand[]): number | null => {
    let filtered = false;
    const numbers: Record<string, number> = {};
    let colorSpace: string | null = null;
    let imageMask = false;
    for (let index = 0; index + 1 < operands.length; index++) {
      const key = operands[index];
      const value = operands[index + 1];
      if (key.t !== "name") continue;
      if (key.name === "F" || key.name === "Filter") filtered = true;
      if ((key.name === "L" || key.name === "Length") && value.t === "num" && Number.isInteger(value.n) && value.n >= 0) {
        return value.n;
      }
      if (value.t === "num" && Number.isFinite(value.n)) numbers[key.name] = value.n;
      if (key.name === "CS" && value.t === "name") colorSpace = value.name;
      if (key.name === "IM" && value.t === "name") imageMask = value.name === "true";
    }
    if (filtered) return null;
    const width = numbers.W;
    const height = numbers.H;
    const bits = numbers.BPC;
    if (!Number.isInteger(width) || !Number.isInteger(height) || width < 0 || height < 0) return null;
    if (width === 0 || height === 0) return 0;
    let components: number | null = null;
    if (imageMask) components = 1;
    else if (colorSpace === "G" || colorSpace === "DeviceGray" || colorSpace === "I" || colorSpace === "Indexed" || colorSpace === "DeviceRGB" && false) components = 1;
    else if (colorSpace === "RGB" || colorSpace === "DeviceRGB") components = 3;
    else if (colorSpace === "CMYK" || colorSpace === "DeviceCMYK") components = 4;
    if (components === null) return null;
    const depth = imageMask ? 1 : Number.isInteger(bits) && bits > 0 ? bits : null;
    if (depth === null) return null;
    const length = Math.ceil((width * components * depth) / 8) * height;
    return Number.isFinite(length) && length >= 0 ? length : null;
  };

  /**
   * An inline image is not parsed: its data may contain any byte, so the scanner
   * jumps over exactly the bytes the image declares and then requires EI. When the
   * length cannot be established the request is refused instead of reporting
   * targets that may be image bytes.
   */
  const skipInlineImage = (from: number, operands: EditOperand[]): void => {
    const declared = declaredInlineLength(operands);
    if (declared === null) {
      // Without a length the data boundary is a guess: an `EI` inside the data is
      // indistinguishable from the real end marker (astra m42).
      failReason("invalid", "edit-targets: an inline image has no declared /L, so its boundary is ambiguous");
    }
    // PDF 32000-1: `ID` is followed by one whitespace (a CRLF is one EOL), then
    // exactly the data; the EI test starts after those bytes.
    if (!editIsWhite(bytes[from])) {
      failReason("invalid", "edit-targets: an inline image's ID is not followed by whitespace");
    }
    let start = from + 1;
    if (bytes[from] === 13 && bytes[start] === 10) start += 1;
    const end = start + declared;
    if (!editIsWhite(bytes[end])) {
      failReason("invalid", "edit-targets: an inline image length does not end at a boundary");
    }
    let cursor = end;
    while (cursor < bytes.length && editIsWhite(bytes[cursor])) cursor++;
    if (bytes[cursor] !== 0x45 || bytes[cursor + 1] !== 0x49) {
      failReason("invalid", "edit-targets: an inline image has no EI after its declared length");
    }
    position = cursor + 2;
  };

  const nextToken = (): EditToken | null => {
    for (;;) {
      while (position < bytes.length && editIsWhite(bytes[position])) position++;
      if (position >= bytes.length) return null;
      if (bytes[position] === 0x25) {
        while (position < bytes.length && bytes[position] !== 10 && bytes[position] !== 13) position++;
        continue;
      }
      break;
    }
    state.budget.tokens += 1;
    if (state.budget.tokens > MAX_EDIT_TOKENS) {
      failReason("too-large", `edit-targets: more than ${MAX_EDIT_TOKENS} tokens on this page`);
    }
    const start = position;
    const byte = bytes[position];
    if (byte === 0x5b) {
      position += 1;
      return { k: "open", container: "array", start, end: position };
    }
    if (byte === 0x5d) {
      position += 1;
      return { k: "close", container: "array", start, end: position };
    }
    if (byte === 0x3c && bytes[position + 1] === 0x3c) {
      position += 2;
      return { k: "open", container: "dict", start, end: position };
    }
    if (byte === 0x3e && bytes[position + 1] === 0x3e) {
      position += 2;
      return { k: "close", container: "dict", start, end: position };
    }
    if (byte === 0x3c) {
      position += 1;
      const out: number[] = [];
      let high = -1;
      for (;;) {
        if (position >= bytes.length) failReason("invalid", "edit-targets: an unterminated hex string");
        const next = bytes[position];
        if (editIsWhite(next)) { position++; continue; }
        if (next === 0x3e) {
          position += 1;
          if (high >= 0) out.push(high << 4);
          return { k: "operand", value: { t: "str", bytes: Uint8Array.from(out), start, end: position } };
        }
        const digit = editHexDigit(next);
        if (digit < 0) failReason("invalid", "edit-targets: a hex string holds a non-hex character");
        position += 1;
        if (high < 0) high = digit;
        else { out.push((high << 4) | digit); high = -1; }
      }
    }
    if (byte === 0x3e) failReason("invalid", "edit-targets: an unexpected '>' in a content stream");
    if (byte === 0x28) {
      position += 1;
      const out: number[] = [];
      let nesting = 1;
      for (;;) {
        if (position >= bytes.length) failReason("invalid", "edit-targets: an unterminated literal string");
        const next = bytes[position];
        if (next === 0x5c) {
          position += 1;
          const escaped = bytes[position];
          if (escaped === undefined) failReason("invalid", "edit-targets: an unterminated literal string");
          if (escaped >= 0x30 && escaped <= 0x37) {
            let value = 0;
            let digits = 0;
            while (digits < 3 && position < bytes.length && bytes[position] >= 0x30 && bytes[position] <= 0x37) {
              value = value * 8 + (bytes[position] - 0x30);
              position += 1;
              digits += 1;
            }
            out.push(value & 0xff);
            continue;
          }
          if (escaped === 0x6e) { out.push(10); position += 1; continue; }
          if (escaped === 0x72) { out.push(13); position += 1; continue; }
          if (escaped === 0x74) { out.push(9); position += 1; continue; }
          if (escaped === 0x62) { out.push(8); position += 1; continue; }
          if (escaped === 0x66) { out.push(12); position += 1; continue; }
          if (escaped === 10) { position += 1; continue; }
          if (escaped === 13) { position += 1; if (bytes[position] === 10) position += 1; continue; }
          out.push(escaped);
          position += 1;
          continue;
        }
        if (next === 0x28) {
          nesting += 1;
          if (nesting > MAX_EDIT_NESTING) failReason("too-large", `edit-targets: literal-string nesting over ${MAX_EDIT_NESTING}`);
          out.push(next);
          position += 1;
          continue;
        }
        if (next === 0x29) {
          nesting -= 1;
          position += 1;
          if (nesting === 0) return { k: "operand", value: { t: "str", bytes: Uint8Array.from(out), start, end: position } };
          out.push(next);
          continue;
        }
        out.push(next);
        position += 1;
      }
    }
    if (byte === 0x2f) {
      position += 1;
      const out: number[] = [];
      while (position < bytes.length && editIsRegular(bytes[position])) {
        if (bytes[position] === 0x23) {
          const high = editHexDigit(bytes[position + 1]);
          const low = editHexDigit(bytes[position + 2]);
          if (high >= 0 && low >= 0) {
            out.push((high << 4) | low);
            position += 3;
            continue;
          }
        }
        out.push(bytes[position]);
        position += 1;
      }
      return { k: "operand", value: { t: "name", name: editBytesToLatin1(Uint8Array.from(out)), start, end: position } };
    }
    // A number and an operator both run to the next delimiter; the regex decides.
    while (position < bytes.length && editIsRegular(bytes[position])) position += 1;
    if (position === start) {
      failReason("invalid", `edit-targets: an unexpected byte 0x${byte.toString(16)} in a content stream`);
    }
    const run = editBytesToLatin1(bytes.subarray(start, position));
    if (/^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/.test(run)) {
      return { k: "operand", value: { t: "num", n: Number(run), start, end: position } };
    }
    return { k: "op", op: run, start, end: position };
  };

  /**
   * The marked-content scope a `BDC` opens: optional content (hidden layers are
   * never offered), an ActualText span that must travel with the operand, and an
   * MCID that is left alone. A `/Properties` entry is a shared resource, so an
   * ActualText reached that way cannot be attributed to this run.
   */
  const markedScope = (operands: EditOperand[]): EditMarkedScope => {
    const scope: EditMarkedScope = { optional: false, actualText: null, sharedActualText: false, mcid: null };
    const tag = operands[0];
    if (tag && tag.t === "name" && tag.name === "OC") scope.optional = true;
    const properties = operands.length > 1 ? operands[operands.length - 1] : null;
    if (properties && properties.t === "dict") {
      // Re-lex the dictionary's own bytes with the same lexer, then restore the
      // walk position: nothing outside this scope is consumed.
      const saved = position;
      position = properties.start + 2;
      let depth = 0;
      let lastName: string | null = null;
      try {
        for (;;) {
          if (position >= properties.end) break;
          const token = nextToken();
          if (!token) break;
          if (token.k === "open") { depth += 1; continue; }
          if (token.k === "close") {
            if (depth === 0) break;
            depth -= 1;
            continue;
          }
          if (token.k !== "operand" || depth !== 0) continue;
          const value = token.value;
          if (value.t === "name") {
            if (lastName === "Type" && (value.name === "OCG" || value.name === "OCMD")) scope.optional = true;
            lastName = value.name;
            continue;
          }
          if (lastName === "ActualText" && value.t === "str") {
            scope.actualText = { start: value.start, end: value.end, bytes: value.bytes };
          } else if (lastName === "MCID" && value.t === "num") {
            scope.mcid = Math.trunc(value.n);
          }
          lastName = null;
        }
      } finally {
        position = saved;
      }
      return scope;
    }
    if (properties && properties.t === "name") {
      const dict = at(at(resources, "Properties"), properties.name);
      if (dict) {
        const type = at(dict, "Type");
        if (type && type.isName() && (type.asName() === "OCG" || type.asName() === "OCMD")) scope.optional = true;
        if (at(dict, "ActualText")) scope.sharedActualText = true;
        const mcid = at(dict, "MCID");
        if (mcid && mcid.isNumber()) scope.mcid = Math.trunc(mcid.asNumber());
      }
    }
    return scope;
  };

  const classifyText = (operands: EditOperand[], token: EditToken & { k: "op" }, unproven: boolean): void => {
    const lone = operands.length === 1 && operands[0].t === "str" ? operands[0] : null;
    const array = token.op === "TJ" && operands.length === 1 && operands[0].t === "arr" ? operands[0] : null;
    const validArray = array && array.items.every(item => item.t === "str" || item.t === "num") ? array : null;
    const strings = validArray ? validArray.items.filter((item): item is Extract<EditOperand, { t: "str" }> => item.t === "str") : [];
    const joined = new Uint8Array(strings.reduce((total, item) => total + item.bytes.length, 0));
    let joinedAt = 0;
    for (const item of strings) { joined.set(item.bytes, joinedAt); joinedAt += item.bytes.length; }
    const payload = lone ?? (validArray ? { t: "str" as const, bytes: joined, start: validArray.start, end: validArray.end } : null);
    const tjAdjustment = validArray
      ? validArray.items.reduce((total, item) => total + (item.t === "num" ? item.n : 0), 0)
      : 0;
    const font = editResolveFont(resources, current.fontName, state);
    const cidDecoded = font?.cid && payload ? editDecodeCid(payload.bytes, font.cid) : null;
    // U3: a code drawn anywhere on the page with this font object proves its glyph,
    // even when the run itself is refused for another reason (OC, clipping, sharing).
    if (font?.cid && payload && font.objectNumber > 0) {
      let codes = state.repertoire.get(font.objectNumber);
      if (!codes) {
        codes = new Set();
        state.repertoire.set(font.objectNumber, codes);
      }
      for (const code of editCodesOf(payload.bytes, font.cid.bytesPerCode)) codes.add(code);
    }
    const ocDepth = carried.scopes.reduce((count, scope) => count + (scope.optional ? 1 : 0), 0);
    const actuals = carried.scopes
      .map(scope => scope.actualText)
      .filter((span): span is EditActualText => span !== null);
    const sharedActualText = carried.scopes.some(scope => scope.sharedActualText);
    const mcid = [...carried.scopes].reverse().find(scope => scope.mcid !== null)?.mcid ?? null;
    let reason: string | null = null;
    if (!current.inText) reason = "unsupported-graphics-state";
    // Tr 3 is invisible and 4-7 clip: a text-show under either is not a visible
    // content target. Only the three visible modes may become candidates.
    else if (current.renderMode !== 0 && current.renderMode !== 1 && current.renderMode !== 2) {
      reason = "unsupported-graphics-state";
    }
    // Text on an optional-content layer may be hidden; visibility is not resolved here.
    else if (ocDepth > 0) reason = "unsupported-graphics-state";
    else if (!payload || payload.bytes.length === 0) reason = "unsupported-text-operator";
    // U1b: a shared path whose every obstacle is a proven shape goes on through the
    // font, association and tag gates; the edit privatises the placement first.
    else if ((depth > 0 || unproven) && !cow) reason = "shared-or-unproven-sharing";
    const cowable = !reason && (depth > 0 || unproven) ? cow : null;
    // S1: the pen sits at Tlm only before this line's first show; the stext origin then disambiguates.
    const runOrigin = (lastShow === null || positionedSinceShow) && current.tlm && current.ctm
      ? editAffine(editAffine(current.tlm, current.ctm), state.pageTransform).slice(4) : null;
    if (!reason && payload) {
      // RTL/shaping text is refused by its encoding, never by looking at a render.
      // A proven Identity-H font decodes first: the decoded code points are what
      // says `rtl-or-shaping`, and a missing or unmappable code is
      // `unsupported-encoding` before any other gate runs.
      if (!font) reason = "unsupported-encoding";
      else if (font.writingMode === 1) reason = "vertical-writing";
      else if (font.subtype === "Type3") reason = "type3";
      else if (font.subtype === "Type0") {
        if (!font.cid) reason = font.subset ? "subset-font" : "unsupported-encoding";
        else if (!cidDecoded) reason = "unsupported-encoding";
        else if (editIsRtlOrShaping(cidDecoded.text)) reason = "rtl-or-shaping";
      } else if (font.cid) {
        // Coded simple font (D2): the 1-byte /ToUnicode decode is the only text it has.
        if (!cidDecoded) reason = "unsupported-encoding";
        else if (editIsRtlOrShaping(cidDecoded.text)) reason = "rtl-or-shaping";
      } else if (!font.simple || font.encodingUnknown) reason = "unsupported-encoding";
      else if (editDecodeWinAnsi(payload.bytes) === null) reason = "unsupported-encoding";
    }
    const text = cidDecoded
      ? cidDecoded.text
      : payload && font && font.simple && !font.encodingUnknown && !font.cid
        ? editDecodeWinAnsi(payload.bytes) ?? undefined
        : undefined;
    let bounds: number[] | undefined;
    if (!reason && text !== undefined && !state.countOnly) {
      state.runs ??= editStructuredRuns(state.page);
      const match = editMatchRuns(state.runs, text, font?.summary.name ?? null, cowable !== null, token.op === "TJ", runOrigin, depth);
      if (!match) reason = "ambiguous-text-association";
      else bounds = match.bounds ?? undefined;
    }
    // Tag gate: only an ActualText that is unshared, single and byte-equal to the
    // operand can be rewritten with it, so anything else is never a candidate.
    let actualText: EditActualText | null = null;
    let structActualText: string | undefined;
    if (!reason && payload && text !== undefined) {
      if (sharedActualText || actuals.length > 1) reason = "ambiguous-text-association";
      else if (actuals.length === 1) {
        // A decoded CID run carries its text as a UTF-16BE ActualText, not as the
        // Identity-H payload bytes, so the equality is on the decoded text.
        const same = cidDecoded ? editDecodeActualText(actuals[0].bytes) === text : editBytesEqual(actuals[0].bytes, payload.bytes);
        if (same) actualText = actuals[0];
        else reason = "ambiguous-text-association";
      }
      // The other place an ActualText can live: the StructElem this MCID maps to.
      // A mismatch, or an inline and a struct ActualText at once, is ambiguous.
      // A Form's MCIDs map through the Form's own /StructParents, which this walk
      // does not resolve, so a tagged run inside a Form is not attributed.
      if (!reason && mcid !== null && depth > 0) reason = "ambiguous-text-association";
      if (!reason && mcid !== null) {
        const element = editStructElement(state.pdf, state.pageObject, mcid);
        const tagged = element ? at(element, "ActualText") : null;
        if (tagged && tagged.isString()) {
          const taggedText = attempt(() => tagged.asString(), null);
          if (taggedText === null || taggedText !== text || actualText) reason = "ambiguous-text-association";
          else structActualText = text;
        }
      }
    }
    const payloadText = payload ? editBytesToLatin1(payload.bytes) : "";
    const record: EditTargetRecord = {
      targetId: editTargetId(state.pageIndex, path, token.start, `${token.op}|${payloadText}`),
      kind: "text", page: state.pageIndex, path, objectNumber,
      opStart: token.start, opEnd: token.end,
      operandStart: payload ? payload.start : token.start,
      operandEnd: payload ? payload.end : token.end,
      depth, support: reason ? "unsupported" : "candidate",
      tailFollows: false, actualText: null, mcid, ocDepth,
      horizontalScale: current.horizontalScale,
      characterSpacing: current.characterSpacing,
      wordSpacing: current.wordSpacing,
      renderMode: current.renderMode,
      tjAdjustment,
      kerned: token.op === "TJ" ? true : undefined,
      winAnsiEncoding: font?.winAnsiEncoding,
      textLinear: current.tm && current.lin ? editMultiply(editMultiply(current.tm, current.lin), state.pageLinear) : null,
      firstOnLine: lastShow === null || positionedSinceShow,
      origin: runOrigin,
      tfName: current.fontName, tfSize: current.fontSize, leading: current.leading,
      prevShowId: prevShow?.targetId ?? null, between: prevShow ? between : null,
    };
    if (reason) record.reason = reason;
    if (text !== undefined) record.text = text;
    // An unresolved font is reported as explicitly unknown, never omitted: the
    // reply has to distinguish "no font" from "font we could not name".
    record.font = font ? font.summary : { name: null, subtype: null, embedded: null, subset: null, writingMode: null };
    if (bounds) record.bounds = bounds;
    if (payload) record.payloadBytes = payload.bytes;
    if (actualText) record.actualText = actualText;
    if (structActualText !== undefined) record.structActualText = structActualText;
    if (!reason && cowable) record.cow = cowable;
    if (!reason && cidDecoded && font?.cid) {
      record.fontSize = current.fontSize;
      record.cidKey = font.objectNumber;
    }
    record.budget = state.budget;
    // A text-show that follows this one with no repositioning between them makes
    // the tail's origin depend on this run's advance.
    if (lastShow && !positionedSinceShow) {
      lastShow.record.tailFollows = true;
      lastShow.record.tailTargetId = record.targetId;
      lastShow.item.tailFollows = true;
    }
    const item = emit(record);
    if (!reason && cidDecoded && font?.cid) state.cidPending.push({ record, item, cid: font.cid });
    lastShow = { record, item };
    positionedSinceShow = false;
    prevShow = record;
    between = [0, 0];
  };

  /**
   * U4 (amended §7.1–3): the `q … cm … Do Q` block a placement can be edited in.
   * The whole block must be this one /Contents element, be the only thing since its
   * own `q`, have exactly one `cm`, no `cm` in any enclosing state, an unflipped
   * axis-aligned scale, and the `Do` must be followed by the matching `Q`.
   */
  const simplePlacement = (token: EditToken & { k: "op" }, rect: number[]): EditPlacement | null => {
    if (savedStates.length !== 1 || current.qToken === null || current.qPath !== path) return null;
    if (!current.clean || current.cms.length !== 1) return null;
    if (!savedStates.every(saved => saved.cms.length === 0)) return null;
    const matrix = current.cms[0].matrix;
    // P6 U6 §1.1: any of the eight axis-aligned orientations.
    const orient = matrix.length === 6 ? editOrientOf(matrix) : null;
    if (!orient) return null;
    const qEnd = editNextQEnd(bytes, token.end);
    if (qEnd === null) return null;
    const box = editMapRect(matrix, rect);
    // P6 U6 §1.1/§9.5: one `re W n` right after `q`; visible = box ∩ clip, at least 1 pt a side.
    const clipRe = current.clipRe && current.clipRe.path === path ? current.clipRe : null;
    let visible = box;
    let clip: EditClip | null = null;
    if (clipRe) {
      visible = [Math.max(box[0], clipRe.rect[0]), Math.max(box[1], clipRe.rect[1]), Math.min(box[2], clipRe.rect[2]), Math.min(box[3], clipRe.rect[3])];
      if (!(visible[2] - visible[0] >= 1) || !(visible[3] - visible[1] >= 1)) return null;
      clip = { rect: clipRe.rect, start: clipRe.start, end: clipRe.end, opsStart: clipRe.opsStart, opsEnd: clipRe.opsEnd };
    }
    return {
      cmStart: current.cms[0].start, cmEnd: current.cms[0].end, qStart: current.qToken.start, qEnd,
      box, rect, view: editViewBox(visible, state.pageTransform), matrix, orient, clip, visible,
    };
  };

  /** U4 (§7.2): a Form's `/BBox` under its `/Matrix`, the rect its own `cm` maps to the page. */
  const formRect = (target: MupdfObject): number[] | null => {
    const bbox = editRectOf(at(target, "BBox"));
    const matrix = editFormMatrix(target);
    if (!bbox || !matrix) return null;
    return [
      matrix[0] * bbox[0] + matrix[4], matrix[3] * bbox[1] + matrix[5],
      matrix[0] * bbox[2] + matrix[4], matrix[3] * bbox[3] + matrix[5],
    ];
  };

  const classifyXObject = (operands: EditOperand[], token: EditToken & { k: "op" }, unproven: boolean): void => {
    const payload = operands[0] && operands[0].t === "name" ? operands[0] : null;
    const base: Omit<EditTargetRecord, "kind" | "support" | "targetId"> = {
      page: state.pageIndex, path, objectNumber,
      opStart: token.start, opEnd: token.end,
      operandStart: payload ? payload.start : token.start,
      operandEnd: payload ? payload.end : token.end,
      depth, budget: state.budget,
    };
    if (!payload) {
      emit({ ...base, targetId: editTargetId(state.pageIndex, path, token.start, "Do|"), kind: "form", support: "unsupported", reason: "unsupported-graphics-state" });
      return;
    }
    const xobjects = at(resources, "XObject");
    const target = xobjects ? at(xobjects, payload.name) : null;
    if (!target) {
      emit({ ...base, targetId: editTargetId(state.pageIndex, path, token.start, `Do|${payload.name}`), kind: "form", support: "unsupported", reason: "unresolved-resource" });
      return;
    }
    const subtypeObject = at(target, "Subtype");
    const subtype = subtypeObject && subtypeObject.isName() ? subtypeObject.asName() : null;
    const reference = attempt(() => (target.isIndirect() ? target.asIndirect() : -1), -1);
    const targetId = editTargetId(state.pageIndex, path, token.start, `Do|${payload.name}`);
    const ocDepth = carried.scopes.reduce((count, scope) => count + (scope.optional ? 1 : 0), 0);
    if (subtype === "Image") {
      const number = (key: string): number => {
        const value = at(target, key);
        return value && value.isNumber() ? Math.trunc(value.asNumber()) : 0;
      };
      const smaskValue = at(target, "SMask");
      const interpolate = at(target, "Interpolate");
      const image: EditObjectImage = {
        width: number("Width"),
        height: number("Height"),
        smask: !!smaskValue && !(smaskValue.isName() && smaskValue.asName() === "None"),
        mask: !!at(target, "Mask"),
        decode: !!at(target, "Decode"),
        interpolate: interpolate ? attempt(() => interpolate.asBoolean(), true) : false,
        oc: !!at(target, "OC") || ocDepth > 0,
      };
      // A mask, a custom /Decode, an image-grain flag or a layer can only be carried
      // by a replacement that keeps this placement's own geometry.
      const clean = !image.smask && !image.mask && !image.decode && !image.interpolate && !image.oc;
      const placement = simplePlacement(token, [0, 0, 1, 1]);
      let ops: EditObjectAction[];
      let reason: string | undefined;
      if (state.geometryBlocked) { ops = []; reason = "unsupported-transform"; }
      else if (depth > 0) { ops = []; reason = "shared-or-unproven-sharing"; }
      else if (!clean) { ops = placement ? ["move", "resize", "delete", "duplicate", "transform", "crop"] : []; reason = "unsupported-image"; }
      else if (placement) { ops = ["move", "resize", "delete", "replace", "duplicate", "transform", "crop"]; reason = undefined; }
      else { ops = ["replace"]; reason = "unsupported-transform"; }
      const record: EditTargetRecord = {
        ...base, targetId, kind: "image", objectNumber: reference > 0 ? reference : objectNumber,
        support: ops.length ? "candidate" : "unsupported", objectName: payload.name, image,
      };
      if (ops.length) record.objectOps = ops;
      if (reason) record.reason = reason;
      if (ops.length && placement) {
        record.placement = placement;
        record.bounds = placement.visible;
      }
      if (ops.length && unproven && cow) record.cow = cow;
      emit(record);
      return;
    }
    const isForm = subtype === "Form";
    if (!isForm) {
      emit({
        ...base, targetId, kind: "form", objectNumber: reference > 0 ? reference : objectNumber,
        support: "unsupported", reason: "unsupported-xobject", objectName: payload.name,
      });
      return;
    }
    const rect = formRect(target);
    const placement = rect ? simplePlacement(token, rect) : null;
    let ops: EditObjectAction[];
    let reason: string | undefined;
    if (state.geometryBlocked) { ops = []; reason = "unsupported-transform"; }
    else if (depth > 0) { ops = []; reason = "shared-or-unproven-sharing"; }
    else if (!placement) { ops = []; reason = "unsupported-transform"; }
    else { ops = ["move", "resize", "delete", "duplicate", "transform", "crop"]; reason = undefined; }
    const form: EditTargetRecord = {
      ...base, targetId, kind: "form", objectNumber: reference > 0 ? reference : objectNumber,
      support: ops.length ? "candidate" : "unsupported", objectName: payload.name,
    };
    if (ops.length) form.objectOps = ops;
    if (reason) form.reason = reason;
    if (ops.length && placement) {
      form.placement = placement;
      form.bounds = placement.visible;
    }
    if (ops.length && unproven && cow) form.cow = cow;
    emit(form);
    if (depth + 1 > MAX_EDIT_FORM_DEPTH) {
      failReason("too-large", `edit-targets: Form nesting over ${MAX_EDIT_FORM_DEPTH}`);
    }
    // A cycle is not an error, it is a path that stops here; repeated invocations
    // elsewhere still get their own targets because only this path is tracked.
    if (reference > 0) {
      if (state.activeForms.has(reference)) return;
      state.activeForms.add(reference);
    }
    // Only a Form invoked straight from a page /Contents element can be privatised
    // (rule 1a); a Form reached through another Form, or a cycle, stays refused.
    const elementIndex = Number(/^c(\d+)$/.exec(path)?.[1] ?? -1);
    const childCow: EditCow | null = cow && depth === 0 && reference > 0 && elementIndex >= 0
      ? { ...cow, form: { name: payload.name, reference, elementIndex, operandStart: payload.start, operandEnd: payload.end } }
      : null;
    // P6 U2: a Form's space is its /Matrix applied to the CTM at the `Do`.
    const formMatrix = at(target, "Matrix");
    const formNumbers = formMatrix ? editNumbersOf(formMatrix, 6) : [1, 0, 0, 1, 0, 0];
    const formLinear = formNumbers && current.lin ? editMultiply(formNumbers.slice(0, 4), current.lin) : null;
    try {
      walkEditContent(state, target, `${path}/${payload.name}@${token.start}`, depth + 1, at(target, "Resources"), unproven, childCow, {
        current: { fontName: null, fontSize: null, horizontalScale: 1, characterSpacing: 0, wordSpacing: 0, inText: false, renderMode: 0, cms: [], clean: true, qToken: null, qPath: null, lin: formLinear, tm: [1, 0, 0, 1], leading: 0,
          ctm: formNumbers && current.ctm ? editAffine(formNumbers, current.ctm) : null, tlm: [1, 0, 0, 1, 0, 0] }, saved: [], scopes: [],
      });
    } finally {
      if (reference > 0) state.activeForms.delete(reference);
    }
  };

  const applyOperator = (token: EditToken & { k: "op" }, operands: EditOperand[]): void => {
    // U4: outside q/Q/cm/Do, any operator means the enclosing block is no longer the
    // `q cm Do Q` shape a placement is edited in.
    // P6 U6 §1.1: `x y w h re W n` directly after `q` (before any `cm`) keeps the block
    // clean and becomes its clip; any other `re`/`W`/`n` (and `W*`) is unclean as before.
    const clipStage = current.clipStage ?? -1;
    let clipStep = false;
    if (token.op === "re" && clipStage === 0 && current.clean && current.cms.length === 0 && operands.length === 4 && operands.every(operand => operand.t === "num")) {
      const [x, y, w, h] = (operands as { t: "num"; n: number; start: number; end: number }[]);
      current.clipPending = {
        rect: [Math.min(x.n, x.n + w.n), Math.min(y.n, y.n + h.n), Math.max(x.n, x.n + w.n), Math.max(y.n, y.n + h.n)],
        start: current.qToken?.end ?? token.start, end: token.end, opsStart: x.start, opsEnd: h.end,
      };
      clipStep = true;
    } else if (token.op === "W" && clipStage === 1 && operands.length === 0) clipStep = true;
    else if (token.op === "n" && clipStage === 2 && operands.length === 0 && current.clipPending) {
      current.clipRe = { ...current.clipPending, end: token.end, path };
      clipStep = true;
    }
    if (clipStep) current.clipStage = clipStage + 1;
    else {
      // A half-read clip (`re` or `re W` then something else) is never a placement.
      if (clipStage === 1 || clipStage === 2) current.clean = false;
      current.clipStage = -1;
    }
    if (!clipStep && token.op !== "q" && token.op !== "Q" && token.op !== "cm" && token.op !== "Do") current.clean = false;
    if (between && token.op !== "Td" && token.op !== "TD" && token.op !== "T*" && token.op !== "Tj" && token.op !== "TJ") between = null;
    switch (token.op) {
      case "q":
        if (savedStates.length >= MAX_EDIT_NESTING) {
          failReason("too-large", `edit-targets: graphics-state nesting over ${MAX_EDIT_NESTING}`);
        }
        savedStates.push({ ...current, cms: [...current.cms] });
        current = { ...current, cms: [], clean: true, qToken: { start: token.start, end: token.end }, qPath: path, clipStage: 0, clipPending: null, clipRe: null };
        return;
      case "Q": {
        const popped = savedStates.pop();
        if (!popped) failReason("invalid", "edit-targets: a 'Q' has no matching 'q'");
        current = popped as TextState;
        return;
      }
      case "cm": {
        const six = operands.length === 6 && operands.every(operand => operand.t === "num")
          ? (operands as { t: "num"; n: number; start: number; end: number }[])
          : null;
        current.cms.push(six
          ? { start: six[0].start, end: six[5].end, matrix: six.map(operand => operand.n) }
          : { start: token.start, end: token.end, matrix: [] });
        current.lin = six && current.lin ? editMultiply(six.slice(0, 4).map(operand => operand.n), current.lin) : null;
        current.ctm = six && current.ctm ? editAffine(six.map(operand => operand.n), current.ctm) : null;
        return;
      }
      case "BT":
        current.inText = true;
        current.tm = [1, 0, 0, 1];
        current.tlm = [1, 0, 0, 1, 0, 0];
        prevShow = null;
        lastShow = null;
        positionedSinceShow = false;
        return;
      case "ET":
        current.inText = false;
        prevShow = null;
        lastShow = null;
        return;
      case "Tm":
        current.tm = operands.length === 6 && operands.every(operand => operand.t === "num")
          ? operands.slice(0, 4).map(operand => (operand as { n: number }).n) : null;
        current.tlm = current.tm ? operands.map(operand => (operand as { n: number }).n) : null;
        positionedSinceShow = true;
        return;
      case "TD":
        current.leading = operands.length === 2 && operands[1].t === "num" ? -operands[1].n : null;
        between = editTranslate(between, operands);
        current.tlm = editLineMove(current.tlm, editTranslate([0, 0], operands));
        positionedSinceShow = true;
        return;
      case "TL":
        current.leading = operands.length === 1 && operands[0].t === "num" ? operands[0].n : null;
        return;
      case "Td":
        between = editTranslate(between, operands);
        current.tlm = editLineMove(current.tlm, editTranslate([0, 0], operands));
        positionedSinceShow = true;
        return;
      case "T*":
        between = between && current.leading !== null ? [between[0], between[1] - current.leading] : null;
        current.tlm = editLineMove(current.tlm, current.leading !== null ? [0, -current.leading] : null);
        positionedSinceShow = true;
        return;
      case "BDC":
        carried.scopes.push(markedScope(operands));
        return;
      case "BMC":
        carried.scopes.push({ optional: false, actualText: null, sharedActualText: false, mcid: null });
        return;
      case "EMC":
        carried.scopes.pop();
        return;
      case "Tf": {
        const name = operands[0];
        const size = operands[1];
        current.fontName = name && name.t === "name" && size && size.t === "num" ? name.name : null;
        current.fontSize = name && name.t === "name" && size && size.t === "num" ? size.n : null;
        return;
      }
      case "Tr": {
        const mode = operands[0];
        // A non-integer mode is treated as unknown, which is not a visible mode.
        current.renderMode = mode && mode.t === "num" && Number.isInteger(mode.n) ? mode.n : -1;
        return;
      }
      case "Tz": {
        const value = operands[0];
        current.horizontalScale = value?.t === "num" && Number.isFinite(value.n) ? value.n / 100 : 1;
        return;
      }
      case "Tc": {
        const value = operands[0];
        current.characterSpacing = value?.t === "num" && Number.isFinite(value.n) ? value.n : 0;
        return;
      }
      case "Tw": {
        const value = operands[0];
        current.wordSpacing = value?.t === "num" && Number.isFinite(value.n) ? value.n : 0;
        return;
      }
      case "Tj":
      case "TJ":
        classifyText(operands, token, structuralUnproven);
        return;
      case "'":
      case "\"":
        classifyText([], token, structuralUnproven);
        // The show after `'`/`"` starts where that one's advance left the pen.
        current.tlm = null;
        lastShow = null;
        positionedSinceShow = true;
        return;
      case "Do":
        classifyXObject(operands, token, structuralUnproven);
        current.clean = false;
        return;
      case "BI":
        pendingImage = true;
        return;
      case "ID":
        if (pendingImage) {
          pendingImage = false;
          skipInlineImage(token.end, operands);
        }
        return;
      default:
        return;
    }
  };

  for (;;) {
    const token = nextToken();
    if (!token) break;
    if (token.k === "operand") {
      pushOperand(token.value);
      continue;
    }
    if (token.k === "open") {
      if (stack.length >= MAX_EDIT_NESTING) {
        failReason("too-large", `edit-targets: container nesting over ${MAX_EDIT_NESTING}`);
      }
      stack.unshift([]);
      openers.unshift(token);
      continue;
    }
    if (token.k === "close") {
      if (stack.length < 2) {
        failReason("invalid", `edit-targets: an unmatched '${token.container === "array" ? "]" : ">>"}'`);
      }
      const items = stack.shift() as EditOperand[];
      const opener = openers.shift() as { container: "array" | "dict"; start: number; end: number };
      pendingOperands -= items.length;
      pushOperand(opener.container === "array"
        ? { t: "arr", items, start: opener.start, end: token.end }
        : { t: "dict", start: opener.start, end: token.end });
      continue;
    }
    // An operator may only appear at the top level; anything else is not a
    // content stream this scanner can follow.
    if (stack.length !== 1) {
      failReason("invalid", `edit-targets: the operator ${token.op} sits inside an unclosed container`);
    }
    const operands = stack[0];
    pendingOperands -= operands.length;
    stack[0] = [];
    applyOperator(token, operands);
  }
  if (stack.length !== 1) failReason("invalid", "edit-targets: a content stream ends inside a container");
  carried.current = current;
  carried.saved = savedStates;
}

// ---------------------------------------------------------------------------
// P6 U2 §3 + §9: paragraphs (scan post-pass, wrap, one proven span rewrite).
// ---------------------------------------------------------------------------

export const MAX_PARAGRAPH_LINES = 200;

/** The reply's paragraph (§3.2), all lengths in page pt in the `bounds` space. */
export type EditParagraphItem = {
  paragraphId: string; targetIds: string[]; bounds: number[]; left: number; width: number;
  leading: number; fontSize: number; lines: number; kerned: boolean; tagged: boolean;
};
/** The worker's paragraph: the reply plus the proof (§9.10) the rewrite depends on. */
export type EditParagraph = {
  item: EditParagraphItem; records: EditTargetRecord[];
  /** Text-space leading L and the exact translation the original span leaves behind. */
  leadingText: number; drift: number[];
};

/** Can this run be a paragraph line at all (§3.1 b, c, g; §9.12)? */
function editParagraphLine(record: EditTargetRecord): boolean {
  const linear = record.textLinear;
  return record.kind === "text" && record.support === "candidate" && !!record.bounds && !!record.text
    && record.firstOnLine === true && !record.tailFollows && record.cidWrite === undefined
    && record.font?.subtype !== "Type0" && !record.actualText && record.structActualText === undefined
    && !!record.tfName && !!record.tfSize && record.leading !== null && record.leading !== undefined
    // Upright only: text up is page up, so "below" and the leading are page-vertical.
    && !!linear && Math.abs(linear[1]) < 1e-6 && Math.abs(linear[2]) < 1e-6 && linear[0] > 0 && linear[3] < 0
    // The write needs a page element to rewrite: a top-level op or a depth-1 Form (same proof as the plan).
    && (!!record.cow?.form || /^c\d+$/.test(record.path ?? ""));
}

function editSameLineState(left: EditTargetRecord, right: EditTargetRecord): boolean {
  const close = (a: number | undefined | null, b: number | undefined | null) => Math.abs((a ?? 0) - (b ?? 0)) < 1e-6;
  return left.path === right.path && left.tfName === right.tfName && close(left.tfSize, right.tfSize)
    && close(left.characterSpacing, right.characterSpacing) && close(left.wordSpacing, right.wordSpacing)
    && close(left.horizontalScale, right.horizontalScale) && !!left.cow === !!right.cow
    && left.winAnsiEncoding === right.winAnsiEncoding
    && left.textLinear!.every((value, index) => close(value, right.textLinear![index]));
}

/** §3.1/§9.10–12: consecutive shows whose in-between ops compose to exactly (0, −L). */
export function editFindParagraphs(state: EditScanState): Map<string, EditParagraph> {
  const found = new Map<string, EditParagraph>();
  let chain: EditTargetRecord[] = [];
  let leading = 0;
  const flush = (): void => {
    const joined = chain.reduce((total, record) => total + (record.text?.length ?? 0) + 1, 0);
    if (chain.length >= 2 && chain.length <= MAX_PARAGRAPH_LINES && joined <= MAX_EDIT_TEXT_CHARS) {
      const first = chain[0];
      const scale = Math.hypot(first.textLinear![2], first.textLinear![3]);
      const boxes = chain.map(record => record.bounds!);
      const left = Math.min(...boxes.map(box => box[0]));
      const bounds = [left, Math.min(...boxes.map(box => box[1])), Math.max(...boxes.map(box => box[2])), Math.max(...boxes.map(box => box[3]))];
      const round = (value: number) => Math.round(value * 100) / 100;
      const paragraphId = `para.${first.targetId}`;
      const drift = chain.slice(1).reduce((total, record) => [total[0] + record.between![0], total[1] + record.between![1]], [0, 0]);
      found.set(paragraphId, {
        records: chain, leadingText: leading, drift,
        item: {
          paragraphId, targetIds: chain.map(record => record.targetId), bounds: bounds.map(round),
          left: round(left), width: round(bounds[2] - left), leading: round(leading * scale),
          fontSize: round(Math.abs(first.tfSize!) * scale), lines: chain.length,
          kerned: chain.some(record => record.kerned === true),
          tagged: chain.some(record => record.mcid !== null && record.mcid !== undefined),
        },
      });
    }
    chain = [];
  };
  for (const item of state.items) {
    const record = state.records.get(item.targetId);
    if (!record || record.kind !== "text") continue;
    if (!editParagraphLine(record)) { flush(); continue; }
    const previous = chain[chain.length - 1];
    const step = record.between;
    const linked = previous && record.prevShowId === previous.targetId && step && Math.abs(step[0]) < 1e-6 && step[1] < 0
      && editSameLineState(previous, record) && (chain.length === 1 || Math.abs(-step[1] - leading) < 1e-4);
    if (linked) {
      if (chain.length === 1) leading = -step![1];
      chain.push(record);
      continue;
    }
    flush();
    chain = [record];
  }
  flush();
  for (const paragraph of found.values()) {
    for (const record of paragraph.records) {
      const item = state.items.find(candidate => candidate.targetId === record.targetId);
      if (item) item.paragraphId = paragraph.item.paragraphId;
    }
  }
  return found;
}

export type EditParagraphRequest = { text: unknown; width?: unknown; move?: unknown; size?: unknown; leading?: unknown };
export type EditParagraphPlan = { lines: string[]; noop: boolean; elementIndex: number; span: EditSpanEdit };

/**
 * §3.3–3.6/§9: wrap the flow greedily, prove the box clears every obstacle and the page,
 * and build the one span rewrite. Refusals use the `edit-text` detail tokens.
 */
export function editPlanParagraph(page: MupdfPage, paragraph: EditParagraph, others: EditTargetRecord[], request: EditParagraphRequest, runs?: EditRun[]): EditParagraphPlan {
  const records = paragraph.records;
  const first = records[0];
  const last = records[records.length - 1];
  const linear = first.textLinear!;
  const scale = Math.hypot(linear[2], linear[3]);
  const fail = (detail: EditTextDetail, message: string, extra?: Record<string, unknown>): never => failEditText(detail, `edit-paragraph: ${message}`, extra);
  if (typeof request.text !== "string" || request.text.length > MAX_EDIT_TEXT_CHARS) fail("invalid", `text is a string of at most ${MAX_EDIT_TEXT_CHARS} characters`);
  const number = (value: unknown, what: string, low: number, high: number): number | undefined => {
    if (value === undefined) return undefined;
    if (typeof value !== "number" || !Number.isFinite(value) || value < low || value > high) fail("invalid", `${what} is a number ${low}..${high}`);
    return value as number;
  };
  const width = number(request.width, "width", 1, 14400) ?? paragraph.item.width;
  const size = number(request.size, "size", 1, 400);
  const leadingPage = number(request.leading, "leading", 0.1, 2000);
  // Hard breaks stay; every other whitespace run is one space (§3.3).
  const blocks = (request.text as string).split("\n").map(block => block.replace(/\s+/g, " ").trim());
  // A stray Enter at either end must not add a blank line (only interior blanks count).
  while (blocks.length > 1 && !blocks[blocks.length - 1]) blocks.pop();
  while (blocks.length > 1 && !blocks[0]) blocks.shift();
  if (!blocks.some(Boolean)) fail("invalid", "the text is empty");
  const winAnsi = first.winAnsiEncoding === true;
  const bad = [...blocks.join("")].filter(char => { const code = char.codePointAt(0) ?? 0; return code < 0x20 || (winAnsi ? winAnsiByte(code) < 0 : code > 0x7e); });
  if (bad.length) fail("unsupported-encoding", winAnsi ? "text is not WinAnsi-encodable" : "only printable ASCII can be written", { missing: bad.slice(0, 20) });
  // Size: the page-pt size is written as Tf = size / scale (§9.2).
  const oldTf = first.tfSize!;
  const newTf = size === undefined ? editNumber(oldTf) : editNumber(Math.sign(oldTf) * size / scale);
  if (Number(newTf) === 0) fail("invalid", "size rounds to zero");
  const ratio = Number(newTf) / oldTf;
  const leadingText = editNumber(leadingPage !== undefined ? leadingPage / scale : paragraph.leadingText * ratio);
  // Widths: glyph advances from the run's own font at the new size, Tc/Tw in text space (§3.4).
  const match = editMatchRuns(runs ?? editStructuredRuns(page), first.text!, first.font?.name ?? null, first.cow !== undefined, first.kerned === true, first.origin ?? null, first.depth);
  if (!match || !match.chars[0]?.font || !match.chars[0].size) fail("unmeasurable-width", "this paragraph's advance cannot be measured in this build");
  const chars = match!.chars;
  const name = first.font?.name ?? "";
  const fallback = winAnsi && first.font?.embedded === false && BASE14_FONTS.includes(name) && base14Font ? base14Font(name) : null;
  const advances = new Map<string, number>();
  const missing: string[] = [];
  const glyph = (char: string): number => {
    let value = advances.get(char);
    if (value === undefined) {
      const measured = editAdvance(chars, char, fallback);
      if (!measured) fail("unmeasurable-width", "this paragraph's advance cannot be measured in this build");
      if (measured!.missing.length) missing.push(char);
      value = measured!.width / chars[0].size * Math.abs(Number(newTf)) * scale;
      advances.set(char, value);
    }
    return value;
  };
  const th = first.horizontalScale ?? 1;
  const measure = (line: string): number => {
    let total = 0;
    for (const char of line) total += glyph(char) + ((first.characterSpacing ?? 0) + (char === " " ? first.wordSpacing ?? 0 : 0)) * scale;
    return total * th;
  };
  const lines: string[] = [];
  for (const block of blocks) {
    let line = "";
    for (const word of block ? block.split(" ") : []) {
      if (measure(word) > width + 0.01) fail("word-too-wide", `"${word}" is wider than ${width} pt`, { word, width });
      const next = line ? `${line} ${word}` : word;
      if (line && measure(next) > width + 0.01) { lines.push(line); line = word; } else line = next;
    }
    lines.push(line);
  }
  if (missing.length) fail("unsupported-encoding", "the font has no glyph for some characters", { missing: [...new Set(missing)].slice(0, 20) });
  if (lines.length > MAX_PARAGRAPH_LINES) fail("too-large", `a paragraph holds at most ${MAX_PARAGRAPH_LINES} lines`);
  // Move: page delta → text space through the inverse linear part (§9.1).
  let tx = "0", ty = "0";
  if (request.move !== undefined) {
    const move = request.move as { dx?: unknown; dy?: unknown } | null;
    if (!move || typeof move !== "object" || typeof move.dx !== "number" || typeof move.dy !== "number" || !Number.isFinite(move.dx) || !Number.isFinite(move.dy)) {
      fail("invalid", "move is { dx, dy } in page points");
    }
    const inverse = editInvert(linear)!;
    const { dx, dy } = move as { dx: number; dy: number };
    tx = editNumber(dx * inverse[0] + dy * inverse[2]);
    ty = editNumber(dx * inverse[1] + dy * inverse[3]);
  }
  const offset = [Number(tx) * linear[0] + Number(ty) * linear[2], Number(tx) * linear[1] + Number(ty) * linear[3]];
  const unchangedLeading = Math.abs(Number(leadingText) - paragraph.leadingText) < 1e-4;
  const noop = tx === "0" && ty === "0" && newTf === editNumber(oldTf) && unchangedLeading
    && lines.length === records.length && lines.every((line, index) => line === records[index].text);
  // §9.13: the new box clears every obstacle by 1 pt and stays on the page.
  const oldLeadingPage = paragraph.leadingText * scale;
  const newLeadingPage = Number(leadingText) * scale;
  const firstBox = first.bounds!;
  const box = (count: number): number[] => [
    paragraph.item.left + offset[0],
    firstBox[3] - (firstBox[3] - firstBox[1]) * Math.abs(ratio) + offset[1],
    paragraph.item.left + width + offset[0],
    last.bounds![3] + (count - 1) * newLeadingPage - (records.length - 1) * oldLeadingPage + offset[1],
  ];
  const own = paragraph.item.bounds;
  const hits = (candidate: number[]): boolean => others.some(record => {
    const other = record.kind === "text" ? record.bounds : record.placement?.view;
    if (!other || records.includes(record)) return false;
    // Something already overlapping the paragraph (a rule, a backdrop) is not an obstacle.
    if (other[0] < own[2] && own[0] < other[2] && other[1] < own[3] && own[1] < other[3]) return false;
    return other[0] < candidate[2] + 1 && candidate[0] < other[2] + 1 && other[1] < candidate[3] + 1 && candidate[1] < other[3] + 1;
  });
  if (!noop) {
    const clip = attempt(() => [...page.getBounds()] as number[], null);
    const target = box(lines.length);
    // Off the left, right or top is `outside-page`; running off the bottom is `overflow` below.
    if (clip && (target[0] < clip[0] - 0.5 || target[2] > clip[2] + 0.5 || target[1] < clip[1] - 0.5)) {
      fail("outside-page", "the paragraph would leave the page", { bounds: target.map(value => Math.round(value * 100) / 100) });
    }
    const fits = (count: number) => !hits(box(count)) && (!clip || box(count)[3] <= clip[3] + 0.5);
    if (!fits(lines.length)) {
      let maxLines = lines.length - 1;
      while (maxLines > 0 && !fits(maxLines)) maxLines--;
      fail("overflow", `the paragraph needs ${lines.length} lines; ${maxLines} fit`, { lines: lines.length, maxLines });
    }
  }
  // §3.5/§9.5/§9.8–9: one span, first operand to last operator, restoring Tlm and TL exactly.
  const encoder = new TextEncoder();
  const font = editNameSource(first.tfName!);
  const sized = newTf !== editNumber(oldTf);
  const negLeading = editNumber(-Number(leadingText));
  const parts: Uint8Array[] = [];
  const text = (value: string) => parts.push(encoder.encode(value));
  if (sized) text(`${font} ${newTf} Tf `);
  if (tx !== "0" || ty !== "0") text(`${tx} ${ty} Td `);
  lines.forEach((line, index) => {
    if (index) text(`0 ${negLeading} Td `);
    if (!line) return;
    parts.push(winAnsi ? concatBytes(new Uint8Array([0x28]), winAnsiBytes(line), new Uint8Array([0x29])) : editLiteral(line));
    text(" Tj ");
  });
  const ex = editNumber(paragraph.drift[0] - Number(tx));
  const ey = editNumber(paragraph.drift[1] - Number(ty) + (lines.length - 1) * Number(leadingText));
  if (ex !== "0" || ey !== "0") text(`${ex} ${ey} Td`);
  if (sized) text(` ${font} ${editNumber(oldTf)} Tf`);
  if (editNumber(first.leading!) !== editNumber(last.leading!)) text(` ${editNumber(last.leading!)} TL`);
  const elementIndex = first.cow?.form ? first.cow.form.elementIndex : Number(/^c(\d+)$/.exec(first.path)?.[1] ?? NaN);
  if (!Number.isInteger(elementIndex)) fail("not-candidate", "only top-level page content is editable");
  return {
    lines, noop, elementIndex,
    span: { record: first, start: first.operandStart, end: last.opEnd, replacement: concatBytes(...parts) },
  };
}

/** The one paragraph writer, called inside a mutate (privatises the element first when shared). */
export function editWriteParagraph(pdf: EditPdf, pageView: MupdfPage, plan: EditParagraphPlan): void {
  editWriteElement(pdf, pageView, plan.elementIndex, [], new Set(), [plan.span]);
}
