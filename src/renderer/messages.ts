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
import { MAX_PDF_BYTES, MAX_REDACT_SOURCE_BYTES, MAX_SAVE_BYTES } from "./limits.js";
import type { RedactSanitizeKey } from "./protocol.js";

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


// ------------------------------------------------------------------ redaction (docs/rust-and-ux-plan.md §2.3)
// The redaction tool's words (redact-ui.ts re-exports them under their old names; main's close prompt uses them too).

/** The nine hidden-information switches, in plain words (they sit under "Details"). */
const SANITIZE_LABELS: Record<RedactSanitizeKey, string> = {
  // The same plain names as the mobile app (owner decision), so both apps read alike.
  info: "Author, title and other document properties",
  xmp: "Extra document details saved by other apps",
  attachments: "Attached files",
  outlines: "Bookmarks",
  annots: "Other comments and notes, on every page",
  links: "Web and file addresses in links",
  fields: "Everything typed into form fields",
  layers: "Layer names",
  struct: "Descriptions written for screen readers",
};

const plural = (count: number, one: string, many = `${one}s`) => `${count} ${count === 1 ? one : many}`;

const REDACT_CHIP = "Drag over anything you want removed. Esc to stop.";
function areasText(count: number): string { return plural(count, "area"); }
function boxesText(count: number): string { return plural(count, "box", "boxes"); }
function panelHeading(count: number): string { return `Redactions (${count})`; }
function redactButtonText(count: number): string { return count > 0 ? `Redact ${areasText(count)}…` : "Redact areas…"; }
function countText(boxes: number, pages: number): string {
  return boxes > 0 ? `${boxesText(boxes)} on ${plural(pages, "page")}` : "No boxes yet";
}
function confirmHeading(count: number): string { return `Permanently remove everything inside ${boxesText(count)}?`; }
const CONFIRM_TEXT = "Text, pictures, drawings, comments and form fields inside the boxes are deleted from the file, not just covered. The result is saved as a new file; your original is not changed.";
function bannerText(name: string, count: number, extra: { flattened?: number[]; unsigned?: boolean } = {}): string {
  const flat = extra.flattened?.length ? ` ${pagesName(extra.flattened, true)} ${extra.flattened.length === 1 ? "was" : "were"} turned into ${extra.flattened.length === 1 ? "an image" : "images"}.` : "";
  const unsigned = extra.unsigned ? " The copy has no signatures." : "";
  return `You are viewing the redacted copy '${name}'. ${areasText(count)} removed and checked.${flat}${unsigned}`;
}
/** "page 4" / "pages 2, 5 and 7" from 0-based pages; `capital` starts it with "Page". */
function pagesName(pages: number[], capital = false): string {
  const list = pages.map(page => page + 1);
  const name = list.length === 1 ? `page ${list[0]}` : `pages ${list.slice(0, -1).join(", ")} and ${list.at(-1)}`;
  return capital ? `${name[0].toUpperCase()}${name.slice(1)}` : name;
}
/** R4: a box on the shown page covers artwork that can only go by turning the page into an image. */
function flattenNote(page: number): string {
  return `A box covers artwork that can't be cut precisely: redacting will turn page ${page + 1} into an image.`;
}
function pendingText(count: number): string {
  return `${areasText(count)} ${count === 1 ? "is" : "are"} marked but not redacted yet.`;
}
/**
 * §2.3 step 7, asked by main before the window closes or another file replaces it while boxes are
 * unsaved: `memory` boxes (a signed file's) live only while the file is open. Buttons: [redact, go on, cancel].
 */
function pendingQuestion(count: number, memory: boolean, action: "close" | "open"): { message: string; detail: string; buttons: [string, string, string] } {
  const verb = action === "close" ? "Closing" : "Opening another file";
  return {
    message: pendingText(count),
    detail: memory
      ? `These boxes are kept only while the file is open. ${verb} now discards them. Nothing has been removed from the file.`
      : `${verb} now discards the boxes and any other unsaved changes. Nothing has been removed from the file.`,
    buttons: ["Redact Now…", action === "close" ? "Close Without Redacting" : "Discard the Boxes", "Cancel"],
  };
}
const SAME_FILE_QUESTION = {
  heading: "That would replace your original.",
  text: "The redacted copy must be a new file. Choose a different name.",
  yes: "Choose another name…",
};
const OTHERS_QUESTION = {
  heading: "You have other unsaved changes.",
  text: "They will be in the redacted copy, but not in your original.",
};
const PENDING_DETAIL = "Saving now keeps the boxes as marks. Everything under them stays in the file until you redact it.";
/** §2.4: the size ceiling in decimal MB, rounded down so the stated size always works. */
function sizeLimitText(bytes = MAX_REDACT_SOURCE_BYTES): string {
  const mb = Math.floor(bytes / 1e5) / 10;
  return `Redaction works on files up to ${mb} MB in this version.`;
}
function flattenQuestion(pages: number[]): { heading: string; text: string; yes: string } {
  const name = pagesName(pages), one = pages.length === 1;
  return {
    heading: `${pagesName(pages, true)} ${one ? "contains" : "contain"} artwork that can't be cut precisely.`,
    text: `Redact ${name} by turning ${one ? "it" : "them"} into an image? ${one ? "Its" : "Their"} text will no longer be selectable.`,
    yes: `Flatten ${name}`,
  };
}
const SIGNED_QUESTION = {
  heading: "Redacting removes the signatures.",
  text: "Make an unsigned redacted copy? The original keeps its signatures.",
  yes: "Make unsigned copy",
};
/**
 * A refused apply in plain words; `undefined` keeps the engine's own sentence. `pages` are 0-based.
 * `hiddenKept`: some hidden information was kept, so a failed raw check may be text left in comments,
 * form fields or properties (R5 decision).
 */
function refusalText(reason: string | undefined, pages: number[] = [], detail?: string, hiddenKept = false): string | undefined {
  if (reason === "too-large") return `${sizeLimitText()} This file is larger.`;
  if (reason === "read-only") return "This document can't be changed, so it can't be redacted.";
  // R3 copies shared pictures and drawings; this refusal is only the engine's safety net now.
  if (reason === "shared-content") return "A picture or drawing in a box could not be redacted safely, so nothing was saved. Try redacting the whole page.";
  if (reason === "no-match") return "That text was not found in this document.";
  if (reason === "verify-failed" && detail === "raw" && hiddenKept) {
    return "Some of the text inside your boxes is still in the document's comments, form fields or properties, or elsewhere on the page, so nothing was saved. Turn on \"Also remove hidden information, other comments and form entries\" and try again; if it still fails, use a larger box or redact the whole page.";
  }
  if (reason === "verify-failed") {
    const where = pages.length ? ` on ${pagesName(pages)}` : "";
    return `The redacted copy did not pass its final check${where}, so nothing was saved. Try a larger box, or redact the whole page.`;
  }
  if (reason === "needs-flatten") return "A box covers artwork that can't be cut precisely. Redact again and agree to turn that page into an image.";
  if (reason === "signed") return "Redacting removes the signatures. Redact again and agree to make an unsigned copy.";
  if (reason === "scan-budget") return "This page is too complex to check box by box. Redact the whole page instead.";
  if (reason === "too-many") return "There are too many boxes (2000 at most). Use one larger box or \"Redact whole page\".";
  return undefined;
}

/** The pending-boxes question's words for each way out ([Redact now…] stays first and default). */
function pendingWords(action: "save" | "export" | "print" | "sign", memory: boolean): { detail: string; go: string } {
  if (action === "export") return { detail: "An exported page or its text still shows everything under the boxes until you redact.", go: "Export anyway" };
  if (action === "print") return { detail: "A printed page still shows everything under the boxes until you redact.", go: "Print anyway" };
  if (action === "sign") return { detail: "The signed copy keeps the boxes as marks, with everything under them, until you redact.", go: "Sign anyway" };
  return memory
    ? { detail: "These boxes are kept only while the file is open; a saved copy will not have them.", go: "Save without the boxes" }
    : { detail: PENDING_DETAIL, go: "Save marks only" };
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
        // Phase 2 (the signing worker, crates/tandem-sign): one plain sentence per reason a certificate is refused.
        "bad-password": "The certificate password is incorrect. Type it again.",
        "unsupported-key": "This certificate's key cannot sign here. Use an RSA key of 2048 bits or more, or a P-256 or P-384 key.",
        "certificate-expired": "This certificate is not valid today. Choose another certificate.",
        "no-certificate": "Choose your certificate file again, then sign.",
        unreadable: "This file could not be read as a certificate. Choose a .p12, .pfx or .pem file that holds your key and certificate.",
        "too-costly": "This certificate file is protected in a way that takes too long to open. Export it again with standard settings.",
        cancelled: "Not signed.",
        busy: "Another save is running. Wait for it to finish, then sign again.",
      } as Record<string, string>)[reason] ?? "This certificate cannot be used. Choose another certificate.",
    /** Why writing the signed copy failed. */
    failed: (reason: string, status?: string): string => (reason === "certificate-expired" || reason === "unsupported-key" || reason === "bad-password" || reason === "too-costly")
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
    /** Phase 2: main asks before signing with a certificate whose key usage excludes document signing. */
    notForSigning: {
      type: "warning" as const, buttons: ["Sign anyway", "Cancel"], defaultId: 1, cancelId: 1,
      message: "This certificate isn't meant for signing documents, so other apps may not trust the signature. Sign anyway?",
    },
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
    /** The Trust cell; `issuer` is the verifier's cleaned issuer name. */
    trust: (trust: string, issuer?: string) => trust === "expired" ? "certificate not valid at the time it claims to have signed"
      : trust === "trusted" ? `trusted by ${issuer || "a certificate you trust"}`
        : "not in your trusted list",
    /**
     * The reason notes a verdict carries, space-separated tokens, as words. `words` is the verifier's own plain
     * phrase for them (Phase 2, crates/tandem-sign report.rs `reason_words`); it is used when given.
     */
    notes: (tokens: string, words?: unknown): string => typeof words === "string" && words ? words : tokens.split(" ").filter(Boolean).map(token => ({
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
      "time-changed": "the date was edited after signing",
      "outdated-security": "the certificate uses outdated security",
      "not-for-signing": "the certificate is not meant for signing",
      "unsupported-ca-rule": "the issuer's rules were not checked",
      "content-type": "not a document signature",
    } as Record<string, string>)[token] ?? token.replaceAll("-", " ")).join("; "),
  },
  /** The redaction tool (§2.3): its dialogs and panel (above), and what the status line and notices say. */
  redact: {
    SANITIZE_LABELS, REDACT_CHIP, areasText, boxesText, panelHeading, redactButtonText, countText, confirmHeading, CONFIRM_TEXT, bannerText, pagesName, flattenNote, pendingText, pendingQuestion, SAME_FILE_QUESTION, OTHERS_QUESTION, PENDING_DETAIL, sizeLimitText, flattenQuestion, SIGNED_QUESTION, refusalText, pendingWords,
    added: "Added a box. Nothing is removed until you redact.",
    addedPage: (page: number) => `Added a box over all of page ${page}. Nothing is removed until you redact.`,
    addedSearch: (added: number, text: string) => `Added ${added === 1 ? "1 box" : `${added} boxes`} over “${text}”. Text inside pictures, form fields and comments is not found: look over each page, or draw boxes.`,
    removedBox: "Removed a redaction box",
    drawFirst: "Draw a box first: drag over anything you want removed.",
    drawHint: "Drag a box over anything you want removed",
    stopped: "Stopped redacting",
    savedKept: (name: string) => `Saved the redacted copy ${name}. This window still shows the original with its boxes.`,
    savedOpened: (name: string) => `Saved and opened the redacted copy ${name}.`,
    saved: (name: string) => `Saved the redacted copy ${name}.`,
    sameFile: `${SAME_FILE_QUESTION.heading} ${SAME_FILE_QUESTION.text} Nothing was saved.`,
    notSaved: (reason: string) => `The redacted copy was not saved. ${MSG.save.newFile(reason) ?? "Try again, or choose another folder."}`,
    originalNotSaved: "Not redacted: the original was not saved.",
    originalUnreadable: "The original file could not be opened. It may have been moved or deleted.",
    /** The pending-boxes question was answered Cancel. */
    cancelled: (action: "save" | "export" | "print" | "sign") => `${action === "export" ? "Export" : action === "print" ? "Print" : action === "sign" ? "Signing" : "Save"} cancelled`,
  },
  main: {
    discard: { message: "Discard unsaved changes?", detail: "Your changes since the last save will be lost.", buttons: ["Discard Changes", "Cancel"] },
    openLink: (uri: string) => `Open ${uri} in your browser?`,
  },
};

/**
 * The texts the catalogue has produced (most recent first, at most 256): userText never rewrites one, even when
 * a value it carries (a file named "open: x.pdf") looks like an internal prefix (re-review fix 5).
 */
const produced = new Set<string>();
/** The catalogue's fixed texts: never evicted. */
const fixedTexts = new Set<string>();
function remember(text: string): string {
  produced.delete(text);
  produced.add(text);
  if (produced.size > 256) produced.delete(produced.values().next().value as string);
  return text;
}
/** Whether userText leaves `text` alone because the catalogue produced it. */
export function isCatalogueText(text: string): boolean {
  return fixedTexts.has(text) || produced.has(text);
}
(function track(node: Record<string, unknown>): void {
  for (const [key, value] of Object.entries(node)) {
    if (typeof value === "string") fixedTexts.add(value);
    else if (typeof value === "function") {
      const fn = value as (...args: unknown[]) => unknown;
      node[key] = (...args: unknown[]) => { const out = fn(...args); return typeof out === "string" ? remember(out) : out; };
    } else if (value && typeof value === "object" && !Array.isArray(value)) track(value as Record<string, unknown>);
  }
})(MSG as unknown as Record<string, unknown>);

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
  // A byte count of a megabyte or more is a size limit worth saying in MB; smaller counts stay as they are.
  [/\b(\d+)([- ]?)(bytes?)\b/g, (all, n) => (Number(n) >= 1024 * 1024 ? megabytes(Number(n)) : all)],
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
  if (fixedTexts.has(text) || produced.has(text)) return text; // catalogue text is already plain
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
