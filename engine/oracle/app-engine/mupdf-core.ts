/**
 * The engine-core readers that take MuPDF objects, moved out of
 * web/engine-core.ts unchanged when the TypeScript MuPDF engine became
 * a test-only oracle (engine/oracle, AGPL-3.0-or-later; see its README).
 */
import { attempt } from "../../../web/engine-core.js";
import type { DocumentAlgorithm, DocumentPermissions } from "../../../web/protocol.js";
import type { MupdfDocument, MupdfObject } from "./engine.js";

// PDF dict helpers: a Null PDFObject throws if you call get() on it, so never chain.
/**
 * The standard security handler named by `/Encrypt`, readable while the document is
 * still locked (none of /V /R /Length /CFM is itself encrypted). `unknown` is reserved
 * for a dict this build cannot classify; the caller treats it as RC4-era for honesty.
 */
export function algorithmOf(trailer: MupdfObject | null): DocumentAlgorithm {
  const enc = at(trailer, "Encrypt");
  if (!enc) return "none";
  const number = (key: string) => attempt(() => (enc.get(key) as MupdfObject).asNumber(), 0);
  const V = number("V");
  const R = number("R");
  const length = number("Length");
  if (V >= 5) return "aes-256";
  if (V === 4) {
    const cf = at(at(enc, "CF"), "StdCF");
    const cfm = cf ? attempt(() => String(cf.get("CFM")), "") : "";
    return cfm.includes("AESV3") ? "aes-256" : "aes-128";
  }
  if (R <= 2 || V <= 1) return "rc4-40";
  return length === 40 ? "rc4-40" : "rc4-128";
}

/** The five permissions mupdf reports; an unencrypted document reports all of them. */
export function permissionsOf(doc: MupdfDocument): DocumentPermissions {
  const read = (permission: "print" | "copy" | "edit" | "annotate" | "form") =>
    attempt(() => doc.hasPermission(permission), true);
  return { print: read("print"), copy: read("copy"), edit: read("edit"), annotate: read("annotate"), form: read("form") };
}

/**
 * The `/P` value as an unsigned 32-bit int, read straight off `/Encrypt`. This is
 * what a non-mupdf reader sees, and unlike `hasPermission` it does not change with
 * who authenticated: mupdf reports all five to a session that matches the owner
 * password, including when the owner and user password are the same string.
 */
export function permissionBitsOf(doc: MupdfDocument): number {
  const enc = at(doc.asPDF()?.getTrailer() ?? null, "Encrypt");
  const raw = enc ? attempt(() => Number(String(enc.get("P"))), NaN) : NaN;
  return Number.isFinite(raw) ? raw >>> 0 : -1;
}

export function at(object: MupdfObject | null, key: string): MupdfObject | null {
  try {
    if (!object || object.isNull()) return null;
    const value = object.get(key) as MupdfObject;
    return value && !value.isNull() ? value : null;
  } catch {
    return null;
  }
}
