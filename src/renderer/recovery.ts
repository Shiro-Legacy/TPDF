/**
 * M9 "recovery for long operations" (engine/design/app-m9-recovery.md). The worker owns the
 * open document, so a worker the renderer has to stop (the 20 s watchdog, a long job past
 * 120 s, a WASM trap) used to take every unsaved edit with it. The renderer now keeps, in its
 * own memory only:
 * - the bytes the document was opened from (the replay base),
 * - every request since that open that changed the worker's session (`OPS` kind `mutate`,
 *   `unlock` or `clipboard`), in the order the worker answered them, with a fingerprint of the
 *   answer, and
 * - the bytes of the last acknowledged save in place (the fallback).
 * After a restart, `replay` reopens the base in the fresh worker and sends the log again; every
 * answer must match its fingerprint, so ids, revisions and undo history are those the UI holds.
 *
 * No engine code is loaded here (types only).
 */
import type { SessionRequest } from "./protocol.js";

export type Op = SessionRequest["op"];

/**
 * How a request touches the worker's session:
 * - `base`: replaces the open document (`open`, `close`); the log restarts there.
 * - `mutate`: changes the document, its journal, its pending security or its saved baseline; logged and replayed.
 * - `unlock`: a successful `authenticate`; logged (with the password, in memory only) and replayed.
 * - `clipboard`: `clip-copy` fills the worker clipboard a later `clip-paste` reads; logged and replayed.
 * - `read`: answers without changing the session; never logged (saves, exports and renders included).
 * - `compare`: the view-only compare slot; not logged, and lost on a restart.
 */
export type OpKind = "base" | "mutate" | "unlock" | "clipboard" | "read" | "compare";

/**
 * Every `SessionRequest` op, classified. The `Record` over the union makes tsc refuse a new op
 * that is not listed here, and `tests/m9-recovery.test.mjs` checks the table against the union
 * in engine.ts. `label` names the request in the restart notice; `long` marks the ops the app
 * runs with the 120 s limit, which a replay keeps.
 */
export const OPS: Record<Op, { kind: OpKind; label: string; long?: true }> = {
  open: { kind: "base", label: "Opening the document" },
  close: { kind: "base", label: "Closing the document" },
  authenticate: { kind: "unlock", label: "Unlocking the document" },
  render: { kind: "read", label: "Showing the page" },
  thumbnail: { kind: "read", label: "Drawing a page thumbnail" },
  search: { kind: "read", label: "Searching" },
  save: { kind: "read", label: "Saving", long: true },
  validate: { kind: "read", label: "Checking the saved file", long: true },
  saved: { kind: "mutate", label: "Recording the save" },
  "security-set": { kind: "mutate", label: "Changing the security settings" },
  annots: { kind: "read", label: "Reading the annotations" },
  "annot-add": { kind: "mutate", label: "Adding the annotation" },
  "annot-set": { kind: "mutate", label: "Editing the annotation" },
  "annot-move": { kind: "mutate", label: "Moving the annotation" },
  "annot-delete": { kind: "mutate", label: "Deleting the annotation" },
  "redact-add": { kind: "mutate", label: "Marking an area for redaction" },
  "redact-set": { kind: "mutate", label: "Changing a redaction mark" },
  "redact-search": { kind: "mutate", label: "Marking search results for redaction" },
  "redact-preview": { kind: "read", label: "Previewing the redactions" },
  "redact-apply": { kind: "mutate", label: "Applying the redactions", long: true },
  "sign-prepare": { kind: "read", label: "Preparing the signature", long: true },
  "sign-list": { kind: "read", label: "Reading the signatures" },
  "page-rotate": { kind: "mutate", label: "Rotating the page" },
  "page-delete": { kind: "mutate", label: "Deleting the page" },
  "page-move": { kind: "mutate", label: "Moving the page" },
  fields: { kind: "read", label: "Reading the form fields" },
  "field-set": { kind: "mutate", label: "Filling in the form field" },
  "field-add": { kind: "mutate", label: "Adding the form field" },
  "field-props": { kind: "mutate", label: "Changing the form field" },
  "field-delete": { kind: "mutate", label: "Deleting the form field" },
  outline: { kind: "read", label: "Reading the bookmarks" },
  "outline-add": { kind: "mutate", label: "Adding the bookmark" },
  "outline-set": { kind: "mutate", label: "Renaming the bookmark" },
  "outline-delete": { kind: "mutate", label: "Deleting the bookmark" },
  links: { kind: "read", label: "Reading the links" },
  "link-add": { kind: "mutate", label: "Adding the link" },
  "link-delete": { kind: "mutate", label: "Deleting the link" },
  "link-set": { kind: "mutate", label: "Changing the link" },
  "insert-pages": { kind: "mutate", label: "Inserting the pages", long: true },
  "extract-pages": { kind: "read", label: "Extracting the pages", long: true },
  "pdf-from-images": { kind: "read", label: "Making a PDF from the images" },
  "scan-compose": { kind: "read", label: "Making the scanned PDF" },
  "scan-measure": { kind: "read", label: "Measuring the scanned text" },
  "text-all": { kind: "read", label: "Exporting the text" },
  "compare-open": { kind: "compare", label: "Opening the document to compare" },
  "compare-close": { kind: "compare", label: "Closing the comparison" },
  "compare-render": { kind: "compare", label: "Showing the compared page" },
  "compare-text": { kind: "compare", label: "Comparing the text" },
  stamp: { kind: "mutate", label: "Adding the watermark or header" },
  "batch-file": { kind: "read", label: "Processing a batch file", long: true },
  labels: { kind: "read", label: "Reading the page labels" },
  "labels-set": { kind: "mutate", label: "Setting the page labels" },
  "labels-clear": { kind: "mutate", label: "Clearing the page labels" },
  layers: { kind: "read", label: "Reading the layers" },
  "layer-set": { kind: "mutate", label: "Showing or hiding the layer" },
  "page-ids": { kind: "read", label: "Reading the pages" },
  "edit-targets": { kind: "read", label: "Finding the editable content" },
  "edit-text": { kind: "mutate", label: "Editing the text" },
  "edit-paragraph": { kind: "mutate", label: "Editing the paragraph" },
  "text-add": { kind: "mutate", label: "Adding the text" },
  "edit-object": { kind: "mutate", label: "Editing the object" },
  "edit-objects": { kind: "mutate", label: "Editing the objects" },
  "edit-replace-preview": { kind: "read", label: "Previewing the replacement" },
  "edit-replace-apply": { kind: "mutate", label: "Replacing the text" },
  "clip-copy": { kind: "clipboard", label: "Copying" },
  "clip-paste": { kind: "mutate", label: "Pasting" },
  "edit-batch": { kind: "mutate", label: "Editing the selection" },
  undo: { kind: "mutate", label: "Undo" },
  redo: { kind: "mutate", label: "Redo" },
};

/** The op's entry, or undefined for a name that is not an op. */
export function opInfo(op: unknown): { kind: OpKind; label: string; long?: true } | undefined {
  return typeof op === "string" && Object.hasOwn(OPS, op) ? OPS[op as Op] : undefined;
}

/** Kinds a replay sends again. */
export function replayed(op: unknown): boolean {
  const kind = opInfo(op)?.kind;
  return kind === "mutate" || kind === "unlock" || kind === "clipboard";
}

/**
 * What the renderer may hold for recovery: the base, the last saved copy and the log together.
 * The app opens up to 120 MiB (MAX_PDF_BYTES) and saves up to 50 MiB (MAX_SAVE_BYTES), so a
 * near-ceiling document still leaves 86 MiB of log; past it recovery is off for the document.
 */
export const RECOVERY_BUDGET_BYTES = 256 * 1024 * 1024;
/** The budget for a phone or tablet WebView (the mobile app has no recovery client yet; this is its default). */
export const MOBILE_RECOVERY_BUDGET_BYTES = 96 * 1024 * 1024;
/** Desktops that report 4 GiB of memory or less (`navigator.deviceMemory`) keep half the desktop budget. */
export const SMALL_RECOVERY_BUDGET_BYTES = 128 * 1024 * 1024;

/**
 * The budget for this device: `mobile` takes the mobile default; a known `deviceMemoryGiB`
 * (Chromium's `navigator.deviceMemory`, rounded and capped at 8) of 4 or less takes the small one.
 */
export function recoveryBudgetFor(device: { mobile?: boolean; deviceMemoryGiB?: unknown } = {}): number {
  if (device.mobile) return MOBILE_RECOVERY_BUDGET_BYTES;
  const memory = device.deviceMemoryGiB;
  if (typeof memory === "number" && Number.isFinite(memory) && memory > 0 && memory <= 4) return SMALL_RECOVERY_BUDGET_BYTES;
  return RECOVERY_BUDGET_BYTES;
}

/**
 * A replay may take this long plus twice what its changes took the first time (`replayBudgetMs`);
 * past it, the restore gives up and reopens the last saved state.
 */
export const REPLAY_BASE_MS = 30_000;
/**
 * Edit handles and replace preview tokens name the worker's document generation, which counts
 * the opens and closes since the worker started; a replay closes this many times at most to
 * reach it (each close is one cheap round trip).
 */
export const MAX_REPLAY_CLOSES = 10_000;

/** Why recovery is off for the open document. */
export type Unavailable = "budget" | "clipboard" | "generation";

/** `capture`: for a `saved` with a ticket, how many entries the log held when that save captured its state. */
type Entry = { op: Op; message: Record<string, unknown>; bytes: number; reply: string; ms: number; capture?: number };

/** The generation a save ticket names (its first field, on both engines), or null. */
function ticketGeneration(ticket: unknown): number | null {
  if (typeof ticket !== "string") return null;
  const generation = Number(ticket.split(":")[0]);
  return Number.isInteger(generation) ? generation : null;
}

/** The generation a logged request names: an edit handle's, or a `saved` ticket's. */
function messageGeneration(message: Record<string, unknown>): number | null {
  if (typeof message.generation === "number") return message.generation;
  return message.op === "saved" ? ticketGeneration(message.ticket) : null;
}

/** What `keep` took from a request before it was posted (its buffers are transferred). */
export type Kept = { op: Op; bytes: number; message: Record<string, unknown> | null };

/** Approximate bytes a request holds: buffers in full, strings at two bytes a unit. */
export function sizeOf(value: unknown): number {
  let total = 0;
  const stack: unknown[] = [value];
  const seen = new Set<object>();
  while (stack.length) {
    const item = stack.pop();
    if (typeof item === "string") total += 16 + item.length * 2;
    else if (item instanceof ArrayBuffer) total += 16 + item.byteLength;
    else if (ArrayBuffer.isView(item)) total += 16 + item.byteLength;
    else if (item && typeof item === "object") {
      if (seen.has(item)) continue;
      seen.add(item);
      total += 16;
      for (const child of Object.values(item)) stack.push(child);
    } else total += 8;
  }
  return total;
}

/** Keys whose value may differ between two runs of the same request: timings, the request id. */
function volatile(key: string): boolean {
  return key === "id" || key === "compareClosed" || /Ms$/.test(key);
}

/** cyrb53: a 53-bit string hash; enough to tell two answers apart, not a security check. */
function hash(text: string): string {
  let h1 = 0xdeadbeef;
  let h2 = 0x41c6ce57;
  for (let i = 0; i < text.length; i++) {
    const code = text.charCodeAt(i);
    h1 = Math.imul(h1 ^ code, 2654435761);
    h2 = Math.imul(h2 ^ code, 1597334677);
  }
  h1 = Math.imul(h1 ^ (h1 >>> 16), 2246822507) ^ Math.imul(h2 ^ (h2 >>> 13), 3266489909);
  h2 = Math.imul(h2 ^ (h2 >>> 16), 2246822507) ^ Math.imul(h1 ^ (h1 >>> 13), 3266489909);
  return (4294967296 * (2097151 & h2) + (h1 >>> 0)).toString(36);
}

/**
 * An answer reduced to what a replay must reproduce: every field but timings and the request
 * id, buffers by length. Two answers with the same fingerprint carry the same ids, revision,
 * dirty and undo state and page geometry.
 */
export function fingerprint(reply: unknown): string {
  const text = JSON.stringify(reply, (key, value: unknown) => {
    if (volatile(key)) return undefined;
    if (value instanceof ArrayBuffer) return { $bytes: value.byteLength };
    if (ArrayBuffer.isView(value)) return { $bytes: value.byteLength };
    return value;
  }) ?? "";
  return `${text.length}:${hash(text)}`;
}

/** The request without its worker id, deep-copied (its buffers too) before it is posted. */
function copyMessage(message: Record<string, unknown>): Record<string, unknown> {
  const { id: _id, ...rest } = message;
  return structuredClone(rest);
}

/**
 * The open document's recovery state. One per window: a successful `open` starts it (`record`),
 * `reset` on close, on a failed open and when a document leaves the window.
 */
export class RecoveryLog {
  /** The bytes the document was opened from; null when none is tracked. */
  base: ArrayBuffer | null = null;
  name = "";
  /** The open answer's fingerprint, and how long the open took (ms, request to answer). */
  opened = "";
  openedMs = 0;
  /** The bytes of the last acknowledged save in place, or null (never saved here, or too large). */
  saved: ArrayBuffer | null = null;
  /** True once a save in place was acknowledged whose bytes are not kept: no fallback then. */
  savedLost = false;
  entries: Entry[] = [];
  /** Why recovery is off for this document; null while it is on (or nothing is open). */
  unavailable: Unavailable | null = null;
  /** The worker clipboard was last filled by a `clip-copy` in this log. */
  clipboardInLog = false;
  private entryBytes = 0;
  /** Save tickets this log saw issued: the entry count at capture, and the restore epoch they belong to. */
  private captures = new Map<string, { index: number; epoch: number }>();
  private epoch = 0;

  constructor(readonly budget = RECOVERY_BUDGET_BYTES) {}

  /** Bytes held: base, saved copy and log. */
  get bytes(): number {
    return (this.base?.byteLength ?? 0) + (this.saved?.byteLength ?? 0) + this.entryBytes;
  }

  /** A document is tracked and its log can be replayed. */
  get available(): boolean {
    return this.base !== null;
  }

  /** Forgets the document: nothing is open, or what is open cannot be tracked. */
  reset(): void {
    this.base = null;
    this.name = "";
    this.opened = "";
    this.openedMs = 0;
    this.saved = null;
    this.savedLost = false;
    this.entries = [];
    this.entryBytes = 0;
    this.unavailable = null;
    this.clipboardInLog = false;
    this.captures.clear();
  }

  /** Turns recovery off for this document and frees what it held. */
  private off(reason: Unavailable): void {
    this.unavailable = reason;
    this.base = null;
    this.entries = [];
    this.entryBytes = 0;
    this.saved = null;
  }

  /**
   * Before a request is posted: a copy of what the log will need if it succeeds, or null when
   * the request is not tracked. A request that would pass the budget keeps nothing; `record`
   * then turns recovery off if it succeeds (a refused one changes nothing).
   */
  keep(message: Record<string, unknown>): Kept | null {
    const op = message.op as Op;
    const kind = opInfo(op)?.kind;
    if (kind === "base") {
      if (op !== "open" || !(message.pdf instanceof ArrayBuffer)) return { op, bytes: 0, message: null };
      const bytes = message.pdf.byteLength;
      return { op, bytes, message: bytes <= this.budget ? copyMessage(message) : null };
    }
    if (!replayed(op)) return null;
    // An untracked copy still has to be seen: it replaces the clipboard a later paste reads.
    if (!this.available) return op === "clip-copy" ? { op, bytes: 0, message: null } : null;
    const bytes = sizeOf(message);
    return { op, bytes, message: this.bytes + bytes <= this.budget ? copyMessage(message) : null };
  }

  /** After the worker answered `kept`'s request successfully (an `ok: false` answer is not recorded). */
  record(kept: Kept, reply: Record<string, unknown>, ms = 0): void {
    const { op } = kept;
    if (op === "open") {
      this.reset();
      const message = kept.message;
      if (message && message.pdf instanceof ArrayBuffer) {
        this.base = message.pdf;
        this.name = typeof message.name === "string" ? message.name : "";
        this.opened = fingerprint(reply);
        this.openedMs = ms;
      } else this.unavailable = "budget";
      return;
    }
    if (op === "close") {
      this.reset();
      return;
    }
    if (op === "authenticate" && reply.authenticated !== true) return;
    if (op === "clip-copy" && !this.available) {
      this.clipboardInLog = false;
      return;
    }
    if (!this.available) return;
    if (!kept.message) return this.off("budget");
    if (op === "clip-paste" && !this.clipboardInLog) return this.off("clipboard");
    const generation = messageGeneration(kept.message);
    if (generation !== null) {
      const known = this.generation();
      if ((known !== null && known !== generation) || generation < 1 || generation - 1 > MAX_REPLAY_CLOSES) return this.off("generation");
    }
    let capture: number | undefined;
    if (op === "saved" && typeof kept.message.ticket === "string") {
      capture = this.captures.get(kept.message.ticket)?.index;
      // A ticket the log never saw issued cannot be mapped onto a replay.
      if (capture === undefined) return this.off("generation");
    }
    this.entries.push({ op, message: kept.message, bytes: kept.bytes, reply: fingerprint(reply), ms: Math.max(0, ms), ...(capture === undefined ? {} : { capture }) });
    this.entryBytes += kept.bytes;
    if (op === "clip-copy") this.clipboardInLog = true;
  }

  /**
   * After an acknowledged save in place (`saved` recorded): `bytes` (a copy of what was written,
   * or null when it could not be kept) becomes the fallback a failed replay reopens.
   */
  savedCopy(bytes: ArrayBuffer | null): void {
    if (!this.available) return;
    this.saved = null;
    if (bytes && this.bytes + bytes.byteLength <= this.budget) {
      this.saved = bytes;
      this.savedLost = false;
    } else this.savedLost = true;
  }

  /** Whether a saved copy of `size` bytes fits (the previous copy is replaced). */
  fitsSaved(size: number): boolean {
    return this.available && this.bytes - (this.saved?.byteLength ?? 0) + size <= this.budget;
  }

  /** How long a replay of this log may take: REPLAY_BASE_MS plus twice the open and the changes as first run. */
  replayBudgetMs(): number {
    return REPLAY_BASE_MS + 2 * (this.openedMs + this.entries.reduce((sum, entry) => sum + entry.ms, 0));
  }

  /** The document generation the logged edit handles carry, or null when none carries one. */
  generation(): number | null {
    for (const entry of this.entries) {
      const generation = messageGeneration(entry.message);
      if (generation !== null) return generation;
    }
    // A save whose `saved` is still to come names it too (a restore must accept that `saved`).
    for (const ticket of this.captures.keys()) {
      const generation = ticketGeneration(ticket);
      if (generation !== null) return generation;
    }
    return null;
  }

  /**
   * A `save` answered with `ticket` (both engines): the state it names is the log's current
   * length. Its `saved` is logged with that index, so a replay can rebuild the ticket.
   */
  noteSave(ticket: unknown): void {
    if (!this.available || typeof ticket !== "string" || ticketGeneration(ticket) === null) return;
    this.captures.set(ticket, { index: this.entries.length, epoch: this.epoch });
  }

  /**
   * The ticket a replayed session accepts for a state captured after `capture` entries. The
   * generation is the replay's own (its closes reach it) and the state part is the same journal
   * position or serial; only the security part differs: Rust counts `security-set` requests
   * over the whole session, a replay from its opened bytes only those in the log.
   */
  replayTicket(ticket: string, capture: number): string {
    const parts = ticket.split(":");
    if (parts.length < 3) return ticket;
    parts[2] = String(this.entries.slice(0, capture).filter(entry => entry.op === "security-set").length);
    return parts.join(":");
  }

  /** After a successful replay: tickets issued before it are rebuilt for the replayed session (`ticketNow`). */
  restored(): void {
    this.epoch += 1;
  }

  /** The ticket to send now: one issued before the last restore, rebuilt for the restored session. */
  ticketNow(ticket: string): string {
    const capture = this.captures.get(ticket);
    if (!capture || capture.epoch === this.epoch) return ticket;
    const rebuilt = this.replayTicket(ticket, capture.index);
    this.captures.set(rebuilt, { index: capture.index, epoch: this.epoch });
    return rebuilt;
  }

  /** What a failed replay reopens: the last saved copy, else the base; null when neither is the saved state. */
  fallback(): ArrayBuffer | null {
    if (this.saved) return this.saved;
    return this.savedLost ? null : this.base;
  }
}

/** A replayed answer that differs from the original: the replay stops there. */
export class ReplayMismatch extends Error {
  constructor(readonly op: Op, readonly index: number) {
    super(`replay: ${op} (step ${index + 1}) did not answer as it did before`);
  }
}

/** The replay ran past `replayBudgetMs`: the restore reopens the last saved state instead. */
export class ReplayOverBudget extends Error {
  constructor(readonly done: number, readonly total: number) {
    super(`replay: still running after its time budget (${done} of ${total} changes replayed)`);
  }
}

/** Sends one request to the fresh worker and resolves with its answer; `long` is the op's 120 s limit. */
export type ReplaySend = (message: Record<string, unknown>, long: boolean) => Promise<Record<string, unknown>>;

export type ReplayResult = {
  /** The open answer (page geometry and info as opened). */
  open: Record<string, unknown>;
  /** The last answer that carried journal state, or the open answer. */
  journal: Record<string, unknown>;
  /** The last answer that carried page geometry, or the open answer. */
  pages: Record<string, unknown>;
  steps: number;
};

/**
 * Reopens `log`'s base in a fresh worker and sends its log again, in order. The send must not
 * let the worker keep or detach the log's buffers (postMessage without a transfer list copies).
 * Throws on the first answer that differs from the original, on a refusal and on a failed send.
 */
export type ReplayOptions = {
  /** Called after the open (done 0) and after each replayed change. */
  onStep?: (done: number, total: number) => void;
  /** Overall limit in ms (default: the log's `replayBudgetMs`); checked between steps. */
  budgetMs?: number;
  /** The clock (performance.now). */
  now?: () => number;
};

export async function replay(log: RecoveryLog, send: ReplaySend, options: ReplayOptions = {}): Promise<ReplayResult> {
  const now = options.now ?? (() => performance.now());
  const deadline = now() + (options.budgetMs ?? log.replayBudgetMs());
  const total = log.entries.length;
  if (!log.available || !log.base) throw new Error("replay: recovery is not available for this document");
  const generation = log.generation();
  // A fresh worker numbers its first open 1; each close before it adds one.
  for (let closes = generation === null ? 0 : generation - 1; closes > 0; closes--) await send({ op: "close" }, false);
  const open = await send({ op: "open", pdf: log.base, name: log.name }, false);
  if (fingerprint(open) !== log.opened) throw new Error("replay: the document did not open as it did before");
  let journal = open;
  let pages = open;
  options.onStep?.(0, total);
  for (const [index, entry] of log.entries.entries()) {
    if (now() > deadline) throw new ReplayOverBudget(index, total);
    const message = entry.capture !== undefined && typeof entry.message.ticket === "string"
      ? { ...entry.message, ticket: log.replayTicket(entry.message.ticket, entry.capture) }
      : entry.message;
    const reply = await send(message, OPS[entry.op].long === true);
    if (fingerprint(reply) !== entry.reply) throw new ReplayMismatch(entry.op, index);
    if (typeof reply.revision === "number") journal = reply;
    if (Array.isArray(reply.pages) && typeof reply.pageCount === "number") pages = reply;
    options.onStep?.(index + 1, total);
  }
  return { open, journal, pages, steps: log.entries.length };
}
