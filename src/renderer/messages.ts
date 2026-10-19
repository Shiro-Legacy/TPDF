/**
 * Tandem PDF's words to the person using it (UX audit X7/X8, docs/rust-and-ux-plan.md §3.2).
 * One catalogue for the renderer and the main process; the Rust UI is meant to reuse it.
 *
 * The rules every entry follows:
 * - Plain language. No internal operation names ("sign-place:"), no error tokens
 *   ("unsupported-encoding"), no byte counts or "MiB", no PDF jargon ("Identity-H",
 *   "incremental update", "copy-on-write", "show-text"), and never "refused".
 * - An error says what happened, then what to do next.
 * - Success and information are never shown as an error: each message has a kind,
 *   and only `error` is drawn in red.
 *
 * Adding a message: put it in the section of its feature, as a string or a function of
 * the values it names, and call it from the code (`MSG.save.saved(name)`). Text that
 * still comes from elsewhere (an engine refusal, a drive-only check) goes through
 * `userText`, which removes internal prefixes and jargon before it is shown.
 */
import { MAX_PDF_BYTES, MAX_SAVE_BYTES } from "./limits.js";

/** `error` is red; `warning` amber (something went wrong but was handled); `success` green; `info` neutral. */
export type MessageKind = "error" | "warning" | "success" | "info";

/** "printing, copying and filling forms". */
export function listing(items: string[]): string {
  return items.length < 2 ? items.join("") : `${items.slice(0, -1).join(", ")} and ${items[items.length - 1]}`;
}

/** A size limit as people read it: whole megabytes. */
export function megabytes(bytes: number): string {
  return `${Math.round(bytes / (1024 * 1024))} MB`;
}

const DESKTOP_ONLY = "This needs the Tandem PDF desktop app.";

export const MSG = {
  general: {
    desktopOnly: DESKTOP_ONLY,
    unexpected: "Something went wrong. Try again; if it keeps happening, save a copy of your work and reopen the file.",
    notPermitted: "This document's settings do not allow that change. You can still read, print or save a copy.",
    readOnly: "This document is read-only, so it cannot be changed. Save a copy to keep your own version.",
  },
  open: {
    tooLarge: (name: string, limit = MAX_PDF_BYTES) => `${name} is larger than ${megabytes(limit)}, the most Tandem PDF can open. Try a smaller file.`,
    unreadable: (name: string) => `${name} could not be opened. It may be damaged or not a PDF. Check the file and try again.`,
    notPdf: (name: string) => `${name} is not a PDF this app can read. Choose a PDF file.`,
    noDocument: "Open a PDF first.",
    sample: "The sample document is built into the app and cannot be saved. Open or create a PDF to save your work.",
  },
  save: {
    saved: (name: string) => `Saved ${name}.`,
    savedCopy: (name: string) => `Saved a copy as ${name}. The window still shows the original.`,
    savedAs: (name: string) => `Saved as ${name}. The window now edits ${name}.`,
    cancelled: "Save cancelled.",
    nothingToSave: "No changes to save.",
    running: "A save is already running. Wait for it to finish.",
    readOnly: "This document cannot be changed, so it cannot be saved over. Use Save As… to keep a copy.",
    /** After a restart, the save that was being written landed. */
    lateShown: (name: string) => `Saved ${name}. The window now shows the saved file.`,
    lateKept: (name: string) => `Saved ${name} from before the restart. The window holds newer changes: Save writes them over ${name}.`,
    /** Why a save did not happen; every reason main can answer with (tests/m10-save-failure.test.mjs). */
    reason: {
      "no-document": "There is no document open to save. Open or create a PDF first.",
      "read-only": "This document is signed or certified, so it cannot be saved in this version. Your file on disk is unchanged.",
      encrypted: "This document is password-protected, and saving a protected copy is not supported yet. Your file on disk is unchanged.",
      invalid: "The saved file did not read back correctly, so nothing was written. Your file on disk is unchanged; try again.",
      worker: "The PDF engine stopped responding, so nothing was written. Try again.",
      cancelled: "Save cancelled.",
      "too-large": `Tandem PDF can open documents up to ${megabytes(MAX_PDF_BYTES)}, but can save at most ${megabytes(MAX_SAVE_BYTES)}. Remove some pages or images and try again.`,
      "same-file": "That would overwrite the open document. Choose a different name.",
      "document-changed": "The open document changed while saving, so nothing was written. Save again.",
      "source-changed": "The file changed on disk since you opened it, so it was not overwritten. Use Save As… to save your version under another name.",
      "disk-full": "There is not enough disk space to save. The file on disk was not changed; free some space and try again.",
      permission: "The file or its folder is read-only, or Tandem PDF is not allowed to write there. The file on disk was not changed; use Save As… to save somewhere else.",
      unreadable: "The file could not be written. The file on disk was not changed; try again or save somewhere else.",
      "write-failed": "The file could not be written. The file on disk was not changed; try again or save somewhere else.",
      busy: "Another save is already running. Wait for it to finish.",
      "broken-link": "That name is a link that does not lead to a file, so nothing was written. Choose another name.",
    } as Record<string, string>,
    /** A new file (export, a PDF from images, a signed copy) could not be written. */
    newFile: (reason: string): string | undefined =>
      reason === "permission" ? "The folder or file is read-only, or Tandem PDF is not allowed to write there. Choose another folder."
        : reason === "disk-full" || reason === "broken-link" ? MSG.save.reason[reason] : undefined,
  },
  /** UX item 8: the first save of a filled form asks once where it goes. */
  filled: {
    question: (name: string) => `Save over “${name}” or as a new file?`,
    detail: "You filled in this form. Saving over it replaces the original; a new file keeps the original empty.",
    overwrite: "Save Over",
    newFile: "Save as New File…",
    cancel: "Cancel",
    /** The native question main shows: Return (the default) never overwrites the original; Esc cancels. */
    box: (name: string) => ({
      type: "question" as const, buttons: [MSG.filled.overwrite, MSG.filled.newFile, MSG.filled.cancel],
      defaultId: 1, cancelId: 2, message: MSG.filled.question(name), detail: MSG.filled.detail,
    }),
  },
  /** The notice under the toolbar that describes the open file (§3.3 Open). */
  banner: {
    locked: "This file is password-protected. Enter its password below to open it.",
    unlocked: (all: boolean, allowed: string[], weak: boolean) =>
      (all ? "This file is password-protected; it is unlocked and you can change it."
        : `This file is password-protected; its settings allow only ${allowed.length ? listing(allowed) : "reading"}.`)
      + (weak ? " It uses old, weak encryption: File ▸ Security… can protect it better." : ""),
    signedReadOnly: "This file is signed. Editing it would break the signature, so it opens for reading. Use Verify to check the signatures.",
    readOnly: "This file is read-only, so it opens for reading.",
    xfa: "This form is an Adobe dynamic form; its fields may not all show or fill here.",
    attachments: (count: number) => `It has ${count} attached file${count === 1 ? "" : "s"}.`,
    openAction: "It asks to run an action when opened; Tandem PDF never runs it.",
    repaired: "This file was damaged and has been repaired. Check it before saving.",
  },
  annotate: {
    noteEmpty: "Note discarded: it had no text.",
    noSelection: "Select some text on the page first, or search for it, then choose Highlight.",
    noMatch: "Search for the text first, then choose Highlight match.",
    highlightedAll: (count: number) => `Highlighted ${count} match${count === 1 ? "" : "es"}.`,
  },
  sign: {
    chooseCertificate: "Choose a certificate first.",
    needsDocument: "Open a PDF first, then sign it.",
    signedOpened: (name: string) => `Signed and saved as ${name}. The signed copy is now open.`,
    noDrawing: "Draw or choose your signature first.",
    rotated: "A signature cannot be placed on a rotated page. Rotate the page back first.",
    /** Why a certificate could not be used. */
    certificate: (reason: string, status?: string) => status ? `This certificate cannot be used: ${status}. Choose another certificate.`
      : ({
        "bad-password": "The certificate password is incorrect. Type it again.",
        "unsupported-key": "This certificate's key type is not supported (an RSA key of 2048 bits or more is needed). Choose another certificate.",
        "certificate-expired": "This certificate is not valid today. Choose another certificate.",
        "no-certificate": "Choose a certificate first.",
        unreadable: "That certificate file could not be read. Check that it is a .p12, .pfx or .pem file and that the password is right.",
        busy: "Another save is running. Wait for it to finish, then sign again.",
      } as Record<string, string>)[reason] ?? "This certificate cannot be used. Choose another certificate.",
    /** Why writing the signed copy failed. */
    failed: (reason: string, status?: string): string => (reason === "certificate-expired" || reason === "unsupported-key" || reason === "bad-password")
      ? MSG.sign.certificate(reason, status)
      : ({
      cancelled: "Signing cancelled: no signed copy was written.",
      invalid: "The signature could not be made: the signing time or details look wrong. Check this computer's date and time, then sign again.",
      unreadable: "The signature could not be made with this certificate. Choose the certificate again, enter its password, and sign again.",
      "too-large": "This document is too large to sign in this version.",
      "too-many": "This document has too many signatures to sign again in this version.",
      busy: "Another save is running. Wait for it to finish, then sign again.",
      "no-certificate": "Choose a certificate and enter its password again, then sign.",
    } as Record<string, string>)[reason] ?? (MSG.save.newFile(reason) ?? "The signed copy could not be written. Try again or choose another folder."),
    invalidInput: "Enter a page that exists in this document, and a reason of at most 200 characters.",
    securityPending: "You changed this document's protection (Security…) and have not saved it yet. Save the document first, then sign it.",
    preparing: "Signing…",
  },
  /** The Verify signatures dialog (§3.3 Sign: "Verify speaks plainly"). */
  verify: {
    certified: (level: number) => level === 1 ? "Certified: the author allows no changes."
      : level === 2 ? "Certified: the author allows filling in the form and signing."
        : level === 3 ? "Certified: the author allows filling in the form, signing and comments."
          : "Certified by its author.",
    partial: "Part of the file: changes made after this signature were not checked",
    whole: "The whole file",
    /** The reason notes a verdict carries, space-separated tokens, as words. */
    notes: (tokens: string): string => tokens.split(" ").filter(Boolean).map(token => ({
      "no-signing-time": "no signing time given",
      "weak-digest": "uses old SHA-1 checking",
      "unknown-digest": "uses an unknown checking method",
      "changes-not-permitted": "the file changed in ways its certification does not allow",
      "missing-subfilter": "the signature format is not stated",
      "byte-range": "the signed part of the file is not what it claims",
      digest: "the document changed after it was signed",
      signature: "the signature does not match the document",
      "signer-cert": "the signer's certificate is not valid",
      malformed: "the signature cannot be read",
      "no-signed-attrs": "the signature format is not supported",
      "signer-id": "the signer cannot be identified",
    } as Record<string, string>)[token] ?? token.replaceAll("-", " ")).join("; "),
  },
  main: {
    discard: { message: "Discard unsaved changes?", detail: "Your changes since the last save will be lost.", buttons: ["Discard Changes", "Cancel"] },
    openLink: (uri: string) => `Open ${uri} in your browser?`,
  },
};

/** Exact internal texts with a plain replacement (engine and worker refusals that reach the error bar). */
const EXACT: Record<string, string> = {
  "That edit was refused.": "That change could not be made. Undo or try a different change.",
  "Draw a bigger link": "Drag a bigger rectangle to make a link.",
  "Draw a bigger signature field": "Drag a bigger rectangle to make a signature field.",
};

/**
 * Internal words, and what people call them. Applied only to internal text (an engine or drive message,
 * recognised by its operation prefix): never to catalogue text or to the file names, quotes and signer
 * names a message carries.
 */
const WORDS: [RegExp, string | ((...match: string[]) => string)][] = [
  [/\b(\d{4,})[- ]?bytes?\b/g, (_all, n) => megabytes(Number(n))],
  [/\b(\d+(?:\.\d+)?) MiB\b/g, "$1 MB"],
  [/\bceiling\b/g, "limit"],
  [/\bwas refused\b/g, "could not be done"],
  [/\bis refused\b/g, "is not possible"],
  [/\bare refused\b/g, "are not possible"],
  [/\brefused\b/g, "not possible"],
  [/\bIdentity-H\b/g, "embedded"],
  [/\bcopy-on-write\b/g, "a private copy"],
  [/\bincremental (?:update|save)\b/g, "signed save"],
  [/\bshow-text runs?\b/g, "lines of text"],
];

/** The operation names engine and drive messages start with ("sign-place: …", "fill-text: …"). */
const INTERNAL_OPS = new Set(("annot annot-add annot-delete annot-move annot-set annots arm assemble authenticate batch batch-file bookmark "
  + "cert-sign-open circle click clip-copy clip-paste compare compare-close compare-open compare-render compare-text content content-edit "
  + "crop decorate delete-page drag edit-batch edit-object edit-objects edit-paragraph edit-replace edit-replace-apply edit-replace-preview "
  + "edit-targets edit-text encrypt encrypt-keep export export-jpeg export-png export-txt extract extract-pages field-add field-delete "
  + "field-props field-set fields fill-text find freetext goto handle header-footer highlight highlight-all ink insert insert-pages key "
  + "labels labels-clear labels-set layer-set layers line link link-add link-delete link-set links menu mode move-down move-up nav "
  + "new-from-images open outline outline-add outline-delete outline-set outlines page-delete page-ids page-move page-retry page-rotate "
  + "pages pdf-from-images redact redact-add redact-apply redact-preview redact-set redo render rotate-left rotate-right save saved "
  + "scan-compose scan-measure search security security-set select shapes sign sign-draw sign-list sign-place sign-prepare square stamp "
  + "strikeout tab text-add text-all thumbnail underline undo validate verify watermark zoom").split(" "));
/** The error tokens a refusal can end with in parentheses ("… (unsupported-transform)"). */
const TOKEN_SUFFIX = / \((unsupported-[a-z-]+|subset-font|stale-target|not-candidate|ambiguous-text-association|tail-would-move|shared-or-unproven-sharing|rtl-or-shaping|vertical-writing|invalid-[a-z-]+|write-failed|read-only|out-of-bounds|unresolved-resource|too-large)\)(?=\.?$)/;

/**
 * The text a message shows. Catalogue text, and anything a person wrote or named, passes through unchanged.
 * Internal text (it starts with an operation name) loses that prefix, and its byte counts and jargon are put
 * into plain words; a known error token at the end is dropped. Never empty.
 */
export function userText(raw: string): string {
  let text = (raw ?? "").trim();
  if (!text) return "";
  if (Object.hasOwn(EXACT, text)) return EXACT[text];
  // An engine message can arrive wrapped by a drive op ("content: edit-text: …"): every leading prefix goes.
  let internal = false;
  for (let prefix = /^([a-z][a-z0-9]*(?:-[a-z0-9]+)*): (?=\S)/.exec(text); prefix && INTERNAL_OPS.has(prefix[1]); prefix = /^([a-z][a-z0-9]*(?:-[a-z0-9]+)*): (?=\S)/.exec(text)) {
    internal = true;
    text = text.slice(prefix[0].length);
  }
  if (internal) for (const [pattern, replacement] of WORDS) text = text.replace(pattern, replacement as string);
  text = text.replace(TOKEN_SUFFIX, "").replace(/ ?\(window\.pdf4all is unavailable\)/g, "").replace(/\s{2,}/g, " ").trim();
  if (!text) return MSG.general.unexpected;
  return internal ? text[0].toUpperCase() + text.slice(1) : text;
}
