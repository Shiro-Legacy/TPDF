/**
 * Module-level engine core: the `/P` bit table and the refusal/coercion helpers
 * the Rust adapter (engine-rust.ts), the worker loader, the tests (OP_PERMISSION)
 * and the MuPDF test oracle (engine/oracle/app-engine) share. Moved byte-for-byte
 * out of the former `engine.ts` (A1 step 1); the readers that take MuPDF objects
 * (`algorithmOf`, `permissionsOf`, `permissionBitsOf`, `at`) moved on to
 * engine/oracle/app-engine/mupdf-core.ts. Only types come from `./protocol.js`.
 */
import type { DocumentInfo, DocumentPermissions, EditObjectDetail, EditReplaceDetail, EditTextDetail, FieldDetail, SaveReason } from "./protocol.js";

/**
 * The `/P` bits this app can set when it writes AES-256 (PDF 32000-1 table 22).
 * mupdf keeps the reserved bits itself, so the value handed to `permissions=` is
 * the sum of the granted flags only.
 */
export const PERMISSION_BITS: [keyof DocumentPermissions, number][] = [
  ["print", 4], ["edit", 8], ["copy", 16], ["annotate", 32], ["form", 256],
];

/** Granted flag names, in the fixed order the smoke logs them. */
export function permissionNames(permissions: DocumentPermissions): (keyof DocumentPermissions)[] {
  return PERMISSION_BITS.map(([name]) => name).filter(name => permissions[name]);
}

/** The `/P` value to write for a set of flags; mupdf keeps the reserved bits itself. */
export function permissionInt(flags: DocumentPermissions): number {
  return PERMISSION_BITS.reduce((sum, [name, bit]) => sum + (flags[name] ? bit : 0), 0);
}

/** Two decimals is well under a point of rounding for a text matrix. */
export function round2(value: number): number {
  return Math.round(value * 100) / 100;
}

/** The pieces of one content stream, concatenated once. */
export function concatBytes(...parts: Uint8Array[]): Uint8Array {
  let length = 0;
  for (const part of parts) length += part.byteLength;
  const out = new Uint8Array(length);
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.byteLength;
  }
  return out;
}

export function attempt<T>(read: () => T, fallback: T): T {
  try {
    return read();
  } catch {
    return fallback;
  }
}

/** Throws an Error tagged with an `edit-text` refusal token; `dispatch` copies it into the reply. */
export function failEditText(detail: EditTextDetail, message: string, extra?: Record<string, unknown>): never {
  const error = new Error(message) as Error & { reason?: SaveReason; detail?: EditTextDetail; extra?: Record<string, unknown> };
  error.reason = "invalid";
  error.detail = detail;
  if (extra) error.extra = extra;
  throw error;
}

/** Throws an Error tagged with an `edit-object` refusal token; `dispatch` copies it into the reply. */
export function failEditObject(detail: EditObjectDetail, message: string, extra?: Record<string, unknown>): never {
  const error = new Error(message) as Error & { reason?: SaveReason; detail?: EditObjectDetail; extra?: Record<string, unknown> };
  error.reason = "invalid";
  error.detail = detail;
  if (extra) error.extra = extra;
  throw error;
}

/** Throws an Error tagged with an `edit-replace-apply` refusal token. */
export function failEditReplace(detail: EditReplaceDetail, message: string, extra?: Record<string, unknown>): never {
  const error = new Error(message) as Error & { reason?: SaveReason; detail?: EditReplaceDetail; extra?: Record<string, unknown> };
  error.reason = "invalid";
  error.detail = detail;
  if (extra) error.extra = extra;
  throw error;
}

/** Throws an Error tagged with a U6 field-designer refusal token (contract §2.1). */
export function failField(detail: FieldDetail, message: string): never {
  const error = new Error(message) as Error & { reason?: SaveReason; detail?: FieldDetail };
  error.reason = "invalid";
  error.detail = detail;
  throw error;
}

/** P6 U3 §9.4: a `redact-set` refusal (reason `invalid`), tagged with what was wrong. */
export function failRedact(detail: "rect" | "no-such-annot" | "not-redact", message: string): never {
  const error = new Error(message) as Error & { reason?: SaveReason; detail?: string };
  error.reason = "invalid";
  error.detail = detail;
  throw error;
}

/** P6 U4 §9.3: a `link-set` refusal (reason `invalid`), tagged with what was wrong. */
export function failLink(detail: "rect" | "no-such-annot" | "not-link" | "target" | "scheme", message: string): never {
  const error = new Error(message) as Error & { reason?: SaveReason; detail?: string };
  error.reason = "invalid";
  error.detail = detail;
  throw error;
}

/** Throws an Error tagged with a worker-save reason; `dispatch` copies it into the reply. */
export function failReason(reason: SaveReason, message: string): never {
  const error = new Error(message) as Error & { reason: SaveReason };
  error.reason = reason;
  throw error;
}

export function errorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export function copyBytes(src: Uint8Array): ArrayBuffer {
  const copy = new Uint8Array(src.byteLength);
  copy.set(src);
  return copy.buffer as ArrayBuffer;
}

export function num(value: unknown, fallback = 0): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

export function str(value: unknown, fallback: string): string {
  return typeof value === "string" && value ? value : fallback;
}

/**
 * The long edge one image page may take. Past this the page is scaled down (the
 * image itself keeps its pixels); the same 14400 pt ceiling mupdf's own writers use.
 */
export const MAX_IMAGE_PAGE_POINTS = 14400;

/** WinAnsi code points whose byte differs from Unicode (0x80–0x9f). */
const WIN_ANSI_EXTRA: Record<number, number> = {
  0x20ac: 0x80, 0x201a: 0x82, 0x0192: 0x83, 0x201e: 0x84, 0x2026: 0x85, 0x2020: 0x86,
  0x2021: 0x87, 0x02c6: 0x88, 0x2030: 0x89, 0x0160: 0x8a, 0x2039: 0x8b, 0x0152: 0x8c,
  0x017d: 0x8e, 0x2018: 0x91, 0x2019: 0x92, 0x201c: 0x93, 0x201d: 0x94, 0x2022: 0x95,
  0x2013: 0x96, 0x2014: 0x97, 0x02dc: 0x98, 0x2122: 0x99, 0x0161: 0x9a, 0x203a: 0x9b,
  0x0153: 0x9c, 0x017e: 0x9e, 0x0178: 0x9f,
};

export function winAnsiByte(code: number): number {
  if (code >= 0x20 && code <= 0x7e) return code;
  if (code >= 0xa0 && code <= 0xff) return code;
  return WIN_ANSI_EXTRA[code] ?? -1;
}

export function winAnsiChar(byte: number): string | null {
  if ((byte >= 0x20 && byte <= 0x7e) || (byte >= 0xa0 && byte <= 0xff)) return String.fromCodePoint(byte);
  const entry = Object.entries(WIN_ANSI_EXTRA).find(([, value]) => value === byte);
  return entry ? String.fromCodePoint(Number(entry[0])) : null;
}

export function checkStampText(text: unknown, op: string, max: number): string {
  if (typeof text !== "string" || text.length < 1 || text.length > max) {
    throw new Error(`${op}: text must be 1..${max} characters`);
  }
  for (const character of text) {
    const code = character.codePointAt(0) ?? 0;
    if (winAnsiByte(code) < 0) {
      throw new Error(`${op}: ${JSON.stringify(character)} (U+${code.toString(16).toUpperCase()}) is not a WinAnsi character`);
    }
  }
  return text;
}

/** The twelve standard-14 text fonts `text-add` writes (Symbol/ZapfDingbats are not WinAnsi). */
export const BASE14_FONTS = ["Helvetica", "Helvetica-Bold", "Helvetica-Oblique", "Helvetica-BoldOblique", "Times-Roman", "Times-Bold", "Times-Italic", "Times-BoldItalic", "Courier", "Courier-Bold", "Courier-Oblique", "Courier-BoldOblique"];

/** WinAnsi bytes escaped for a PDF literal string; callers validate before writing. */
export function winAnsiBytes(text: string): Uint8Array {
  const bytes: number[] = [];
  for (const character of text) {
    const byte = winAnsiByte(character.codePointAt(0) ?? 0);
    const value = byte < 0 ? 0x3f : byte;
    if (value === 0x28 || value === 0x29 || value === 0x5c) bytes.push(0x5c);
    bytes.push(value);
  }
  return new Uint8Array(bytes);
}

/**
 * Which permission each mutation needs. Annotations and links are `/P` bit 6
 * (`annotate`), page and structure work bit 4 (`edit`), field fill bit 9 (`form`).
 * Undo/redo needs any of the three: the journal only holds operations the
 * session was already allowed to make.
 */
export const OP_PERMISSION: Record<string, keyof DocumentPermissions | "any" | "edit+annotate"> = {
  "field-set": "form",
  "text-add": "edit",
  "field-add": "edit+annotate", "field-props": "edit+annotate", "field-delete": "edit+annotate",
  "annot-add": "annotate", "annot-set": "annotate", "annot-move": "annotate", "annot-delete": "annotate",
  "redact-add": "annotate", "redact-search": "annotate", "redact-set": "annotate", "redact-apply": "edit",
  "link-add": "annotate", "link-delete": "annotate", "link-set": "annotate",
  "outline-add": "edit", "outline-set": "edit", "outline-delete": "edit",
  "insert-pages": "edit", "stamp": "edit", "labels-set": "edit", "labels-clear": "edit",
  "page-rotate": "edit", "page-delete": "edit", "page-move": "edit",
  // P6 U7 §0/§10: a copy writes nothing (the copy bit decides); paste and batch check per item.
  "clip-copy": "copy", "clip-paste": "any", "edit-batch": "any",
  undo: "any", redo: "any",
};

/** Owner authentication makes mupdf report all five, so `/P` alone decides here. */
export function allowed(info: DocumentInfo, need: keyof DocumentPermissions | "any" | "edit+annotate"): boolean {
  if (!info.encrypted) return true;
  if (need === "any") return info.permissions.annotate || info.permissions.edit || info.permissions.form;
  // U6 §7.2: authoring writes the field tree (bit 4) *and* the page `/Annots` (bit 6).
  if (need === "edit+annotate") return info.permissions.edit && info.permissions.annotate;
  return info.permissions[need];
}
