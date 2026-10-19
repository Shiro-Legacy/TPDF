// Values only from modules that load no engine code; types from ./protocol.js.
import { allowed, permissionNames } from "./engine-core.js";
import { MAX_PDF_BYTES, MAX_SAVE_BYTES } from "./limits.js";
import { opInfo, recoveryBudgetFor, replay, replayed, RecoveryLog, ReplayOverBudget, type Kept, type ReplayResult } from "./recovery.js";
import {
  createGate,
  createLatestGuard,
  createRenderCache,
  MAX_HITS_DEFAULT,
  MAX_FIELD_TEXT_CHARS,
  MAX_BOOKMARK_CHARS,
  MAX_NOTE_CHARS,
  MAX_STAMP_IMAGE_BYTES,
  MAX_INK_POINTS,
  MAX_URI_CHARS,
  MAX_PIXEL_EDGE,
  normaliseLine,
  MAX_SCALE,
  MAX_PASSWORD_CHARS,
  MIN_SCALE,
  NOTE_SIZE,
  parsePageRange,
  SHAPE_SIZE,
  SIGNATURE_SIZE,
  SIGNATURE_SUBJECT,
  DRAWN_SIGNATURE_SUBJECT,
  THUMB_CACHE_BYTES,
  THUMB_CACHE_ENTRIES,
  THUMB_MAX_EDGE,
  visibleRange,
} from "./engine-shared.js";
import type {
  AnnotItem,
  AuthLevel,
  DocumentInfo,
  DocumentPermissions,
  EditTarget,
  FieldItem,
  JournalState,
  LayerItem,
  Line,
  LinkEntry,
  OutlineEntry,
  PageIdsReply,
  PageSize,
  RenderResult,
  SearchHit,
  SecurityState,
  OpenReply,
  RenderReply,
  SearchReply,
  SaveReply,
  ThumbReply,
  ValidateReply,
  AnnotsReply,
  MutationReply,
  PageReply,
  FieldsReply,
  OutlineReply,
  LinksReply,
  InsertReply,
  ExtractReply,
  PdfFromImagesReply,
  TextAllReply,
  LabelsReply,
  LabelsSetReply,
  LayersReply,
  LayerSetReply,
  AuthenticateReply,
  SecurityReply,
  Reply,
  ErrorReply,
  RefusalReply,
  RedactInventory,
  RedactPreview,
  RedactSanitize,
  CompareLine,
  CompareOpenReply,
  CompareRenderReply,
  CompareTextReply,
} from "./protocol.js";


type Pending = {
  resolve: (reply: Reply) => void;
  reject: (error: Error) => void;
  timer: number;
  /** The request's op and page; `kept` is what the recovery log keeps of it (M9), null when untracked. */
  op: unknown;
  page: number | null;
  kept: Kept | null;
  /** A replay's own request (M9): never logged. */
  replaying: boolean;
  /** When it was posted, and when it is overdue (later if it waited behind a long job). */
  started: number;
  deadline: number;
  /** It runs under the long-job (120 s) or the thumbnail (10 s) limit. */
  long: boolean;
  thumb: boolean;
  /** Timed out without stopping the worker, which is still running it; its answer is dropped. */
  abandoned: boolean;
};

type OpenError = { reason: "too-large" | "unreadable" | "cancelled"; name?: string; limit?: number };

/** main delivers the document as a one-shot `app://pdf4all/open/<token>` URL (contract §1). */
type IncomingDocument = { name: string; size?: number; url?: string; untitled?: boolean };

/** main's save-result contract; the renderer only reads it. */
type SaveResult =
  | { ok: true; name: string; warning?: "source-changed" | "durability-uncertain" }
  | {
    ok: false;
    reason: "cancelled" | "too-large" | "disk-full" | "permission" | "unreadable"
      | "same-file" | "document-changed" | "source-changed" | "busy" | "write-failed" | "broken-link";
  };
/** Every reason main can refuse a save with; SAVE_REASON_TEXT must word each one (tests/m10-save-failure.test.mjs). */
type SaveFail = Extract<SaveResult, { ok: false }>["reason"];

/** Unit 5 main/preload side; reason tokens are fixed by contract §B.2. */
type PrintResult =
  | { ok: true }
  | { ok: false; reason: "cancelled" | "busy" | "failed" | "no-printer" };

type Pdf4AllApi = {
  openFile(): Promise<void>;
  /** Unit 4 main/preload side; optional so the renderer still runs on an older bridge. */
  openDropped?(file: File): Promise<void>;
  onDocument(callback: (doc: IncomingDocument) => void): void;
  onOpenError(callback: (error: OpenError) => void): void;
  saveFile(bytes: ArrayBuffer, documentUrl: string, options?: { inPlace?: boolean; filled?: boolean; suggestedName?: string }): Promise<SaveResult>;
  onSaveRequest(callback: (payload?: { inPlace?: boolean }) => void): void;
  /**
   * P2 unit 1 main/preload side; optional so an older bridge still runs. Main
   * owns the discard prompt (before its open dialog and before delivering a
   * drop), so the renderer only reports its dirty state and never asks.
   */
  setDirty?(dirty: boolean): void;
  onEditRequest?(callback: (payload: { op: string; contents?: string }) => void): void;
  /** Unit 5 main/preload side; optional so the renderer still runs on an older bridge. */
  print?(): Promise<PrintResult>;
  onPrintRequest?(callback: (payload: { range?: string }) => void): void;
  /**
   * Unit 5 main/preload side; optional so the renderer still runs on an older bridge.
   * Main re-checks the scheme allowlist, confirms and opens it in the browser.
   */
  openLink?(uri: string): void;
  /**
   * Unit 6 main/preload side; optional so the renderer still runs on an older bridge.
   * Main owns the picker (and `--insert-from` in smoke mode) and hands back bytes only.
   */
  pickInsertSource?(): Promise<
    | { ok: true; name: string; bytes: ArrayBuffer }
    | { ok: false; reason: "cancelled" | "too-large" | "unreadable" }
  >;
  /**
   * Unit 7 main/preload side; optional so the renderer still runs on an older bridge.
   * Main owns the dialog, the extension and the write; `open` asks it to open the
   * written PDF (the "New from images…" path).
   */
  saveNew?(bytes: ArrayBuffer, suggestedName: string, kind: "pdf" | "png" | "jpeg" | "txt", options?: { open?: boolean }): Promise<
    | { ok: true; name: string }
    | { ok: false; reason: "cancelled" | "too-large" | "unreadable" | "busy" | "permission" | "disk-full" | "broken-link" }
  >;
  pickImages?(): Promise<
    | { ok: true; images: { name: string; bytes: ArrayBuffer }[] }
    | { ok: false; reason: "cancelled" | "too-many" | "too-large" | "unreadable" }
  >;
  pickCertificate?(opts?: { path?: string }): Promise<{ ok: true; name: string } | { ok: false; reason: string }>;
  certificateInfo?(password: string): Promise<{ ok: true; cn: string; notAfter: string; certs: number } | { ok: false; reason: string; date?: string; status?: string }>;
  certificateClear?(): Promise<{ ok: boolean }>;
  verifySignatures?(bytes: ArrayBuffer, entries: Record<string, unknown>[]): Promise<{ ok: true; verdicts: Record<string, unknown>[]; trusted: { cn: string; notAfter: string }[] } | { ok: false; reason: string }>;
  trustAdd?(opts?: { path?: string }): Promise<{ ok: boolean; reason?: string; added?: number; total?: number }>;
  trustClear?(): Promise<{ ok: boolean }>;
  /** draw-signature D3: the saved drawn signature (validated in main), or null to remove it. */
  signatureGet?(): Promise<{ ok: boolean; signature: unknown }>;
  signatureSet?(value: DrawnSig | ImageSig | null): Promise<{ ok: boolean }>;
  /** Batch (contract-batch.md D3): main lists a folder and reads/writes only inside it. */
  pickFolder?(): Promise<{ ok: true; folder: string; out: string; files: { name: string; bytes: number }[] } | { ok: false; reason: string }>;
  batchIo?(name: string, action: "read" | "write" | "exists", bytes?: ArrayBuffer): Promise<{ ok: true; bytes?: ArrayBuffer; exists?: boolean } | { ok: false; reason: string }>;
  signFile?(bytes: ArrayBuffer, offsets: { contentsAt: number; contentsLen: number; byteRangeAt: number }, when: number, suggestedName: string): Promise<{ ok: true; name: string; signer: string } | { ok: false; reason: string }>;
};

type ZoomMode = "fit-width" | "fit-page" | "fixed";
/** The chosen task: which context row is shown, and which inspector tab it belongs to. */
type UiMode = "read" | "annotate" | "organize" | "forms" | "edit" | "redact";

/** The command modifier is ⌘ on macOS and Ctrl elsewhere (a drive `Meta+` key sets metaKey on every platform). */
const IS_MAC = navigator.platform.startsWith("Mac");
const cmdKey = (event: { metaKey: boolean; ctrlKey?: boolean }): boolean => event.metaKey || (!IS_MAC && event.ctrlKey === true);
/** index.html is written with the macOS keys; elsewhere the page names this platform's: ⌥⌘K → Ctrl+Alt+K, ⌥-click → Alt-click, "this Mac" → "this computer". */
function platformWording(text: string): string {
  return text
    .replace(/[⌘⌥⇧]+(?=([\s-]|$)?)/g, (keys: string, bare: string | undefined) =>
      [["⌘", "Ctrl"], ["⌥", "Alt"], ["⇧", "Shift"]].filter(([glyph]) => keys.includes(glyph as string)).map(([, name]) => name).join("+") + (bare === undefined ? "+" : ""))
    .replaceAll("⌫", "Backspace")
    .replaceAll("Cmd/Ctrl", "Ctrl")
    .replaceAll("this Mac", "this computer");
}
if (!IS_MAC) {
  // No inset window buttons to keep clear of (styles.css `#titlebar`).
  document.getElementById("titlebar")?.classList.add("no-inset");
  for (const element of document.querySelectorAll("body, body *")) {
    for (const name of ["title", "aria-label"]) {
      const value = element.getAttribute(name);
      if (value !== null && platformWording(value) !== value) element.setAttribute(name, platformWording(value));
    }
    for (const node of element.childNodes) {
      if (node.nodeType === Node.TEXT_NODE && node.nodeValue && platformWording(node.nodeValue) !== node.nodeValue) node.nodeValue = platformWording(node.nodeValue);
    }
  }
}
const CACHE_LIMIT = 8;
const CACHE_BYTES = 96 * 1024 * 1024;
const REQUEST_TIMEOUT_MS = 20_000;
/** A full rewrite of a near-ceiling document can take well over the render watchdog. */
const SAVE_TIMEOUT_MS = 120_000;
const MAX_WORKER_RESTARTS = 3;
const MAX_ZOOM = 8;
const ZOOM_STEPS = [0.5, 0.75, 1, 1.25, 1.5, 2, 3, 4];
const SEARCH_DEBOUNCE_MS = 200;
/** Sidebar geometry. THUMB_ROW_PX is duplicated as the inline row height in buildThumbs. */
const THUMB_ROW_PX = 88;
const THUMB_BOX_PX = 64;
const THUMB_BUFFER = 4;
/** Thumbnails are decoration: a slow one must not be mistaken for a hung worker. */
const THUMB_TIMEOUT_MS = 10_000;
/** Print raster resolution. ponytail: fixed knob - a print path that needs 300 DPI or a
 * per-page paper size changes this and the sheet CSS, not the rest of the build. */
const PRINT_DPI = 150;
/**
 * Total JPEG data-URL bytes for one job. Measured, not guessed: printing all 500 pages
 * of large.pdf made 52,563,572 bytes of data URLs and peaked the Tab at 599,920 KiB
 * against the 572 MiB gate (~6.5 MiB of working set per data-URL MiB, on top of the
 * ~236 MiB baseline) - .quad/shared/p1/unit5-print-large-all.json. 40 MiB is about 380
 * pages of that document, i.e. ~500 MiB peak. ponytail: one size for every document and
 * one job at a time; a real ceiling per document (page size x DPI) only if someone hits it.
 */
const PRINT_MAX_BYTES = 40 * 1024 * 1024;

const statusEl = document.getElementById("status") as HTMLElement;
const errorEl = document.getElementById("error") as HTMLElement;
const bannerEl = document.getElementById("banner") as HTMLElement;
const bannerTextEl = document.getElementById("banner-text") as HTMLElement;
const bannerDismissEl = document.getElementById("banner-dismiss") as HTMLButtonElement;
const warningEl = document.getElementById("warning") as HTMLElement;
const canvas = document.getElementById("canvas") as HTMLCanvasElement;
const overlay = document.getElementById("text-layer") as unknown as SVGSVGElement;
const hitLayer = document.getElementById("hit-layer") as HTMLElement;
const fieldBar = document.getElementById("field-bar") as HTMLElement;
const fieldCount = document.getElementById("field-count") as HTMLElement;
const fieldHighlight = document.getElementById("field-highlight") as HTMLInputElement;
fieldHighlight.setAttribute("aria-label", "Highlight fields");
const fieldNext = document.getElementById("field-next") as HTMLButtonElement;
const extracted = document.getElementById("extracted") as HTMLElement;
const caption = document.getElementById("caption") as HTMLElement;
const viewport = document.getElementById("viewport") as HTMLElement;
const pageRegion = document.getElementById("page") as HTMLElement;
/** P6 U5 §2.4: the object context menu (Replace / Duplicate / Delete). */
const objectMenu = document.getElementById("object-menu") as HTMLElement;
const objectMenuButtons = {
  // P6 U7 §2.1: Cut, Copy and Paste come first (DOM order).
  cut: document.getElementById("object-menu-cut") as HTMLButtonElement,
  copy: document.getElementById("object-menu-copy") as HTMLButtonElement,
  paste: document.getElementById("object-menu-paste") as HTMLButtonElement,
  replace: document.getElementById("object-menu-replace") as HTMLButtonElement,
  duplicate: document.getElementById("object-menu-duplicate") as HTMLButtonElement,
  // P6 U6 §2.1: 90° steps, flips and crop mode (a menuitemcheckbox), in DOM order; Delete stays last.
  rotateCw: document.getElementById("object-menu-rotate-cw") as HTMLButtonElement,
  rotateCcw: document.getElementById("object-menu-rotate-ccw") as HTMLButtonElement,
  flipH: document.getElementById("object-menu-flip-h") as HTMLButtonElement,
  flipV: document.getElementById("object-menu-flip-v") as HTMLButtonElement,
  crop: document.getElementById("object-menu-crop") as HTMLButtonElement,
  delete: document.getElementById("object-menu-delete") as HTMLButtonElement,
};
/** The page and zoom the open menu belongs to; the press that dismissed it (§9.22). */
let objectMenuAt: { page: number; zoom: number } | null = null;
let objectMenuDismiss: Event | null = null;
/** P6 U7 §9.25: what the open menu belongs to, and the page menu's view point (y-down page pt; null = keyboard, no `at`). */
let objectMenuKind: "selection" | "page" = "selection";
let objectMenuPoint: number[] | null = null;
/** P6 U7 §10.1/D1: the worker clipboard's counts from the last `clip-copy` (kept across `open`; a worker failure clears it). */
let clipboardInfo: { count: number; kinds: Record<string, number> } | null = null;
/** The drive's `menu` page point while its synthetic contextmenu is dispatched (null otherwise). */
let driveMenuPoint: number[] | null = null;
const pageInput = document.getElementById("page-input") as HTMLInputElement;
const prevButton = document.getElementById("prev") as HTMLButtonElement;
const nextButton = document.getElementById("next") as HTMLButtonElement;
const openButton = document.getElementById("open") as HTMLButtonElement;
const saveButton = document.getElementById("save") as HTMLButtonElement;
const saveCopyButton = document.getElementById("save-copy") as HTMLButtonElement;
const undoButton = document.getElementById("undo") as HTMLButtonElement;
const redoButton = document.getElementById("redo") as HTMLButtonElement;
const editStatusEl = document.getElementById("edit-status") as HTMLElement;
const notesHeading = document.getElementById("notes-heading") as HTMLElement;
const notesEmptyEl = document.getElementById("notes-empty") as HTMLElement;
const noteAddButton = document.getElementById("note-add") as HTMLButtonElement;
const markupButtons: [HTMLButtonElement, "Highlight" | "Underline" | "StrikeOut", string][] = [
  [document.getElementById("markup-highlight") as HTMLButtonElement, "Highlight", "Highlighted match"],
  [document.getElementById("markup-underline") as HTMLButtonElement, "Underline", "Underlined match"],
  [document.getElementById("markup-strikeout") as HTMLButtonElement, "StrikeOut", "Struck out match"],
];
const freeTextAddButton = document.getElementById("freetext-add") as HTMLButtonElement;
const shapeButtons: [HTMLButtonElement, "Square" | "Circle" | "Line", string][] = [
  [document.getElementById("shape-square") as HTMLButtonElement, "Square", "Added square"],
  [document.getElementById("shape-circle") as HTMLButtonElement, "Circle", "Added circle"],
  [document.getElementById("shape-line") as HTMLButtonElement, "Line", "Added line"],
];
const inkDrawButton = document.getElementById("ink-draw") as HTMLButtonElement;
const stampIconButton = document.getElementById("stamp-icon") as HTMLButtonElement;
const signButton = document.getElementById("sign-add") as HTMLButtonElement;
const stampIconDialog = document.getElementById("stamp-icon-dialog") as HTMLDialogElement;
const stampIconSaveButton = document.getElementById("stamp-icon-save") as HTMLButtonElement;
const stampIconCancelButton = document.getElementById("stamp-icon-cancel") as HTMLButtonElement;
const noteListEl = document.getElementById("note-list") as HTMLUListElement;
const noteDialog = document.getElementById("note-dialog") as HTMLDialogElement;
const noteDialogHeading = document.getElementById("note-dialog-heading") as HTMLElement;
const noteTextInput = document.getElementById("note-text") as HTMLTextAreaElement;
const noteSaveButton = document.getElementById("note-save") as HTMLButtonElement;
const noteCancelButton = document.getElementById("note-cancel") as HTMLButtonElement;
const zoomOutButton = document.getElementById("zoom-out") as HTMLButtonElement;
const zoomInButton = document.getElementById("zoom-in") as HTMLButtonElement;
const fitWidthButton = document.getElementById("fit-width") as HTMLButtonElement;
const fitPageButton = document.getElementById("fit-page") as HTMLButtonElement;
const zoom100Button = document.getElementById("zoom-100") as HTMLButtonElement;
const zoomLevel = document.getElementById("zoom-level") as HTMLOutputElement;
const searchInput = document.getElementById("search-input") as HTMLInputElement;
const searchPrevButton = document.getElementById("search-prev") as HTMLButtonElement;
const searchNextButton = document.getElementById("search-next") as HTMLButtonElement;
const searchStatusEl = document.getElementById("search-status") as HTMLElement;
const thumbsEl = document.getElementById("thumbs") as HTMLElement;
const thumbsToggle = document.getElementById("thumbs-toggle") as HTMLButtonElement;
const sidebarEl = document.getElementById("page-sidebar") as HTMLElement;
const dropHint = document.getElementById("drop-hint") as HTMLElement;
const passwordForm = document.getElementById("password-form") as HTMLFormElement;
const passwordName = document.getElementById("password-name") as HTMLElement;
const passwordInput = document.getElementById("password-input") as HTMLInputElement;
const passwordErrorEl = document.getElementById("password-error") as HTMLElement;
const printButton = document.getElementById("print") as HTMLButtonElement;
const printDialog = document.getElementById("print-dialog") as HTMLDialogElement;
const printRangeInput = document.getElementById("print-range") as HTMLInputElement;
const printStatusEl = document.getElementById("print-status") as HTMLElement;
const printStartButton = document.getElementById("print-start") as HTMLButtonElement;
const printCancelButton = document.getElementById("print-cancel") as HTMLButtonElement;
const printPagesEl = document.getElementById("print-pages") as HTMLElement;
const pageStatusEl = document.getElementById("page-status") as HTMLElement;
const fieldsEl = document.getElementById("fields") as HTMLElement;
const fieldsHeading = document.getElementById("fields-heading") as HTMLElement;
const fieldsEmptyEl = document.getElementById("fields-empty") as HTMLElement;
const fillTextboxAdd = document.getElementById("fill-textbox-add") as HTMLButtonElement;
const fillSignButton = document.getElementById("fill-sign") as HTMLButtonElement;
const fillSignDrawButton = document.getElementById("fill-sign-draw") as HTMLButtonElement;
const sigpadDialog = document.getElementById("sigpad-dialog") as HTMLDialogElement;
const sigpadCanvas = document.getElementById("sigpad-canvas") as HTMLCanvasElement;
const sigpadStatus = document.getElementById("sigpad-status") as HTMLElement;
const sigpadClearButton = document.getElementById("sigpad-clear") as HTMLButtonElement;
const sigpadImageButton = document.getElementById("sigpad-image") as HTMLButtonElement;
const sigpadRemoveButton = document.getElementById("sigpad-remove") as HTMLButtonElement;
const sigpadSaveButton = document.getElementById("sigpad-save") as HTMLButtonElement;
const sigpadCancelButton = document.getElementById("sigpad-cancel") as HTMLButtonElement;
const fieldsStatusEl = document.getElementById("fields-status") as HTMLElement;
const fieldsWarningEl = document.getElementById("fields-warning") as HTMLElement;
const fieldListEl = document.getElementById("field-list") as HTMLElement;
/** U6 field designer controls (contract-p3-unit6.md §3, archived in .quad/backup/history-20260924.tgz). */
const designerEl = document.getElementById("field-designer") as HTMLDetailsElement;
const designerHintEl = document.getElementById("field-designer-hint") as HTMLElement;
const fieldNewKindEl = document.getElementById("field-new-kind") as HTMLSelectElement;
const fieldNewNameEl = document.getElementById("field-new-name") as HTMLInputElement;
const fieldNewXEl = document.getElementById("field-new-x") as HTMLInputElement;
const fieldNewYEl = document.getElementById("field-new-y") as HTMLInputElement;
const fieldNewWEl = document.getElementById("field-new-w") as HTMLInputElement;
const fieldNewHEl = document.getElementById("field-new-h") as HTMLInputElement;
const fieldNewRectEl = document.getElementById("field-new-rect") as HTMLElement;
const fieldNewTextEl = document.getElementById("field-new-text") as HTMLElement;
const fieldNewComboEl = document.getElementById("field-new-combo") as HTMLElement;
const fieldNewRadioEl = document.getElementById("field-new-radio") as HTMLElement;
const fieldNewMaxLenEl = document.getElementById("field-new-maxlen") as HTMLInputElement;
const fieldNewMultilineEl = document.getElementById("field-new-multiline") as HTMLInputElement;
const fieldNewOptionsEl = document.getElementById("field-new-options") as HTMLTextAreaElement;
const fieldNewKidsEl = document.getElementById("field-new-kids") as HTMLTextAreaElement;
/** U7: the four shared properties, on the Add group and the Selected group (§3). */
const fieldNewLabelEl = document.getElementById("field-new-label") as HTMLInputElement;
const fieldNewReadOnlyEl = document.getElementById("field-new-readonly") as HTMLInputElement;
const fieldNewRequiredEl = document.getElementById("field-new-required") as HTMLInputElement;
const fieldNewDefaultTextEl = document.getElementById("field-new-default") as HTMLInputElement;
const fieldNewDefaultBoolEl = document.getElementById("field-new-default-check") as HTMLInputElement;
const fieldNewAddEl = document.getElementById("field-new-add") as HTMLButtonElement;
const fieldSelectEl = document.getElementById("field-select") as HTMLSelectElement;
const fieldPropNameEl = document.getElementById("field-prop-name") as HTMLInputElement;
const fieldPropXEl = document.getElementById("field-prop-x") as HTMLInputElement;
const fieldPropYEl = document.getElementById("field-prop-y") as HTMLInputElement;
const fieldPropWEl = document.getElementById("field-prop-w") as HTMLInputElement;
const fieldPropHEl = document.getElementById("field-prop-h") as HTMLInputElement;
const fieldPropRectEl = document.getElementById("field-prop-rect") as HTMLElement;
const fieldPropTextEl = document.getElementById("field-prop-text") as HTMLElement;
const fieldPropComboEl = document.getElementById("field-prop-combo") as HTMLElement;
const fieldPropMaxLenEl = document.getElementById("field-prop-maxlen") as HTMLInputElement;
const fieldPropMultilineEl = document.getElementById("field-prop-multiline") as HTMLInputElement;
const fieldPropOptionsEl = document.getElementById("field-prop-options") as HTMLTextAreaElement;
const fieldPropLabelEl = document.getElementById("field-prop-label") as HTMLInputElement;
const fieldPropReadOnlyEl = document.getElementById("field-prop-readonly") as HTMLInputElement;
const fieldPropRequiredEl = document.getElementById("field-prop-required") as HTMLInputElement;
const fieldPropDefaultTextEl = document.getElementById("field-prop-default") as HTMLInputElement;
const fieldPropDefaultBoolEl = document.getElementById("field-prop-default-check") as HTMLInputElement;
const fieldPropApplyEl = document.getElementById("field-prop-apply") as HTMLButtonElement;
const fieldPropDeleteEl = document.getElementById("field-prop-delete") as HTMLButtonElement;
/** U7 help surface (contract-p3-unit7.md §3, archived in .quad/backup/history-20260924.tgz): one static dialog, agent-driveable. */
const helpDialog = document.getElementById("help-dialog") as HTMLDialogElement;
const helpMatrixEl = document.getElementById("help-matrix") as HTMLTableElement;
const helpCloseEl = document.getElementById("help-close") as HTMLButtonElement;
const redactAreaButton = document.getElementById("redact-area") as HTMLButtonElement;
const redactSearchText = document.getElementById("redact-search-text") as HTMLInputElement;
const redactSearchGo = document.getElementById("redact-search-go") as HTMLButtonElement;
const redactCountEl = document.getElementById("redact-count") as HTMLOutputElement;
const redactApplyOpen = document.getElementById("redact-apply-open") as HTMLButtonElement;
const redactDialog = document.getElementById("redact-dialog") as HTMLDialogElement;
const redactPreviewEl = document.getElementById("redact-preview") as HTMLTableElement;
const redactSharedEl = document.getElementById("redact-shared") as HTMLUListElement;
const redactInlineAltEl = document.getElementById("redact-inline-alt") as HTMLElement;
const redactKeptEl = document.getElementById("redact-kept") as HTMLElement;
const redactApplyButton = document.getElementById("redact-apply") as HTMLButtonElement;
const redactCancelButton = document.getElementById("redact-cancel") as HTMLButtonElement;
const REDACT_KEYS = ["info", "xmp", "attachments", "outlines", "annots", "links", "fields", "layers", "struct"] as const;
const redactSanitizeBoxes = Object.fromEntries(REDACT_KEYS.map(key => [key, document.getElementById(`redact-sanitize-${key}`) as HTMLInputElement])) as Record<(typeof REDACT_KEYS)[number], HTMLInputElement>;
const outlineListEl = document.getElementById("outline-list") as HTMLUListElement;
const bookmarkAddButton = document.getElementById("bookmark-add") as HTMLButtonElement;
const linksHeading = document.getElementById("links-heading") as HTMLElement;
const linkListEl = document.getElementById("link-list") as HTMLUListElement;
const linkPageAddButton = document.getElementById("link-page-add") as HTMLButtonElement;
const linkUriAddButton = document.getElementById("link-uri-add") as HTMLButtonElement;
const navStatusEl = document.getElementById("nav-status") as HTMLElement;
const bookmarkDialog = document.getElementById("bookmark-dialog") as HTMLDialogElement;
const bookmarkDialogHeading = document.getElementById("bookmark-dialog-heading") as HTMLElement;
const bookmarkTitleInput = document.getElementById("bookmark-title") as HTMLInputElement;
const bookmarkSaveButton = document.getElementById("bookmark-save") as HTMLButtonElement;
const bookmarkCancelButton = document.getElementById("bookmark-cancel") as HTMLButtonElement;
const linkDialog = document.getElementById("link-dialog") as HTMLDialogElement;
const linkDialogHeading = document.getElementById("link-dialog-heading") as HTMLElement;
const linkTargetLabel = document.getElementById("link-target-label") as HTMLElement;
const linkTargetInput = document.getElementById("link-target") as HTMLInputElement;
const linkSaveButton = document.getElementById("link-save") as HTMLButtonElement;
const linkCancelButton = document.getElementById("link-cancel") as HTMLButtonElement;
const linkKindSelect = document.getElementById("link-kind") as HTMLSelectElement;
const linkDrawButton = document.getElementById("link-draw") as HTMLButtonElement;
const sigfieldDrawButton = document.getElementById("sigfield-draw") as HTMLButtonElement;
const editOutlinesInput = document.getElementById("edit-outlines") as HTMLInputElement;
/** Page tools act on the current page; ids match the Page menu's `edit-request` ops. */
const pageToolButtons: [HTMLButtonElement, string][] = [
  [document.getElementById("page-rotate-right") as HTMLButtonElement, "rotate-right"],
  [document.getElementById("page-rotate-left") as HTMLButtonElement, "rotate-left"],
  [document.getElementById("page-move-up") as HTMLButtonElement, "move-up"],
  [document.getElementById("page-move-down") as HTMLButtonElement, "move-down"],
  [document.getElementById("page-delete") as HTMLButtonElement, "delete-page"],
];
const pageLabelEl = document.getElementById("page-label") as HTMLOutputElement;
const watermarkButton = document.getElementById("stamp-watermark") as HTMLButtonElement;
const headerButton = document.getElementById("stamp-header") as HTMLButtonElement;
const pageLabelsButton = document.getElementById("page-labels") as HTMLButtonElement;
const stampDialog = document.getElementById("stamp-dialog") as HTMLDialogElement;
const stampDialogHeading = document.getElementById("stamp-dialog-heading") as HTMLElement;
const stampKindSelect = document.getElementById("stamp-kind") as HTMLSelectElement;
const stampTextInput = document.getElementById("stamp-text") as HTMLInputElement;
const stampPagesInput = document.getElementById("stamp-pages") as HTMLInputElement;
const stampAlignSelect = document.getElementById("stamp-align") as HTMLSelectElement;
const stampBatesStartInput = document.getElementById("stamp-bates-start") as HTMLInputElement;
const stampBatesPadInput = document.getElementById("stamp-bates-pad") as HTMLInputElement;
const stampSaveButton = document.getElementById("stamp-save") as HTMLButtonElement;
const stampCancelButton = document.getElementById("stamp-cancel") as HTMLButtonElement;
const labelsDialog = document.getElementById("labels-dialog") as HTMLDialogElement;
const labelsFromInput = document.getElementById("labels-from") as HTMLInputElement;
const labelsStyleSelect = document.getElementById("labels-style") as HTMLSelectElement;
const labelsPrefixInput = document.getElementById("labels-prefix") as HTMLInputElement;
const labelsStartInput = document.getElementById("labels-start") as HTMLInputElement;
const labelsSaveButton = document.getElementById("labels-save") as HTMLButtonElement;
const labelsClearButton = document.getElementById("labels-clear") as HTMLButtonElement;
const labelsCancelButton = document.getElementById("labels-cancel") as HTMLButtonElement;
const layersEl = document.getElementById("layers") as HTMLElement;
const layerListEl = document.getElementById("layer-list") as HTMLUListElement;
const layersNoteEl = document.getElementById("layers-note") as HTMLElement;
const layersEmptyEl = document.getElementById("layers-empty") as HTMLElement;
const linksEmptyEl = document.getElementById("links-empty") as HTMLElement;
const outlineEmptyEl = document.getElementById("outline-empty") as HTMLElement;
const pagesEmptyEl = document.getElementById("pages-empty") as HTMLElement;
const textEmptyEl = document.getElementById("text-empty") as HTMLElement;
const contentTargetsEl = document.getElementById("content-targets") as HTMLUListElement;
const contentEmptyEl = document.getElementById("content-empty") as HTMLElement;
const contentRefreshButton = document.getElementById("content-refresh") as HTMLButtonElement;
const contentAddArmButton = document.getElementById("content-add-arm") as HTMLButtonElement;
const contentAddFont = document.getElementById("content-add-font") as HTMLSelectElement;
const contentAddSize = document.getElementById("content-add-size") as HTMLInputElement;
const contentAddColor = document.getElementById("content-add-color") as HTMLInputElement;
const contentAddText = document.getElementById("content-add-text") as HTMLTextAreaElement;
const contentAddGo = document.getElementById("content-add-go") as HTMLButtonElement;
const contentForm = document.getElementById("content-edit-form") as HTMLFormElement;
const contentEditLabel = document.getElementById("content-edit-label") as HTMLElement;
const contentNewText = document.getElementById("content-new-text") as HTMLInputElement;
const contentHint = document.getElementById("content-hint") as HTMLElement;
/** U3: one line naming the characters this run's font is proven to draw. */
const contentRepertoire = document.getElementById("content-repertoire") as HTMLElement;
const contentDeleteButton = document.getElementById("content-delete") as HTMLButtonElement;
const contentCancelButton = document.getElementById("content-cancel") as HTMLButtonElement;
// U4: the Image/Form placement form, next to the text form and never open with it.
const contentObjectForm = document.getElementById("content-object-form") as HTMLFormElement;
const contentObjectLabel = document.getElementById("content-object-label") as HTMLElement;
const contentBoxX = document.getElementById("content-box-x") as HTMLInputElement;
const contentBoxY = document.getElementById("content-box-y") as HTMLInputElement;
const contentBoxW = document.getElementById("content-box-w") as HTMLInputElement;
const contentBoxH = document.getElementById("content-box-h") as HTMLInputElement;
const contentObjectHint = document.getElementById("content-object-hint") as HTMLElement;
const contentReplaceButton = document.getElementById("content-replace") as HTMLButtonElement;
// U5: the scoped find/replace panel inside the content editor.
const contentReplacePanel = document.getElementById("content-find-replace") as HTMLDetailsElement;
const contentFindInput = document.getElementById("content-find") as HTMLInputElement;
const contentReplaceWith = document.getElementById("content-replace-with") as HTMLInputElement;
const contentScopePage = document.getElementById("content-scope-page") as HTMLInputElement;
const contentScopeRun = document.getElementById("content-scope-run") as HTMLInputElement;
const contentPreviewButton = document.getElementById("content-preview-button") as HTMLButtonElement;
const contentMatchesEl = document.getElementById("content-matches") as HTMLElement;
const contentReplaceSummary = document.getElementById("content-replace-summary") as HTMLElement;
const contentReplaceApply = document.getElementById("content-replace-apply") as HTMLButtonElement;
const contentReplaceCancel = document.getElementById("content-replace-cancel") as HTMLButtonElement;
const contentDeleteObjectButton = document.getElementById("content-delete-object") as HTMLButtonElement;
const contentCancelObjectButton = document.getElementById("content-cancel-object") as HTMLButtonElement;
/** P6 U6 §2.1/§9.26: the Inspector's rotate / flip / crop row; it follows the menu's enable rules. */
const contentTransformButtons = {
  rotateCcw: document.getElementById("content-rotate-ccw") as HTMLButtonElement,
  rotateCw: document.getElementById("content-rotate-cw") as HTMLButtonElement,
  flipH: document.getElementById("content-flip-h") as HTMLButtonElement,
  flipV: document.getElementById("content-flip-v") as HTMLButtonElement,
  crop: document.getElementById("content-crop") as HTMLButtonElement,
};
const textPanelEl = document.getElementById("text-panel") as HTMLElement;
/** The target whose form is open, if any. */
let contentTarget: EditTarget | null = null;
/** U4: the Image/Form target whose placement form is open, if any. */
let contentObjectTarget: EditTarget | null = null;
const documentTitleEl = document.getElementById("document-title") as HTMLElement;
const dirtyChipEl = document.getElementById("dirty-chip") as HTMLElement;
const inspectorEl = document.getElementById("inspector") as HTMLElement;
const inspectorToggle = document.getElementById("inspector-toggle") as HTMLButtonElement;

/** The mode radios, and the one context row each mode shows. */
const modeInputs: [HTMLInputElement, UiMode, HTMLElement][] = [
  [document.getElementById("mode-read") as HTMLInputElement, "read", document.getElementById("context-read") as HTMLElement],
  [document.getElementById("mode-annotate") as HTMLInputElement, "annotate", document.getElementById("context-annotate") as HTMLElement],
  [document.getElementById("mode-organize") as HTMLInputElement, "organize", document.getElementById("context-organize") as HTMLElement],
  [document.getElementById("mode-forms") as HTMLInputElement, "forms", document.getElementById("context-forms") as HTMLElement],
  [document.getElementById("mode-edit") as HTMLInputElement, "edit", document.getElementById("context-edit") as HTMLElement],
  [document.getElementById("mode-redact") as HTMLInputElement, "redact", document.getElementById("context-redact") as HTMLElement],
];
const sidebarTabs = [
  document.getElementById("tab-pages") as HTMLButtonElement,
  document.getElementById("tab-bookmarks") as HTMLButtonElement,
];
const sidebarPanels = [
  document.getElementById("panel-pages") as HTMLElement,
  document.getElementById("panel-bookmarks") as HTMLElement,
];
const inspectorTabs = [
  document.getElementById("tab-notes") as HTMLButtonElement,
  document.getElementById("tab-fields") as HTMLButtonElement,
  document.getElementById("tab-links") as HTMLButtonElement,
  document.getElementById("tab-text") as HTMLButtonElement,
  document.getElementById("tab-layers") as HTMLButtonElement,
];
const inspectorPanels = [
  document.getElementById("panel-notes") as HTMLElement,
  document.getElementById("panel-fields") as HTMLElement,
  document.getElementById("panel-links") as HTMLElement,
  document.getElementById("text-panel") as HTMLElement,
  document.getElementById("panel-layers") as HTMLElement,
];
/** Which inspector tab each mode belongs to; Read and Organize read the document. */
const MODE_TAB: Record<UiMode, string> = {
  read: "tab-links",
  annotate: "tab-notes",
  organize: "tab-text",
  forms: "tab-fields",
  edit: "tab-text",
  redact: "tab-notes",
};
/** The chosen task. Read is the safe start: it arms nothing. */
let uiMode: UiMode = "read";
let inspectorOpen = true;
const newImagesButton = document.getElementById("new-images") as HTMLButtonElement;
const exportButton = document.getElementById("export") as HTMLButtonElement;
const exportDialog = document.getElementById("export-dialog") as HTMLDialogElement;
const exportKindSelect = document.getElementById("export-kind") as HTMLSelectElement;
const exportDpiSelect = document.getElementById("export-dpi") as HTMLSelectElement;
const exportStartButton = document.getElementById("export-start") as HTMLButtonElement;
const exportCancelButton = document.getElementById("export-cancel") as HTMLButtonElement;
const insertButton = document.getElementById("insert-pages") as HTMLButtonElement;
const extractButton = document.getElementById("extract-pages") as HTMLButtonElement;
const insertDialog = document.getElementById("insert-dialog") as HTMLDialogElement;
const insertSourceEl = document.getElementById("insert-source") as HTMLElement;
const insertAfterInput = document.getElementById("insert-after") as HTMLInputElement;
const insertRangeInput = document.getElementById("insert-range") as HTMLInputElement;
const insertStartButton = document.getElementById("insert-start") as HTMLButtonElement;
const insertCancelButton = document.getElementById("insert-cancel") as HTMLButtonElement;
const extractDialog = document.getElementById("extract-dialog") as HTMLDialogElement;
const extractRangeInput = document.getElementById("extract-range") as HTMLInputElement;
const extractStartButton = document.getElementById("extract-start") as HTMLButtonElement;
const extractCancelButton = document.getElementById("extract-cancel") as HTMLButtonElement;
const securityButton = document.getElementById("security") as HTMLButtonElement;
const certSignOpenButton = document.getElementById("cert-sign-open") as HTMLButtonElement;
const certSignDialog = document.getElementById("cert-sign-dialog") as HTMLDialogElement;
const certSignPickButton = document.getElementById("cert-sign-pick") as HTMLButtonElement;
const certSignFileEl = document.getElementById("cert-sign-file") as HTMLElement;
const certSignPassword = document.getElementById("cert-sign-password") as HTMLInputElement;
const certSignPage = document.getElementById("cert-sign-page") as HTMLInputElement;
const certSignVisible = document.getElementById("cert-sign-visible") as HTMLInputElement;
const certSignBox = document.getElementById("cert-sign-box") as HTMLElement;
const certSignX = document.getElementById("cert-sign-x") as HTMLInputElement;
const certSignY = document.getElementById("cert-sign-y") as HTMLInputElement;
const certSignW = document.getElementById("cert-sign-w") as HTMLInputElement;
const certSignH = document.getElementById("cert-sign-h") as HTMLInputElement;
const certSignReason = document.getElementById("cert-sign-reason") as HTMLInputElement;
const certSignCertify = document.getElementById("cert-sign-certify") as HTMLInputElement;
const certSignCertifyReason = document.getElementById("cert-sign-certify-reason") as HTMLElement;
const certSignStatus = document.getElementById("cert-sign-status") as HTMLElement;
const certSignFieldEl = document.getElementById("cert-sign-field") as HTMLElement;
const certSignGo = document.getElementById("cert-sign-go") as HTMLButtonElement;
const certSignCancel = document.getElementById("cert-sign-cancel") as HTMLButtonElement;
const verifyDialog = document.getElementById("verify-dialog") as HTMLDialogElement;
const verifyOpenButton = document.getElementById("verify-open") as HTMLButtonElement;
const verifySummary = document.getElementById("verify-summary") as HTMLElement;
const verifyList = document.getElementById("verify-list") as HTMLElement;
const verifyCertified = document.getElementById("verify-certified") as HTMLElement;
const verifyTrusted = document.getElementById("verify-trusted") as HTMLElement;
const verifyTrustAdd = document.getElementById("verify-trust-add") as HTMLButtonElement;
const verifyTrustClear = document.getElementById("verify-trust-clear") as HTMLButtonElement;
const verifyRefresh = document.getElementById("verify-refresh") as HTMLButtonElement;
const verifyClose = document.getElementById("verify-close") as HTMLButtonElement;
let verifyLast: Record<string, unknown> | null = null;
const batchDialog = document.getElementById("batch-dialog") as HTMLDialogElement;
/** Compare (contract-compare.md D4, §9.13-§9.14): the view beside/instead of #reader. */
const readerEl = document.getElementById("reader") as HTMLElement;
const compareView = document.getElementById("compare-view") as HTMLElement;
const compareCanvasA = document.getElementById("compare-canvas-a") as HTMLCanvasElement;
const compareCanvasB = document.getElementById("compare-canvas-b") as HTMLCanvasElement;
const compareOverlay = document.getElementById("compare-overlay") as HTMLCanvasElement;
const compareShowDiffInput = document.getElementById("compare-show-diff") as HTMLInputElement;
const compareSummaryEl = document.getElementById("compare-summary") as HTMLElement;
const compareListEl = document.getElementById("compare-list") as HTMLTableElement;
const comparePageBInput = document.getElementById("compare-page-b") as HTMLInputElement;
const compareCloseButton = document.getElementById("compare-close") as HTMLButtonElement;
const compareNameA = document.getElementById("compare-name-a") as HTMLElement;
const compareNameB = document.getElementById("compare-name-b") as HTMLElement;
type CompareText = { same: number; removed: number; added: number; refused?: string; removedLines: string[]; addedLines: string[] };
type ComparePixels = { differing: number; total: number; ratio: number; scale: number };
/** The compare slot's renderer mirror; `open`/`close` of the main document leave it alone (D1). */
let compareDoc: {
  name: string; pageCount: number; pages: PageSize[]; pageA: number; pageB: number;
  text: CompareText | null; pixels: ComparePixels | null;
} | null = null;
let compareShowDiff = false;
let compareLast: { action: "open" | "close"; ok: boolean; detail?: string } | null = null;
/** §9.12: a newer refresh drops the older one's replies; drive steps await `compareWork`. */
let compareGen = 0;
let compareWork: Promise<void> = Promise.resolve();
/** B's last render, keyed `cmp:${pageB}:${S}`: never in A's cache (§9.8). */
let compareB: { key: string; reply: CompareRenderReply } | null = null;
const batchFolderInput = document.getElementById("batch-folder") as HTMLInputElement;
const batchPickButton = document.getElementById("batch-pick") as HTMLButtonElement;
const batchKindSelect = document.getElementById("batch-kind") as HTMLSelectElement;
const batchTextInput = document.getElementById("batch-text") as HTMLInputElement;
const batchPagesInput = document.getElementById("batch-pages") as HTMLInputElement;
const batchAlignSelect = document.getElementById("batch-align") as HTMLSelectElement;
const batchBatesStartInput = document.getElementById("batch-bates-start") as HTMLInputElement;
const batchBatesPadInput = document.getElementById("batch-bates-pad") as HTMLInputElement;
const batchSummaryEl = document.getElementById("batch-summary") as HTMLElement;
const batchListEl = document.getElementById("batch-list") as HTMLTableElement;
const batchStartButton = document.getElementById("batch-start") as HTMLButtonElement;
const batchCancelButton = document.getElementById("batch-cancel") as HTMLButtonElement;
const batchCloseButton = document.getElementById("batch-close") as HTMLButtonElement;
type BatchAction = { kind: "watermark" | "header" | "footer"; text: string; pages?: string; align?: "left" | "right"; batesPad?: number };
type BatchRow = { name: string; status: "queued" | "running" | "ok" | "failed"; pages: number; detail: string; bates?: { from: number; to: number } };
/**
 * Batch (contract-batch.md D4, §3.3, §9.10-9.14): the folder listing and the run, never reset by
 * open/close. STATE `batch` appears once the dialog or a drive step has touched it.
 */
let batchState: {
  folder: string | null; out: string | null; action: BatchAction | null; files: BatchRow[];
  running: boolean; cancelled: boolean; report: string | null; bates: { start: number; next: number; pad: number };
} | null = null;
let batchCancelAfter: number | null = null;
let certSignPath: string | null = null;
let certSignInFlight = false;
/** P5 U3 §2.3: the open sign dialog's field mode (sign an existing empty field in place); null = page/rect mode. */
let certSignField: { name: string; page: number } | null = null;
/** P5 U3 §2.5: the last Verify row status per signed field name, in this document session. */
const verifyByField = new Map<string, string>();
const securityDialog = document.getElementById("security-dialog") as HTMLDialogElement;
const securityCurrentEl = document.getElementById("security-current") as HTMLElement;
const securityModeNoneInput = document.getElementById("security-mode-none") as HTMLInputElement;
const securityModeAesInput = document.getElementById("security-mode-aes") as HTMLInputElement;
const securityUserInput = document.getElementById("security-user") as HTMLInputElement;
const securityOwnerInput = document.getElementById("security-owner") as HTMLInputElement;
const securityStatusEl = document.getElementById("security-status") as HTMLElement;
const securityApplyButton = document.getElementById("security-apply") as HTMLButtonElement;
const securityCancelButton = document.getElementById("security-cancel") as HTMLButtonElement;
/** The five permission checkboxes, in the engine's fixed order. */
const permissionBoxes: [keyof DocumentPermissions, HTMLInputElement][] = [
  ["print", document.getElementById("security-perm-print") as HTMLInputElement],
  ["copy", document.getElementById("security-perm-copy") as HTMLInputElement],
  ["edit", document.getElementById("security-perm-edit") as HTMLInputElement],
  ["annotate", document.getElementById("security-perm-annotate") as HTMLInputElement],
  ["form", document.getElementById("security-perm-form") as HTMLInputElement],
];
/** The Page menu's edit-request names, and the journal step names the worker records. */
const PAGE_OPS = new Set(["rotate-right", "rotate-left", "move-up", "move-down", "delete-page"]);
/** A journal step that moved the page tree, so an undo/redo of it rebuilds the page list. */
const PAGE_STEPS = new Set(["rotate page", "delete page", "move page", "insert pages"]);
/** A journal step that changed the labels; an undo/redo of one reloads them. */
const LABEL_STEPS = new Set(["set page labels", "clear page labels"]);
/** Which mode owns each menu `edit-request` op, so the row that shows it is up first. */
const OP_MODE: Record<string, UiMode> = {
  note: "annotate", highlight: "annotate", underline: "annotate", strikeout: "annotate",
  freetext: "annotate", square: "annotate", circle: "annotate", line: "annotate",
  ink: "annotate", stamp: "annotate", sign: "annotate", shapes: "annotate",
  pages: "organize", insert: "organize", assemble: "organize", extract: "organize",
  watermark: "organize", "header-footer": "organize", labels: "organize", decorate: "organize", security: "organize",
  form: "forms", "fill-text": "forms", "sign-place": "forms",
  "content-edit": "annotate", "text-add-open": "annotate", content: "annotate", "content-object": "annotate",
  "field-add": "forms", "field-props": "forms", "field-delete": "forms", fields: "forms",
  "redact-area": "redact", "redact-search": "redact", "redact-apply-open": "redact",
  "cert-sign-open": "forms",
  redact: "redact", "redact-preview": "redact", "redact-apply": "redact",
};

const api = (window as unknown as { pdf4all?: Pdf4AllApi }).pdf4all;
let worker: Worker;
let ready: Promise<void>;
let nextId = 1;
let generation = 0;
let p0Failed = false;
let workerRestarts = 0;
let workerStopped = false;
/** Ends the startup wait of a worker that failed before it was ready. */
let failReady: ((reason: string) => void) | null = null;
let saving = false;
let unlocking = false;
/** One print job at a time: `printing` holds the lock, `printToken` cancels the loop. */
let printing = false;
let printToken = 0;

/** `untitled`: a File → New document, whose Save asks main for a path (M9: a restore does not wait on that dialog). */
type OpenDoc = { name: string; url: string | null; pageCount: number; pages: PageSize[]; rotations: number[]; info: DocumentInfo; openMs: number; untitled?: boolean };
let doc: OpenDoc | null = null;
let wantedPage = 0;
let shownPage = -1;
let shownKey: string | null = null;
let shownZoom = 0;
let inFlight: string | null = null;
let lastResult: RenderReply | null = null;

// ------------------------------------------------------------- edit state
/** The dirty/undo slice every mutation, undo, redo and saved reply carries. */
let revision = 0;
let dirty = false;
let canUndo = false;
let canRedo = false;
/** How the session proved access; the worker owns this, the renderer mirrors it. */
let auth: AuthLevel = "none";
/** What the next save will write, as the worker reports it (never a password). */
let securityState: SecurityState | null = null;
/** The scripted-field warning, shown on the row and once at the top of the panel. */
const SCRIPT_WARNING = "Runs a script this app does not execute; the value is saved as typed.";
/** Notes and other annotations on the shown page, newest reply wins. */
let notes: AnnotItem[] = [];
let notesSeq = 0;
/** The fillable fields on the shown page, newest reply wins. */
let fields: FieldItem[] = [];
let fieldsSeq = 0;
let fieldsPage = -1;
/** The document's bookmarks, and the shown page's links, newest reply wins. */
let outline: OutlineEntry[] = [];
let outlineSeq = 0;
let links: LinkEntry[] = [];
let linksSeq = 0;
/** P6 U4 §9.5: the page `links` holds, so link targets never paint another page's links. */
let linksPage = -1;
/** P6 U4 §2.2: the shown page's `/Tabs` name from the `fields` reply. */
let fieldsTabs: string | null = null;
/** P6 U4 §1.6: the rect "Draw link" drew, while its dialog is open. */
let navLinkDraft: { page: number; rect: number[] } | null = null;
/**
 * The smoke's `P2_EDIT.nav.links` slice, keyed by page index. `refreshLinks` keeps
 * the shown page current; the nav smoke fills every page once before it logs.
 */
let navLinks: Record<number, { kind: string; page?: number; uri?: string }[]> = {};
/** The rect the next link is anchored to: the search hit the dialog was opened for. */
let linkAnchor: number[] | null = null;
/** Which dialog the one link form is in: a page number or a web address. */
let linkMode: "page" | "uri" = "page";
/** The bookmark being renamed, or null when the dialog is adding one. */
let editingBookmark: number | null = null;
/** The PDF the insert dialog is about to bring pages in from (bytes live nowhere else). */
let insertSource: { name: string; bytes: ArrayBuffer } | null = null;
/** What the last insert added, for `P2_EDIT.assemble`; a refusal leaves only `last`. */
let assembleSummary: { inserted?: number; source?: string; fieldNames?: string[]; last?: { action: string; ok: boolean; detail?: string } } | null = null;
/** What the last extract wrote, for `P2_EDIT.extract`. */
let extractSummary: { pages: number[]; bytes: number } | null = null;
/** What the last export wrote, for `P2_EDIT.export`. */
let exportSummary: { kind: string; bytes: number; width?: number; height?: number; pages?: number } | null = null;
/** The page labels, newest reply wins; `labels[i]` is page i's label. */
let labels: string[] = [];
let labelsSeq = 0;
/** The document's optional-content layers, and whether a toggle is written on save. */
let layers: LayerItem[] = [];
let layersSeq = 0;
let layersPersisted = false;
/** What the last decorate smoke did, for `P2_EDIT.decorate`. */
let decorateSummary: { stamps: { watermark: number; header: number }; labels: string[]; layers: { name: string; visible: boolean }[]; layersSaved: boolean; last?: { action: string; kind?: string; pages?: number[]; ok: boolean; detail?: string; bates?: { from: number; to: number } } } | null = null;
/**
 * Bates (contract-bates.md D2, §9.13): the counter spans every document opened in this
 * window, so these are never touched by the open/close reset. `stampBates` is the range
 * the latest stamp request produced (null when it had no {bates}).
 * ponytail: undo does not roll the counter back; the dialog's start field is the override.
 */
let batesNext = 1;
let batesPad = 6;
let batesLast: { from: number; to: number; pages: number[] } | null = null;
let stampBates: { from: number; to: number } | null = null;
/** The signature the shapes smoke added, read back from the shown page's list for `P2_EDIT.sign`. */
function signFromNotes(): { name: string; subj: string } | null {
  const signed = notes.find(item => item.type === "Stamp" && item.subject === SIGNATURE_SUBJECT);
  const match = signed && /^Signed by (.*) on \d{4}-\d{2}-\d{2}$/.exec(signed.contents);
  return match ? { name: match[1], subj: signed!.subject } : null;
}
/** The ink stroke a pointer drag is recording, in page points; null when not drawing. */
let inkStroke: [number, number][] | null = null;
/** draw-signature D3: the saved drawing (pad CSS px, bounds at the origin), read from main at start-up and after every set. */
type DrawnSig = { v: 1; kind?: undefined; width: number; height: number; strokes: [number, number][][] };
/** sign-merge D2: or an image (its PNG/JPEG bytes as base64, decoded pixel size); never its file name. */
type ImageSig = { v: 1; kind: "image"; width: number; height: number; data: string };
let savedSignature: DrawnSig | ImageSig | null = null;
/** draw-signature D5/D6/D7: armed placement, the focused on-page signature, STATE `fill.lastSignature`. */
let signArmed = false;
let directFocusedSig: number | null = null;
let fillSignLast: { op: string; id: number; rect: number[] | null } | null = null;
/** draw-signature D4: the pad's finished strokes and the one being drawn, in pad CSS px. */
let padStrokes: [number, number][][] = [];
let padStroke: [number, number][] | null = null;
/** A Sign press with nothing saved opened the pad; its Save arms placement. */
let padThenArm = false;
let padFull = false;
/** sign-merge D1: the image chosen with "Use an image…" (replaces the strokes until a stroke or Clear drops it). */
let padImage: { bitmap: ImageBitmap; width: number; height: number; data: string } | null = null;
let editingNote: number | null = null;
/** A pending text-box add: the dialog is open because the user is typing its contents. */
let boxPoint: [number, number] | null = null;
/** Latest native-Open wins; a fetch that resolves late must not replace a newer one. */
const openGuard = createLatestGuard();
/** Render/search wait behind a save so their 20 s watchdog is not spent queued behind it. */
const workerGate = createGate();
let zoomMode: ZoomMode = "fit-width";
let fixedZoom = 1;
const cache = createRenderCache<RenderReply>(CACHE_LIMIT, CACHE_BYTES);
const pending = new Map<number, Pending>();

// ----------------------------------------------------------------- sidebar state

let thumbItems: { button: HTMLButtonElement; canvas: HTMLCanvasElement }[] = [];
let thumbOpen = false;
/** The user's sidebar choice: collapsed keeps the tab rail but gives the width back. */
let sidebarOpen = true;
/** At most one thumbnail request in flight, ever. */
let thumbBusy = false;
let thumbCurrent = -1;
let thumbTab = -1;
let thumbRendered = 0;
let thumbMs = 0;
let thumbSmokeDone = false;
const thumbPainted = new Set<number>();
/** Thumbnails have their own cache: a thumbnail must never evict a page render. */
const thumbCache = createRenderCache<ThumbReply>(THUMB_CACHE_ENTRIES, THUMB_CACHE_BYTES);
/** Pages whose thumbnail already failed; a failure leaves the placeholder alone. */
const thumbTried = new Set<number>();

let searchHits: SearchHit[] = [];
let searchIndex = -1;
let searchSeq = 0;
let searchTimer: number | null = null;
let searchQuery = "";
let searchTruncated = false;

/** A2 drive state. `pageIds` is fetched only on the drive path (`driven()`); the two
 * `last` records are set by the parameterized ops (contract §8.3, §8.4, §8.11). */
let pageIds: number[] | null = null;
let annotLast: { action: string; index: number; detail?: string } | null = null;
let navLast: { action?: string; index?: number; detail?: string; kind?: string; page?: number; uri?: string; rect?: number[] } | null = null;
let redactState: RedactPreview | null = null;
let redactLast: { op: string; ok: boolean; reason?: string; verified?: { text: number; pixels: number; raw: number } } | null = null;
let signLast: { op: string; ok: boolean; reason?: string; name?: string; signer?: string; revisions?: number; field?: string } | null = null;

startWorker();

void start();

async function start() {
  registerOpenPath();
  wireControls();
  wireDropZone();
  await openStartupFixture();
}

// ----------------------------------------------------------------- worker IPC

function startWorker() {
  workerStopped = false;
  worker = new Worker(new URL("./worker.js", import.meta.url), { type: "module" });
  worker.addEventListener("message", onWorkerMessage);
  worker.addEventListener("error", event => workerFailed(event.message || "worker failed to load"));
  ready = waitReady();
  ready.catch(() => {}); // requests that wait on it see the failure
}

function waitReady(): Promise<void> {
  return new Promise((resolve, reject) => {
    const timer = window.setTimeout(() => {
      reject(new Error("worker startup timed out"));
      workerFailed("worker startup timed out");
    }, REQUEST_TIMEOUT_MS);
    const listener = (event: MessageEvent<{ ready?: boolean }>) => {
      if (!event.data?.ready) return;
      clearTimeout(timer);
      worker.removeEventListener("message", listener);
      failReady = null;
      resolve();
    };
    worker.addEventListener("message", listener);
    // Its timer must not fail the restarted worker, and waiting requests fail now.
    failReady = reason => {
      clearTimeout(timer);
      reject(new Error(reason));
    };
  });
}

/** The next macrotask: a worker that answered `reason: "worker"` is replaced on one (onWorkerMessage). */
function nextTask(): Promise<void> {
  return new Promise(resolve => window.setTimeout(resolve));
}

async function request(
  message: Record<string, unknown>,
  transfer: Transferable[] = [],
  timeoutMs = REQUEST_TIMEOUT_MS,
  ownsGate = false,
  replaying = false,
) {
  const page = typeof message.page === "number" ? message.page : null;
  // M9: a read that hung the worker is not sent again automatically while the document is unchanged.
  if (!replaying && isPaused(message.op, page)) throw workerFailure("skipped", pausedText(message.op, page));
  // Only a ready worker that is not being replaced or restored gets the request. A save's gate,
  // a restart and a restore can each begin while another wait is pending, so the checks repeat
  // until all of them hold at the same moment (M9 review: an edit queued behind a hung save
  // was posted to the still-loading worker and lost).
  for (;;) {
    if (workerStopped) throw workerFailure("worker", "the PDF worker has stopped; reopen the app");
    if (!replaying && restoreBarrier) {
      await restoreBarrier;
      continue;
    }
    if (worker === failingWorker) {
      await nextTask();
      continue;
    }
    const awaited = ready;
    try {
      await awaited;
    } catch (err) {
      // A worker that failed to start: a request outside a restore waits for its replacement.
      if (!replaying && !workerStopped && ready !== awaited) continue;
      throw err;
    }
    // Work queued behind a save must not burn its watchdog waiting for it.
    if (!ownsGate) await workerGate.wait();
    if (workerStopped || ready !== awaited || worker === failingWorker || (!replaying && restoreBarrier)) continue;
    break;
  }
  // M9: the date and entropy the worker would draw are drawn here, so the log replays them;
  // what the log needs is copied now, as `transfer` detaches the request's buffers. A save
  // ticket issued before a restore is rebuilt for the restored session.
  let sent = replaying ? message : withRecordedInputs(message);
  if (!replaying && sent.op === "saved" && typeof sent.ticket === "string") sent = { ...sent, ticket: recovery.ticketNow(sent.ticket) };
  const kept = replaying ? null : recovery.keep(sent);
  return new Promise<Reply>((resolve, reject) => {
    const id = nextId++;
    const started = performance.now();
    const entry: Pending = {
      resolve, reject, timer: 0, op: sent.op, page, kept, replaying, started,
      deadline: started + timeoutMs, long: timeoutMs !== REQUEST_TIMEOUT_MS, thumb: timeoutMs === THUMB_TIMEOUT_MS, abandoned: false,
    };
    const expire = () => {
      // The worker answers in order, so the oldest unanswered request is the one it is running.
      const head = pending.values().next().value as Pending | undefined;
      if (head && head !== entry && !head.abandoned) {
        // The worker is busy with an earlier request that is still within its own limit (a long
        // job, say): this one has not started yet. It waits for that limit, then for its own.
        entry.deadline = Math.max(head.deadline, performance.now()) + timeoutMs;
        entry.timer = window.setTimeout(expire, entry.deadline - performance.now());
        return;
      }
      // A slow thumbnail is decoration and never stops the worker. Another long job (save,
      // redact-apply, batch, ...) stops it only when the document can be restored (M9): the
      // worker is stuck in it, and what it finally answers would be dropped anyway. A request
      // left running stays listed (abandoned): a later timeout then blames it, not itself.
      const stop = !entry.thumb && (!entry.long || replaying || (doc !== null && recovery.available));
      if (!stop) {
        entry.abandoned = true;
        reject(workerFailure("worker", "the request took too long and was stopped"));
        return;
      }
      pending.delete(id);
      reject(entry.long ? workerFailure("worker", "the request took too long and was stopped") : new Error("worker timed out"));
      const culprit = head && head !== entry ? head : entry;
      workerFailed("worker stopped responding", { op: culprit.op, page: culprit.page }, "slow");
    };
    entry.timer = window.setTimeout(expire, timeoutMs);
    pending.set(id, entry);
    worker.postMessage({ ...sent, id }, transfer);
  });
}

function onWorkerMessage(event: MessageEvent<Reply | ErrorReply | { ready?: boolean }>) {
  const message = event.data;
  if (!message || typeof (message as { id?: unknown }).id !== "number") return;
  const entry = pending.get((message as { id: number }).id);
  if (!entry) return;
  pending.delete((message as { id: number }).id);
  clearTimeout(entry.timer);
  // Its caller was told it took too long; the late answer only frees the worker.
  if (entry.abandoned) return;
  if ("ok" in message && message.ok === false) {
    // M9: a refused open leaves no document to restore; any other refusal changed nothing.
    if (entry.kept?.op === "open") recovery.reset();
    // The whole refusal travels with the error: `detail`, `missing`, widths.
    const failure = typeof message.reason === "string"
      ? workerFailure(message.reason, message.error || "worker error")
      : new Error(message.error || "worker error");
    entry.reject(Object.assign(failure, { reply: message }));
    // A `worker` reply is an engine that stopped after an internal error (a WASM trap) and
    // lost its sessions, so the worker restarts as a dead one does, after the callers have
    // handled this reply. Later `worker` replies from the same worker change nothing, and no
    // request is posted to it meanwhile (`failingWorker`).
    if (message.reason === "worker") {
      const stopped = worker;
      failingWorker = stopped;
      window.setTimeout(() => { if (worker === stopped && !workerStopped) workerFailed("the PDF engine hit an internal error", { op: entry.op, page: entry.page }, "trap"); });
    }
  } else {
    // A save's ticket names the state it captured: the log notes where that is.
    if (!entry.replaying && entry.op === "save") recovery.noteSave((message as { ticket?: unknown }).ticket);
    if (entry.kept) recovery.record(entry.kept, message as unknown as Record<string, unknown>, performance.now() - entry.started);
    // A change the user made went through: the session is healthy again, so the restart cap
    // (which stops a restore loop) starts over, as it does on an open.
    if (!entry.replaying && replayed(entry.op)) workerRestarts = 0;
    entry.resolve(message as Reply);
  }
}

function workerFailure(reason: string, message: string): Error & { reason: string } {
  const error = new Error(message) as Error & { reason: string };
  error.reason = reason;
  return error;
}

/**
 * The session lives in the worker, so a dead worker loses the open document. M9: when the
 * recovery log can rebuild it, `restoreDocument` reopens it in the new worker and replays the
 * changes since it was opened, without `culprit` (the request the worker was running when it
 * hung or failed) and without the changes posted after it. A failure during a restore only
 * restarts the worker: the restore sees its own request fail and falls back to the last saved
 * state, so it never loops. A read that hung is paused, so no restore issues it again.
 */
function workerFailed(reason: string, culprit: Culprit | null = null, cause: RestoreCause = "stopped") {
  failReady?.(reason);
  failReady = null;
  // The worker answers in order: the oldest unanswered request is the one it was running. A
  // crash names no request, so that one is blamed; a timeout or trap names it already.
  const running = pending.values().next().value as Pending | undefined;
  const blamed = culprit ?? (running ? { op: running.op, page: running.page } : null);
  const blamedRunning = running && blamed && running.op === blamed.op && running.page === blamed.page ? running : null;
  let lost = 0;
  for (const entry of pending.values()) {
    if (entry.abandoned || entry === blamedRunning) continue;
    if (!entry.replaying && replayed(entry.op)) lost += 1;
  }
  for (const entry of pending.values()) {
    clearTimeout(entry.timer);
    if (!entry.abandoned) entry.reject(new Error(reason));
  }
  pending.clear();
  worker.terminate();
  if (!restoreRunning && doc && blamed) pauseRead(blamed);
  const restore: RestoreSnapshot | null = !restoreRunning && doc && recovery.available
    ? { doc, wantedPage, auth, securityState, fillSave, layersPersisted, clipboardInfo, dirty, culprit: blamed, cause, lost }
    : null;
  const untracked = !restoreRunning && doc ? recovery.unavailable : null;
  clipboardInfo = null; // §9.15: the clipboard lived in the dead worker
  clearCompare(); // §9.6: so did the compare slot
  // Main keeps asking before a close or an open while the restore runs (M9 review).
  if (restore) restoreDirty = restore.dirty;
  resetDocument();
  if (workerRestarts >= MAX_WORKER_RESTARTS) {
    workerStopped = true;
    recovery.reset();
    restoreDirty = false;
    syncDirty();
    stoppedNotice = `The PDF worker stopped (${reason}) and has already been restarted ${MAX_WORKER_RESTARTS} times. Reopen the app to continue.`;
    showError(stoppedNotice);
    setStatus("The PDF worker stopped");
    return;
  }
  workerRestarts += 1;
  startWorker();
  if (restoreRunning) return;
  if (restore) {
    void restoreDocument(restore);
    return;
  }
  recovery.reset();
  const kept = untracked ? ` Your unsaved changes could not be kept: ${UNTRACKED_TEXT[untracked]}` : "";
  const notice = `The PDF worker stopped (${reason}). Restarted it (${workerRestarts} of ${MAX_WORKER_RESTARTS}); open the document again to continue.${kept}`;
  showError(notice);
  // The callers of the requests rejected above show their own errors first; this notice is the one that stays.
  const restarted = worker;
  window.setTimeout(() => { if (worker === restarted && !doc) showError(notice); });
}

// ----------------------------------------------------------------- recovery (M9)

/** Why the worker was stopped: a request past its time limit, an engine trap, or the worker itself. */
type RestoreCause = "slow" | "trap" | "stopped";

/** The request the worker was running when it hung or failed. */
type Culprit = { op: unknown; page: number | null };

/** What the window showed when the worker stopped: the restore puts it back once the replay matches. */
type RestoreSnapshot = {
  doc: OpenDoc;
  wantedPage: number;
  auth: AuthLevel;
  securityState: SecurityState | null;
  fillSave: "none" | "needs-copy" | "copied";
  layersPersisted: boolean;
  clipboardInfo: { count: number; kinds: Record<string, number> } | null;
  dirty: boolean;
  culprit: Culprit | null;
  cause: RestoreCause;
  /** Later changes that were posted but never answered: lost with the culprit. */
  lost: number;
};

/** The renderer's recovery log for the open document (engine/design/app-m9-recovery.md). */
const recovery = new RecoveryLog(recoveryBudgetFor({ deviceMemoryGiB: (navigator as { deviceMemory?: unknown }).deviceMemory }));
/** Set while a restore replays; every other request waits for it. */
let restoreBarrier: Promise<void> | null = null;
let restoreRunning = false;
/** The replay's progress while one runs, for the status line and STATE `recovery.progress`. */
let restoreProgress: { done: number; total: number } | null = null;
/** The window's dirty flag while a restore runs: main must keep asking before a close. */
let restoreDirty = false;
/** What syncDirty last told main (STATE `recovery.dirtyReported`). */
let reportedDirty = false;
/** The worker whose engine answered `reason: "worker"`: nothing is posted to it (it is replaced on the next task). */
let failingWorker: Worker | null = null;
/** Once the restart cap stopped the worker, this notice is the last word. */
let stoppedNotice: string | null = null;
/**
 * A save in place whose bytes main is writing (or wrote) but whose `saved` is not recorded yet.
 * `dialog`: main asks for a path first (Untitled, or a filled copy); a restore does not wait for it.
 */
let diskWrite: { done: Promise<boolean>; bytes: ArrayBuffer; dialog: boolean } | null = null;
/** The `generation` a restore's fallback reopen started; a save it superseded may still land. */
let fallbackGeneration = -1;
/** The last restore, for STATE `recovery.last`. */
let restoreLast: { outcome: "restored" | "reopened" | "lost"; dropped: string | null; cause: RestoreCause; lost: number; steps: number; overBudget?: boolean } | null = null;

const UNTRACKED_TEXT: Record<string, string> = {
  budget: "they were too large to keep a recovery copy.",
  clipboard: "they include a paste from another document.",
  generation: "they could not be recorded for recovery.",
};

/**
 * Reads the window issues by itself (drawing, the panels' lists). One that hung or crashed the
 * worker is paused: the window does not send it again until the user presses Try again or
 * opens a document, so neither a restore nor a later edit hangs on it a second time.
 */
const AUTO_READS = new Set<unknown>([
  "render", "thumbnail", "annots", "fields", "links", "outline", "labels", "layers", "page-ids",
  "edit-targets", "redact-preview", "sign-list",
]);
/** Paused read keys (`op` or `op:page`). */
const pausedReads = new Set<string>();
const pausedBar = document.getElementById("page-paused") as HTMLElement;
const pausedBarText = document.getElementById("page-paused-text") as HTMLElement;
const pausedRetry = document.getElementById("page-retry") as HTMLButtonElement;
pausedRetry.addEventListener("click", () => void retryPaused());

function pauseKeys(op: unknown, page: number | null): string[] {
  if (!AUTO_READS.has(op)) return [];
  // A page that does not draw does not draw as a thumbnail either.
  if ((op === "render" || op === "thumbnail") && page !== null) return [`render:${page}`, `thumbnail:${page}`];
  return [page === null ? String(op) : `${String(op)}:${page}`];
}

function pauseRead(culprit: Culprit): void {
  for (const key of pauseKeys(culprit.op, culprit.page)) pausedReads.add(key);
  renderPaused();
}

function isPaused(op: unknown, page: number | null): boolean {
  return pauseKeys(op, page).some(key => pausedReads.has(key));
}

/** The refusal a paused read gets: it names what is not done, for showing, printing and exporting alike. */
function pausedText(op: unknown, page: number | null): string {
  return (op === "render" || op === "thumbnail") && page !== null
    ? `Page ${page + 1} could not be drawn because it took too long, so it is not shown, printed or exported. Choose Try again, or open the document again.`
    : `${opInfo(op)?.label ?? "This"} took too long, so it is paused. Choose Try again, or open the document again.`;
}

/** The notice bar with Try again, shown while anything is paused. */
function renderPaused(): void {
  const pages = [...pausedReads].filter(key => key.startsWith("render:")).map(key => Number(key.slice(7)) + 1);
  pausedBar.hidden = pausedReads.size === 0;
  pausedBarText.textContent = pages.length
    ? `${pages.length === 1 ? `Page ${pages[0]} is` : `Pages ${pages.join(", ")} are`} not drawn: drawing took too long.`
    : pausedReads.size ? "Part of this document was not read: it took too long." : "";
}

/** Try again: the paused reads are sent once more (a hang then restores and pauses them again). */
async function retryPaused(): Promise<void> {
  if (!pausedReads.size) return;
  for (const key of pausedReads) if (key.startsWith("thumbnail:")) thumbTried.delete(Number(key.slice(10)));
  pausedReads.clear();
  renderPaused();
  if (!doc) return;
  shownKey = null;
  void refreshNotes();
  void refreshFields();
  void refreshLinks();
  void refreshOutline();
  void refreshLabels();
  void refreshLayers();
  await drain();
  pumpThumbs();
}

/** The local date the worker would stamp on an annotation (engine.ts `signatureContents`). */
function localDate(now = new Date()): string {
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

/**
 * The request with what the engine would otherwise draw for itself: `annot-add`'s local date
 * and `redact-apply`'s 32 bytes of entropy (engine-rust.ts `wire`). Recorded with the request,
 * they make a replay write what the original wrote.
 */
function withRecordedInputs(message: Record<string, unknown>): Record<string, unknown> {
  if (message.op === "annot-add" && message.today === undefined) return { ...message, today: localDate() };
  if (message.op === "redact-apply" && message.entropy === undefined) {
    return { ...message, entropy: crypto.getRandomValues(new Uint8Array(32)).buffer };
  }
  return message;
}

type RestoreOutcome = "restored" | "reopened" | "lost";

/** The notice after a restart: what was stopped, then what was kept. */
function restoreNotice(snap: RestoreSnapshot, outcome: RestoreOutcome, savedEver: boolean, overBudget = false): string {
  const op = snap.culprit?.op;
  const label = opInfo(op)?.label;
  const first = snap.cause === "slow" && label ? `${label} took too long, so it was stopped.`
    : snap.cause === "trap" && label ? `${label} failed inside the PDF engine, so it was stopped.`
      : label ? `${label} stopped the PDF engine unexpectedly.` : "The PDF engine stopped unexpectedly.";
  const save = op === "save" || op === "validate" ? " Nothing was saved." : "";
  const later = snap.lost ? ` ${snap.lost === 1 ? "A change made after it was" : `${snap.lost} changes made after it were`} not applied either.` : "";
  const page = snap.culprit?.page;
  const paused = !AUTO_READS.has(op) ? ""
    : (op === "render" || op === "thumbnail") && typeof page === "number"
      ? ` Page ${page + 1} was not drawn again; choose Try again to draw it.`
      : " It was not tried again; choose Try again to retry it.";
  if (outcome === "restored") {
    const kept = replayed(op) ? " That change was not applied; your other changes were restored." : " Your document and changes were restored.";
    return `${first}${save}${kept}${later}${paused}`;
  }
  const why = overBudget ? " Restoring your changes took too long, so the document was reopened" : " Your changes since the last save could not be restored, so the document was reopened";
  if (outcome === "reopened") return `${first}${save}${why} ${savedEver ? "as last saved" : "as it was opened"}.${paused}`;
  return `${first}${save} Your unsaved changes could not be restored; open the document again to continue.`;
}

/**
 * Reopens the document in the restarted worker and replays its log (`replay` checks every
 * answer against the original, within a time budget scaled to the log). On a mismatch, a
 * refusal, another stop or the budget, the document is reopened from what is on disk instead:
 * the save in place being written if one is, else the last saved copy (or the opened bytes when
 * it was never saved here). Never a half-replayed document, never older than the disk.
 */
async function restoreDocument(snap: RestoreSnapshot): Promise<void> {
  restoreRunning = true;
  let release = () => {};
  restoreBarrier = new Promise<void>(resolve => { release = resolve; });
  const startedGeneration = generation;
  const hadChanges = recovery.entries.length > 0;
  let savedEver = recovery.saved !== null;
  let overBudget = false;
  let fallbackBytes: ArrayBuffer | null = null;
  setStatus(`Restoring ${snap.doc.name}…`);
  let result: ReplayResult | null = null;
  try {
    result = await replay(recovery, (message, long) =>
      request(message, [], long ? SAVE_TIMEOUT_MS : REQUEST_TIMEOUT_MS, true, true) as unknown as Promise<Record<string, unknown>>, {
      onStep: (done, total) => {
        restoreProgress = { done, total };
        if (total >= 20) setStatus(`Restoring ${snap.doc.name}… ${done} of ${total} changes`);
      },
    });
    // Save tickets issued before the restart are rebuilt for this session from now on.
    recovery.restored();
  } catch (err) {
    overBudget = err instanceof ReplayOverBudget;
    console.warn(`recovery: replay failed (${messageOf(err)})`);
    // An engine that trapped is replaced on the next task: let that restart happen inside the
    // restore, so the fallback below opens in the new worker (M9 review).
    while (worker === failingWorker && !workerStopped) await nextTask();
    // A save in place main is writing now decides what is on disk: wait for it. One that
    // first asks the user for a path is not waited for (saveInPlace handles it if it lands).
    const write = diskWrite;
    if (write && !write.dialog && await write.done) {
      fallbackBytes = write.bytes;
      savedEver = true;
    }
  } finally {
    restoreRunning = false;
    restoreBarrier = null;
    restoreProgress = null;
    release();
  }
  const settleDirty = () => {
    restoreDirty = false;
    syncDirty();
  };
  // A newer open took over while the replay ran, or the restart cap stopped the worker.
  if (startedGeneration !== generation || workerStopped) {
    settleDirty();
    return;
  }
  const dropped = typeof snap.culprit?.op === "string" ? snap.culprit.op : null;
  if (result) {
    restoreLast = { outcome: "restored", dropped, cause: snap.cause, lost: snap.lost, steps: result.steps };
    restoreDirty = false;
    await showRestored(snap, result);
    return;
  }
  const bytes = fallbackBytes ?? recovery.fallback();
  const outcome: RestoreOutcome = !bytes ? "lost" : hadChanges || fallbackBytes ? "reopened" : "restored";
  restoreLast = { outcome, dropped, cause: snap.cause, lost: snap.lost, steps: 0, ...(overBudget ? { overBudget } : {}) };
  if (!bytes) {
    recovery.reset();
    settleDirty();
    // The worker may hold a half-replayed document: close it.
    try { await request({ op: "close" }); } catch { /* a dead worker holds nothing */ }
    showError(restoreNotice(snap, "lost", savedEver));
    return;
  }
  const notice = restoreNotice(snap, outcome, savedEver, overBudget);
  // openDocument starts a new log from these bytes: the reopened state is the new base.
  restoreDirty = false;
  fallbackGeneration = generation + 1;
  await openDocument(snap.doc.name, bytes, snap.doc.url, true);
  if (!doc) return; // the open failed and said why
  if (snap.doc.untitled && doc.url === snap.doc.url) doc.untitled = true;
  if (snap.fillSave === "copied") fillSave = "copied"; // the window still edits the "(filled)" copy it saved
  showError(notice);
  announce(notice);
}

/** The replay matched: the window shows the document as it was, at the page it was on. */
async function showRestored(snap: RestoreSnapshot, result: ReplayResult): Promise<void> {
  const journal = result.journal as Partial<JournalState>;
  doc = snap.doc;
  revision = typeof journal.revision === "number" ? journal.revision : 0;
  dirty = journal.dirty === true;
  canUndo = journal.canUndo === true;
  canRedo = journal.canRedo === true;
  auth = snap.auth;
  securityState = snap.securityState;
  fillSave = snap.fillSave;
  layersPersisted = snap.layersPersisted;
  // The clipboard came back only if the copy that filled it was replayed.
  clipboardInfo = recovery.clipboardInLog ? snap.clipboardInfo : null;
  document.title = `${doc.name} — Tandem PDF`;
  documentTitleEl.textContent = doc.name;
  syncDirty();
  renderBanner(doc.info);
  wantedPage = Math.max(0, Math.min(snap.wantedPage, doc.pageCount - 1));
  shownPage = -1;
  shownKey = null;
  refreshThumbs();
  void refreshNotes();
  void refreshFields();
  void refreshOutline();
  void refreshLinks();
  void refreshLabels();
  void refreshLayers();
  void refreshRedactPreview();
  syncControls();
  const notice = restoreNotice(snap, "restored", false);
  showError(notice);
  announce(notice);
  setStatus(`Restored ${doc.name}`);
  if (doc.info.needsPassword) showPasswordForm();
  else await drain();
}

// ----------------------------------------------------------------- documents

/** Drops every trace of the open document; the bytes themselves live in the worker. */
function resetDocument() {
  doc = null;
  wantedPage = 0;
  shownPage = -1;
  shownKey = null;
  shownZoom = 0;
  inFlight = null;
  lastResult = null;
  cache.clear();
  resetThumbs();
  setThumbSidebar(true); // a closed document shows the sidebar and its empty line
  resetEditState();
  extracted.textContent = "";
  textEmptyEl.hidden = false;
  documentTitleEl.textContent = "Tandem PDF";
  searchInput.value = "";
  clearSearch();
  clearWarning();
  hidePasswordForm();
}

/** No document, nothing dirty, no history: what a fresh open starts from. */
function resetEditState() {
  revision = 0;
  contentEdited = null; // debt1 row 19: STATE content never carries the last document's edit
  contentRefusals = [];
  dirty = false;
  canUndo = false;
  canRedo = false;
  notes = [];
  notesSeq += 1;
  fields = [];
  fieldsPage = -1;
  fieldsTabs = null;
  linksPage = -1;
  navLinkDraft = null;
  directFocusedLink = null;
  if (directActive) removeDirect(directActive); // a new document never inherits the last one's editor
  directActive = null;
  directFocusedField = null;
  directFieldIndex = -1;
  directFocusedBox = null; // fill-anywhere D7: a new or closed document is not filled
  fillLast = null;
  signArmed = false; fillSignButton.setAttribute("aria-pressed", "false"); fillSignLast = null; directFocusedSig = null; // draw-signature D5/D7
  fillSave = "none";
  fieldsSeq += 1;
  fieldDesign = null;
  outline = [];
  outlineSeq += 1;
  links = [];
  linksSeq += 1;
  navLinks = {};
  linkAnchor = null;
  editingBookmark = null;
  insertSource = null;
  assembleSummary = null;
  extractSummary = null;
  pageIds = null;
  annotLast = null;
  navLast = null;
  redactState = null;
  redactLast = null;
  certSignField = null;
  verifyByField.clear();
  exportSummary = null;
  labels = [];
  labelsSeq += 1;
  layers = [];
  layersSeq += 1;
  layersPersisted = false;
  decorateSummary = null;
  editingNote = null;
  boxPoint = null;
  // A new document starts with no proven access and no pending security change.
  auth = "none";
  securityState = null;
  noteDialog.close();
  bookmarkDialog.close();
  linkDialog.close();
  insertDialog.close();
  extractDialog.close();
  exportDialog.close();
  stampDialog.close();
  labelsDialog.close();
  securityDialog.close();
  redactDialog.close();
  renderRedactState();
  renderNotes();
  renderFields();
  renderOutline();
  renderLinks();
  renderLayers();
  announceNav("");
  pageStatusEl.textContent = "";
  syncDirty();
}

/** main hands the document over as a one-shot `app://pdf4all/open/<token>` URL (contract §1). */
function registerOpenPath() {
  if (api && typeof api.onDocument === "function") {
    api.onDocument(incoming => {
      if (!incoming || typeof incoming.name !== "string" || typeof incoming.url !== "string") return;
      const { name, url } = incoming;
      driven(() => openFromUrl(name, url, incoming.untitled === true));
    });
  }
  if (api && typeof api.onOpenError === "function") {
    api.onOpenError(error => {
      driven(() => undefined);
      if (error.reason === "cancelled") {
        clearError();
        return;
      }
      const limit = error.limit ? `${(error.limit / (1024 * 1024)).toFixed(0)} MiB` : "the size";
      showError(error.reason === "too-large"
        ? `${error.name ?? "That file"} is larger than the ${limit} limit.`
        : `Could not read ${error.name ?? "that file"}.`);
    });
  }
  if (api && typeof api.onSaveRequest === "function") {
    api.onSaveRequest(payload => {
      // The File menu sends `{inPlace:true}` for Save; anything else is Save a copy.
      driven(() => payload && payload.inPlace ? saveInPlace() : saveAs());
    });
  }
  if (api && typeof api.onEditRequest === "function") {
    api.onEditRequest(payload => {
      const op = payload && typeof payload.op === "string" ? payload.op : "";
      // A missing contents field is what tells the menu path from the smoke path.
      const contents = payload && typeof payload.contents === "string" ? payload.contents : null;
      // A menu op must land in the mode whose row shows it, before it arms anything.
      // sign-merge D5: a bare `sign` (old menu op) is a Place Signature press, so Fill forms; a certificate `sign` keeps its mode.
      const wantedMode = op === "sign" && contents === null ? "forms" : OP_MODE[op];
      if (wantedMode && wantedMode !== uiMode) setMode(wantedMode);
      driven(() => runEditRequest(op, contents));
    });
  }
  if (api && typeof api.onPrintRequest === "function") {
    api.onPrintRequest(payload => {
      // `--print-range` reaches the renderer verbatim; only parsePageRange reads it.
      const range = payload && typeof payload.range === "string" ? payload.range : undefined;
      driven(() => range === undefined ? openPrintDialog() : startPrint(range));
    });
  }
}

/**
 * Requests are serialized so a `--drive` script's `state` op sees the previous op
 * finished; the STATE line after each one is what the driver waits for.
 */
let driveQueue: Promise<unknown> = Promise.resolve();
function driven(work: () => unknown): void {
  // A2 §8.7: the page ids are gathered once per request, on the drive's STATE path only
  // (never from a UI refresh), so `pages.ids` shows a move or delete right after it lands.
  driveQueue = driveQueue.then(work).catch(() => undefined).then(refreshPageIds).then(logState);
}

/** The current page object numbers, or null when there is nothing to ask. */
async function refreshPageIds(): Promise<void> {
  if (!doc || doc.info.needsPassword) {
    pageIds = null;
    return;
  }
  try {
    const reply = (await request({ op: "page-ids" })) as PageIdsReply;
    pageIds = reply.ids;
  } catch {
    // A dead worker or a closing document: stale ids would lie, so report none.
    pageIds = null;
  }
}

// ---------------------------------------------------------------- A2 drive ops

/** `goto {page}`: 0-based, refused before `goTo` (which clamps silently), §8.6. */
async function driveGoto(contents: string | null): Promise<void> {
  clearError();
  const wanted = driveWanted(contents);
  if (!wanted || !doc || doc.info.needsPassword) return;
  const page = wanted.page;
  if (!Number.isInteger(page) || (page as number) < 0 || (page as number) >= doc.pageCount) {
    showError(`goto: page must be an integer between 0 and ${doc.pageCount - 1}`);
    return;
  }
  goTo(page as number);
  await Promise.all([refreshNotes(), refreshFields(), refreshLinks(), uiMode === "edit" ? refreshContentTargets(true) : Promise.resolve()]);
  await compareWork;
}

/** `zoom {mode}` or `{scale}`; a scale must be one of `ZOOM_STEPS` (§8.16). */
async function driveZoom(contents: string | null): Promise<void> {
  clearError();
  const wanted = driveWanted(contents);
  if (!wanted || !doc || doc.info.needsPassword) return;
  if (typeof wanted.mode === "string") {
    if (wanted.mode !== "fit-width" && wanted.mode !== "fit-page") {
      showError("zoom: mode must be fit-width or fit-page");
      return;
    }
    setZoom(wanted.mode);
    await drain();
    return;
  }
  const scale = wanted.scale;
  if (typeof scale !== "number" || !ZOOM_STEPS.includes(scale)) {
    showError(`zoom: scale must be one of ${ZOOM_STEPS.join(", ")}`);
    return;
  }
  setZoom("fixed", scale);
  await drain();
}

/** `find {query}` or `{dir: next|prev}`: the toolbar search handlers (§8.14). */
async function driveFind(contents: string | null): Promise<void> {
  clearError();
  const wanted = driveWanted(contents);
  if (!wanted || !doc || doc.info.needsPassword) return;
  if (typeof wanted.query === "string") {
    searchInput.value = wanted.query;
    await startSearch();
    return;
  }
  if (wanted.dir === "next") moveHit(1);
  else if (wanted.dir === "prev") moveHit(-1);
  else showError("find: send a query, or a dir of next/prev");
}

/** `tab {id}`: the inspector tablist's own click handler, unknown ids refused (§8.13). */
function driveTab(contents: string | null): void {
  clearError();
  const wanted = driveWanted(contents);
  if (!wanted) return;
  const id = wanted.id;
  const tab = typeof id === "string" ? inspectorTabs.find(item => item.id === id) : undefined;
  if (!tab) {
    showError(`tab: no inspector tab named ${typeof id === "string" ? id : ""}`);
    return;
  }
  tab.click();
}

/** `annot {action: set|delete, index}`: the Comments row's own edit/delete calls (§8.5). */
async function driveAnnot(contents: string | null): Promise<void> {
  clearError();
  const wanted = driveWanted(contents);
  if (!wanted || !doc || doc.info.needsPassword) return;
  const action = wanted.action === "set" ? "set" : wanted.action === "delete" ? "delete" : null;
  const at = typeof wanted.index === "number" && Number.isInteger(wanted.index) ? wanted.index : -1;
  if (!action) {
    annotLast = { action: String(wanted.action ?? ""), index: at, detail: "invalid-action" };
    return;
  }
  // §8.5: the list is refreshed before the index is resolved, so an undo/redo/goto
  // that re-listed asynchronously cannot point the index at the previous rows.
  await refreshNotes();
  const item = notes[at];
  if (!item) {
    annotLast = { action, index: at, detail: "not-found" };
    return;
  }
  if (!item.editable && !(action === "delete" && item.type === "Redact")) {
    annotLast = { action, index: at, detail: "not-editable" };
    return;
  }
  if (action === "set") await editNote(item.id, String(wanted.contents ?? "").slice(0, MAX_NOTE_CHARS));
  else await deleteNote(item.id, item.type === "Redact" ? "Deleted redaction mark" : undefined);
  annotLast = { action, index: at };
  await refreshNotes();
}

/** `underline` / `strikeout`: markup on the current search match, no implicit query (§8.12). */
async function driveMarkup(type: "Underline" | "StrikeOut"): Promise<void> {
  clearError();
  const hit = currentHit();
  if (!hit) {
    annotLast = { action: type.toLowerCase(), index: -1, detail: "no-anchor" };
    announce("Find something first");
    return;
  }
  await addMarkup(hit.page, type, hit.quads, type === "Underline" ? "Underlined match" : "Struck out match");
}

/** `insert {after, range}` with the line's `from`: the insert dialog's own save path (§8.2). */
async function driveInsert(contents: string | null): Promise<void> {
  clearError();
  const wanted = driveWanted(contents);
  if (!wanted || !doc || !canEdit()) return;
  const picked = await pickInsertSource();
  if (!picked) {
    assembleSummary = { ...(assembleSummary ?? {}), last: { action: "insert", ok: false, detail: "unreadable" } };
    return;
  }
  openInsertDialog(picked);
  const after = wanted.after;
  insertAfterInput.value = typeof after === "number" || typeof after === "string" ? String(after) : "";
  insertRangeInput.value = typeof wanted.range === "string" ? wanted.range : "";
  await saveInsertDialog();
  if (insertDialog.open) insertDialog.close();
}

/** `watermark` / `header-footer` with a payload: the stamp dialog's own save path (§8.3). */
async function driveStamp(contents: string | null): Promise<void> {
  clearError();
  const wanted = driveWanted(contents);
  if (!wanted || !doc || !canEdit()) return;
  const kind = wanted.kind;
  if (kind !== "watermark" && kind !== "header" && kind !== "footer") {
    decorateLast({ action: "stamp", ok: false, detail: "invalid-kind" });
    return;
  }
  openStampDialog(kind);
  if (typeof wanted.text === "string") stampTextInput.value = wanted.text;
  stampPagesInput.value = typeof wanted.pages === "string" ? wanted.pages : "";
  // §9.11: present keys go into the fields verbatim, so the dialog's check is the only check.
  if ("batesStart" in wanted) stampBatesStartInput.value = String(wanted.batesStart);
  if ("batesPad" in wanted) stampBatesPadInput.value = String(wanted.batesPad);
  const pages = parsePageRange(stampPagesInput.value, doc.pageCount);
  stampBates = null;
  let ok = false;
  if ("align" in wanted && wanted.align !== "left" && wanted.align !== "right") {
    showError("Align must be left or right");
  } else {
    if (wanted.align === "left" || wanted.align === "right") stampAlignSelect.value = wanted.align;
    ok = await saveStampDialog();
  }
  if (stampDialog.open) stampDialog.close();
  decorateLast({ action: "stamp", kind, ...(pages ? { pages } : {}), ok, ...(ok && stampBates ? { bates: stampBates } : {}) });
}

/** `labels {from, style, prefix, start}` or `{clear, from}`: the dialog's own save (§8.9). */
async function driveLabels(contents: string | null): Promise<void> {
  clearError();
  const wanted = driveWanted(contents);
  if (!wanted || !doc || !canEdit()) return;
  openLabelsDialog();
  if (typeof wanted.from === "number" || typeof wanted.from === "string") labelsFromInput.value = String(wanted.from);
  if (wanted.clear === true) {
    const ok = await clearLabels();
    if (labelsDialog.open) labelsDialog.close();
    await refreshLabels();
    decorateLast({ action: "labels-clear", ok });
    return;
  }
  const style = typeof wanted.style === "string" ? wanted.style : "";
  if (![...labelsStyleSelect.options].some(option => option.value === style)) {
    // §8.9: checked against the select before anything is written, so the label list cannot move.
    labelsDialog.close();
    decorateLast({ action: "labels-set", ok: false, detail: "invalid-style" });
    return;
  }
  labelsStyleSelect.value = style;
  if (typeof wanted.prefix === "string") labelsPrefixInput.value = wanted.prefix;
  if (wanted.start !== undefined) labelsStartInput.value = String(wanted.start);
  const ok = await saveLabelsDialog();
  if (labelsDialog.open) labelsDialog.close();
  await refreshLabels();
  decorateLast({ action: "labels-set", ok });
}

/** `link {kind, page|uri}` / `{action: delete, index}`: a prior find hit is required (§8.11). */
async function driveLink(contents: string | null): Promise<void> {
  clearError();
  const wanted = driveWanted(contents);
  if (!wanted || !doc || !canAnnotate()) return;
  const at = typeof wanted.index === "number" && Number.isInteger(wanted.index) ? wanted.index : -1;
  if (wanted.action === "delete") {
    await refreshLinks();
    const item = links[at];
    if (!item) {
      navLast = { action: "link-delete", index: at, detail: "no-such-link" };
      return;
    }
    await deleteLink(item.id);
    await refreshLinks();
    navLast = { action: "link-delete", index: at };
    return;
  }
  if (wanted.action === "set") {
    // P6 U4 §9.10: by the row id; the request goes by that row's annotId; page is 1-based.
    await refreshLinks();
    const item = links.find(entry => entry.id === at);
    if (!item) {
      navLast = { action: "link-set", index: at, detail: "no-such-link" };
      showError(`link: no link ${at} on this page`);
      return;
    }
    const change: { rect?: unknown; target?: { page: number } | { uri: string } } = {};
    if (wanted.rect !== undefined) change.rect = wanted.rect;
    if (typeof wanted.page === "number") change.target = { page: wanted.page - 1 };
    else if (typeof wanted.uri === "string") change.target = { uri: wanted.uri };
    await linkSet(wantedPage, item, change);
    return;
  }
  const kind = wanted.kind;
  // P6 U4 §1.6: a drawn rect (its dialog still open) is the anchor; otherwise the find hit, as before.
  if (navLinkDraft !== null && linkDialog.open) {
    if (kind !== "page" && kind !== "uri") {
      navLast = { action: "link-add", detail: "invalid-kind" };
      return;
    }
    setLinkMode(kind, false);
    linkTargetInput.value = kind === "page" ? String(wanted.page ?? "") : String(wanted.uri ?? "");
    const ok = await saveLinkDialog();
    closeLinkDialog();
    navLast = { action: "link-add", ...(ok ? {} : { detail: "invalid" }) };
    return;
  }
  const hit = currentHit();
  const rect = hit ? hitRect(hit) : null;
  if (!hit || !rect) {
    navLast = { action: "link-add", detail: "no-anchor" };
    announceNav("Find something first");
    return;
  }
  if (kind !== "page" && kind !== "uri") {
    navLast = { action: "link-add", detail: "invalid-kind" };
    return;
  }
  openLinkDialog(kind);
  linkTargetInput.value = kind === "page" ? String(wanted.page ?? "") : String(wanted.uri ?? "");
  const ok = await saveLinkDialog();
  closeLinkDialog();
  navLast = { action: "link-add", ...(ok ? {} : { detail: "invalid" }) };
}

async function driveDirectClick(contents: string | null): Promise<void> {
  const value = driveWanted(contents); if (!value || !doc) return;
  if (typeof value.page !== "number" || typeof value.x !== "number" || typeof value.y !== "number") { showError("click: page, x and y are required"); return; }
  if (!Number.isInteger(value.page) || value.page < 0 || value.page >= doc.pageCount) { showError(`click: page must be an integer between 0 and ${doc.pageCount - 1}`); return; }
  goTo(value.page); await drain();
  while (doc && (shownPage !== value.page || inFlight !== null || fieldsPage !== value.page || linksPage !== value.page)) await new Promise(resolve => window.setTimeout(resolve, 10));
  const size = doc.pages[shownPage]; if (!size || size.height < 1) return;
  // §8.3: scroll the point into view, then hit-test. elementFromPoint, not a name lookup,
  // so an armed tool (pointer-events:none on the layer) receives the click.
  const canvasTop = canvas.getBoundingClientRect().top - viewport.getBoundingClientRect().top + viewport.scrollTop;
  viewport.scrollTop = Math.max(0, canvasTop + (value.y / size.height) * (canvas.clientHeight || 0) - viewport.clientHeight / 2);
  await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
  await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
  const box = canvas.getBoundingClientRect(); if (box.width < 1) return;
  const x = box.left + value.x * box.width / size.width, y = box.top + value.y * box.height / size.height;
  void document.body.offsetHeight;
  void hitLayer.offsetHeight;
  const target = document.elementsFromPoint(x, y)[0] as HTMLElement | undefined;
  if (!target) return;
  const placingNote = noteAddButton.getAttribute("aria-pressed") === "true", noteCount = notes.length;
  const clickTarget = placingNote && hitLayer.contains(target) ? pageRegion : target;
  if (hitLayer.contains(clickTarget)) clickTarget.focus();
  directClickWork = null;
  clickTarget.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: x, clientY: y, pointerId: 1, pointerType: "mouse", altKey: value.alt === true, shiftKey: value.shift === true }));
  // §9.9: a press on the page outside the layer moves focus to #page, so an open editor blurs and commits first.
  if (!hitLayer.contains(clickTarget) && pageRegion.contains(clickTarget)) {
    directFinishWork = null;
    const open = directActive;
    pageRegion.focus({ preventScroll: true });
    let work = directFinishWork as Promise<void> | null;
    if (!work && open && directActive === open) work = finishDirect(); // no blur event in an unfocused window
    if (work) await work;
    directFinishWork = null;
  }
  clickTarget.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, clientX: x, clientY: y, pointerId: 1, pointerType: "mouse", altKey: value.alt === true, shiftKey: value.shift === true }));
  clickTarget.dispatchEvent(new MouseEvent("click", { bubbles: true, clientX: x, clientY: y, button: 0, altKey: value.alt === true, shiftKey: value.shift === true }));
  if (directClickWork) { const work = directClickWork; directClickWork = null; await work; }
  if (placingNote) for (let i = 0; i < 500 && notes.length === noteCount; i++) await new Promise(resolve => window.setTimeout(resolve, 10));
}
/** P6 U5 §2.5/§9.16: `menu {page, x, y, item?}` — a right-click on the target under the point, then optionally one item. */
async function driveObjectMenu(contents: string | null): Promise<void> {
  const value = driveWanted(contents); if (!value || !doc) return;
  if (typeof value.page !== "number" || typeof value.x !== "number" || typeof value.y !== "number") { showError("menu: page, x and y are required"); return; }
  if (!Number.isInteger(value.page) || value.page < 0 || value.page >= doc.pageCount) { showError(`menu: page must be an integer between 0 and ${doc.pageCount - 1}`); return; }
  // P6 U6 §9.15: drive item names → menu keys.
  const MENU_ITEMS: Record<string, ObjectMenuItem> = { cut: "cut", copy: "copy", paste: "paste", replace: "replace", duplicate: "duplicate", "rotate-cw": "rotateCw", "rotate-ccw": "rotateCcw", "flip-h": "flipH", "flip-v": "flipV", crop: "crop", delete: "delete" };
  const wanted = value.item;
  if (wanted !== undefined && (typeof wanted !== "string" || !Object.hasOwn(MENU_ITEMS, wanted))) { showError("menu: item must be cut, copy, paste, replace, duplicate, rotate-cw, rotate-ccw, flip-h, flip-v, crop or delete"); return; }
  closeObjectMenu(false); // as the outside press of a real right-click would
  goTo(value.page); await drain();
  while (doc && (shownPage !== value.page || inFlight !== null || fieldsPage !== value.page || linksPage !== value.page)) await new Promise(resolve => window.setTimeout(resolve, 10));
  const size = doc.pages[shownPage]; if (!size || size.height < 1) return;
  const canvasTop = canvas.getBoundingClientRect().top - viewport.getBoundingClientRect().top + viewport.scrollTop;
  viewport.scrollTop = Math.max(0, canvasTop + (value.y / size.height) * (canvas.clientHeight || 0) - viewport.clientHeight / 2);
  await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
  await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
  const box = canvas.getBoundingClientRect(); if (box.width < 1) return;
  const x = box.left + value.x * box.width / size.width, y = box.top + value.y * box.height / size.height;
  void document.body.offsetHeight;
  void hitLayer.offsetHeight;
  const target = document.elementsFromPoint(x, y).find(node => hitLayer.contains(node)) as HTMLElement | undefined;
  // P6 U7 §9.28: empty page space opens the page menu; the drive hands it the exact page point (client px are whole numbers).
  driveMenuPoint = [value.x, value.y];
  try { (target ?? pageRegion).dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: x, clientY: y, button: 2 })); }
  finally { driveMenuPoint = null; }
  if (wanted === undefined) return;
  const item = objectMenuButtons[MENU_ITEMS[wanted as string]];
  if (objectMenu.hidden || item.disabled) { showError(`menu: ${wanted} is not available`); return; }
  item.click();
  if (directGestureWork) await directGestureWork;
}
async function driveDirectGesture(op: "drag" | "handle", contents: string | null): Promise<void> {
  const value = driveWanted(contents);
  if (!value || !doc || typeof value.page !== "number" || typeof value.x !== "number" || typeof value.y !== "number") return;
  goTo(value.page); await drain();
  // P6 U4 §9.15: wait as a click does (fields and links painted), scroll the point into view, then read the box.
  while (doc && (shownPage !== value.page || inFlight !== null || fieldsPage !== value.page || linksPage !== value.page)) await new Promise(resolve => window.setTimeout(resolve, 10));
  const size = doc.pages[value.page]; if (!size || size.height < 1) return;
  const canvasTop = canvas.getBoundingClientRect().top - viewport.getBoundingClientRect().top + viewport.scrollTop;
  viewport.scrollTop = Math.max(0, canvasTop + (value.y / size.height) * (canvas.clientHeight || 0) - viewport.clientHeight / 2);
  await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
  await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
  const box = canvas.getBoundingClientRect(); if (box.width < 1) return;
  const x = box.left + value.x * box.width / size.width, y = box.top + value.y * box.height / size.height;
  const target = document.elementsFromPoint(x, y).find(node => hitLayer.contains(node)) as HTMLElement | undefined;
  if (op === "drag" && !(layerLive() && target?.closest(".hit-target, .hit-handle"))) {
    // Empty space (or an armed tool): the page takes the press, the window the move and release (Draw link, area marks).
    const endX = x + (typeof value.dx === "number" ? value.dx * box.width / size.width : 0), endY = y + (typeof value.dy === "number" ? value.dy * box.height / size.height : 0);
    const init = { bubbles: true, cancelable: true, pointerId: 1, pointerType: "mouse", button: 0, shiftKey: value.shift === true };
    directGestureWork = null;
    pageRegion.dispatchEvent(new PointerEvent("pointerdown", { ...init, clientX: x, clientY: y }));
    window.dispatchEvent(new PointerEvent("pointermove", { ...init, clientX: endX, clientY: endY }));
    window.dispatchEvent(new PointerEvent("pointerup", { ...init, clientX: endX, clientY: endY }));
    if (directGestureWork) await directGestureWork;
    return;
  }
  const fieldTarget = target?.closest<HTMLElement>(".hit-target") ?? [...hitLayer.querySelectorAll<HTMLElement>(".hit-target")].find(node => { const rect = node.getBoundingClientRect(); return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom; });
  // P6 U6 §9.13: a handle on a group member resolves against the union's `.group-box`.
  const owner = target?.closest<HTMLElement>(".group-box") ?? (fieldTarget?.classList.contains("multi") ? hitLayer.querySelector<HTMLElement>(".group-box") : fieldTarget);
  const handle = op === "handle" ? owner?.querySelector<HTMLElement>(`[data-handle="${CSS.escape(String(value.handle ?? "grip"))}"]`) : target;
  if (!handle) return;
  const start = op === "handle" ? handle.getBoundingClientRect() : null;
  const startX = start ? start.left + start.width / 2 : x, startY = start ? start.top + start.height / 2 : y;
  const endX = startX + (typeof value.dx === "number" ? value.dx * box.width / size.width : 0);
  const endY = startY + (typeof value.dy === "number" ? value.dy * box.height / size.height : 0);
  const shiftKey = value.shift === true, altKey = value.alt === true, metaKey = value.meta === true;
  const fire = (element: EventTarget, type: string, clientX: number, clientY: number) => element.dispatchEvent(new PointerEvent(type, { bubbles: true, clientX, clientY, pointerId: 1, pointerType: "mouse", shiftKey, altKey, metaKey }));
  handle.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: startX, clientY: startY, pointerId: 1, pointerType: "mouse", shiftKey, altKey, metaKey }));
  fire(window, "pointermove", endX, endY);
  if (value.escape === true) window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  fire(window, "pointerup", endX, endY);
  if (directGestureWork) await directGestureWork;
}
function driveDirectType(contents: string | null): void {
  const value = driveWanted(contents), editor = directActive?.editor; if (!value || !editor || typeof value.text !== "string") return;
  if (editor instanceof HTMLSelectElement) { const option = [...editor.options].find(item => item.text === value.text || item.value === value.text); if (!option) { fieldsStatusEl.textContent = "not an option"; return; } editor.value = option.value; }
  else {
    const start = editor.selectionStart ?? editor.value.length, end = editor.selectionEnd ?? start;
    editor.setRangeText(value.text, start, end, "end");
    // maxlength does not stop setRangeText; §8.14 truncates and commits that one value.
    if (editor.maxLength > 0 && editor.value.length > editor.maxLength) editor.value = editor.value.slice(0, editor.maxLength);
  }
  editor.dispatchEvent(new InputEvent("input", { bubbles: true, data: value.text, inputType: "insertText" }));
}
async function driveDirectKey(contents: string | null): Promise<void> {
  const value = driveWanted(contents), editor = directActive?.editor; if (!value || typeof value.key !== "string") return;
  let key = value.key, altKey = false, metaKey = false, shiftKey = false;
  for (const [prefix, set] of [["Alt+", () => { altKey = true; }], ["Meta+", () => { metaKey = true; }], ["Shift+", () => { shiftKey = true; }]] as const) if (key.startsWith(prefix)) { set(); key = key.slice(prefix.length); }
  // P5 U3 §9.12: Escape first closes an open sign or verify dialog, as a real Escape does.
  if (key === "Escape" && (certSignDialog.open || verifyDialog.open)) { if (certSignDialog.open) certSignDialog.close(); if (verifyDialog.open) verifyDialog.close(); return; }
  // P6 U7 §10.35: ⌘C/⌘X/⌘V are the Edit-menu roles: a DOM clipboard event on the focused element (never a keydown).
  const DRIVE_CLIPBOARD: Record<string, string> = { c: "copy", x: "cut", v: "paste" };
  if (metaKey && !altKey && !shiftKey && Object.hasOwn(DRIVE_CLIPBOARD, key)) {
    (document.activeElement ?? document.body).dispatchEvent(new ClipboardEvent(DRIVE_CLIPBOARD[key], { bubbles: true, cancelable: true }));
    if (directGestureWork) await directGestureWork;
    return;
  }
  // P6 U5 §9.17: an open object menu owns every key; Escape closes it (focus back to the anchor).
  if (!objectMenu.hidden) {
    if (key === "Escape") { closeObjectMenu(true); return; }
    const focused = objectMenu.contains(document.activeElement) ? document.activeElement as HTMLElement : objectMenu;
    focused.dispatchEvent(new KeyboardEvent("keydown", { key: key === "Space" ? " " : key, bubbles: true, cancelable: true, metaKey, altKey, shiftKey }));
    if (directGestureWork) await directGestureWork;
    return;
  }
  // draw-signature D5: while armed, Enter places at the page centre and Escape disarms.
  if (signArmed && !directActive && (key === "Enter" || key === "Escape")) { if (key === "Escape") disarmSign(); else await placeSignatureAtCentre(); return; }
  // P6 U6 §9.18: Escape or Enter leaves crop mode first (the selection stays).
  if (directCrop && (key === "Escape" || key === "Enter")) { leaveCrop(); return; }
  // §9.21: ⌥⌘←/→ on a group reaches the anchor's keydown wherever focus is.
  if (altKey && metaKey && (key === "ArrowLeft" || key === "ArrowRight") && !directActive && directSelection.length >= 2 && directFocusedObject) {
    const anchor = objectNode(directFocusedObject.targetId);
    if (anchor) { anchor.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, metaKey, altKey, shiftKey })); if (directGestureWork) await directGestureWork; return; }
  }
  if (key === "Space" && directFocusedField && (directFocusedField.kind === "checkbox" || directFocusedField.kind === "radio")) {
    const item = directFocusedField;
    await commitField(item, item.kind === "checkbox" ? item.value !== true : true);
    return;
  }
  if ((key === "ArrowLeft" || key === "ArrowRight") && directFocusedField?.kind === "radio" && !altKey) { await moveRadio(directFocusedField, key === "ArrowRight" ? 1 : -1); return; }
  if (key === "Tab") { const page = directActive?.page ?? wantedPage, index = directFieldIndex; if (directActive?.kind === "add" || directActive?.kind === "link" || directActive?.kind === "textbox") { await finishDirect(); return; } if (directActive) await finishDirect(); await focusNextField(shiftKey, page, index); return; }
  // §9.7: a focused object or redaction mark owns the key (Delete, Enter, ⌥-arrows); the script waits for its commit.
  const onTarget = !directActive && document.activeElement instanceof HTMLElement && document.activeElement.matches("#hit-layer .object-target, #hit-layer .redact-target, #hit-layer .textbox-target, #hit-layer .signature-target, #hit-layer .link-target, #hit-layer .sig-target" + (directSelection.length >= 2 ? ", #hit-layer .text-target, #hit-layer [data-field-id]" : "")) ? document.activeElement : null;
  if (onTarget && key !== "Escape") {
    onTarget.dispatchEvent(new KeyboardEvent("keydown", { key: key === "Space" ? " " : key, bubbles: true, cancelable: true, metaKey, altKey, shiftKey }));
    if (directGestureWork) await directGestureWork;
    return;
  }
  if (altKey && ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(key)) {
    const item = directActive?.kind === "field" ? directActive.item as FieldItem : directFocusedField;
    if (item) { await nudgeField(item, { key, shiftKey, metaKey } as KeyboardEvent); return; }
  }
  // §9.7 plain Enter is a newline in the add and paragraph editors; a synthetic keydown would insert nothing (review F5).
  if (key === "Enter" && !metaKey && editor && (directActive?.kind === "add" || directActive?.paragraph)) { driveDirectType(JSON.stringify({ text: "\n" })); return; }
  if (key === "Enter" && (metaKey || !(directActive?.paragraph || directActive?.kind === "add"))) { if (directActive) await finishDirect(); return; }
  if (key === "Enter" && !directActive) return;
  if (key === "Escape") { if (directActive?.kind === "add") clearError(); if (directActive) closeDirect(); directFocusedField = null; dropDirectFocus(); return; }
  const focused = (editor ?? document.activeElement) as HTMLElement;
  const translated = key === "Space" ? " " : key;
  focused.dispatchEvent(new KeyboardEvent("keydown", { key: translated, bubbles: true, metaKey, altKey, shiftKey }));
  if (directGestureWork) await directGestureWork;
  if (key === "Space" && directActive?.kind === "field") { const item = directActive.item as FieldItem; if (item.kind === "checkbox" || item.kind === "radio") await commitField(item, item.kind === "checkbox" ? item.value !== true : true); }
}

/** `bookmark {action: add|rename|delete|go, index?, title?}`: the existing nav paths (§8.11). */
async function driveBookmark(contents: string | null): Promise<void> {
  clearError();
  const wanted = driveWanted(contents);
  if (!wanted || !doc || !canEdit()) return;
  const action = wanted.action;
  const at = typeof wanted.index === "number" && Number.isInteger(wanted.index) ? wanted.index : -1;
  if (action === "add") {
    openBookmarkDialog(null);
    bookmarkTitleInput.value = String(wanted.title ?? "").slice(0, MAX_BOOKMARK_CHARS);
    const ok = await saveBookmarkDialog();
    if (bookmarkDialog.open) bookmarkDialog.close();
    await refreshOutline();
    navLast = { action: "bookmark-add", ...(ok ? {} : { detail: "invalid" }) };
    return;
  }
  await refreshOutline();
  const entry = outline[at];
  if (!entry) {
    navLast = { action: `bookmark-${String(action ?? "")}`, index: at, detail: "no-such-bookmark" };
    return;
  }
  if (action === "rename") {
    openBookmarkDialog(entry.id);
    bookmarkTitleInput.value = String(wanted.title ?? "").slice(0, MAX_BOOKMARK_CHARS);
    const ok = await saveBookmarkDialog();
    if (bookmarkDialog.open) bookmarkDialog.close();
    navLast = { action: "bookmark-rename", index: at, ...(ok ? {} : { detail: "invalid" }) };
  } else if (action === "delete") {
    await deleteBookmark(entry.id);
    navLast = { action: "bookmark-delete", index: at };
  } else if (action === "go") {
    followOutline(entry);
    navLast = { action: "bookmark-go", index: at };
  } else {
    navLast = { action: String(action ?? ""), index: at, detail: "invalid-action" };
    return;
  }
  await refreshOutline();
}

const DIRECT_MODES = ["read", "annotate", "organize", "forms", "edit", "redact"] as const;

/** `mode {mode}`: the chip the drive script cannot click. */
async function driveMode(contents: string | null): Promise<void> {
  const wanted = driveWanted(contents);
  const mode = wanted?.mode;
  if (typeof mode !== "string" || !(DIRECT_MODES as readonly string[]).includes(mode)) {
    showError("mode: expected read, annotate, organize, forms, edit or redact");
    return;
  }
  setMode(mode as UiMode);
  if (mode === "edit") await refreshContentTargets(true);
  paintHits(canvas.clientWidth, canvas.clientHeight, shownPage);
}

/** `arm {tool: note|freetext|ink|redact|text|off}`: press the tool the click must pass through. */
function driveArm(contents: string | null): void {
  const wanted = driveWanted(contents);
  const tool = typeof wanted?.tool === "string" ? wanted.tool : "";
  cancelPendingTools();
  const button = ({ note: noteAddButton, freetext: freeTextAddButton, ink: inkDrawButton, redact: redactAreaButton, text: contentAddArmButton, link: linkDrawButton, sigfield: sigfieldDrawButton } as Record<string, HTMLButtonElement>)[tool];
  if (tool !== "off" && !button) { showError(`arm: unknown tool ${tool}`); return; }
  button?.setAttribute("aria-pressed", "true");
  const prompt = ({ link: DRAW_LINK_PROMPT, sigfield: DRAW_SIGFIELD_PROMPT } as Record<string, string>)[tool];
  if (prompt) announce(prompt); // the same prompt the toolbar button announces
  paintHits(canvas.clientWidth, canvas.clientHeight, shownPage);
}
const DRAW_LINK_PROMPT = "Draw link: drag a rectangle on the page";
const DRAW_SIGFIELD_PROMPT = "Draw signature field: drag a rectangle on the page";

function runEditRequest(op: string, contents: string | null): unknown {
  if (op === "state") return;
  if (op === "page-retry") return pausedRetry.hidden ? showError("page-retry: nothing is paused") : retryPaused();
  if (op === "click") return driveDirectClick(contents);
  if (op === "drag" || op === "handle") return driveDirectGesture(op, contents);
  if (op === "menu") return driveObjectMenu(contents);
  if (op === "type") return driveDirectType(contents);
  if (op === "key") return driveDirectKey(contents);
  if (op === "mode") return driveMode(contents);
  if (op === "arm") return driveArm(contents);
  if (op === "outlines") { const on = driveWanted(contents)?.on; if (typeof on === "boolean") setOutlines(on); else showError("outlines: on must be true or false"); return; }
  // §8.8: the help dialog needs no document, so it is answered before any doc-bound branch.
  if (op === "help") return helpOp(contents);
  // Batch needs no document either (contract-batch.md §9.14): the menu opens the dialog, a drive runs it.
  if (op === "batch") return contents === null ? openBatchDialog() : driveBatch(contents);
  // Compare (contract-compare.md §9.11): close works without a main document; everything else needs one.
  if (op === "compare") {
    if (contents !== null && driveWanted(contents)?.close === true) return driveCompare(contents);
    if (!doc || doc.info.needsPassword) { showError("Open a document first, then choose Compare with…"); return; }
    return contents === null ? requestCompare() : driveCompare(contents);
  }
  if (op === "undo") return smokeUndo();
  else if (op === "redo") return smokeRedo();
  else if (op === "note") return smokeNote(contents ?? "");
  else if (op === "highlight") return smokeHighlight(contents);
  else if (op === "underline") return driveMarkup("Underline");
  else if (op === "strikeout") return driveMarkup("StrikeOut");
  else if (op === "annot") return driveAnnot(contents);
  else if (op === "goto") return driveGoto(contents);
  else if (op === "zoom") return driveZoom(contents);
  else if (op === "find") return driveFind(contents);
  else if (op === "tab") driveTab(contents);
  else if (op === "freetext") return smokeFreeText(contents);
  else if (op === "pages") return smokePages();
  else if (op === "form") return smokeForm(contents);
  else if (op === "fill-text") return driveFillText(contents);
  else if (op === "sign-draw") return driveSignDraw(contents);
  else if (op === "sign-place") return driveSignPlace(contents);
  else if (op === "fields") return smokeFieldDesign(contents);
  else if (op === "nav") return smokeNav(contents);
  else if (op === "assemble") return smokeAssemble();
  // The menu opens the dialogs; a `contents` payload marks the smoke path, which
  // has no way to press a button (the same convention the note/highlight ops use).
  else if (op === "extract") { if (contents === null) requestExtract(); else return smokeExtract(); }
  else if (op === "insert") return contents === null ? requestInsert() : driveInsert(contents);
  else if (op === "new-from-images") return requestNewFromImages();
  else if (op === "export") requestExport();
  else if (op === "watermark") return contents === null ? openStampDialog("watermark") : driveStamp(contents);
  else if (op === "header-footer") return contents === null ? openStampDialog("header") : driveStamp(contents);
  else if (op === "labels") return contents === null ? openLabelsDialog() : driveLabels(contents);
  else if (op === "decorate") return smokeDecorate(contents);
  else if (op === "security") openSecurityDialog();
  else if (op === "cert-sign-open") openCertSignDialog();
  else if (op === "verify-open" || op === "verify") return contents === null ? openVerifyDialog() : driveVerify(contents);
  else if (op === "encrypt") return smokeEncrypt(contents);
  else if (op === "encrypt-keep") return smokeEncryptKeep(contents);
  else if (op === "password") return smokePassword(contents);
  else if (op === "square" || op === "circle" || op === "line" || op === "ink") return addShapeOp(op);
  else if (op === "stamp") openStampIconDialog();
  else if (op === "sign") return contents === null ? driveSignPlace(null) : driveSign(contents);
  else if (op === "shapes") return smokeShapes(contents);
  else if (op === "export-png" || op === "export-jpeg" || op === "export-txt") return smokeExport(op.slice(7) as "png" | "jpeg" | "txt");
  else if (op === "bookmark") return contents === null ? openBookmarkDialog() : driveBookmark(contents);
  else if (op === "link") return contents === null ? openLinkDialog("page") : driveLink(contents);
  else if (op === "content-edit") return openContentEditor();
  else if (op === "text-add-open") { selectTabById(inspectorTabs, inspectorPanels, "tab-text"); setInspectorOpen(true); contentAddText.focus(); }
  else if (op === "text-add") return driveTextAdd(contents);
  else if (op === "content") return smokeContent(contents);
  else if (op === "redact") return driveRedact(contents);
  else if (op === "redact-preview") return driveRedactPreview();
  else if (op === "redact-apply") return driveRedactApply(contents);
  else if (op === "redact-area") return redactAreaButton.click();
  else if (op === "redact-search") { redactSearchText.focus(); redactSearchText.select(); }
  else if (op === "redact-apply-open") return openRedactDialog();
  else if (PAGE_OPS.has(op)) return runPageOp(op);
  // A typo must not read as a green step (the chain above IS the list of renderer ops).
  else showError(`Unknown command "${op}"`);
}

/**
 * The help surface (U7 §3): one static dialog, opened by the Help menu item, the drive
 * `help` command or the close button. §8.8: opening an open dialog and closing a closed
 * one are no-ops, and the state reads the element live (no cached flag).
 */
function helpOp(contents: string | null): void {
  if (contents === "close") {
    if (helpDialog.open) helpDialog.close();
    return;
  }
  if (!helpDialog.open) helpDialog.showModal();
}

/** The matrix body rows, which is what the drive state reports. */
function helpRows(): number {
  return helpMatrixEl.tBodies[0]?.rows.length ?? 0;
}

async function openFromUrl(name: string, url: string, untitled = false) {
  const token = openGuard.begin();
  try {
    const response = await fetch(url);
    if (!openGuard.isCurrent(token)) return; // a newer Open arrived while fetching
    if (!response.ok) throw new Error(`${name} could not be delivered (HTTP ${response.status}).`);
    const bytes = await response.arrayBuffer();
    if (!openGuard.isCurrent(token)) return;
    await openDocument(name, bytes, url);
    if (untitled && doc?.url === url) doc.untitled = true;
  } catch (err) {
    if (openGuard.isCurrent(token)) showError(messageOf(err));
  }
}

async function openStartupFixture() {
  const started = performance.now();
  const token = openGuard.begin();
  try {
    setStatus("Fetching fixture…");
    const fixture = document.querySelector("meta[name=pdf4all-fixture]")?.getAttribute("content") || "hello.pdf";
    if (!/^[A-Za-z0-9._-]+\.pdf$/.test(fixture)) throw new Error("bad fixture");
    const response = await fetch(`/fixtures/${fixture}`);
    if (!openGuard.isCurrent(token)) return; // a real file was opened while the fixture loaded
    if (!response.ok) throw new Error(`fixture HTTP ${response.status}`);
    const bytes = await response.arrayBuffer();
    if (!openGuard.isCurrent(token)) return;
    const first = await openDocument(fixture, bytes, null);
    if (!openGuard.isCurrent(token)) return; // a real file took over while the fixture rendered
    if (!first) throw new Error("startup render produced no page");
    if (!first.text.includes("PDF4All")) throw new Error("extracted text missing PDF4All");
    console.log(`P0_RESULT ${JSON.stringify({
      width: first.width,
      height: first.height,
      pixelWidth: first.pixelWidth,
      pixelHeight: first.pixelHeight,
      text: first.text,
      renderMs: Math.round(performance.now() - started),
      openMs: doc?.openMs ?? 0,
      firstPageRenderMs: first.paintMs,
      nonWhite: first.nonWhite,
    })}`);
  } catch (err) {
    // Never fail the smoke over a fixture that a real Open already replaced.
    if (openGuard.isCurrent(token)) failP0(err instanceof Error ? err.message : String(err));
  }
}

/**
 * `url` is null for the built-in startup fixture, which has no native source to save from.
 * `restoring`: M9's fallback reopen after a restart, which does not count as proof that the
 * worker is healthy (the restart cap stays as it is).
 */
async function openDocument(name: string, bytes: ArrayBuffer, url: string | null, restoring = false): Promise<RenderReply | null> {
  const startedGeneration = ++generation;
  resetDocument();
  recovery.reset(); // the open below starts the next log
  pausedReads.clear(); // a read paused for the last document may work for this one
  renderPaused();
  diskWrite = null; // what the last document's save wrote no longer matters
  clearError();
  if (!(bytes instanceof ArrayBuffer) || bytes.byteLength < 5) {
    showError(`${name} is not a readable PDF.`);
    return null;
  }
  if (bytes.byteLength > MAX_PDF_BYTES) {
    showError(`${name} is larger than the ${MAX_PDF_BYTES} byte limit.`);
    return null;
  }
  setStatus(`Opening ${name}…`);
  let opened: OpenReply;
  try {
    opened = (await request({ op: "open", pdf: bytes, name }, [bytes])) as OpenReply;
  } catch (err) {
    // §9.19(d): the pair-cap close happened before the open failed, so the engine slot is gone.
    const compareClosed = (err as { reply?: { compareClosed?: boolean } }).reply?.compareClosed === true;
    if (compareClosed && compareDoc) {
      clearCompare();
      compareLast = { action: "close", ok: true, detail: "too-large" };
    }
    if (startedGeneration === generation) showError(messageOf(err));
    if (compareClosed) announce("Compare closed: the two files together exceed the size limit");
    return null;
  }
  if (startedGeneration !== generation) return null;
  if (!restoring) workerRestarts = 0; // a session that opened a document is healthy; the cap only stops a restart loop
  if (opened.compareClosed) {
    // §9.3: the worker dropped the compare slot because the pair is over the size limit.
    clearCompare();
    compareLast = { action: "close", ok: true, detail: "too-large" };
  }

  doc = { name, url, pageCount: opened.pageCount, pages: opened.pages, rotations: opened.rotations, info: opened.info, openMs: opened.openMs };
  revision = opened.revision ?? 0;
  auth = opened.auth ?? "none";
  securityState = stateFromReply(opened.info, auth);
  document.title = `${name} — Tandem PDF`;
  documentTitleEl.textContent = name;
  syncDirty();
  bannerDismissed = null;
  renderBanner(opened.info);
  wantedPage = 0;
  shownPage = -1;
  shownKey = null;
  refreshThumbs();
  void refreshNotes();
  void refreshFields();
  void refreshOutline();
  void refreshLinks();
  void refreshLabels();
  void refreshLayers();
  void refreshRedactPreview();
  syncControls();
  setStatus(`Opened ${name} — ${opened.pageCount} page${opened.pageCount === 1 ? "" : "s"} in ${opened.openMs} ms`);

  if (opened.info.needsPassword) {
    setStatus(`${name} is encrypted; a password is required, so nothing can be rendered.`);
    showPasswordForm();
  } else {
    await drain();
  }
  // A newer document took over during the render: report nothing for this one.
  if (startedGeneration !== generation) return null;
  if (compareDoc) await refreshCompare(true); // D1: the slot survives; B follows A's new page 0
  if (opened.compareClosed) announce("Compare closed: the two files together exceed the size limit");
  const first = shownPage === 0 ? lastResult : null;
  emitResult(first);
  return first;
}

/** One line per open (and per unlock); grok's smoke waits for it by name. */
function emitResult(first: RenderReply | null) {
  if (!doc) return;
  console.log(`P1_RESULT ${JSON.stringify({
    name: doc.name,
    pageCount: doc.pageCount,
    info: doc.info,
    openMs: doc.openMs,
    firstPageRenderMs: first ? first.renderMs : null,
    readOnly: isReadOnly(doc.info),
  })}`);
}

function isReadOnly(info: DocumentInfo): boolean {
  return info.signatures > 0 || info.certified || info.needsPassword;
}

/** The worker's security view when no `security-set` has been made this session. */
function stateFromReply(info: DocumentInfo, level: AuthLevel): SecurityState {
  return {
    mode: "keep",
    algorithm: info.algorithm,
    permissions: info.needsPassword ? [] : permissionNames(info.permissions),
    auth: level,
    pending: false,
  };
}

/**
 * One `/P` permission for this document. An unencrypted document permits all of
 * them; mupdf reports all five for an owner-authenticated session, so the owner
 * needs no special case here. False whenever the document is still locked.
 */
function permitted(name: keyof DocumentPermissions): boolean {
  const info = doc?.info;
  if (!doc || !info || info.needsPassword) return false;
  return allowed(info, name);
}

/**
 * Whether the worker will accept any mutation: unlocked, not signed or certified,
 * and granting at least one of the three editing permissions.
 */
function editable(): boolean {
  const info = doc?.info;
  if (!doc || !info || info.needsPassword) return false;
  if (info.signatures > 0 || info.certified) return false;
  return allowed(info, "any");
}

const ALGORITHM_LABEL: Record<string, string> = {
  none: "no encryption",
  "rc4-40": "40-bit RC4, weak encryption",
  "rc4-128": "128-bit RC4, weak encryption",
  "aes-128": "128-bit AES",
  "aes-256": "256-bit AES",
  unknown: "an unrecognised encryption",
};

const SECURITY_MODE_LABEL: Record<SecurityState["mode"], string> = {
  keep: "keeping the current encryption",
  none: "no encryption",
  "aes-256": "AES-256",
};

const PERMISSION_TITLE = "This document does not permit changes";

/** The three editing capabilities every interactive path checks before it starts. */
function canAnnotate(): boolean {
  return editable() && permitted("annotate");
}

function canEdit(): boolean {
  return editable() && permitted("edit");
}

function canForm(): boolean {
  return editable() && permitted("form");
}

/** U6 §7.2: authoring writes the field tree (edit) and the page `/Annots` (annotate). */
function canDesign(): boolean {
  const info = doc?.info;
  return !!info && editable() && allowed(info, "edit+annotate");
}

/** The notice text the user closed. The bar stays away until the text changes or another document opens. */
let bannerDismissed: string | null = null;

bannerDismissEl.addEventListener("click", () => {
  bannerDismissed = bannerTextEl.textContent;
  bannerEl.hidden = true;
  pageRegion.focus();
});

function renderBanner(info: DocumentInfo) {
  const flags: string[] = [];
  if (info.encrypted) {
    if (info.needsPassword) flags.push("encrypted, password required");
    else {
      flags.push(`encrypted (${ALGORITHM_LABEL[info.algorithm] ?? info.algorithm})`);
      const granted = permissionNames(info.permissions);
      flags.push(granted.length === 5
        ? `opened with the ${auth === "owner" ? "owner" : "user"} password, every permission`
        : `opened with the ${auth === "owner" ? "owner" : "user"} password, permits ${granted.join(", ") || "nothing"}`);
    }
  }
  if (info.signatures > 0) flags.push(`${info.signatures} signature field${info.signatures === 1 ? "" : "s"}`);
  if (info.certified) flags.push("certified");
  if (info.xfa) flags.push("XFA form");
  if (info.attachments > 0) flags.push(`${info.attachments} attachment${info.attachments === 1 ? "" : "s"}`);
  if (info.openAction) flags.push("open action");
  if (info.repaired) flags.push("repaired");
  const pending = securityState?.pending ? ` Security will change to ${SECURITY_MODE_LABEL[securityState.mode]} when you save.` : "";
  if (!flags.length && !pending) {
    bannerEl.hidden = true;
    bannerTextEl.textContent = "";
    return;
  }
  const text = `${flags.length ? `Detected: ${flags.join(", ")}.` : ""}${isReadOnly(info) ? (info.signatures > 0 || info.certified ? " Read-only: signed; use Verify to check the signatures." : " Read-only.") : ""}${pending}`;
  bannerTextEl.textContent = text;
  bannerEl.hidden = text === bannerDismissed;
}

// ----------------------------------------------------------------- password

function showPasswordForm() {
  if (!doc) return;
  passwordName.textContent = doc.name;
  passwordErrorEl.textContent = "";
  passwordInput.value = "";
  passwordForm.hidden = false;
  passwordInput.focus();
}

function hidePasswordForm() {
  passwordForm.hidden = true;
  passwordInput.value = "";
  passwordErrorEl.textContent = "";
  unlocking = false;
}

/**
 * The password goes to the worker and nowhere else: no IPC, no log, no P1_* line.
 * A correct one re-runs the open path (banner, thumbs, controls, first page).
 */
async function submitPassword(event: Event) {
  event.preventDefault();
  if (!doc || !doc.info.needsPassword || unlocking) return;
  const password = passwordInput.value;
  passwordInput.value = ""; // the request is the only place it lives from here on
  const startedGeneration = generation;
  unlocking = true;
  passwordErrorEl.textContent = "";
  setStatus(`Unlocking ${doc.name}…`);
  let reply: AuthenticateReply;
  try {
    reply = (await request({ op: "authenticate", password })) as AuthenticateReply;
  } catch (err) {
    unlocking = false;
    if (startedGeneration !== generation) return;
    showError(messageOf(err));
    return;
  }
  unlocking = false;
  if (startedGeneration !== generation || !doc) return; // a newer document owns the UI now
  if (!reply.authenticated) {
    passwordErrorEl.textContent = "Incorrect password";
    passwordInput.focus();
    setStatus(`${doc.name} is still locked.`);
    return;
  }
  await applyUnlocked(reply);
}

/** A successful unlock: the same reopen path the first page took (banner, thumbs, page). */
async function applyUnlocked(reply: Extract<AuthenticateReply, { authenticated: true }>): Promise<void> {
  if (!doc) return;
  doc.pageCount = reply.pageCount;
  doc.pages = reply.pages;
  doc.rotations = reply.rotations;
  doc.info = reply.info;
  auth = reply.auth ?? "user";
  securityState = stateFromReply(doc.info, auth);
  hidePasswordForm();
  renderBanner(doc.info);
  refreshThumbs();
  void refreshFields();
  void refreshOutline();
  void refreshLinks();
  void refreshLabels();
  void refreshLayers();
  void refreshRedactPreview();
  wantedPage = 0;
  shownPage = -1;
  shownKey = null;
  shownZoom = 0;
  syncControls();
  clearError(); // §9.19(c): a stale refusal (e.g. the compare guard) does not outlive the unlock
  await drain();
  if (compareDoc) await refreshCompare(true); // §9.19(h)
  emitResult(shownPage === 0 ? lastResult : null);
}

// ----------------------------------------------------------------- zoom

/** CSS pixels per PDF point. Fit modes read the page region, which is the resize-driven part. */
function zoomNow(page: number): number {
  if (zoomMode === "fixed" || !doc) return fixedZoom;
  const size = doc.pages[page] ?? doc.pages[0];
  if (!size) return fixedZoom;
  const width = Math.max(1, viewport.clientWidth - 2);
  const fit = width / size.width;
  if (zoomMode === "fit-page") {
    const height = Math.max(1, viewport.clientHeight - 2);
    return Math.min(fit, height / size.height, MAX_ZOOM);
  }
  return Math.min(fit, MAX_ZOOM);
}

/** Worker render scale: zoom × dpr, inside the engine bounds and the 4096 px edge. */
function renderScale(page: number): number {
  const raw = zoomNow(page) * (window.devicePixelRatio || 1);
  const clamped = Math.min(MAX_SCALE, Math.max(MIN_SCALE, raw));
  const size = doc?.pages[page];
  if (!size) return clamped;
  const edge = Math.min(MAX_PIXEL_EDGE / size.width, MAX_PIXEL_EDGE / size.height);
  // Clamped down, never over: the engine rejects anything past the edge.
  return Math.min(clamped, Math.floor(edge * 100) / 100);
}

/** The one wanted state: page + scale. Anything newer supersedes it in `drain`. */
function target() {
  if (!doc || doc.info.needsPassword) return null;
  const page = wantedPage;
  const zoom = zoomNow(page);
  const scale = renderScale(page);
  // The revision is part of the key: a mutation must repaint without a full reset.
  return { page, zoom, scale, key: `${page}:${scale}:${revision}` };
}

function setZoom(mode: ZoomMode, value = fixedZoom) {
  zoomMode = mode;
  if (mode === "fixed") fixedZoom = value;
  syncZoom();
  void drain();
}

function zoomStep(direction: number) {
  if (!doc) return;
  const current = zoomNow(wantedPage);
  const next = direction > 0
    ? ZOOM_STEPS.find(step => step > current + 0.001) ?? ZOOM_STEPS[ZOOM_STEPS.length - 1]
    : [...ZOOM_STEPS].reverse().find(step => step < current - 0.001) ?? ZOOM_STEPS[0];
  setZoom("fixed", next);
}

function syncZoom() {
  const percent = doc && !doc.info.needsPassword ? Math.round(zoomNow(wantedPage) * 100) : 100;
  zoomLevel.textContent = `${percent}%`;
  fitWidthButton.setAttribute("aria-pressed", String(zoomMode === "fit-width"));
  fitPageButton.setAttribute("aria-pressed", String(zoomMode === "fit-page"));
}

// ----------------------------------------------------------------- navigation

function goTo(page: number) {
  if (!doc || doc.pageCount < 1 || doc.info.needsPassword) return;
  const nextPage = Math.min(Math.max(0, Math.trunc(page)), doc.pageCount - 1);
  if (nextPage !== wantedPage) { clearDirectFocus(); clearError(); }
  wantedPage = nextPage;
  // Leaving the page commits the open editor there (its own captured page), as click-away does.
  if (directActive && directActive.page !== wantedPage) dropDirect();
  syncControls();
  void refreshNotes();
  void refreshFields();
  void refreshLinks();
  void refreshContentTargets(uiMode === "edit");
  void drain();
  if (compareDoc) void refreshCompare();
}

async function drain() {
  if (inFlight !== null || !doc || doc.info.needsPassword) return;
  const startedGeneration = generation;
  while (doc && startedGeneration === generation) {
    const goal = target();
    if (!goal || (goal.key === shownKey && goal.zoom === shownZoom)) return;
    inFlight = goal.key;
    let result: RenderReply;
    try {
      result = cache.get(goal.key) ?? (await request({ op: "render", page: goal.page, scale: goal.scale }) as RenderReply);
    } catch (err) {
      if (startedGeneration !== generation) return; // a newer document owns inFlight and the cache now
      inFlight = null;
      if ((err as { reason?: unknown }).reason === "skipped") {
        // M9: this page hung the worker at this revision; show it blank instead of asking again.
        lastResult = null;
        shownPage = goal.page;
        shownKey = goal.key;
        shownZoom = goal.zoom;
        canvas.getContext("2d")?.clearRect(0, 0, canvas.width, canvas.height);
        setStatus(messageOf(err));
        pumpThumbs();
        return;
      }
      showError(messageOf(err));
      return;
    }
    // Too late: the page belongs to a document that is no longer open.
    if (startedGeneration !== generation) return;
    cache.set(goal.key, result);
    inFlight = null;
    const now = target();
    if (!now || now.key !== goal.key || now.zoom !== goal.zoom) continue; // a newer wanted state won
    lastResult = result;
    shownPage = goal.page;
    shownKey = goal.key;
    shownZoom = goal.zoom;
    paint(result, goal.zoom, goal.page);
    setStatus(`Page ${goal.page + 1}${labelSuffix(goal.page)} of ${doc.pageCount} — rendered in ${result.renderMs} ms at ${Math.round(goal.zoom * 100)}%`);
    pumpThumbs(); // page renders first: thumbnails only start once the wanted page is painted
  }
}

function syncControls() {
  const writable = editable();
  // Per-permission gates from `/P`: annotate for notes/markup/links, edit for page
  // and structure work, form for field fill, copy for extract/export.
  const canAnnotate = writable && permitted("annotate");
  const canEdit = writable && permitted("edit");
  const canCopy = !!doc && !doc.info.needsPassword && permitted("copy");
  // `applyLabelText` below owns the caption: it is the same line with the page label
  // appended when the document's labels say something other than the index.
  const total = doc?.pageCount ?? 0;
  pageInput.value = total > 0 ? String(wantedPage + 1) : "";
  pageInput.max = String(Math.max(1, total));
  pageInput.disabled = total < 2;
  prevButton.disabled = total < 2 || wantedPage <= 0;
  nextButton.disabled = total < 2 || wantedPage >= total - 1;
  // Plain Save rewrites the file the document came from; a fixture or a read-only
  // document has nowhere safe to go, Save a copy still does.
  saveButton.disabled = saving || !dirty || !doc || !doc.url || !writable;
  saveCopyButton.disabled = saving || !doc || !doc.url || doc.info.needsPassword;
  undoButton.disabled = saving || !canUndo;
  redoButton.disabled = saving || !canRedo;
  noteAddButton.disabled = !canAnnotate || !doc;
  if (noteAddButton.disabled) noteAddButton.setAttribute("aria-pressed", "false");
  noteAddButton.title = permissionTitle(canAnnotate);
  applyLabelText();
  const hit = currentHit();
  for (const [button] of markupButtons) {
    button.disabled = !canAnnotate || !doc || !hit;
    button.title = permissionTitle(canAnnotate);
  }
  freeTextAddButton.disabled = !canAnnotate || !doc;
  if (freeTextAddButton.disabled) freeTextAddButton.setAttribute("aria-pressed", "false");
  freeTextAddButton.title = permissionTitle(canAnnotate);
  // Shapes, stamps and the signature all place themselves off the current hit or the
  // page centre, so none of them needs a search hit to be usable.
  for (const [button] of shapeButtons) {
    button.disabled = !canAnnotate || !doc;
    button.title = permissionTitle(canAnnotate);
  }
  inkDrawButton.disabled = !canAnnotate || !doc;
  if (inkDrawButton.disabled) inkDrawButton.setAttribute("aria-pressed", "false");
  inkDrawButton.title = permissionTitle(canAnnotate);
  stampIconButton.disabled = !canAnnotate || !doc;
  stampIconButton.title = permissionTitle(canAnnotate);
  signButton.disabled = !canAnnotate || !doc;
  signButton.title = permissionTitle(canAnnotate);
  redactAreaButton.disabled = !canAnnotate || !doc;
  redactSearchGo.disabled = !canAnnotate || !doc || !redactSearchText.value;
  redactApplyOpen.disabled = !canEdit || !doc || !redactState?.marks;
  redactAreaButton.title = permissionTitle(canAnnotate);
  redactSearchGo.title = permissionTitle(canAnnotate);
  redactApplyOpen.title = permissionTitle(canEdit);
  bookmarkAddButton.disabled = !canEdit || !doc;
  bookmarkAddButton.title = permissionTitle(canEdit);
  // A link is anchored on the current search match, so both add buttons need one.
  linkPageAddButton.disabled = !canAnnotate || !doc || !hit;
  linkUriAddButton.disabled = !canAnnotate || !doc || !hit;
  linkPageAddButton.title = permissionTitle(canAnnotate);
  linkUriAddButton.title = permissionTitle(canAnnotate);
  linkDrawButton.disabled = !canAnnotate || !doc;
  linkDrawButton.title = canAnnotate ? "Drag on the page to draw a link" : permissionTitle(canAnnotate);
  // P5 U3 §2.4: drawing a signature field authors the field tree like the designer.
  // Review R-U3-1: the page/rect Sign path (§9.15) is live whenever a PDF is open, as the menu item is.
  certSignOpenButton.disabled = !doc || doc.info.needsPassword;
  sigfieldDrawButton.disabled = !doc || !canDesign();
  if (sigfieldDrawButton.disabled) sigfieldDrawButton.setAttribute("aria-pressed", "false");
  sigfieldDrawButton.title = sigfieldDrawButton.disabled && doc ? permissionTitle(false) || "This document is read-only" : "Drag on the page to place a signature field";
  insertButton.disabled = !canEdit || !doc;
  insertButton.title = permissionTitle(canEdit);
  // Extracting writes a copy and never touches the open document, so a signed or
  // certified file (read-only for edits) can still be extracted from.
  extractButton.disabled = !canCopy;
  extractButton.title = canCopy ? "" : permissionTitle(false);
  exportButton.disabled = !canCopy;
  exportButton.title = canCopy ? "" : permissionTitle(false);
  watermarkButton.disabled = !canEdit || !doc;
  watermarkButton.title = permissionTitle(canEdit);
  headerButton.disabled = !canEdit || !doc;
  headerButton.title = permissionTitle(canEdit);
  pageLabelsButton.disabled = !canEdit || !doc;
  pageLabelsButton.title = permissionTitle(canEdit);
  securityButton.disabled = !canChangeSecurity();
  securityButton.title = canChangeSecurity() ? "" : securityRefusal();
  printButton.disabled = printing || !printAvailable();
  printButton.title = doc && !doc.info.needsPassword && !doc.info.printable
    ? "This document does not permit printing"
    : "";
  thumbsToggle.disabled = !doc || doc.info.needsPassword;
  verifyOpenButton.hidden = !doc || !(doc.info.signatures > 0 || doc.info.certified);
  syncPageTools();
  setThumbCurrent(wantedPage);
  syncZoom();
}

/** The `title` a button carries when a `/P` permission (or the lock) disabled it. */
function permissionTitle(allowed: boolean): string {
  if (allowed) return "";
  const info = doc?.info;
  if (!info) return "";
  if (info.needsPassword) return "Enter the document password first";
  if (info.signatures > 0 || info.certified) return "This document is read-only";
  return PERMISSION_TITLE;
}

/** Changing security is an owner act; an unencrypted document has no owner. */
function canChangeSecurity(): boolean {
  const info = doc?.info;
  if (!doc || !info || info.needsPassword) return false;
  if (info.signatures > 0 || info.certified) return false;
  return !info.encrypted || auth === "owner";
}

function securityRefusal(): string {
  const info = doc?.info;
  if (!info || info.needsPassword) return "Enter the document password first";
  if (info.signatures > 0 || info.certified) return "This document is read-only";
  return "Open this document with its owner password to change its security";
}

/** Page tools follow the current page: Move up on page 1 or Delete at one page are dead ends. */
function syncPageTools() {
  const total = doc?.pageCount ?? 0;
  const writable = editable() && permitted("edit");
  const page = wantedPage;
  const enabled: Record<string, boolean> = {
    "rotate-right": writable && total > 0,
    "rotate-left": writable && total > 0,
    "move-up": writable && page > 0,
    "move-down": writable && page < total - 1,
    "delete-page": writable && total > 1,
  };
  for (const [button, op] of pageToolButtons) {
    button.disabled = !enabled[op];
    button.title = permissionTitle(writable);
    // The accessible name carries the page the button will act on ("Delete page 3").
    const label = op === "rotate-right" ? `Rotate page ${page + 1} right`
      : op === "rotate-left" ? `Rotate page ${page + 1} left`
        : op === "move-up" ? `Move page ${page + 1} up`
          : op === "move-down" ? `Move page ${page + 1} down`
            : `Delete page ${page + 1}`;
    button.setAttribute("aria-label", label);
  }
}

function paint(result: RenderReply, zoom: number, page: number) {
  const context = canvas.getContext("2d");
  if (!context) throw new Error("canvas 2d unavailable");
  canvas.width = result.pixelWidth;
  canvas.height = result.pixelHeight;
  context.putImageData(new ImageData(new Uint8ClampedArray(result.pixels), result.pixelWidth, result.pixelHeight), 0, 0);
  const cssWidth = Math.round(result.width * zoom);
  const cssHeight = Math.round(result.height * zoom);
  canvas.style.width = `${cssWidth}px`;
  canvas.style.height = `${cssHeight}px`;
  paintText(result.lines, zoom, cssWidth, cssHeight, page);
  paintHits(cssWidth, cssHeight, page);
  extracted.textContent = result.text;
  textEmptyEl.hidden = result.text.length > 0;
}

/** Repaints from the last render, e.g. when highlights or the hit index change. */
function repaint() {
  if (lastResult && shownPage >= 0) paint(lastResult, shownZoom, shownPage);
}

export function readingOrder(items: FieldItem[]): FieldItem[] {
  const rows: FieldItem[][] = [];
  for (const item of [...items].filter(item => item.kind !== "signature").sort((a, b) => a.rect[1] - b.rect[1] || a.rect[0] - b.rect[0])) {
    const row = rows.find(group => group.some(other => Math.max(0, Math.min(item.rect[3], other.rect[3]) - Math.max(item.rect[1], other.rect[1])) >= Math.min(item.rect[3] - item.rect[1], other.rect[3] - other.rect[1]) * 0.5));
    if (row) row.push(item); else rows.push([item]);
  }
  const seen = new Set<string>(), ordered: FieldItem[] = [];
  for (const row of rows.sort((a, b) => Math.min(...a.map(item => item.rect[1])) - Math.min(...b.map(item => item.rect[1])))) {
    for (const item of row.sort((a, b) => a.rect[0] - b.rect[0])) {
      if (item.kind !== "radio") ordered.push(item);
      else if (!seen.has(item.name)) { seen.add(item.name); ordered.push(items.find(kid => kid.name === item.name && kid.kind === "radio" && kid.value === true) ?? item); }
    }
  }
  return ordered;
}
/** P6 U4 §2.1: the page's `/Tabs` name as the fields reply gives it, null for anything else. */
function tabsName(value: unknown): string | null {
  return typeof value === "string" && ["R", "C", "S", "A", "W"].includes(value) ? value : null;
}
/** A radio group once (its checked kid, else its first), everything else as given; signatures dropped. */
function collapseRadios(list: FieldItem[], items: FieldItem[]): FieldItem[] {
  const seen = new Set<string>(), ordered: FieldItem[] = [];
  for (const item of list) {
    if (item.kind === "signature") continue;
    if (item.kind !== "radio") ordered.push(item);
    else if (!seen.has(item.name)) { seen.add(item.name); ordered.push(items.find(kid => kid.name === item.name && kid.kind === "radio" && kid.value === true) ?? item); }
  }
  return ordered;
}
/**
 * P6 U4 §2.2: Tab order per the page's `/Tabs`: R/null reading order; A/W the reply's (`/Annots`) order;
 * C columns (horizontal overlap ≥ 50 % of the narrower width, columns by their left, then top→bottom, left→right);
 * S by `structIndex`, the rest in reading order after — with no index at all, reading order as 'fallback'.
 */
export function tabOrder(items: FieldItem[], tabs: string | null): { order: FieldItem[]; used: "R" | "C" | "S" | "A" | "W" | "fallback" } {
  if (tabs === "A" || tabs === "W") return { order: collapseRadios(items, items), used: tabs };
  if (tabs === "C") {
    const columns: FieldItem[][] = [];
    for (const item of [...items].filter(item => item.kind !== "signature").sort((a, b) => a.rect[0] - b.rect[0] || a.rect[1] - b.rect[1])) {
      const column = columns.find(group => group.some(other => Math.max(0, Math.min(item.rect[2], other.rect[2]) - Math.max(item.rect[0], other.rect[0])) >= Math.min(item.rect[2] - item.rect[0], other.rect[2] - other.rect[0]) * 0.5));
      if (column) column.push(item); else columns.push([item]);
    }
    const list = columns.sort((a, b) => Math.min(...a.map(item => item.rect[0])) - Math.min(...b.map(item => item.rect[0])))
      .flatMap(column => column.sort((a, b) => a.rect[1] - b.rect[1] || a.rect[0] - b.rect[0]));
    return { order: collapseRadios(list, items), used: "C" };
  }
  if (tabs === "S") {
    const index = (item: FieldItem) => (item as FieldItem & { structIndex?: unknown }).structIndex;
    const indexed = items.filter(item => typeof index(item) === "number");
    if (!indexed.length) return { order: readingOrder(items), used: "fallback" };
    const sorted = indexed.sort((a, b) => (index(a) as number) - (index(b) as number));
    return { order: collapseRadios([...sorted, ...readingOrder(items.filter(item => typeof index(item) !== "number"))], items), used: "S" };
  }
  return { order: readingOrder(items), used: "R" };
}
export function editorBox(rect: number[], zoom: number): { left: number; top: number; width: number; height: number; fontPx: number } {
  const [x, y, x2, y2] = rect, width = Math.max(1, (x2 - x) * zoom), height = Math.max(1, (y2 - y) * zoom);
  return { left: x * zoom, top: y * zoom, width, height, fontPx: Math.max(10, Math.min(400, height * 0.6)) };
}
/** P6 U3 §1.1: a PDF user-space box (y up) through the page transform → the y-down view rect (4 corners, min/max). */
export function viewBox(box: number[], m: number[]): number[] {
  const [a, b, c, d, e, f] = m, xs: number[] = [], ys: number[] = [];
  for (const [x, y] of [[box[0], box[1]], [box[2], box[1]], [box[0], box[3]], [box[2], box[3]]]) { xs.push(a * x + c * y + e); ys.push(b * x + d * y + f); }
  return [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)];
}
/** The exact inverse of `viewBox` (2×2 linear part + translation), rounded to 4 decimals; throws on a zero determinant. */
export function pdfBox(rect: number[], m: number[]): number[] {
  const [a, b, c, d, e, f] = m, det = a * d - b * c;
  if (!det || !Number.isFinite(det)) throw new Error("pdfBox: the page transform is not invertible");
  const xs: number[] = [], ys: number[] = [], round4 = (value: number) => Math.round(value * 1e4) / 1e4 + 0;
  for (const [x, y] of [[rect[0], rect[1]], [rect[2], rect[1]], [rect[0], rect[3]], [rect[2], rect[3]]]) { const u = x - e, v = y - f; xs.push((d * u - c * v) / det); ys.push((a * v - b * u) / det); }
  return [round4(Math.min(...xs)), round4(Math.min(...ys)), round4(Math.max(...xs)), round4(Math.max(...ys))];
}
/**
 * P6 U3 §9.8: the rect a grip/side/corner drag of (dx, dy) page pt makes from `initial` (y-down).
 * Grip moves and stays on the page; sides stretch; corners with `lockAspect` scale about the opposite
 * corner by s = max of the two side ratios, clamped so each side stays ≥ min and the corner stays on the page.
 */
export function gestureRect(initial: number[], handle: string, dx: number, dy: number, options: { min: number; lockAspect: boolean; page: number[]; within?: number[] }): number[] {
  const [x0, y0, x1, y1] = initial, [pw, ph] = options.page, min = options.min, w0 = x1 - x0, h0 = y1 - y0;
  // P6 U6 §2.3/§9.18: `within` (crop mode: the uncropped box ∩ the page) replaces the page clamp; no aspect lock with it.
  const [lx, ly, rx, ry] = options.within ?? [0, 0, pw, ph];
  if (handle === "grip") {
    const x = Math.max(lx, Math.min(rx - w0, x0 + dx)), y = Math.max(ly, Math.min(ry - h0, y0 + dy));
    return [x, y, x + w0, y + h0];
  }
  const west = handle.includes("w"), east = handle.includes("e"), north = handle.includes("n"), south = handle.includes("s");
  if (options.lockAspect && !options.within && (west || east) && (north || south) && w0 > 0 && h0 > 0) {
    const sx = (w0 + (east ? dx : -dx)) / w0, sy = (h0 + (south ? dy : -dy)) / h0;
    const most = Math.min(east ? (pw - x0) / w0 : x1 / w0, south ? (ph - y0) / h0 : y1 / h0);
    const s = Math.min(most, Math.max(min / w0, min / h0, sx, sy));
    const w = w0 * s, h = h0 * s;
    return [east ? x0 : x1 - w, south ? y0 : y1 - h, east ? x0 + w : x1, south ? y0 + h : y1];
  }
  const rect = [...initial];
  if (west) rect[0] = Math.max(lx, Math.min(x1 - min, x0 + dx));
  if (east) rect[2] = Math.min(rx, Math.max(x0 + min, x1 + dx));
  if (north) rect[1] = Math.max(ly, Math.min(y1 - min, y0 + dy));
  if (south) rect[3] = Math.min(ry, Math.max(y0 + min, y1 + dy));
  return rect;
}
export type SnapEdge = "x0" | "x1" | "cx" | "y0" | "y1" | "cy";
export type SnapGuide = { axis: "x" | "y"; at: number };
/**
 * P6 U6 §2.4/§9.19: alignment guides (view space, y down). Per axis the listed moving edges (order x0, x1, cx / y0, y1, cy)
 * meet every candidate (each other rect's x0, x1, cx / y0, y1, cy in order, then the page's edges and centre); the smallest
 * |delta| ≤ tolerance wins, a tie keeps the first. The rect is translated by at most one delta per axis, never resized.
 */
export function snapRect(rect: number[], others: number[][], page: { width: number; height: number }, tolerance = 4, edges: SnapEdge[] = ["x0", "x1", "cx", "y0", "y1", "cy"]): { rect: number[]; guides: SnapGuide[] } {
  const out = [...rect], guides: SnapGuide[] = [];
  for (const [axis, lo, extent] of [["x", 0, page.width], ["y", 1, page.height]] as const) {
    const value = { [`${axis}0`]: rect[lo], [`${axis}1`]: rect[lo + 2], [`c${axis}`]: (rect[lo] + rect[lo + 2]) / 2 } as Record<string, number>;
    const candidates = [...others.flatMap(other => [other[lo], other[lo + 2], (other[lo] + other[lo + 2]) / 2]), 0, extent, extent / 2];
    let best: { delta: number; at: number } | null = null;
    for (const edge of [`${axis}0`, `${axis}1`, `c${axis}`]) {
      if (!edges.includes(edge as SnapEdge)) continue;
      for (const at of candidates) {
        const delta = at - value[edge];
        if (Math.abs(delta) <= tolerance && (!best || Math.abs(delta) < Math.abs(best.delta))) best = { delta, at };
      }
    }
    if (best) { out[lo] += best.delta; out[lo + 2] += best.delta; guides.push({ axis, at: best.at }); }
  }
  return { rect: out, guides };
}
function placeDirectActive(zoom: number): void {
  const active = directActive; if (!active) return;
  placeBox(active.editor, active.paragraph?.bounds ?? (active.kind === "field" ? (active.item as FieldItem).rect : active.kind === "link" ? (active.item as LinkEntry).rect : (active.item as EditTarget).bounds ?? []), zoom);
  if (active.paragraph) { active.editor.style.width = `${active.paragraph.width * zoom}px`; active.editor.style.lineHeight = `${active.paragraph.leading * zoom}px`; growParagraphEditor(active.editor, (active.paragraph.bounds[3] - active.paragraph.bounds[1]) * zoom); }
  if (active.kind === "add" || active.kind === "textbox") growAddEditor(active);
}
/** P6 U3 §9.9: the add editor keeps its own font size (no 10 px floor) and grows to its draft, capped at the page's right edge. */
function growAddEditor(active: NonNullable<typeof directActive>): void {
  const editor = active.editor, rect = (active.item as EditTarget).bounds ?? [], page = doc?.pages[active.page];
  if (rect.length !== 4 || !page) return;
  editor.style.fontSize = `${Math.min(400, Number(editor.dataset.fontSize) * shownZoom)}px`;
  editor.style.width = `${(rect[2] - rect[0]) * shownZoom}px`;
  if (editor.scrollWidth > editor.clientWidth) editor.style.width = `${Math.min((page.width - rect[0]) * shownZoom, editor.scrollWidth + editor.offsetWidth - editor.clientWidth)}px`;
  growParagraphEditor(editor, (rect[3] - rect[1]) * shownZoom);
}
/** Paragraph textarea: at least the paragraph box, taller when the draft wraps to more lines (browser metrics differ from the PDF's). */
function growParagraphEditor(editor: HTMLElement, minPx?: number): void {
  if (minPx !== undefined) editor.dataset.minHeight = String(minPx);
  editor.style.height = `${Number(editor.dataset.minHeight) || 0}px`;
  if (editor.scrollHeight > editor.clientHeight) editor.style.height = `${editor.scrollHeight + editor.offsetHeight - editor.clientHeight}px`;
}
function placeBox(element: HTMLElement, rect: number[], zoom: number): void {
  const box = editorBox(rect, zoom);
  element.style.left = `${box.left}px`; element.style.top = `${box.top}px`;
  element.style.width = `${box.width}px`; element.style.height = `${box.height}px`;
  if (element.classList.contains("inline-editor")) element.style.fontSize = element.classList.contains("text-inline-editor")
    ? `${Math.max(10, Math.min(400, Number(element.dataset.fontSize ?? box.fontPx / zoom) * zoom))}px` : `${box.fontPx}px`;
}
let directActive: { kind: "field" | "text" | "add" | "link" | "textbox"; item: FieldItem | EditTarget | LinkEntry; editor: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement; page: number; scan?: ContentScan; paragraph?: ParagraphItem; optionList?: HTMLElement; point?: [number, number];
  /** fill-anywhere D3/D4: the FreeText a `textbox` editor edits (null = a new box). */
  annotId?: number | null } | null = null;
/** fill-anywhere D4: the focused text box by its `/Annots` object number; D6: the last text-box op for STATE `fill.last`. */
let directFocusedBox: number | null = null;
let fillLast: { op: string; id: number; rect: number[] | null } | null = null;
/** fill-anywhere D7: the first Save after a fill writes a "(filled)" copy; a "(filled).pdf" document keeps plain Save. */
let fillSave: "none" | "needs-copy" | "copied" = "none";
/** STATE `fill.last`, built here so the op-coverage scan in tests/p2-shell.test.mjs never reads it as a request op. */
function fillOp(op: string, id: number, rect: number[] | null): { op: string; id: number; rect: number[] | null } { return { op, id, rect }; }
function noteFilled(): void { if (fillSave === "none" && doc && !/ \(filled\)\.pdf$/i.test(doc.name)) fillSave = "needs-copy"; }
let directClickWork: Promise<void> | null = null;
/** P6 U3 §9.6: on-page focus of an object or a redaction mark (no editor); `rect` is the last view rect, for re-resolving after a commit. */
let directFocusedObject: ObjectFocus | null = null;
type ObjectFocus = { kind: string; name: string; targetId: string; rect: number[]; text?: string };
/** P6 U5 §2.1/§9.13: the object selection in click order; the last member is the anchor (`directFocusedObject`). */
let directSelection: ObjectFocus[] = [];
/** P6 U6 §2.3: crop mode on the single selected member; `full` is its uncropped view box (the visible box when uncropped). */
let directCrop: { targetId: string; full: number[] } | null = null;
/** The press that ended crop mode (it adds no text, §9.18). */
let cropDismiss: Event | null = null;
let directFocusedMark: { annotId: number } | null = null;
/** P6 U4 §9.6: on-page focus of a link in Annotate, by its `/Annots` object number. */
let directFocusedLink: { annotId: number } | null = null;
/** P6 U3 §9.9: a pointerdown on the page while an editor was open; that click only commits/closes it. */
let addBlocked = false;
/** P6 U3 §9.9: the blur's commit, which a drive click awaits before it clicks. */
let directFinishWork: Promise<void> | null = null;
function clearDirectFocus(): void { directFocusedBox = null; directFocusedSig = null; directFocusedObject = null; directSelection = []; directFocusedMark = null; directFocusedLink = null; closeObjectMenu(false); leaveCrop(); hitLayer.querySelectorAll(".group-box").forEach(node => node.remove()); }
let directFieldIndex = -1;
let directFocusedField: FieldItem | null = null;
let directFocusedPage = -1;
let highlightFields = (() => { try { return localStorage.getItem("field-highlight") !== "false"; } catch { return true; } })();
fieldHighlight.checked = highlightFields;
fieldHighlight.addEventListener("change", () => { highlightFields = fieldHighlight.checked; try { localStorage.setItem("field-highlight", String(highlightFields)); } catch {} paintHits(canvas.clientWidth, canvas.clientHeight, shownPage); });
fieldNext.addEventListener("click", () => focusNextField());
// fill-anywhere D5: the panel mirror of a click on the page — the text-box editor at the page centre.
fillTextboxAdd.addEventListener("click", () => {
  const page = wantedPage, size = doc?.pages[page];
  if (!doc || !size || !canAnnotate()) return;
  if ((doc.rotations?.[page] ?? 0) % 360 !== 0) { showError(TEXTBOX_ROTATED); return; }
  if (uiMode !== "forms") setMode("forms");
  openTextBox(page, [size.width / 2, size.height / 2], null);
});
/** P6 U4 §3.2: Edit mode outlines every object and candidate text block; default on (null storage included). */
let showOutlines = (() => { try { return localStorage.getItem("edit-outlines") !== "false"; } catch { return true; } })();
editOutlinesInput.checked = showOutlines;
editOutlinesInput.addEventListener("change", () => setOutlines(editOutlinesInput.checked));
function setOutlines(on: boolean): void {
  showOutlines = on; editOutlinesInput.checked = on;
  try { localStorage.setItem("edit-outlines", String(on)); } catch {}
  paintHits(canvas.clientWidth, canvas.clientHeight, shownPage);
}
function layerLive(): boolean {
  return ["read", "annotate", "forms", "edit", "redact"].includes(uiMode) && noteAddButton.getAttribute("aria-pressed") !== "true" && freeTextAddButton.getAttribute("aria-pressed") !== "true" && inkDrawButton.getAttribute("aria-pressed") !== "true" && redactAreaButton.getAttribute("aria-pressed") !== "true" && linkDrawButton.getAttribute("aria-pressed") !== "true" && sigfieldDrawButton.getAttribute("aria-pressed") !== "true" && !inkStroke && !(uiMode === "edit" && contentAddArmButton.getAttribute("aria-pressed") === "true") && !signArmed;
}
type DirectGesture = { kind: "move" | "resize" | "duplicate" | "crop"; handle: string; targetId: number | string; rect: number[]; guides?: SnapGuide[] };
let directGesture: DirectGesture | null = null;
let directGestureWork: Promise<void> | null = null;
function addTextHandles(button: HTMLButtonElement, item: EditTarget): void {
  const add = (name: string) => {
    const handle = document.createElement("span"); handle.className = `hit-handle ${name}`; handle.dataset.handle = name;
    handle.setAttribute("aria-hidden", "true"); handle.addEventListener("pointerdown", event => startTextGesture(event, item, name)); button.append(handle);
  };
  if (item.moveable || item.paragraphId) { const grip = document.createElement("span"); grip.className = "hit-handle hit-grip"; grip.dataset.handle = "grip"; grip.setAttribute("aria-hidden", "true"); grip.addEventListener("pointerdown", event => startTextGesture(event, item, "grip")); button.append(grip); }
  if (item.paragraphId) for (const name of ["e", "w"]) add(name);
  if (typeof item.fontSize === "number") for (const name of ["ne", "se", "sw", "nw"]) add(name);
}
function addFieldHandles(button: HTMLElement, start: (event: PointerEvent, handle: string) => void, grip = true): void {
  const add = (name: string, cls: string) => {
    const handle = document.createElement("span"); handle.className = `hit-handle ${cls}`; handle.dataset.handle = name;
    handle.setAttribute("aria-hidden", "true"); handle.addEventListener("pointerdown", event => start(event, name)); button.append(handle);
  };
  if (grip) add("grip", "hit-grip");
  for (const side of ["n", "ne", "e", "se", "s", "sw", "w", "nw"]) add(side, side);
}
/** The field-props commit a field gesture (or nudge) ends with. */
async function commitFieldRect(item: FieldItem, pageIndex: number, rect: number[]): Promise<boolean> {
  const reply = await request({ op: "field-props", page: pageIndex, widgetId: item.id, rect }) as MutationReply; await applyEdit(reply, ""); await refreshFields();
  return true;
}
function fieldGesture(item: FieldItem): (event: PointerEvent, handle: string) => void {
  return (event, handle) => { const pageIndex = wantedPage; startRectGesture(event, { rect: item.rect, id: item.id, handle, min: 8, lockAspect: false, can: canEdit, commit: rect => commitFieldRect(item, pageIndex, rect) }); };
}
async function nudgeField(item: FieldItem, event: KeyboardEvent): Promise<void> {
  if (!doc || !editable() || !permitted("edit")) return;
  const rect = [...item.rect], amount = !cmdKey(event) && event.shiftKey ? 10 : 1, pageIndex = wantedPage, page = doc.pages[pageIndex];
  const dx = event.key === "ArrowLeft" ? -amount : event.key === "ArrowRight" ? amount : 0;
  const dy = event.key === "ArrowUp" ? -amount : event.key === "ArrowDown" ? amount : 0;
  if (cmdKey(event)) { rect[2] = Math.max(rect[0] + 8, Math.min(page.width, rect[2] + dx)); rect[3] = Math.max(rect[1] + 8, Math.min(page.height, rect[3] + dy)); }
  else { const w = rect[2] - rect[0], h = rect[3] - rect[1]; rect[0] = Math.max(0, Math.min(page.width - w, rect[0] + dx)); rect[1] = Math.max(0, Math.min(page.height - h, rect[1] + dy)); rect[2] = rect[0] + w; rect[3] = rect[1] + h; }
  try { const reply = await request({ op: "field-props", page: pageIndex, widgetId: item.id, rect }) as MutationReply; await applyEdit(reply, ""); await refreshFields(); if (directActive?.kind === "field" && (directActive.item as FieldItem).id === item.id) directActive.item = { ...item, rect }; }
  catch (err) { showError(messageOf(err)); }
}
type RectGesture = { rect: number[]; id: number | string; handle: string; min: number; lockAspect: boolean; can: () => boolean; commit: (rect: number[]) => Promise<boolean>;
  /** P6 U5 §9.14: places the gesture rect (a group moves every member); unset = the pressed button. */
  place?: (rect: number[]) => void; duplicate?: boolean; done?: () => void;
  /** P6 U6 §9.13: the button to focus when the pressed element is not inside a `.hit-target` (group-box handles). */
  focus?: HTMLElement;
  /** §2.3: crop mode — the clamp box (view full ∩ page) and the gesture kind `crop`. */
  within?: number[]; crop?: boolean;
  /** §2.4/§9.20: snap to these view rects and the page (object gestures only; computed once at press). */
  snap?: number[][];
  /** draw-signature D6: the aspect stays locked under Shift (a drawn signature never stretches). */
  keepAspect?: boolean };
/** P6 U6 review R1: the click a drag's release makes (on pageRegion when it lands off the moved element) is not a click; it is
 * dispatched in the same task as pointerup, so the one-shot capture listener is dropped on the next tick if no click came. */
function swallowNextClick(): void {
  const swallow = (event: MouseEvent) => { event.stopPropagation(); event.preventDefault(); };
  window.addEventListener("click", swallow, { capture: true, once: true });
  setTimeout(() => window.removeEventListener("click", swallow, { capture: true }), 0);
}
/** P6 U3 §9.8: one move/resize gesture for fields, objects and redaction marks; the commit callback says whether it landed. */
function startRectGesture(event: PointerEvent, options: RectGesture): void {
  if (!doc || !options.can()) { options.done?.(); return; }
  event.preventDefault(); event.stopPropagation();
  const { handle, min, commit } = options;
  const pageIndex = wantedPage, initial = [...options.rect], startX = event.clientX, startY = event.clientY, button = options.focus ?? (event.currentTarget as HTMLElement).closest<HTMLElement>(".hit-target")!;
  // CSS px → page pt through the canvas box, as `pagePoint` maps a click (the canvas is whole pixels, shownZoom is not).
  const canvasBox = canvas.getBoundingClientRect(), pageSize = doc.pages[pageIndex];
  const ptX = canvasBox.width >= 1 ? pageSize.width / canvasBox.width : 1 / shownZoom, ptY = canvasBox.height >= 1 ? pageSize.height / canvasBox.height : 1 / shownZoom;
  button.focus();
  const place = options.place ?? ((rect: number[]) => placeBox(button, rect, shownZoom));
  let moved = false;
  // P6 U6 §2.4: guides are removed with the ghost on every exit (no move, commit's finally, Escape).
  const done = options.snap ? () => { hitLayer.querySelectorAll(".guide").forEach(node => node.remove()); options.done?.(); } : options.done;
  const update = (x: number, y: number, shift: boolean, meta = false) => {
    if (!moved && Math.hypot(x - startX, y - startY) < 3) return;
    moved = true;
    const page = doc!.pages[pageIndex];
    let dx = (x - startX) * ptX, dy = (y - startY) * ptY, guides: SnapGuide[] | undefined;
    if (options.snap) {
      // §9.19: snap the moved edges (grip: all; a handle: its own), then clamp through gestureRect with the snapped delta.
      const all = handle === "grip", west = handle.includes("w"), east = handle.includes("e"), north = handle.includes("n"), south = handle.includes("s");
      const edges = (all ? ["x0", "x1", "cx", "y0", "y1", "cy"] : [west && "x0", east && "x1", north && "y0", south && "y1"].filter(Boolean)) as SnapEdge[];
      const moving = [initial[0] + (all || west ? dx : 0), initial[1] + (all || north ? dy : 0), initial[2] + (all || east ? dx : 0), initial[3] + (all || south ? dy : 0)];
      const snapped = meta ? { rect: moving, guides: [] } : snapRect(moving, options.snap, { width: page.width, height: page.height }, 4, edges);
      dx += snapped.rect[0] - moving[0]; dy += snapped.rect[1] - moving[1];
      guides = snapped.guides;
    }
    const rect = gestureRect(initial, handle, dx, dy, { min, lockAspect: (options.keepAspect === true || options.lockAspect && !shift) && !options.within, page: [page.width, page.height], ...(options.within ? { within: options.within } : {}) });
    if (guides) {
      const at = (axis: "x" | "y") => axis === "x" ? [rect[0], rect[2], (rect[0] + rect[2]) / 2] : [rect[1], rect[3], (rect[1] + rect[3]) / 2];
      guides = guides.filter(guide => at(guide.axis).some(value => Math.abs(value - guide.at) <= 0.01));
      hitLayer.querySelectorAll(".guide").forEach(node => node.remove());
      for (const guide of guides) {
        const line = document.createElement("div"); line.className = `guide ${guide.axis}`; line.setAttribute("aria-hidden", "true");
        if (guide.axis === "x") line.style.left = `${guide.at * shownZoom}px`; else line.style.top = `${guide.at * shownZoom}px`;
        hitLayer.append(line);
      }
    }
    place(rect); directGesture = { kind: options.crop ? "crop" : options.duplicate ? "duplicate" : handle === "grip" ? "move" : "resize", handle, targetId: options.id, rect, ...(guides ? { guides } : {}) };
  };
  const move = (next: PointerEvent) => update(next.clientX, next.clientY, next.shiftKey, cmdKey(next));
  const end = async (next: PointerEvent) => {
    window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", end); window.removeEventListener("keydown", cancel);
    update(next.clientX, next.clientY, next.shiftKey, cmdKey(next)); const gesture = directGesture; directGesture = null;
    if (moved) swallowNextClick();
    if (!moved || !gesture || gesture.rect.every((n, i) => n === initial[i])) { place(initial); done?.(); return; }
    clearError();
    directGestureWork = (async () => {
      try { if (!await commit(gesture.rect)) place(initial); }
      catch (err) { showError(messageOf(err)); place(initial); }
      finally { done?.(); }
    })();
    await directGestureWork; directGestureWork = null;
  };
  const cancel = (next: KeyboardEvent) => { if (next.key !== "Escape") return; next.preventDefault(); window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", end); window.removeEventListener("keydown", cancel); directGesture = null; addBlocked = true; window.addEventListener("pointerup", swallowNextClick, { once: true }); place(initial); done?.(); };
  window.addEventListener("pointermove", move); window.addEventListener("pointerup", end); window.addEventListener("keydown", cancel);
}
async function applyParagraphEdit(scan: ContentScan, paragraph: ParagraphItem, text: string, transform: { width?: number; move?: { dx: number; dy: number }; size?: number; leading?: number } = {}): Promise<boolean> {
  try {
    const reply = await request({ op: "edit-paragraph", page: scan.page, paragraphId: paragraph.paragraphId, generation: scan.generation, revision: scan.revision, text, ...transform }) as MutationReply;
    await applyEdit(reply, ""); await refreshContentTargets(true);
    if (directActive?.paragraph && contentScan) {
      const dx = transform.move?.dx ?? 0, dy = transform.move?.dy ?? 0;
      const next = [...contentScan.paragraphs].sort((a, b) => Math.hypot(a.bounds[0] - paragraph.bounds[0] - dx, a.bounds[1] - paragraph.bounds[1] - dy) - Math.hypot(b.bounds[0] - paragraph.bounds[0] - dx, b.bounds[1] - paragraph.bounds[1] - dy))[0];
      if (next) { directActive.paragraph = next; directActive.item = contentScan.items.find(item => item.targetId === next.targetIds[0]) ?? directActive.item; directActive.scan = contentScan; directActive.editor.dataset.fontSize = String(next.fontSize); placeDirectActive(shownZoom); markActiveTextTarget(directActive); }
      else closeDirect();
    }
    return true;
  } catch (err) { showError(messageOf(err)); return false; }
}
async function applyTextTransform(scan: ContentScan, item: EditTarget, transform: { move?: { dx: number; dy: number }; size?: number }): Promise<boolean> {
  try {
    const reply = await request({ op: "edit-text", page: scan.page, targetId: item.targetId, generation: scan.generation, revision: scan.revision, text: item.text ?? "", ...transform }) as MutationReply;
    await applyEdit(reply, ""); await refreshContentTargets(true);
    if (directActive?.kind === "text" && contentScan) {
      const old = directActive.item as EditTarget, dx = transform.move?.dx ?? 0, dy = transform.move?.dy ?? 0;
      const next = contentScan.items.filter(item => item.kind === "text" && item.support === "candidate" && item.text === old.text && item.bounds)
        .sort((a, b) => Math.hypot(a.bounds![0] - old.bounds![0] - dx, a.bounds![1] - old.bounds![1] - dy) - Math.hypot(b.bounds![0] - old.bounds![0] - dx, b.bounds![1] - old.bounds![1] - dy))[0];
      if (next) { directActive.item = next; directActive.scan = contentScan; directActive.editor.dataset.fontSize = String(next.fontSize ?? transform.size ?? 12); placeDirectActive(shownZoom); markActiveTextTarget(directActive); }
    }
    return true;
  } catch (err) { showError(messageOf(err)); return false; }
}
function restoreTextGesture(button: HTMLElement, rect: number[]): void {
  placeBox(button, rect, shownZoom);
  if (directActive?.kind === "text") placeDirectActive(shownZoom);
}
function startTextGesture(event: PointerEvent, item: EditTarget, handle: string): void {
  if (!doc || !contentScan || !item.bounds || item.support !== "candidate") return;
  event.preventDefault(); event.stopPropagation();
  const scan = contentScan, pageIndex = wantedPage, initial = [...item.bounds], paragraph = item.paragraphId ? scan.paragraphs.find(value => value.paragraphId === item.paragraphId) : undefined;
  const initialSize = paragraph?.fontSize ?? item.fontSize ?? 0, initialWidth = paragraph?.width ?? 0;
  const startX = event.clientX, startY = event.clientY, button = (event.currentTarget as HTMLElement).closest<HTMLElement>(".text-target")!;
  const editingThisTarget = directActive?.kind === "text" && (paragraph
    ? directActive.paragraph?.paragraphId === paragraph.paragraphId
    : (directActive.item as EditTarget).targetId === item.targetId);
  if (!editingThisTarget) button.focus();
  let moved = false, size = initialSize, width = initialWidth;
  const update = (x: number, y: number) => {
    if (!moved && Math.hypot(x - startX, y - startY) < 3) return;
    moved = true; const dx = (x - startX) / shownZoom, dy = (y - startY) / shownZoom, rect = [...initial];
    if (handle === "grip") { rect[0] += dx; rect[1] += dy; rect[2] += dx; rect[3] += dy; }
    else if ((handle === "e" || handle === "w") && paragraph) { width = Math.max(8, initialWidth + (handle === "e" ? dx : -dx)); if (handle === "w") { rect[0] = initial[2] - width; rect[2] = initial[2]; } else rect[2] = rect[0] + width; }
    else { const startHeight = initial[3] - initial[1], change = handle.includes("n") ? -dy : dy; const ratio = Math.max(1 / 400, Math.min(400, (startHeight + change) / startHeight)); size = Math.max(1, Math.min(400, Math.round(initialSize * ratio * 2) / 2)); const height = startHeight * size / Math.max(1, initialSize); if (handle.includes("n")) rect[1] = initial[3] - height; else rect[3] = initial[1] + height; }
    placeBox(button, rect, shownZoom); if (directActive?.kind === "text") { placeBox(directActive.editor, rect, shownZoom); directActive.editor.style.fontSize = `${Math.max(10, Math.min(400, size * shownZoom))}px`; if (paragraph) { directActive.editor.style.width = `${width * shownZoom}px`; growParagraphEditor(directActive.editor, (rect[3] - rect[1]) * shownZoom); } }
  };
  const move = (next: PointerEvent) => update(next.clientX, next.clientY);
  const end = async (next: PointerEvent) => {
    window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", end); window.removeEventListener("keydown", cancel);
    update(next.clientX, next.clientY); const dx = (next.clientX - startX) / shownZoom, dy = (next.clientY - startY) / shownZoom;
    if (!moved) return;
    const currentScan = contentScan?.page === pageIndex ? contentScan : scan;
    const currentParagraph = paragraph && currentScan.paragraphs.find(value => value.paragraphId === paragraph.paragraphId) || paragraph;
    const currentItem = currentScan.items.find(value => value.targetId === item.targetId) ?? item;
    directGestureWork = (async () => {
      let accepted = true;
      if (currentParagraph) {
        const text = directActive?.paragraph?.paragraphId === currentParagraph.paragraphId ? directActive.editor.value : paragraphText(currentParagraph, currentScan);
        if (handle === "grip") accepted = await applyParagraphEdit(currentScan, currentParagraph, text, { move: { dx, dy } });
        else if (handle === "e" || handle === "w") accepted = await applyParagraphEdit(currentScan, currentParagraph, text, { width });
        else if (size !== initialSize) accepted = await applyParagraphEdit(currentScan, currentParagraph, text, { size });
        else restoreTextGesture(button, initial);
      } else if (handle === "grip") accepted = await applyTextTransform(currentScan, currentItem, { move: { dx, dy } });
      else if (size !== initialSize) accepted = await applyTextTransform(currentScan, currentItem, { size });
      else restoreTextGesture(button, initial);
      if (!accepted) restoreTextGesture(button, initial);
    })();
    await directGestureWork; directGestureWork = null;
  };
  const cancel = (next: KeyboardEvent) => { if (next.key !== "Escape") return; next.preventDefault(); window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", end); window.removeEventListener("keydown", cancel); restoreTextGesture(button, initial); };
  window.addEventListener("pointermove", move); window.addEventListener("pointerup", end); window.addEventListener("keydown", cancel);
}
function paintHits(width: number, height: number, page: number): void {
  const focusedField = (document.activeElement as HTMLElement | null)?.dataset?.fieldId;
  const refocus = !document.activeElement || document.activeElement === document.body || hitLayer.contains(document.activeElement);
  const liveLayer = layerLive();
  hitLayer.style.width = `${width}px`; hitLayer.style.height = `${height}px`; hitLayer.style.pointerEvents = "none"; hitLayer.classList.toggle("live", liveLayer); hitLayer.classList.toggle("outlined", uiMode === "edit" && showOutlines); hitLayer.querySelectorAll(".hit-target, .para-outline, .group-box, .crop-full, .guide").forEach(node => node.remove());
  const live = page === wantedPage && ["read", "annotate", "forms"].includes(uiMode) ? fields : [];
  if (directActive?.page === page) placeDirectActive(shownZoom);
  fieldBar.hidden = !doc || fields.length === 0; fieldCount.textContent = `This page has ${fields.length} form field${fields.length === 1 ? "" : "s"}.`;
  const radioFocus = new Map(readingOrder(live).filter(item => item.kind === "radio").map(item => [item.name, item.id]));
  // P6 U4 §1.4/§9.5: links before fields, so a field over a link stays on top (fill is the primary job).
  if ((uiMode === "read" || uiMode === "annotate") && linksPage === page && page === wantedPage) for (const item of links) if (item.rect.length === 4) hitLayer.append(paintLinkTarget(item, page));
  // fill-anywhere D4: text boxes before fields, so a field over a box stays on top (fields keep every behaviour).
  if (uiMode === "forms" && page === wantedPage && doc && (doc.rotations?.[page] ?? 0) % 360 === 0) {
    let index = 0;
    for (const item of notes) if (item.type === "FreeText" && item.editable && item.rect.length === 4) hitLayer.append(paintTextBoxTarget(item, page, index++));
    let signIndex = 0;
    for (const item of notes) if (isSignature(item) && item.editable && item.rect.length === 4) hitLayer.append(paintSignatureTarget(item, page, signIndex++));
  }
  for (const item of [...live].sort((a, b) => a.rect[1] - b.rect[1] || a.rect[0] - b.rect[0])) {
    if (item.kind === "signature" || item.rect.length !== 4) continue;
    const button = document.createElement("button"); button.type = "button"; button.className = `hit-target${highlightFields ? " highlighted" : ""}${item.required ? " required" : ""}${item.readOnly ? " readonly" : ""}`;
    placeBox(button, item.rect, shownZoom); button.setAttribute("aria-label", fieldLabel(item)); button.setAttribute("role", item.kind === "text" ? "textbox" : "button");
    button.dataset.fieldId = String(item.id);
    if (!editable() || !permitted("edit")) button.title = permissionTitle(false) || "This document is read-only";
    else if (item.tu) button.title = item.tu;
    if (item.kind === "radio") button.tabIndex = radioFocus.get(item.name) === item.id ? 0 : -1;
    if (editable() && permitted("edit")) addFieldHandles(button, fieldGesture(item));
    button.addEventListener("click", event => {
      event.preventDefault(); if ((event.target as HTMLElement).closest(".hit-handle, .hit-grip") || event.ctrlKey) return;
      // P6 U7 §9.21: in Fill forms a Shift-click adds or removes the field (no editor, no toggle).
      if (event.shiftKey && uiMode === "forms") { directClickWork = shiftSelect(memberOfNode(button)); return; }
      if (directSelection.length) dropDirectFocus();
      button.focus(); directClickWork = activateField(item);
    });
    button.addEventListener("keydown", event => {
      // P6 U4 §9.12: ⌥-arrow nudges; radio plain ←/→ moves the selection; a checkbox/radio in Forms nudges on other plain arrows; the rest scroll.
      const arrow = ARROW_KEYS.includes(event.key), plain = arrow && !event.altKey && !event.metaKey && !event.ctrlKey;
      if (arrow && event.altKey) { event.preventDefault(); if (!directGestureWork) runGestureWork(() => nudgeField(item, event)); }
      else if (event.key === "Tab") { event.preventDefault(); void focusNextField(event.shiftKey, wantedPage, directFieldIndex); }
      else if (item.kind === "radio" && plain && (event.key === "ArrowLeft" || event.key === "ArrowRight")) { event.preventDefault(); if (!directGestureWork) runGestureWork(() => moveRadio(item, event.key === "ArrowRight" ? 1 : -1)); }
      else if (plain && uiMode === "forms" && (item.kind === "checkbox" || item.kind === "radio")) { event.preventDefault(); if (!directGestureWork) runGestureWork(() => nudgeField(item, event)); }
      else if (event.key === " " && (item.kind === "radio" || item.kind === "checkbox")) { event.preventDefault(); void commitField(item, item.kind === "checkbox" ? item.value !== true : true); }
    });
    hitLayer.append(button);
  }
  for (const item of live) {
    if (item.kind !== "signature" || item.rect.length !== 4) continue;
    // P5 U3 §2.1/§9.16/§9.17/§9.26: an empty field is a "Sign" target, a signed one carries its badge.
    const signed = sigSigned(item), status = sigStatus(item), label = `Signature field ${item.name}, ${status}`;
    const button = document.createElement("button"); button.type = "button";
    button.className = `hit-target sig-target${["valid", "invalid", "unsupported"].includes(status) ? ` ${status}` : ""}${status === "valid" && !sigTrusted(item.name) ? " untrusted" : ""}`;
    button.dataset.fieldName = item.name; button.dataset.signed = String(signed); button.dataset.fieldId = String(item.id);
    placeBox(button, item.rect, shownZoom); button.setAttribute("aria-label", label); button.title = label;
    const movable = editable() && permitted("edit") && !signed;
    if (movable) addFieldHandles(button, fieldGesture(item));
    // Debt1 row 13: in Forms a body drag (≥ 3 px, startRectGesture's threshold) moves the field; the click that trails a drag is not a sign.
    let pressAt: [number, number] | null = null;
    button.addEventListener("pointerdown", event => {
      event.preventDefault(); pressAt = null;
      if (!movable || uiMode !== "forms" || event.button !== 0 || (event.target as HTMLElement).closest(".hit-handle")) return;
      pressAt = [event.clientX, event.clientY]; fieldGesture(item)(event, "grip");
    });
    button.addEventListener("click", event => {
      event.preventDefault(); const press = pressAt; pressAt = null;
      if ((event.target as HTMLElement).closest(".hit-handle, .hit-grip")) return;
      if (press && Math.hypot(event.clientX - press[0], event.clientY - press[1]) >= 3) return;
      button.focus(); directClickWork = activateField(item);
    });
    button.addEventListener("keydown", event => {
      const arrow = ARROW_KEYS.includes(event.key), plain = arrow && !event.altKey && !event.metaKey && !event.ctrlKey;
      if (!signed && ((arrow && event.altKey) || (plain && uiMode === "forms"))) { event.preventDefault(); if (!directGestureWork) runGestureWork(() => nudgeField(item, event)); }
      else if (event.key === "Enter" || event.key === " ") { event.preventDefault(); if (!directGestureWork) runGestureWork(() => activateField(item)); }
    });
    hitLayer.append(button);
  }
  // A repaint keeps focus where it was without scrolling the view back to it (a drive click scrolls first).
  if (focusedField) hitLayer.querySelector<HTMLElement>(`[data-field-id="${CSS.escape(focusedField)}"]`)?.focus({ preventScroll: true });
  if (uiMode === "edit" && contentScan?.page === page) {
    for (const paragraph of contentScan.paragraphs) { const outline = document.createElement("div"); outline.className = "para-outline"; outline.hidden = true; outline.dataset.paragraphId = paragraph.paragraphId; placeBox(outline, paragraph.bounds, shownZoom); hitLayer.append(outline); }
    if (page === wantedPage) for (const item of objectTargetItems(contentScan)) hitLayer.append(paintObjectTarget(item, contentScan));
    const paintedParagraphs = new Set<string>();
    for (const item of contentScan.items) {
    if (item.kind !== "text" || !item.bounds || item.bounds.length !== 4) continue;
    if (item.paragraphId && paintedParagraphs.has(item.paragraphId)) continue;
    if (item.paragraphId) paintedParagraphs.add(item.paragraphId);
    const paragraph = item.paragraphId ? contentScan.paragraphs.find(value => value.paragraphId === item.paragraphId) : undefined;
    const button = document.createElement("button"); button.type = "button"; button.className = `hit-target text-target${item.paragraphId ? " paragraph" : ""}${item.support === "candidate" ? "" : " unsupported"}`;
    button.dataset.targetId = String(item.targetId); if (item.paragraphId) button.dataset.paragraphId = item.paragraphId;
    button.classList.toggle("active", directActive?.kind === "text" && (directActive.paragraph ? directActive.paragraph.paragraphId === item.paragraphId : (directActive.item as EditTarget).targetId === item.targetId));
    placeBox(button, paragraph?.bounds ?? item.bounds, shownZoom); button.setAttribute("aria-label", paragraph ? `Paragraph, ${paragraph.lines} lines` : item.text || item.reason || "Text block"); button.title = item.support === "candidate" ? "Edit text" : CONTENT_REASON_TEXT[item.reason ?? ""] ?? "Not editable";
    if (item.support === "candidate") { addTextHandles(button, item); if (!item.moveable && !item.paragraphId) button.title = `This text cannot be moved (${item.moveReason ?? "reason"})`; }
    if (item.paragraphId) { button.addEventListener("pointerenter", () => { const outline = hitLayer.querySelector<HTMLElement>(`.para-outline[data-paragraph-id="${CSS.escape(item.paragraphId!)}"]`); if (outline) outline.hidden = false; }); button.addEventListener("pointerleave", () => { const outline = hitLayer.querySelector<HTMLElement>(`.para-outline[data-paragraph-id="${CSS.escape(item.paragraphId!)}"]`); if (outline) outline.hidden = true; }); }
    button.addEventListener("click", event => { event.preventDefault(); if ((event.target as HTMLElement).closest(".hit-handle, .hit-grip") || event.ctrlKey) return; if (event.shiftKey) { directClickWork = shiftSelect(memberOfNode(button)); return; } button.focus(); if (item.support === "candidate") { openInlineText(item, event.altKey); directClickWork = Promise.resolve(); } else { const i = contentScan?.items.indexOf(item) ?? -1; if (i >= 0) selectContentRow(i); announce(button.title); directClickWork = Promise.resolve(); } }); hitLayer.append(button);
  }
  }
  if (uiMode === "redact" && page === wantedPage) notes.forEach((item, index) => { if (item.type === "Redact" && item.rect.length === 4) hitLayer.append(paintMarkTarget(item, page, index)); });
  if (page === wantedPage) restoreDirectFocus(refocus);
  if (page === wantedPage && directFocusedBox !== null && refocus && !directActive) hitLayer.querySelector<HTMLElement>(`.textbox-target[data-annot-id="${directFocusedBox}"]`)?.focus({ preventScroll: true });
  if (page === wantedPage && directFocusedSig !== null && refocus && !directActive) hitLayer.querySelector<HTMLElement>(`.signature-target[data-annot-id="${directFocusedSig}"]`)?.focus({ preventScroll: true });
  // §9.16: a paint of another page or zoom, or a rescan that lost the anchor, closes the menu; otherwise it re-derives its items.
  if (!objectMenu.hidden) {
    const anchor = directSelection.at(-1);
    if (!objectMenuAt || objectMenuAt.page !== page || objectMenuAt.zoom !== shownZoom || (objectMenuKind === "selection" && (!anchor || !memberNode(anchor)))) closeObjectMenu(false);
    else syncObjectMenu();
  }
}
const ARROW_KEYS = ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"];
/** P5 U3 §1.1: the engine's `signed` flag (the widget's own `/V`). */
function sigSigned(item: FieldItem): boolean { return (item as FieldItem & { signed?: boolean }).signed === true; }
/** P5 U3 §2.5: empty → signed → the last Verify row status for this field. */
function sigStatus(item: FieldItem): "empty" | "signed" | "valid" | "invalid" | "unsupported" {
  if (!sigSigned(item)) return "empty";
  const status = verifyByField.get(item.name);
  return status === "valid" || status === "invalid" || status === "unsupported" ? status : "signed";
}
/** The badge colour alone shows trust (§9.34): the last Verify row's trust for that field. */
function sigTrusted(name: string): boolean {
  const rows = (verifyLast?.signatures ?? []) as { field?: unknown; trust?: unknown }[];
  return rows.some(row => row.field === name && row.trust === "trusted");
}
/** STATE `signatures[]`: the shown page's signature fields in `fields` order. */
function signaturesSummary(): { field: string; rect: number[]; status: string }[] {
  if (!doc) return [];
  return fields.filter(item => item.kind === "signature").map(item => ({ field: item.name, rect: item.rect, status: sigStatus(item) }));
}
function rectDistance(a: number[], b: number[]): number { return Math.hypot(a[0] - b[0], a[1] - b[1]) + Math.hypot(a[2] - b[2], a[3] - b[3]); }
/** P6 U3 §1.4: movable Image/Form placements get an on-page target; one covering ≥ 90 % of the page stays panel-only. */
function objectTargetItems(scan: ContentScan): EditTarget[] {
  const size = doc?.pages[scan.page], m = scan.transform;
  if (!size || !m) return [];
  return scan.items.filter(item => {
    if (item.kind === "text" || item.support !== "candidate" || !item.bounds || !item.placement?.box || !(item.objectOps ?? []).includes("move")) return false;
    const [x0, y0, x1, y1] = viewBox(item.placement.box, m);
    const area = Math.max(0, Math.min(x1, size.width) - Math.max(x0, 0)) * Math.max(0, Math.min(y1, size.height) - Math.max(y0, 0));
    return area < 0.9 * size.width * size.height;
  });
}
/** §9.6: the same object after a commit (targetIds change on a cm rewrite): same id + kind + name, else the nearest of that kind + name. */
function resolveObject(scan: ContentScan, item: EditTarget, rect: number[]): EditTarget | null {
  const same = scan.items.filter(value => value.kind === item.kind && (value.objectName ?? "") === (item.objectName ?? "") && value.placement?.box);
  const exact = same.find(value => value.targetId === item.targetId);
  if (exact || !scan.transform) return exact ?? null;
  return same.map(value => ({ value, distance: rectDistance(viewBox(value.placement!.box, scan.transform!), rect) })).sort((a, b) => a.distance - b.distance)[0]?.value ?? null;
}
function markSelected(button: HTMLElement | null): void {
  hitLayer.querySelectorAll(".hit-target.selected").forEach(node => node.classList.remove("selected"));
  hitLayer.querySelectorAll(".group-box, .crop-full").forEach(node => node.remove());
  button?.classList.add("selected");
}
/** P6 U5 §2.1: every member is `.selected`; a selection of two or more is also `.multi` (no handles). */
function markSelection(): void {
  hitLayer.querySelectorAll(".hit-target.selected, .hit-target.multi").forEach(node => node.classList.remove("selected", "multi"));
  hitLayer.querySelectorAll(".group-box").forEach(node => node.remove());
  // P6 U7 §9.19: every member kind (objects, runs, fields, links, marks) by its own node.
  const nodes = selectionMembers().map(entry => entry.node);
  for (const node of nodes) { node.classList.add("selected"); node.classList.toggle("multi", nodes.length >= 2); }
  if (nodes.length >= 2) paintGroupBox();
}
/** P6 U6 §2.2/§9.13/§9.16: one unclassed frame on the union with 8 handles (no grip); never an `.object-target`. */
function paintGroupBox(): void {
  const targets = selectionTargets(), members = selectionMembers();
  if (members.length < 2) return;
  const box = document.createElement("div"); box.className = "group-box"; box.setAttribute("aria-hidden", "true");
  placeBox(box, unionRect(members.map(entry => entry.rect)), shownZoom);
  // P6 U7 §9.19: handles only when every member is an object (the U6 group resize stays objects-only).
  if (targets && targets.length >= 2 && !selectionMixed() && canEdit()) addFieldHandles(box, (event, handle) => startGroupResize(event, handle), false);
  hitLayer.append(box);
}
/** §9.19: the union's smallest side keeps every member at 1 pt or more. */
function groupMin(targets: ObjectTargets, union: number[]): number {
  return Math.max(1, ...targets.map(target => Math.max((union[2] - union[0]) / Math.max(1e-6, target.rect[2] - target.rect[0]), (union[3] - union[1]) / Math.max(1e-6, target.rect[3] - target.rect[1]))));
}
/** §2.2: `rect` (inside `from`) under the axis-aligned affine that maps `from` onto `to`. */
export function mapRect(rect: number[], from: number[], to: number[]): number[] {
  const sx = (to[2] - to[0]) / ((from[2] - from[0]) || 1), sy = (to[3] - to[1]) / ((from[3] - from[1]) || 1);
  return [to[0] + (rect[0] - from[0]) * sx, to[1] + (rect[1] - from[1]) * sy, to[0] + (rect[2] - from[0]) * sx, to[1] + (rect[3] - from[1]) * sy];
}
/** §2.1: `rect` turned 90° or flipped about (cx, cy) in view space (y down; clockwise is (x, y) → (−y, x) about the centre). */
export function turnRect(rect: number[], which: "rotateCw" | "rotateCcw" | "flipH" | "flipV", cx: number, cy: number): number[] {
  const [x0, y0, x1, y1] = rect;
  if (which === "rotateCw") return [cx - (y1 - cy), cy + (x0 - cx), cx - (y0 - cy), cy + (x1 - cx)];
  if (which === "rotateCcw") return [cx + (y0 - cy), cy - (x1 - cx), cx + (y1 - cy), cy - (x0 - cx)];
  if (which === "flipH") return [2 * cx - x1, y0, 2 * cx - x0, y1];
  return [x0, 2 * cy - y1, x1, 2 * cy - y0];
}
/** §9.20: snap sources — every painted object's view rect except the gesture's own members, taken once at press. */
function snapOthers(exclude: string[]): number[][] {
  const scan = contentScan, m = scan?.transform;
  if (!scan || !m || scan.page !== wantedPage) return [];
  return objectTargetItems(scan).filter(item => !exclude.includes(item.targetId)).map(item => viewBox(item.placement!.box, m));
}
function objectNode(targetId: string): HTMLElement | null {
  return hitLayer.querySelector<HTMLElement>(`.object-target[data-target-id="${CSS.escape(targetId)}"]`);
}
/** The selection's painted members with their scan items and view rects; null when any member is not painted. */
function selectionTargets(): { member: ObjectFocus; item: EditTarget; rect: number[]; node: HTMLElement }[] | null {
  const scan = contentScan, m = scan?.transform;
  if (!scan || !m || scan.page !== wantedPage || !directSelection.length) return null;
  const out: { member: ObjectFocus; item: EditTarget; rect: number[]; node: HTMLElement }[] = [];
  for (const member of directSelection) {
    const item = scan.items.find(value => value.targetId === member.targetId), node = objectNode(member.targetId);
    if (!item?.placement?.box || !node) return null;
    out.push({ member, item, rect: viewBox(item.placement.box, m), node });
  }
  return out;
}
export function unionRect(rects: number[][]): number[] {
  return [Math.min(...rects.map(r => r[0])), Math.min(...rects.map(r => r[1])), Math.max(...rects.map(r => r[2])), Math.max(...rects.map(r => r[3]))];
}
function shiftRect(rect: number[], dx: number, dy: number): number[] { return [rect[0] + dx, rect[1] + dy, rect[2] + dx, rect[3] + dy]; }
// ------------------------------------------------------------------ P6 U7 selection members (§2.2, §9.19–§9.33, §10.29–§10.39)

function isObjectMember(member: ObjectFocus): boolean { return member.kind === "image" || member.kind === "form"; }
/** §10.29: the one mode whose layer paints a member kind. */
function memberMode(kind: string): string { return kind === "field" ? "forms" : kind === "link" ? "annotate" : kind === "mark" ? "redact" : "edit"; }
function textMember(item: EditTarget): ObjectFocus { return { kind: "text", name: (item.text ?? "").slice(0, 12), targetId: item.targetId, rect: [...(item.bounds ?? [])], text: item.text ?? "" }; }
function fieldMember(item: FieldItem): ObjectFocus { return { kind: "field", name: item.name, targetId: String(item.id), rect: [...item.rect] }; }
function linkMember(item: LinkEntry): ObjectFocus { const id = linkAnnotId(item); return { kind: "link", name: `link ${id}`, targetId: String(id), rect: [...item.rect] }; }
function markMember(item: AnnotItem): ObjectFocus { return { kind: "mark", name: `mark ${item.id}`, targetId: String(item.id), rect: [...item.rect] }; }
function sameMember(a: ObjectFocus, b: ObjectFocus): boolean { return a.kind === b.kind && a.targetId === b.targetId; }
function selectionMixed(): boolean { return directSelection.some(member => !isObjectMember(member)); }
function memberNode(member: ObjectFocus): HTMLElement | null {
  if (isObjectMember(member)) return objectNode(member.targetId);
  const id = CSS.escape(member.targetId);
  if (member.kind === "text") return hitLayer.querySelector<HTMLElement>(`.text-target[data-target-id="${id}"]`);
  if (member.kind === "field") return hitLayer.querySelector<HTMLElement>(`[data-field-id="${id}"]:not(.sig-target)`);
  return hitLayer.querySelector<HTMLElement>(`${member.kind === "link" ? ".link-target" : ".redact-target"}[data-annot-id="${id}"]`);
}
/** The member's current view rect (y-down page pt) from the data its layer paints; null when it is gone. */
function memberRect(member: ObjectFocus): number[] | null {
  if (isObjectMember(member) || member.kind === "text") {
    const scan = contentScan, item = scan?.page === wantedPage ? scan.items.find(value => value.targetId === member.targetId) : undefined;
    if (!scan || !item) return null;
    if (member.kind === "text") return item.bounds?.length === 4 ? item.bounds : null;
    return item.placement?.box && scan.transform ? viewBox(item.placement.box, scan.transform) : null;
  }
  const id = Number(member.targetId);
  if (member.kind === "field") return fields.find(item => item.id === id)?.rect ?? null;
  if (member.kind === "link") return links.find(item => linkAnnotId(item) === id)?.rect ?? null;
  return notes.find(item => item.id === id && item.type === "Redact")?.rect ?? null;
}
/** §9.19: every painted member with its view rect and node, in click order. */
function selectionMembers(): { member: ObjectFocus; rect: number[]; node: HTMLElement }[] {
  const out: { member: ObjectFocus; rect: number[]; node: HTMLElement }[] = [];
  for (const member of directSelection) { const node = memberNode(member), rect = node ? memberRect(member) : null; if (node && rect) out.push({ member, rect, node }); }
  return out;
}
/** The member a painted target stands for, in its own mode only (§10.29); null for paragraphs, unsupported runs, signatures. */
function memberOfNode(node: HTMLElement): ObjectFocus | null {
  if (node.classList.contains("text-target")) {
    const item = uiMode === "edit" && contentScan?.page === wantedPage ? contentScan.items.find(value => value.targetId === node.dataset.targetId) : undefined;
    return item && item.kind === "text" && !item.paragraphId && item.support === "candidate" && item.moveable && item.bounds?.length === 4 ? textMember(item) : null;
  }
  if (node.dataset.fieldId && !node.classList.contains("sig-target")) {
    const item = uiMode === "forms" ? fields.find(value => String(value.id) === node.dataset.fieldId) : undefined;
    return item && item.kind !== "signature" ? fieldMember(item) : null;
  }
  const annotId = Number(node.dataset.annotId);
  if (node.classList.contains("link-target")) {
    const item = uiMode === "annotate" && annotId > 0 ? links.find(value => linkAnnotId(value) === annotId) : undefined;
    return item ? linkMember(item) : null;
  }
  if (node.classList.contains("redact-target")) {
    const item = uiMode === "redact" ? notes.find(value => value.id === annotId && value.type === "Redact") : undefined;
    return item ? markMember(item) : null;
  }
  return null;
}
/** Whether this session may delete the member (runs never are, and need nothing: Cut leaves them, §10.37). */
function memberDeletable(member: ObjectFocus): boolean {
  if (member.kind === "text") return true;
  if (isObjectMember(member)) { const item = contentScan?.items.find(value => value.targetId === member.targetId); return canEdit() && !!item && hasOp(item, "delete"); }
  return member.kind === "field" ? canDesign() : canAnnotate();
}
/** §9.31: the gate a member's own move gesture checks. */
function memberCan(member: ObjectFocus): boolean {
  return isObjectMember(member) || member.kind === "text" ? canEdit() : member.kind === "field" ? canDesign() : canAnnotate();
}
/** The anchor (last member) drives the single focus of its kind; `directFocusedObject` stays object-only (§9.19). */
function setSelection(members: ObjectFocus[]): void {
  directSelection = members;
  const anchor = members.at(-1);
  directFocusedObject = anchor && isObjectMember(anchor) ? anchor : null;
  directFocusedLink = anchor?.kind === "link" ? { annotId: Number(anchor.targetId) } : null;
  directFocusedMark = anchor?.kind === "mark" ? { annotId: Number(anchor.targetId) } : null;
}
/** §9.33: the anchor is mirrored without opening an editor. */
function syncAnchor(anchor: ObjectFocus): void {
  if (isObjectMember(anchor)) syncObjectInspector(anchor.targetId);
  else if (anchor.kind === "text") { const index = contentScan?.items.findIndex(value => value.targetId === anchor.targetId) ?? -1; if (index >= 0) selectContentRow(index, false); }
  else if (anchor.kind === "field") for (const row of fieldListEl.querySelectorAll<HTMLElement>("[data-field-id]")) { if (row.dataset.fieldId === anchor.targetId) row.setAttribute("aria-current", "true"); else row.removeAttribute("aria-current"); }
  syncNoteCurrent(); syncLinkCurrent();
}
/** §9.21: Shift-click adds a member (it becomes the anchor) or removes one; never an editor, a toggle or a link follow. */
function toggleMember(member: ObjectFocus): void {
  leaveCrop();
  const kept = directSelection.filter(value => memberMode(value.kind) === uiMode);
  const next = kept.some(value => sameMember(value, member)) ? kept.filter(value => !sameMember(value, member)) : [...kept, member];
  if (!next.length) { dropDirectFocus(); pageRegion.focus({ preventScroll: true }); return; }
  setSelection(next);
  const anchor = next[next.length - 1];
  markSelection(); memberNode(anchor)?.focus({ preventScroll: true }); syncAnchor(anchor);
}
/** §9.21/§9.22: a Shift-click on a run, field, link or mark; an open run or field editor commits first and seeds the selection. */
function shiftSelect(member: ObjectFocus | null): Promise<void> {
  if (!member) { announce("Paragraphs cannot join a selection"); return Promise.resolve(); }
  const active = directActive;
  if (!active) { toggleMember(member); return Promise.resolve(); }
  const seed = active.kind === "field" ? fieldMember(active.item as FieldItem) : active.kind === "text" && !active.paragraph ? textMember(active.item as EditTarget) : null;
  return (async () => {
    if (directFinishWork) await directFinishWork; else if (directActive === active) await finishDirect();
    if (seed && memberMode(seed.kind) === uiMode && !sameMember(seed, member)) { clearDirectFocus(); setSelection([seed]); }
    toggleMember(member);
  })();
}
/** §10.39: re-resolve a selection holding a non-object member after a repaint; a member whose layer is painted but has no target drops. */
function resolveMembers(): void {
  const scan = contentScan, used = new Set<string>(), next: ObjectFocus[] = [];
  // P6 U7 review F1: a scan from before the write is stale; keep the member (and its moved rect) until the new scan lands.
  const contentReady = scan?.page === wantedPage && scan.revision === revision && !!scan.transform;
  for (const member of directSelection) {
    if (memberMode(member.kind) !== uiMode) continue;
    if (isObjectMember(member) || member.kind === "text") {
      if (!scan || !contentReady) { next.push(member); continue; }
      const painted = (item: EditTarget) => !used.has(item.targetId) && !!memberNode({ ...member, targetId: item.targetId });
      let found: EditTarget | undefined;
      if (member.kind === "text") {
        const near = (item: EditTarget) => item.kind === "text" && item.bounds?.length === 4 && item.bounds.every((value, index) => Math.abs(value - member.rect[index]) <= 1.01);
        found = scan.items.find(item => item.targetId === member.targetId && item.kind === "text" && painted(item))
          ?? scan.items.find(item => near(item) && item.text === member.text && painted(item));
      } else {
        const m = scan.transform!;
        found = scan.items.find(item => item.targetId === member.targetId && item.placement?.box && painted(item))
          ?? scan.items.filter(item => item.kind === member.kind && (item.objectName ?? "") === member.name && item.placement?.box && painted(item))
            .map(item => ({ item, distance: rectDistance(viewBox(item.placement!.box, m), member.rect) })).sort((a, b) => a.distance - b.distance)[0]?.item;
      }
      if (!found) continue;
      used.add(found.targetId);
      next.push(member.kind === "text" ? textMember(found) : { ...member, targetId: found.targetId, rect: viewBox(found.placement!.box, scan.transform!) });
      continue;
    }
    const ready = member.kind === "field" ? fieldsPage === wantedPage : member.kind === "link" ? linksPage === wantedPage : true;
    const rect = memberNode(member) ? memberRect(member) : null;
    if (rect) next.push({ ...member, rect: [...rect] }); else if (!ready) next.push(member);
  }
  setSelection(next);
}
/** §9.31: a body drag on any member of a selection of two or more moves every member by one clamped delta. */
function startSelectionGesture(event: PointerEvent, node: HTMLElement): void {
  const members = selectionMembers(), anchor = directSelection.at(-1);
  if (members.length < 2 || !anchor) return;
  const union = unionRect(members.map(entry => entry.rect)), box = hitLayer.querySelector<HTMLElement>(".group-box");
  const place = (rect: number[]) => { for (const entry of members) placeBox(entry.node, shiftRect(entry.rect, rect[0] - union[0], rect[1] - union[1]), shownZoom); if (box) placeBox(box, rect, shownZoom); };
  startRectGesture(event, { rect: union, id: anchor.targetId, handle: "grip", min: 1, lockAspect: false, focus: node, place,
    can: () => members.every(entry => memberCan(entry.member)),
    snap: snapOthers(members.map(entry => entry.member.targetId)),
    commit: rect => commitSelectionMove(members, rect[0] - union[0], rect[1] - union[1]) });
}
/** §9.30: arrows move the whole selection by 1 pt (Shift 10 pt), clamped as one union. */
function nudgeSelection(event: { key: string; shiftKey: boolean }): Promise<boolean> {
  const members = selectionMembers(), size = doc?.pages[wantedPage];
  if (members.length < 2 || !size) return Promise.resolve(false);
  const union = unionRect(members.map(entry => entry.rect)), next = nudgeRect(union, { key: event.key, shiftKey: event.shiftKey, metaKey: false }, 1, size);
  return commitSelectionMove(members, next[0] - union[0], next[1] - union[1]);
}
/** §2.3/§9.32: one `edit-batch` moving every member by the same view delta. */
async function commitSelectionMove(members: { member: ObjectFocus; rect: number[] }[], dx: number, dy: number): Promise<boolean> {
  const scan = contentScan, size = doc?.pages[wantedPage];
  if (!size || (dx === 0 && dy === 0) || !members.every(entry => memberCan(entry.member))) return false;
  const round = (rect: number[]) => rect.map(value => Math.round(value * 1e4) / 1e4);
  const items: Record<string, unknown>[] = [];
  for (const { member, rect } of members) {
    const id = Number(member.targetId), moved = shiftRect(rect, dx, dy);
    if (isObjectMember(member)) { if (!scan?.transform) return false; items.push({ op: "edit-object", targetId: member.targetId, action: "move", box: objectBox(scan, size, moved) }); }
    else if (member.kind === "text") items.push({ op: "edit-text", targetId: member.targetId, move: { dx, dy } });
    else if (member.kind === "field") items.push({ op: "field-props", widgetId: id, rect: round(moved) });
    else if (member.kind === "link") items.push({ op: "link-set", annotId: id, rect: round(moved) });
    else items.push({ op: "redact-set", annotId: id, rect: round(moved) });
  }
  // §9.13: every member's rect is its requested rect before the reply repaints (runs re-resolve within 1 pt of it).
  const previous = directSelection;
  setSelection(directSelection.map(value => { const entry = members.find(candidate => candidate.member === value); return entry ? { ...value, rect: shiftRect(entry.rect, dx, dy) } : value; }));
  if (await applySelectionBatch(items, "move", `Moved ${members.length} items`, members.length, directSelection.at(-1)?.name ?? "")) return true;
  setSelection(previous); restoreDirectFocus(false);
  return false;
}
const RUN_DELETE_TEXT = "Text runs cannot be deleted; deselect them first or use Cut";
/** §2.3/§9.40/§10.42: Delete on a selection with a non-object member; a run refuses on the page with no request. */
function deleteSelection(): Promise<boolean> {
  if (directSelection.some(member => member.kind === "text")) { showError(RUN_DELETE_TEXT); return Promise.resolve(false); }
  const count = directSelection.length;
  return deleteMembers(directSelection.slice(), `Deleted ${count} item${count === 1 ? "" : "s"}`);
}
/** §10.21: one delete per member (a radio: every kid of its group); one item is its single op, more are one `edit-batch`. */
async function deleteMembers(members: ObjectFocus[], announcement: string): Promise<boolean> {
  if (!members.length || !members.every(memberDeletable)) return false;
  const items: Record<string, unknown>[] = [];
  for (const member of members) {
    const id = Number(member.targetId);
    if (isObjectMember(member)) items.push({ op: "edit-object", targetId: member.targetId, action: "delete" });
    else if (member.kind === "field") {
      const field = fields.find(value => value.id === id);
      for (const kid of field?.kind === "radio" ? fields.filter(value => value.kind === "radio" && value.name === field.name) : field ? [field] : []) items.push({ op: "field-delete", widgetId: kid.id });
    } else if (member.kind === "link") items.push({ op: "link-delete", annotId: id });
    else if (member.kind === "mark") items.push({ op: "annot-delete", annotId: id });
  }
  const previous = directSelection, name = members[members.length - 1].name;
  setSelection([]);
  if (await applySelectionBatch(items, "delete", announcement, members.length, name)) return true;
  setSelection(previous); restoreDirectFocus(false);
  return false;
}
/** §9.32: one journal op for a selection's moves or deletes; `content.edited {kind:'selection'}`; ONE restore after the refreshes. */
async function applySelectionBatch(items: Record<string, unknown>[], action: "move" | "delete", announcement: string, count: number, name: string): Promise<boolean> {
  const page = wantedPage, scan = contentScan, content = items.some(item => item.op === "edit-text" || item.op === "edit-object");
  if (!items.length || (content && (!scan || scan.page !== page))) return false;
  const stamp = content && scan ? { generation: scan.generation, revision: scan.revision } : { revision };
  let payload: Record<string, unknown>;
  if (items.length >= 2) payload = { op: "edit-batch", page, ...stamp, items };
  else {
    const { op, ...rest } = items[0];
    if (op === "link-delete") { const entry = links.find(value => linkAnnotId(value) === rest.annotId); if (!entry) return false; payload = { op, page, linkId: entry.id }; }
    else payload = { op, page, ...(op === "edit-object" ? stamp : {}), ...rest };
  }
  let reply: Record<string, unknown>;
  try { reply = (await request(payload)) as unknown as Record<string, unknown>; }
  catch (err) { showClipRefusal(err, name); paintHits(canvas.clientWidth, canvas.clientHeight, shownPage); return false; }
  if (reply.noop === true) return true;
  contentEdited = { kind: "selection", action, name, count };
  await applyEdit(reply as unknown as JournalState, announcement);
  if (uiMode !== "edit") await refreshContentTargets(true);
  restoreDirectFocus(true);
  return true;
}

// ------------------------------------------------------------------ P6 U7 clipboard (§2.1, D1, D2, D5, §10)

/** §9.34/§10.41: clipboard refusals in words, per kind first; `content.refused[].detail` stays the bare token. */
const CLIPBOARD_DETAIL_TEXT: Record<string, string> = {
  "too-large": "That selection is too large to copy or paste.",
  "empty-clipboard": "Nothing to paste",
  "not-candidate": "That selection cannot be copied.",
  "not-candidate:image": "That image cannot be copied.",
  "not-candidate:form": "That form cannot be copied.",
  "not-candidate:text": "That text run cannot be copied (a run inside a Form, a run that continues a line, or a Type 3 font).",
  "not-candidate:field": "That field cannot be copied (a signature, scripted, list or nested field).",
  "not-candidate:link": "That link cannot be copied.",
  "not-candidate:mark": "That redaction mark cannot be copied.",
  "out-of-bounds": "The pasted items do not fit on this page.",
  "unbalanced-content": "This page's drawing instructions are unbalanced; nothing was pasted.",
  "too-many:mark": "The redaction mark limit is reached",
  "too-many:field": "This form is too large to design fields in.",
  "no-such-page": "Every link's page is missing from this document; nothing was pasted.",
};
/** The kind a refusal names: `kind` (paste), else the member its `targetId`/`widgetId`/`annotId` points at (copy, batch). */
function refusedKind(reply: Record<string, unknown> | null): string | undefined {
  if (!reply) return undefined;
  if (typeof reply.kind === "string") return reply.kind;
  if (reply.widgetId !== undefined) return "field";
  const id = reply.targetId ?? reply.annotId;
  return id === undefined ? undefined : directSelection.find(member => member.targetId === String(id))?.kind;
}
function showClipRefusal(err: unknown, name: string): void {
  const { reply, reason } = refusalOf(err);
  const raw = reply as Record<string, unknown> | null;
  const detail = typeof raw?.detail === "string" ? raw.detail : reason === "read-only" ? "read-only" : "invalid-selection";
  const text = CLIPBOARD_DETAIL_TEXT[`${detail}:${refusedKind(raw)}`] ?? CLIPBOARD_DETAIL_TEXT[detail] ?? CONTENT_DETAIL_TEXT[detail]
    ?? (typeof raw?.error === "string" ? raw.error : messageOf(err));
  contentRefusals.push({ text: name, detail });
  announce(text); showError(text);
}
/** §10.30: whether a DOM copy/cut/paste event (Edit-menu role, chord or drive) belongs to the page selection. Pure. */
export function clipboardChordOk(kind: "copy" | "cut" | "paste", ctx: {
  mode: string; layerLive: boolean; dialogOpen: boolean; menuOpen: boolean; editorOpen: boolean;
  active: { tag: string; type?: string; contentEditable?: boolean; inlineEditor?: boolean } | null;
  collapsed: boolean; selection: number; clipboard: boolean; editable: boolean;
}): boolean {
  if (!["edit", "forms", "annotate", "redact"].includes(ctx.mode) || !ctx.layerLive || ctx.dialogOpen || ctx.menuOpen || ctx.editorOpen) return false;
  const active = ctx.active, tag = active?.tag.toUpperCase() ?? "";
  if (active && (tag === "TEXTAREA" || tag === "SELECT" || active.contentEditable || active.inlineEditor
    || (tag === "INPUT" && !["radio", "checkbox", "button", "submit", "range", "color", "file"].includes((active.type || "text").toLowerCase())))) return false;
  return kind === "paste" ? ctx.clipboard && ctx.editable : ctx.collapsed && ctx.selection > 0;
}
/** §9.39: Cut, Copy and Paste (menu, chords, page menu) run as gesture work. */
async function clipboardWork(which: "cut" | "copy" | "paste", point: number[] | null): Promise<boolean> {
  if (which === "paste") return pasteClipboard(point);
  const members = directSelection.slice(), count = members.length;
  if (!count || !await copySelection(members)) return false;
  if (which === "copy") { announce(members.every(isObjectMember) ? `Copied ${count} object${count === 1 ? "" : "s"}` : `Copied ${count} item${count === 1 ? "" : "s"}`); return true; }
  return cutMembers(members);
}
/** §10.1/§10.25: one `clip-copy` over the whole selection; nothing is written. */
async function copySelection(members: ObjectFocus[]): Promise<boolean> {
  const scan = contentScan, content = members.some(member => isObjectMember(member) || member.kind === "text");
  if (!doc || (content && (!scan || scan.page !== wantedPage))) return false;
  try {
    const reply = (await request({
      op: "clip-copy", page: wantedPage, ...(content && scan ? { generation: scan.generation, revision: scan.revision } : {}),
      targets: members.map(member => ({ kind: member.kind, id: isObjectMember(member) || member.kind === "text" ? member.targetId : Number(member.targetId) })),
    })) as unknown as { count: number; kinds: Record<string, number> };
    clipboardInfo = { count: reply.count, kinds: { ...reply.kinds } };
    return true;
  } catch (err) { showClipRefusal(err, members[members.length - 1].name); return false; }
}
/** §10.8/§10.21/§10.42: after the copy, one delete of every member but the runs; the selection is empty afterwards. */
async function cutMembers(members: ObjectFocus[]): Promise<boolean> {
  const runs = members.filter(member => member.kind === "text").length, rest = members.filter(member => member.kind !== "text");
  const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;
  if (!rest.length) { announce(`Copied ${plural(runs, "text run")}; text runs cannot be cut`); dropDirectFocus(); return true; }
  const said = cutAnnouncement(rest.length, runs);
  if (!rest.every(isObjectMember)) { const ok = await deleteMembers(rest, said); if (ok) dropDirectFocus(); return ok; }
  // §9.38: objects keep the U5 delete paths and `content.edited` shapes; only the announcement says Cut.
  const scan = contentScan, entries: EditTarget[] = [];
  for (const member of rest) { const item = scan?.page === wantedPage ? scan.items.find(value => value.targetId === member.targetId) : undefined; if (item) entries.push(item); }
  if (!scan || entries.length !== rest.length || !rest.every(memberDeletable)) return false;
  const previous = directSelection;
  setSelection([]);
  const ok = entries.length === 1 ? await applyObjectEdit(scan, entries[0], "delete", {}) : await applyObjectsEdit(scan, entries.map(item => ({ item })), "delete");
  if (!ok) { setSelection(previous); restoreDirectFocus(false); return false; }
  if (runs) { contentEdited = { kind: "selection", action: "delete", name: rest[rest.length - 1].name, count: rest.length }; announce(said); }
  else announce(entries.length === 1 ? `Cut ${entries[0].kind} ${entries[0].objectName ?? ""}`.trim() : `Cut ${entries.length} objects`);
  dropDirectFocus();
  return true;
}
const CLIP_KIND_ORDER = ["image", "form", "text", "field", "link", "mark"];
const CLIP_KIND_WORD: Record<string, string> = { image: "image", form: "form", text: "text run", field: "field", link: "link", mark: "redaction mark" };
const MODE_LABEL: Record<string, string> = { edit: "Edit text", forms: "Fill forms", annotate: "Annotate", redact: "Redact" };
/** Debt sweep 2 §9.10: the Cut announcement of a selection whose text runs stay (verb agrees with the run count). */
export function cutAnnouncement(items: number, runs: number): string {
  const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;
  return `Cut ${plural(items, "item")}${runs ? `; ${plural(runs, "text run")} ${runs === 1 ? "stays (it cannot" : "stay (they cannot"} be deleted)` : ""}`;
}
/** §10.33: the paste announcement (exact; pinned at the hello paste). */
export function pasteAnnouncement(kinds: Record<string, number>, count: number, skipped: number, mode: string, names: string[]): string {
  const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;
  const objectsOnly = CLIP_KIND_ORDER.every(kind => kind === "image" || kind === "form" || !kinds[kind]) && !skipped;
  if (objectsOnly && mode === "edit") return count === 1 && names.length === 1 ? `Pasted ${(kinds.form ?? 0) > 0 ? "form" : "image"} ${names[0]}` : `Pasted ${count} objects`;
  let text = `Pasted ${plural(count, "item")}`;
  for (const kind of CLIP_KIND_ORDER) {
    const n = (kinds[kind] ?? 0) - (kind === "link" ? skipped : 0);
    if (n > 0 && memberMode(kind) !== mode) text += `; ${plural(n, CLIP_KIND_WORD[kind])} shown in ${MODE_LABEL[memberMode(kind)]}`;
  }
  if (skipped) text += `; ${plural(skipped, "link")} skipped (its page is not in this document)`;
  return text;
}
/** §10.34: one `clip-paste`; the pasted members of this mode's layer become the selection (anchor = the last). */
async function pasteClipboard(point: number[] | null): Promise<boolean> {
  const info = clipboardInfo;
  if (!doc || !info || !editable()) return false;
  const page = wantedPage;
  let at: number[] | undefined;
  if (point) {
    // §9.28: the view point through the inverse page transform (MediaBox origin and /Rotate) → PDF points.
    if (!contentScan || contentScan.page !== page || !contentScan.transform || contentScan.docGeneration !== generation) await refreshContentTargets(true);
    const m = contentScan?.page === page ? contentScan.transform : undefined;
    if (!m) return false;
    const box = pdfBox([point[0], point[1], point[0], point[1]], m); at = [box[0], box[1]];
  }
  const scan = contentScan, stamp = scan?.page === page && scan.docGeneration === generation && scan.revision === revision ? { generation: scan.generation, revision: scan.revision } : {};
  let reply: Record<string, unknown>;
  try { reply = (await request({ op: "clip-paste", page, ...stamp, ...(at ? { at } : {}) })) as unknown as Record<string, unknown>; }
  catch (err) { showClipRefusal(err, ""); return false; }
  const list = <T>(value: unknown): T[] => Array.isArray(value) ? value as T[] : [];
  const names = list<string>(reply.names), targetIds = list<string>(reply.targetIds), fieldIds = list<number>(reply.fieldIds);
  const linkIds = list<number>(reply.linkIds), markIds = list<number>(reply.markIds), skipped = list<unknown>(reply.skipped).length;
  const count = typeof reply.count === "number" ? reply.count : names.length;
  const objectsOnly = CLIP_KIND_ORDER.every(kind => kind === "image" || kind === "form" || !info.kinds[kind]) && !skipped;
  const edited = objectsOnly ? { kind: "objects", action: "paste", name: names.join(","), count } : { kind: "selection", action: "paste", name: "", count, skipped };
  contentEdited = edited;
  await applyEdit(reply as unknown as JournalState, pasteAnnouncement(info.kinds, count, skipped, uiMode, names));
  if (uiMode !== "edit") await refreshContentTargets(true);
  // Set only after both awaits, so a concurrent repaint never sees the old selection (§10.34).
  const picks: ObjectFocus[] = [], now = contentScan;
  if (uiMode === "edit" && now?.page === page && now.transform) {
    const m = now.transform;
    for (const name of names) { const item = now.items.find(value => value.kind !== "text" && value.objectName === name && value.placement?.box); if (item) picks.push({ kind: item.kind, name, targetId: item.targetId, rect: viewBox(item.placement!.box, m) }); }
    for (const id of targetIds) { const item = now.items.find(value => value.targetId === id); if (item?.bounds?.length === 4) picks.push(textMember(item)); }
  } else if (uiMode === "forms") {
    const groups = new Set<string>();
    for (const id of fieldIds) { const item = fields.find(value => value.id === id); if (!item || (item.kind === "radio" && groups.has(item.name))) continue; groups.add(item.name); picks.push(fieldMember(item)); }
  } else if (uiMode === "annotate") { for (const id of linkIds) { const item = links.find(value => linkAnnotId(value) === id); if (item) picks.push(linkMember(item)); } }
  else if (uiMode === "redact") { for (const id of markIds) { const item = notes.find(value => value.id === id); if (item) picks.push(markMember(item)); } }
  if (!objectsOnly) edited.name = picks.at(-1)?.name ?? "";
  clearDirectFocus(); setSelection(picks);
  restoreDirectFocus(true);
  const anchor = picks.at(-1);
  if (anchor) syncAnchor(anchor);
  return true;
}
// ------------------------------------------------------------------ object context menu (P6 U5 §2.4)

type ObjectMenuItem = keyof typeof objectMenuButtons;
type ObjectTargets = { member: ObjectFocus; item: EditTarget; rect: number[]; node?: HTMLElement | null }[];
/** P6 U6: the engine's `placement.orient` / `full` (read through a cast; engine.ts owns the type). */
function placementOf(item: EditTarget): { box: number[]; orient?: { swap: boolean; sx: 1 | -1; sy: 1 | -1 }; full?: number[] } | undefined {
  return item.placement as { box: number[]; orient?: { swap: boolean; sx: 1 | -1; sy: 1 | -1 }; full?: number[] } | undefined;
}
function hasOp(item: EditTarget, op: string): boolean { return ((item.objectOps ?? []) as string[]).includes(op); }
function objectMenuItems(): Record<ObjectMenuItem, boolean> {
  // P6 U7 §9.25/§9.35/§10.37: the page menu offers Paste only; a selection with a non-object member offers Cut/Copy/Paste/Delete.
  if (objectMenuKind === "page") return objectItemsFor(null);
  if (!selectionMixed()) return objectItemsFor(selectionTargets());
  const copy = directSelection.length > 0 && permitted("copy"), remove = directSelection.length > 0 && directSelection.every(memberDeletable);
  return { ...objectItemsFor(null), cut: copy && remove, copy, delete: remove };
}
/** U5 §2.4 + U6 §2.1/§9.22: rotate/flip need `transform` on every member (rotate also needs the turned union to fit the page); crop needs one member listing `crop`. */
function objectItemsFor(targets: ObjectTargets | null): Record<ObjectMenuItem, boolean> {
  const edit = !!targets && targets.length > 0 && canEdit(), list = targets ?? [], size = doc?.pages[wantedPage];
  const every = (op: string) => edit && list.every(target => hasOp(target.item, op));
  const union = list.length ? unionRect(list.map(target => target.rect)) : null;
  const turns = !!union && !!size && union[3] - union[1] <= size.width + 1e-6 && union[2] - union[0] <= size.height + 1e-6;
  // P6 U7 §10.37: copy needs the copy bit; cut also needs every delete; paste needs a clipboard and an editable document.
  const copy = list.length > 0 && permitted("copy");
  return {
    cut: copy && every("delete"),
    copy,
    paste: !!clipboardInfo && editable(),
    replace: edit && list.length === 1 && list[0].item.kind === "image" && hasOp(list[0].item, "replace"),
    duplicate: every("duplicate"),
    rotateCw: every("transform") && turns,
    rotateCcw: every("transform") && turns,
    flipH: every("transform"),
    flipV: every("transform"),
    crop: edit && list.length === 1 && hasOp(list[0].item, "crop"),
    delete: every("delete"),
  };
}
function syncObjectMenu(): void {
  const items = objectMenuItems();
  for (const [name, button] of Object.entries(objectMenuButtons) as [ObjectMenuItem, HTMLButtonElement][]) {
    button.disabled = !items[name]; button.setAttribute("aria-disabled", String(!items[name]));
  }
  objectMenuButtons.crop.setAttribute("aria-checked", String(directCrop !== null));
}
function objectMenuState(): { name: string; kind: string; count: number; items: Record<ObjectMenuItem, boolean> } | null {
  if (objectMenu.hidden) return null;
  const items = Object.fromEntries((Object.keys(objectMenuButtons) as ObjectMenuItem[]).map(name => [name, !objectMenuButtons[name].disabled])) as Record<ObjectMenuItem, boolean>;
  if (objectMenuKind === "page") return { name: "", kind: "page", count: 0, items };
  const anchor = directSelection.at(-1);
  return anchor ? { name: anchor.name, kind: anchor.kind, count: directSelection.length, items } : null;
}
/** Opens at a client point (the pointer, or the focused target's top-left), in `#page` coordinates, kept inside `#viewport`. */
function openObjectMenu(clientX: number, clientY: number, kind: "selection" | "page" = "selection", point: number[] | null = null): void {
  if (!doc || (kind === "selection" && !directSelection.length)) return;
  objectMenuKind = kind; objectMenuPoint = point;
  objectMenuAt = { page: wantedPage, zoom: shownZoom };
  syncObjectMenu();
  objectMenu.hidden = false;
  const pageBox = pageRegion.getBoundingClientRect(), view = viewport.getBoundingClientRect();
  const width = objectMenu.offsetWidth, height = objectMenu.offsetHeight;
  const x = Math.max(view.left, Math.min(clientX, view.right - width)), y = Math.max(view.top, Math.min(clientY, view.bottom - height));
  objectMenu.style.left = `${x - pageBox.left}px`; objectMenu.style.top = `${y - pageBox.top}px`;
  // P6 U7: focus opens on the first enabled U5/U6 item (Replace…Delete, so the keyboard walk of direct-edit6 holds); Cut/Copy/Paste sit above it.
  const entries = Object.entries(objectMenuButtons), clip = ["cut", "copy", "paste"];
  (entries.find(([name, button]) => !clip.includes(name) && !button.disabled)?.[1] ?? entries.find(([, button]) => !button.disabled)?.[1] ?? objectMenu).focus({ preventScroll: true });
}
/** §9.21: every close but an outside press puts focus back on the anchor's button. */
function closeObjectMenu(refocus: boolean): void {
  if (objectMenu.hidden) return;
  objectMenu.hidden = true; objectMenuAt = null;
  if (!refocus) return;
  const anchor = directSelection.at(-1);
  if (objectMenuKind === "page") pageRegion.focus({ preventScroll: true });
  else if (anchor) memberNode(anchor)?.focus({ preventScroll: true });
}
/** §9.25: the menu duplicate's rect: +10 pt right and down, −10 on an axis with no room, then kept on the page. */
export function menuDuplicateRect(union: number[], page: number[]): number[] {
  const dx = union[2] + 10 <= page[0] ? 10 : -10, dy = union[3] + 10 <= page[1] ? 10 : -10;
  return gestureRect(union, "grip", dx, dy, { min: 1, lockAspect: false, page });
}
/** Replace, duplicate (+10 pt right and down, −10 on an axis with no room, §9.25), rotate/flip (U6 §2.1) or delete the selection. */
async function objectMenuWork(which: ObjectMenuItem, point: number[] | null = null): Promise<unknown> {
  // P6 U7 §2.1/§9.25: the clipboard items (and the page menu) never need object targets; a mixed selection only deletes.
  if (which === "cut" || which === "copy" || which === "paste") return clipboardWork(which, point);
  if (selectionMixed()) return which === "delete" ? deleteSelection() : undefined;
  const targets = selectionTargets(), size = doc?.pages[wantedPage];
  if (!targets || !size) return;
  if (which === "rotateCw" || which === "rotateCcw" || which === "flipH" || which === "flipV") return transformObjects(targets, which);
  if (which === "crop") return;
  if (which === "delete" && targets.length >= 2) return groupDelete();
  const union = unionRect(targets.map(target => target.rect)), moved = menuDuplicateRect(union, [size.width, size.height]);
  if (which === "duplicate" && targets.length >= 2) return commitObjectGroup(targets, moved[0] - union[0], moved[1] - union[1], "duplicate");
  const { item, rect } = targets[0];
  if (which === "duplicate") return commitObjectDuplicate(item, wantedPage, rect, shiftRect(rect, moved[0] - union[0], moved[1] - union[1]));
  return objectAction(item, wantedPage, rect, which);
}
function activateObjectMenu(which: ObjectMenuItem): void {
  if (objectMenu.hidden || objectMenuButtons[which].disabled) return;
  const point = objectMenuKind === "page" ? objectMenuPoint : null;
  closeObjectMenu(true);
  // §9.15: Crop toggles a mode; it is not gesture work.
  if (which === "crop") { toggleCrop(); return; }
  if (!directGestureWork) runGestureWork(() => objectMenuWork(which, point));
}
objectMenu.addEventListener("pointerdown", event => event.stopPropagation());
objectMenu.addEventListener("click", event => {
  event.stopPropagation(); event.preventDefault();
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>('[role^="menuitem"]');
  const which = (Object.keys(objectMenuButtons) as ObjectMenuItem[]).find(name => objectMenuButtons[name] === button);
  if (which) activateObjectMenu(which);
});
objectMenu.addEventListener("keydown", event => {
  const enabled = Object.values(objectMenuButtons).filter(button => !button.disabled), at = enabled.indexOf(document.activeElement as HTMLButtonElement);
  const key = event.key;
  if (key === "ArrowDown" || key === "ArrowUp") { if (enabled.length) enabled[(at + (key === "ArrowDown" ? 1 : enabled.length - (at < 0 ? 0 : 1))) % enabled.length].focus({ preventScroll: true }); }
  else if (key === "Home" || key === "End") enabled[key === "Home" ? 0 : enabled.length - 1]?.focus({ preventScroll: true });
  else if (key === "Enter" || key === " ") { const which = (Object.keys(objectMenuButtons) as ObjectMenuItem[]).find(name => objectMenuButtons[name] === document.activeElement); if (which) activateObjectMenu(which); }
  else if (key === "Escape" || key === "Tab") closeObjectMenu(true);
  else return;
  event.preventDefault(); event.stopPropagation();
});
// §9.22: outside-close on the capture phase, so presses that stop propagation (object bodies) still close it.
document.addEventListener("pointerdown", event => {
  if (objectMenu.hidden || objectMenu.contains(event.target as Node)) return;
  closeObjectMenu(false); objectMenuDismiss = event;
}, true);
viewport.addEventListener("scroll", () => closeObjectMenu(true));

/** §9.24: an unclassed outline where a duplicated original stays; removed when the gesture ends. */
function objectGhosts(rects: number[][]): () => void {
  const ghosts = rects.map(rect => { const ghost = document.createElement("div"); ghost.className = "ghost"; ghost.setAttribute("aria-hidden", "true"); placeBox(ghost, rect, shownZoom); hitLayer.append(ghost); return ghost; });
  return () => ghosts.forEach(ghost => ghost.remove());
}
/** §9.6: the Annotations row of the focused mark is the current one. */
function syncNoteCurrent(): void {
  const id = directFocusedMark ? String(directFocusedMark.annotId) : null;
  for (const row of noteListEl.querySelectorAll("li")) {
    if (id !== null && row.querySelector(`[data-annot-id="${CSS.escape(id)}"]`)) row.setAttribute("aria-current", "true"); else row.removeAttribute("aria-current");
  }
}
function dropDirectFocus(): void { clearDirectFocus(); markSelected(null); syncNoteCurrent(); syncLinkCurrent(); }
function restoreDirectFocus(refocus: boolean): void {
  // P6 U7 §10.39: a selection holding a run, field, link or mark re-resolves each member by its own layer.
  if (selectionMixed()) {
    const link = directFocusedLink;
    resolveMembers();
    if (!directFocusedLink && directActive?.kind === "link") directFocusedLink = link;
    const anchor = directSelection.at(-1), target = anchor ? memberNode(anchor) : null;
    if (directSelection.length) markSelection(); else markSelected(null);
    if (target && refocus && !directActive) target.focus({ preventScroll: true });
    syncNoteCurrent(); syncLinkCurrent();
    return;
  }
  let target: HTMLElement | null = null;
  const focus = directFocusedObject;
  if (focus) {
    const scan = contentScan;
    if (uiMode !== "edit") directFocusedObject = null;
    else if (scan?.page === wantedPage && scan.transform) {
      // Nearest painted target of the same kind + name to the last committed/clicked rect (ids change on a rewrite).
      // A replace renames the XObject (Im2 → Im2r1): with no same-name target, the same-kind one in the same place (≤ 2 px) keeps focus.
      // P6 U5 §9.13: the anchor first, then every other member the same way; each painted target is used at most once.
      const used = new Set<HTMLElement>(), transform = scan.transform;
      const resolve = (member: ObjectFocus, rename: boolean): { node: HTMLElement; member: ObjectFocus } | null => {
        let best = Infinity, found: HTMLElement | null = null, renamed: EditTarget | null = null;
        for (const pass of rename ? [true, false] : [true]) {
          for (const node of hitLayer.querySelectorAll<HTMLElement>(".object-target")) {
            if (used.has(node)) continue;
            const item = scan.items.find(value => value.targetId === node.dataset.targetId);
            if (!item?.placement || item.kind !== member.kind || ((item.objectName ?? "") === member.name) !== pass) continue;
            const distance = rectDistance(viewBox(item.placement.box, transform), member.rect);
            if (distance < best && (pass || distance <= 2)) { best = distance; found = node; renamed = pass ? null : item; }
          }
          if (found) break;
        }
        if (!found) return null;
        used.add(found);
        return { node: found, member: { ...member, name: renamed ? renamed.objectName ?? "" : member.name, targetId: found.dataset.targetId ?? member.targetId } };
      };
      const anchor = resolve(focus, true);
      target = anchor?.node ?? null;
      directFocusedObject = anchor?.member ?? null;
      const others = directSelection.slice(0, -1).map(member => resolve(member, false)?.member).filter((member): member is ObjectFocus => !!member);
      directSelection = directFocusedObject ? [...others, directFocusedObject] : [];
    }
  }
  if (!directFocusedObject) directSelection = [];
  if (directFocusedMark) {
    target = hitLayer.querySelector<HTMLElement>(`.redact-target[data-annot-id="${directFocusedMark.annotId}"]`);
    if (!target) directFocusedMark = null;
  }
  if (directFocusedLink) {
    target = hitLayer.querySelector<HTMLElement>(`.link-target[data-annot-id="${directFocusedLink.annotId}"]`);
    // An open link editor keeps its link focused while the target is repainted under it.
    if (!target && directActive?.kind !== "link") directFocusedLink = null;
  }
  if (directSelection.length > 0) markSelection(); else markSelected(target);
  // P6 U6 §9.18: crop mode survives a repaint on the re-resolved anchor (its targetId changes on every write).
  if (directCrop) {
    const node = directSelection.length === 1 && directFocusedObject ? objectNode(directFocusedObject.targetId) : null;
    const item = node ? contentScan?.items.find(value => value.targetId === directFocusedObject!.targetId) : undefined;
    if (node && item?.placement && contentScan?.transform) { directCrop = { targetId: item.targetId, full: viewFull(item, contentScan.transform) }; paintCrop(node); }
    else leaveCrop();
  }
  // §9.6: a repaint while an editor is open must not steal its focus (the blur would commit and close it).
  if (target && refocus && !directActive) target.focus({ preventScroll: true });
  // P6 U6 review R4 (§9.26): the Inspector follows the re-resolved anchor (a rescan cleared it); a repaint that kept it leaves typed values alone.
  if (directFocusedObject && contentObjectTarget?.targetId !== directFocusedObject.targetId) syncObjectInspector(directFocusedObject.targetId);
  syncNoteCurrent();
  syncLinkCurrent();
}
/** Runs one keyboard/dblclick commit as the pending gesture work (drive awaits it; keys wait for it). */
function runGestureWork(work: () => Promise<unknown>): void {
  clearError();
  const promise: Promise<void> = (async () => {
    try { await work(); } catch (err) { showError(messageOf(err)); }
  })();
  directGestureWork = promise;
  void promise.then(() => { if (directGestureWork === promise) directGestureWork = null; });
}
/** ⌥-arrow 1 pt, ⌥⇧ 10 pt, ⌥⌘←/→ width 1 pt (⌥⌘↑/↓ is Move Page), in y-down page pt. */
function nudgeRect(rect: number[], event: { key: string; shiftKey: boolean; metaKey: boolean; ctrlKey?: boolean }, min: number, size: PageSize): number[] {
  const amount = !cmdKey(event) && event.shiftKey ? 10 : 1;
  const dx = event.key === "ArrowLeft" ? -amount : event.key === "ArrowRight" ? amount : 0, dy = event.key === "ArrowUp" ? -amount : event.key === "ArrowDown" ? amount : 0;
  return gestureRect(rect, cmdKey(event) ? "e" : "grip", dx, cmdKey(event) ? 0 : dy, { min, lockAspect: false, page: [size.width, size.height] });
}
function paintObjectTarget(item: EditTarget, scan: ContentScan): HTMLButtonElement {
  const rect = viewBox(item.placement!.box, scan.transform!), ops = item.objectOps ?? [], page = scan.page;
  const button = document.createElement("button"); button.type = "button";
  button.className = `hit-target object-target ${item.kind}`; button.dataset.targetId = item.targetId;
  button.setAttribute("aria-label", objectRowLabel(item)); button.title = objectRowState(item);
  placeBox(button, rect, shownZoom);
  let pressAt: [number, number] | null = null;
  const isMember = () => directSelection.some(member => member.targetId === item.targetId);
  const cropping = () => directCrop?.targetId === item.targetId;
  // P6 U6 §2.5/§9.24: a form around page content takes presses on a 6 px border ring only.
  if (item.kind === "form" && wrapsPageContent(item, scan)) {
    button.classList.add("wrapping");
    const ring = document.createElement("span"); ring.className = "ring"; ring.setAttribute("aria-hidden", "true"); button.append(ring);
  }
  if (canEdit()) {
    const start = (event: PointerEvent, handle: string) => {
      // §2.3/§9.18: in crop mode the handles and the grip move the crop window inside the uncropped box (∩ page).
      if (cropping() && directCrop) {
        const size = doc!.pages[page], full = directCrop.full;
        const within = [Math.max(0, full[0]), Math.max(0, full[1]), Math.min(size.width, full[2]), Math.min(size.height, full[3])];
        startRectGesture(event, { rect, id: item.targetId, handle, min: 1, lockAspect: false, can: canEdit, within, crop: true, snap: snapOthers([item.targetId]), commit: next => commitObjectCrop(item, page, rect, next) });
        return;
      }
      startRectGesture(event, { rect, id: item.targetId, handle, min: 1, lockAspect: item.kind === "image", can: canEdit, snap: snapOthers([item.targetId]), commit: next => commitObjectRect(item, page, rect, next) });
    };
    addFieldHandles(button, start);
    button.addEventListener("pointerdown", event => {
      // §9.27: a Control-click is the macOS right-click; it opens the menu, never a drag.
      if (event.button !== 0 || event.ctrlKey || (event.target as HTMLElement).closest(".hit-handle")) return;
      pressAt = [event.clientX, event.clientY];
      // §9.18: a body press on the cropping object moves the crop window.
      if (cropping() && !event.shiftKey) { start(event, "grip"); return; }
      // §9.23: a plain press on a non-member of a group makes it the single selection first.
      if (directSelection.length >= 2 && !isMember() && !event.shiftKey) focusObject(item, rect, button);
      if (directSelection.length >= 2 && isMember()) { if (selectionMixed()) startSelectionGesture(event, button); else startGroupGesture(event, event.altKey); return; }
      if (!event.altKey) { start(event, "grip"); return; }
      // §2.3: ⌥-drag duplicates; the original keeps an outline where it was.
      startRectGesture(event, { rect, id: item.targetId, handle: "grip", min: 1, lockAspect: false, can: canEdit, duplicate: true, done: objectGhosts([rect]), snap: snapOthers([item.targetId]), commit: next => commitObjectDuplicate(item, page, rect, next) });
    });
  }
  button.addEventListener("contextmenu", event => {
    event.preventDefault(); event.stopPropagation();
    if (!isMember()) focusObject(item, rect, button);
    openObjectMenu(event.clientX, event.clientY);
  });
  button.addEventListener("click", event => {
    event.preventDefault(); const press = pressAt; pressAt = null;
    if ((event.target as HTMLElement).closest(".hit-handle") || event.ctrlKey) return;
    // A real drag ends in a click on the moved button; only a click that stayed put selects.
    if (press && Math.hypot(event.clientX - press[0], event.clientY - press[1]) >= 3) return;
    if (event.shiftKey) toggleObject(item, rect, button); else focusObject(item, rect, button);
    directClickWork = Promise.resolve();
  });
  button.addEventListener("dblclick", event => { event.preventDefault(); if (directSelection.length < 2 && !directCrop && !directGestureWork && canEdit() && item.kind === "image" && ops.includes("replace")) runGestureWork(() => objectAction(item, page, rect, "replace")); });
  button.addEventListener("keydown", event => {
    // §9.18: crop mode owns Escape/Enter (leave it, keep the selection); arrows and Delete do nothing; a gesture's Escape is the window's.
    if (event.key === "Escape" && directGesture) return;
    if (cropping()) {
      if (event.key === "Escape" || event.key === "Enter") { event.preventDefault(); leaveCrop(); return; }
      if (ARROW_KEYS.includes(event.key) || event.key === "Delete" || event.key === "Backspace") { event.preventDefault(); return; }
    }
    if (event.key === "Escape") { event.preventDefault(); dropDirectFocus(); return; }
    if ((event.key === "F10" && event.shiftKey) || event.key === "ContextMenu") {
      event.preventDefault();
      if (!isMember()) focusObject(item, rect, button);
      const box = button.getBoundingClientRect(); openObjectMenu(box.left, box.top);
      return;
    }
    const size = doc?.pages[page];
    let work: (() => Promise<unknown>) | null = null;
    if (directSelection.length >= 2 && isMember()) {
      // §2.2: a focused member moves or deletes the whole group; Enter (replace) and ⌥⌘ widths are single-only.
      if (ARROW_KEYS.includes(event.key) && (event.altKey || (!event.metaKey && !event.ctrlKey))) {
        // P6 U6 §9.21: ⌥⌘←/→ resize the union's width by 1 pt (⌥⌘↑/↓ is Move Page).
        if (cmdKey(event) && (event.key === "ArrowUp" || event.key === "ArrowDown")) return;
        const targets = selectionTargets();
        if (targets && size && canEdit()) {
          const union = unionRect(targets.map(target => target.rect)), next = nudgeRect(union, event, cmdKey(event) ? groupMin(targets, union) : 1, size);
          work = cmdKey(event) ? () => commitGroupResize(targets, union, next) : () => commitObjectGroup(targets, next[0] - union[0], next[1] - union[1], "move");
        }
      } else if (event.key === "Delete" || event.key === "Backspace") work = () => groupDelete();
      else if (event.key !== "Enter" && event.key !== " ") return;
      event.preventDefault();
      if (work && !directGestureWork) runGestureWork(work);
      return;
    }
    if (ARROW_KEYS.includes(event.key) && (event.altKey || (!event.metaKey && !event.ctrlKey))) {
      if (cmdKey(event) && (event.key === "ArrowUp" || event.key === "ArrowDown")) return;
      if (size && canEdit()) { const next = nudgeRect(rect, event, 1, size); work = () => commitObjectRect(item, page, rect, next); }
    } else if (event.key === "Delete" || event.key === "Backspace") { if (canEdit() && ops.includes("delete")) work = () => objectAction(item, page, rect, "delete"); }
    else if (event.key === "Enter") { if (canEdit() && item.kind === "image" && ops.includes("replace")) work = () => objectAction(item, page, rect, "replace"); }
    else if (event.key !== " ") return;
    event.preventDefault();
    if (work && !directGestureWork) runGestureWork(work);
  });
  return button;
}
/** §9.6: click focuses the object and mirrors it in the Inspector without moving focus there. */
function focusObject(item: EditTarget, rect: number[], button: HTMLElement): void {
  directFocusedObject = { kind: item.kind, name: item.objectName ?? "", targetId: item.targetId, rect }; directFocusedMark = null;
  directSelection = [directFocusedObject];
  markSelection(); button.focus({ preventScroll: true }); syncNoteCurrent();
  syncObjectInspector(item.targetId);
}
/** §0 parity: the Inspector row and form follow the anchor (focusObject, Shift-click, the copies after a duplicate). */
function syncObjectInspector(targetId: string): void {
  const scan = contentScan, index = scan?.items.findIndex(value => value.targetId === targetId) ?? -1;
  if (scan && index >= 0) { selectContentRow(index, false); openContentObjectForm(scan.items[index], false); }
}
/** §1.5: one `edit-object` move/resize with the view rect mapped back to PDF user space. */
async function commitObjectRect(item: EditTarget, page: number, initial: number[], rect: number[]): Promise<boolean> {
  const scan = contentScan, size = doc?.pages[page];
  if (!scan || scan.page !== page || !scan.transform || !size) return false;
  const current = resolveObject(scan, item, initial), old = current?.placement?.box;
  if (!current || !old) return false;
  const clip = pdfBox([0, 0, size.width, size.height], scan.transform);
  const box = pdfBox(rect, scan.transform).map((value, index) => Math.abs(value - clip[index]) <= 0.01 ? clip[index] : value);
  const action = Math.abs((box[2] - box[0]) - (old[2] - old[0])) < 0.05 && Math.abs((box[3] - box[1]) - (old[3] - old[1])) < 0.05 ? "move" : "resize";
  const previous = directFocusedObject, selection = directSelection;
  directFocusedObject = { kind: current.kind, name: current.objectName ?? "", targetId: current.targetId, rect }; directFocusedMark = null;
  directSelection = [directFocusedObject];
  if (await applyObjectEdit(scan, current, action, { box })) return true;
  directFocusedObject = previous; directSelection = selection;
  return false;
}
/** A view rect → the PDF box an edit sends (snapped to the page clip within 0.01, as `commitObjectRect`). */
function objectBox(scan: ContentScan, size: PageSize, rect: number[]): number[] {
  const clip = pdfBox([0, 0, size.width, size.height], scan.transform!);
  return pdfBox(rect, scan.transform!).map((value, index) => Math.abs(value - clip[index]) <= 0.01 ? clip[index] : value);
}
/** §9.18: after a duplicate the copies are the selection: per requested member, the nearest same kind + name target absent from the pre-reply scan. */
function selectCopies(before: Set<string>, wanted: ObjectFocus[]): void {
  const scan = contentScan, m = scan?.transform;
  if (!scan || !m) return;
  const used = new Set<string>();
  const picks = wanted.map(want => {
    const copy = scan.items.filter(value => !before.has(value.targetId) && !used.has(value.targetId) && value.kind === want.kind && (value.objectName ?? "") === want.name && value.placement?.box)
      .map(value => ({ value, distance: rectDistance(viewBox(value.placement!.box, m), want.rect) })).sort((a, b) => a.distance - b.distance)[0]?.value;
    if (!copy) return null;
    used.add(copy.targetId);
    return { ...want, targetId: copy.targetId };
  }).filter((pick): pick is ObjectFocus => !!pick);
  if (!picks.length) return;
  directSelection = picks; directFocusedObject = picks[picks.length - 1];
  markSelection();
  objectNode(directFocusedObject.targetId)?.focus({ preventScroll: true }); syncObjectInspector(directFocusedObject.targetId);
}
/** §2.3: one `edit-object {action:'duplicate', box}`; the copy becomes the selection. */
async function commitObjectDuplicate(item: EditTarget, page: number, initial: number[], rect: number[]): Promise<boolean> {
  const scan = contentScan, size = doc?.pages[page];
  if (!scan || scan.page !== page || !scan.transform || !size) return false;
  const current = resolveObject(scan, item, initial);
  if (!current?.placement?.box) return false;
  const before = new Set(scan.items.map(value => value.targetId));
  const previous = directFocusedObject, selection = directSelection;
  directFocusedObject = { kind: current.kind, name: current.objectName ?? "", targetId: current.targetId, rect }; directFocusedMark = null;
  directSelection = [directFocusedObject];
  if (!await applyObjectEdit(scan, current, "duplicate", { box: objectBox(scan, size, rect) })) { directFocusedObject = previous; directSelection = selection; return false; }
  selectCopies(before, [{ kind: current.kind, name: current.objectName ?? "", targetId: current.targetId, rect }]);
  return true;
}
/** §2.2: a group drag on the union rect; every member follows, and one `edit-objects` commits the delta. */
function startGroupGesture(event: PointerEvent, duplicate: boolean): void {
  const targets = selectionTargets(), anchor = directFocusedObject;
  if (!targets || !anchor) return;
  const union = unionRect(targets.map(target => target.rect)), box = hitLayer.querySelector<HTMLElement>(".group-box");
  const place = (rect: number[]) => { for (const target of targets) placeBox(target.node, shiftRect(target.rect, rect[0] - union[0], rect[1] - union[1]), shownZoom); if (box) placeBox(box, rect, shownZoom); };
  startRectGesture(event, { rect: union, id: anchor.targetId, handle: "grip", min: 1, lockAspect: false, can: canEdit, place, duplicate, snap: snapOthers(targets.map(target => target.item.targetId)),
    done: duplicate ? objectGhosts(targets.map(target => target.rect)) : undefined,
    commit: rect => commitObjectGroup(targets, rect[0] - union[0], rect[1] - union[1], duplicate ? "duplicate" : "move") });
}
/** §2.2/§2.3: every member moved or duplicated by the same view delta, as one `edit-objects`. */
async function commitObjectGroup(targets: { member: ObjectFocus; item: EditTarget; rect: number[] }[], dx: number, dy: number, action: "move" | "duplicate"): Promise<boolean> {
  const scan = contentScan, size = doc?.pages[wantedPage];
  if (!scan || scan.page !== wantedPage || !scan.transform || !size || (dx === 0 && dy === 0)) return false;
  const before = new Set(scan.items.map(value => value.targetId));
  const wanted = targets.map(target => ({ ...target.member, targetId: target.item.targetId, rect: shiftRect(target.rect, dx, dy) }));
  const previous = directFocusedObject, selection = directSelection;
  // §9.13: every member's rect is its requested rect before the reply repaints.
  directSelection = wanted; directFocusedObject = wanted[wanted.length - 1]; directFocusedMark = null;
  const ok = await applyObjectsEdit(scan, targets.map((target, index) => ({ item: target.item, box: objectBox(scan, size, wanted[index].rect) })), action);
  if (!ok) { directFocusedObject = previous; directSelection = selection; markSelection(); return false; }
  if (action === "duplicate") selectCopies(before, wanted);
  return true;
}
/** §2.2: Delete on a group; the selection is cleared. */
async function groupDelete(): Promise<boolean> {
  const scan = contentScan, targets = selectionTargets();
  if (!scan || !targets || !canEdit() || !targets.every(target => (target.item.objectOps ?? []).includes("delete"))) return false;
  const previous = directFocusedObject, selection = directSelection;
  directFocusedObject = null; directSelection = [];
  if (await applyObjectsEdit(scan, targets.map(target => ({ item: target.item })), "delete")) return true;
  directFocusedObject = previous; directSelection = selection; markSelection();
  return false;
}
/** P6 U6 §2.2/§9.13: a `.group-box` handle resizes the union; every member maps by the same affine; one `edit-objects` resize. */
function startGroupResize(event: PointerEvent, handle: string): void {
  const targets = selectionTargets(), anchor = directFocusedObject;
  if (!targets || targets.length < 2 || !anchor) return;
  const union = unionRect(targets.map(target => target.rect)), box = (event.currentTarget as HTMLElement).closest<HTMLElement>(".group-box");
  const focus = (targets.find(target => target.member.targetId === anchor.targetId) ?? targets[targets.length - 1]).node;
  const place = (rect: number[]) => { for (const target of targets) placeBox(target.node, mapRect(target.rect, union, rect), shownZoom); if (box) placeBox(box, rect, shownZoom); };
  startRectGesture(event, { rect: union, id: anchor.targetId, handle, min: groupMin(targets, union), lockAspect: targets.every(target => target.item.kind === "image"), can: canEdit, place, focus,
    snap: snapOthers(targets.map(target => target.item.targetId)), commit: rect => commitGroupResize(targets, union, rect) });
}
async function commitGroupResize(targets: ObjectTargets, union: number[], rect: number[]): Promise<boolean> {
  const scan = contentScan, size = doc?.pages[wantedPage];
  if (!scan || scan.page !== wantedPage || !scan.transform || !size || rect.every((value, index) => value === union[index])) return false;
  const wanted = targets.map(target => ({ ...target.member, targetId: target.item.targetId, rect: mapRect(target.rect, union, rect) }));
  const previous = directFocusedObject, selection = directSelection;
  directSelection = wanted; directFocusedObject = wanted[wanted.length - 1]; directFocusedMark = null;
  const ok = await applyObjectsEdit(scan, targets.map((target, index) => ({ item: target.item, box: objectBox(scan, size, wanted[index].rect) })), "resize");
  if (!ok) { directFocusedObject = previous; directSelection = selection; markSelection(); return false; }
  return true;
}
/** §2.1/§9.22/§9.23: rotate 90° or flip the targets about their union's centre (view space), kept on the page; one request. */
async function transformObjects(targets: ObjectTargets, which: "rotateCw" | "rotateCcw" | "flipH" | "flipV", from?: HTMLElement): Promise<boolean> {
  const scan = contentScan, size = doc?.pages[wantedPage], m = scan?.transform;
  if (!scan || scan.page !== wantedPage || !m || !size || !targets.length) return false;
  const union = unionRect(targets.map(target => target.rect)), cx = (union[0] + union[2]) / 2, cy = (union[1] + union[3]) / 2;
  const turned = targets.map(target => turnRect(target.rect, which, cx, cy)), next = unionRect(turned);
  const placed = gestureRect(next, "grip", 0, 0, { min: 1, lockAspect: false, page: [size.width, size.height] });
  const rects = turned.map(rect => shiftRect(rect, placed[0] - next[0], placed[1] - next[1]));
  // §9.23: on an axis-swapping page (/Rotate 90/270) a view flip is the other PDF flip; rotations keep their direction.
  const swaps = Math.abs(m[1]) > Math.abs(m[0]);
  const transform = which === "rotateCw" ? "rotate-cw" : which === "rotateCcw" ? "rotate-ccw" : (which === "flipH") !== swaps ? "flip-h" : "flip-v";
  const wanted = targets.map((target, index) => ({ ...target.member, targetId: target.item.targetId, rect: rects[index] }));
  const previous = directFocusedObject, selection = directSelection;
  directSelection = wanted; directFocusedObject = wanted[wanted.length - 1]; directFocusedMark = null;
  const ok = targets.length === 1
    ? await applyObjectEdit(scan, targets[0].item, "transform", { transform, box: objectBox(scan, size, rects[0]) })
    : await applyObjectsEdit(scan, targets.map((target, index) => ({ item: target.item, box: objectBox(scan, size, rects[index]) })), "transform", transform);
  if (!ok) { directFocusedObject = previous; directSelection = selection; markSelection(); return false; }
  afterObjectEdit(from);
  return true;
}
/** §9.26: the Inspector shows the anchor's new box; an Inspector button that acted gets focus back. */
function afterObjectEdit(from?: HTMLElement): void {
  if (directFocusedObject) syncObjectInspector(directFocusedObject.targetId);
  from?.focus({ preventScroll: true });
}
/** §2.3: one `edit-object {action:'crop', box}`; the box is the new visible rect. */
async function commitObjectCrop(item: EditTarget, page: number, initial: number[], rect: number[]): Promise<boolean> {
  const scan = contentScan, size = doc?.pages[page];
  if (!scan || scan.page !== page || !scan.transform || !size) return false;
  const current = resolveObject(scan, item, initial);
  if (!current?.placement?.box) return false;
  const previous = directFocusedObject, selection = directSelection;
  directFocusedObject = { kind: current.kind, name: current.objectName ?? "", targetId: current.targetId, rect }; directFocusedMark = null;
  directSelection = [directFocusedObject];
  if (await applyObjectEdit(scan, current, "crop", { box: objectBox(scan, size, rect) })) { afterObjectEdit(); return true; }
  directFocusedObject = previous; directSelection = selection;
  return false;
}
function viewFull(item: EditTarget, m: number[]): number[] { const placement = placementOf(item)!; return viewBox(placement.full ?? placement.box, m); }
function toggleCrop(): void { if (directCrop) leaveCrop(); else enterCrop(); }
function enterCrop(): void {
  const targets = selectionTargets(), m = contentScan?.transform;
  if (!targets || targets.length !== 1 || !m || !objectItemsFor(targets).crop) return;
  directCrop = { targetId: targets[0].item.targetId, full: viewFull(targets[0].item, m) };
  paintCrop(targets[0].node);
}
function leaveCrop(): void { directCrop = null; paintCrop(null); }
/** `.cropping` on the member and one dashed `.crop-full` under it; the menu item and Inspector button mirror the mode. */
function paintCrop(node: HTMLElement | null): void {
  hitLayer.querySelectorAll(".crop-full").forEach(frame => frame.remove());
  hitLayer.querySelectorAll(".object-target.cropping").forEach(target => target.classList.remove("cropping"));
  if (directCrop && node) {
    node.classList.add("cropping");
    const frame = document.createElement("div"); frame.className = "crop-full"; frame.setAttribute("aria-hidden", "true");
    placeBox(frame, directCrop.full, shownZoom); hitLayer.insertBefore(frame, node);
  }
  objectMenuButtons.crop.setAttribute("aria-checked", String(directCrop !== null));
  contentTransformButtons.crop.setAttribute("aria-pressed", String(directCrop !== null));
}
// §9.18: a press anywhere but the cropping object (Shift-click included), the menu or the Inspector leaves crop mode.
document.addEventListener("pointerdown", event => {
  if (!directCrop) return;
  const target = event.target as HTMLElement;
  if (inspectorEl.contains(target) || objectMenu.contains(target)) return;
  if (!event.shiftKey && objectNode(directCrop.targetId)?.contains(target)) return;
  leaveCrop(); cropDismiss = event;
}, true);
/** §2.5/§9.24: a Form is wrapping when it contains (±0.5 pt) an item the page itself draws (targetId path without '/'). */
function wrapsPageContent(form: EditTarget, scan: ContentScan): boolean {
  const box = form.placement?.box;
  if (!box) return false;
  return scan.items.some(other => {
    if (other === form || (other.targetId.split(".")[1] ?? "/").includes("/")) return false;
    const bounds = other.placement?.box ?? other.bounds;
    return !!bounds && bounds.length === 4 && bounds[0] >= box[0] - 0.5 && bounds[1] >= box[1] - 0.5 && bounds[2] <= box[2] + 0.5 && bounds[3] <= box[3] + 0.5;
  });
}
/** §9.26: the Inspector acts on the selection when its object is a member, else on that object alone. */
function inspectorTargets(): ObjectTargets | null {
  const item = contentObjectTarget, scan = contentScan, m = scan?.transform;
  if (!item || !scan || !m || scan.page !== wantedPage) return null;
  if (directSelection.some(member => member.targetId === item.targetId)) return selectionTargets();
  const current = scan.items.find(value => value.targetId === item.targetId);
  if (!current?.placement?.box) return null;
  const rect = viewBox(current.placement.box, m);
  return [{ member: { kind: current.kind, name: current.objectName ?? "", targetId: current.targetId, rect }, item: current, rect, node: objectNode(current.targetId) }];
}
function syncInspectorTransform(): void {
  const items = objectItemsFor(inspectorTargets());
  for (const [name, button] of Object.entries(contentTransformButtons) as [keyof typeof contentTransformButtons, HTMLButtonElement][]) button.disabled = !items[name];
  contentTransformButtons.crop.setAttribute("aria-pressed", String(directCrop !== null));
}
function inspectorTransform(which: keyof typeof contentTransformButtons, button: HTMLButtonElement): void {
  const targets = inspectorTargets();
  if (!targets || !objectItemsFor(targets)[which]) return;
  if (which !== "crop") { if (!directGestureWork) runGestureWork(() => transformObjects(targets, which, button)); return; }
  if (directCrop) { leaveCrop(); return; }
  const target = targets[0];
  if (!target.node) return;
  if (!directSelection.some(member => member.targetId === target.item.targetId)) focusObject(target.item, target.rect, target.node);
  enterCrop(); button.focus({ preventScroll: true });
}
/** §2.1: Shift-click adds a non-member (it becomes the anchor) or removes a member (anchor = the last remaining). */
function toggleObject(item: EditTarget, rect: number[], button: HTMLElement): void {
  if (!directSelection.length) { focusObject(item, rect, button); return; }
  const index = directSelection.findIndex(member => member.targetId === item.targetId);
  if (index < 0) {
    const member = { kind: item.kind, name: item.objectName ?? "", targetId: item.targetId, rect };
    directSelection = [...directSelection, member]; directFocusedObject = member; directFocusedMark = null; directFocusedLink = null;
    markSelection(); button.focus({ preventScroll: true }); syncNoteCurrent(); syncObjectInspector(item.targetId);
    return;
  }
  directSelection = directSelection.filter((_, at) => at !== index);
  // §9.23: focus follows the anchor, so the next key never reaches the removed button.
  if (!directSelection.length) { dropDirectFocus(); pageRegion.focus({ preventScroll: true }); return; }
  const anchor = directSelection[directSelection.length - 1];
  directFocusedObject = isObjectMember(anchor) ? anchor : null;
  markSelection(); memberNode(anchor)?.focus({ preventScroll: true });
  if (directFocusedObject) syncObjectInspector(directFocusedObject.targetId); else syncAnchor(anchor);
}
async function objectAction(item: EditTarget, page: number, rect: number[], action: "delete" | "replace"): Promise<boolean> {
  let scan = contentScan;
  if (!scan || scan.page !== page) return false;
  if (action === "delete") {
    const current = resolveObject(scan, item, rect); if (!current) return false;
    directFocusedObject = null; directSelection = [];
    return applyObjectEdit(scan, current, "delete", {});
  }
  const picked = await pickOneImage(); if (!picked) return false;
  scan = contentScan;
  const current = scan?.page === page ? resolveObject(scan, item, rect) : null;
  if (!scan || !current) return false;
  directFocusedObject = { kind: current.kind, name: current.objectName ?? "", targetId: current.targetId, rect }; directSelection = [directFocusedObject];
  return applyObjectEdit(scan, current, "replace", { image: picked });
}
function paintMarkTarget(item: AnnotItem, page: number, index: number): HTMLButtonElement {
  const rect = [...item.rect], button = document.createElement("button"); button.type = "button";
  button.className = "hit-target redact-target"; button.dataset.annotId = String(item.id);
  button.setAttribute("aria-label", `Redaction mark ${index + 1}${item.search === true ? " (search)" : ""}`);
  button.title = canAnnotate() ? "Redaction mark: drag to move, Delete removes it" : "This document is read-only";
  placeBox(button, rect, shownZoom);
  if (canAnnotate()) {
    const start = (event: PointerEvent, handle: string) => startRectGesture(event, { rect, id: item.id, handle, min: 1, lockAspect: false, can: canAnnotate, commit: next => commitMarkRect(item.id, page, next) });
    addFieldHandles(button, start);
    button.addEventListener("pointerdown", event => { if (event.button === 0 && !event.ctrlKey && !(event.target as HTMLElement).closest(".hit-handle")) start(event, "grip"); });
  }
  button.addEventListener("click", event => {
    event.preventDefault(); if ((event.target as HTMLElement).closest(".hit-handle") || event.ctrlKey) return;
    // P6 U7 §9.21: a mark is a selection member; Shift-click adds or removes it.
    if (event.shiftKey) { directClickWork = shiftSelect(markMember(item)); return; }
    directFocusedObject = null; setSelection([markMember(item)]); markSelection(); button.focus({ preventScroll: true }); syncNoteCurrent(); directClickWork = Promise.resolve();
  });
  button.addEventListener("keydown", event => {
    if (event.key === "Escape") { event.preventDefault(); dropDirectFocus(); return; }
    const size = doc?.pages[page];
    let work: (() => Promise<unknown>) | null = null;
    if (ARROW_KEYS.includes(event.key) && (event.altKey || (!event.metaKey && !event.ctrlKey))) {
      if (cmdKey(event) && (event.key === "ArrowUp" || event.key === "ArrowDown")) return;
      if (size && canAnnotate()) { const next = nudgeRect(rect, event, 1, size); work = () => commitMarkRect(item.id, page, next); }
    } else if (event.key === "Delete" || event.key === "Backspace") { if (canAnnotate()) work = async () => { directFocusedMark = null; await deleteNote(item.id, "Deleted redaction mark"); }; }
    else if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    if (work && !directGestureWork) runGestureWork(work);
  });
  return button;
}
/** §3.2: one `redact-set`; a no-op reply writes nothing; `applyEdit` refreshes the notes and the redact preview. */
async function commitMarkRect(annotId: number, page: number, rect: number[]): Promise<boolean> {
  directFocusedMark = { annotId }; directFocusedObject = null;
  try {
    const reply = await request({ op: "redact-set", page, annotId, rect: rect.map(value => Math.round(value * 1e4) / 1e4) }) as MutationReply & { noop?: boolean };
    if (reply.noop !== true) await applyEdit(reply, "Moved redaction mark");
    return true;
  } catch (err) {
    const { reply } = refusalOf(err);
    showError(typeof reply?.error === "string" ? reply.error : messageOf(err));
    return false;
  }
}
/** fill-anywhere D4: a FreeText in Fill forms — drag moves it, arrows nudge 1 pt, Delete removes it, a click or Enter edits it. */
function paintTextBoxTarget(item: AnnotItem, page: number, index: number): HTMLButtonElement {
  const rect = [...item.rect], point: [number, number] = [rect[0], rect[1] + TEXTBOX_HALF_LINE], button = document.createElement("button"); button.type = "button";
  button.className = "hit-target textbox-target"; button.dataset.annotId = String(item.id);
  button.setAttribute("aria-label", `Text box ${index + 1}`);
  button.title = canAnnotate() ? "Text box: click to edit, drag to move, Delete removes it" : "This document is read-only";
  placeBox(button, rect, shownZoom);
  if (canAnnotate()) button.addEventListener("pointerdown", event => { if (event.button === 0 && !event.ctrlKey) startRectGesture(event, { rect, id: item.id, handle: "grip", min: 1, lockAspect: false, can: canAnnotate, commit: next => commitBoxMove(item.id, page, next) }); });
  button.addEventListener("click", event => {
    event.preventDefault(); if (event.ctrlKey || event.shiftKey || !canAnnotate()) return;
    openTextBox(page, point, item); directClickWork = Promise.resolve();
  });
  button.addEventListener("keydown", event => {
    if (event.key === "Escape") { event.preventDefault(); dropDirectFocus(); return; }
    const size = doc?.pages[page];
    let work: (() => Promise<unknown>) | null = null;
    if (ARROW_KEYS.includes(event.key) && !event.metaKey && !event.ctrlKey) { if (size && canAnnotate()) { const next = nudgeRect(rect, { key: event.key, shiftKey: event.shiftKey, metaKey: false }, 1, size); work = () => commitBoxMove(item.id, page, next); } }
    else if (event.key === "Delete" || event.key === "Backspace") { if (canAnnotate()) work = () => commitTextBox(page, item.id, point, ""); }
    else if (event.key === "Enter") { event.preventDefault(); if (canAnnotate()) openTextBox(page, point, item); return; }
    else return;
    event.preventDefault();
    if (work && !directGestureWork) runGestureWork(work);
  });
  return button;
}
/** Half the FreeText line height (Helv 12 x 1.2 / 2): a box's point is its left edge and first line's centre. */
const TEXTBOX_HALF_LINE = 7.2;
const TEXTBOX_ROTATED = "A text box cannot be placed on a rotated page";
/** fill-anywhere D3: the text-box editor, `openAddText`'s textarea in Helvetica 12 black; Enter, Tab and blur commit. */
function openTextBox(page: number, point: [number, number], box: AnnotItem | null): void {
  clearDirectFocus(); closeDirect(); clearError();
  const rect = [point[0], point[1] - TEXTBOX_HALF_LINE, point[0] + 96, point[1] + TEXTBOX_HALF_LINE];
  const editor = document.createElement("textarea");
  editor.className = "inline-editor text-inline-editor add-text"; editor.wrap = "off"; editor.maxLength = MAX_NOTE_CHARS; editor.spellcheck = false;
  editor.dataset.fontSize = "12"; editor.setAttribute("aria-label", box ? "Edit text box" : `New text box at ${point[0].toFixed(1)}, ${point[1].toFixed(1)} pt`);
  editor.style.fontFamily = "Helvetica, Arial, sans-serif"; editor.style.color = "#000000";
  editor.value = box?.contents ?? ""; editor.dataset.initial = editor.value;
  const item = { targetId: "", kind: "text", bounds: rect, support: "candidate", text: "" } as EditTarget;
  placeBox(editor, rect, shownZoom); hitLayer.append(editor);
  const active = { kind: "textbox" as const, item, editor, page, point, annotId: box?.id ?? null };
  directActive = active; growAddEditor(active);
  editor.addEventListener("input", () => { if (directActive === active) growAddEditor(active); });
  setupInlineEvents(editor, false); editor.focus(); if (box) editor.select();
}
/** fill-anywhere D3/D4/D6: the one commit the editor, Delete and `fill-text` share (new → add, emptied → delete, else re-fit). */
async function commitTextBox(page: number, annotId: number | null, point: number[], text: string): Promise<boolean> {
  if (!doc || !canAnnotate()) return false;
  if ((doc.rotations?.[page] ?? 0) % 360 !== 0) { showError(TEXTBOX_ROTATED); return false; }
  if (annotId === null && text === "") return true;
  clearError();
  const rectOf = (id: number) => { const rect = notes.find(item => item.id === id)?.rect; return rect ? rect.map(round2) : null; };
  try {
    if (annotId === null) {
      const before = new Set(((await request({ op: "annots", page })) as AnnotsReply).items.map(item => item.id));
      const reply = await request({ op: "annot-add", page, type: "FreeText", point, contents: text, fit: true }) as MutationReply;
      noteFilled();
      await applyEdit(reply, "Added text box");
      const id = notes.find(item => item.type === "FreeText" && !before.has(item.id))?.id ?? -1;
      fillLast = fillOp("add", id, rectOf(id)); directFocusedBox = id < 0 ? null : id;
    } else if (text === "") {
      const reply = await request({ op: "annot-delete", page, annotId }) as MutationReply;
      noteFilled(); directFocusedBox = null; fillLast = fillOp("delete", annotId, null);
      await applyEdit(reply, "Deleted text box");
    } else {
      const reply = await request({ op: "annot-set", page, annotId, contents: text, fit: true }) as MutationReply;
      noteFilled();
      await applyEdit(reply, "Edited text box");
      fillLast = fillOp("edit", annotId, rectOf(annotId)); directFocusedBox = annotId;
    }
    return true;
  } catch (err) {
    const { reply: refused } = refusalOf(err);
    const missing = Array.isArray(refused?.missing) ? refused.missing.filter(char => typeof char === "string") : [];
    const message = refused?.detail === "unsupported-encoding" && missing.length ? `Cannot draw: ${missing.join(" ")} (not in this font)` : typeof refused?.error === "string" ? refused.error : messageOf(err);
    announce(message); showError(message);
    return false;
  }
}
/** fill-anywhere D4: one `annot-move` to the rect's top-left; a no-op reply writes nothing. */
async function commitBoxMove(annotId: number, page: number, rect: number[]): Promise<boolean> {
  directFocusedBox = annotId;
  try {
    const reply = await request({ op: "annot-move", page, annotId, point: [rect[0], rect[1]].map(value => Math.round(value * 1e4) / 1e4) }) as MutationReply & { noop?: boolean };
    if (reply.noop === true) return true;
    noteFilled();
    await applyEdit(reply, "Moved text box");
    const moved = notes.find(item => item.id === annotId)?.rect;
    fillLast = fillOp("move", annotId, moved ? moved.map(round2) : null);
    return true;
  } catch (err) {
    const { reply } = refusalOf(err);
    showError(typeof reply?.error === "string" ? reply.error : messageOf(err));
    return false;
  }
}
/** fill-anywhere D6: `fill-text {page, x, y, text}` — Fill forms (OP_MODE), that page, the editor's commit. */
async function driveFillText(contents: string | null): Promise<void> {
  const value = driveWanted(contents); if (!value || !doc) return;
  const { page, x, y, text } = value;
  if (typeof page !== "number" || !Number.isInteger(page) || typeof x !== "number" || !Number.isFinite(x) || typeof y !== "number" || !Number.isFinite(y) || typeof text !== "string") { showError("fill-text: page, x, y and text are required"); return; }
  if (page < 0 || page >= doc.pageCount) { showError(`fill-text: page must be an integer between 0 and ${doc.pageCount - 1}`); return; }
  // Review: an empty text or a document that forbids annotations is refused in `error`, never a silent no-op.
  if (addDraft(text) === "") { showError("fill-text: text is empty"); return; }
  if (!canAnnotate()) { showError("fill-text: this document does not permit annotations"); return; }
  if (directActive) await finishDirect();
  goTo(page); await drain();
  while (doc && (shownPage !== page || inFlight !== null)) await new Promise(resolve => window.setTimeout(resolve, 10));
  await commitTextBox(page, null, [x, y], addDraft(text));
}
/** draw-signature D5/D6: an Ink this app drew as a signature (by its `/Subj`). */
function isDrawnSignature(item: AnnotItem): boolean { return item.type === "Ink" && item.subject === DRAWN_SIGNATURE_SUBJECT; }
/** sign-merge D4: a drawn Ink or a signature-appearance Stamp (image signatures and the old dialog's) — one kind of target. */
function isSignature(item: AnnotItem): boolean { return isDrawnSignature(item) || item.type === "Stamp" && item.subject === SIGNATURE_SUBJECT; }
const SIGN_ROTATED = "A signature cannot be placed on a rotated page";
const SIGN_MIN_WIDTH = 24;
const sigRect = (id: number): number[] | null => notes.find(item => item.id === id)?.rect.map(round2) ?? null;
/** draw-signature D4: pad CSS px of a pointer event. */
function padPointOf(event: PointerEvent): [number, number] { const box = sigpadCanvas.getBoundingClientRect(); return [event.clientX - box.left, event.clientY - box.top]; }
/**
 * draw-signature D4 (amended at acceptance): a 65th stroke is refused, a sample under 2 px from the last is skipped,
 * 2048 points stop a stroke, a tap (1 point) is kept as a dot. The whole drawing stops at 4096 points: mupdf's Ink
 * appearance is quadratic in the point count (about 1 s at 4096, minutes beyond), and 2 px steps keep a real
 * signature far below that.
 */
function padBegin(point: [number, number]): boolean {
  dropPadImage(); // sign-merge D1: drawing discards a chosen image
  if (padStrokes.length >= MAX_PAD_STROKES) { sigpadStatus.textContent = "A signature can have at most 64 strokes"; return false; }
  if (padCount() + 2 > MAX_PAD_POINTS) { padFull = true; sigpadStatus.textContent = PAD_FULL; return false; }
  padStroke = [point]; drawPad(); return true;
}
function padPoint(point: [number, number]): void {
  const stroke = padStroke; if (!stroke || stroke.length >= MAX_INK_POINTS) return;
  const last = stroke[stroke.length - 1]; if (Math.hypot(point[0] - last[0], point[1] - last[1]) < 2) return;
  if (padCount() >= MAX_PAD_POINTS) { padFull = true; sigpadStatus.textContent = PAD_FULL; return; }
  stroke.push(point); drawPad();
}
function padEnd(): void {
  const stroke = padStroke; padStroke = null;
  if (stroke) { if (stroke.length === 1) stroke.push([stroke[0][0] + 0.5, stroke[0][1]]); padStrokes.push(stroke); } // round caps draw the 0.5 px stroke as a dot
  syncPad(); drawPad();
}
const MAX_PAD_STROKES = 64, MAX_PAD_POINTS = 4096;
const PAD_FULL = "The drawing is too long; press Clear and draw it again";
const padCount = () => padStrokes.reduce((sum, stroke) => sum + stroke.length, padStroke?.length ?? 0);
function padBounds(): number[] | null {
  // Review: a loop, not Math.min(...xs) — 64 × 2048 points overflow the argument stack.
  const points = padStrokes.flat(); if (!points.length) return null;
  return points.reduce((box, [x, y]) => [Math.min(box[0], x), Math.min(box[1], y), Math.max(box[2], x), Math.max(box[3], y)], [Infinity, Infinity, -Infinity, -Infinity]);
}
function syncPad(): void { const bounds = padBounds(); sigpadSaveButton.disabled = !padImage && (!bounds || bounds[2] - bounds[0] < 8); sigpadRemoveButton.disabled = savedSignature === null; }
/** White paper, black ink, backing store × devicePixelRatio (both themes). */
function drawPad(): void {
  if (!sigpadDialog.open) return; // review: a closed pad (sign-draw) is not repainted per point (that is O(n²))
  const ratio = window.devicePixelRatio || 1, width = sigpadCanvas.clientWidth || 520, height = sigpadCanvas.clientHeight || 180;
  if (sigpadCanvas.width !== Math.round(width * ratio) || sigpadCanvas.height !== Math.round(height * ratio)) { sigpadCanvas.width = Math.round(width * ratio); sigpadCanvas.height = Math.round(height * ratio); }
  const context = sigpadCanvas.getContext("2d"); if (!context) return;
  context.setTransform(ratio, 0, 0, ratio, 0, 0); context.fillStyle = "#fff"; context.fillRect(0, 0, width, height);
  // sign-merge D1: a chosen image is painted scaled to fit, aspect kept.
  if (padImage) { const fit = Math.min(width / padImage.width, height / padImage.height), w = padImage.width * fit, h = padImage.height * fit; context.drawImage(padImage.bitmap, (width - w) / 2, (height - h) / 2, w, h); }
  context.strokeStyle = "#000"; context.lineWidth = 2; context.lineCap = "round"; context.lineJoin = "round";
  for (const stroke of padStroke ? [...padStrokes, padStroke] : padStrokes) {
    context.beginPath(); stroke.forEach(([x, y], index) => index ? context.lineTo(x, y) : context.moveTo(x, y));
    if (stroke.length === 1) context.lineTo(stroke[0][0] + 0.1, stroke[0][1]);
    context.stroke();
  }
}
function clearPad(): void { dropPadImage(); padStrokes = []; padStroke = null; padFull = false; sigpadStatus.textContent = ""; syncPad(); drawPad(); }
function dropPadImage(): void { padImage?.bitmap.close(); padImage = null; }
const SIGN_IMAGE_ONE = "A signature uses one image; pick a single file.", SIGN_IMAGE_LARGE = "That image is larger than this app will read.";
const SIGN_IMAGE_UNREADABLE = "That image could not be read.", SIGN_IMAGE_NOT = "That image is not a PNG or JPEG this app can read.";
/** sign-merge D2: base64 in chunks (a spread of 4 MiB arguments overflows the stack) and back. */
function base64Of(bytes: ArrayBuffer): string { const view = new Uint8Array(bytes); let text = ""; for (let i = 0; i < view.length; i += 0x8000) text += String.fromCharCode(...view.subarray(i, i + 0x8000)); return btoa(text); }
function bytesOf(data: string): ArrayBuffer { const text = atob(data), view = new Uint8Array(text.length); for (let i = 0; i < text.length; i++) view[i] = text.charCodeAt(i); return view.buffer; }
/**
 * sign-merge D1: "Use an image…" — exactly one PNG or JPEG through the picker, decoded here with createImageBitmap
 * (no object URL), shown fitted in the pad in place of any strokes. null = chosen, "" = cancelled, else the refusal (nothing changed).
 */
async function padUseImage(): Promise<string | null> {
  if (!api || typeof api.pickImages !== "function") return "Choosing an image needs the desktop app (window.pdf4all is unavailable).";
  let picked;
  try { picked = await api.pickImages(); } catch { return SIGN_IMAGE_UNREADABLE; }
  if (!picked.ok) return picked.reason === "cancelled" ? "" : picked.reason === "too-many" ? SIGN_IMAGE_ONE : picked.reason === "too-large" ? SIGN_IMAGE_LARGE : SIGN_IMAGE_UNREADABLE;
  if (picked.images.length > 1) return SIGN_IMAGE_ONE;
  const bytes = picked.images[0]?.bytes; if (!bytes) return "";
  if (bytes.byteLength > MAX_STAMP_IMAGE_BYTES) return SIGN_IMAGE_LARGE;
  const head = new Uint8Array(bytes, 0, Math.min(8, bytes.byteLength)), starts = (magic: number[]) => magic.every((byte, index) => head[index] === byte);
  let bitmap: ImageBitmap | null = null;
  if (starts([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]) || starts([0xff, 0xd8, 0xff])) { try { bitmap = await createImageBitmap(new Blob([bytes])); } catch { bitmap = null; } }
  if (!bitmap || bitmap.width < 1 || bitmap.height < 1 || bitmap.width > 16384 || bitmap.height > 16384) { bitmap?.close(); return SIGN_IMAGE_NOT; }
  // Review F1: Chromium decodes upright (EXIF orientation applied) but mupdf draws the file's pixels as stored, so the
  // picked file itself would place a phone photo sideways. Keep a fresh PNG of the decoded pixels instead: upright, and
  // no EXIF/GPS or other metadata ever reaches a PDF. ponytail: long side capped at 1000 px (5 px per point on the
  // 200 pt default; 1000×1000 RGBA stays under the store's 4 MiB) — raise both together if signatures print soft.
  const scale = Math.min(1, 1000 / Math.max(bitmap.width, bitmap.height));
  const canvas = new OffscreenCanvas(Math.max(1, Math.round(bitmap.width * scale)), Math.max(1, Math.round(bitmap.height * scale)));
  const context = canvas.getContext("2d");
  if (!context) { bitmap.close(); return SIGN_IMAGE_UNREADABLE; }
  context.imageSmoothingQuality = "high";
  context.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
  bitmap.close();
  let png: ArrayBuffer;
  try { png = await (await canvas.convertToBlob({ type: "image/png" })).arrayBuffer(); } catch { return SIGN_IMAGE_UNREADABLE; }
  if (png.byteLength > MAX_STAMP_IMAGE_BYTES) return SIGN_IMAGE_LARGE;
  const { width, height } = canvas;
  dropPadImage(); padStrokes = []; padStroke = null; padFull = false;
  padImage = { bitmap: canvas.transferToImageBitmap(), width, height, data: base64Of(png) };
  sigpadStatus.textContent = ""; syncPad(); drawPad();
  return null;
}
/** The pad always opens empty; `thenArm` = a Sign press with nothing saved (its Save arms placement). */
function openSigpad(thenArm = false): void { if (!sigpadDialog.open) sigpadDialog.showModal(); clearPad(); padThenArm = thenArm; }
async function loadSavedSignature(): Promise<void> {
  try { const reply = await api?.signatureGet?.(); savedSignature = reply?.ok ? reply.signature as DrawnSig | ImageSig | null : null; } catch { savedSignature = null; }
  syncPad();
}
/** draw-signature D4: translate to the origin, round, store through main, re-read, close, announce. sign-merge D1: or the chosen image. */
async function saveDrawnSignature(): Promise<boolean> {
  const bounds = padBounds(); if (!padImage && (!bounds || bounds[2] - bounds[0] < 8)) return false;
  const r2 = (value: number) => Math.round(value * 100) / 100;
  const value: DrawnSig | ImageSig = padImage ? { v: 1, kind: "image", width: padImage.width, height: padImage.height, data: padImage.data }
    : { v: 1, width: r2(bounds![2] - bounds![0]), height: r2(bounds![3] - bounds![1]), strokes: padStrokes.map(stroke => stroke.map(([x, y]) => [r2(x - bounds![0]), r2(y - bounds![1])] as [number, number])) };
  let ok = false;
  try { ok = (await api?.signatureSet?.(value))?.ok === true; } catch { ok = false; }
  if (!ok) { sigpadStatus.textContent = "The signature could not be saved"; return false; }
  await loadSavedSignature();
  const thenArm = padThenArm; padThenArm = false;
  if (sigpadDialog.open) sigpadDialog.close();
  announce("Signature saved");
  if (thenArm) armSign();
  return true;
}
async function removeDrawnSignature(): Promise<boolean> {
  let ok = false;
  try { ok = (await api?.signatureSet?.(null))?.ok === true; } catch { ok = false; }
  if (!ok) return false;
  await loadSavedSignature();
  sigpadStatus.textContent = "Saved signature removed"; announce("Saved signature removed");
  return true;
}
/** draw-signature D5: armed placement — the next plain click on the page, or Enter at the page centre. */
function armSign(): void {
  if (!doc || !canAnnotate()) return;
  if (uiMode !== "forms") setMode("forms");
  signArmed = true; fillSignButton.setAttribute("aria-pressed", "true");
  paintHits(canvas.clientWidth, canvas.clientHeight, shownPage);
  pageRegion.focus({ preventScroll: true });
  announce("Click the page to place your signature");
}
function disarmSign(): void {
  if (!signArmed) return;
  signArmed = false; fillSignButton.setAttribute("aria-pressed", "false");
  paintHits(canvas.clientWidth, canvas.clientHeight, shownPage);
}
/** `#fill-sign`, Edit ▸ Place Signature and `sign-place` with no contents: a second press disarms; nothing saved opens the pad first. */
function pressSign(): void {
  if (signArmed) { disarmSign(); return; }
  if (!doc || !canAnnotate()) return;
  if (!savedSignature) { openSigpad(true); return; }
  armSign();
}
async function placeSignatureAtCentre(): Promise<void> {
  const size = doc?.pages[wantedPage];
  if (size) await placeSignature(wantedPage, [size.width / 2, size.height / 2]); else disarmSign();
}
/** draw-signature D5: the one placement — fit inside SIGNATURE_SIZE (or `width`, clamped as the engine's D2), centred on the point, inside the page. */
async function placeSignature(page: number, point: number[], width?: number): Promise<boolean> {
  disarmSign();
  const size = doc?.pages[page], sig = savedSignature;
  if (!doc || !size) return false;
  const refuse = (message: string) => { announce(message); showError(message); return false; };
  if ((doc.rotations?.[page] ?? 0) % 360 !== 0) return refuse(SIGN_ROTATED);
  if (!sig) return refuse("Draw your signature first");
  if (!canAnnotate()) return refuse("This document does not permit annotations");
  const [boxW, boxH] = SIGNATURE_SIZE, w0 = sig.width, h0 = sig.height;
  // Review: the default fit is also inside the page (a page under 200×60 pt otherwise clamps, i.e. squashes, the points).
  const scale = width === undefined ? Math.min(boxW / w0, size.width / w0, h0 > 0 ? Math.min(boxH, size.height) / h0 : Infinity)
    : Math.min(Math.max(width, SIGN_MIN_WIDTH), size.width, h0 > 0 ? size.height * w0 / h0 : Infinity) / w0;
  const w = w0 * scale, h = h0 * scale;
  const left = Math.max(0, Math.min(size.width - w, point[0] - w / 2)), top = Math.max(0, Math.min(size.height - h, point[1] - h / 2));
  const at = (value: number, most: number) => Math.max(0, Math.min(most, Math.round(value * 1e4) / 1e4));
  clearError();
  try {
    const before = new Set(((await request({ op: "annots", page })) as AnnotsReply).items.map(item => item.id));
    // sign-merge D4: an image is one signature Stamp over the fitted rect (the engine keeps the rect asked for).
    const reply = await request(sig.kind === "image"
      ? { op: "annot-add", page, type: "Stamp", rect: [at(left, size.width), at(top, size.height), at(left + w, size.width), at(top + h, size.height)], image: { name: "signature", bytes: bytesOf(sig.data) }, signature: true, contents: "" }
      : { op: "annot-add", page, type: "Ink", strokes: sig.strokes.map(stroke => stroke.map(([x, y]) => [at(left + x * scale, size.width), at(top + y * scale, size.height)])), contents: "", signature: true }) as MutationReply;
    noteFilled();
    await applyEdit(reply, "Added signature");
    const id = notes.find(item => isSignature(item) && !before.has(item.id))?.id ?? -1;
    fillSignLast = fillOp("add", id, sigRect(id)); directFocusedSig = id < 0 ? null : id;
    if (id >= 0) hitLayer.querySelector<HTMLElement>(`.signature-target[data-annot-id="${id}"]`)?.focus({ preventScroll: true });
    return true;
  } catch (err) {
    const { reply } = refusalOf(err);
    return refuse(typeof reply?.error === "string" ? reply.error : messageOf(err));
  }
}
/** draw-signature D6: one `annot-move` (point = new top-left; `width` on a resize); a side handle that only changed the height scales the width alike. */
async function commitSignatureRect(annotId: number, page: number, initial: number[], rect: number[], resize: boolean): Promise<boolean> {
  directFocusedSig = annotId;
  const w0 = initial[2] - initial[0], h0 = initial[3] - initial[1];
  const width = !resize ? undefined : Math.abs(rect[2] - rect[0] - w0) <= 0.01 && h0 > 0 ? w0 * (rect[3] - rect[1]) / h0 : rect[2] - rect[0];
  try {
    const reply = await request({ op: "annot-move", page, annotId, point: [rect[0], rect[1]].map(value => Math.round(value * 1e4) / 1e4), ...(width === undefined ? {} : { width: Math.round(width * 1e4) / 1e4 }) }) as MutationReply & { noop?: boolean };
    if (reply.noop === true) return true;
    const resized = width !== undefined && Math.abs(width - w0) > 0.01;
    noteFilled();
    await applyEdit(reply, resized ? "Resized signature" : "Moved signature");
    fillSignLast = fillOp(resized ? "resize" : "move", annotId, sigRect(annotId));
    return true;
  } catch (err) {
    const { reply } = refusalOf(err);
    showError(typeof reply?.error === "string" ? reply.error : messageOf(err));
    return false;
  }
}
async function deleteSignature(annotId: number): Promise<void> {
  directFocusedSig = null;
  await deleteNote(annotId, "Deleted signature");
  if (notes.some(item => item.id === annotId)) return;
  noteFilled(); fillSignLast = fillOp("delete", annotId, null);
}
/** draw-signature D6: drag moves, handles resize (aspect kept, Shift too), arrows nudge, +/- resize 10 % about the top-left, Delete removes; click and Enter only focus. */
function paintSignatureTarget(item: AnnotItem, page: number, index: number): HTMLButtonElement {
  const rect = [...item.rect], button = document.createElement("button"); button.type = "button";
  button.className = "hit-target signature-target"; button.dataset.annotId = String(item.id);
  button.setAttribute("aria-label", `Signature ${index + 1}`);
  button.title = canAnnotate() ? "Signature: drag to move, handles or +/- resize, Delete removes it" : "This document is read-only";
  placeBox(button, rect, shownZoom);
  const w0 = rect[2] - rect[0], h0 = rect[3] - rect[1];
  // gestureRect's corner lock keeps both sides ≥ min; this min keeps the width ≥ 24 pt without forcing a short signature taller.
  const min = h0 > 0 && w0 > 0 ? SIGN_MIN_WIDTH * Math.min(1, h0 / w0) : SIGN_MIN_WIDTH;
  if (canAnnotate()) {
    const start = (event: PointerEvent, handle: string) => { directFocusedSig = item.id; startRectGesture(event, { rect, id: item.id, handle, min, lockAspect: true, keepAspect: true, can: canAnnotate, commit: next => commitSignatureRect(item.id, page, rect, next, handle !== "grip") }); };
    addFieldHandles(button, start);
    button.addEventListener("pointerdown", event => { if (event.button === 0 && !event.ctrlKey && !(event.target as HTMLElement).closest(".hit-handle")) start(event, "grip"); });
  }
  button.addEventListener("click", event => {
    event.preventDefault(); if ((event.target as HTMLElement).closest(".hit-handle")) return;
    directFocusedSig = item.id; button.focus({ preventScroll: true }); directClickWork = Promise.resolve();
  });
  button.addEventListener("keydown", event => {
    if (event.key === "Escape") { event.preventDefault(); dropDirectFocus(); return; }
    const size = doc?.pages[page];
    let work: (() => Promise<unknown>) | null = null;
    if (ARROW_KEYS.includes(event.key) && !event.metaKey && !event.ctrlKey) { if (size && canAnnotate()) { const next = nudgeRect(rect, { key: event.key, shiftKey: event.shiftKey, metaKey: false }, 1, size); work = () => commitSignatureRect(item.id, page, rect, next, false); } }
    else if (event.key === "+" || event.key === "=" || event.key === "-") { if (canAnnotate()) { const factor = event.key === "-" ? 0.9 : 1.1; work = () => commitSignatureRect(item.id, page, rect, [rect[0], rect[1], rect[0] + w0 * factor, rect[1] + h0 * factor], true); } }
    else if (event.key === "Delete" || event.key === "Backspace") { if (canAnnotate()) work = () => deleteSignature(item.id); }
    else if (event.key === "Enter" || event.key === " ") { event.preventDefault(); directFocusedSig = item.id; return; }
    else return;
    event.preventDefault();
    if (work && !directGestureWork) runGestureWork(work);
  });
  return button;
}
/** draw-signature D7: `sign-draw {strokes}` runs the pad's own recorder and Save; `{clear:true}` removes it. No document, no journal, never dirty.
 * sign-merge D6: `{image:true}` runs the pad's "Use an image…" (the line's `images` pre-answer) and its Save; one of the three keys only. */
async function driveSignDraw(contents: string | null): Promise<void> {
  clearError();
  // Review: preload sends an over-cap (> 4 MiB) sign-draw as "" so it is refused here, not dropped.
  if (contents === "") { showError("sign-draw: contents are empty or larger than 4 MiB"); return; }
  const value = driveWanted(contents); if (!value) return;
  if (["strokes", "image", "clear"].filter(key => key in value).length > 1) { showError("sign-draw: give one of strokes, image or clear"); return; }
  if (value.image === true) {
    padThenArm = false;
    const refused = await padUseImage();
    if (refused !== null) { showError(`sign-draw: ${refused || "no image was given"}`); return; }
    if (!await saveDrawnSignature()) showError("sign-draw: the signature could not be saved");
    return;
  }
  if (value.clear === true) { if (!await removeDrawnSignature()) showError("sign-draw: the saved signature could not be removed"); return; }
  const strokes = value.strokes;
  const isPoint = (point: unknown) => Array.isArray(point) && point.length === 2 && point.every(n => typeof n === "number" && Number.isFinite(n));
  if (!Array.isArray(strokes) || !strokes.every(stroke => Array.isArray(stroke) && stroke.length > 0 && stroke.every(isPoint))) { showError("sign-draw: strokes must be a list of strokes of [x, y] points, or clear must be true"); return; }
  if (strokes.length > MAX_PAD_STROKES) { showError("sign-draw: a signature can have at most 64 strokes"); return; }
  padThenArm = false; clearPad();
  for (const stroke of strokes as [number, number][][]) { padBegin(stroke[0]); for (const point of stroke.slice(1)) padPoint(point); padEnd(); }
  if (padFull) { clearPad(); showError("sign-draw: a signature can have at most 4096 points (points under 2 px apart are skipped)"); return; }
  if (sigpadSaveButton.disabled) { showError("sign-draw: the drawing needs a stroke and must be at least 8 px wide"); return; }
  if (!await saveDrawnSignature()) showError("sign-draw: the signature could not be saved");
}
/** draw-signature D7: `sign-place {page, x, y, width?}` — Fill forms (OP_MODE), that page, `placeSignature`; no contents arms (the menu path). */
async function driveSignPlace(contents: string | null): Promise<void> {
  clearError();
  if (contents === null) { pressSign(); return; }
  const value = driveWanted(contents); if (!value || !doc) return;
  const { page, x, y, width } = value;
  if (typeof page !== "number" || !Number.isInteger(page) || typeof x !== "number" || !Number.isFinite(x) || typeof y !== "number" || !Number.isFinite(y)) { showError("sign-place: page, x and y are required"); return; }
  if (width !== undefined && (typeof width !== "number" || !Number.isFinite(width))) { showError("sign-place: width must be a number"); return; }
  if (page < 0 || page >= doc.pageCount) { showError(`sign-place: page must be an integer between 0 and ${doc.pageCount - 1}`); return; }
  if (!savedSignature) { showError("sign-place: no signature has been drawn"); return; }
  if ((doc.rotations?.[page] ?? 0) % 360 !== 0) { announce(SIGN_ROTATED); showError(SIGN_ROTATED); return; }
  if (!canAnnotate()) { showError("sign-place: this document does not permit annotations"); return; }
  if (directActive) await finishDirect();
  goTo(page); await drain();
  while (doc && (shownPage !== page || inFlight !== null)) await new Promise(resolve => window.setTimeout(resolve, 10));
  await placeSignature(page, [x, y], width as number | undefined);
}
fillSignButton.addEventListener("click", () => pressSign());
fillSignDrawButton.addEventListener("click", () => { if (doc && canAnnotate()) openSigpad(false); });
sigpadClearButton.addEventListener("click", () => clearPad());
sigpadImageButton.addEventListener("click", () => { void padUseImage().then(refused => { if (refused) sigpadStatus.textContent = refused; }); });
sigpadRemoveButton.addEventListener("click", () => { void removeDrawnSignature(); });
sigpadSaveButton.addEventListener("click", () => { void saveDrawnSignature(); });
sigpadCancelButton.addEventListener("click", () => sigpadDialog.close());
// Cancel, Escape and Save all close here; only Save wrote anything.
sigpadDialog.addEventListener("close", () => { padThenArm = false; padStrokes = []; padStroke = null; dropPadImage(); });
sigpadCanvas.addEventListener("pointerdown", event => {
  if (event.button !== 0) return;
  event.preventDefault();
  if (!padBegin(padPointOf(event))) return;
  try { sigpadCanvas.setPointerCapture(event.pointerId); } catch { /* a synthetic pointer has nothing to capture */ }
});
sigpadCanvas.addEventListener("pointermove", event => {
  if (!padStroke) return;
  const samples = typeof event.getCoalescedEvents === "function" ? event.getCoalescedEvents() : [];
  for (const sample of samples.length ? samples : [event]) padPoint(padPointOf(sample));
});
sigpadCanvas.addEventListener("pointerup", () => padEnd());
sigpadCanvas.addEventListener("pointercancel", () => padEnd());
void loadSavedSignature();
function removeDirect(active: NonNullable<typeof directActive>): void { active.editor.remove(); active.optionList?.remove(); }
function closeDirect(): void { const active = directActive; directActive = null; if (active) removeDirect(active); paintHits(canvas.clientWidth, canvas.clientHeight, shownPage); }
async function focusNextField(reverse = false, fromPage = wantedPage, fromIndex = directFieldIndex): Promise<void> {
  if (!doc) return;
  const ordered = tabOrder(fields, fieldsTabs).order.filter(item => !item.readOnly && item.kind !== "signature");
  const next = fromIndex + (reverse ? -1 : 1);
  if (fromPage === wantedPage && next >= 0 && next < ordered.length) { directFieldIndex = next; await activateField(ordered[next], false); return; }
  const direction = reverse ? -1 : 1;
  for (let step = 1; step <= doc.pageCount; step++) {
    const page = (fromPage + direction * step + doc.pageCount) % doc.pageCount;
    const reply = await request({ op: "fields", page }) as FieldsReply;
    const candidates = readingOrder(reply.items).filter(item => !item.readOnly && item.kind !== "signature");
    if (!candidates.length) continue;
    goTo(page);
    while (doc && (shownPage !== page || inFlight !== null)) await new Promise(resolve => window.setTimeout(resolve, 10));
    await refreshFields();
    const current = tabOrder(fields, fieldsTabs).order.filter(item => !item.readOnly && item.kind !== "signature");
    if (!current.length) continue;
    directFieldIndex = reverse ? current.length - 1 : 0;
    await activateField(current[directFieldIndex], false);
    return;
  }
}
async function moveRadio(item: FieldItem, delta: number): Promise<void> {
  const group = fields.filter(field => field.kind === "radio" && field.name === item.name).sort((a, b) => a.rect[0] - b.rect[0] || a.rect[1] - b.rect[1]);
  if (!group.length) return;
  const index = Math.max(0, group.findIndex(field => field.id === item.id));
  const next = group[(index + delta + group.length) % group.length];
  await commitField(next, true);
  hitLayer.querySelector<HTMLElement>(`[data-field-id="${CSS.escape(String(next.id))}"]`)?.focus();
}
async function activateField(item: FieldItem, commit = true): Promise<void> {
  directFieldIndex = tabOrder(fields, fieldsTabs).order.filter(field => !field.readOnly && field.kind !== "signature").findIndex(field => field.id === item.id || field.kind === "radio" && field.name === item.name);
  if (item.kind === "signature") {
    // P5 U3 §2.2: an empty field opens the sign dialog on it; a signed one opens Verify.
    directFocusedField = null;
    if (sigSigned(item)) { fieldsStatusEl.textContent = "Verify signatures"; await openVerifyDialog(); markVerifyRow(item.name); }
    else { fieldsStatusEl.textContent = "Sign with certificate"; openCertSignDialog(item); }
    return;
  }
  if (item.readOnly || !canForm()) { directFocusedField = null; fieldsStatusEl.textContent = item.readOnly ? "This field is read-only." : "Form filling is not permitted."; return; }
  directFocusedField = item; directFocusedPage = wantedPage;
  if (!commit) fieldsStatusEl.textContent = "";
  clearDirectFocus(); closeDirect();
  if (item.kind === "checkbox" || item.kind === "radio") {
    if (!commit) { closeDirect(); hitLayer.querySelector<HTMLElement>(`[data-field-id="${CSS.escape(String(item.id))}"]`)?.focus(); return; }
    await commitField(item, item.kind === "checkbox" ? item.value !== true : true); directFocusedField = null; return;
  }
  const editor = item.kind === "text" && item.multiline ? document.createElement("textarea") : item.kind === "combo" && !item.edit ? document.createElement("select") : document.createElement("input");
  let optionList: HTMLElement | undefined;
  if (editor instanceof HTMLInputElement) editor.type = "text";
  if (editor instanceof HTMLSelectElement) for (const option of item.options ?? []) { const node = document.createElement("option"); node.value = option; node.textContent = option; editor.append(node); }
  if (item.kind === "combo" && item.edit && editor instanceof HTMLInputElement) { const list = document.createElement("datalist"); list.id = `direct-options-${item.id}`; for (const option of item.options ?? []) { const node = document.createElement("option"); node.value = option; list.append(node); } editor.setAttribute("list", list.id); optionList = list; hitLayer.append(list); }
  editor.value = String(item.value); if (editor instanceof HTMLInputElement || editor instanceof HTMLTextAreaElement) editor.maxLength = item.maxLen ?? (item.kind === "combo" ? MAX_FIELD_TEXT_CHARS : 4096); editor.className = "inline-editor"; editor.setAttribute("aria-label", fieldLabel(item)); editor.title = item.tu || fieldLabel(item); placeBox(editor, item.rect, shownZoom); hitLayer.append(editor);
  directActive = { kind: "field", item, editor, page: wantedPage, optionList }; directFieldIndex = tabOrder(fields, fieldsTabs).order.filter(field => !field.readOnly && field.kind !== "signature").findIndex(field => field.id === item.id); setupInlineEvents(editor, item.kind === "text" && !!item.multiline); editor.focus(); if (!(editor instanceof HTMLSelectElement)) editor.select();
}
function paragraphText(paragraph: ParagraphItem, scan = contentScan): string {
  const ids = new Set(paragraph.targetIds);
  return (scan?.items ?? []).filter(item => ids.has(item.targetId)).map(item => item.text ?? "").join(" ");
}
function markActiveTextTarget(active: NonNullable<typeof directActive>): void {
  if (active.kind !== "text") return;
  const singleRun = !active.paragraph && !!(active.item as EditTarget).paragraphId;
  for (const target of hitLayer.querySelectorAll<HTMLElement>(".text-target")) {
    const matches = active.paragraph ? target.dataset.paragraphId === active.paragraph.paragraphId : target.dataset.targetId === String((active.item as EditTarget).targetId);
    target.classList.toggle("active", matches); target.classList.toggle("single-run-active", matches && singleRun);
  }
}
function openInlineText(item: EditTarget, singleRun = false): void {
  if (!contentScan || !item.bounds) return;
  const scan = contentScan, paragraph = !singleRun && item.paragraphId ? scan.paragraphs.find(value => value.paragraphId === item.paragraphId) : undefined;
  clearDirectFocus(); closeDirect();
  const editor = paragraph ? document.createElement("textarea") : document.createElement("input");
  if (editor instanceof HTMLInputElement) editor.type = "text";
  editor.value = paragraph ? paragraphText(paragraph) : item.text ?? ""; editor.maxLength = 4096;
  editor.className = `inline-editor text-inline-editor${paragraph ? " paragraph" : ""}`; editor.dataset.fontSize = String(paragraph?.fontSize ?? item.fontSize ?? 12);
  editor.setAttribute("aria-label", paragraph ? `Paragraph, ${paragraph.lines} lines` : item.text ?? "Text");
  placeBox(editor, paragraph?.bounds ?? item.bounds, shownZoom);
  if (paragraph) { editor.style.width = `${paragraph.width * shownZoom}px`; editor.style.lineHeight = `${paragraph.leading * shownZoom}px`; editor.style.whiteSpace = "pre-wrap"; editor.addEventListener("input", () => growParagraphEditor(editor)); }
  hitLayer.append(editor); if (paragraph) growParagraphEditor(editor, (paragraph.bounds[3] - paragraph.bounds[1]) * shownZoom); directActive = { kind: "text", item, editor, page: scan.page, scan, paragraph };
  markActiveTextTarget(directActive);
  setupInlineEvents(editor, !!paragraph); editor.focus(); editor.select();
}
function setupInlineEvents(editor: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement, multiline: boolean): void {
  editor.addEventListener("keydown", event => { const key = event as KeyboardEvent; if (key.key === "Escape") { key.preventDefault(); if (directActive?.kind === "add") clearError(); directFocusedField = null; closeDirect(); } else if (key.altKey && ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(key.key) && directActive?.kind === "field") { key.preventDefault(); void nudgeField(directActive.item as FieldItem, key); } else if (key.key === "Tab") { key.preventDefault(); if (directActive?.kind === "add" || directActive?.kind === "link" || directActive?.kind === "textbox") { void finishDirect(); return; } const reverse = key.shiftKey, page = directActive?.page ?? wantedPage, index = directFieldIndex; void finishDirect().then(() => focusNextField(reverse, page, index)); } else if (key.key === "Enter" && (!multiline || key.metaKey || key.ctrlKey)) { key.preventDefault(); void finishDirect(); } });
  editor.addEventListener("blur", () => { if (directActive?.editor === editor) directFinishWork = finishDirect(); });
}
/** An editor whose value is still the document's commits nothing (no journal op, not dirty). */
function directUnchanged(active: NonNullable<typeof directActive>): boolean {
  if (active.kind === "add") return addDraft(active.editor.value) === "";
  if (active.kind === "textbox") return active.annotId == null ? addDraft(active.editor.value) === "" : addDraft(active.editor.value) === addDraft(active.editor.dataset.initial ?? "");
  if (active.kind === "link") return active.editor.value.trim() === (active.editor.dataset.initial ?? "");
  return active.editor.value === (active.kind === "field" ? String((active.item as FieldItem).value) : active.paragraph ? paragraphText(active.paragraph, active.scan) : (active.item as EditTarget).text ?? "");
}
let directCommitting = false;
/** Closes the editor now and commits its draft in the background (mode or page change). */
function dropDirect(): void {
  const active = directActive; if (!active) return;
  directActive = null; directFocusedField = null; removeDirect(active);
  if (directCommitting || directUnchanged(active)) return;
  if (active.kind === "field") void commitField(active.item as FieldItem, active.editor.value, active.page);
  else if (active.kind === "link") void commitLinkEditor(active, true);
  else if (active.kind === "add") void addTextAt(active.page, addPoint(active), addDraft(active.editor.value));
  else if (active.kind === "textbox") void commitTextBox(active.page, active.annotId ?? null, active.point ?? [0, 0], addDraft(active.editor.value));
  else if (active.paragraph) void applyParagraphEdit(active.scan!, active.paragraph, active.editor.value);
  else void applyContentEdit(active.scan!, active.item as EditTarget, active.editor.value);
}
async function finishDirect(): Promise<void> {
  const active = directActive; if (!active || directCommitting) return;
  if (active.kind === "field" && ((active.item as FieldItem).kind === "checkbox" || (active.item as FieldItem).kind === "radio")) { closeDirect(); return; }
  if (directUnchanged(active)) { directFocusedField = null; closeDirect(); return; }
  const value = active.editor.value;
  // One commit per editor: Enter then blur (or a page change) must not send a second field-set.
  directCommitting = true;
  let ok: boolean;
  try {
    ok = active.kind === "field" ? await commitField(active.item as FieldItem, value, active.page) : active.kind === "link" ? await commitLinkEditor(active) : active.kind === "add" ? await addTextAt(active.page, addPoint(active), addDraft(value)) : active.kind === "textbox" ? await commitTextBox(active.page, active.annotId ?? null, active.point ?? [0, 0], addDraft(value)) : active.paragraph ? await applyParagraphEdit(active.scan!, active.paragraph, value) : await applyContentEdit(active.scan!, active.item as EditTarget, value);
    // §8.4: the editor covers the old appearance until the new render has painted.
    if (ok) for (let i = 0; i < 200 && doc && shownKey !== target()?.key; i++) await new Promise(resolve => window.setTimeout(resolve, 10));
  } finally { directCommitting = false; }
  if (ok) {
    directFocusedField = null; if (directActive === active) closeDirect(); else removeDirect(active);
    // §9.7: focus the new run only when nothing else took the page meanwhile.
    if (active.kind === "add" && !directActive && contentLastAdd?.targetId) hitLayer.querySelector<HTMLElement>(`.text-target[data-target-id="${CSS.escape(contentLastAdd.targetId)}"]`)?.focus({ preventScroll: true });
  } else if (active.kind !== "add" && active.kind !== "link" && active.kind !== "textbox") active.editor.value = active.kind === "field" ? String((active.item as FieldItem).value) : active.paragraph ? paragraphText(active.paragraph) : (active.item as EditTarget).text ?? "";
}
/** P6 U3 §9.9: base-14 names in CSS terms, so the draft looks like what lands. */
function addFontCss(name: string): { family: string; weight: string; style: string } {
  const family = name.startsWith("Times") ? "'Times New Roman', Times, serif" : name.startsWith("Courier") ? "'Courier New', Courier, monospace" : "Helvetica, Arial, sans-serif";
  return { family, weight: /Bold/.test(name) ? "bold" : "normal", style: /Italic|Oblique/.test(name) ? "italic" : "normal" };
}
function addFontSize(): number { return Math.min(400, Math.max(4, Number(contentAddSize.value) || 12)); }
/** §9.5: leading and trailing blank lines are dropped; whitespace only is empty. */
function addDraft(value: string): string {
  const lines = value.split("\n");
  while (lines.length && !lines[0].trim()) lines.shift();
  while (lines.length && !lines[lines.length - 1].trim()) lines.pop();
  return lines.join("\n");
}
/** §9.5: the baseline ≈ top + 0.8·size for base-14, kept on the page. */
function addPoint(active: NonNullable<typeof directActive>): number[] {
  const point = active.point ?? [0, 0], size = Number(active.editor.dataset.fontSize) || 12, height = doc?.pages[active.page]?.height ?? Infinity;
  return [point[0], Math.min(point[1] + 0.8 * size, height)];
}
function openAddText(page: number, point: [number, number]): void {
  clearDirectFocus(); closeDirect(); clearError();
  const size = addFontSize(), font = addFontCss(contentAddFont.value), rect = [point[0], point[1], point[0] + Math.max(8 * size, 48), point[1] + 1.2 * size];
  const editor = document.createElement("textarea");
  editor.className = "inline-editor text-inline-editor add-text"; editor.wrap = "off"; editor.maxLength = 4096; editor.spellcheck = false;
  editor.dataset.fontSize = String(size); editor.setAttribute("aria-label", `New text at ${point[0].toFixed(1)}, ${point[1].toFixed(1)} pt`);
  editor.style.fontFamily = font.family; editor.style.fontWeight = font.weight; editor.style.fontStyle = font.style; editor.style.color = contentAddColor.value;
  const item = { targetId: "", kind: "text", bounds: rect, support: "candidate", text: "" } as EditTarget;
  placeBox(editor, rect, shownZoom); hitLayer.append(editor);
  const active = { kind: "add" as const, item, editor, page, point };
  directActive = active; growAddEditor(active);
  editor.addEventListener("input", () => { if (directActive === active) growAddEditor(active); });
  setupInlineEvents(editor, true); editor.focus();
}
const ADD_ROTATED_TEXT = "Text cannot be added on a rotated page (unsupported-transform)";
/** §2.1/§9.9: a click on empty page space in Edit text opens the add editor there; true when the click was used. */
function clickToAdd(event: MouseEvent): boolean {
  const blocked = addBlocked; addBlocked = false;
  // P6 U7 §9.27: a Control-click is the macOS right-click; it never adds text.
  // P6 U7 review F3: nor does a Shift-click, which only extends the selection (§9.26).
  if (event.ctrlKey || event.shiftKey || event.button !== 0) return false;
  if (hitLayer.contains(event.target as Node)) return false;
  if ((uiMode === "edit" || uiMode === "redact" || uiMode === "forms") && (directFocusedObject || directFocusedMark || directFocusedBox !== null || directSelection.length)) dropDirectFocus();
  if (uiMode === "forms") return clickToType(event, blocked);
  if (uiMode !== "edit" || blocked || directActive || directCommitting || !layerLive() || !doc || !canEdit()) return false;
  if (!(window.getSelection()?.isCollapsed ?? true)) return false;
  const page = wantedPage, size = doc.pages[page], point = pagePoint(event);
  if (!point || !size || shownPage !== page || point[0] < 0 || point[1] < 0 || point[0] > size.width || point[1] > size.height) return false;
  const scan = contentScan, m = scan?.transform;
  const refuse = () => { showError(ADD_ROTATED_TEXT); contentRefusals.push({ text: "", detail: "unsupported-transform" }); return true; };
  if ((doc.rotations?.[page] ?? 0) % 360 !== 0) return refuse();
  if (!scan || scan.page !== page || scan.docGeneration !== generation || !m) return false;
  if (Math.abs(m[0] - 1) > 1e-6 || Math.abs(m[1]) > 1e-6 || Math.abs(m[2]) > 1e-6 || Math.abs(m[3] + 1) > 1e-6) return refuse();
  openAddText(page, point);
  return true;
}
/** fill-anywhere D3: in Fill forms a click on empty page space opens the text-box editor there (`clickToAdd`'s guards + `canAnnotate`). */
function clickToType(event: MouseEvent, blocked: boolean): boolean {
  if (blocked || directActive || directCommitting || !layerLive() || !doc || !canAnnotate()) return false;
  if (!(window.getSelection()?.isCollapsed ?? true)) return false;
  const page = wantedPage, size = doc.pages[page], point = pagePoint(event);
  if (!point || !size || shownPage !== page || point[0] < 0 || point[1] < 0 || point[0] > size.width || point[1] > size.height) return false;
  if ((doc.rotations?.[page] ?? 0) % 360 !== 0) { showError(TEXTBOX_ROTATED); return true; }
  openTextBox(page, point, null);
  return true;
}
/** §2.4: the one `text-add` both the page editor and the panel send; a refusal shows in #error and `content.refused`. */
async function addTextAt(page: number, point: number[], text: string): Promise<boolean> {
  if (!doc || !canEdit() || text === "") return false;
  clearError();
  const hex = contentAddColor.value.slice(1);
  const color = [0, 2, 4].map(index => parseInt(hex.slice(index, index + 2), 16) / 255);
  let reply: MutationReply & { targetId?: string; bounds?: number[] | null };
  try {
    reply = await request({ op: "text-add", page, point, text, font: contentAddFont.value, size: addFontSize(), color }) as MutationReply & { targetId?: string; bounds?: number[] | null };
  } catch (err) {
    const { reply: refused, reason } = refusalOf(err);
    const detail = typeof refused?.detail === "string" ? refused.detail : reason ?? "invalid";
    const missing = Array.isArray(refused?.missing) ? refused.missing.filter(char => typeof char === "string") : [];
    const message = detail === "unsupported-encoding" && missing.length ? `Cannot draw: ${missing.join(" ")} (not in this font)` : CONTENT_DETAIL_TEXT[detail] ?? messageOf(err);
    contentRefusals.push({ text: "", detail });
    announce(message); showError(message);
    return false;
  }
  contentLastAdd = { targetId: typeof reply.targetId === "string" ? reply.targetId : "", bounds: reply.bounds ?? null };
  await applyEdit(reply, "Added text");
  await refreshContentTargets(true);
  // P6 U8: text added exactly on top of the same text stays ambiguous (overprint) and gets no on-page target.
  if (contentLastAdd.targetId !== "" && !contentLastAdd.bounds) announce("Added text is not selectable on the page (it lies exactly on top of the same text)");
  // R8-3: the engine found no target for the new run (e.g. text-add after undoing a text-add); say so instead of a silent "Added text".
  else if (contentLastAdd.targetId === "") announce("Added text is not editable on the page");
  return true;
}

function paintText(lines: Line[], zoom: number, cssWidth: number, cssHeight: number, page: number) {
  overlay.setAttribute("width", String(cssWidth));
  overlay.setAttribute("height", String(cssHeight));
  overlay.setAttribute("viewBox", `0 0 ${cssWidth} ${cssHeight}`);
  const ns = "http://www.w3.org/2000/svg";
  const fragment = document.createDocumentFragment();
  for (const line of lines) {
    const text = document.createElementNS(ns, "text");
    text.setAttribute("x", `${line.x * zoom}`);
    text.setAttribute("y", `${line.y * zoom}`);
    text.setAttribute("textLength", `${line.w * zoom}`);
    text.setAttribute("lengthAdjust", "spacingAndGlyphs");
    text.style.fontSize = `${line.size * zoom}px`;
    text.style.fontFamily = line.family;
    text.style.fontWeight = line.weight;
    text.style.fontStyle = line.style;
    text.textContent = line.text;
    fragment.appendChild(text);
  }
  searchHits.forEach((hit, index) => {
    if (hit.page !== page) return;
    for (const quad of hit.quads) {
      const xs = [quad[0], quad[2], quad[4], quad[6]];
      const ys = [quad[1], quad[3], quad[5], quad[7]];
      const x = Math.min(...xs) * zoom;
      const y = Math.min(...ys) * zoom;
      const rect = document.createElementNS(ns, "rect");
      rect.setAttribute("x", String(x));
      rect.setAttribute("y", String(y));
      rect.setAttribute("width", String(Math.max(...xs) * zoom - x));
      rect.setAttribute("height", String(Math.max(...ys) * zoom - y));
      rect.setAttribute("class", index === searchIndex ? "hit current" : "hit");
      fragment.appendChild(rect);
    }
  });
  for (const mark of notes.filter(item => item.type === "Redact" && item.rect.length === 4)) {
    const rect = document.createElementNS(ns, "rect");
    rect.setAttribute("x", String(mark.rect[0] * zoom));
    rect.setAttribute("y", String(mark.rect[1] * zoom));
    rect.setAttribute("width", String((mark.rect[2] - mark.rect[0]) * zoom));
    rect.setAttribute("height", String((mark.rect[3] - mark.rect[1]) * zoom));
    rect.setAttribute("class", "redact-mark");
    fragment.appendChild(rect);
  }
  overlay.replaceChildren(fragment);
}

// ----------------------------------------------------------------- search

function onSearchInput() {
  if (searchTimer !== null) {
    window.clearTimeout(searchTimer);
    searchTimer = null;
  }
  const query = searchInput.value.trim();
  if (!query) {
    clearSearch();
    return;
  }
  setSearchStatus("Searching…");
  searchTimer = window.setTimeout(() => {
    searchTimer = null;
    void startSearch();
  }, SEARCH_DEBOUNCE_MS);
}

async function startSearch() {
  const query = searchInput.value.trim();
  if (!query) {
    clearSearch();
    return;
  }
  if (searchTimer !== null) {
    window.clearTimeout(searchTimer);
    searchTimer = null;
  }
  const startedGeneration = generation;
  const seq = ++searchSeq;
  searchQuery = query;
  setSearchStatus("Searching…");
  let reply: SearchReply;
  try {
    reply = (await request({ op: "search", query, maxHits: MAX_HITS_DEFAULT })) as SearchReply;
  } catch (err) {
    if (seq === searchSeq && startedGeneration === generation) setSearchStatus(`Search failed: ${messageOf(err)}`);
    return;
  }
  // A newer query, or a closed document, owns the UI now.
  if (seq !== searchSeq || startedGeneration !== generation || query !== searchQuery) return;
  searchHits = reply.hits;
  searchTruncated = reply.truncated;
  searchIndex = searchHits.length ? 0 : -1;
  if (searchIndex === 0) goTo(searchHits[0].page);
  repaint();
  updateSearchStatus();
}

function moveHit(delta: number) {
  if (!searchHits.length) return;
  searchIndex = (searchIndex + delta + searchHits.length) % searchHits.length;
  goTo(searchHits[searchIndex].page);
  repaint();
  updateSearchStatus();
}

function clearSearch() {
  searchSeq += 1; // any in-flight reply is for the old query
  searchHits = [];
  searchIndex = -1;
  searchQuery = "";
  searchTruncated = false;
  setSearchStatus("");
  repaint();
  syncControls(); // no match means no markup to add
}

function updateSearchStatus() {
  if (!searchQuery) return;
  if (!searchHits.length) {
    setSearchStatus("No matches");
    return;
  }
  const position = `Match ${searchIndex + 1} of ${searchHits.length}`;
  setSearchStatus(searchTruncated ? `${position} — showing first ${searchHits.length} matches` : position);
}

function setSearchStatus(text: string) {
  searchStatusEl.textContent = text;
}

// ----------------------------------------------------------------- thumbnails

/**
 * Fixed-row page list. Every page gets a sized placeholder (a canvas with no
 * bitmap yet), so the scroll height of a 500-page document is right before a
 * single pixel is rendered; only the rows in the viewport plus THUMB_BUFFER are
 * ever asked for, one request at a time, and never in front of a page render.
 */
function resetThumbs() {
  thumbsEl.replaceChildren();
  thumbsEl.hidden = true;
  thumbsToggle.disabled = true;
  thumbsToggle.setAttribute("aria-pressed", "false");
  thumbItems = [];
  thumbTried.clear();
  thumbPainted.clear();
  thumbCache.clear();
  thumbCurrent = -1;
  thumbTab = -1;
  thumbOpen = false;
  thumbRendered = 0;
  thumbMs = 0;
  thumbSmokeDone = false;
  // thumbBusy is left alone: an in-flight request settles on its own and drops
  // its reply by generation, and clearing the flag here could allow a second one.
}

function refreshThumbs() {
  resetThumbs();
  if (!doc) return;
  buildThumbs(doc.pages);
  setThumbCurrent(wantedPage);
  // Default open for a document with pages to page through, closed otherwise.
  // The user's own collapse survives a page op: only a new document resets it.
  setThumbSidebar(sidebarOpen && doc.pageCount >= 2 && !doc.info.needsPassword);
}

function buildThumbs(pages: PageSize[]) {
  const fragment = document.createDocumentFragment();
  for (let page = 0; page < pages.length; page++) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "thumb";
    button.dataset.page = String(page);
    button.style.height = `${THUMB_ROW_PX}px`;
    button.tabIndex = -1;
    button.setAttribute("aria-label", `Page ${page + 1}${labelSuffix(page)} of ${pages.length}`);
    // No backing store until the worker paints it: the placeholder is the CSS box only.
    const canvas = document.createElement("canvas");
    canvas.className = "thumb-canvas";
    canvas.width = 0;
    canvas.height = 0;
    canvas.setAttribute("aria-hidden", "true");
    const size = pages[page];
    if (size && size.width > 0 && size.height > 0) {
      const fit = THUMB_BOX_PX / Math.max(size.width, size.height);
      canvas.style.width = `${Math.max(1, Math.round(size.width * fit))}px`;
      canvas.style.height = `${Math.max(1, Math.round(size.height * fit))}px`;
    }
    const number = document.createElement("span");
    number.className = "thumb-number";
    number.textContent = String(page + 1);
    button.append(canvas, number);
    button.addEventListener("click", () => goTo(page));
    button.addEventListener("keydown", onThumbKey);
    fragment.append(button);
    thumbItems.push({ button, canvas });
  }
  thumbsEl.append(fragment);
}

/** Arrow keys move focus inside the list; Enter/click is what changes the page. */
function onThumbKey(event: KeyboardEvent) {
  const page = Number((event.currentTarget as HTMLElement).dataset.page);
  const direction = event.key === "ArrowDown" || event.key === "ArrowRight" ? 1
    : event.key === "ArrowUp" || event.key === "ArrowLeft" ? -1
      : 0;
  let target: number;
  if (direction) target = page + direction;
  else if (event.key === "Home") target = 0;
  else if (event.key === "End") target = thumbItems.length - 1;
  else return;
  event.preventDefault();
  target = Math.min(Math.max(0, target), thumbItems.length - 1);
  setThumbTab(target);
  thumbItems[target]?.button.focus();
}

function setThumbCurrent(page: number) {
  if (thumbCurrent !== page) {
    thumbItems[thumbCurrent]?.button.removeAttribute("aria-current");
    thumbCurrent = page;
    const button = thumbItems[page]?.button;
    if (button) {
      button.setAttribute("aria-current", "page");
      if (thumbOpen) button.scrollIntoView({ block: "nearest" });
    }
  }
  setThumbTab(page);
}

/** Roving tabindex: exactly one row is a tab stop. */
function setThumbTab(page: number) {
  if (thumbTab === page) return;
  if (thumbItems[thumbTab]) thumbItems[thumbTab].button.tabIndex = -1;
  thumbTab = page;
  if (thumbItems[page]) thumbItems[page].button.tabIndex = 0;
}

/**
 * The Pages collapse: the thumbnail list goes away and so does the column, so the
 * page gets the width back. The tab rail stays, which is how it is reopened.
 */
function setThumbSidebar(open: boolean) {
  const active = document.activeElement as HTMLElement | null;
  const dropsFocus = !open && !!active && sidebarEl.contains(active);
  sidebarOpen = open;
  thumbOpen = open && !!doc && !doc.info.needsPassword;
  sidebarEl.classList.toggle("collapsed", !open);
  thumbsEl.hidden = !thumbOpen;
  pagesEmptyEl.hidden = !!doc;
  // The button is named for the thumbnails it shows, so it reports those, not the column.
  thumbsToggle.setAttribute("aria-pressed", String(thumbOpen));
  if (dropsFocus) thumbsToggle.focus();
  if (!thumbOpen) return;
  setThumbCurrent(wantedPage);
  pumpThumbs();
}

/** The rows in the sidebar viewport, ± THUMB_BUFFER, as an index range. */
function thumbRange(): { start: number; end: number } {
  const tops: number[] = [];
  const heights: number[] = [];
  for (let page = 0; page < thumbItems.length; page++) {
    tops.push(page * THUMB_ROW_PX);
    heights.push(THUMB_ROW_PX);
  }
  return visibleRange(thumbsEl.scrollTop, thumbsEl.clientHeight, tops, heights, THUMB_BUFFER);
}

function thumbDone(page: number): boolean {
  return thumbPainted.has(page) || thumbTried.has(page);
}

/** A mutation only repaints the affected page's thumbnail; the rest stay valid. */
function invalidateThumb(page: number) {
  if (page < 0) return;
  thumbPainted.delete(page);
  thumbTried.delete(page);
}

/** True while the wanted page is not the painted one: a page render always wins. */
function pageRenderPending(): boolean {
  if (!doc || doc.info.needsPassword) return false;
  const goal = target();
  return !!goal && (goal.key !== shownKey || goal.zoom !== shownZoom);
}

function pumpThumbs() {
  if (!thumbOpen || !doc || doc.info.needsPassword) return;
  // `saving` is exactly when the worker gate is held; a retargeting scroll just
  // falls through to the new range on the next call.
  if (thumbBusy || inFlight !== null || saving || pageRenderPending()) return;
  const range = thumbRange();
  let next = -1;
  for (let page = range.start; page < range.end; page++) {
    if (!thumbDone(page)) {
      next = page;
      break;
    }
  }
  if (next < 0) {
    reportThumbs(range);
    return;
  }
  void renderThumb(next);
}

async function renderThumb(page: number) {
  const startedGeneration = generation;
  const key = `${page}:${THUMB_MAX_EDGE}:${revision}`;
  const started = performance.now();
  thumbBusy = true;
  try {
    const cached = thumbCache.get(key);
    const reply = cached ?? (await request({ op: "thumbnail", page, maxEdge: THUMB_MAX_EDGE }, [], THUMB_TIMEOUT_MS) as ThumbReply);
    if (startedGeneration !== generation) return; // a reply for a document that is no longer open
    thumbMs += Math.round(performance.now() - started);
    if (drawThumb(page, reply)) {
      if (!cached) thumbCache.set(key, reply);
      thumbRendered += 1;
    } else {
      // No canvas or no 2d context (the browser can refuse one): mark the page
      // tried, otherwise the cached reply would be found again and re-drawn in a
      // synchronous loop behind pumpThumbs.
      thumbTried.add(page);
    }
  } catch {
    // Decoration only: the placeholder stays, and a failure never reaches
    // showError, the status line or workerFailed.
    if (startedGeneration === generation) thumbTried.add(page);
  } finally {
    thumbBusy = false;
  }
  pumpThumbs();
}

/** Returns false when the row cannot be painted; the caller must then stop retrying it. */
function drawThumb(page: number, reply: ThumbReply): boolean {
  const canvas = thumbItems[page]?.canvas;
  const context = canvas?.getContext("2d");
  if (!canvas || !context) return false;
  canvas.width = reply.pixelWidth;
  canvas.height = reply.pixelHeight;
  context.putImageData(new ImageData(new Uint8ClampedArray(reply.pixels), reply.pixelWidth, reply.pixelHeight), 0, 0);
  thumbPainted.add(page);
  return true;
}

/** One line per open, once the first visible batch is settled. grok's smoke parses it. */
function reportThumbs(range: { start: number; end: number }) {
  if (thumbSmokeDone || range.end <= range.start) return;
  thumbSmokeDone = true;
  console.log(`P1_THUMBS ${JSON.stringify({
    visible: range.end - range.start,
    rendered: thumbRendered,
    thumbMs,
  })}`);
}

// ------------------------------------------------------ existing content (P3 U2)

/** One `edit-text` refusal token in words; the tokens are the contract's fixed set. */
const CONTENT_DETAIL_TEXT: Record<string, string> = {
  "stale-target": "This page changed since the list was scanned; refresh and try again.",
  "not-candidate": "That text cannot be edited yet.",
  "unsupported-encoding": "The replacement has characters this font cannot draw.",
  overflow: "The replacement exceeds the available free space.",
  "tail-would-move": "Following text would move past the available free-space limit.",
  "unmeasurable-width": "This run's width cannot be measured in this version.",
  "ambiguous-text-association": "That text cannot be told apart from another run on the page.",
  "too-large": "That text is too long.",
  "read-only": "This document does not permit changes to its content.",
  "out-of-bounds": "That box must stay inside the page.",
  "invalid-image": "That file is not a PNG or JPEG this app can read.",
  "stale-preview": "The page changed; preview again.",
  "write-failed": "The whole set was not written; nothing changed.",
  "invalid-selection": "That selection cannot be edited together.",
  "invalid-transform": "That rotation or flip is not one the editor offers.",
};

/** Why a discovered target is not editable, in the words the panel shows. */
const CONTENT_REASON_TEXT: Record<string, string> = {
  "subset-font": "Subset font: new characters cannot be drawn",
  "shared-or-unproven-sharing": "Shared or nested content (not yet editable)",
  "unsupported-graphics-state": "Hidden, clipped or unusual drawing state",
  "ambiguous-text-association": "Appears more than once; cannot be told apart",
  "unsupported-encoding": "Encoding not supported",
  "unsupported-text-operator": "Quote operators are not editable",
  type3: "Type 3 / vertical text is read-only",
  "vertical-writing": "Type 3 / vertical text is read-only",
  "rtl-or-shaping": "Right-to-left or shaped text is read-only",
  "unsupported-transform": "Skewed, nested or fully clipped placement (not yet editable)",
  "unsupported-image": "Image with a mask or layer (not yet replaceable)",
  "shared-placement": "Shared placement: edit it individually",
  "unresolved-resource": "Object type not supported",
  "unsupported-xobject": "Object type not supported",
};

type ParagraphItem = { paragraphId: string; targetIds: string[]; bounds: number[]; left: number; width: number; leading: number; fontSize: number; lines: number; kerned: boolean; tagged: boolean };
/** `transform` (P6 U3 §1.1) maps PDF user space to the y-down page view; `docGeneration` is the renderer's open counter at scan time. */
type ContentScan = { page: number; generation: number; revision: number; items: EditTarget[]; paragraphs: ParagraphItem[]; transform?: number[]; docGeneration?: number };
let contentScan: ContentScan | null = null;
let contentSeq = 0;
let contentIndex = -1;
let contentRefusals: { text: string; detail: string }[] = [];
let contentEdited: { from: string; to: string } | { kind: string; action: string; name: string; count?: number; skipped?: number } | null = null;
let contentLastShift = 0;
let contentLastAdd: { targetId: string; bounds: number[] | null } | null = null;
let contentAddPoint: [number, number] | null = null;
function lastAddedTargetId(): string | undefined { return contentLastAdd?.targetId; }
/** U5: the last preview, its token and which matches are ticked. */
type ReplaceMatch = { targetId: string; text: string; next: string; count: number; oldWidth?: number; newWidth?: number; tailFollows?: boolean };
type ReplaceRejected = { targetId: string; text: string; count: number; detail: string };
let replacePreview: {
  page: number; generation: number; revision: number; find: string; replace: string; token: string; targetId?: string;
  matches: ReplaceMatch[]; rejected: ReplaceRejected[];
} | null = null;
/** U5 drive state: `content.replaced` for a script's preview/apply line. */
let contentReplaced: { find: string; replace: string; applied: number; matches: number; rejected: { text: string; detail: string }[] } | null = null;
let contentSmokeActive = false;

/** The panel is only scanned while the Text tab is the visible one. */
function contentPanelVisible(): boolean {
  return inspectorOpen && !textPanelEl.hidden;
}

async function refreshContentTargets(force = false): Promise<void> {
  if (!force && !contentPanelVisible()) return;
  const seq = ++contentSeq;
  if (!doc || doc.info.needsPassword) {
    contentScan = null;
    renderContentTargets("No document open yet.");
    return;
  }
  const startedGeneration = generation;
  const page = wantedPage;
  let reply: Record<string, unknown>;
  try {
    reply = (await request({ op: "edit-targets", page })) as unknown as Record<string, unknown>;
  } catch (err) {
    if (seq === contentSeq) renderContentTargets(messageOf(err));
    return;
  }
  if (seq !== contentSeq || startedGeneration !== generation) return;
  if (reply.ok !== true) {
    contentScan = null;
    // A request-level refusal (read-only, too-large, inline image) is part of the
    // smoke's `refused` list, not a silent empty page.
    contentRefusals.push({ text: "", detail: typeof reply.reason === "string" ? reply.reason : "invalid" });
    renderContentTargets(typeof reply.error === "string" ? reply.error : "This page cannot be scanned.");
    return;
  }
  contentScan = {
    page,
    generation: reply.generation as number,
    revision: reply.revision as number,
    items: (reply.items ?? []) as EditTarget[],
    paragraphs: (reply.paragraphs ?? []) as ParagraphItem[],
    transform: Array.isArray(reply.transform) && reply.transform.length === 6 && reply.transform.every(value => typeof value === "number") ? reply.transform as number[] : undefined,
    docGeneration: startedGeneration,
  };
  renderContentTargets(null);
  paintHits(canvas.clientWidth, canvas.clientHeight, shownPage);
}

function renderContentTargets(problem: string | null): void {
  contentForm.hidden = true;
  contentObjectForm.hidden = true;
  contentTarget = null;
  contentObjectTarget = null;
  contentIndex = -1;
  clearReplacePreview();
  const items = contentScan?.items ?? [];
  contentTargetsEl.hidden = items.length === 0;
  contentEmptyEl.hidden = items.length > 0;
  contentEmptyEl.textContent = problem ?? "No content on this page.";
  const fragment = document.createDocumentFragment();
  for (const paragraph of contentScan?.paragraphs ?? []) {
    const first = items.find(item => item.targetId === paragraph.targetIds[0]); if (!first) continue;
    const row = document.createElement("li"); row.className = "content-paragraph-row";
    const button = document.createElement("button"); button.type = "button"; button.textContent = `Paragraph, ${paragraph.lines} lines, ${first.font?.name ?? "Text"} ${paragraph.fontSize}`;
    button.addEventListener("click", () => openInlineText(first)); button.addEventListener("keydown", event => event.stopPropagation()); row.append(button); fragment.append(row);
  }
  items.forEach((item, index) => {
    const row = document.createElement("li");
    row.id = `content-target-${index}`;
    row.setAttribute("role", "option");
    row.setAttribute("aria-selected", "false");
    row.tabIndex = -1;
    row.dataset.index = String(index);
    const shown = document.createElement("span");
    shown.className = "content-text";
    // An empty string is a deleted run; an object row names its kind, size and name.
    shown.textContent = item.kind !== "text" ? objectRowLabel(item)
      : item.text !== undefined && item.text !== "" ? item.text : item.kind;
    const meta = document.createElement("span");
    meta.className = "content-meta";
    const fontName = item.font ? item.font.name ?? item.font.subtype : null;
    // U2 edits text, U4 edits Image/Form placements: two forms, never open together.
    const state = item.kind !== "text" ? objectRowState(item)
      : item.support === "candidate" ? (item.cowable ? "Editable (this placement only)" : "Editable") : CONTENT_REASON_TEXT[item.reason ?? ""] ?? "Not editable";
    meta.textContent = fontName ? `${fontName} — ${state}` : state;
    row.append(shown, meta);
    fragment.append(row);
  });
  contentTargetsEl.replaceChildren(fragment);
  // Roving tabindex needs one reachable row: Tab enters the list at its first row.
  const firstOption = contentTargetsEl.querySelector<HTMLElement>('li[role="option"]');
  if (firstOption) firstOption.tabIndex = 0;
}

function contentItem(index: number): EditTarget | null {
  const items = contentScan?.items ?? [];
  return items[index] ?? null;
}

/** U4: `Image 64×64 pt` / `Form Fm1 120×40 pt`, from the placement box, else the pixels. */
function objectRowLabel(item: EditTarget): string {
  const box = item.placement?.box;
  const size = box
    ? `${Math.round((box[2] - box[0]) * 10) / 10}×${Math.round((box[3] - box[1]) * 10) / 10} pt`
    : item.image ? `${item.image.width}×${item.image.height} px` : "";
  return item.kind === "image" ? `Image ${size}`.trim() : `Form ${item.objectName ?? ""} ${size}`.trim();
}

/** U4: what the row says an Image/Form placement can do. */
function objectRowState(item: EditTarget): string {
  if (item.support !== "candidate") return CONTENT_REASON_TEXT[item.reason ?? ""] ?? "Not editable";
  if (!(item.objectOps ?? []).includes("move")) return "Editable (replace only)";
  return item.reason ? CONTENT_REASON_TEXT[item.reason] ?? "Editable" : "Editable";
}

/** Roving tabindex over one listbox: arrows, Home, End (WAI-ARIA automatic activation). */
function selectContentRow(index: number, focus = true): void {
  const rows = [...contentTargetsEl.querySelectorAll<HTMLElement>('li[role="option"]')];
  if (!rows.length) return;
  const bounded = Math.min(Math.max(0, index), rows.length - 1);
  contentIndex = bounded;
  const runItem = contentItem(bounded);
  contentScopeRun.disabled = !(runItem && runItem.kind === "text" && runItem.support === "candidate");
  if (contentScopeRun.disabled && contentScopeRun.checked) contentScopePage.checked = true;
  rows.forEach((row, i) => {
    row.setAttribute("aria-selected", String(i === bounded));
    row.tabIndex = i === bounded ? 0 : -1;
  });
  if (focus) rows[bounded].focus();
}

function openContentForm(item: EditTarget): void {
  contentObjectForm.hidden = true;
  contentObjectTarget = null;
  contentTarget = item;
  contentEditLabel.textContent = `Replace ${JSON.stringify(item.text ?? "")}`
    + (item.font?.name ? ` (${item.font.name})` : "");
  contentNewText.value = item.text ?? "";
  // A deleted run keeps an empty operator; the form shows nothing to type into.
  contentHint.textContent = item.tailFollows
    ? "Text follows on this line: the new text must be the same width."
    : "Shorter text leaves a gap; longer text is refused.";
  // U3: a CID run can only be written with characters its own font already draws here.
  contentRepertoire.hidden = !item.repertoire;
  contentRepertoire.textContent = item.repertoire
    ? `Only characters already in this font on this page can be used: ${item.repertoire}`
    : "";
  contentForm.hidden = false;
  contentNewText.focus();
  contentNewText.select();
}

function closeContentForm(focusList = true): void {
  contentForm.hidden = true;
  contentTarget = null;
  const rows = [...contentTargetsEl.querySelectorAll<HTMLElement>('li[role="option"]')];
  if (!rows.length) return;
  if (focusList && contentIndex >= 0) selectContentRow(contentIndex);
  else (rows[Math.max(0, contentIndex)] ?? rows[0]).focus();
}

/** Selecting a row: candidates open the form, everything else says why not. */
function activateContentRow(index: number): void {
  selectContentRow(index);
  const item = contentItem(index);
  if (!item) return;
  if (item.kind !== "text") {
    if (item.support === "candidate") openContentObjectForm(item);
    else announce(CONTENT_REASON_TEXT[item.reason ?? ""] ?? "That object cannot be edited yet.");
    return;
  }
  if (item.support !== "candidate") {
    announce(CONTENT_REASON_TEXT[item.reason ?? ""] ?? "That content cannot be edited yet.");
    return;
  }
  openContentForm(item);
}

/**
 * One `edit-text` request. Returns false and records the refusal token on any
 * refusal, so the panel and the smoke share one path.
 */
async function applyContentEdit(scan: ContentScan, item: EditTarget, text: string): Promise<boolean> {
  let reply: Record<string, unknown>;
  try {
    reply = (await request({
      op: "edit-text", page: scan.page, targetId: item.targetId,
      generation: scan.generation, revision: scan.revision, text,
    })) as unknown as Record<string, unknown>;
  } catch (err) {
    // Every refusal arrives here (the worker helper rejects any ok:false reply).
    const { reply: refused, reason } = refusalOf(err);
    const detail = typeof refused?.detail === "string" ? refused.detail
      : reason === "read-only" ? "read-only" : "not-candidate";
    contentRefusals.push({ text: item.text ?? "", detail });
    // U3: a repertoire refusal names the characters this font cannot draw.
    const missing = Array.isArray(refused?.missing) ? refused.missing.filter(char => typeof char === "string") : [];
    announce(detail === "unsupported-encoding" && missing.length
      ? `Cannot draw: ${missing.join(" ")} (not in this font)`
      : CONTENT_DETAIL_TEXT[detail] ?? messageOf(err));
    return false;
  }
  if (reply.ok !== true) {
    contentRefusals.push({ text: item.text ?? "", detail: "not-candidate" });
    announce(typeof reply.error === "string" ? reply.error : "That edit was refused.");
    return false;
  }
  const from = item.text ?? "";
  contentEdited = { from, to: text };
  contentLastShift = typeof reply.shift === "number" ? reply.shift : 0;
  const notes = [contentLastShift ? `shifted following text by ${contentLastShift} pt` : "", item.kerned ? "kerning in this run was dropped" : ""].filter(Boolean);
  if (notes.length) contentHint.textContent = notes.join("; ");
  await applyEdit(reply as unknown as JournalState, text === ""
    ? `Deleted ${JSON.stringify(from)}`
    : `Replaced ${JSON.stringify(from)} with ${JSON.stringify(text)}`);
  return true;
}

async function driveTextAdd(contents: string | null): Promise<void> {
  if (!contents || !doc) return;
  try {
    const value = JSON.parse(contents) as { page?: number; point?: number[]; text?: string; font?: string; size?: number; color?: number[] };
    const reply = await request({ op: "text-add", ...value }) as MutationReply & { targetId?: string; bounds?: number[] | null };
    if (reply.ok !== true || typeof reply.targetId !== "string") throw new Error("Could not add text.");
    contentLastAdd = { targetId: reply.targetId, bounds: reply.bounds ?? null };
    await applyEdit(reply, "Added text");
    await refreshContentTargets(true);
  } catch (err) { showError(messageOf(err)); }
}

async function submitContentAdd(): Promise<void> {
  if (!doc || !canEdit()) return;
  const keyboardBox = shapeBox();
  const point = contentAddPoint ?? (keyboardBox ? [keyboardBox[0], keyboardBox[1]] : [72, 72]);
  if (!await addTextAt(wantedPage, point, contentAddText.value)) return;
  const index = (contentScan?.items ?? []).findIndex(item => item.targetId === contentLastAdd?.targetId);
  if (index >= 0) selectContentRow(index);
  contentAddText.value = "";
  contentAddPoint = null;
}

async function submitContentEdit(text: string): Promise<void> {
  const scan = contentScan;
  const item = contentTarget;
  if (!scan || !item) return;
  const ok = await applyContentEdit(scan, item, text);
  if (ok) closeContentForm(false);
}

/** U4: the first image the picker returns, or null with a status line. */
async function pickOneImage(): Promise<{ name: string; bytes: ArrayBuffer } | null> {
  if (!api || typeof api.pickImages !== "function") {
    announce("Choosing an image needs the desktop app.");
    return null;
  }
  let picked;
  try {
    picked = await api.pickImages();
  } catch (err) {
    announce(messageOf(err));
    return null;
  }
  if (!picked.ok) {
    if (picked.reason !== "cancelled") {
      announce(picked.reason === "too-large" ? "That image is larger than this app will read." : "That image could not be read.");
    }
    return null;
  }
  const first = picked.images[0];
  return first ? { name: first.name, bytes: first.bytes } : null;
}

/** U4: one placement form, filled from the item's own box. */
function openContentObjectForm(item: EditTarget, focus = true): void {
  contentForm.hidden = true;
  contentTarget = null;
  contentObjectTarget = item;
  const ops = item.objectOps ?? [];
  const box = item.placement?.box ?? null;
  const canMove = box !== null && ops.includes("move");
  contentObjectLabel.textContent = objectRowLabel(item);
  contentBoxX.disabled = contentBoxY.disabled = contentBoxW.disabled = contentBoxH.disabled = !canMove;
  // Debt sweep 2 §9.8: shown to 2 decimals; an unchanged field passes the unrounded value through on Apply.
  contentBoxX.value = String(box ? round2(box[0]) : 0);
  contentBoxY.value = String(box ? round2(box[1]) : 0);
  contentBoxW.value = String(box ? round2(box[2] - box[0]) : item.image?.width ?? 0);
  contentBoxH.value = String(box ? round2(box[3] - box[1]) : item.image?.height ?? 0);
  contentReplaceButton.hidden = !ops.includes("replace");
  contentDeleteObjectButton.hidden = !ops.includes("delete");
  contentObjectHint.textContent = ops.includes("move")
    ? "Moves this placement only. Delete removes the drawing, not the image data (not secure redaction)."
    : "This placement is skewed or nested: the new image is stretched into it.";
  syncInspectorTransform();
  contentObjectForm.hidden = false;
  if (focus) (canMove ? contentBoxX : contentReplaceButton.hidden ? contentDeleteObjectButton : contentReplaceButton).focus();
}

function closeContentObjectForm(focusList = true): void {
  contentObjectForm.hidden = true;
  contentObjectTarget = null;
  const rows = [...contentTargetsEl.querySelectorAll<HTMLElement>('li[role="option"]')];
  if (!rows.length) return;
  if (focusList && contentIndex >= 0) selectContentRow(contentIndex);
  else (rows[Math.max(0, contentIndex)] ?? rows[0]).focus();
}

/** One `edit-object` request: the whole refusal travels back, and nothing lands on a refusal. */
async function applyObjectEdit(scan: ContentScan, item: EditTarget, action: string, extra: Record<string, unknown>): Promise<boolean> {
  let reply: Record<string, unknown>;
  try {
    reply = (await request({
      op: "edit-object", page: scan.page, targetId: item.targetId,
      generation: scan.generation, revision: scan.revision, action, ...extra,
    })) as unknown as Record<string, unknown>;
  } catch (err) {
    const { reply: refused, reason } = refusalOf(err);
    const detail = typeof refused?.detail === "string" ? refused.detail
      : reason === "read-only" ? "read-only" : "not-candidate";
    contentRefusals.push({ text: item.objectName ?? "", detail });
    announce(CONTENT_DETAIL_TEXT[detail] ?? messageOf(err)); showError(CONTENT_DETAIL_TEXT[detail] ?? messageOf(err));
    return false;
  }
  if (reply.ok !== true) {
    contentRefusals.push({ text: item.objectName ?? "", detail: "not-candidate" });
    announce(typeof reply.error === "string" ? reply.error : "That edit was refused."); showError(typeof reply.error === "string" ? reply.error : "That edit was refused.");
    return false;
  }
  // P6 U3 §1.2: the same box again is a no-op reply: no journal op, nothing to mirror.
  if (reply.noop === true) return true;
  contentEdited = { kind: item.kind, action, name: item.objectName ?? "" };
  const box = Array.isArray(extra.box) && extra.box.every(value => typeof value === "number") ? extra.box as number[] : null;
  await applyEdit(reply as unknown as JournalState, objectAnnouncement(item, action, box, typeof extra.transform === "string" ? extra.transform : undefined));
  return true;
}

/** P6 U5 §2.2: one `edit-objects` (2..n members, all or nothing); `content.edited` carries `count`. */
async function applyObjectsEdit(scan: ContentScan, entries: { item: EditTarget; box?: number[] }[], action: "move" | "delete" | "duplicate" | "resize" | "transform", transform?: string): Promise<boolean> {
  const names = entries.map(entry => entry.item.objectName ?? "").join(",");
  let reply: Record<string, unknown>;
  try {
    reply = (await request({
      op: "edit-objects", page: scan.page, generation: scan.generation, revision: scan.revision,
      items: entries.map(entry => ({ targetId: entry.item.targetId, action, ...(transform ? { transform } : {}), ...(entry.box ? { box: entry.box } : {}) })),
    })) as unknown as Record<string, unknown>;
  } catch (err) {
    const { reply: refused, reason } = refusalOf(err);
    const detail = typeof refused?.detail === "string" ? refused.detail
      : reason === "read-only" ? "read-only" : "not-candidate";
    contentRefusals.push({ text: names, detail });
    announce(CONTENT_DETAIL_TEXT[detail] ?? messageOf(err)); showError(CONTENT_DETAIL_TEXT[detail] ?? messageOf(err));
    return false;
  }
  if (reply.ok !== true) {
    contentRefusals.push({ text: names, detail: "not-candidate" });
    announce(typeof reply.error === "string" ? reply.error : "That edit was refused."); showError(typeof reply.error === "string" ? reply.error : "That edit was refused.");
    return false;
  }
  if (reply.noop === true) return true;
  const count = typeof reply.count === "number" ? reply.count : entries.length;
  contentEdited = { kind: "objects", action, name: names, count };
  const verb = action === "move" ? "Moved" : action === "delete" ? "Deleted" : action === "resize" ? "Resized" : action === "transform" ? (transform?.startsWith("rotate") ? "Rotated" : "Flipped") : "Duplicated";
  await applyEdit(reply as unknown as JournalState, `${verb} ${count} objects`);
  return true;
}

/** The one-line undo label and announcement for one placement action. */
function objectAnnouncement(item: EditTarget, action: string, box: number[] | null, transform?: string): string {
  const kind = item.kind === "image" ? "image" : `Form ${item.objectName ?? ""}`.trim();
  const round = (value: number) => Math.round(value * 10) / 10;
  if (action === "transform" && transform) return `${transform.startsWith("rotate") ? "Rotated" : "Flipped"} ${kind}`;
  if (action === "crop") return `Cropped ${kind}`;
  if (action === "delete") return `Deleted ${kind}`;
  if (action === "duplicate") return `Duplicated ${kind}`;
  if (action === "replace") return `Replaced ${kind} with a new image`;
  if (!box) return action === "move" ? `Moved ${kind}` : `Resized ${kind}`;
  if (action === "move") return `Moved ${kind} to ${round(box[0])}, ${round(box[1])}`;
  return `Resized ${kind} to ${round(box[2] - box[0])}×${round(box[3] - box[1])}`;
}

/** `box` reads the four fields; `delete` and `replace` need no box. */
async function submitObjectEdit(what: "box" | "delete" | "replace"): Promise<void> {
  const scan = contentScan;
  const item = contentObjectTarget;
  if (!scan || !item) return;
  let ok = false;
  if (what === "delete") {
    ok = await applyObjectEdit(scan, item, "delete", {});
  } else if (what === "replace") {
    const picked = await pickOneImage();
    if (!picked) return;
    ok = await applyObjectEdit(scan, item, "replace", { image: picked });
  } else {
    const old = item.placement?.box;
    // §9.8: a field still showing round2 of the placement's own value passes the unrounded value through.
    const exact = old ? [old[0], old[1], old[2] - old[0], old[3] - old[1]] : [];
    const same = [contentBoxX, contentBoxY, contentBoxW, contentBoxH].map((input, k) => !!old && input.value === String(round2(exact[k])));
    const [x, y, width, height] = [contentBoxX, contentBoxY, contentBoxW, contentBoxH].map((input, k) => same[k] ? exact[k] : Number(input.value));
    if (![x, y, width, height].every(Number.isFinite)) {
      announce("That box is not four numbers.");
      return;
    }
    // An unchanged edge is the placement's own edge, so an unchanged Apply sends placement.box exactly (a no-op).
    const box = [x, y, same[0] && same[2] && old ? old[2] : x + width, same[1] && same[3] && old ? old[3] : y + height];
    const action = old && Math.abs(width - (old[2] - old[0])) < 0.05 && Math.abs(height - (old[3] - old[1])) < 0.05 ? "move" : "resize";
    ok = await applyObjectEdit(scan, item, action, { box });
  }
  if (ok) closeContentObjectForm(false);
}

/**
 * U5: one preview request for the current scan. `#content-scope-run` limits it to
 * the selected text candidate. Nothing is written and no operation is opened.
 */
async function previewReplace(): Promise<void> {
  const scan = contentScan;
  if (!scan) {
    announce("Open a page first.");
    return;
  }
  const find = contentFindInput.value;
  const replace = contentReplaceWith.value;
  if (!find) {
    announce("Type the text to find.");
    return;
  }
  const targetId = contentScopeRun.checked && contentTarget?.kind === "text" ? contentTarget.targetId : undefined;
  try {
    const reply = (await request({
      op: "edit-replace-preview", page: scan.page, generation: scan.generation, revision: scan.revision,
      find, replace, ...(targetId ? { targetId } : {}),
    })) as unknown as Record<string, unknown>;
    replacePreview = {
      page: scan.page, generation: scan.generation, revision: scan.revision, find, replace,
      token: String(reply.previewToken ?? ""),
      targetId,
      matches: (reply.matches ?? []) as ReplaceMatch[],
      rejected: (reply.rejected ?? []) as ReplaceRejected[],
    };
    renderReplacePreview();
    announce(`${replacePreview.matches.length} can change, ${replacePreview.rejected.length} cannot.`);
  } catch (err) {
    clearReplacePreview();
    announce(messageOf(err));
  }
}

function clearReplacePreview(focusList = false): void {
  replacePreview = null;
  contentMatchesEl.replaceChildren();
  contentReplaceSummary.textContent = "";
  contentReplaceApply.disabled = true;
  if (focusList) contentReplacePanel.open = false;
}

/** One list: ticked matches first, then the rejected runs with their reason. */
function renderReplacePreview(): void {
  const preview = replacePreview;
  contentMatchesEl.replaceChildren();
  if (!preview) {
    contentReplaceSummary.textContent = "";
    contentReplaceApply.disabled = true;
    return;
  }
  const fragment = document.createDocumentFragment();
  preview.matches.forEach((match, index) => {
    const row = document.createElement("li");
    const label = document.createElement("label");
    const box = document.createElement("input");
    box.type = "checkbox";
    box.id = `content-match-${index}`;
    box.checked = true;
    box.addEventListener("change", () => syncReplaceApply());
    const shown = document.createElement("span");
    shown.textContent = ` ${match.text} → ${match.next}${match.count > 1 ? ` (${match.count})` : ""}`;
    label.append(box, shown);
    row.append(label);
    fragment.append(row);
  });
  preview.rejected.forEach(rejected => {
    const row = document.createElement("li");
    row.className = "content-meta";
    row.textContent = `${rejected.text} (${CONTENT_DETAIL_TEXT[rejected.detail] ?? CONTENT_REASON_TEXT[rejected.detail] ?? rejected.detail})`;
    fragment.append(row);
  });
  contentMatchesEl.replaceChildren(fragment);
  contentReplaceSummary.textContent = `${preview.matches.length} runs can change, ${preview.rejected.length} cannot`;
  syncReplaceApply();
}

function checkedReplaceIds(): string[] {
  if (!replacePreview) return [];
  return replacePreview.matches
    .filter((_, index) => (document.getElementById(`content-match-${index}`) as HTMLInputElement | null)?.checked)
    .map(match => match.targetId);
}

function syncReplaceApply(): void {
  contentReplaceApply.disabled = checkedReplaceIds().length === 0;
}

/** U5: one `edit-replace-apply`; all ticked runs or none. */
async function applyReplace(): Promise<void> {
  const preview = replacePreview;
  if (!preview) return;
  const targetIds = checkedReplaceIds();
  if (targetIds.length === 0) return;
  let reply: Record<string, unknown>;
  try {
    reply = (await request({
      op: "edit-replace-apply", page: preview.page, generation: preview.generation, revision: preview.revision,
      find: preview.find, replace: preview.replace, previewToken: preview.token, targetIds,
      ...(preview.targetId ? { targetId: preview.targetId } : {}),
    })) as unknown as Record<string, unknown>;
  } catch (err) {
    const { reply: refused } = refusalOf(err);
    const detail = typeof refused?.detail === "string" ? refused.detail : "invalid";
    announce(CONTENT_DETAIL_TEXT[detail] ?? messageOf(err));
    return;
  }
  const replaced = Number(reply.replaced ?? targetIds.length);
  await applyEdit(reply as unknown as JournalState,
    `Replaced ${preview.find} in ${replaced} run${replaced === 1 ? "" : "s"}`);
}

/** The U5 drive path: preview the whole page (or one run), then apply every match. */
async function smokeReplace(wanted: { find?: string; replace?: string; preview?: boolean; page?: number; target?: string }): Promise<void> {
  if (typeof wanted.page === "number") goTo(wanted.page);
  await refreshContentTargets(true);
  const scan = contentScan;
  if (!scan) return;
  const find = wanted.find ?? "";
  const replace = wanted.replace ?? "";
  const targetItem = wanted.target
    ? (contentScan?.items ?? []).find(item => item.kind === "text" && item.text === wanted.target) : undefined;
  let preview: Record<string, unknown>;
  try {
    preview = (await request({
      op: "edit-replace-preview", page: scan.page, generation: scan.generation, revision: scan.revision,
      find, replace, ...(targetItem ? { targetId: targetItem.targetId } : {}),
    })) as unknown as Record<string, unknown>;
  } catch (err) {
    contentReplaced = { find, replace, applied: 0, matches: 0, rejected: [{ text: "", detail: "invalid" }] };
    announce(messageOf(err));
    return;
  }
  const matches = (preview.matches ?? []) as ReplaceMatch[];
  const rejected = ((preview.rejected ?? []) as ReplaceRejected[]).map(item => ({ text: item.text, detail: item.detail }));
  if (wanted.preview === true || matches.length === 0) {
    contentReplaced = { find, replace, applied: 0, matches: matches.length, rejected };
    return;
  }
  try {
    const applied = (await request({
      op: "edit-replace-apply", page: scan.page, generation: scan.generation, revision: scan.revision,
      find, replace, previewToken: preview.previewToken, targetIds: matches.map(match => match.targetId),
      ...(targetItem ? { targetId: targetItem.targetId } : {}),
    })) as unknown as Record<string, unknown>;
    const count = Number(applied.replaced ?? matches.length);
    contentReplaced = { find, replace, applied: count, matches: matches.length, rejected };
    await applyEdit(applied as unknown as JournalState, `Replaced ${find} in ${count} runs`);
  } catch (err) {
    const { reply: refused } = refusalOf(err);
    contentReplaced = {
      find, replace, applied: 0, matches: matches.length,
      rejected: [...rejected, { text: "", detail: typeof refused?.detail === "string" ? refused.detail : "invalid" }],
    };
    announce(messageOf(err));
  }
}

function contentSummary() {
  const items = contentScan?.items ?? [];
  return {
    targets: items.length,
    paragraphs: contentScan?.paragraphs.length ?? 0,
    bounds: items.filter(item => item.kind === "text" && item.bounds).map(item => ({ targetId: item.targetId, bounds: item.bounds, fontSize: item.fontSize, paragraphId: item.paragraphId })),
    objects: items.filter(item => item.kind !== "text").map(item => {
      const box = item.placement?.box ?? null, m = contentScan?.transform, placement = placementOf(item);
      // P6 U6 §2.6: `orient` verbatim (null without a placement); `full` (view space) only when cropped.
      return { targetId: item.targetId, kind: item.kind, name: item.objectName ?? "", box: box ? box.map(value => Math.round(value * 100) / 100) : null, view: box && m ? viewBox(box, m).map(value => Math.round(value * 100) / 100) : null, ops: item.objectOps ?? [], orient: placement?.orient ?? null, ...(placement?.full && m ? { full: viewBox(placement.full, m).map(value => Math.round(value * 100) / 100) } : {}) };
    }),
    candidates: items.filter(item => item.support === "candidate").length,
    /** print-edit D7: print-to-PDF glyph lines on the shown page. */
    glyphLines: items.filter(item => item.glyphs !== undefined).length,
    edited: contentEdited,
    last: { shift: contentLastShift },
    lastAdd: contentLastAdd,
    replaced: contentReplaced,
    refused: contentRefusals,
  };
}

function logContentSmoke(): void {
  console.log(`P3_EDIT ${JSON.stringify({ content: contentSummary() })}`);
}

/** `edit-request {op:'content-edit'}` from the menu: land in Annotate, show the list. */
async function openContentEditor(): Promise<void> {
  selectTabById(inspectorTabs, inspectorPanels, "tab-text");
  setInspectorOpen(true);
  await refreshContentTargets(true);
  const first = (contentScan?.items ?? []).findIndex(item => item.kind === "text" && item.support === "candidate");
  selectContentRow(first >= 0 ? first : 0);
}

/** `--edit=content`: the smoke's fixed sequence, logged as one `P3_EDIT` line. */
async function smokeContent(contents: string | null = null): Promise<void> {
  contentSmokeActive = true;
  contentRefusals = [];
  contentEdited = null;
  contentReplaced = null;
  // A locked fixture is unlocked first with the payload's password (the smoke's only password path).
  let wanted: { target?: string; targetId?: string; text?: string; action?: string; box?: number[]; image?: string; find?: string; replace?: string; preview?: boolean; page?: number } = {}; 
  // §4.3: a plain string (`"hello"`) is not a step either; only null (the --edit=content smoke) reaches the self-test.
  if (contents !== null && !contents.trim().startsWith("{")) {
    showError("content: give text, find or action");
    logContentSmoke();
    return;
  }
  if (contents && contents.trim().startsWith("{")) {
    try {
      const parsed = JSON.parse(contents) as { password?: string; target?: string; targetId?: string; text?: string; action?: string; box?: number[]; image?: string; find?: string; replace?: string; preview?: boolean; page?: number };
      // A well-formed content step starts clean (like find), so a stale refusal never outlives it.
      clearError();
      if (parsed.password) await smokePassword(parsed.password);
      wanted = parsed;
      // An object with none of the keys never runs the self-test below (that stays the null/password path).
      const has = (key: "text" | "find" | "action" | "password") => Object.prototype.hasOwnProperty.call(parsed, key) && typeof parsed[key] === "string";
      if (!has("text") && !has("find") && !has("action") && !has("password")) {
        showError("content: give text, find or action");
        logContentSmoke();
        return;
      }
    } catch (err) {
      showError(`content: ${messageOf(err)}`);
      logContentSmoke();
      return;
    }
  }
  if (typeof wanted.find === "string") {
    await smokeReplace(wanted);
    logContentSmoke();
    return;
  }
  await refreshContentTargets(true);
  const items = contentScan?.items ?? [];
  if (typeof wanted.action === "string") {
    const item = items.find(entry => entry.kind !== "text" && (wanted.target === undefined
      ? entry.support === "candidate" && (entry.objectOps?.length ?? 0) > 0
      : entry.objectName === wanted.target));
    if (item && contentScan) await smokeObjectAction(contentScan, item, wanted);
    else announce(`No object target ${JSON.stringify(wanted.target ?? "")} on this page`);
    logContentSmoke();
    return;
  }
  if (typeof wanted.text === "string") {
    const targetId = wanted.targetId === "$lastAdd" ? lastAddedTargetId() : wanted.targetId;
    const item = items.find(entry => entry.kind === "text" && (targetId ? entry.targetId === targetId : wanted.target === undefined ? entry.support === "candidate" : entry.text === wanted.target));
    if (item && contentScan) await applyContentEdit(contentScan, item, wanted.text);
    else announce(`No text target ${JSON.stringify(wanted.target ?? "")} on this page`);
    logContentSmoke();
    return;
  }
  const first = items.find(item => item.support === "candidate" && item.text === "ALPHA")
    ?? items.find(item => item.kind === "text" && item.support === "candidate");
  if (first && contentScan) {
    await applyContentEdit(contentScan, first, "BETA");
  }
  // Re-scan after every write: handles are revision-bound and growth changes the
  // target ordering. Exercise available-space growth, then an actual overflow.
  await refreshContentTargets(true);
  const hasTail = (contentScan?.items ?? []).some(item => item.kind === "text" && item.text === " TAIL");
  let next = hasTail
    ? (contentScan?.items ?? []).find(item => item.kind === "text" && item.support === "candidate" && item.text === " TAIL")
    : (contentScan?.items ?? []).find(item => item.kind === "text" && item.support === "candidate" && item.text === "BETA");
  if (!hasTail && next && contentScan) {
    await applyContentEdit(contentScan, next, "BRAVO");
    await refreshContentTargets(true);
    next = (contentScan?.items ?? []).find(item => item.kind === "text" && item.support === "candidate" && item.text === "BRAVO")
      ?? (contentScan?.items ?? []).find(item => item.kind === "text" && item.support === "candidate" && item.text === "BETA");
  }
  if (next && contentScan) await applyContentEdit(contentScan, next, "BRAVO".repeat(120));
  logContentSmoke();
}

/** U4 drive path: one placement action from a script line (`box` or the picker's image). */
async function smokeObjectAction(scan: ContentScan, item: EditTarget, wanted: { action?: string; box?: number[] }): Promise<void> {
  const action = wanted.action ?? "";
  if (action === "delete") {
    await applyObjectEdit(scan, item, "delete", {});
    return;
  }
  if (action === "replace") {
    const picked = await pickOneImage();
    if (!picked) return;
    await applyObjectEdit(scan, item, "replace", { image: picked });
    return;
  }
  if (action !== "move" && action !== "resize") {
    announce(`Unknown object action ${JSON.stringify(action)}`);
    return;
  }
  const box = wanted.box;
  if (!Array.isArray(box) || box.length !== 4 || !box.every(value => typeof value === "number" && Number.isFinite(value))) {
    announce("An object action needs a box of four numbers.");
    return;
  }
  await applyObjectEdit(scan, item, action, { box });
}

// ----------------------------------------------------------------- redaction

function renderRedactState(): void {
  const marks = redactState?.marks ?? 0;
  const pages = redactState?.pages.length ?? 0;
  redactCountEl.textContent = `${marks} mark${marks === 1 ? "" : "s"} on ${pages} page${pages === 1 ? "" : "s"}`;
  redactApplyOpen.disabled = !canEdit() || marks === 0;
}

async function refreshRedactPreview(): Promise<RedactPreview | null> {
  if (!doc || doc.info.needsPassword) {
    redactState = null;
    renderRedactState();
    return null;
  }
  try {
    redactState = (await request({ op: "redact-preview" })) as unknown as RedactPreview;
  } catch (err) {
    if (uiMode === "redact") showError(messageOf(err));
    redactState = null;
  }
  renderRedactState();
  return redactState;
}

async function addRedactArea(page: number, rect: number[]): Promise<boolean> {
  if (!doc || !canAnnotate()) return false;
  try {
    const reply = (await request({ op: "redact-add", page, rect })) as MutationReply;
    redactLast = { op: "redact", ok: true };
    await applyEdit(reply, "Marked area for redaction — content is not removed until Apply Redactions");
    return true;
  } catch (err) {
    const reason = refusalOf(err).reason ?? "invalid";
    redactLast = { op: "redact", ok: false, reason };
    showError(messageOf(err));
    return false;
  }
}

async function addRedactSearch(text: string): Promise<boolean> {
  if (!doc || !canAnnotate()) return false;
  try {
    const reply = (await request({ op: "redact-search", text })) as MutationReply;
    redactLast = { op: "redact", ok: true };
    await applyEdit(reply, "Marked every match for redaction — content is not removed until Apply Redactions");
    return true;
  } catch (err) {
    const reason = refusalOf(err).reason ?? "invalid";
    redactLast = { op: "redact", ok: false, reason };
    showError(messageOf(err));
    return false;
  }
}

function sanitizeSelection(): RedactSanitize {
  return Object.fromEntries(REDACT_KEYS.map(key => [key, redactSanitizeBoxes[key].checked])) as RedactSanitize;
}

function updateRedactKept(): void {
  if (!redactState) { redactKeptEl.textContent = ""; return; }
  const kept = REDACT_KEYS.filter(key => !redactSanitizeBoxes[key].checked && (key === "xmp" ? redactState!.sanitize.xmp : redactState!.sanitize[key] > 0));
  redactKeptEl.textContent = kept.length ? `Kept by choice: ${kept.join(", ")}.` : "";
}

function renderRedactDialog(preview: RedactPreview): void {
  const body = redactPreviewEl.tBodies[0];
  body.replaceChildren(...preview.pageCounts.map(item => {
    const row = document.createElement("tr");
    const page = document.createElement("td");
    const count = document.createElement("td");
    page.textContent = String(item.page + 1);
    count.textContent = String(item.marks);
    row.append(page, count);
    return row;
  }));
  redactSharedEl.replaceChildren(...preview.shared.map(item => {
    const row = document.createElement("li");
    const same = item.otherPages.every(page => page === item.page);
    row.textContent = same
      ? `Page ${item.page + 1}: this shared ${item.kind} appears again on this page without a mark.`
      : `Page ${item.page + 1}: this shared ${item.kind} also appears on page(s) ${item.otherPages.map(page => page + 1).join(", ")} without a mark. Mark it there too.`;
    return row;
  }));
  redactInlineAltEl.textContent = preview.inlineAlt
    ? `${preview.inlineAlt} alternate-text entr${preview.inlineAlt === 1 ? "y is" : "ies are"} present on marked pages; alternate text on this page is not removed by a rect mark.`
    : "";
  redactApplyButton.disabled = preview.shared.length > 0 || preview.marks === 0;
  updateRedactKept();
}

async function openRedactDialog(): Promise<void> {
  const preview = await refreshRedactPreview();
  if (!preview) return;
  for (const key of REDACT_KEYS) redactSanitizeBoxes[key].checked = true;
  renderRedactDialog(preview);
  redactDialog.showModal();
  redactApplyButton.focus();
}

async function applyRedact(sanitize = sanitizeSelection()): Promise<boolean> {
  if (!doc || !canEdit()) return false;
  try {
    const reply = (await request({ op: "redact-apply", sanitize }, [], SAVE_TIMEOUT_MS)) as unknown as {
      bytes: ArrayBuffer; marks: number; pages: number[]; sanitized: RedactInventory; kept: RedactInventory;
      verified: { text: number; pixels: number; raw: number };
    };
    const name = await saveNewFile(reply.bytes, `${fileStem(doc.name)} redacted.pdf`, "pdf");
    if (!name) return false;
    redactLast = { op: "redact-apply", ok: true, verified: reply.verified };
    setStatus(`Saved ${name}; the open document is unchanged and still contains its marks.`);
    redactDialog.close();
    return true;
  } catch (err) {
    const reason = refusalOf(err).reason ?? "invalid";
    redactLast = { op: "redact-apply", ok: false, reason };
    showError(reason === "shared-content" ? "Also appears without a mark. Mark every placement too." : messageOf(err));
    return false;
  }
}

async function driveRedact(contents: string | null): Promise<void> {
  clearError();
  const wanted = driveWanted(contents);
  if (!wanted) return;
  if (Array.isArray(wanted.rect)) {
    const page = typeof wanted.page === "number" ? wanted.page : wantedPage;
    await addRedactArea(page, wanted.rect as number[]);
  } else if (typeof wanted.search === "string") await addRedactSearch(wanted.search);
  else {
    redactLast = { op: "redact", ok: false, reason: "invalid" };
    showError("redact: send rect and page, or search text");
  }
}

async function driveRedactPreview(): Promise<void> {
  clearError();
  try {
    await refreshRedactPreview();
    redactLast = { op: "redact-preview", ok: true };
  } catch (err) {
    redactLast = { op: "redact-preview", ok: false, reason: refusalOf(err).reason ?? "invalid" };
  }
}

async function driveRedactApply(contents: string | null): Promise<void> {
  const wanted = driveWanted(contents);
  if (!wanted) return;
  const supplied = wanted.sanitize as Partial<RedactSanitize> | undefined;
  const sanitize = Object.fromEntries(REDACT_KEYS.map(key => [key, typeof supplied?.[key] === "boolean" ? supplied[key] : true])) as RedactSanitize;
  await applyRedact(sanitize);
}

// ----------------------------------------------------------------- notes / edits

/**
 * The worker owns dirty state; the renderer mirrors the last reply and tells
 * main, which mirrors it once more for the window (setDocumentEdited).
 */
function syncDirty() {
  document.title = !doc ? "Tandem PDF" : dirty ? `• ${doc.name} — Tandem PDF` : `${doc.name} — Tandem PDF`;
  dirtyChipEl.hidden = !dirty;
  // M9: while a restore runs the window shows no document, but its unsaved edits are not gone.
  reportedDirty = dirty || restoreDirty;
  api?.setDirty?.(reportedDirty);
}

/** The edit and nav live regions show only the latest message (STATE `announcement` reads either). */
function announce(text: string) {
  editStatusEl.textContent = text;
  navStatusEl.textContent = "";
}

// ------------------------------------------------------------- modes and panels

/** One tablist: exactly one selected tab, roving tabindex, one visible panel. */
function selectTab(tabs: HTMLButtonElement[], panels: HTMLElement[], index: number) {
  // Hiding the panel the user is typing in would drop focus to the body.
  const active = document.activeElement as HTMLElement | null;
  const dropsFocus = !!active && panels.some((panel, i) => i !== index && !panel.hidden && panel.contains(active));
  for (let i = 0; i < tabs.length; i++) {
    const selected = i === index;
    tabs[i].setAttribute("aria-selected", String(selected));
    tabs[i].tabIndex = selected ? 0 : -1;
    panels[i].hidden = !selected;
  }
  if (dropsFocus) tabs[index].focus();
}

function selectTabById(tabs: HTMLButtonElement[], panels: HTMLElement[], id: string) {
  const index = tabs.findIndex(tab => tab.id === id);
  if (index >= 0) selectTab(tabs, panels, index);
}

function setInspectorOpen(open: boolean) {
  // Collapsing the inspector must not strand focus inside a hidden panel.
  const active = document.activeElement as HTMLElement | null;
  const dropsFocus = !open && !!active && inspectorEl.contains(active);
  inspectorOpen = open;
  inspectorEl.hidden = !open;
  inspectorToggle.setAttribute("aria-expanded", String(open));
  if (dropsFocus) inspectorToggle.focus();
}

/** A pending note, text box or ink stroke is only meaningful inside Annotate. */
function cancelPendingTools() {
  directFocusedField = null; clearDirectFocus();
  noteAddButton.setAttribute("aria-pressed", "false");
  freeTextAddButton.setAttribute("aria-pressed", "false");
  inkDrawButton.setAttribute("aria-pressed", "false");
  redactAreaButton.setAttribute("aria-pressed", "false");
  contentAddArmButton.setAttribute("aria-pressed", "false");
  linkDrawButton.setAttribute("aria-pressed", "false");
  sigfieldDrawButton.setAttribute("aria-pressed", "false");
  signArmed = false; fillSignButton.setAttribute("aria-pressed", "false"); // draw-signature D5: a mode change or another tool disarms
  inkStroke = null;
  // §9.14: a dialog a draw opened goes with its draft (its close listener clears the draft).
  if (navLinkDraft && linkDialog.open) linkDialog.close();
  navLinkDraft = null;
}

/**
 * Switching mode shows that task's tool row and that task's inspector panel. It
 * never touches the document: nothing is marked by choosing a mode.
 */
function setMode(mode: UiMode) {
  if (directActive && mode !== uiMode) dropDirect();
  uiMode = mode;
  for (const [input, name, row] of modeInputs) {
    input.checked = name === mode;
    row.hidden = name !== mode;
  }
  cancelPendingTools();
  if (mode === "redact") void refreshRedactPreview();
  selectTabById(inspectorTabs, inspectorPanels, MODE_TAB[mode]);
  // Annotate and Forms work in the inspector, so a collapsed one reopens for them.
  if (!inspectorOpen && (mode === "annotate" || mode === "forms" || mode === "edit")) setInspectorOpen(true);
  if (mode === "edit") void refreshContentTargets(true);
  paintHits(canvas.clientWidth, canvas.clientHeight, shownPage);
  announce("");
}

/** The panel's empty line: a closed document is not the same as an empty page. */
function setEmpty(el: HTMLElement, count: number, pageText: string, documentText: string) {
  el.hidden = count > 0;
  el.textContent = doc ? pageText : documentText;
}

/**
 * Mirrors one mutation/undo/redo/saved reply, then repaints the current page at
 * the new revision, refreshes that page's thumbnail only, and reloads the list.
 */
async function applyEdit(reply: JournalState, announcement: string) {
  revision = reply.revision;
  dirty = reply.dirty;
  canUndo = reply.canUndo;
  canRedo = reply.canRedo;
  syncDirty();
  syncControls();
  if (announcement) announce(announcement);
  await refreshNotes();
  if (uiMode === "redact" || redactState?.marks) await refreshRedactPreview();
  await refreshFields();
  await refreshOutline();
  await refreshLinks();
  await refreshContentTargets(uiMode === "edit");
  invalidateThumb(wantedPage);
  void drain();
  pumpThumbs();
  if (compareDoc) await refreshCompare(false, false);
}

// ----------------------------------------------------------------- page organisation

/**
 * A page op, or an undo/redo of one: the reply is the only source of the new
 * page list, so the thumbnails, the render caches, the now-stale search hits
 * and the annotation list are all rebuilt from it before the page is chosen.
 */
async function applyPageEdit(reply: PageReply, announcement: string, page: number) {
  revision = reply.revision;
  dirty = reply.dirty;
  canUndo = reply.canUndo;
  canRedo = reply.canRedo;
  if (doc) {
    doc.pageCount = reply.pageCount;
    doc.pages = reply.pages;
    doc.rotations = reply.rotations;
  }
  syncDirty();
  cache.clear(); // every cached render and thumbnail is keyed by a page index that just moved
  clearSearch();
  // Page indices behind `navLinks` just moved; only the shown page is reloaded below.
  navLinks = {};
  const sidebarWasOpen = thumbOpen;
  refreshThumbs(); // rebuilds the rows from the new sizes and drops the thumbnail cache
  // The sidebar keeps whatever state the user left it in; a page op must not pop it open.
  setThumbSidebar(sidebarWasOpen);
  if (announcement) announce(announcement);
  goTo(page); // clamps to the new page count, refreshes the list and repaints
  pumpThumbs();
  // A delete repairs the outline, so the panels follow the reply like any other edit.
  await refreshOutline();
  await refreshLinks();
  if (compareDoc) await refreshCompare(false, false);
}

/** "Rotate right"/"Rotate left" on one page; the page keeps its index. */
async function rotatePage(page: number, by: 90 | -90) {
  try {
    const reply = (await request({ op: "page-rotate", page, by })) as PageReply;
    await applyPageEdit(reply, `Rotated page ${page + 1}`, page);
  } catch (err) {
    showError(messageOf(err));
  }
}

/** "Move up"/"Move down": the current page follows to its new index (`to`). */
async function movePage(page: number, to: number) {
  // The ends are a dead end, not an error: the button and the menu item are disabled there.
  if (!doc || to < 0 || to >= doc.pageCount) return;
  try {
    const reply = (await request({ op: "page-move", page, to })) as PageReply;
    await applyPageEdit(reply, `Moved page ${page + 1} ${to < page ? "up" : "down"}`, to);
  } catch (err) {
    showError(messageOf(err));
  }
}

/** "Delete page": one Undo brings it back, which is what the status line says. */
async function deletePage(page: number) {
  if (!doc || doc.pageCount < 2) return;
  try {
    const reply = (await request({ op: "page-delete", page })) as PageReply;
    await applyPageEdit(reply, `Deleted page ${page + 1} — Undo restores it`, Math.min(page, reply.pageCount - 1));
  } catch (err) {
    showError(messageOf(err));
  }
}

/** One path for the toolbar and the Page menu; both act on the current page. */
function runPageOp(op: string): Promise<unknown> | undefined {
  const page = wantedPage;
  if (op === "rotate-right") return rotatePage(page, 90);
  if (op === "rotate-left") return rotatePage(page, -90);
  if (op === "move-up") return movePage(page, page - 1);
  if (op === "move-down") return movePage(page, page + 1);
  if (op === "delete-page") return deletePage(page);
}

/** `--edit=pages`: rotate page 1, move page 3 up twice, delete the last page, then log. */
async function smokePages() {
  if (!doc) {
    logEdit();
    return;
  }
  goTo(0);
  await rotatePage(0, 90);
  goTo(2);
  await movePage(wantedPage, wantedPage - 1);
  await movePage(wantedPage, wantedPage - 1);
  goTo(doc.pageCount - 1);
  await deletePage(wantedPage);
  logEdit();
}

/** `--edit=form`: fill name, check agree, pick combo two, radio M, then log. No script runs. */
async function smokeForm(contents: string | null) {
  if (!doc) {
    logEdit();
    return;
  }
  await refreshFields();
  const text = contents ?? "PDF4All P2 form";
  const target = (name: string) => fields.find(item => item.name === name);
  const nameField = target("name");
  if (nameField) await commitField(nameField, text);
  const agree = target("agree");
  if (agree) await commitField(agree, true);
  const combo = target("choice");
  if (combo) await commitField(combo, "two");
  const radio = fields.find(item => item.kind === "radio" && item.name === "size" && item.on === "M");
  if (radio) await commitField(radio, true);
  logEdit();
}

type FieldDesignWanted = {
  action?: string; kind?: string; name?: string; on?: string; rename?: string;
  rect?: number[]; options?: string[]; kids?: { on: string; rect: number[] }[];
  maxLen?: number; multiline?: boolean; edit?: boolean; value?: string | boolean;
  label?: string; readOnly?: boolean; required?: boolean; default?: string | boolean;
};

/**
 * The U6 drive path (`{"cmd":"fields","contents":{…}}`, contract §3): one designer
 * action per line, every target resolved by name against the current `fields` list.
 * A refusal lands in `P2_EDIT.fields.last.detail` and in the error line, never throws.
 */
async function smokeFieldDesign(contents: string | null): Promise<void> {
  await refreshFields();
  let wanted: FieldDesignWanted = {};
  if (contents && contents.trim().startsWith("{")) {
    try {
      wanted = JSON.parse(contents) as FieldDesignWanted;
    } catch (err) {
      showError(`fields: ${messageOf(err)}`);
      return;
    }
  }
  const action = typeof wanted.action === "string" ? wanted.action : "";
  const name = typeof wanted.name === "string" ? wanted.name : "";
  const target = (on?: string) => fields.find(item => item.name === name && (on === undefined || item.on === on));
  try {
    let reply: MutationReply;
    if (action === "add") {
      const payload: Record<string, unknown> = { op: "field-add", page: wantedPage, kind: wanted.kind, name };
      // P5 U3 §1.2: a signature field's name is optional (the engine picks Signature<n>).
      if (wanted.kind === "signature" && name === "") delete payload.name;
      if (wanted.rect) payload.rect = wanted.rect;
      if (wanted.kids) payload.kids = wanted.kids;
      if (wanted.options) payload.options = wanted.options;
      if (wanted.edit !== undefined) payload.edit = wanted.edit;
      if (wanted.maxLen !== undefined) payload.maxLen = wanted.maxLen;
      if (wanted.multiline !== undefined) payload.multiline = wanted.multiline;
      if (wanted.label !== undefined) payload.label = wanted.label;
      if (wanted.readOnly !== undefined) payload.readOnly = wanted.readOnly;
      if (wanted.required !== undefined) payload.required = wanted.required;
      if (wanted.default !== undefined) payload.default = wanted.default;
      fieldDesign = { action, name };
      reply = (await request(payload)) as MutationReply;
      // §9.13: the name the engine used (a signature field may have been given none).
      const used = (reply as MutationReply & { name?: unknown }).name;
      if (typeof used === "string") fieldDesign = { action, name: used };
      clearError(); // debt1 row 15: an add that lands drops an earlier refusal's #error
    } else if (action === "props") {
      const item = target(wanted.on);
      if (!item) throw new Error(`fields: no field ${name} on this page`);
      const payload: Record<string, unknown> = { op: "field-props", page: wantedPage, widgetId: item.id };
      if (typeof wanted.rename === "string") payload.name = wanted.rename;
      if (wanted.rect) payload.rect = wanted.rect;
      if (wanted.options) payload.options = wanted.options;
      if (wanted.edit !== undefined) payload.edit = wanted.edit;
      if (wanted.maxLen !== undefined) payload.maxLen = wanted.maxLen;
      if (wanted.multiline !== undefined) payload.multiline = wanted.multiline;
      if (wanted.label !== undefined) payload.label = wanted.label;
      if (wanted.readOnly !== undefined) payload.readOnly = wanted.readOnly;
      if (wanted.required !== undefined) payload.required = wanted.required;
      if (wanted.default !== undefined) payload.default = wanted.default;
      const renamed = typeof wanted.rename === "string";
      fieldDesign = { action: renamed ? "rename" : "props", name: renamed ? wanted.rename! : name };
      reply = (await request(payload)) as MutationReply;
    } else if (action === "delete") {
      const item = target(wanted.on);
      if (!item) throw new Error(`fields: no field ${name} on this page`);
      fieldDesign = { action, name };
      reply = (await request({ op: "field-delete", page: wantedPage, widgetId: item.id })) as MutationReply;
    } else if (action === "fill") {
      // A radio's `value` names the kid whose export value is chosen; every other
      // kind takes the value itself (§3).
      const item = fields.find(entry => entry.name === name && (entry.kind === "radio"
        ? (typeof wanted.value === "string" ? entry.on === wanted.value : entry.on === wanted.on)
        : wanted.on === undefined || entry.on === wanted.on));
      if (!item) throw new Error(`fields: no field ${name} on this page`);
      fieldDesign = { action, name };
      const value = item.kind === "radio" ? true : wanted.value;
      reply = (await request({ op: "field-set", page: wantedPage, widgetId: item.id, value })) as MutationReply;
      noteFilled();
    } else {
      throw new Error(`fields: unknown action ${JSON.stringify(action)}`);
    }
    await applyEdit(reply, "");
  } catch (err) {
    const detail = refusalDetail(err);
    fieldDesign = { action, name, detail };
    showError(FIELD_DETAIL_TEXT[detail] ?? messageOf(err));
    await refreshFields();
  }
}

/** The page list is the semantic path to a sticky note; nothing depends on the canvas. */
async function refreshNotes() {
  const seq = ++notesSeq;
  if (!doc || doc.info.needsPassword) {
    notes = [];
    renderNotes();
    return;
  }
  const page = wantedPage;
  const startedGeneration = generation;
  let reply: AnnotsReply;
  try {
    reply = (await request({ op: "annots", page })) as AnnotsReply;
  } catch {
    // A document that changed under us owns the list now; a real failure is not worth an alert.
    return;
  }
  if (seq !== notesSeq || startedGeneration !== generation || wantedPage !== page) return;
  notes = reply.items;
  renderNotes();
  repaint();
}

/** Row label for each editable type; anything else stays a read-only line. */
const ANNOT_LABELS: Record<string, string> = {
  Text: "Note", Highlight: "Highlight", Underline: "Underline", StrikeOut: "StrikeOut", FreeText: "Text box",
  Square: "Square", Circle: "Circle", Line: "Line", Ink: "Ink", Stamp: "Stamp", Redact: "Redaction mark",
};

/** A signature appearance is a Stamp by type; its `/Subj` is what tells the two apart. */
function annotName(item: AnnotItem): string | undefined {
  return item.type === "Stamp" && item.subject === SIGNATURE_SUBJECT
    ? "Signature appearance"
    : ANNOT_LABELS[item.type];
}

// ----------------------------------------------------------------- form fields

/**
 * The panel's own copy of the page's widgets, fetched on every page change and
 * after every edit, so the native controls always show what the document holds.
 */
async function refreshFields() {
  const seq = ++fieldsSeq;
  if (!doc || doc.info.needsPassword) {
    fields = []; fieldsPage = -1; fieldsTabs = null;
    renderFields();
    return;
  }
  const page = wantedPage;
  const startedGeneration = generation;
  let reply: FieldsReply;
  try {
    reply = (await request({ op: "fields", page })) as FieldsReply;
  } catch {
    // A document that changed under us owns the panel now; a real failure is not worth an alert.
    return;
  }
  if (seq !== fieldsSeq || startedGeneration !== generation || wantedPage !== page) return;
  fields = reply.items; fieldsPage = page; fieldsTabs = tabsName((reply as { tabs?: unknown }).tabs);
  if (directFocusedField && directFocusedPage === page) directFocusedField = fields.find(item => item.id === directFocusedField!.id) ?? null;
  renderFields();
  paintHits(canvas.clientWidth, canvas.clientHeight, shownPage);
}

function fieldLabel(item: FieldItem): string {
  return item.label || item.name || `field ${item.id}`;
}

/**
 * §8.10: a fill control of a required field says so itself, and the marker beside the label
 * is decoration for the eye only.
 */
function markRequired(label: HTMLElement, control: HTMLElement | null, item: FieldItem): void {
  if (!item.required) return;
  control?.setAttribute("aria-required", "true");
  const mark = document.createElement("span");
  mark.className = "required-mark";
  mark.textContent = "*";
  mark.title = "required";
  mark.setAttribute("aria-hidden", "true");
  label.append(mark);
}

function controlId(item: FieldItem): string {
  return `field-${item.id}`;
}

/** The scripted warning text, on the row and once at the top of the panel. */
function scriptNote(): HTMLElement {
  const note = document.createElement("p");
  note.className = "field-script";
  note.textContent = SCRIPT_WARNING;
  return note;
}

function renderFields() {
  fieldsHeading.textContent = `Form fields on page ${wantedPage + 1}`;
  fieldsWarningEl.hidden = !fields.some(item => item.scripted);
  fieldsEl.hidden = !doc || doc.info.needsPassword || fields.length === 0;
  setEmpty(fieldsEmptyEl, fields.length, canAnnotate() ? "No fillable fields on this page. Click the page to type." : "No fillable fields on this page.", "No document open yet.");
  fillTextboxAdd.disabled = !doc || !canAnnotate();
  fillSignButton.disabled = !doc || !canAnnotate(); fillSignDrawButton.disabled = !doc || !canAnnotate();
  // An edit rebuilds the rows; the control the user was in keeps focus.
  const activeId = (document.activeElement as HTMLElement | null)?.dataset?.fieldId;
  const radioGroups = new Map<string, FieldItem[]>();
  for (const item of fields) {
    if (item.kind !== "radio") continue;
    const group = radioGroups.get(item.name);
    if (group) group.push(item);
    else radioGroups.set(item.name, [item]);
  }
  const fragment = document.createDocumentFragment();
  const radioDone = new Set<string>();
  for (const item of fields) {
    if (item.kind === "radio") {
      if (radioDone.has(item.name)) continue;
      radioDone.add(item.name);
      fragment.append(radioFieldset(item, radioGroups.get(item.name) ?? [item]));
      continue;
    }
    if (item.kind === "signature") {
      const row = document.createElement("p");
      row.className = "field-row field-note";
      row.textContent = `Signature field ${item.name} — ${sigStatus(item)}; click it on the page to sign`;
      fragment.append(row);
      continue;
    }
    fragment.append(fieldRow(item));
  }
  fieldListEl.replaceChildren(fragment);
  renderDesigner();
  if (activeId) fieldListEl.querySelector<HTMLElement>(`[data-field-id="${activeId}"]`)?.focus();
}

/** One native control for one widget: text/textarea, checkbox or select. */
function fieldRow(item: FieldItem): HTMLElement {
  const row = document.createElement("div");
  row.className = "field-row";
  let control: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement;
  if (item.kind === "checkbox") {
    control = document.createElement("input");
    control.type = "checkbox";
    control.checked = item.value === true;
    control.id = controlId(item);
    const label = document.createElement("label");
    label.className = "field-check";
    label.append(control, document.createTextNode(fieldLabel(item)));
    markRequired(label, control, item);
    row.append(label);
  } else {
    const label = document.createElement("label");
    label.htmlFor = controlId(item);
    label.textContent = fieldLabel(item);
    if (item.kind === "text" && item.multiline) {
      const area = document.createElement("textarea");
      area.rows = 3;
      area.maxLength = item.maxLen ?? MAX_NOTE_CHARS;
      area.value = String(item.value);
      control = area;
    } else if (item.kind === "text") {
      const input = document.createElement("input");
      input.type = "text";
      input.maxLength = item.maxLen ?? MAX_NOTE_CHARS;
      input.value = String(item.value);
      control = input;
    } else if (item.kind === "combo" && item.edit) {
      const input = document.createElement("input"); input.type = "text"; input.maxLength = MAX_FIELD_TEXT_CHARS; input.value = String(item.value);
      const list = document.createElement("datalist"); list.id = `field-options-${item.id}`; input.setAttribute("list", list.id);
      for (const option of item.options ?? []) { const node = document.createElement("option"); node.value = option; list.append(node); }
      row.append(list); control = input;
    } else {
      const select = document.createElement("select");
      for (const option of item.options ?? []) {
        const node = document.createElement("option");
        node.value = option;
        node.textContent = option;
        select.append(node);
      }
      select.value = String(item.value);
      control = select;
    }
    control.id = controlId(item);
    markRequired(label, control, item);
    row.append(label, control);
  }
  control.dataset.fieldId = String(item.id);
  // A read-only field and a document without the form permission are both dead ends.
  control.disabled = item.readOnly || !canForm();
  // `change` is the commit: blur or Enter for single-line text, blur for a textarea.
  control.addEventListener("change", () => void commitField(item, item.kind === "checkbox"
    ? (control as HTMLInputElement).checked
    : (control as HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement).value));
  if (item.scripted) row.append(scriptNote());
  return row;
}

/** Radio kids share one fieldset; each kid is one native radio for its export value. */
function radioFieldset(first: FieldItem, group: FieldItem[]): HTMLElement {
  const fieldset = document.createElement("fieldset");
  fieldset.className = "field-row";
  const legend = document.createElement("legend");
  legend.textContent = fieldLabel(first);
  markRequired(legend, null, first);
  fieldset.append(legend);
  for (const item of group) {
    const label = document.createElement("label");
    label.className = "field-check";
    const input = document.createElement("input");
    input.type = "radio";
    input.name = `field-radio-${item.name}-${first.id}`;
    input.id = controlId(item);
    input.dataset.fieldId = String(item.id);
    input.value = item.on || String(item.id);
    input.checked = item.value === true;
    input.disabled = item.readOnly || !canForm();
    if (first.required) input.setAttribute("aria-required", "true");
    input.addEventListener("change", () => void commitField(item, true));
    label.append(input, document.createTextNode(item.on || `option ${item.id}`));
    fieldset.append(label);
  }
  if (group.some(item => item.scripted)) fieldset.append(scriptNote());
  return fieldset;
}

function fieldAnnouncement(item: FieldItem, value: string | boolean): string {
  if (item.kind === "checkbox") return `${value ? "Checked" : "Unchecked"} ${item.name}`;
  if (item.kind === "radio") return `Chose ${item.on || item.name}`;
  if (item.kind === "text") return `Filled ${item.name}`;
  return `Chose ${String(value)}`;
}

/**
 * One `field-set` per commit, then the panel and the page follow the reply. A
 * refusal (over MaxLen, not an option, read-only) reaches the user and the
 * control snaps back to the document's value without dirtying anything.
 */
async function commitField(item: FieldItem, value: string | boolean, page = wantedPage): Promise<boolean> {
  if (!doc) return false;
  try {
    const reply = (await request({ op: "field-set", page, widgetId: item.id, value })) as MutationReply;
    noteFilled();
    fieldsStatusEl.textContent = fieldAnnouncement(item, value);
    await applyEdit(reply, "");
    return true;
  } catch (err) {
    fieldsStatusEl.textContent = messageOf(err);
    showError(messageOf(err));
    await refreshFields();
    return false;
  }
}

/** The `P2_EDIT.fields` slice: how many, how many scripted, and every value by name. */
function fieldSummary() {
  const values: Record<string, string | boolean> = {};
  for (const item of fields) {
    if (item.kind === "signature") continue;
    if (item.kind === "radio") {
      // The group keeps its key even with nothing selected, so a name never disappears.
      if (!(item.name in values)) values[item.name] = "";
      if (item.value === true) values[item.name] = item.on || "";
      continue;
    }
    values[item.name] = item.value;
  }
  const tabs = tabOrder(fields, fieldsTabs);
  const order = tabs.order.filter(item => !item.readOnly && item.kind !== "signature").map(item => item.name);
  return { count: fields.length, scripted: fields.filter(item => item.scripted).length, values, defs: fieldDefs(), ...(fieldDesign ? { last: fieldDesign } : {}), tabs: fieldsTabs, tabsOrder: tabs.used, order };
}

/** U6 §3: every field's kind and `getRect()` rect, radio kids as `<name>.<on>`. U7 §3 adds the
 * four properties "only when set": a raw `/TU`, a true flag bit, or a stored `/DV` (a false
 * flag and an absent key are the same thing, so they stay out of the drive state). */
function fieldDefs() {
  const defs: Record<string, { kind: string; rect: number[]; label?: string; readOnly?: boolean; required?: boolean; default?: string | boolean }> = {};
  const properties = (item: FieldItem) => ({
    ...(item.tu !== undefined ? { label: item.tu } : {}),
    ...(item.readOnly ? { readOnly: true } : {}),
    ...(item.required ? { required: true } : {}),
    ...(item.default !== undefined ? { default: item.default } : {}),
    ...(item.edit ? { edit: true } : {}),
  });
  for (const item of fields) {
    if (item.kind === "signature") continue;
    if (item.kind === "radio") {
      defs[`${item.name}.${item.on ?? item.id}`] = { kind: "radio", rect: item.rect };
      if (!(item.name in defs)) defs[item.name] = { kind: "radio", rect: item.rect, ...properties(item) };
      continue;
    }
    defs[item.name] = { kind: item.kind, rect: item.rect, ...properties(item) };
  }
  return defs;
}

// ----------------------------------------------------------------- field designer (U6)

/** The U6 refusal tokens in the words the panel shows (contract §3). */
const FIELD_DETAIL_TEXT: Record<string, string> = {
  xfa: "This is an XFA form; fields cannot be added.",
  scripted: "Runs a script this app does not execute.",
  signature: "A signed field cannot be changed.",
  "duplicate-name": "A field with that name already exists.",
  "invalid-name": "Names are 1–128 characters without dots.",
  "too-many": "This form is too large to design fields in.",
  "stale-target": "That field is no longer on this page.",
  // §8.1: the field's own read-only flag, told apart from the document-level refusal.
  "field-read-only": "This field is read-only",
  invalid: "That change is not possible on this field.",
};

/** The last designer action, for the drive state (`P2_EDIT.fields.last`, §3). */
let fieldDesign: { action: string; name: string; detail?: string } | null = null;

/** The refusal token a worker error carries, `"invalid"` when it has none. */
/**
 * The worker's refusal reply and the error's own `reason` tag, read in one place
 * (contract §5.6). `err.reply` is the `{ ok: false }` message the renderer attaches
 * in `onWorkerMessage`; `err.reason` is the tagged reason on the Error itself.
 */
function refusalOf(err: unknown): { reply: RefusalReply | null; reason?: string } {
  const failure = err as { reply?: RefusalReply; reason?: unknown } | null;
  return {
    reply: failure?.reply ?? null,
    ...(typeof failure?.reason === "string" ? { reason: failure.reason } : {}),
  };
}

function refusalDetail(err: unknown): string {
  const { reply } = refusalOf(err);
  return typeof reply?.detail === "string" ? reply.detail : "invalid";
}

/** The designer's Add rect in page points, y down (the `#content-box-*` convention). */
function designerRect(els: [HTMLInputElement, HTMLInputElement, HTMLInputElement, HTMLInputElement]): number[] {
  const number = (el: HTMLInputElement, fallback: number) => {
    const value = Number(el.value);
    return Number.isFinite(value) ? value : fallback;
  };
  const x = number(els[0], 72);
  const y = number(els[1], 72);
  return [x, y, x + number(els[2], 180), y + number(els[3], 24)];
}

/** One non-empty trimmed line per entry (the options and radio-kid textareas). */
function textareaLines(area: HTMLTextAreaElement): string[] {
  return area.value.split("\n").map(line => line.trim()).filter(line => line !== "");
}

const round2 = (value: number) => Math.round(value * 100) / 100;
const round3 = (value: number) => Math.round(value * 1000) / 1000;

/** The designer shows what the current kind needs and lists the page's fields (§3, §7.13). */
function renderDesigner() {
  const usable = !!doc && !doc.info.needsPassword && canDesign() && !doc.info.xfa;
  designerEl.hidden = !doc || doc.info.needsPassword || !(canDesign() || doc.info.xfa);
  if (!doc || doc.info.needsPassword) return;
  const xfa = doc.info.xfa;
  designerHintEl.hidden = false;
  // §7.2 visibility, and the ceilings §6 says the hint must state.
  designerHintEl.textContent = xfa
    ? "This is an XFA form; fields cannot be added"
    : "Position is in page points from the page top-left, unrotated. Top-level fields only, Helvetica 12 pt; no list boxes or push buttons. A signature field's name is optional.";
  fieldNewNameEl.disabled = !usable;
  fieldNewAddEl.disabled = !usable;
  fieldNewKindEl.disabled = !usable;
  const kind = fieldNewKindEl.value;
  fieldNewRectEl.hidden = kind === "radio";
  fieldNewTextEl.hidden = kind !== "text";
  fieldNewComboEl.hidden = kind !== "combo";
  fieldNewRadioEl.hidden = kind !== "radio";
  // U7: a checkbox default is a checked box; every other kind reads or names a value.
  fieldNewDefaultTextEl.hidden = kind === "checkbox" || kind === "signature";
  fieldNewDefaultBoolEl.hidden = kind !== "checkbox";
  // The checkbox default is its own 16x16, the rest keep the text-box 180x24.
  if (kind === "checkbox" && fieldNewWEl.value === "180" && fieldNewHEl.value === "24") {
    fieldNewWEl.value = "16";
    fieldNewHEl.value = "16";
  } else if (kind !== "checkbox" && fieldNewWEl.value === "16" && fieldNewHEl.value === "16") {
    fieldNewWEl.value = "180";
    fieldNewHEl.value = "24";
  }
  const previous = fieldSelectEl.value;
  fieldSelectEl.replaceChildren(...fields.map(item => {
    const option = document.createElement("option");
    option.value = String(item.id);
    option.textContent = item.kind === "radio" ? `${item.name} · ${item.on ?? ""}` : (item.name || item.label || `field ${item.id}`);
    const signedSig = item.kind === "signature" && sigSigned(item);
    option.disabled = item.scripted || signedSig;
    if (signedSig) option.title = FIELD_DETAIL_TEXT.signature;
    else if (item.scripted) option.title = SCRIPT_WARNING;
    return option;
  }));
  if (fields.some(item => String(item.id) === previous)) fieldSelectEl.value = previous;
  renderSelectedField();
}

/** The selected field's values, with the inputs that do not apply to its kind disabled. */
function renderSelectedField() {
  const item = fields.find(entry => String(entry.id) === fieldSelectEl.value);
  const signedSig = !!item && item.kind === "signature" && sigSigned(item);
  const usable = !!item && !item.scripted && !signedSig && canDesign();
  fieldPropApplyEl.disabled = !usable;
  fieldPropDeleteEl.disabled = !usable;
  // §9.1/§9.19: a signature field keeps its name (display only).
  fieldPropNameEl.disabled = !usable || item?.kind === "signature";
  const kind = item?.kind ?? "text";
  fieldPropRectEl.hidden = !item || signedSig;
  fieldPropTextEl.hidden = kind !== "text";
  fieldPropComboEl.hidden = kind !== "combo";
  fieldPropMaxLenEl.disabled = !usable || kind !== "text";
  fieldPropMultilineEl.disabled = !usable || kind !== "text";
  fieldPropOptionsEl.disabled = !usable || kind !== "combo";
  fieldPropLabelEl.disabled = !usable;
  fieldPropReadOnlyEl.disabled = !usable;
  fieldPropRequiredEl.disabled = !usable;
  fieldPropDefaultTextEl.disabled = !usable || kind === "signature";
  fieldPropDefaultBoolEl.disabled = !usable || kind === "signature";
  fieldPropDefaultTextEl.hidden = kind === "checkbox";
  fieldPropDefaultBoolEl.hidden = kind !== "checkbox";
  if (!item) {
    fieldPropNameEl.value = "";
    return;
  }
  fieldPropNameEl.value = item.name;
  // §8.4: the label input is the field's own `/TU`, not the `label` fallback to the name,
  // so applying without touching it never writes one.
  fieldPropLabelEl.value = item.tu ?? "";
  fieldPropReadOnlyEl.checked = item.readOnly;
  fieldPropRequiredEl.checked = item.required;
  if (kind === "checkbox") fieldPropDefaultBoolEl.checked = item.default === true;
  else fieldPropDefaultTextEl.value = typeof item.default === "string" ? item.default : "";
  const [x0, y0, x1, y1] = item.rect.length === 4 ? item.rect : [0, 0, 0, 0];
  fieldPropXEl.value = String(round2(x0));
  fieldPropYEl.value = String(round2(y0));
  fieldPropWEl.value = String(round2(x1 - x0));
  fieldPropHEl.value = String(round2(y1 - y0));
  fieldPropMaxLenEl.value = String(item.maxLen ?? 0);
  fieldPropMultilineEl.checked = item.multiline === true;
  fieldPropOptionsEl.value = (item.options ?? []).join("\n");
}

/** The one refusal path every designer action shares: announce, keep the panel truthful. */
async function refuseDesigner(action: string, name: string, err: unknown) {
  const detail = refusalDetail(err);
  fieldDesign = { action, name, detail };
  fieldsStatusEl.textContent = FIELD_DETAIL_TEXT[detail] ?? messageOf(err);
  showError(FIELD_DETAIL_TEXT[detail] ?? messageOf(err));
  await refreshFields();
}

/** `field-add` from the Add group: one op, one undo step, then the new row is focused. */
async function addDesignedField() {
  if (!doc || !canDesign()) return;
  const kind = fieldNewKindEl.value as "text" | "checkbox" | "combo" | "radio" | "signature";
  const name = fieldNewNameEl.value;
  const payload: Record<string, unknown> = { op: "field-add", page: wantedPage, kind, name };
  if (kind === "signature" && name === "") delete payload.name;
  if (kind === "radio") {
    const x = Number(fieldNewXEl.value) || 72;
    const y = Number(fieldNewYEl.value) || 72;
    payload.kids = textareaLines(fieldNewKidsEl).map((on, index) => ({
      on, rect: [x, y + index * 20, x + 16, y + 16 + index * 20],
    }));
  } else {
    payload.rect = designerRect([fieldNewXEl, fieldNewYEl, fieldNewWEl, fieldNewHEl]);
    if (kind === "combo") payload.options = textareaLines(fieldNewOptionsEl);
    if (kind === "text") {
      const maxLen = Number(fieldNewMaxLenEl.value) || 0;
      if (maxLen > 0) payload.maxLen = maxLen;
      if (fieldNewMultilineEl.checked) payload.multiline = true;
    }
  }
  // U7: an empty label or default is "no label", not an empty one.
  if (fieldNewLabelEl.value !== "") payload.label = fieldNewLabelEl.value;
  if (fieldNewReadOnlyEl.checked) payload.readOnly = true;
  if (fieldNewRequiredEl.checked) payload.required = true;
  if (kind === "checkbox") { if (fieldNewDefaultBoolEl.checked) payload.default = true; }
  else if (kind !== "signature" && fieldNewDefaultTextEl.value !== "") payload.default = fieldNewDefaultTextEl.value;
  try {
    const reply = (await request(payload)) as MutationReply & { widgetId?: number; name?: unknown };
    const used = typeof reply.name === "string" ? reply.name : name;
    fieldDesign = { action: "add", name: used };
    fieldsStatusEl.textContent = `Added ${kind} field ${used}`;
    clearError(); // debt1 row 15: an add that lands drops an earlier refusal's #error
    await applyEdit(reply, "");
    const created = fields.find(item => item.id === reply.widgetId);
    if (created) fieldListEl.querySelector<HTMLElement>(`[data-field-id="${created.id}"]`)?.focus();
  } catch (err) {
    await refuseDesigner("add", name, err);
  }
}

/** `field-props` for the selected field: only the inputs its kind owns travel. */
async function applyDesignedField() {
  const item = fields.find(entry => String(entry.id) === fieldSelectEl.value);
  if (!doc || !item || !canDesign() || item.scripted || (item.kind === "signature" && sigSigned(item))) return;
  const payload: Record<string, unknown> = { op: "field-props", page: wantedPage, widgetId: item.id };
  // §9.1: a signature field keeps its name, so none travels (a same-name rename would be refused).
  if (item.kind !== "signature") payload.name = fieldPropNameEl.value;
  payload.rect = designerRect([fieldPropXEl, fieldPropYEl, fieldPropWEl, fieldPropHEl]);
  if (item.kind === "text") {
    payload.maxLen = Number(fieldPropMaxLenEl.value) || 0;
    payload.multiline = fieldPropMultilineEl.checked;
  }
  if (item.kind === "combo") payload.options = textareaLines(fieldPropOptionsEl);
  // U7: only what the user actually changed travels, so an untouched label never deletes a
  // `/TU` and an untouched default never rewrites one.
  if (fieldPropLabelEl.value !== (item.tu ?? "")) payload.label = fieldPropLabelEl.value;
  if (fieldPropReadOnlyEl.checked !== item.readOnly) payload.readOnly = fieldPropReadOnlyEl.checked;
  if (fieldPropRequiredEl.checked !== item.required) payload.required = fieldPropRequiredEl.checked;
  if (item.kind === "checkbox") {
    if (fieldPropDefaultBoolEl.checked !== (item.default === true)) payload.default = fieldPropDefaultBoolEl.checked;
  } else if (item.kind !== "signature" && fieldPropDefaultTextEl.value !== (typeof item.default === "string" ? item.default : "")) {
    payload.default = fieldPropDefaultTextEl.value;
  }
  try {
    const reply = (await request(payload)) as MutationReply;
    const renamed = payload.name !== undefined && payload.name !== item.name;
    // The status names what the user actually changed; the rect travels on every apply, so it
    // is only listed when the box really moved (U7 review follow-up 2).
    const box = payload.rect as number[] | undefined;
    const was = item.rect.length === 4 ? item.rect : [0, 0, 0, 0];
    const touched = [
      ...(box && box.some((value, index) => Math.abs(value - was[index]) > 0.01) ? ["position"] : []),
      ...(payload.label !== undefined ? ["label"] : []),
      ...(payload.readOnly !== undefined ? ["read-only"] : []),
      ...(payload.required !== undefined ? ["required"] : []),
      ...(payload.default !== undefined ? ["default"] : []),
      ...(payload.edit !== undefined ? ["edit"] : []),
      ...(item.kind === "text" && (payload.maxLen !== undefined || payload.multiline !== undefined) ? ["text options"] : []),
      ...(item.kind === "combo" && payload.options !== undefined ? ["options"] : []),
    ];
    fieldDesign = { action: renamed ? "rename" : "props", name: String(payload.name ?? item.name) };
    const movedOnly = touched.length === 0 || (touched.length === 1 && touched[0] === "position");
    fieldsStatusEl.textContent = renamed
      ? `Renamed ${item.name} to ${String(payload.name)}`
      : movedOnly ? `Moved ${item.name}` : `Updated ${item.name}: ${touched.join(", ")}`;
    await applyEdit(reply, "");
  } catch (err) {
    await refuseDesigner("props", item.name, err);
  }
}

/** `field-delete` for the selected field; focus returns to the select after it. */
async function deleteDesignedField() {
  const item = fields.find(entry => String(entry.id) === fieldSelectEl.value);
  if (!doc || !item || !canDesign() || item.scripted || (item.kind === "signature" && sigSigned(item))) return;
  try {
    const reply = (await request({ op: "field-delete", page: wantedPage, widgetId: item.id })) as MutationReply;
    fieldDesign = { action: "delete", name: item.name };
    fieldsStatusEl.textContent = `Deleted ${item.name}`;
    await applyEdit(reply, "");
    fieldSelectEl.focus();
  } catch (err) {
    await refuseDesigner("delete", item.name, err);
  }
}

// ----------------------------------------------------------------- bookmarks and links

/**
 * The document's bookmarks, refreshed on every page change, edit, undo/redo and
 * page op like the other panels; the reply's `id` is the preorder index.
 */
async function refreshOutline() {
  const seq = ++outlineSeq;
  if (!doc || doc.info.needsPassword) {
    outline = [];
    renderOutline();
    return;
  }
  const startedGeneration = generation;
  let reply: OutlineReply;
  try {
    reply = (await request({ op: "outline" })) as OutlineReply;
  } catch {
    // A document that changed under us owns the panel now.
    return;
  }
  if (seq !== outlineSeq || startedGeneration !== generation) return;
  outline = reply.items;
  renderOutline();
}

/** The shown page's links, plus the smoke's `P2_EDIT.nav.links` slice for that page. */
async function refreshLinks() {
  const seq = ++linksSeq;
  if (!doc || doc.info.needsPassword) {
    links = [];
    linksPage = wantedPage;
    renderLinks();
    return;
  }
  const page = wantedPage;
  const startedGeneration = generation;
  let reply: LinksReply;
  try {
    reply = (await request({ op: "links", page })) as LinksReply;
  } catch {
    // §9.5: a current failure empties the page's links (so the drive's wait passes); a stale one touches nothing.
    if (seq === linksSeq && startedGeneration === generation && wantedPage === page) { links = []; linksPage = page; renderLinks(); paintHits(canvas.clientWidth, canvas.clientHeight, shownPage); }
    return;
  }
  if (seq !== linksSeq || startedGeneration !== generation || wantedPage !== page) return;
  links = reply.items;
  linksPage = page;
  navLinks = { ...navLinks, [page]: linkSummary(reply.items) };
  renderLinks();
  if (page === shownPage) paintHits(canvas.clientWidth, canvas.clientHeight, shownPage);
}

/** The `P2_EDIT.nav.links` entry for one page: page links keep their index, the rest their URI. */
function linkSummary(items: LinkEntry[]): { kind: string; page?: number; uri?: string }[] {
  return items.map(item => item.kind === "page"
    ? { kind: "page", page: item.page }
    : { kind: item.kind, uri: item.uri });
}

/** The nav smoke reads every page's links; every other path reads only the shown one. */
async function loadAllLinks() {
  if (!doc) return;
  const startedGeneration = generation;
  for (let page = 0; page < doc.pageCount; page++) {
    const reply = (await request({ op: "links", page })) as LinksReply;
    if (startedGeneration !== generation) return;
    navLinks = { ...navLinks, [page]: linkSummary(reply.items) };
    if (page === wantedPage) {
      links = reply.items;
      linksPage = page;
      renderLinks();
    }
  }
}

/** A bookmark row: the title navigates, Rename and Delete act on the preorder id. */
function renderOutline() {
  // A nested <ul> per depth: the preorder list maps onto one list per level.
  const lists: HTMLUListElement[] = [outlineListEl];
  outlineListEl.replaceChildren();
  setEmpty(outlineEmptyEl, outline.length, "No bookmarks in this document.", "No document open yet.");
  if (!outline.length) return;
  for (let index = 0; index < outline.length; index++) {
    const entry = outline[index];
    const depth = Math.min(entry.depth, lists.length - 1);
    const list = lists[depth];
    const row = document.createElement("li");
    if (entry.page !== null) row.dataset.page = String(entry.page);
    const go = document.createElement("button");
    go.type = "button";
    go.dataset.action = "go";
    go.dataset.outlineId = String(entry.id);
    go.textContent = entry.title;
    go.setAttribute("aria-label", entry.page !== null
      ? `Go to bookmark ${entry.title}, page ${entry.page + 1}`
      : `Bookmark ${entry.title}`);
    row.append(go);
    const rename = document.createElement("button");
    rename.type = "button";
    rename.dataset.action = "rename";
    rename.dataset.outlineId = String(entry.id);
    rename.textContent = "Rename";
    rename.setAttribute("aria-label", `Rename bookmark ${entry.title}`);
    rename.disabled = !canEdit();
    const remove = document.createElement("button");
    remove.type = "button";
    remove.dataset.action = "delete";
    remove.dataset.outlineId = String(entry.id);
    remove.textContent = "Delete";
    remove.setAttribute("aria-label", `Delete bookmark ${entry.title}`);
    remove.disabled = !canEdit();
    row.append(rename, remove);
    list.append(row);
    // The next level's list is created lazily, so an item with children owns one.
    if (outline[index + 1]?.depth === depth + 1) {
      const nested = document.createElement("ul");
      row.append(nested);
      lists[depth + 1] = nested;
    }
  }
}

/** "Page 3", the URI, or "Blocked link (file:)"; the last one has no action. */
function linkLabel(item: LinkEntry): string {
  if (item.kind === "page") return `Page ${(item.page ?? 0) + 1}`;
  if (item.kind === "uri") return item.uri ?? "";
  return `Blocked link (${schemeName(item.uri ?? "")})`;
}

/** The scheme as the UI names it, with a colon when there is one. */
function schemeName(uri: string): string {
  const match = /^([A-Za-z][A-Za-z0-9+.-]*):/.exec(uri.trim());
  return match ? `${match[1].toLowerCase()}:` : "unknown";
}

function renderLinks() {
  linksHeading.textContent = `Links on page ${wantedPage + 1}`;
  const fragment = document.createDocumentFragment();
  for (const item of links) {
    const row = document.createElement("li");
    row.dataset.annotId = String(linkAnnotId(item));
    const label = document.createElement("span");
    label.textContent = linkLabel(item);
    row.append(label);
    if (item.kind === "page") {
      const go = document.createElement("button");
      go.type = "button";
      go.dataset.action = "go";
      go.dataset.linkId = String(item.id);
      go.textContent = "Go";
      go.setAttribute("aria-label", `Go to ${linkLabel(item)}`);
      row.append(go);
    } else if (item.kind === "uri") {
      const open = document.createElement("button");
      open.type = "button";
      open.dataset.action = "open";
      open.dataset.linkId = String(item.id);
      open.textContent = "Open";
      open.setAttribute("aria-label", `Open ${item.uri ?? ""}`);
      row.append(open);
    }
    // A blocked link has no action button at all; Delete is the only thing to do with it.
    const remove = document.createElement("button");
    remove.type = "button";
    remove.dataset.action = "delete";
    remove.dataset.linkId = String(item.id);
    remove.textContent = "Delete";
    remove.setAttribute("aria-label", `Delete link ${linkLabel(item)}`);
    remove.disabled = !canAnnotate();
    row.append(remove);
    fragment.append(row);
  }
  linkListEl.replaceChildren(fragment);
  setEmpty(linksEmptyEl, links.length, "No links on this page.", "No document open yet.");
  syncLinkCurrent();
}

/** Following a bookmark: a page destination navigates, a web one goes to main. */
function followOutline(entry: OutlineEntry) {
  if (entry.page !== null) {
    goTo(entry.page);
    return;
  }
  if (!entry.uri) {
    announceNav("That bookmark has no destination");
    return;
  }
  const scheme = schemeName(entry.uri);
  if (scheme === "http:" || scheme === "https:" || scheme === "mailto:") {
    api?.openLink?.(entry.uri);
    return;
  }
  announceNav(`Blocked link (${scheme})`);
}

/** Following a link: only a page or an allowed URI has an action. */
function followLink(item: LinkEntry) {
  if (item.kind === "page") {
    navLast = { action: "link-follow", kind: "page", page: item.page ?? 0 };
    goTo(item.page ?? 0);
  } else if (item.kind === "uri" && item.uri) {
    navLast = { action: "link-follow", kind: "uri", uri: item.uri };
    api?.openLink?.(item.uri);
  } else {
    navLast = { action: "link-follow", kind: "blocked", uri: item.uri ?? "" };
    announceNav(linkLabel(item)); // same words as the panel row and a blocked bookmark
  }
}

/** The union of a search hit's quads, the anchor a new link takes (like a markup). */
function hitRect(hit: SearchHit): number[] | null {
  const quads = hit.quads.filter(quad => quad.length === 8);
  if (!quads.length) return null;
  const xs = quads.flatMap(quad => [quad[0], quad[2], quad[4], quad[6]]);
  const ys = quads.flatMap(quad => [quad[1], quad[3], quad[5], quad[7]]);
  return [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)];
}

function announceNav(text: string) {
  navStatusEl.textContent = text;
  editStatusEl.textContent = "";
}

/** "Add bookmark" and a bookmark's Rename share this dialog; `editingBookmark` picks the mode. */
function openBookmarkDialog(outlineId: number | null = null) {
  if (!doc || !canEdit()) return;
  editingBookmark = outlineId;
  const entry = outlineId === null ? null : outline.find(item => item.id === outlineId);
  bookmarkDialogHeading.textContent = entry ? "Rename bookmark" : "Add bookmark";
  bookmarkTitleInput.value = entry ? entry.title : `Page ${wantedPage + 1}`;
  bookmarkDialog.showModal();
  bookmarkTitleInput.focus();
  bookmarkTitleInput.select();
}

async function saveBookmarkDialog(): Promise<boolean> {
  const outlineId = editingBookmark;
  const title = bookmarkTitleInput.value.slice(0, MAX_BOOKMARK_CHARS);
  if (!title.trim()) {
    // The engine refuses it too; keeping the dialog open is the keyboard-friendly path.
    showError("A bookmark needs a title");
    bookmarkTitleInput.focus();
    return false;
  }
  bookmarkDialog.close();
  if (outlineId === null) return addBookmark(title, wantedPage);
  return renameBookmark(outlineId, title);
}

async function addBookmark(title: string, page: number): Promise<boolean> {
  try {
    const reply = (await request({ op: "outline-add", title, page })) as MutationReply;
    await applyEdit(reply, "");
    announceNav(`Added bookmark Page ${page + 1}`);
    return true;
  } catch (err) {
    showError(messageOf(err));
    return false;
  }
}

async function renameBookmark(outlineId: number, title: string): Promise<boolean> {
  try {
    const reply = (await request({ op: "outline-set", outlineId, title })) as MutationReply;
    await applyEdit(reply, "");
    announceNav("Renamed bookmark");
    return true;
  } catch (err) {
    showError(messageOf(err));
    return false;
  }
}

async function deleteBookmark(outlineId: number) {
  try {
    const reply = (await request({ op: "outline-delete", outlineId })) as MutationReply;
    await applyEdit(reply, "");
    announceNav("Deleted bookmark");
  } catch (err) {
    showError(messageOf(err));
  }
}

/**
 * "Link to page…" and "Web link…" share one dialog; both anchor on the current
 * search match, whose rect is captured when the dialog opens.
 */
function openLinkDialog(mode: "page" | "uri") {
  const hit = currentHit();
  const rect = hit && hitRect(hit);
  if (!doc || !canAnnotate()) return;
  if (!rect) {
    announceNav("Find something first");
    return;
  }
  openLinkDialogAt(rect, mode);
}

function openLinkDialogAt(rect: number[], mode: "page" | "uri") {
  linkAnchor = rect;
  setLinkMode(mode, false);
  linkTargetInput.value = mode === "page" ? "" : "https://";
  if (!linkDialog.open) linkDialog.showModal();
  linkTargetInput.focus();
  linkTargetInput.select();
}

/** §1.6: the dialog's Page / Web choice; switching keeps a typed value and swaps only the other kind's default. */
function setLinkMode(mode: "page" | "uri", swapDefault: boolean) {
  linkMode = mode;
  linkKindSelect.value = mode;
  linkDialogHeading.textContent = mode === "page" ? "Link to page" : "Web link";
  linkTargetLabel.textContent = mode === "page" ? "Page number" : "Web address";
  linkTargetInput.placeholder = mode === "page" ? `1–${doc?.pageCount ?? 1}` : "https://";
  if (swapDefault && mode === "uri" && linkTargetInput.value === "") linkTargetInput.value = "https://";
  if (swapDefault && mode === "page" && linkTargetInput.value === "https://") linkTargetInput.value = "";
}

/**
 * P6 U4 §9.8: one page-level area drag (Draw link, Mark area, unarmed Redact). Listens on `window`
 * with no pointer capture so a drive's window-dispatched events reach it; under 3 CSS px it is a click.
 */
function startAreaDrag(event: PointerEvent, page: number, onRect: (rect: number[]) => unknown, onEnd?: () => void): void {
  event.preventDefault();
  const startX = event.clientX, startY = event.clientY;
  const draft = document.createElement("div"); draft.className = "area-draft"; draft.hidden = true; hitLayer.append(draft);
  const toPt = (x: number, y: number): [number, number] | null => {
    const size = doc?.pages[page], box = canvas.getBoundingClientRect();
    if (!size || box.width < 1 || box.height < 1) return null;
    const round = (value: number) => Math.round(value * 100) / 100;
    return [round(Math.max(0, Math.min(size.width, (x - box.left) * size.width / box.width))), round(Math.max(0, Math.min(size.height, (y - box.top) * size.height / box.height)))];
  };
  const rectTo = (x: number, y: number): number[] | null => {
    const a = toPt(startX, startY), b = toPt(x, y);
    return a && b ? [Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.max(a[0], b[0]), Math.max(a[1], b[1])] : null;
  };
  const move = (next: PointerEvent) => {
    const rect = Math.hypot(next.clientX - startX, next.clientY - startY) >= 3 ? rectTo(next.clientX, next.clientY) : null;
    draft.hidden = !rect; if (rect) placeBox(draft, rect, shownZoom);
  };
  const end = (next: PointerEvent) => {
    window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", end); window.removeEventListener("pointercancel", end);
    draft.remove();
    // The trailing click never reaches click-to-add; it still drops a mark's focus (clickToAdd does that first).
    addBlocked = true;
    onEnd?.();
    if (next.type === "pointercancel" || Math.hypot(next.clientX - startX, next.clientY - startY) < 3) return;
    const rect = rectTo(next.clientX, next.clientY);
    if (rect) runGestureWork(async () => { await onRect(rect); });
  };
  window.addEventListener("pointermove", move); window.addEventListener("pointerup", end); window.addEventListener("pointercancel", end);
}

/** P5 U3 §2.4/§9.14/§9.21: a drawn rect becomes one empty signature field (Forms mode, designer rights). */
async function addSignatureField(page: number, rect: number[]): Promise<void> {
  if (!doc || !canDesign() || uiMode !== "forms") { fieldDesign = { action: "add", name: "", detail: "invalid" }; return; }
  if (rect[2] - rect[0] < 40 || rect[3] - rect[1] < 20) { showError("Draw a bigger signature field"); return; }
  try {
    const reply = (await request({ op: "field-add", page, kind: "signature", rect })) as MutationReply & { name?: unknown };
    const name = typeof reply.name === "string" ? reply.name : "";
    fieldDesign = { action: "add", name };
    fieldsStatusEl.textContent = `Added signature field ${name}`;
    clearError();
    await applyEdit(reply, "");
  } catch (err) {
    const detail = refusalDetail(err);
    fieldDesign = { action: "add", name: "", detail };
    showError(FIELD_DETAIL_TEXT[detail] ?? messageOf(err));
    await refreshFields();
  }
}

/** §1.6: a drawn rect opens the link dialog on it; under 4 × 4 pt is refused. */
function drawnLink(page: number, rect: number[]): void {
  if (!doc || !canAnnotate()) return;
  if (rect[2] - rect[0] < 4 || rect[3] - rect[1] < 4) { showError("Draw a bigger link"); return; }
  openLinkDialogAt(rect, linkMode);
  navLinkDraft = { page, rect };
  navLast = { action: "link-draw", page, rect };
}

/** The dialog's `close` event only arrives with the next frame; a drive step may log STATE before it, so the anchor and the draft go now. */
function closeLinkDialog(): void {
  if (linkDialog.open) linkDialog.close();
  linkAnchor = null; navLinkDraft = null;
}

async function saveLinkDialog(): Promise<boolean> {
  const anchor = linkAnchor;
  const mode = linkMode;
  const text = linkTargetInput.value.trim();
  if (!doc || !anchor) {
    closeLinkDialog();
    return false;
  }
  if (mode === "page") {
    const page = Number(text);
    if (!Number.isInteger(page) || page < 1 || page > doc.pageCount) {
      showError(`Enter a page number between 1 and ${doc.pageCount}`);
      return false;
    }
    closeLinkDialog();
    return addLink(wantedPage, anchor, { page: page - 1 });
  }
  if (!text) {
    // Keep the refusal in the dialog rather than round-tripping a URI the engine will reject.
    showError("Enter a web address");
    linkTargetInput.focus();
    return false;
  }
  closeLinkDialog();
  return addLink(wantedPage, anchor, { uri: text.slice(0, MAX_URI_CHARS) });
}

async function addLink(page: number, rect: number[], target: { page: number } | { uri: string }): Promise<boolean> {
  try {
    const reply = (await request({ op: "link-add", page, rect, target })) as MutationReply;
    await applyEdit(reply, "");
    announceNav("page" in target ? `Added link to page ${target.page + 1}` : "Added web link");
    return true;
  } catch (err) {
    showError(messageOf(err));
    return false;
  }
}

async function deleteLink(linkId: number) {
  // A deleted link takes its on-page focus with it (page key, panel row or drive alike).
  const gone = links.find(entry => entry.id === linkId);
  if (gone && directFocusedLink?.annotId === linkAnnotId(gone)) { directFocusedLink = null; markSelected(null); syncLinkCurrent(); }
  try {
    const reply = (await request({ op: "link-delete", page: wantedPage, linkId })) as MutationReply;
    await applyEdit(reply, "");
    announceNav("Deleted link");
  } catch (err) {
    showError(messageOf(err));
  }
}

/** P6 U4 §1.1: the link's `/Annots` object number (0 until the engine reports one, or for an inline dict). */
function linkAnnotId(item: LinkEntry): number {
  const id = (item as LinkEntry & { annotId?: unknown }).annotId;
  return typeof id === "number" && Number.isInteger(id) ? id : 0;
}

/** P6 U4 §9.7: every link-set (drag, resize, nudge, editor, panel) records `nav.last`; a noop writes nothing. */
async function linkSet(page: number, item: LinkEntry, change: { rect?: unknown; target?: { page: number } | { uri: string } }, announcement = "Changed link", silent = false): Promise<boolean> {
  const index = item.id;
  try {
    const reply = await request({ op: "link-set", page, annotId: linkAnnotId(item), ...change }) as MutationReply & { noop?: boolean };
    navLast = { action: "link-set", index };
    if (reply.noop !== true) await applyEdit(reply, announcement);
    return true;
  } catch (err) {
    const { reply } = refusalOf(err);
    navLast = { action: "link-set", index, detail: typeof reply?.detail === "string" ? reply.detail : "invalid" };
    if (!silent) showError(typeof reply?.error === "string" ? reply.error : messageOf(err));
    return false;
  }
}

/** §1.4: one `link-set {rect}` from a drag, resize or nudge (rounded like `commitMarkRect`); a refusal snaps back. */
async function commitLinkRect(item: LinkEntry, page: number, rect: number[]): Promise<boolean> {
  directFocusedLink = { annotId: linkAnnotId(item) }; directFocusedObject = null; directFocusedMark = null;
  const ok = await linkSet(page, item, { rect: rect.map(value => Math.round(value * 1e4) / 1e4) }, "Moved link");
  if (!ok) paintHits(canvas.clientWidth, canvas.clientHeight, shownPage);
  return ok;
}

/** §1.7: the Links row of the focused link is the current one. */
function syncLinkCurrent(): void {
  const id = directFocusedLink ? String(directFocusedLink.annotId) : null;
  for (const row of linkListEl.querySelectorAll<HTMLElement>("li")) {
    if (id !== null && row.dataset.annotId === id) row.setAttribute("aria-current", "true"); else row.removeAttribute("aria-current");
  }
}

function focusLink(item: LinkEntry, button: HTMLElement): void {
  directFocusedLink = { annotId: linkAnnotId(item) }; directFocusedObject = null; directFocusedMark = null; directFocusedField = null;
  markSelected(button); button.focus({ preventScroll: true }); syncNoteCurrent(); syncLinkCurrent();
}

/** §1.4: Read follows a link; Annotate focuses it, and drags, resizes, nudges, retargets (Enter) and deletes it. */
function paintLinkTarget(item: LinkEntry, page: number): HTMLButtonElement {
  const rect = [...item.rect], annotId = linkAnnotId(item), editing = uiMode === "annotate" && canAnnotate() && annotId > 0;
  const button = document.createElement("button"); button.type = "button";
  button.className = `hit-target link-target${item.kind === "blocked" ? " blocked" : ""}${editing ? " editable" : ""}`;
  button.dataset.annotId = String(annotId); button.dataset.linkId = String(item.id);
  button.setAttribute("role", "link"); button.setAttribute("aria-label", linkLabel(item));
  button.title = item.kind === "page" ? `Page ${(item.page ?? 0) + 1}` : item.kind === "uri" ? item.uri ?? "" : `Blocked scheme ${schemeName(item.uri ?? "")}`;
  placeBox(button, rect, shownZoom);
  if (editing) {
    const start = (event: PointerEvent, handle: string) => startRectGesture(event, { rect, id: annotId, handle, min: 4, lockAspect: false, can: canAnnotate, commit: next => commitLinkRect(item, page, next) });
    addFieldHandles(button, start);
    button.addEventListener("pointerdown", event => { if (event.button === 0 && !event.ctrlKey && !(event.target as HTMLElement).closest(".hit-handle")) start(event, "grip"); });
    button.addEventListener("dblclick", event => { event.preventDefault(); if (!directGestureWork) openLinkEditor(item, page); });
  }
  button.addEventListener("click", event => {
    event.preventDefault();
    if ((event.target as HTMLElement).closest(".hit-handle") || event.ctrlKey) return;
    // P6 U7 §9.21/§10.29: in Annotate a link (annotId > 0) is a selection member; Shift-click adds or removes it.
    if (uiMode === "annotate" && annotId > 0 && event.shiftKey) { directClickWork = shiftSelect(linkMember(item)); return; }
    if (uiMode === "read") followLink(item);
    else if (uiMode === "annotate") { focusLink(item, button); if (annotId > 0) { setSelection([linkMember(item)]); markSelection(); } }
    directClickWork = Promise.resolve();
  });
  button.addEventListener("keydown", event => {
    if (uiMode !== "annotate") return;
    if (event.key === "Escape") { event.preventDefault(); dropDirectFocus(); return; }
    const size = doc?.pages[page];
    let work: (() => Promise<unknown>) | null = null;
    if (ARROW_KEYS.includes(event.key) && (event.altKey || (!event.metaKey && !event.ctrlKey))) {
      if (cmdKey(event) && (event.key === "ArrowUp" || event.key === "ArrowDown")) return;
      if (size && editing) { const next = nudgeRect(rect, event, 4, size); work = () => commitLinkRect(item, page, next); }
    } else if (event.key === "Delete" || event.key === "Backspace") { if (canAnnotate()) work = () => deleteLink(item.id); }
    else if (event.key === "Enter") { event.preventDefault(); if (!directGestureWork && editing) openLinkEditor(item, page); return; }
    else if (event.key !== " ") return;
    event.preventDefault();
    if (work && !directGestureWork) runGestureWork(work);
  });
  return button;
}

/** §1.5: the value the inline target editor opens with: a 1-based page, or the URI (blocked ones too). */
function linkEditorValue(item: LinkEntry): string {
  return item.kind === "page" ? String((item.page ?? 0) + 1) : item.uri ?? "";
}

function openLinkEditor(item: LinkEntry, page: number): void {
  if (!doc || !canAnnotate() || linkAnnotId(item) <= 0) return;
  clearError(); closeDirect();
  const editor = document.createElement("input"); editor.type = "text";
  editor.className = "inline-editor link-editor"; editor.maxLength = MAX_URI_CHARS; editor.spellcheck = false; editor.autocomplete = "off";
  editor.value = linkEditorValue(item); editor.dataset.initial = editor.value;
  editor.setAttribute("aria-label", `Link target (page number or web address): ${linkLabel(item)}`);
  editor.title = "Enter a page number or a web address; Enter saves, Escape cancels";
  placeBox(editor, item.rect, shownZoom); hitLayer.append(editor);
  directFocusedLink = { annotId: linkAnnotId(item) }; directFocusedObject = null; directFocusedMark = null;
  directActive = { kind: "link", item, editor, page };
  setupInlineEvents(editor, false); editor.focus(); editor.select();
  syncLinkCurrent();
}

/** §1.5: digits → a page (checked like the dialog), anything else → a URI; a refusal keeps the editor and its draft. */
/** `silent` (§9.6, a mode or page change): a refusal drops the draft without #error. */
async function commitLinkEditor(active: NonNullable<typeof directActive>, silent = false): Promise<boolean> {
  const item = links.find(entry => linkAnnotId(entry) === linkAnnotId(active.item as LinkEntry)) ?? active.item as LinkEntry, value = active.editor.value.trim();
  if (!doc) return false;
  if (/^\d+$/.test(value)) {
    const number = Number(value);
    if (number < 1 || number > doc.pageCount) { if (!silent) showError(`Enter a page number between 1 and ${doc.pageCount}`); return false; }
    return linkSet(active.page, item, { target: { page: number - 1 } }, `Link now goes to page ${number}`, silent);
  }
  return linkSet(active.page, item, { target: { uri: value.slice(0, MAX_URI_CHARS) } }, "Link now opens a web address", silent);
}

/** §1.8: STATE `direct.active` for a focused or edited link. */
function linkState(item: LinkEntry, box: ReturnType<typeof cssBox>, editing: boolean, draft?: string): Record<string, unknown> {
  return {
    kind: "link", annotId: linkAnnotId(item), id: item.id, page: wantedPage, box, rect: item.rect,
    target: item.kind === "page" ? { kind: "page", page: item.page ?? 0 } : { kind: item.kind, uri: item.uri ?? "" },
    editing, ...(draft !== undefined ? { draft } : {}),
  };
}

/**
 * `--edit=nav`: bookmark the last page, link the search hit "one" (page 1) to page 3,
 * delete page 2, then log. The repair that delete runs is what the panels show.
 */
async function smokeNav(contents: string | null) {
  if (!doc) {
    logEdit();
    return;
  }
  const title = contents ?? "PDF4All P2 nav";
  await addBookmark(title, doc.pageCount - 1);
  searchInput.value = "one";
  await startSearch();
  const hit = searchHits.find(item => item.page === 0) ?? searchHits[0];
  const rect = hit && hitRect(hit);
  if (hit && rect) await addLink(hit.page, rect, { page: 2 });
  await deletePage(1);
  await loadAllLinks();
  logEdit();
  // The blocked link's own activation: main re-checks the allowlist and logs
  // P2_LINK with allowed:false rather than opening anything.
  const blocked = links.find(item => item.kind === "blocked");
  if (blocked?.uri) api?.openLink?.(blocked.uri);
}

function annotSummary(item: AnnotItem): string {
  const text = item.contents.replace(/\s+/g, " ").trim();
  if (text) return text.length > 60 ? `${text.slice(0, 60)}…` : text;
  return item.type === "Text" ? "(empty note)" : item.type === "FreeText" ? "(empty text box)" : "(no comment)";
}

function renderNotes() {
  notesHeading.textContent = `Annotations on page ${wantedPage + 1}`;
  const fragment = document.createDocumentFragment();
  for (const item of notes) {
    const row = document.createElement("li");
    const label = annotSummary(item);
    const name = annotName(item);
    if (!name) {
      row.textContent = `${item.type} (not editable in this version)`;
      fragment.append(row);
      continue;
    }
    const text = document.createElement("span");
    text.className = "note-text";
    text.textContent = `${name}: ${label}`;
    text.title = item.contents;
    row.append(text);
    if (item.editable || item.type === "Redact") {
      if (item.editable) {
        const edit = document.createElement("button");
        edit.type = "button";
        edit.dataset.action = "edit";
        edit.dataset.annotId = String(item.id);
        edit.textContent = "Edit";
        edit.setAttribute("aria-label", `Edit ${name.toLowerCase()}: ${label}`);
        row.append(edit);
      }
      const remove = document.createElement("button");
      remove.type = "button";
      remove.dataset.action = "delete";
      remove.dataset.annotId = String(item.id);
      remove.textContent = "Delete";
      remove.setAttribute("aria-label", `Delete ${name.toLowerCase()}: ${label}`);
      row.append(remove);
    }
    fragment.append(row);
  }
  noteListEl.replaceChildren(fragment);
  syncNoteCurrent();
  setEmpty(notesEmptyEl, notes.length, "No annotations on this page yet.", "No document open yet.");
}

/** Adds a 20x20 Text note whose top-left is `point` (PDF points, y down from the page top). */
async function addNoteAt(page: number, point: [number, number], contents: string, announcement = "Added note"): Promise<boolean> {
  if (!doc || !canAnnotate()) {
    showError("This document does not permit annotations.");
    return false;
  }
  try {
    const reply = (await request({
      op: "annot-add", page, type: "Text", point, contents,
    })) as MutationReply;
    await applyEdit(reply, announcement);
    return true;
  } catch (err) {
    showError(messageOf(err));
    return false;
  }
}

/** The current search match, but only when it is on the page being shown. */
function currentHit(): SearchHit | null {
  if (searchIndex < 0 || searchIndex >= searchHits.length) return null;
  const hit = searchHits[searchIndex];
  return hit.page === wantedPage ? hit : null;
}

/** Markup over the given search-hit quads; the comment starts empty and is edited from the list. */
async function addMarkup(page: number, type: "Highlight" | "Underline" | "StrikeOut", quads: number[][], announcement: string): Promise<boolean> {
  if (!doc || !canAnnotate()) {
    showError("This document does not permit annotations.");
    return false;
  }
  try {
    const reply = (await request({ op: "annot-add", page, type, quads, contents: "" })) as MutationReply;
    await applyEdit(reply, announcement);
    return true;
  } catch (err) {
    showError(messageOf(err));
    return false;
  }
}

/** Adds a 160x40 text box whose top-left is `point` (PDF points, y down from the page top). */
async function addTextBoxAt(page: number, point: [number, number], contents: string): Promise<boolean> {
  if (!doc || !canAnnotate()) {
    showError("This document does not permit annotations.");
    return false;
  }
  try {
    const reply = (await request({ op: "annot-add", page, type: "FreeText", point, contents })) as MutationReply;
    await applyEdit(reply, "Added text box");
    return true;
  } catch (err) {
    showError(messageOf(err));
    return false;
  }
}

// ------------------------------------------------- shapes and stamps (unit 9)

/** A `width`x`height` box with its top-left at `point`, kept inside the page box (the FreeText rule). */
function boxAt(point: [number, number], width: number, height: number): number[] | null {
  const size = doc?.pages[wantedPage];
  if (!size) return null;
  const w = Math.min(round2(width), size.width);
  const h = Math.min(round2(height), size.height);
  const x = Math.max(0, Math.min(round2(point[0]), size.width - w));
  const y = Math.max(0, Math.min(round2(point[1]), size.height - h));
  // The far corner is clamped to the page too: a hit at the edge must not round past it.
  return [x, y, Math.min(x + w, size.width), Math.min(y + h, size.height)];
}

/** The current hit's quad union padded 4 pt, kept inside the page box; null with no hit shown. */
function hitBox(): number[] | null {
  const hit = currentHit();
  if (!hit) return null;
  const rect = hitRect(hit);
  return rect ? boxAt([rect[0] - 4, rect[1] - 4], rect[2] - rect[0] + 8, rect[3] - rect[1] + 8) : null;
}

/** Where a Square/Circle/Line/Ink/icon-Stamp goes: the hit box, else a SHAPE_SIZE box at the page centre. */
function shapeBox(): number[] | null {
  const anchored = hitBox();
  if (anchored) return anchored;
  const size = doc?.pages[wantedPage];
  return size ? boxAt([size.width / 2, size.height / 2], SHAPE_SIZE[0], SHAPE_SIZE[1]) : null;
}

/** The signature's own box: SIGNATURE_SIZE centred on the hit, else on the page centre. */
function signatureBox(): number[] | null {
  const size = doc?.pages[wantedPage];
  if (!size) return null;
  const hit = hitBox();
  const centre: [number, number] = hit
    ? [(hit[0] + hit[2]) / 2, (hit[1] + hit[3]) / 2]
    : [size.width / 2, size.height / 2];
  const [width, height] = SIGNATURE_SIZE;
  return boxAt([centre[0] - width / 2, centre[1] - height / 2], width, height);
}

/** A drag point kept inside the page box: a pointer on the very edge must not round past it. */
function inkPoint(event: PointerEvent): [number, number] | null {
  const size = doc?.pages[wantedPage];
  const point = pagePoint(event);
  if (!size || !point) return null;
  return [Math.max(0, Math.min(point[0], size.width)), Math.max(0, Math.min(point[1], size.height))];
}

/** The stand-in Ink the keyboard path draws: a 4-point zigzag along the box's diagonal. */
function inkZigzag(box: number[]): [number, number][] {
  const [x0, y0, x1, y1] = box;
  return [
    [x0, y0],
    [x0 + (x1 - x0) / 3, y0 + ((y1 - y0) * 2) / 3],
    [x0 + ((x1 - x0) * 2) / 3, y0 + (y1 - y0) / 3],
    [x1, y1],
  ];
}

/** True (and announced) when the open document cannot take an annotation. */
function refusesAdd(): boolean {
  if (doc && editable()) return false;
  showError("This document is read-only, so annotations cannot be added.");
  return true;
}

/** One Square/Circle (its rect) or Line (left-mid → right-mid of the box). */
async function addShape(type: "Square" | "Circle" | "Line", box: number[], announcement: string): Promise<boolean> {
  if (refusesAdd()) return false;
  const middle = (box[1] + box[3]) / 2;
  const payload = type === "Line"
    ? { op: "annot-add", page: wantedPage, type, points: [[box[0], middle], [box[2], middle]], contents: "" }
    : { op: "annot-add", page: wantedPage, type, rect: box, contents: "" };
  try {
    const reply = (await request(payload)) as MutationReply;
    await applyEdit(reply, announcement);
    return true;
  } catch (err) {
    showError(messageOf(err));
    return false;
  }
}

/** One Ink with the strokes a pointer drag (or the keyboard stand-in) recorded. */
async function addInk(strokes: number[][][], announcement: string): Promise<boolean> {
  if (refusesAdd()) return false;
  try {
    const reply = (await request({ op: "annot-add", page: wantedPage, type: "Ink", strokes, contents: "" })) as MutationReply;
    await applyEdit(reply, announcement);
    return true;
  } catch (err) {
    showError(messageOf(err));
    return false;
  }
}

/** One icon stamp; mupdf synthesises the appearance for the six icons it knows. */
async function addIconStamp(box: number[], icon: string): Promise<boolean> {
  if (refusesAdd()) return false;
  try {
    const reply = (await request({
      op: "annot-add", page: wantedPage, type: "Stamp", rect: box, icon, contents: "",
    })) as MutationReply;
    await applyEdit(reply, `Added stamp ${icon}`);
    return true;
  } catch (err) {
    showError(messageOf(err));
    return false;
  }
}

/** One signature appearance: an image stamp whose `/Subj` says it is a picture, not a signature. */
async function addSignature(box: number[], name: string, image: { name: string; bytes: ArrayBuffer }): Promise<boolean> {
  if (refusesAdd()) return false;
  try {
    const reply = (await request({
      op: "annot-add", page: wantedPage, type: "Stamp", rect: box, image, sign: { name }, contents: "",
    })) as MutationReply;
    await applyEdit(reply, "Added signature appearance (a picture, not a certificate signature)");
    return true;
  } catch (err) {
    showError(messageOf(err));
    return false;
  }
}

/** The Insert menu's four geometric shapes: no dialog, one add each. */
async function addShapeOp(op: "square" | "circle" | "line" | "ink") {
  if (!doc || !canAnnotate()) return;
  const box = shapeBox();
  if (!box) return;
  if (op === "ink") await addInk([inkZigzag(box)], "Added ink (4 points)");
  else if (op === "line") await addShape("Line", box, "Added line");
  else if (op === "circle") await addShape("Circle", box, "Added circle");
  else await addShape("Square", box, "Added square");
}

function openStampIconDialog() {
  if (!doc || !canAnnotate()) return;
  stampIconDialog.showModal();
  stampIconDialog.querySelector<HTMLInputElement>("input[name=stamp-icon]:checked")?.focus();
}

async function saveStampIconDialog() {
  const icon = stampIconDialog.querySelector<HTMLInputElement>("input[name=stamp-icon]:checked")?.value;
  const box = shapeBox();
  stampIconDialog.close();
  if (icon && box) await addIconStamp(box, icon);
}

/**
 * `--edit=shapes`: search `one`, then add one of each new type — Square, Circle,
 * Line and a keyboard Ink over the hit, a Draft icon stamp, and a signature
 * appearance from `--images`. The signature is last, so one undo leaves one Stamp.
 */
async function smokeShapes(contents: string | null) {
  if (!doc) {
    logEdit();
    return;
  }
  const name = contents ?? "PDF4All P2 shapes";
  searchInput.value = "one";
  await startSearch();
  const box = shapeBox();
  if (box) {
    await addShape("Square", box, "Added square");
    await addShape("Circle", box, "Added circle");
    await addShape("Line", box, "Added line");
    await addInk([inkZigzag(box)], "Added ink (4 points)");
  }
  if (box) await addIconStamp(box, "Draft");
  let picked;
  try {
    picked = await api?.pickImages?.();
  } catch {
    picked = undefined;
  }
  const image = picked?.ok && picked.images.length ? picked.images[0] : null;
  const signature = signatureBox();
  if (image && signature) await addSignature(signature, name, image);
  logEdit();
}

async function editNote(annotId: number, contents: string) {
  try {
    const reply = (await request({ op: "annot-set", page: wantedPage, annotId, contents })) as MutationReply;
    await applyEdit(reply, "Edited note");
  } catch (err) {
    showError(messageOf(err));
  }
}

async function deleteNote(annotId: number, announcement = "Deleted note") {
  try {
    const reply = (await request({ op: "annot-delete", page: wantedPage, annotId })) as MutationReply;
    await applyEdit(reply, announcement);
  } catch (err) {
    showError(messageOf(err));
  }
}

async function undoEdit() {
  if (directActive) { closeDirect(); directFocusedField = null; return; }
  // P6 U5 §9.28: the selection survives; the reply's repaint re-resolves each member (restoreDirectFocus).
  directFocusedField = null; directFocusedMark = null; directFocusedLink = null; closeObjectMenu(false);
  if (!canUndo) return;
  clearError(); // Bugfix 1 D5: a performed undo clears the error bar; a failing one shows its own.
  try {
    const reply = (await request({ op: "undo" })) as PageReply;
    // A reverted page op moved the page tree, so the same full refresh applies.
    if (PAGE_STEPS.has(reply.step ?? "")) {
      await applyPageEdit(reply, `Undid ${reply.step}`, wantedPage);
      await refreshLabels();
    } else {
      await applyEdit(reply, "Undid the last change");
      if (LABEL_STEPS.has(reply.step ?? "")) await refreshLabels();
    }
    refreshDecorateSummary();
  } catch (err) {
    showError(messageOf(err));
  }
}

async function redoEdit() {
  if (directActive) { closeDirect(); directFocusedField = null; return; }
  directFocusedField = null; directFocusedMark = null; directFocusedLink = null; closeObjectMenu(false);
  if (!canRedo) return;
  clearError(); // Bugfix 1 D5
  try {
    const reply = (await request({ op: "redo" })) as PageReply;
    if (PAGE_STEPS.has(reply.step ?? "")) {
      await applyPageEdit(reply, `Redid ${reply.step}`, wantedPage);
      await refreshLabels();
    } else {
      await applyEdit(reply, "Redid the last change");
      if (LABEL_STEPS.has(reply.step ?? "")) await refreshLabels();
    }
    refreshDecorateSummary();
  } catch (err) {
    showError(messageOf(err));
  }
}

/** The menu/smoke path for a note: the current page centre, with the payload's text when sent. */
async function smokeNote(contents: string) {
  const size = doc?.pages[wantedPage];
  if (!size) return;
  await addNoteAt(wantedPage, [size.width / 2 - NOTE_SIZE / 2, size.height / 2 - NOTE_SIZE / 2], contents);
  logEdit();
}

/**
 * The menu/smoke path for a highlight. The smoke sends `contents`, so a cold run
 * first finds hello.pdf's only word; the menu sends none and only acts on a hit
 * the user already has.
 */
async function smokeHighlight(contents: string | null) {
  if (!doc) return;
  let hit = currentHit();
  if (!hit && contents !== null) {
    searchInput.value = "PDF4All";
    await startSearch();
    hit = currentHit();
  }
  if (!hit) {
    announce("Find something first");
    return;
  }
  await addMarkup(hit.page, "Highlight", hit.quads, "Highlighted match");
  logEdit();
}

/**
 * The menu opens the dialog for a typed box; the smoke supplies the text and lands
 * a box at the page centre without touching the dialog.
 */
async function smokeFreeText(contents: string | null) {
  const size = doc?.pages[wantedPage];
  if (!size) return;
  const point: [number, number] = [size.width / 2, size.height / 2];
  if (contents === null) {
    openTextBoxDialog(point);
    return;
  }
  await addTextBoxAt(wantedPage, point, contents);
  logEdit();
}

/** `edit-request {op:'undo'|'redo'}` from main's menu or the smoke driver. */
async function smokeUndo() {
  if (directActive) { closeDirect(); directFocusedField = null; return; } // §8.7: editor open — Esc, journal untouched
  await undoEdit();
  logEdit();
  if (contentSmokeActive) logContentSmoke();
}

async function smokeRedo() {
  if (directActive) { closeDirect(); directFocusedField = null; return; }
  await redoEdit();
  logEdit();
  if (contentSmokeActive) logContentSmoke();
}

// ------------------------------------------------------------ assemble (unit 6)

function announcePage(text: string) {
  pageStatusEl.textContent = text;
}

/** The picked source, or null (a cancel is not an error). */
async function pickInsertSource(action: "insert" | "compare" = "insert"): Promise<{ name: string; bytes: ArrayBuffer } | null> {
  if (!api || typeof api.pickInsertSource !== "function") {
    showError(`${action === "compare" ? "Comparing" : "Inserting pages"} needs the desktop app (window.pdf4all is unavailable).`);
    return null;
  }
  const picked = await api.pickInsertSource();
  if (!picked.ok) {
    if (picked.reason !== "cancelled") {
      showError(picked.reason === "too-large"
        ? `That PDF is larger than the ${action} limit.`
        : "Could not read that PDF.");
    }
    return null;
  }
  if (!(picked.bytes instanceof ArrayBuffer) || picked.bytes.byteLength < 5) {
    showError("That PDF is not readable.");
    return null;
  }
  return { name: picked.name, bytes: picked.bytes };
}

/** "Insert pages…": pick a source, then ask where its pages go and which ones. */
async function requestInsert() {
  if (!doc || !canEdit()) return;
  const picked = await pickInsertSource();
  if (!picked) return;
  openInsertDialog(picked);
}

// ----------------------------------------------------------------- compare (contract-compare.md)

/** "Compare with…" (⌥⌘K) and the drive's `compare` with a line-level `from`: pick, then `compare-open`. */
async function requestCompare(): Promise<void> {
  clearError();
  const picked = await pickInsertSource("compare");
  if (!picked) return;
  await openCompare(picked.bytes, picked.name);
}

/** The bytes are transferred to the worker; no JS copy is kept (§9.18). */
async function openCompare(bytes: ArrayBuffer, name: string): Promise<void> {
  let reply: CompareOpenReply;
  try {
    reply = (await request({ op: "compare-open", pdf: bytes, name }, [bytes])) as CompareOpenReply;
  } catch (err) {
    // A refusal keeps the previous slot (§9.7).
    compareLast = { action: "open", ok: false, detail: (err as { reason?: string }).reason ?? "unreadable" };
    showError(`Could not compare with ${name}: ${messageOf(err)}`);
    return;
  }
  compareDoc = { name: reply.name, pageCount: reply.pageCount, pages: reply.pages, pageA: wantedPage, pageB: 0, text: null, pixels: null };
  compareB = null;
  compareLast = { action: "open", ok: true };
  clearError(); // §9.19(c)
  setCompareShowDiff(false); // §9.13: unchecked on every open
  comparePageBInput.max = String(reply.pageCount);
  readerEl.hidden = true;
  compareView.hidden = false;
  compareView.focus();
  await refreshCompare(true);
}

/** Drops the renderer's compare state and shows #reader again; the worker slot is the caller's business. */
function clearCompare(): void {
  compareGen += 1;
  compareDoc = null;
  compareB = null;
  setCompareShowDiff(false);
  compareSummaryEl.textContent = "";
  compareListEl.tBodies[0]?.replaceChildren();
  for (const canvas of [compareCanvasA, compareCanvasB, compareOverlay]) { canvas.width = 0; canvas.height = 0; }
  if (!compareView.hidden) {
    compareView.hidden = true;
    readerEl.hidden = false;
    syncZoom();
    void drain();
  }
}

async function closeCompare(): Promise<void> {
  try {
    await request({ op: "compare-close" });
  } catch {
    // A dead worker already dropped the slot.
  }
  clearCompare();
  compareLast = { action: "close", ok: true };
  announce("Compare closed");
}

function setCompareShowDiff(on: boolean): void {
  compareShowDiff = on;
  compareShowDiffInput.checked = on;
  compareOverlay.hidden = !on;
}

/** §9.12: a hand-set page of B (0-based here, 1-based in #compare-page-b), clamped; holds until A's page changes. */
function setComparePageB(page: number): Promise<void> {
  if (!compareDoc) return compareWork;
  compareDoc.pageB = Math.min(Math.max(0, Math.trunc(page)), compareDoc.pageCount - 1);
  return refreshCompare();
}

/**
 * Runs at the end of goTo/applyEdit/applyPageEdit/openDocument while compare is open. `speak` false
 * (after an edit) keeps that edit's own announcement; the summary text is still updated.
 */
function refreshCompare(resetB = false, speak = true): Promise<void> {
  const c = compareDoc;
  if (!c) return compareWork;
  const gen = ++compareGen;
  if (resetB || wantedPage !== c.pageA) {
    c.pageA = wantedPage;
    c.pageB = Math.min(wantedPage, c.pageCount - 1);
  }
  comparePageBInput.value = String(c.pageB + 1);
  // §9.19(g): each side names its file.
  compareNameA.textContent = doc?.name ?? "";
  compareNameB.textContent = c.name;
  compareWork = runCompare(c, gen, speak).catch(err => {
    if (gen === compareGen) showError(messageOf(err));
  });
  return compareWork;
}

async function runCompare(c: NonNullable<typeof compareDoc>, gen: number, speak: boolean): Promise<void> {
  if (!doc || doc.info.needsPassword) {
    // §9.19(h): a locked main shows nothing of the previous document; applyUnlocked refreshes.
    c.text = null;
    c.pixels = null;
    compareSummaryEl.textContent = "";
    compareListEl.tBodies[0]?.replaceChildren();
    for (const canvas of [compareCanvasA, compareCanvasB, compareOverlay]) { canvas.width = 0; canvas.height = 0; }
    return;
  }
  const pageA = Math.min(c.pageA, doc.pageCount - 1);
  const pageB = c.pageB;
  const sa = doc.pages[pageA];
  const sb = c.pages[pageB];
  // §9.8: one deterministic scale from both pages' pt sizes (no zoom, no dpr).
  const S = Math.min(1, Math.floor(MAX_PIXEL_EDGE / Math.max(sa.width, sa.height, sb.width, sb.height) * 100) / 100);
  const keyA = `${pageA}:${S}:${revision}`;
  const a = cache.get(keyA) ?? (await request({ op: "render", page: pageA, scale: S }) as RenderReply);
  if (gen !== compareGen) return;
  if (a.revision === revision) cache.set(keyA, a);
  const keyB = `cmp:${pageB}:${S}`;
  const b = compareB?.key === keyB ? compareB.reply : (await request({ op: "compare-render", page: pageB, scale: S }) as CompareRenderReply);
  if (gen !== compareGen) return;
  compareB = { key: keyB, reply: b };
  let text: CompareText;
  let lines: { side: "Only left" | "Only right"; page: number; line: CompareLine }[] = [];
  try {
    const diff = (await request({ op: "compare-text", pageA, pageB })) as CompareTextReply;
    text = {
      same: diff.same, removed: diff.removed.length, added: diff.added.length,
      removedLines: diff.removed.slice(0, 50).map(line => normaliseLine(line.text)),
      addedLines: diff.added.slice(0, 50).map(line => normaliseLine(line.text)),
    };
    lines = [
      ...diff.removed.map(line => ({ side: "Only left" as const, page: pageA, line })),
      ...diff.added.map(line => ({ side: "Only right" as const, page: pageB, line })),
    ];
  } catch (err) {
    const reason = (err as { reason?: string }).reason;
    if (!reason) throw err;
    text = { same: 0, removed: 0, added: 0, refused: reason, removedLines: [], addedLines: [] };
  }
  if (gen !== compareGen) return;
  const pixels = paintCompare(a, b, S);
  c.text = text;
  c.pixels = pixels;
  const body = compareListEl.tBodies[0];
  body.replaceChildren(...lines.map(({ side, page, line }) => {
    const row = document.createElement("tr");
    row.title = `Page ${page + 1}, ${line.bbox.map(v => Math.round(v)).join(" ")} pt`;
    for (const value of [side, line.text]) {
      const cell = document.createElement("td");
      cell.textContent = value;
      row.append(cell);
    }
    return row;
  }));
  const percent = `${(pixels.ratio * 100).toFixed(1)} % of pixels differ`;
  // §9.19(f): each refusal reason in its own words.
  const refusal = text.refused === "too-many" ? "more than 2000 lines on a page, only pixels compared"
    : text.refused === "read-only" ? "copying is not permitted, only pixels compared"
    : `text not compared (${text.refused}), only pixels compared`;
  const summary = text.refused
    ? `Page ${pageA + 1} vs ${pageB + 1}: ${refusal}; ${percent}`
    : `Page ${pageA + 1} vs ${pageB + 1}: ${text.same} lines same, ${text.removed} removed, ${text.added} added; ${percent}`;
  compareSummaryEl.textContent = summary;
  if (speak) announce(summary);
}

/**
 * D3/§9.8: diff the raw RGBA buffers on a maxW × maxH grid (white outside a page), paint A and B,
 * and paint the differing pixels inside B red at 60 % into a fresh B-sized overlay (never into the cached A buffer).
 * ponytail: no anti-aliasing tolerance beyond the threshold, no alignment search.
 */
function paintCompare(a: RenderResult, b: RenderResult, scale: number): ComparePixels {
  const W = Math.max(a.pixelWidth, b.pixelWidth);
  const H = Math.max(a.pixelHeight, b.pixelHeight);
  const pa = new Uint8Array(a.pixels);
  const pb = new Uint8Array(b.pixels);
  // §9.19(e): the overlay is B's pixel grid (it sits on B); differing/total still count maxW × maxH.
  const overlay = new ImageData(b.pixelWidth, b.pixelHeight);
  const out = overlay.data;
  let differing = 0;
  for (let y = 0; y < H; y++) {
    for (let x = 0; x < W; x++) {
      const inA = x < a.pixelWidth && y < a.pixelHeight;
      const inB = x < b.pixelWidth && y < b.pixelHeight;
      const ia = (y * a.pixelWidth + x) * 4;
      const ib = (y * b.pixelWidth + x) * 4;
      const delta = Math.abs((inA ? pa[ia] : 255) - (inB ? pb[ib] : 255))
        + Math.abs((inA ? pa[ia + 1] : 255) - (inB ? pb[ib + 1] : 255))
        + Math.abs((inA ? pa[ia + 2] : 255) - (inB ? pb[ib + 2] : 255));
      if (delta > 48) {
        differing++;
        if (inB) {
          out[ib] = 255;
          out[ib + 3] = 153;
        }
      }
    }
  }
  const put = (canvas: HTMLCanvasElement, image: ImageData) => {
    canvas.width = image.width;
    canvas.height = image.height;
    canvas.getContext("2d")?.putImageData(image, 0, 0);
  };
  put(compareCanvasA, new ImageData(new Uint8ClampedArray(a.pixels), a.pixelWidth, a.pixelHeight));
  put(compareCanvasB, new ImageData(new Uint8ClampedArray(b.pixels), b.pixelWidth, b.pixelHeight));
  put(compareOverlay, overlay);
  const total = W * H;
  return { differing, total, ratio: Math.round(differing / total * 10000) / 10000, scale };
}

/** Drive `compare`: `{close}`, `{pageB}`, `{page}` or `{showDiff}` (the open step has no contents, §9.10). */
async function driveCompare(contents: string): Promise<void> {
  clearError();
  const wanted = driveWanted(contents);
  if (!wanted) return;
  if (wanted.close === true) return closeCompare();
  if (!compareDoc) {
    showError("compare: no compare document is open");
    return;
  }
  if (typeof wanted.showDiff === "boolean") {
    setCompareShowDiff(wanted.showDiff);
    return;
  }
  if (wanted.pageB !== undefined) {
    if (!Number.isInteger(wanted.pageB)) {
      showError("compare: pageB must be an integer");
      return;
    }
    await setComparePageB(wanted.pageB as number);
    return;
  }
  if (wanted.page !== undefined) {
    await driveGoto(JSON.stringify({ page: wanted.page }));
    return;
  }
  showError("compare: contents must be {close}, {pageB}, {page} or {showDiff}");
}

function compareSummary() {
  return {
    open: !!compareDoc,
    name: compareDoc?.name ?? null,
    pageCount: compareDoc?.pageCount ?? null,
    pageA: compareDoc?.pageA ?? 0,
    pageB: compareDoc?.pageB ?? 0,
    text: compareDoc?.text ?? null,
    pixels: compareDoc?.pixels ?? null,
    showDiff: compareShowDiff,
    last: compareLast,
  };
}

function openInsertDialog(source: { name: string; bytes: ArrayBuffer }) {
  insertSource = source;
  insertSourceEl.textContent = `Insert pages from ${source.name}`;
  // The number is the 1-based page the new pages follow; 0 puts them first.
  insertAfterInput.value = String(wantedPage + 1);
  insertAfterInput.max = String(doc?.pageCount ?? 1);
  insertRangeInput.value = "";
  insertDialog.showModal();
  insertAfterInput.focus();
  insertAfterInput.select();
}

async function saveInsertDialog(): Promise<boolean> {
  const source = insertSource;
  if (!doc || !source) {
    insertDialog.close();
    return false;
  }
  const at = Number(insertAfterInput.value);
  if (!Number.isInteger(at) || at < 0 || at > doc.pageCount) {
    showError(`Enter a page number between 0 (before the first) and ${doc.pageCount}`);
    insertAfterInput.focus();
    return false;
  }
  insertDialog.close();
  return insertPages(source, at, insertRangeInput.value);
}

/** One `insert-pages`; the range text is parsed by the engine, which knows the source count. */
async function insertPages(source: { name: string; bytes: ArrayBuffer }, at: number, range: string): Promise<boolean> {
  try {
    const reply = (await request({
      op: "insert-pages", pdf: source.bytes, name: source.name, at, range,
    }, [source.bytes], SAVE_TIMEOUT_MS)) as InsertReply;
    const inserted = reply.inserted;
    assembleSummary = { inserted, source: source.name, fieldNames: reply.fieldNames ?? [], last: { action: "insert", ok: true } };
    await applyPageEdit(reply, "", Math.min(Math.max(at, 0), reply.pageCount - 1));
    announcePage(inserted === 1
      ? (at === 0 ? "Inserted 1 page before page 1" : `Inserted 1 page after page ${at}`)
      : (at === 0 ? `Inserted ${inserted} pages before page 1` : `Inserted ${inserted} pages after page ${at}`));
    return true;
  } catch (err) {
    showError(messageOf(err));
    // A2 §8.15: a refusal leaves the summary (created if needed) with only its `last`.
    assembleSummary = { ...(assembleSummary ?? {}), last: { action: "insert", ok: false, detail: refusalDetail(err) } };
    return false;
  }
}

/** "Extract pages…": ask which pages, then write them to a new file. */
function requestExtract() {
  if (!doc || doc.info.needsPassword) return;
  extractRangeInput.value = String(wantedPage + 1);
  extractDialog.showModal();
  extractRangeInput.focus();
  extractRangeInput.select();
}

async function saveExtractDialog() {
  const text = extractRangeInput.value;
  extractDialog.close();
  await extractPages(text);
}

/** `stem pages 2-4.pdf`, the page list compacted into ascending ranges. */
function extractFileName(name: string, pages: number[]): string {
  const stem = name.replace(/\.pdf$/i, "") || "document";
  const sorted = [...new Set(pages)].map(page => page + 1).sort((a, b) => a - b);
  const parts: string[] = [];
  for (let index = 0; index < sorted.length; index++) {
    const start = sorted[index];
    let end = start;
    while (index + 1 < sorted.length && sorted[index + 1] === end + 1) {
      end = sorted[++index];
    }
    parts.push(start === end ? String(start) : `${start}-${end}`);
  }
  return `${stem} pages ${parts.join(", ")}.pdf`;
}

/** One `extract-pages` plus the save; the open document's own bytes never change. */
async function extractPages(range: string): Promise<boolean> {
  if (!doc) return false;
  const pages = parsePageRange(range, doc.pageCount);
  if (!pages || !pages.length) {
    showError(`Pages must look like "1-3, 5" (this document has ${doc.pageCount})`);
    return false;
  }
  if (!api || typeof api.saveFile !== "function") {
    showError("Saving needs the desktop app (window.pdf4all is unavailable).");
    return false;
  }
  if (!doc.url) {
    showError("This is the built-in fixture, not a file on disk; open a file to extract from.");
    return false;
  }
  extractSummary = null;
  const name = extractFileName(doc.name, pages);
  try {
    const reply = (await request({ op: "extract-pages", pages }, [], SAVE_TIMEOUT_MS)) as ExtractReply;
    if (!(reply.bytes instanceof ArrayBuffer) || reply.bytes.byteLength < 5) throw new Error("extract-pages: no bytes");
    const byteLength = reply.bytes.byteLength; // read before main may take the buffer
    const result = await api.saveFile(reply.bytes, doc.url, { suggestedName: name });
    if (!result.ok) {
      showError(result.reason === "cancelled" ? "" : `The extract was not saved (${result.reason}).`);
      if (result.reason === "cancelled") clearError();
      return false;
    }
    extractSummary = { pages, bytes: byteLength };
    announcePage(`Extracted ${pages.length === 1 ? "1 page" : `${pages.length} pages`} to ${result.name}`);
    return true;
  } catch (err) {
    showError(messageOf(err));
    return false;
  }
}

/**
 * `--edit=assemble`: insert every page of the picked source after page 1 and log.
 * `--insert-from` is what answers the picker in smoke mode.
 */
async function smokeAssemble() {
  if (!doc) {
    logEdit();
    return;
  }
  const source = await pickInsertSource();
  if (source) await insertPages(source, 1, "");
  logEdit();
}

/**
 * `--edit=extract`: page 2 on its own, written out and logged. The bytes go to main
 * through the same `saveFile` the dialog uses, which `--save-to` answers in smoke mode.
 */
async function smokeExtract() {
  if (!doc) {
    logEdit();
    return;
  }
  await extractPages(String(Math.min(2, doc.pageCount)));
  logEdit();
}

// --------------------------------------------------------------- exports (unit 7)

/** File stem for a suggested name: `nav.pdf` to `nav`. */
function fileStem(name: string): string {
  return name.replace(/\.[A-Za-z0-9]+$/, "") || "document";
}

/**
 * The one place a new file leaves the app. main owns the dialog, the extension and
 * the write; a cancel is not an error and reads as `null`.
 */
async function saveNewFile(bytes: ArrayBuffer, suggestedName: string, kind: "pdf" | "png" | "jpeg" | "txt", open = false): Promise<string | null> {
  if (!api || typeof api.saveNew !== "function") {
    showError("Saving a new file needs the desktop app (window.pdf4all is unavailable).");
    return null;
  }
  const result = await api.saveNew(bytes, suggestedName, kind, open ? { open } : undefined);
  if (result.ok) return result.name;
  if (result.reason === "cancelled") return null;
  const what = kind === "txt" ? "text" : kind.toUpperCase();
  const text = newFileReasonText(result.reason);
  showError(text ? `The ${what} was not saved. ${text}` : `The ${what} was not saved (${result.reason}).`);
  return null;
}

/** "Export…": the dialog picks a kind and a resolution. */
function requestExport() {
  if (!doc || doc.info.needsPassword) return;
  exportDialog.showModal();
  exportKindSelect.focus();
}

function syncExportDpi() {
  exportDpiSelect.disabled = exportKindSelect.value === "txt";
}

async function saveExportDialog() {
  const kind = exportKindSelect.value === "jpeg" ? "jpeg" : exportKindSelect.value === "txt" ? "txt" : "png";
  const dpi = Number(exportDpiSelect.value) || 150;
  exportDialog.close();
  await exportCurrent(kind, dpi);
}

/**
 * One export: a page rendered at `dpi` and encoded by the platform, or the whole
 * text. Nothing here touches the document's journal or dirty state.
 */
async function exportCurrent(kind: "png" | "jpeg" | "txt", dpi: number): Promise<boolean> {
  if (!doc) return false;
  exportSummary = null;
  if (kind === "txt") {
    try {
      const reply = (await request({ op: "text-all" })) as TextAllReply;
      const bytes = new TextEncoder().encode(reply.text).buffer as ArrayBuffer;
      const name = await saveNewFile(bytes, `${fileStem(doc.name)}.txt`, "txt");
      if (!name) return false;
      exportSummary = { kind: "txt", bytes: bytes.byteLength, pages: doc.pageCount };
      announcePage(`Exported text (${doc.pageCount} page${doc.pageCount === 1 ? "" : "s"})`);
      return true;
    } catch (err) {
      showError(messageOf(err));
      return false;
    }
  }

  if (!printAvailable()) {
    showError("This document does not permit printing, so a page cannot be exported.");
    return false;
  }
  const page = wantedPage;
  // The engine caps a render at MAX_SCALE (4) and a 4096 px edge, so 300 dpi on a
  // letter page lands at 288 dpi; the status line reports the pixels actually made.
  const scale = Math.min(MAX_SCALE, dpi / 72);
  try {
    const reply = (await request({ op: "render", page, scale })) as RenderReply;
    if (reply.pixels.byteLength > PRINT_MAX_BYTES) {
      showError("That page is too many pixels to export at this resolution; try a lower dpi.");
      return false;
    }
    const canvas = new OffscreenCanvas(reply.pixelWidth, reply.pixelHeight);
    const context = canvas.getContext("2d");
    if (!context) throw new Error("canvas 2d unavailable");
    context.putImageData(new ImageData(new Uint8ClampedArray(reply.pixels), reply.pixelWidth, reply.pixelHeight), 0, 0);
    const blob = kind === "png"
      ? await canvas.convertToBlob({ type: "image/png" })
      : await canvas.convertToBlob({ type: "image/jpeg", quality: 0.9 });
    const bytes = await blob.arrayBuffer();
    const name = await saveNewFile(bytes, `${fileStem(doc.name)} page ${page + 1}.${kind === "png" ? "png" : "jpg"}`, kind);
    if (!name) return false;
    exportSummary = { kind, bytes: bytes.byteLength, width: reply.pixelWidth, height: reply.pixelHeight };
    announcePage(`Exported page ${page + 1} as ${kind.toUpperCase()} (${reply.pixelWidth}×${reply.pixelHeight})`);
    return true;
  } catch (err) {
    showError(messageOf(err));
    return false;
  }
}

/** "New from images…": pick images, build one PDF, save it and let main open it. */
async function requestNewFromImages() {
  if (!api || typeof api.pickImages !== "function") {
    showError("Building a PDF from images needs the desktop app (window.pdf4all is unavailable).");
    return;
  }
  let picked;
  try {
    picked = await api.pickImages();
  } catch (err) {
    showError(messageOf(err));
    return;
  }
  if (!picked.ok) {
    if (picked.reason !== "cancelled") {
      showError(picked.reason === "too-many" ? "Too many images; pick at most 50."
        : picked.reason === "too-large" ? "Those images are larger than this app will read at once."
          : "Those images could not be read.");
    }
    return;
  }
  if (!picked.images.length) return;
  try {
    const reply = (await request({ op: "pdf-from-images", images: picked.images })) as PdfFromImagesReply;
    const suggested = `${fileStem(picked.images[0].name)}.pdf`;
    const name = await saveNewFile(reply.bytes, suggested, "pdf", true);
    if (!name) return;
    announcePage(`Created ${name} (${reply.pageCount} page${reply.pageCount === 1 ? "" : "s"})`);
    console.log(`P2_NEW ${JSON.stringify({ pages: reply.pageCount, bytes: reply.bytes.byteLength })}`);
  } catch (err) {
    showError(messageOf(err));
  }
}

/** `--edit=export-png|export-jpeg|export-txt`: page 1 at 150 dpi (or all text), then log. */
async function smokeExport(kind: "png" | "jpeg" | "txt") {
  if (!doc) {
    logEdit();
    return;
  }
  await exportCurrent(kind, 150);
  logEdit();
}

// ---------------------------------------------------- decoration (unit 8)

/**
 * The page labels, fetched on open and after anything that can move them: a labels
 * op, a page op, or an undo/redo of either (both step sets are checked in undoEdit).
 * Nothing else writes labels, and the worker has to load every page to answer.
 */
async function refreshLabels() {
  const seq = ++labelsSeq;
  if (!doc || doc.info.needsPassword) {
    labels = [];
    applyLabelText();
    return;
  }
  const startedGeneration = generation;
  let reply: LabelsReply;
  try {
    reply = (await request({ op: "labels" })) as LabelsReply;
  } catch {
    return;
  }
  if (seq !== labelsSeq || startedGeneration !== generation) return;
  labels = reply.items;
  applyLabelText();
}

/** `''` when the label is just the 1-based index, `' (iii)'` when the document says more. */
function labelSuffix(page: number): string {
  const label = labels[page];
  return label && label !== String(page + 1) ? ` (${label})` : "";
}

/** Wherever a page number is shown, the label joins it when it says something new. */
function applyLabelText() {
  const suffix = doc ? labelSuffix(wantedPage) : "";
  pageLabelEl.hidden = !suffix;
  pageLabelEl.textContent = suffix ? suffix.trim() : "";
  pageInput.setAttribute("aria-label", doc ? `Page ${wantedPage + 1}${suffix} of ${doc.pageCount}` : "Page");
  caption.textContent = !doc ? "No page"
    : doc.info.needsPassword ? "Password required"
      : `Page ${wantedPage + 1}${suffix} of ${doc.pageCount}`;
}

/** The optional-content layers; hidden entirely when the document has none. */
async function refreshLayers() {
  const seq = ++layersSeq;
  if (!doc || doc.info.needsPassword) {
    layers = [];
    renderLayers();
    return;
  }
  const startedGeneration = generation;
  let reply: LayersReply;
  try {
    reply = (await request({ op: "layers" })) as LayersReply;
  } catch {
    return;
  }
  if (seq !== layersSeq || startedGeneration !== generation) return;
  layers = reply.items;
  renderLayers();
}

function renderLayers() {
  layersEl.hidden = layers.length === 0;
  layersNoteEl.hidden = layersPersisted || layers.length === 0;
  setEmpty(layersEmptyEl, layers.length, "This document has no optional-content layers.", "No document open yet.");
  const fragment = document.createDocumentFragment();
  for (const item of layers) {
    const row = document.createElement("li");
    const label = document.createElement("label");
    label.className = "field-check";
    const box = document.createElement("input");
    box.type = "checkbox";
    box.id = `layer-${item.id}`;
    box.checked = item.visible;
    box.dataset.layerId = String(item.id);
    box.disabled = !doc || doc.info.needsPassword;
    box.addEventListener("change", () => void setLayer(item.id, box.checked));
    label.append(box, document.createTextNode(item.name));
    row.append(label);
    fragment.append(row);
  }
  layerListEl.replaceChildren(fragment);
}

/**
 * A view-only toggle (measured): the worker changes the in-memory document and
 * writes nothing, so the renderer clears its caches and repaints rather than
 * pretending the document changed.
 */
async function setLayer(layerId: number, visible: boolean) {
  try {
    const reply = (await request({ op: "layer-set", layerId, visible })) as LayerSetReply;
    layers = reply.items;
    layersPersisted = reply.persisted;
    renderLayers();
    const item = layers.find(entry => entry.id === layerId);
    announcePage(`Layer ${item?.name ?? layerId + 1} ${visible ? "shown" : "hidden"}${reply.persisted ? "" : " (view only, not saved)"}`);
    cache.clear();
    shownKey = null; // drain() skips a page whose key is already shown; the toggle changed its pixels
    thumbCache.clear();
    thumbPainted.clear();
    invalidateThumb(wantedPage);
    void drain();
  } catch (err) {
    showError(messageOf(err));
    await refreshLayers();
  }
}

/** "Watermark…" and "Header/footer…" share one dialog; the kind picker picks it. */
function openStampDialog(kind?: "watermark" | "header" | "footer") {
  if (!doc || !canEdit()) return;
  if (kind) stampKindSelect.value = kind;
  syncStampHeading();
  stampTextInput.value = stampKindSelect.value === "watermark" ? "DRAFT"
    : stampKindSelect.value === "header" ? "Page {page} of {pages}" : "Page {page} of {pages}";
  stampPagesInput.value = "";
  stampAlignSelect.value = "left";
  stampBatesStartInput.value = String(batesNext);
  stampBatesPadInput.value = String(batesPad);
  stampDialog.showModal();
  stampTextInput.focus();
  stampTextInput.select();
}

function syncStampHeading() {
  const kind = stampKindSelect.value;
  stampDialogHeading.textContent = kind === "watermark" ? "Watermark" : kind === "header" ? "Add header" : "Add footer";
  stampAlignSelect.disabled = kind === "watermark";
}

async function saveStampDialog(): Promise<boolean> {
  clearError();
  if (!doc) {
    stampDialog.close();
    return false;
  }
  const pages = parsePageRange(stampPagesInput.value, doc.pageCount);
  if (!pages) {
    showError(`Pages must look like "1-3, 5" (this document has ${doc.pageCount})`);
    return false;
  }
  const kind = stampKindSelect.value === "watermark" ? "watermark" : stampKindSelect.value === "header" ? "header" : "footer";
  const text = stampTextInput.value;
  // §9.11: the Bates fields count only when the text uses {bates}; an empty start is a refusal, never 0.
  let bates: { batesStart: number; batesPad: number } | undefined;
  if (text.includes("{bates}")) {
    const start = stampBatesStartInput.value.trim();
    const pad = stampBatesPadInput.value.trim();
    if (!/^\d{1,15}$/.test(start) || !/^\d{1,2}$/.test(pad) || Number(pad) < 1 || Number(pad) > 15) {
      showError("Bates start must be a whole number and digits 1\u201315");
      decorateLast({ action: "stamp", kind, ok: false });
      return false;
    }
    bates = { batesStart: Number(start), batesPad: Number(pad) };
  }
  const align = kind === "watermark" || stampAlignSelect.value !== "right" ? undefined : "right";
  stampDialog.close();
  return addStamp(kind, text, pages.length === doc.pageCount ? undefined : pages, { ...bates, ...(align ? { align } : {}) });
}

async function addStamp(kind: "watermark" | "header" | "footer", text: string, pages?: number[],
  options: { batesStart?: number; batesPad?: number; align?: string } = {}): Promise<boolean> {
  stampBates = null;
  try {
    const reply = (await request({ op: "stamp", kind, text, ...(pages ? { pages } : {}), ...options })) as MutationReply & { bates?: { from: number; to: number } };
    const count = pages ? pages.length : doc?.pageCount ?? 0;
    const stamped = pages ?? Array.from({ length: doc?.pageCount ?? 0 }, (_, index) => index);
    await applyEdit(reply, "");
    let suffix = "";
    if (reply.bates) {
      const pad = options.batesPad ?? 6;
      stampBates = { from: reply.bates.from, to: reply.bates.to };
      batesNext = reply.bates.to + 1;
      batesPad = pad;
      batesLast = { from: reply.bates.from, to: reply.bates.to, pages: stamped };
      const from = String(reply.bates.from).padStart(pad, "0");
      const to = String(reply.bates.to).padStart(pad, "0");
      suffix = from === to ? ` (Bates ${from})` : ` (Bates ${from}\u2013${to})`;
    }
    decorateLast({ action: "stamp", kind, pages: stamped, ok: true, ...(stampBates ? { bates: stampBates } : {}) });
    announcePage(`Added ${kind} to ${count} page${count === 1 ? "" : "s"}${suffix}`);
    return true;
  } catch (err) {
    decorateLast({ action: "stamp", kind, ok: false });
    showError(messageOf(err));
    return false;
  }
}

function openLabelsDialog() {
  if (!doc || !canEdit()) return;
  labelsFromInput.value = String(wantedPage + 1);
  labelsFromInput.max = String(doc.pageCount);
  labelsStyleSelect.value = "D";
  labelsPrefixInput.value = "";
  labelsStartInput.value = "1";
  labelsDialog.showModal();
  labelsFromInput.focus();
  labelsFromInput.select();
}

/** The 1-based page the labels dialog is acting on, or null when it is out of range. */
function labelsFrom(): number | null {
  if (!doc) return null;
  const from = Number(labelsFromInput.value);
  if (!Number.isInteger(from) || from < 1 || from > doc.pageCount) return null;
  return from - 1;
}

async function saveLabelsDialog(): Promise<boolean> {
  const from = labelsFrom();
  if (from === null) {
    showError(`Enter a page between 1 and ${doc?.pageCount ?? 1}`);
    return false;
  }
  const style = labelsStyleSelect.value;
  const prefix = labelsPrefixInput.value.slice(0, 32);
  const start = Number(labelsStartInput.value) || 1;
  labelsDialog.close();
  try {
    const reply = (await request({ op: "labels-set", from, style, prefix, start })) as LabelsSetReply;
    labels = reply.items;
    await applyEdit(reply, "");
    applyLabelText();
    announcePage(`Page labels set from page ${from + 1}`);
    return true;
  } catch (err) {
    showError(messageOf(err));
    return false;
  }
}

async function clearLabels(): Promise<boolean> {
  const from = labelsFrom();
  if (from === null) {
    showError(`Enter a page between 1 and ${doc?.pageCount ?? 1}`);
    return false;
  }
  labelsDialog.close();
  try {
    const reply = (await request({ op: "labels-clear", from })) as LabelsSetReply;
    labels = reply.items;
    await applyEdit(reply, "");
    applyLabelText();
    announcePage(`Cleared page labels from page ${from + 1}`);
    return true;
  } catch (err) {
    showError(messageOf(err));
    return false;
  }
}

/** A2 §8.3: record the last dialog-backed decorate action without dropping the stamps view. */
function decorateLast(last: { action: string; kind?: string; pages?: number[]; ok: boolean; detail?: string; bates?: { from: number; to: number } }): void {
  decorateSummary = {
    stamps: decorateSummary?.stamps ?? { watermark: 0, header: 0 },
    layersSaved: decorateSummary?.layersSaved ?? layersPersisted,
    // The labels and layers are read live: a clear moves them backwards (like refreshDecorateSummary).
    labels: [...labels],
    layers: layers.map(item => ({ name: item.name, visible: item.visible })),
    last,
  };
}

/**
 * `--edit=decorate`: watermark and header on every page, labels from page 1 in roman
 * numerals, and on a document with layers the second layer hidden (view only).
 */
async function smokeDecorate(contents: string | null) {
  if (!doc) {
    logEdit();
    return;
  }
  const text = contents ?? "PDF4All P2 decorate";
  const count = doc.pageCount;
  const watermarkOk = await addStamp("watermark", text);
  const headerOk = await addStamp("header", "Page {page} of {pages}");
  try {
    const reply = (await request({ op: "labels-set", from: 0, style: "r" })) as LabelsSetReply;
    labels = reply.items;
    await applyEdit(reply, "");
  } catch (err) {
    showError(messageOf(err));
  }
  applyLabelText();
  // The contract's "hide layer 1": mupdf lists layers in the reverse of /OCGs, so the
  // fixture's own order decides which one that is; the logged name says which.
  if (layers.length > 1) await setLayer(1, false);
  decorateSummary = {
    stamps: { watermark: watermarkOk ? count : 0, header: headerOk ? count : 0 },
    labels: [...labels],
    layers: layers.map(item => ({ name: item.name, visible: item.visible })),
    layersSaved: layersPersisted,
  };
  logEdit();
}

/**
 * `P2_EDIT.decorate` after an undo/redo: the stamps and labels moved, the layers did not
 * (a toggle is not journaled), so the cached summary would otherwise log the old labels.
 */
function refreshDecorateSummary() {
  if (!decorateSummary) return;
  decorateSummary = {
    ...decorateSummary,
    labels: [...labels],
    layers: layers.map(item => ({ name: item.name, visible: item.visible })),
  };
}

/** One fixed line grok's smoke waits for by name: the state after the edit. */
function logEdit() {
  console.log(`P2_EDIT ${JSON.stringify(editSummary())}`);
}

let lastSave: Record<string, unknown> | null = null;
let lastPrint: Record<string, unknown> | null = null;

/** The selected inspector tab id, read live from its tablist (A2 §8.13). */
function activeInspectorTabId(): string {
  return inspectorTabs.find(tab => tab.getAttribute("aria-selected") === "true")?.id ?? "";
}

/** A2 §8.4: the shown page's annotations in `renderNotes` order, text capped at 200 chars. */
function driveComments(): { index: number; type: string; contents: string; editable: boolean }[] {
  return notes.map((item, index) => ({
    index, type: item.type, contents: item.contents.slice(0, 200), editable: item.editable,
  }));
}

/** The `contents` of a parameterized drive op: `{}` when absent, null when unparseable. */
function driveWanted(contents: string | null): Record<string, unknown> | null {
  if (contents === null) return {};
  try {
    const parsed: unknown = JSON.parse(contents);
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) return parsed as Record<string, unknown>;
  } catch {
    // Falls through to the refusal below.
  }
  // §1: a refusal lands in STATE `error`, never silently.
  showError("The drive contents must be a JSON object");
  return null;
}

/** The `--drive` state: every smoke summary plus what the panels show. No document text, no paths. */
/** `{left, top, width, height}` CSS px of a target or editor (§9.11). */
function cssBox(element: HTMLElement): { left: number; top: number; width: number; height: number } {
  return { left: parseFloat(element.style.left) || 0, top: parseFloat(element.style.top) || 0, width: parseFloat(element.style.width) || 0, height: parseFloat(element.style.height) || 0 };
}
function directStateActive(): Record<string, unknown> | null {
  if (!layerLive()) return null;
  if (directActive?.kind === "textbox") return { kind: "textbox", annotId: directActive.annotId ?? null, page: directActive.page, point: directActive.point, box: cssBox(directActive.editor), draft: directActive.editor.value };
  if (directActive?.kind === "add") return { kind: "add", page: directActive.page, point: directActive.point, fontSize: Number(directActive.editor.dataset.fontSize) || 0, box: cssBox(directActive.editor), draft: directActive.editor.value };
  if (!directActive && directFocusedObject) {
    const focus = directFocusedObject, node = hitLayer.querySelector<HTMLElement>(`.object-target[data-target-id="${CSS.escape(focus.targetId)}"]`);
    const item = contentScan?.items.find(value => value.targetId === focus.targetId);
    if (node && item) return { kind: "object", name: focus.name, targetId: focus.targetId, page: wantedPage, box: cssBox(node), placement: item.placement?.box ?? null, ops: item.objectOps ?? [] };
  }
  if (directActive?.kind === "link") {
    const active = directActive, annotId = linkAnnotId(active.item as LinkEntry);
    return linkState(links.find(entry => linkAnnotId(entry) === annotId) ?? active.item as LinkEntry, cssBox(active.editor), true, active.editor.value);
  }
  if (!directActive && directFocusedLink) {
    const focus = directFocusedLink, node = hitLayer.querySelector<HTMLElement>(`.link-target[data-annot-id="${focus.annotId}"]`);
    const item = links.find(entry => linkAnnotId(entry) === focus.annotId);
    if (node && item) return linkState(item, cssBox(node), false);
  }
  if (!directActive && directFocusedMark) {
    const focus = directFocusedMark, node = hitLayer.querySelector<HTMLElement>(`.redact-target[data-annot-id="${focus.annotId}"]`);
    const item = notes.find(value => value.id === focus.annotId);
    if (node && item) return { kind: "redact", annotId: focus.annotId, page: wantedPage, box: cssBox(node), rect: item.rect };
  }
  if (directActive) return { kind: directActive.paragraph ? "paragraph" : directActive.kind,
    ...(directActive.kind === "field" ? { name: (directActive.item as FieldItem).name } : { targetId: (directActive.item as EditTarget).targetId, ...(directActive.paragraph ? { paragraphId: directActive.paragraph.paragraphId, lines: directActive.paragraph.lines, width: directActive.paragraph.width } : { fontSize: (directActive.item as EditTarget).fontSize, moveable: (directActive.item as EditTarget).moveable }) }),
    page: directActive.page,
    rect: directActive.kind === "field" ? (directActive.item as FieldItem).rect : (directActive.paragraph?.bounds ?? (directActive.item as EditTarget).bounds),
    box: { left: parseFloat(directActive.editor.style.left) || 0, top: parseFloat(directActive.editor.style.top) || 0, width: parseFloat(directActive.editor.style.width) || 0, height: parseFloat(directActive.editor.style.height) || 0 },
    fontPx: parseFloat(directActive.editor.style.fontSize) || 0,
    value: directActive.kind === "field" ? String((directActive.item as FieldItem).value) : directActive.paragraph ? paragraphText(directActive.paragraph, directActive.scan) : (directActive.item as EditTarget).text ?? "",
    draft: directActive.editor.value,
  };
  const item = directFocusedPage === wantedPage ? directFocusedField : null;
  return item ? { kind: "field", name: item.name, page: wantedPage, value: String(item.value), draft: String(item.value) } : null;
}
function stateSummary() {
  return {
    doc: doc ? { name: doc.name, pageCount: doc.pageCount, readOnly: isReadOnly(doc.info), needsPassword: doc.info.needsPassword } : null,
    sign: { last: signLast, dialog: certSignDialog.open && certSignField ? { field: certSignField.name, page: certSignField.page } : null },
    signatures: signaturesSummary(),
    verify: { last: verifyLast },
    mode: uiMode,
    // A2 §2: page/zoom/search/tab/comments exist only while a document is open; the
    // fixed zoom carries a rounded `scale`, the fit modes only their mode (§8.16).
    ...(doc ? { page: wantedPage } : {}),
    ...(doc ? { zoom: zoomMode === "fixed" ? { mode: "fixed", scale: round3(zoomNow(wantedPage)) } : { mode: zoomMode } } : {}),
    ...(doc ? { search: { query: searchQuery, count: searchHits.length, index: searchIndex } } : {}),
    ...(doc ? { tab: activeInspectorTabId(), comments: driveComments() } : {}),
    ...(annotLast ? { annotLast } : {}),
    redact: {
      marks: redactState?.marks ?? 0,
      pages: redactState?.pages ?? [],
      rects: notes.filter(item => item.type === "Redact").map(item => ({ id: item.id, rect: item.rect, search: item.search === true })),
      preview: redactState ? { shared: redactState.shared.length, sanitize: redactState.sanitize, inlineAlt: redactState.inlineAlt } : null,
      last: redactLast,
    },
    status: statusEl.textContent ?? "",
    error: errorEl.hidden ? "" : errorEl.textContent ?? "",
    // M9: whether a worker restart can restore the open document, and what the last restart did.
    recovery: { available: recovery.available, unavailable: recovery.unavailable, entries: recovery.entries.length, bytes: recovery.bytes, saved: recovery.saved !== null, budget: recovery.budget, restoring: restoreRunning, dirtyReported: reportedDirty, progress: restoreProgress, paused: [...pausedReads], last: restoreLast },
    announcement: editStatusEl.textContent || navStatusEl.textContent || "",
    canRedo,
    help: { open: helpDialog.open, rows: helpRows() },
    ...(batchState ? { batch: batchSummary() } : {}),
    compare: compareSummary(),
    ...editSummary(),
    // fill-anywhere D6/D7: the text boxes on the shown page and the Save-as-copy rule.
    fill: { filled: fillSave !== "none", saveAsCopy: fillSave === "needs-copy", boxes: notes.filter(item => item.type === "FreeText").map(item => ({ id: item.id, rect: item.rect.map(round2), chars: item.contents.length })), last: fillLast, signature: { saved: savedSignature !== null, kind: savedSignature === null ? null : savedSignature.kind === "image" ? "image" : "drawn", strokes: savedSignature?.kind === undefined ? savedSignature?.strokes.length ?? 0 : 0, points: savedSignature?.kind === undefined ? savedSignature?.strokes.reduce((sum, stroke) => sum + stroke.length, 0) ?? 0 : 0, armed: signArmed, pad: sigpadDialog.open }, signatures: notes.filter(isSignature).map(item => ({ id: item.id, rect: item.rect.map(round2), kind: isDrawnSignature(item) ? "drawn" : "image" })), lastSignature: fillSignLast },
    content: contentSummary(),
    ...(lastSave ? { save: lastSave } : {}),
    ...(lastPrint ? { print: lastPrint } : {}),
  };
}

function logState() {
  console.log(`STATE ${JSON.stringify(stateSummary())}`);
}
// CDP readers (`Runtime.evaluate`) get one JSON instead of scraping ids.
Object.defineProperty(window, "pdf4allState", { value: stateSummary });

function editSummary() {
  const annots = Object.fromEntries(
    (["Text", "Highlight", "Underline", "StrikeOut", "FreeText", "Square", "Circle", "Line", "Ink", "Stamp", "Redact"] as const)
      .map(type => [type, notes.filter(item => item.type === type).length]),
  );
  const sign = signFromNotes();
  return {
    revision,
    dirty,
    canUndo,
    notes: annots.Text,
    annots,
    pages: { count: doc?.pageCount ?? 0, rotate: doc?.rotations ?? [], ...(pageIds ? { ids: pageIds } : {}) },
    fields: fieldSummary(),
    direct: {
      mode: uiMode,
      active: directStateActive(),
      gesture: directGesture,
      // P6 U5 §2.5: the object selection (click order, anchor last) and the open context menu.
      // P6 U7 §10.29: every member kind, in every mode.
      selection: doc && directSelection.length ? directSelection.map(member => ({ targetId: member.targetId, kind: member.kind, name: member.name })) : [],
      menu: objectMenuState(),
      // P6 U7 §2.5: this window's clipboard (count and kinds only).
      clipboard: clipboardInfo ? { count: clipboardInfo.count, kinds: { ...clipboardInfo.kinds } } : null,
      // P6 U6 §2.3: crop mode (view space; `full` is the visible box when uncropped).
      crop: doc && uiMode === "edit" && directCrop && directFocusedObject ? { name: directFocusedObject.name, full: directCrop.full.map(value => Math.round(value * 100) / 100) } : null,
      highlight: highlightFields,
      targets: !layerLive() || uiMode === "redact" ? 0 : (uiMode === "edit" ? (contentScan?.items.filter(item => item.bounds?.length === 4 && item.kind === "text").length ?? 0) : fields.filter(item => item.kind !== "signature").length),
      objects: uiMode === "edit" && layerLive() ? hitLayer.querySelectorAll(".object-target").length : 0,
      marks: uiMode === "redact" && layerLive() ? hitLayer.querySelectorAll(".redact-target").length : 0,
      links: layerLive() ? hitLayer.querySelectorAll(".link-target").length : 0,
      outlines: showOutlines,
      status: fieldsStatusEl.textContent ?? "",
    },
    nav: {
      outline: outline.map(entry => entry.page === null && entry.uri
        ? { title: entry.title, page: null, uri: entry.uri }
        : { title: entry.title, page: entry.page }),
      links: Object.fromEntries(Object.entries(navLinks)),
      ...(navLast ? { last: navLast } : {}),
      linkDraft: navLinkDraft,
    },
    // P6 U4 §1.8: the shown page's links with their stable ids; `nav.links` keeps its pinned shape.
    links: doc ? links.map(item => ({ id: item.id, annotId: linkAnnotId(item), rect: item.rect.map(value => Math.round(value * 100) / 100), kind: item.kind, ...(item.kind === "page" ? { page: item.page } : { uri: item.uri }) })) : [],
    ...(assembleSummary ? { assemble: assembleSummary } : {}),
    ...(extractSummary ? { extract: extractSummary } : {}),
    ...(exportSummary ? { export: exportSummary } : {}),
    ...(decorateSummary ? { decorate: { ...decorateSummary, bates: { next: batesNext, pad: batesPad, last: batesLast } } } : {}),
    ...(sign ? { sign } : {}),
    ...(securityState ? { security: { mode: securityState.mode, algorithm: securityState.algorithm, permissions: securityState.permissions, auth: securityState.auth } } : {}),
  };
}

/** The note's top-left in PDF points; the canvas CSS box maps 1:1 to the page box. */
function pagePoint(event: MouseEvent): [number, number] | null {
  const size = doc?.pages[shownPage];
  const box = canvas.getBoundingClientRect();
  if (!size || box.width < 1 || box.height < 1) return null;
  return [
    (event.clientX - box.left) * (size.width / box.width),
    (event.clientY - box.top) * (size.height / box.height),
  ];
}

function openNoteDialog(annotId: number) {
  const item = notes.find(note => note.id === annotId);
  if (!item) return;
  editingNote = annotId;
  boxPoint = null;
  noteDialogHeading.textContent = "Edit note";
  noteTextInput.value = item.contents;
  noteDialog.showModal();
  noteTextInput.focus();
  noteTextInput.select();
}

/** The same dialog in add mode: the typed text becomes the new box's contents. */
function openTextBoxDialog(point: [number, number]) {
  if (!doc || !canAnnotate()) return;
  editingNote = null;
  boxPoint = point;
  noteDialogHeading.textContent = "Add text box";
  noteTextInput.value = "";
  noteDialog.showModal();
  noteTextInput.focus();
}

async function saveNoteDialog() {
  const annotId = editingNote;
  const point = boxPoint;
  noteDialog.close();
  if (point) {
    await addTextBoxAt(wantedPage, point, noteTextInput.value.slice(0, MAX_NOTE_CHARS));
    return;
  }
  if (annotId === null) return;
  await editNote(annotId, noteTextInput.value.slice(0, MAX_NOTE_CHARS));
}

// ------------------------------------------------------- certificate signing

async function driveSign(contents: string | null): Promise<void> {
  const values = driveWanted(contents);
  if (!values) { signLast = { op: "sign", ok: false, reason: "invalid" }; return; }
  const cert = values.cert;
  if (typeof cert !== "string" || typeof values.password !== "string") { signLast = { op: "sign", ok: false, reason: "invalid" }; return; }
  // P5 U3 §2.5/§9.23: `field` signs that existing field on the shown page, once its fields are listed.
  let field: FieldItem | null = null;
  if (typeof values.field === "string") {
    while (doc && fieldsPage !== wantedPage) await new Promise(resolve => window.setTimeout(resolve, 10));
    field = fields.find(item => item.kind === "signature" && item.name === values.field) ?? null;
    if (!field) { signLast = { op: "sign", ok: false, reason: "no-such-field" }; return; }
  }
  await pickCertSignFile(cert);
  if (!certSignPath) { signLast = { op: "sign", ok: false, reason: "unreadable" }; return; }
  certSignPage.value = String((Number(values.page) || 1));
  certSignReason.value = typeof values.reason === "string" ? values.reason : "";
  certSignCertify.disabled = !!doc && doc.info.signatures > 0;
  certSignCertify.checked = values.certify === true;
  certSignVisible.checked = Array.isArray(values.rect);
  certSignBox.hidden = !certSignVisible.checked;
  if (Array.isArray(values.rect) && values.rect.length === 4) {
    certSignX.value = String(values.rect[0]); certSignY.value = String(values.rect[1]);
    certSignW.value = String(Number(values.rect[2]) - Number(values.rect[0]));
    certSignH.value = String(Number(values.rect[3]) - Number(values.rect[1]));
  }
  certSignPassword.value = values.password;
  setCertSignMode(field);
  if (!certSignDialog.open) certSignDialog.showModal();
  await runCertSign();
}

// ------------------------------------------------------------------- batch

function batchSummary() {
  const state = batchState!;
  const done = state.files.filter(row => row.status === "ok" || row.status === "failed");
  return {
    folder: state.folder, out: state.out, action: state.action,
    total: state.files.length, done: done.length,
    ok: done.filter(row => row.status === "ok").length, failed: done.filter(row => row.status === "failed").length,
    running: state.running, cancelled: state.cancelled, report: state.report, bates: { ...state.bates },
    files: state.files.map(row => ({ name: row.name, status: row.status, pages: row.pages, detail: row.detail, ...(row.bates ? { bates: { ...row.bates } } : {}) })),
  };
}

function ensureBatchState() {
  batchState ??= { folder: null, out: null, action: null, files: [], running: false, cancelled: false, report: null, bates: { start: batesNext, next: batesNext, pad: batesPad } };
  return batchState;
}

/** The table and the live summary, redrawn from batchState. */
function renderBatch() {
  const state = ensureBatchState();
  const body = batchListEl.tBodies[0];
  body.replaceChildren(...state.files.map(row => {
    const tr = document.createElement("tr");
    const range = row.bates ? `${String(row.bates.from).padStart(state.bates.pad, "0")}\u2013${String(row.bates.to).padStart(state.bates.pad, "0")}` : "";
    for (const text of [row.name, row.status, row.status === "ok" ? String(row.pages) : "", range, row.detail]) {
      const td = document.createElement("td");
      td.textContent = text;
      tr.append(td);
    }
    return tr;
  }));
  batchFolderInput.value = state.folder ?? "";
  batchPickButton.disabled = state.running;
  batchStartButton.disabled = state.running || state.files.length < 1;
  batchCloseButton.disabled = state.running;
  if (state.running) {
    const summary = batchSummary();
    batchSummaryEl.textContent = `${summary.done} of ${summary.total} done, ${summary.failed} failed`;
  }
}

function openBatchDialog() {
  const state = ensureBatchState();
  if (!state.running) {
    batchBatesStartInput.value = String(batesNext);
    batchBatesPadInput.value = String(batesPad);
    batchAlignSelect.disabled = batchKindSelect.value === "watermark";
  }
  renderBatch();
  if (!batchDialog.open) batchDialog.showModal();
}

/** `#batch-pick`: main's folder listing becomes the queued rows (§9.14). */
async function pickBatchFolder(): Promise<boolean> {
  const state = ensureBatchState();
  if (state.running) return false;
  if (!api || typeof api.pickFolder !== "function") {
    showError("Batch is not available in this window");
    return false;
  }
  let picked: Awaited<ReturnType<NonNullable<Pdf4AllApi["pickFolder"]>>>;
  try {
    picked = await api.pickFolder();
  } catch (err) {
    picked = { ok: false, reason: messageOf(err) };
  }
  if (!picked.ok) {
    if (picked.reason !== "cancelled") showError(picked.reason === "too-many-files" ? "Batch: the folder has more than 200 PDF files" : `Batch: the folder could not be read (${picked.reason})`);
    return false;
  }
  Object.assign(state, {
    folder: picked.folder, out: picked.out, report: null, cancelled: false,
    files: picked.files.map(file => ({ name: file.name, status: "queued", pages: 0, detail: "" })),
  });
  batchSummaryEl.textContent = picked.files.length ? `${picked.files.length} PDF file${picked.files.length === 1 ? "" : "s"} queued` : "No PDF files in this folder";
  renderBatch();
  return true;
}

/** `#batch-start`: the action fields, checked as the stamp dialog checks them (Bates §9.11). */
async function startBatch(): Promise<boolean> {
  clearError();
  const state = ensureBatchState();
  if (state.running) {
    showError("A batch is already running");
    return false;
  }
  if (!state.files.length) {
    showError("Batch: choose a folder with PDF files first");
    return false;
  }
  const kind = batchKindSelect.value === "watermark" ? "watermark" : batchKindSelect.value === "header" ? "header" : "footer";
  const text = batchTextInput.value;
  const start = batchBatesStartInput.value.trim();
  const pad = batchBatesPadInput.value.trim();
  if (text.includes("{bates}") && (!/^\d{1,15}$/.test(start) || !/^\d{1,2}$/.test(pad) || Number(pad) < 1 || Number(pad) > 15)) {
    showError("Bates start must be a whole number and digits 1\u201315");
    return false;
  }
  const pages = batchPagesInput.value.trim();
  const action: BatchAction = {
    kind, text, ...(pages ? { pages } : {}),
    ...(kind !== "watermark" && batchAlignSelect.value === "right" ? { align: "right" as const } : {}),
    ...(text.includes("{bates}") ? { batesPad: Number(pad) } : {}),
  };
  if (text.includes("{bates}")) batesNext = Number(start);
  await runBatch(action);
  return true;
}

/** One file's refusal text: the reason, plus the engine's detail token when it has one. */
function batchFailure(err: unknown): string {
  const { reason, reply } = refusalOf(err);
  const detail = typeof reply?.detail === "string" ? reply.detail : "";
  if (!reason || reason === "worker") return messageOf(err);
  return detail ? `${reason}: ${detail}` : reason;
}

/**
 * The queue (contract-batch.md §3.2, §9.11, §9.13): one file at a time - exists probe, read,
 * batch-file on a transient document, write - and the counter moves only after a write lands.
 */
async function runBatch(action: BatchAction): Promise<void> {
  const state = ensureBatchState();
  const io = api?.batchIo;
  if (!io) return;
  const pad = action.batesPad ?? batesPad;
  for (const row of state.files) Object.assign(row, { status: "queued", pages: 0, detail: "", bates: undefined });
  Object.assign(state, { action, running: true, cancelled: false, report: null, bates: { start: batesNext, next: batesNext, pad } });
  renderBatch();
  try {
    for (const [index, row] of state.files.entries()) {
      if (state.cancelled || (batchCancelAfter !== null && index >= batchCancelAfter)) {
        state.cancelled = true;
        break;
      }
      row.status = "running";
      renderBatch();
      try {
        const probe = await io(row.name, "exists");
        if (!probe.ok) throw workerFailure(probe.reason, probe.reason);
        if (probe.exists) throw workerFailure("exists", "exists");
        const read = await io(row.name, "read");
        if (!read.ok || !read.bytes) throw workerFailure(read.ok ? "unreadable" : read.reason, "unreadable");
        const reply = (await request({ op: "batch-file", name: row.name, bytes: read.bytes, action: { ...action, batesStart: batesNext } }, [read.bytes], SAVE_TIMEOUT_MS)) as unknown as {
          pages: number; bytes: ArrayBuffer; bates?: { from: number; to: number };
        };
        const written = await io(row.name, "write", reply.bytes);
        if (!written.ok) throw workerFailure(written.reason, written.reason);
        Object.assign(row, { status: "ok", pages: reply.pages, detail: "", ...(reply.bates ? { bates: reply.bates } : {}) });
        if (reply.bates) {
          batesNext = reply.bates.to + 1;
          batesPad = pad;
        }
      } catch (err) {
        Object.assign(row, { status: "failed", pages: 0, detail: batchFailure(err) });
        // A timeout or a dead worker stops the queue (§9.11); the rest stay queued.
        if (refusalOf(err).reason === "worker") state.cancelled = true;
      }
      state.bates.next = batesNext;
      renderBatch();
    }
    const done = state.files.filter(row => row.status === "ok" || row.status === "failed");
    const ok = done.filter(row => row.status === "ok").length;
    const lines = done.map(row => [row.name, row.status, row.pages, row.detail,
      row.bates ? `${String(row.bates.from).padStart(pad, "0")}-${String(row.bates.to).padStart(pad, "0")}` : ""].join("\t") + "\n").join("");
    let reportText = "";
    if (lines) {
      const bytes = new TextEncoder().encode(lines);
      let written: { ok: boolean; reason?: string };
      try {
        written = await io("report.txt", "write", bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer);
      } catch (err) {
        written = { ok: false, reason: messageOf(err) };
      }
      state.report = written.ok ? "ok" : written.reason ?? "unwritable";
      reportText = written.ok ? ", report in pdf4all-out" : `; report not written: ${state.report === "exists" ? "report.txt exists" : state.report}`;
    }
    const text = `Batch ${state.cancelled ? "cancelled" : "done"}: ${ok} ok, ${done.length - ok} failed${reportText}`;
    batchSummaryEl.textContent = text;
    announce(text);
  } finally {
    state.running = false;
    batchCancelAfter = null;
    // A second Start continues the counter (contract-batch.md §9.13, D1), never the stale prefill.
    batchBatesStartInput.value = String(batesNext);
    batchBatesPadInput.value = String(batesPad);
    renderBatch();
  }
}

/**
 * Drive `batch {kind, text, pages?, align?, batesStart?, batesPad?, cancelAfter?}` with the line-level
 * `folder` pre-answer (§9.9): pick, fill the fields, run; resolves after the whole queue.
 */
async function driveBatch(contents: string): Promise<void> {
  clearError();
  if (!contents) {
    showError("batch: contents required");
    return;
  }
  const wanted = driveWanted(contents);
  if (!wanted) return;
  if (batchState?.running) {
    showError("A batch is already running");
    return;
  }
  const kind = wanted.kind;
  if (kind !== "watermark" && kind !== "header" && kind !== "footer") {
    showError("batch: kind must be watermark, header or footer");
    return;
  }
  if ("align" in wanted && wanted.align !== "left" && wanted.align !== "right") {
    showError("Align must be left or right");
    return;
  }
  const after = wanted.cancelAfter;
  if (after !== undefined && (!Number.isInteger(after) || (after as number) < 0)) {
    showError("batch: cancelAfter must be a whole number");
    return;
  }
  if (!(await pickBatchFolder())) {
    if (errorEl.hidden) showError("batch: no folder was given");
    return;
  }
  batchKindSelect.value = kind;
  batchTextInput.value = typeof wanted.text === "string" ? wanted.text : "";
  batchPagesInput.value = typeof wanted.pages === "string" ? wanted.pages : "";
  batchAlignSelect.value = wanted.align === "right" ? "right" : "left";
  batchBatesStartInput.value = "batesStart" in wanted ? String(wanted.batesStart) : String(batesNext);
  batchBatesPadInput.value = "batesPad" in wanted ? String(wanted.batesPad) : String(batesPad);
  batchCancelAfter = after === undefined ? null : after as number;
  if (!(await startBatch())) batchCancelAfter = null;
}

function openVerifyDialog() {
  if (!verifyDialog.open) verifyDialog.showModal();
  return refreshVerify();
}

async function refreshVerify(): Promise<void> {
  if (!api?.verifySignatures) return;
  verifySummary.textContent = "Verifying…";
  const refuse = (reason: string) => { const message = verifyRefusal(reason); verifySummary.textContent = message; verifyLast = { op: "verify", ok: false, reason }; showError(message); verifyByField.clear(); paintHits(canvas.clientWidth, canvas.clientHeight, shownPage); };
  // Worker refusals reject (encrypted, too-large, no-document); they must still reach the dialog and STATE.
  let listed: { signatures?: Record<string, unknown>[]; certified?: number | null; bytes?: ArrayBuffer };
  try { listed = await request({ op: "sign-list" }) as typeof listed; } catch (err) { refuse(refusalOf(err).reason ?? "unreadable"); return; }
  const result = await api.verifySignatures(listed.bytes ?? new ArrayBuffer(0), listed.signatures ?? []).catch(() => ({ ok: false as const, reason: "unreadable" }));
  if (!result.ok) { refuse(result.reason); return; }
  const rows = result.verdicts;
  const nonempty = rows.filter(row => row.status !== "empty");
  const counted = nonempty.filter(row => row.status === "valid" && row.trust === "trusted");
  const hasPartial = counted.some(row => row.coverage === "partial");
  verifySummary.textContent = nonempty.length
    ? `${counted.length} of ${nonempty.length} signatures valid and trusted${hasPartial ? "; later changes were not verified" : ""}`
    : (rows.length ? "No signed signatures" : "No signature fields");
  verifyCertified.textContent = listed.certified == null ? "" : `Certified (P=${listed.certified})`;
  verifyList.replaceChildren();
  for (const row of rows) {
    const tr = document.createElement("tr");
    tr.dataset.field = String(row.field ?? "");
    // The verifier's plain-English sentence (crates/tandem-sign/src/report.rs), shown on hover and to screen readers.
    if (typeof row.plain === "string" && row.plain) { tr.title = row.plain; tr.setAttribute("aria-description", row.plain); }
    const reason = String(row.reason ?? "");
    const cells = [
      `${String(row.field ?? "")} / ${Number(row.page) || 0}`,
      String(row.signerCn ?? "—"), String(row.signingTime ?? "—"),
      verifyStatusText(row),
      row.trust === "expired" ? "certificate not valid at claimed signing time" : row.trust === "trusted" ? `trusted by ${String(row.issuerCn ?? "certificate")}` : String(row.trust ?? "none"),
      row.coverage === "partial" ? `first ${Number(row.covered)} of ${Number(row.total)} bytes — later changes were not verified` : row.coverage === "entire-file" ? "whole file" : "—",
      reason,
    ];
    for (const text of cells) { const td = document.createElement("td"); td.textContent = text; tr.append(td); }
    verifyList.append(tr);
  }
  verifyTrusted.replaceChildren();
  for (const cert of result.trusted) { const li = document.createElement("li"); li.textContent = cert.cn; verifyTrusted.append(li); }
  verifyLast = { op: "verify", ok: true, certified: listed.certified ?? null, trusted: result.trusted.length,
    summary: verifySummary.textContent,
    signatures: rows.map(row => ({ field: row.field, status: row.status, trust: row.trust, coverage: row.coverage, signerCn: row.signerCn, reason: row.reason, cell: verifyStatusText(row), plain: row.plain })) };
  // P5 U3 §9.18: the badges follow the rows just shown.
  verifyByField.clear();
  for (const row of nonempty) verifyByField.set(String(row.field ?? ""), String(row.status ?? "unsupported"));
  paintHits(canvas.clientWidth, canvas.clientHeight, shownPage);
}

/** §2.2: the Verify row of the signed field that was clicked is the current one. */
function markVerifyRow(name: string): void {
  for (const row of verifyList.querySelectorAll<HTMLElement>("tr")) {
    if (row.dataset.field === name) row.setAttribute("aria-current", "true"); else row.removeAttribute("aria-current");
  }
}

/** The status cell: `reason` is space-joined notes, so weak-digest may sit beside no-signing-time. */
function verifyStatusText(row: Record<string, unknown>): string {
  const status = String(row.status ?? "unsupported");
  return status === "valid" && String(row.reason ?? "").split(" ").includes("weak-digest") ? "valid (weak SHA-1 digest)" : status;
}

function verifyRefusal(reason: string): string {
  return ({
    "too-large": "This document is too large to verify in this version",
    encrypted: "Encrypted signed documents cannot be verified in this version",
    "no-document": "Open a document before verifying signatures",
    unreadable: "The signatures could not be read",
    "too-many": "There are too many signatures to verify in this version",
  } as Record<string, string>)[reason] ?? reason;
}

async function driveVerify(contents: string | null): Promise<void> {
  clearError();
  const wanted = driveWanted(contents);
  if (!wanted) { verifyLast = { op: "verify", ok: false, reason: "invalid" }; return; }
  if (wanted.forget === true) await api?.trustClear?.();
  if (typeof wanted.trust === "string") {
    const added = await api?.trustAdd?.({ path: wanted.trust });
    if (!added?.ok) { verifyLast = { op: "verify", ok: false, reason: added?.reason ?? "unreadable" }; return; }
  }
  await refreshVerify();
}

/** P5 U3 §9.15: field mode hides the page and appearance inputs (the field is both). */
function setCertSignMode(field: FieldItem | null): void {
  certSignField = field ? { name: field.name, page: wantedPage } : null;
  certSignFieldEl.hidden = !field;
  certSignFieldEl.replaceChildren();
  if (field) certSignFieldEl.append("Signing field ", Object.assign(document.createElement("b"), { textContent: field.name }), ` on page ${wantedPage + 1}`);
  const pageRow = certSignPage.closest("p") as HTMLElement | null;
  if (pageRow) pageRow.hidden = !!field;
  const appearance = certSignVisible.closest("fieldset") as HTMLElement | null;
  if (appearance) appearance.hidden = !!field;
}

function openCertSignDialog(field?: FieldItem) {
  if (!doc) { showError("Sign with certificate needs an open PDF."); return; }
  setCertSignMode(field ?? null);
  certSignPage.value = String(wantedPage + 1);
  const size = doc.pages[wantedPage];
  certSignX.value = String(Math.max(0, size.width - 236));
  certSignY.value = String(Math.max(0, size.height - 96));
  // A certified document accepts only an invisible signature (engine refuses a visible one).
  certSignVisible.checked = !doc.info.certified;
  certSignVisible.disabled = doc.info.certified;
  certSignBox.hidden = doc.info.certified;
  const canCertify = doc.info.signatures === 0;
  certSignCertify.disabled = !canCertify;
  certSignCertify.checked = false;
  certSignCertifyReason.textContent = canCertify ? "" : "Certification must be the first signature.";
  certSignStatus.textContent = "";
  if (!certSignDialog.open) certSignDialog.showModal();
  certSignPickButton.focus();
}

async function pickCertSignFile(path?: string) {
  if (!api?.pickCertificate) { certSignStatus.textContent = "Certificate signing needs the desktop app."; return; }
  const picked = await api.pickCertificate(path ? { path } : undefined);
  if (!picked.ok) {
    if (picked.reason !== "cancelled") certSignStatus.textContent = "Could not read that certificate file.";
    return;
  }
  certSignPath = path ?? picked.name;
  certSignFileEl.textContent = picked.name;
  certSignStatus.textContent = "";
}

async function runCertSign() {
  if (!doc || !certSignPath || !api?.certificateInfo || !api.signFile) return;
  const field = certSignField;
  const page = field ? field.page : Number(certSignPage.value) - 1;
  const reason = certSignReason.value;
  const when = Date.now();
  if (!Number.isInteger(page) || page < 0 || page >= doc.pageCount || reason.length > 200 || /[\x00-\x1f\x7f]/.test(reason)) {
    signLast = { op: "sign", ok: false, reason: "invalid" };
    certSignStatus.textContent = "Enter a valid page and reason."; return;
  }
  let cert: Awaited<ReturnType<NonNullable<Pdf4AllApi["certificateInfo"]>>>;
  const password = certSignPassword.value;
  certSignGo.disabled = true;
  try {
    const pending = api.certificateInfo(password);
    cert = await pending;
  } catch (err) {
    certSignStatus.textContent = messageOf(err); certSignGo.disabled = false; return;
  } finally { certSignPassword.value = ""; }
  if (!certSignDialog.open) { await api.certificateClear?.(); certSignGo.disabled = false; return; }
  if (!cert.ok) {
    signLast = { op: "sign", ok: false, reason: cert.reason };
    certSignStatus.textContent = cert.status ?? (cert.reason === "bad-password" ? "The certificate password is incorrect." : `Certificate refused: ${cert.reason}.`);
    await api.certificateClear?.();
    certSignGo.disabled = false;
    return;
  }
  let prepared: { ok: boolean; error?: string; reason?: string; bytes?: ArrayBuffer; contentsAt?: number; contentsLen?: number; byteRangeAt?: number; revisions?: number };
  try {
    const rect = !field && certSignVisible.checked ? [Number(certSignX.value), Number(certSignY.value), Number(certSignX.value) + Number(certSignW.value), Number(certSignY.value) + Number(certSignH.value)] : null;
    prepared = await request({ op: "sign-prepare", name: cert.cn, reason, page, rect, certify: certSignCertify.checked, when, ...(field ? { field: field.name } : {}) }, [], SAVE_TIMEOUT_MS) as typeof prepared;
  } catch (err) {
    await api.certificateClear?.();
    signLast = { op: "sign", ok: false, reason: refusalOf(err).reason ?? "invalid" };
    certSignStatus.textContent = messageOf(err);
    certSignGo.disabled = false;
    return;
  }
  if (!certSignDialog.open) { await api.certificateClear?.(); certSignGo.disabled = false; return; }
  if (!prepared.ok) {
    await api.certificateClear?.();
    signLast = { op: "sign", ok: false, reason: prepared.reason ?? "invalid" };
    certSignStatus.textContent = prepared.error ?? prepared.reason ?? "Signing was refused.";
    setStatus(certSignStatus.textContent);
    certSignGo.disabled = false;
    return;
  }
  certSignInFlight = true;
  const sourceName = doc.name;
  const suggested = sourceName.replace(/\.pdf$/i, "") + (certSignCertify.checked ? " certified.pdf" : " signed.pdf");
  let signed: Awaited<ReturnType<NonNullable<Pdf4AllApi["signFile"]>>>;
  try {
    signed = await api.signFile(prepared.bytes as ArrayBuffer, {
      contentsAt: Number(prepared.contentsAt), contentsLen: Number(prepared.contentsLen), byteRangeAt: Number(prepared.byteRangeAt),
    }, when, suggested);
  } catch (err) {
    await api.certificateClear?.();
    certSignStatus.textContent = messageOf(err);
    certSignInFlight = false;
    certSignGo.disabled = false;
    return;
  }
  certSignInFlight = false;
  certSignGo.disabled = false;
  if (!signed.ok) {
    signLast = { op: "sign", ok: false, reason: signed.reason };
    const text = newFileReasonText(signed.reason);
    certSignStatus.textContent = text ? `Signing failed. ${text}` : `Signing failed: ${signed.reason}.`;
    return;
  }
  signLast = { op: "sign", ok: true, name: signed.name, signer: signed.signer, revisions: Number(prepared.revisions), ...(field ? { field: field.name } : {}) };
  certSignPath = null;
  certSignDialog.close();
  const status = `Signed as ${signed.signer} → ${signed.name}. The open document is unchanged.`;
  certSignStatus.textContent = status;
  setStatus(status);
  announce(status);
}

// ----------------------------------------------------------------- security

/**
 * The dialog mirrors the worker's security state and never keeps a password: the
 * two fields are cleared on submit and on close, and only the request carries them.
 */
function openSecurityDialog() {
  if (!doc) return;
  const allowed = canChangeSecurity();
  const info = doc.info;
  const current = securityState;
  securityCurrentEl.textContent = !info.encrypted
    ? "Currently: no encryption"
    : `Currently: ${ALGORITHM_LABEL[info.algorithm] ?? info.algorithm}, opened with the ${auth === "owner" ? "owner" : "user"} password`;
  const mode = current?.mode === "none" ? "none" : "aes-256";
  securityModeNoneInput.checked = mode === "none";
  securityModeAesInput.checked = mode === "aes-256";
  securityUserInput.value = "";
  securityOwnerInput.value = "";
  const granted = current?.mode === "aes-256" ? current.permissions : permissionNames(info.permissions);
  for (const [name, box] of permissionBoxes) box.checked = granted.includes(name);
  securityStatusEl.textContent = allowed ? "" : securityRefusal();
  securityApplyButton.disabled = !allowed;
  syncSecurityMode();
  securityDialog.showModal();
  if (allowed) (mode === "none" ? securityModeNoneInput : securityUserInput).focus();
}

/** AES-256 needs a password pair; "No encryption" does not. */
function syncSecurityMode() {
  const allowed = canChangeSecurity();
  const aes = securityModeAesInput.checked;
  const boxes = permissionBoxes.map(([, box]) => box);
  for (const control of [securityModeNoneInput, securityModeAesInput, ...boxes]) control.disabled = !allowed;
  securityUserInput.disabled = !allowed || !aes;
  securityOwnerInput.disabled = !allowed || !aes;
  securityApplyButton.disabled = !allowed;
}

/**
 * One `security-set`: the worker stores the pending setting and marks the document
 * dirty. The change lands in the file on the next save; it is not journaled.
 */
async function applySecurityDialog() {
  if (!doc) return;
  const mode = securityModeAesInput.checked ? "aes-256" : "none";
  const permissions = Object.fromEntries(permissionBoxes.map(([name, box]) => [name, box.checked]));
  const message: Record<string, unknown> = { op: "security-set", mode, permissions };
  if (mode === "aes-256") {
    message.userPassword = securityUserInput.value;
    message.ownerPassword = securityOwnerInput.value;
  }
  // The request is the only place the passwords live from here on.
  securityUserInput.value = "";
  securityOwnerInput.value = "";
  let reply: SecurityReply;
  try {
    reply = (await request(message)) as SecurityReply;
  } catch (err) {
    securityStatusEl.textContent = messageOf(err);
    return;
  }
  applySecurityReply(reply);
  securityDialog.close();
  announce("Security will change when you save");
  logEdit();
}

/** The edit-state slice a `security-set` reply carries, mirrored like any mutation. */
function applySecurityReply(reply: SecurityReply): void {
  securityState = reply.security;
  revision = reply.revision ?? revision;
  dirty = reply.dirty ?? dirty;
  canUndo = reply.canUndo ?? canUndo;
  canRedo = reply.canRedo ?? canRedo;
  syncDirty();
  syncControls();
  if (doc) renderBanner(doc.info);
  // security-set moves the revision without a journal step: rescan as
  // `applyEdit` does, so text edits and replace previews name the new one.
  void refreshContentTargets(uiMode === "edit");
}

/**
 * After a write in place landed, the document on disk is the pending security and
 * the banner must stop saying it will change. Save a copy leaves the pending change
 * in place: the open file itself has not been rewritten.
 */
function markSecuritySaved(): void {
  if (!securityState) return;
  securityState = { ...securityState, pending: false };
  if (doc) {
    if (securityState.mode === "none") {
      doc.info = {
        ...doc.info, encrypted: false, needsPassword: false, algorithm: "none",
        permissions: { print: true, copy: true, edit: true, annotate: true, form: true },
      };
    } else if (securityState.mode === "aes-256") {
      const permissions: DocumentPermissions = { print: false, copy: false, edit: false, annotate: false, form: false };
      for (const name of securityState.permissions) permissions[name] = true;
      doc.info = { ...doc.info, encrypted: true, needsPassword: false, algorithm: "aes-256", permissions };
    }
    renderBanner(doc.info);
  }
}

/** The smoke's `--edit=encrypt` payload: `{"user":…,"owner":…,"permissions":[…]}`. */function parseSecurityPayload(contents: string | null): Record<string, unknown> | null {
  if (!contents) return null;
  const parsed = JSON.parse(contents) as { user?: unknown; owner?: unknown; permissions?: unknown };
  return {
    mode: "aes-256",
    userPassword: parsed.user,
    ownerPassword: parsed.owner,
    permissions: Object.fromEntries(
      ((Array.isArray(parsed.permissions) ? parsed.permissions : []) as string[]).map(name => [name, true]),
    ),
  };
}

/** `--edit=encrypt` with `--new-passwords`: apply AES-256, then save a copy. */
async function smokeEncrypt(contents: string | null): Promise<void> {
  let setting: Record<string, unknown> | null = null;
  try {
    setting = parseSecurityPayload(contents);
  } catch (err) {
    showError(`encrypt: ${messageOf(err)}`);
    return;
  }
  if (!setting) return;
  try {
    applySecurityReply((await request({ op: "security-set", ...setting })) as SecurityReply);
  } catch (err) {
    showError(messageOf(err));
    return;
  }
  logEdit();
  await saveAs();
}

/**
 * `--edit=encrypt-keep` [contents JSON `{password, note}` or just the password]:
 * unlock if needed, add one note, then save a copy with the encryption preserved.
 */
async function smokeEncryptKeep(contents: string | null): Promise<void> {
  let password: string | undefined;
  let note: string | undefined;
  if (contents && contents.trim().startsWith("{")) {
    try {
      const parsed = JSON.parse(contents) as { password?: string; note?: string };
      password = parsed.password;
      note = parsed.note;
    } catch (err) {
      showError(`encrypt-keep: ${messageOf(err)}`);
      return;
    }
  } else if (contents) password = contents;
  if (password) await smokePassword(password);
  if (note && doc) {
    const size = doc.pages[wantedPage];
    if (size) await addNoteAt(wantedPage, [size.width / 2 - NOTE_SIZE / 2, size.height / 2 - NOTE_SIZE / 2], note);
  }
  logEdit();
  await saveAs();
}

/** The smoke's unlock path: the CLI password goes to the worker and nowhere else. */
async function smokePassword(value: string | null): Promise<void> {
  if (!doc || !doc.info.needsPassword || typeof value !== "string" || !value) return;
  let reply: AuthenticateReply;
  try {
    reply = (await request({ op: "authenticate", password: value })) as AuthenticateReply;
  } catch (err) {
    showError(messageOf(err));
    return;
  }
  if (!reply.authenticated) {
    showError("The document password was not accepted.");
    return;
  }
  await applyUnlocked(reply);
}

// ----------------------------------------------------------------- controls

function wireControls() {
  openButton.addEventListener("click", () => void requestOpen());
  saveButton.addEventListener("click", () => void saveInPlace());
  saveCopyButton.addEventListener("click", () => void saveAs());
  // §1.5: a click must not blur (and so commit) an open editor before undoEdit closes it.
  for (const button of [undoButton, redoButton]) button.addEventListener("mousedown", event => { if (directActive) event.preventDefault(); });
  // A disabled Undo gets no mousedown or click, but the press still blurs the editor: close it first, no request.
  window.addEventListener("pointerdown", event => { if (directActive && event.target instanceof Element && (event.target.closest("#undo, #redo") as HTMLButtonElement | null)?.disabled) { closeDirect(); directFocusedField = null; } }, true);
  undoButton.addEventListener("click", () => void undoEdit());
  redoButton.addEventListener("click", () => void redoEdit());
  noteAddButton.addEventListener("click", event => {
    // Keyboard activation carries no click point (detail 0): place at the page
    // centre. A pointer click arms press-then-click-on-the-page instead.
    if (event.detail === 0) {
      const size = doc?.pages[wantedPage];
      if (size) void addNoteAt(wantedPage, [size.width / 2 - NOTE_SIZE / 2, size.height / 2 - NOTE_SIZE / 2], "");
      return;
    }
    const pressed = noteAddButton.getAttribute("aria-pressed") === "true";
    noteAddButton.setAttribute("aria-pressed", String(!pressed));
    announce(!pressed ? "Add note: click the page where the note should go" : "");
    paintHits(canvas.clientWidth, canvas.clientHeight, shownPage);
  });
  for (const [button, type, announcement] of markupButtons) {
    button.addEventListener("click", () => {
      const hit = currentHit();
      if (!hit) return;
      void addMarkup(hit.page, type, hit.quads, announcement);
    });
  }
  freeTextAddButton.addEventListener("click", event => {
    // Keyboard activation (detail 0) opens the dialog at the page centre; a pointer
    // click arms press-then-click-on-the-page so the box lands where it is wanted.
    if (event.detail === 0) {
      const size = doc?.pages[wantedPage];
      if (size) openTextBoxDialog([size.width / 2, size.height / 2]);
      return;
    }
    const pressed = freeTextAddButton.getAttribute("aria-pressed") === "true";
    freeTextAddButton.setAttribute("aria-pressed", String(!pressed));
    announce(!pressed ? "Add text box: click the page where the box should go" : "");
    paintHits(canvas.clientWidth, canvas.clientHeight, shownPage);
  });
  for (const [button, type, announcement] of shapeButtons) {
    button.addEventListener("click", () => {
      const box = shapeBox();
      if (box) void addShape(type, box, announcement);
    });
  }
  inkDrawButton.addEventListener("click", event => {
    // Keyboard activation (detail 0) draws the stand-in zigzag; a pointer click arms
    // a drag on the page, the same press-then-act convention the note button uses.
    const pressed = inkDrawButton.getAttribute("aria-pressed") === "true";
    inkDrawButton.setAttribute("aria-pressed", String(!pressed));
    if (event.detail === 0) {
      inkDrawButton.setAttribute("aria-pressed", "false");
      void addShapeOp("ink");
      return;
    }
    announce(!pressed ? "Draw ink: drag on the page to draw one stroke" : "");
    paintHits(canvas.clientWidth, canvas.clientHeight, shownPage);
  });
  redactAreaButton.addEventListener("click", event => {
    if (event.detail === 0) {
      const box = shapeBox();
      if (box) void addRedactArea(wantedPage, box);
      return;
    }
    const pressed = redactAreaButton.getAttribute("aria-pressed") === "true";
    redactAreaButton.setAttribute("aria-pressed", String(!pressed));
    announce(!pressed ? "Mark redaction area: drag a rectangle on the page" : "");
    paintHits(canvas.clientWidth, canvas.clientHeight, shownPage);
  });
  redactSearchText.addEventListener("input", () => { redactSearchGo.disabled = !canAnnotate() || !redactSearchText.value; });
  redactSearchText.addEventListener("keydown", event => {
    if (event.key !== "Enter") return;
    event.preventDefault();
    void addRedactSearch(redactSearchText.value);
  });
  redactSearchGo.addEventListener("click", () => void addRedactSearch(redactSearchText.value));
  redactApplyOpen.addEventListener("click", () => void openRedactDialog());
  redactApplyButton.addEventListener("click", () => void applyRedact());
  redactCancelButton.addEventListener("click", () => redactDialog.close());
  for (const key of REDACT_KEYS) redactSanitizeBoxes[key].addEventListener("change", updateRedactKept);
  stampIconButton.addEventListener("click", () => openStampIconDialog());
  // sign-merge D5: Annotate's "Sign…" is the Fill forms Sign press (switch to Fill forms; arm, or the pad first).
  signButton.addEventListener("click", () => { if (uiMode !== "forms") setMode("forms"); pressSign(); });
  stampIconSaveButton.addEventListener("click", () => void saveStampIconDialog());
  stampIconCancelButton.addEventListener("click", () => stampIconDialog.close());
  noteListEl.addEventListener("click", event => {
    const button = (event.target as HTMLElement).closest("button[data-action]");
    if (!button) return;
    const annotId = Number((button as HTMLElement).dataset.annotId);
    if (!Number.isInteger(annotId)) return;
    if ((button as HTMLElement).dataset.action === "edit") openNoteDialog(annotId);
    else void deleteNote(annotId, notes.find(note => note.id === annotId)?.type === "Redact" ? "Deleted redaction mark" : undefined);
  });
  noteSaveButton.addEventListener("click", () => void saveNoteDialog());
  noteCancelButton.addEventListener("click", () => noteDialog.close());
  noteDialog.addEventListener("close", () => { editingNote = null; boxPoint = null; });
  // U7 help surface: the menu item, the drive `help` command and this button share one dialog.
  helpCloseEl.addEventListener("click", () => helpDialog.close());
  bookmarkAddButton.addEventListener("click", () => openBookmarkDialog());
  // U6 designer: Enter on Add/Apply is the native button's own activation.
  fieldNewAddEl.addEventListener("click", () => void addDesignedField());
  fieldPropApplyEl.addEventListener("click", () => void applyDesignedField());
  fieldPropDeleteEl.addEventListener("click", () => void deleteDesignedField());
  fieldNewKindEl.addEventListener("change", () => renderDesigner());
  fieldSelectEl.addEventListener("change", () => renderSelectedField());
  bookmarkSaveButton.addEventListener("click", () => void saveBookmarkDialog());
  bookmarkCancelButton.addEventListener("click", () => bookmarkDialog.close());
  bookmarkDialog.addEventListener("close", () => { editingBookmark = null; });
  linkPageAddButton.addEventListener("click", () => openLinkDialog("page"));
  linkUriAddButton.addEventListener("click", () => openLinkDialog("uri"));
  linkSaveButton.addEventListener("click", () => void saveLinkDialog());
  linkCancelButton.addEventListener("click", () => linkDialog.close());
  linkDialog.addEventListener("close", () => { linkAnchor = null; navLinkDraft = null; });
  linkKindSelect.addEventListener("change", () => setLinkMode(linkKindSelect.value === "uri" ? "uri" : "page", true));
  linkDrawButton.addEventListener("click", () => {
    const pressed = linkDrawButton.getAttribute("aria-pressed") === "true";
    cancelPendingTools();
    linkDrawButton.setAttribute("aria-pressed", String(!pressed));
    announce(!pressed ? DRAW_LINK_PROMPT : "");
    paintHits(canvas.clientWidth, canvas.clientHeight, shownPage);
  });
  sigfieldDrawButton.addEventListener("click", () => {
    const pressed = sigfieldDrawButton.getAttribute("aria-pressed") === "true";
    cancelPendingTools();
    sigfieldDrawButton.setAttribute("aria-pressed", String(!pressed));
    announce(!pressed ? DRAW_SIGFIELD_PROMPT : "");
    paintHits(canvas.clientWidth, canvas.clientHeight, shownPage);
  });
  insertButton.addEventListener("click", () => void requestInsert());
  insertStartButton.addEventListener("click", () => void saveInsertDialog());
  insertCancelButton.addEventListener("click", () => { insertSource = null; insertDialog.close(); });
  insertDialog.addEventListener("close", () => { insertSource = null; });
  extractButton.addEventListener("click", () => requestExtract());
  extractStartButton.addEventListener("click", () => void saveExtractDialog());
  extractCancelButton.addEventListener("click", () => extractDialog.close());
  securityButton.addEventListener("click", () => openSecurityDialog());
  certSignOpenButton.addEventListener("click", () => openCertSignDialog());
  certSignPickButton.addEventListener("click", () => void pickCertSignFile());
  certSignGo.addEventListener("click", () => void runCertSign());
  certSignCancel.addEventListener("click", () => certSignDialog.close());
  verifyOpenButton.addEventListener("click", () => openVerifyDialog());
  verifyClose.addEventListener("click", () => verifyDialog.close());
  batchPickButton.addEventListener("click", () => void pickBatchFolder());
  batchStartButton.addEventListener("click", () => void startBatch());
  batchCancelButton.addEventListener("click", () => {
    if (batchState?.running) batchState.cancelled = true;
    else batchDialog.close();
  });
  batchCloseButton.addEventListener("click", () => { if (!batchState?.running) batchDialog.close(); });
  // §9.14: Escape while running cancels after the running file instead of hiding the progress.
  batchDialog.addEventListener("cancel", event => {
    if (!batchState?.running) return;
    event.preventDefault();
    batchState.cancelled = true;
  });
  batchKindSelect.addEventListener("change", () => { batchAlignSelect.disabled = batchKindSelect.value === "watermark"; });
  verifyRefresh.addEventListener("click", () => refreshVerify());
  verifyTrustAdd.addEventListener("click", async () => {
    const result = await api?.trustAdd?.();
    if (result && !result.ok && result.reason !== "cancelled") setStatus(`Trust certificates: ${result.reason}`);
    await refreshVerify();
  });
  verifyTrustClear.addEventListener("click", async () => {
    await api?.trustClear?.();
    await refreshVerify();
  });
  certSignVisible.addEventListener("change", () => { certSignBox.hidden = !certSignVisible.checked; });
  certSignDialog.addEventListener("close", () => {
    // The close event is a queued task: when the dialog was already re-opened (drive Escape then sign, P6 U5 acceptance
    // 2/2 red under a full `npm test`), acting here would drop the new attempt's field, path and held certificate.
    if (certSignDialog.open) return;
    certSignField = null;
    certSignPassword.value = "";
    if (!certSignInFlight && certSignPath) void api?.certificateClear?.();
    certSignPath = null;
    certSignFileEl.textContent = "No certificate selected";
  });
  securityApplyButton.addEventListener("click", () => void applySecurityDialog());
  securityCancelButton.addEventListener("click", () => securityDialog.close());
  securityModeNoneInput.addEventListener("change", syncSecurityMode);
  securityModeAesInput.addEventListener("change", syncSecurityMode);
  securityDialog.addEventListener("close", () => {
    securityUserInput.value = "";
    securityOwnerInput.value = "";
    securityStatusEl.textContent = "";
  });
  newImagesButton.addEventListener("click", () => void requestNewFromImages());
  exportButton.addEventListener("click", () => requestExport());
  exportKindSelect.addEventListener("change", syncExportDpi);
  exportStartButton.addEventListener("click", () => void saveExportDialog());
  exportCancelButton.addEventListener("click", () => exportDialog.close());
  syncExportDpi();
  outlineListEl.addEventListener("click", event => {
    const button = (event.target as HTMLElement).closest("button[data-action]");
    if (!button) return;
    const outlineId = Number((button as HTMLElement).dataset.outlineId);
    if (!Number.isInteger(outlineId)) return;
    const action = (button as HTMLElement).dataset.action;
    if (action === "go") followOutline(outline.find(entry => entry.id === outlineId) ?? { id: outlineId, title: "", depth: 0, page: null });
    else if (action === "rename") openBookmarkDialog(outlineId);
    else if (action === "delete") void deleteBookmark(outlineId);
  });
  linkListEl.addEventListener("click", event => {
    const button = (event.target as HTMLElement).closest("button[data-action]");
    if (!button) return;
    const linkId = Number((button as HTMLElement).dataset.linkId);
    if (!Number.isInteger(linkId)) return;
    const item = links.find(entry => entry.id === linkId);
    if (!item) return;
    const action = (button as HTMLElement).dataset.action;
    if (action === "delete") void deleteLink(linkId);
    else followLink(item);
  });
  // P6 U7 §9.30/§9.31: a selection of two or more holding a run, field, link or mark moves and deletes as one;
  // these capture listeners run before the per-target handlers (which only know their own kind).
  const memberTarget = (event: Event): HTMLElement | null => {
    const node = (event.target as HTMLElement).closest?.<HTMLElement>(".hit-target") ?? null;
    return node && hitLayer.contains(node) ? node : null;
  };
  const isMemberNode = (node: HTMLElement) => directSelection.some(member => memberNode(member) === node);
  hitLayer.addEventListener("keydown", event => {
    if (directActive || directGesture) return;
    const node = memberTarget(event);
    if (!node) return;
    if (((event.key === "F10" && event.shiftKey) || event.key === "ContextMenu") && !node.classList.contains("object-target")) {
      const member = memberOfNode(node);
      if (!member) return;
      event.preventDefault(); event.stopPropagation();
      if (!isMemberNode(node)) { clearDirectFocus(); toggleMember(member); }
      const box = node.getBoundingClientRect(); openObjectMenu(box.left, box.top);
      return;
    }
    if (directSelection.length < 2 || !selectionMixed() || !isMemberNode(node)) return;
    let work: (() => Promise<unknown>) | null = null;
    if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); dropDirectFocus(); return; }
    if (ARROW_KEYS.includes(event.key) && !event.metaKey && !event.ctrlKey) work = () => nudgeSelection(event);
    else if (event.key === "Delete" || event.key === "Backspace") work = () => deleteSelection();
    else return;
    event.preventDefault(); event.stopPropagation();
    if (!directGestureWork) runGestureWork(work);
  }, true);
  hitLayer.addEventListener("pointerdown", event => {
    if (event.button !== 0 || event.ctrlKey || event.shiftKey || directActive || (event.target as HTMLElement).closest(".hit-handle")) return;
    const node = memberTarget(event);
    if (!node || directSelection.length < 2 || !selectionMixed() || !isMemberNode(node)) return;
    startSelectionGesture(event, node);
  }, true);
  // §2.1/§9.27: a right-click (or Control-click) on a run, field, link or mark selects it alone unless it is a member.
  hitLayer.addEventListener("contextmenu", event => {
    const node = memberTarget(event);
    if (!node || node.classList.contains("object-target") || directActive) return;
    const member = memberOfNode(node);
    if (!member) return;
    event.preventDefault(); event.stopPropagation();
    if (!isMemberNode(node)) { clearDirectFocus(); toggleMember(member); }
    openObjectMenu(event.clientX, event.clientY);
  });
  // §2.1/§9.28: right-click on empty page space opens the page menu (only Paste can be enabled); it pastes there.
  pageRegion.addEventListener("contextmenu", event => {
    const target = event.target as HTMLElement;
    if (objectMenu.contains(target) || (hitLayer.contains(target) && target !== hitLayer)) return;
    if (!doc || !["edit", "forms", "annotate", "redact"].includes(uiMode) || !layerLive()) return;
    event.preventDefault();
    dropDirectFocus(); addBlocked = true;
    openObjectMenu(event.clientX, event.clientY, "page", driveMenuPoint ?? pagePoint(event));
  });
  // §10.30: the Edit menu's Cut/Copy/Paste roles and the chords reach the page selection as DOM clipboard events.
  for (const kind of ["copy", "cut", "paste"] as const) document.addEventListener(kind, event => {
    const active = document.activeElement instanceof HTMLElement && document.activeElement !== document.body ? document.activeElement : null;
    const ok = clipboardChordOk(kind, {
      mode: uiMode, layerLive: layerLive(), dialogOpen: !!document.querySelector("dialog[open]"), menuOpen: !objectMenu.hidden, editorOpen: directActive !== null,
      active: active ? { tag: active.tagName, type: (active as HTMLInputElement).type, contentEditable: active.isContentEditable, inlineEditor: active.classList.contains("inline-editor") } : null,
      collapsed: window.getSelection()?.isCollapsed ?? true, selection: directSelection.length, clipboard: !!clipboardInfo, editable: editable(),
    });
    if (!ok) return;
    event.preventDefault();
    if (!directGestureWork) runGestureWork(() => clipboardWork(kind, null));
  });
  pageRegion.addEventListener("click", event => {
    if (objectMenu.contains(event.target as Node)) return; // §9.15
    // draw-signature D5: an armed Sign outranks every other click on the page (hit-layer targets are not live while armed).
    if (signArmed && event.button === 0 && !event.ctrlKey) {
      const point = pagePoint(event); if (!point) return;
      addBlocked = false; directClickWork = placeSignature(wantedPage, point).then(() => undefined);
      return;
    }
    if (clickToAdd(event)) return;
    if (contentAddArmButton.getAttribute("aria-pressed") === "true") {
      const point = pagePoint(event);
      if (!point) return;
      contentAddPoint = point;
      contentAddArmButton.setAttribute("aria-pressed", "false");
      announce(`Text baseline picked at ${point[0].toFixed(1)}, ${point[1].toFixed(1)} pt.`);
      return;
    }
    if (freeTextAddButton.getAttribute("aria-pressed") === "true") {
      const point = pagePoint(event);
      if (!point) return;
      freeTextAddButton.setAttribute("aria-pressed", "false");
      openTextBoxDialog(point);
      return;
    }
    if (noteAddButton.getAttribute("aria-pressed") !== "true") return;
    const point = pagePoint(event);
    if (!point) return;
    noteAddButton.setAttribute("aria-pressed", "false");
    void addNoteAt(wantedPage, point, "");
  });
  // Ink by pointer: one stroke per drag, in page points, bounded like the worker's check.
  pageRegion.addEventListener("pointerdown", event => {
    if (objectMenu.contains(event.target as Node)) return; // §9.15
    // §9.9: a press with an editor open only commits/closes it; the click after it adds nothing.
    // P6 U5 §9.22: so does the press that only dismissed the object menu.
    addBlocked = !!directActive || directCommitting || event === objectMenuDismiss || event === cropDismiss || directCrop !== null;
    // P6 U4 §9.14: armed redact → armed link-draw → unarmed redact, all on the one window-listening helper.
    // P6 U7 review F2: a secondary or Control-click never starts an armed tool's gesture (§9.26/§10.38); the page menu gets it.
    if (event.button !== 0 || event.ctrlKey) return;
    const page = wantedPage;
    if (redactAreaButton.getAttribute("aria-pressed") === "true") {
      startAreaDrag(event, page, rect => addRedactArea(page, rect), () => { redactAreaButton.setAttribute("aria-pressed", "false"); paintHits(canvas.clientWidth, canvas.clientHeight, shownPage); });
      return;
    }
    if (linkDrawButton.getAttribute("aria-pressed") === "true") {
      startAreaDrag(event, page, rect => drawnLink(page, rect), () => { linkDrawButton.setAttribute("aria-pressed", "false"); announce(""); paintHits(canvas.clientWidth, canvas.clientHeight, shownPage); });
      return;
    }
    if (sigfieldDrawButton.getAttribute("aria-pressed") === "true") {
      startAreaDrag(event, page, rect => addSignatureField(page, rect), () => { sigfieldDrawButton.setAttribute("aria-pressed", "false"); announce(""); paintHits(canvas.clientWidth, canvas.clientHeight, shownPage); });
      return;
    }
    if (uiMode === "redact" && canAnnotate() && event.button === 0 && !event.ctrlKey && !(event.target as HTMLElement).closest?.(".hit-target, .hit-handle")) {
      event.preventDefault(); pageRegion.focus({ preventScroll: true });
      startAreaDrag(event, page, rect => addRedactArea(page, rect));
      return;
    }
    if (inkDrawButton.getAttribute("aria-pressed") !== "true") return;
    const point = inkPoint(event);
    if (!point) return;
    inkStroke = [point];
    pageRegion.setPointerCapture(event.pointerId);
    event.preventDefault();
  });
  pageRegion.addEventListener("pointermove", event => {
    if (!inkStroke) return;
    const point = inkPoint(event);
    if (!point) return;
    const last = inkStroke[inkStroke.length - 1];
    // A drag sample is only a new point when it moved; a stationary pointer is not 2048 points.
    if (Math.abs(point[0] - last[0]) < 0.5 && Math.abs(point[1] - last[1]) < 0.5) return;
    if (inkStroke.length >= MAX_INK_POINTS) return;
    inkStroke.push(point);
  });
  const finishInk = (event: PointerEvent) => {
    const stroke = inkStroke;
    inkStroke = null;
    if (pageRegion.hasPointerCapture(event.pointerId)) pageRegion.releasePointerCapture(event.pointerId);
    if (!stroke) return;
    inkDrawButton.setAttribute("aria-pressed", "false");
    if (stroke.length < 2) {
      announce("Ink needs a drag, not a click");
      return;
    }
    void addInk([stroke], `Added ink (${stroke.length} points)`);
  };
  pageRegion.addEventListener("pointerup", finishInk);
  pageRegion.addEventListener("pointercancel", finishInk);
  printButton.addEventListener("click", () => openPrintDialog());
  for (const [button, op] of pageToolButtons) button.addEventListener("click", () => runPageOp(op));
  watermarkButton.addEventListener("click", () => openStampDialog("watermark"));
  headerButton.addEventListener("click", () => openStampDialog("header"));
  pageLabelsButton.addEventListener("click", () => openLabelsDialog());
  stampKindSelect.addEventListener("change", syncStampHeading);
  stampSaveButton.addEventListener("click", () => void saveStampDialog());
  stampCancelButton.addEventListener("click", () => stampDialog.close());
  labelsSaveButton.addEventListener("click", () => void saveLabelsDialog());
  labelsClearButton.addEventListener("click", () => void clearLabels());
  labelsCancelButton.addEventListener("click", () => labelsDialog.close());
  printRangeInput.addEventListener("input", updatePrintRange);
  printStartButton.addEventListener("click", () => {
    const range = printRangeInput.value;
    printDialog.close();
    void startPrint(range);
  });
  printCancelButton.addEventListener("click", () => printDialog.close());
  // Same bound as the worker's own check; the HTML attribute is the fallback.
  passwordInput.maxLength = MAX_PASSWORD_CHARS;
  passwordForm.addEventListener("submit", event => void submitPassword(event));
  thumbsToggle.addEventListener("click", () => setThumbSidebar(!thumbOpen));
  // Tabs: click selects, arrows/Home/End move and select (WAI-ARIA automatic activation).
  const tablists: [HTMLButtonElement[], HTMLElement[]][] = [
    [sidebarTabs, sidebarPanels],
    [inspectorTabs, inspectorPanels],
  ];
  for (const [tabs, panels] of tablists) {
    for (const [index, tab] of tabs.entries()) {
      tab.addEventListener("click", () => {
        selectTab(tabs, panels, index);
        // A tab is also how a collapsed sidebar comes back.
        if (tabs === sidebarTabs && !sidebarOpen) setThumbSidebar(true);
        if (tabs === inspectorTabs && tab.id === "tab-text") void refreshContentTargets();
      });
      tab.addEventListener("keydown", event => {
        const step = event.key === "ArrowRight" || event.key === "ArrowDown" ? 1
          : event.key === "ArrowLeft" || event.key === "ArrowUp" ? -1 : 0;
        const next = step ? (index + step + tabs.length) % tabs.length
          : event.key === "Home" ? 0
            : event.key === "End" ? tabs.length - 1
              : null;
        if (next === null) return;
        event.preventDefault();
        selectTab(tabs, panels, next);
        tabs[next].focus();
        if (tabs === inspectorTabs && tabs[next].id === "tab-text") void refreshContentTargets();
      });
    }
  }

  // Existing content: one listbox, one form. Enter opens the form for a candidate;
  // a non-candidate row only says why it is read-only.
  contentRefreshButton.addEventListener("click", () => void refreshContentTargets(true));
  contentAddArmButton.addEventListener("click", () => {
    const armed = contentAddArmButton.getAttribute("aria-pressed") === "true";
    contentAddArmButton.setAttribute("aria-pressed", String(!armed));
    if (armed) { contentAddPoint = null; announce("Text placement cancelled."); }
    else announce("Click the page to choose the text baseline.");
    paintHits(canvas.clientWidth, canvas.clientHeight, shownPage);
  });
  contentAddGo.addEventListener("click", () => void submitContentAdd());
  contentTargetsEl.addEventListener("click", event => {
    const row = (event.target as HTMLElement).closest("li[role=option]") as HTMLElement | null;
    if (!row) return;
    const index = Number(row.dataset.index);
    if (Number.isInteger(index)) activateContentRow(index);
  });
  contentTargetsEl.addEventListener("keydown", event => {
    const count = contentScan?.items.length ?? 0;
    if (!count) return;
    const step = event.key === "ArrowDown" ? 1 : event.key === "ArrowUp" ? -1 : 0;
    if (step) {
      event.preventDefault();
      selectContentRow(Math.min(Math.max(0, contentIndex + step), count - 1));
      return;
    }
    if (event.key === "Home" || event.key === "End") {
      event.preventDefault();
      selectContentRow(event.key === "Home" ? 0 : count - 1);
      return;
    }
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      activateContentRow(contentIndex < 0 ? 0 : contentIndex);
    }
  });
  contentForm.addEventListener("submit", event => {
    event.preventDefault();
    void submitContentEdit(contentNewText.value);
  });
  contentDeleteButton.addEventListener("click", () => void submitContentEdit(""));
  contentCancelButton.addEventListener("click", () => closeContentForm());
  contentForm.addEventListener("keydown", event => {
    if (event.key !== "Escape") return;
    event.preventDefault();
    closeContentForm();
  });
  contentObjectForm.addEventListener("submit", event => {
    event.preventDefault();
    void submitObjectEdit("box");
  });
  contentReplaceButton.addEventListener("click", () => void submitObjectEdit("replace"));
  contentDeleteObjectButton.addEventListener("click", () => void submitObjectEdit("delete"));
  for (const [name, button] of Object.entries(contentTransformButtons) as [keyof typeof contentTransformButtons, HTMLButtonElement][]) button.addEventListener("click", () => inspectorTransform(name, button));
  contentCancelObjectButton.addEventListener("click", () => closeContentObjectForm());
  contentObjectForm.addEventListener("keydown", event => {
    if (event.key !== "Escape") return;
    event.preventDefault();
    // P6 U6 review R3 (§2.3): Escape leaves crop mode first, as the drive's key Escape does.
    if (directCrop) { leaveCrop(); return; }
    closeContentObjectForm();
  });
  contentPreviewButton.addEventListener("click", () => void previewReplace());
  contentReplaceApply.addEventListener("click", () => void applyReplace());
  contentReplaceCancel.addEventListener("click", () => { clearReplacePreview(); contentFindInput.focus(); });
  for (const input of [contentFindInput, contentReplaceWith]) {
    input.addEventListener("keydown", event => {
      if (event.key !== "Enter") return;
      event.preventDefault();
      void previewReplace();
    });
  }
  for (const [input, mode] of modeInputs) {
    input.addEventListener("change", () => { if (input.checked) setMode(mode); });
  }
  inspectorToggle.addEventListener("click", () => setInspectorOpen(!inspectorOpen));
  thumbsEl.addEventListener("scroll", () => pumpThumbs());
  prevButton.addEventListener("click", () => goTo(wantedPage - 1));
  nextButton.addEventListener("click", () => goTo(wantedPage + 1));
  pageInput.addEventListener("change", () => {
    goTo(Number(pageInput.value) - 1);
    // Enter commits the page, so leave the field: a focused number input eats the Page
    // menu's ⌥⌘↑/↓ (value steps) and ⇧⌘⌫ (delete to line start) before the accelerator.
    pageInput.blur();
  });
  zoomOutButton.addEventListener("click", () => zoomStep(-1));
  zoomInButton.addEventListener("click", () => zoomStep(1));
  fitWidthButton.addEventListener("click", () => setZoom("fit-width"));
  fitPageButton.addEventListener("click", () => setZoom("fit-page"));
  zoom100Button.addEventListener("click", () => setZoom("fixed", 1));
  searchInput.addEventListener("input", onSearchInput);
  searchInput.addEventListener("keydown", event => {
    if (event.key !== "Enter") return;
    event.preventDefault();
    if (searchHits.length) moveHit(event.shiftKey ? -1 : 1);
    else void startSearch();
  });
  searchPrevButton.addEventListener("click", () => moveHit(-1));
  searchNextButton.addEventListener("click", () => moveHit(1));
  // Compare (§9.14): PageUp/PageDown turn A (B follows), Escape leaves; every control has its drive twin.
  compareView.addEventListener("keydown", event => {
    if (event.target === comparePageBInput) return;
    if (event.key === "PageUp") goTo(wantedPage - 1);
    else if (event.key === "PageDown") goTo(wantedPage + 1);
    else if (event.key === "Escape") void closeCompare();
    else return;
    event.preventDefault();
    event.stopPropagation();
  });
  compareShowDiffInput.addEventListener("change", () => setCompareShowDiff(compareShowDiffInput.checked));
  comparePageBInput.addEventListener("change", () => {
    const page = Number(comparePageBInput.value);
    // §9.19(g): an emptied field reverts (Number("") is 0, which would commit page 1).
    if (comparePageBInput.value.trim() !== "" && Number.isInteger(page)) void setComparePageB(page - 1);
    else if (compareDoc) comparePageBInput.value = String(compareDoc.pageB + 1);
  });
  compareCloseButton.addEventListener("click", () => void closeCompare());
  pageRegion.addEventListener("keydown", event => {
    // draw-signature D5: while armed, Enter places at the page centre and Escape disarms.
    if (signArmed && (event.key === "Enter" || event.key === "Escape")) { event.preventDefault(); if (event.key === "Escape") disarmSign(); else void placeSignatureAtCentre(); return; }
    // Arrows, Home and End belong to an inline editor or a focused field target, not page turns.
    if (hitLayer.contains(event.target as Node)) return;
    const key = event.key;
    // P6 U7 §9.28: Shift+F10 / the Menu key on the page opens the page menu at the view's corner (no paste point).
    if (((key === "F10" && event.shiftKey) || key === "ContextMenu") && !objectMenu.contains(event.target as Node)) {
      if (!doc || !["edit", "forms", "annotate", "redact"].includes(uiMode) || !layerLive()) return;
      event.preventDefault(); dropDirectFocus();
      const box = viewport.getBoundingClientRect(); openObjectMenu(box.left, box.top, "page", null);
      return;
    }
    if (key === "PageUp" || key === "ArrowLeft") goTo(wantedPage - 1);
    else if (key === "PageDown" || key === "ArrowRight") goTo(wantedPage + 1);
    else if (key === "Home") goTo(0);
    else if (key === "End") goTo((doc?.pageCount ?? 1) - 1);
    else return;
    event.preventDefault();
  });
  document.addEventListener("keydown", event => {
    // §8.9: an open help dialog owns the keyboard; its native cancel closes it on Escape.
    if (helpDialog.open) return;
    if (event.key === "Escape") {
      if (printing) {
        event.preventDefault();
        cancelPrint();
      }
      return;
    }
    if (!(event.metaKey || event.ctrlKey)) return;
    const key = event.key.toLowerCase();
    // Cmd/Ctrl+Z, Shift+Cmd/Ctrl+Z and Cmd/Ctrl+S are main's menu accelerators;
    // they arrive as edit-request/save-request. Only keys with no menu item are
    // handled here, so one keystroke has exactly one owner.
    if (key === "t" && event.shiftKey) {
      event.preventDefault();
      if (!thumbsToggle.disabled) setThumbSidebar(!thumbOpen);
      return;
    }
    if (key === "o") {
      event.preventDefault();
      void requestOpen();
    } else if (key === "=" || key === "+") {
      event.preventDefault();
      zoomStep(1);
    } else if (key === "-" || key === "_") {
      event.preventDefault();
      zoomStep(-1);
    } else if (key === "0") {
      event.preventDefault();
      setZoom("fit-width");
    } else if (key === "f") {
      event.preventDefault();
      searchInput.focus();
      searchInput.select();
    }
  });
  // Fit modes follow the page region; one rAF per burst of resize events.
  let queued = false;
  const observer = new ResizeObserver(() => {
    if (queued) return;
    queued = true;
    requestAnimationFrame(() => {
      queued = false;
      syncZoom();
      void drain();
    });
  });
  observer.observe(viewport);
}

async function requestOpen() {
  if (!api || typeof api.openFile !== "function") {
    showError("Opening files needs the desktop app (window.pdf4all is unavailable).");
    return;
  }
  clearError();
  await api.openFile();
}

// ----------------------------------------------------------------- drop zone

let dropDepth = 0;
let dropBusy = false;

/**
 * Electron navigates the window to `file://` if a drag is not cancelled, so
 * every one of these cancels the default. The renderer never reads the File: it
 * hands it to main, which is the only side that may resolve a path.
 */
function wireDropZone() {
  const dragging = (event: DragEvent): boolean => {
    const types = event.dataTransfer?.types;
    return !!types && Array.from(types).includes("Files");
  };
  document.addEventListener("dragenter", event => {
    if (!dragging(event)) return;
    event.preventDefault();
    dropDepth += 1;
    dropHint.hidden = false;
  });
  document.addEventListener("dragover", event => {
    event.preventDefault();
    if (!dragging(event)) return;
    if (event.dataTransfer) event.dataTransfer.dropEffect = "copy";
    dropHint.hidden = false;
  });
  document.addEventListener("dragleave", () => {
    // Some drags report no types on the way out, so this is counted, not filtered.
    dropDepth = Math.max(0, dropDepth - 1);
    if (dropDepth === 0) dropHint.hidden = true;
  });
  document.addEventListener("drop", event => {
    event.preventDefault();
    dropDepth = 0;
    dropHint.hidden = true;
    const file = event.dataTransfer?.files?.[0];
    if (file) void openDropped(file);
  });
}

async function openDropped(file: File) {
  if (dropBusy) return; // one drop at a time per window
  if (!api || typeof api.openDropped !== "function") {
    showError("Opening a dropped file needs the desktop app (window.pdf4all is unavailable).");
    return;
  }
  dropBusy = true;
  clearError();
  setStatus(`Opening ${file.name}…`);
  try {
    await api.openDropped(file);
  } catch {
    showError(`Could not read ${file.name}.`);
  } finally {
    dropBusy = false;
  }
}

// ----------------------------------------------------------------- save a copy

// M10: each text says what happened and that the file on disk is unchanged; in-place Save and Save As share them.
const SAVE_REASON_TEXT: Record<string, string> = {
  "no-document": "There is no document open to save.",
  "read-only": "This document is signed or certified, so saving a copy is refused in this version.",
  encrypted: "This document is encrypted; saving an encryption-preserving copy comes in a later unit.",
  invalid: "The saved bytes did not reopen as the same document, so nothing was written.",
  worker: "The PDF worker stopped responding, so nothing was written.",
  ...({
    cancelled: "Save cancelled.",
    "too-large": `Tandem PDF can open documents up to ${MAX_PDF_BYTES / (1024 * 1024)} MiB, but saving is capped at ${MAX_SAVE_BYTES / (1024 * 1024)} MiB in this version.`,
    "same-file": "That would overwrite the open document; choose a different name.",
    "document-changed": "The open document changed while saving, so nothing was written.",
    "source-changed": "The file changed on disk since you opened it, so it was not overwritten. Use Save As instead.",
    "disk-full": "There is not enough disk space to save. The file on disk was not changed.",
    permission: "The file or its folder is read-only, or Tandem PDF is not allowed to write there. The file on disk was not changed; use Save As to save elsewhere.",
    unreadable: "The file could not be written. The file on disk was not changed.",
    "write-failed": "The file could not be written. The file on disk was not changed.",
    busy: "Another save is already running.",
    "broken-link": "That name is a link that does not lead to a file (its target is missing, or the link points back to itself), so nothing was written. Choose another name.",
  } satisfies Record<SaveFail, string>),
};

/** A new file's write failure (export, new PDF, signed copy) as text; there is no Save As there. */
function newFileReasonText(reason: string): string | undefined {
  if (reason === "permission") return "The folder or file is read-only, or Tandem PDF is not allowed to write there. Choose another folder.";
  if (reason === "disk-full" || reason === "broken-link") return SAVE_REASON_TEXT[reason];
  return undefined;
}

/**
 * Plain Save: the same worker transaction as Save a copy, but main rewrites the
 * source file in place (source-changed blocks instead of warning) and the worker
 * only becomes clean once main reports the write landed.
 */
async function saveInPlace() {
  if (!api || typeof api.saveFile !== "function") {
    showError("Saving needs the desktop app (window.pdf4all is unavailable).");
    return;
  }
  if (!doc) return;
  if (!doc.url) {
    showError("This is the built-in fixture, not a file on disk; open a file to save.");
    return;
  }
  if (!editable()) {
    showError("This document is read-only, so it cannot be saved in place.");
    return;
  }
  if (!dirty) {
    setStatus("No changes to save");
    return;
  }
  if (saving) {
    setStatus("A save is already running…");
    return;
  }
  const name = doc.name;
  const url = doc.url;
  const startedGeneration = generation;
  const superseded = () => startedGeneration !== generation || doc?.url !== url;
  let reason = "worker";
  saving = true;
  clearError();
  clearWarning();
  syncControls();
  setStatus(`Saving ${name}…`);
  const releaseGate = workerGate.hold();
  let write: typeof diskWrite = null;
  let landed = false;
  try {
    const save = (await request({ op: "save" }, [], SAVE_TIMEOUT_MS, true)) as SaveReply;
    if (superseded()) return;
    const validated = (await request({ op: "validate", pdf: save.bytes }, [save.bytes], SAVE_TIMEOUT_MS, true)) as ValidateReply;
    if (superseded()) return;
    releaseGate();
    if (!(validated.bytes instanceof ArrayBuffer) || validated.bytes.byteLength < 5) throw new Error("validate: no bytes");
    const byteLength = validated.bytes.byteLength;
    const filledCopy = fillSave === "needs-copy";
    // M9: what lands on disk is what a failed restore reopens (main may take the buffer). Until
    // `saved` is recorded (or another document opens), a restore that falls back waits for this
    // write and, when it landed, opens these bytes: never a state older than the disk.
    const diskCopy = validated.bytes.slice(0);
    let written = (_ok: boolean) => {};
    write = { done: new Promise<boolean>(resolve => { written = resolve; }), bytes: diskCopy, dialog: filledCopy || doc?.untitled === true };
    diskWrite = write;
    let result: Awaited<ReturnType<NonNullable<Pdf4AllApi["saveFile"]>>>;
    try {
      result = await api.saveFile(validated.bytes, url, filledCopy ? { inPlace: true, filled: true } : { inPlace: true });
    } catch (err) {
      written(false);
      throw err;
    }
    written(result.ok);
    landed = result.ok;
    // M9: a Save As (Untitled or filled) that landed after a restore reopened the last saved
    // state: main now binds the window to the written file, so the window shows that file.
    if (result.ok && write.dialog && generation === fallbackGeneration && doc?.url === url) {
      await showLateSave(result.name, diskCopy, url);
      return;
    }
    if (superseded()) return;
    if (result.ok) {
      // The write landed: the state the save captured is the clean baseline now. The ticket
      // keeps an edit made while main was writing unsaved (both engines), so the window stays
      // dirty and a close still asks.
      const marked = (await request({ op: "saved", ...(typeof save.ticket === "string" ? { ticket: save.ticket } : {}) })) as MutationReply;
      if (superseded()) return;
      recovery.savedCopy(recovery.fitsSaved(byteLength) ? diskCopy : null);
      if (diskWrite === write) diskWrite = null;
      revision = marked.revision;
      dirty = marked.dirty;
      canUndo = marked.canUndo;
      canRedo = marked.canRedo;
      markSecuritySaved();
      // Bugfix 1 D3: Save on an untitled document wrote a new file and main rebound the window to it.
      if (doc && result.name !== doc.name) {
        doc.name = result.name;
        documentTitleEl.textContent = result.name;
      }
      if (doc) doc.untitled = false; // main bound the window to a file on disk
      if (filledCopy) fillSave = "copied"; // fill-anywhere D7: the window now edits the "(filled)" copy

      syncDirty();
      syncControls();
      setStatus(redactState?.marks
        ? `Saved — ${redactState.marks} marks are still marks; use Apply Redactions to remove the content`
        : `Saved ${result.name}`);
      lastSave = { ok: true, name: result.name, source: name, bytes: byteLength, saveMs: save.saveMs, validated: true, inPlace: true };
      console.log(`P1_SAVE ${JSON.stringify(lastSave)}`);
      return;
    }
    reason = result.reason;
    if (result.reason === "cancelled") {
      setStatus("Save cancelled");
      lastSave = { ok: false, reason: "cancelled" }; // Bugfix 1 D3b: STATE save shows the cancel
      console.log(`P1_SAVE_ERROR ${JSON.stringify({ reason: "cancelled" })}`); // a smoke --save-in-place ends on it
      return;
    }
    setStatus("Not saved");
    showError(SAVE_REASON_TEXT[result.reason] ?? "The document could not be saved.");
  } catch (err) {
    reason = refusalOf(err).reason ?? "worker";
    if (landed) {
      // M9: the file was written but the worker stopped before it was marked saved; the
      // restore notice says what the window holds now, which stays marked unsaved.
      setStatus(`Saved ${name}, but the window still marks its changes unsaved`);
      lastSave = { ok: false, reason, written: true };
      console.log(`P1_SAVE_ERROR ${JSON.stringify({ reason, written: true })}`);
      return;
    }
    setStatus("Not saved");
    showError(SAVE_REASON_TEXT[reason] ?? messageOf(err));
  } finally {
    releaseGate();
    // A write that landed stays the disk's state until it is marked saved or another document opens.
    if (write && diskWrite === write && !landed) diskWrite = null;
    saving = false;
    syncControls();
  }
  lastSave = { ok: false, reason };
  console.log(`P1_SAVE_ERROR ${JSON.stringify({ reason })}`);
}

/**
 * M9: a Save As that asked for a path landed after a restore reopened the last saved state.
 * Main binds the window to the written file: with nothing edited since, the window shows it;
 * otherwise it keeps the edits and says the file holds another version.
 */
async function showLateSave(name: string, bytes: ArrayBuffer, url: string): Promise<void> {
  if (!dirty) {
    await openDocument(name, bytes, url, true);
    const notice = `Saved ${name}. The window now shows the saved file.`;
    showError(notice);
    announce(notice);
    return;
  }
  if (doc) {
    doc.name = name;
    doc.untitled = false;
    documentTitleEl.textContent = name;
    syncDirty();
  }
  showError(`Saved ${name} from before the restart. The window holds other changes: Save writes them over ${name}.`);
}

/** worker save → validate (bytes transferred through) → main writes the copy. */
async function saveAs() {
  if (!api || typeof api.saveFile !== "function") {
    showError("Saving needs the desktop app (window.pdf4all is unavailable).");
    return;
  }
  if (!doc) return;
  if (!doc.url) {
    showError("This is the built-in fixture, not a file on disk; open a file to save a copy.");
    return;
  }
  if (saving) {
    setStatus("A save is already running…");
    return;
  }
  const name = doc.name;
  const url = doc.url;
  const startedGeneration = generation;
  /** A newer document (or a fresh open of the same URL) makes this flow stale. */
  const superseded = () => startedGeneration !== generation || doc?.url !== url;
  let reason = "worker";
  saving = true;
  clearError();
  clearWarning();
  // The rewrite plus the validated copy is the memory peak of the app: drop the
  // offscreen page cache first rather than holding 96 MiB of it alongside.
  cache.clear();
  syncControls();
  setStatus(`Saving a copy of ${name}…`);
  const releaseGate = workerGate.hold();
  try {
    const save = (await request({ op: "save" }, [], SAVE_TIMEOUT_MS, true)) as SaveReply;
    if (superseded()) return;
    const validated = (await request({ op: "validate", pdf: save.bytes }, [save.bytes], SAVE_TIMEOUT_MS, true)) as ValidateReply;
    if (superseded()) return;
    // The worker is free again; only the bridge call and the disk transaction remain.
    releaseGate();
    if (!(validated.bytes instanceof ArrayBuffer) || validated.bytes.byteLength < 5) throw new Error("validate: no bytes");
    const byteLength = validated.bytes.byteLength; // read the length before main may take the buffer
    const result = await api.saveFile(validated.bytes, url);
    if (superseded()) return;
    if (result.ok) {
      setStatus(redactState?.marks
        ? `Saved a copy as ${result.name} — ${redactState.marks} marks are still marks; use Apply Redactions to remove the content`
        : `Saved a copy as ${result.name}`);
      showWarning(result.warning);
      // `name` is the copy main wrote (the smoke checks it against --save-to);
      // `source` is the document that was opened. No paths in logs.
      lastSave = { ok: true, name: result.name, source: name, bytes: byteLength, saveMs: save.saveMs, validated: true, ...(result.warning ? { warning: result.warning } : {}) };
      console.log(`P1_SAVE ${JSON.stringify(lastSave)}`);
      return;
    }
    reason = result.reason;
    if (result.reason === "cancelled") {
      setStatus("Save cancelled");
      return;
    }
    setStatus("Not saved");
    showError(SAVE_REASON_TEXT[result.reason] ?? "The copy could not be saved.");
  } catch (err) {
    reason = refusalOf(err).reason ?? "worker";
    setStatus("Not saved");
    showError(SAVE_REASON_TEXT[reason] ?? messageOf(err));
  } finally {
    releaseGate();
    saving = false;
    syncControls();
  }
  lastSave = { ok: false, reason };
  console.log(`P1_SAVE_ERROR ${JSON.stringify({ reason })}`);
}

function showWarning(warning?: "source-changed" | "durability-uncertain") {
  if (!warning) return;
  warningEl.hidden = false;
  warningEl.textContent = warning === "source-changed"
    ? "The original file changed on disk after it was opened; this copy is from the version shown here. Your original was not modified."
    : "The copy was written, but the folder could not be flushed to disk; the copy may not survive a power loss.";
}

function clearWarning() {
  warningEl.hidden = true;
  warningEl.textContent = "";
}

function setStatus(text: string) {
  statusEl.textContent = text;
}

function showError(text: string) {
  // M9 review: once the restart cap stopped the worker, no later failure replaces that notice.
  if (stoppedNotice !== null && text !== stoppedNotice) return;
  errorEl.hidden = false;
  errorEl.textContent = text;
}

function clearError() {
  errorEl.hidden = true;
  errorEl.textContent = "";
}

function failP0(text: string) {
  if (p0Failed) return;
  p0Failed = true;
  resetDocument();
  setStatus("Failed");
  showError(text);
  console.log(`P0_ERROR ${JSON.stringify({ message: text })}`);
}

/** Print sheets are built from the worker's own pixmaps, one page at a time. */
function clearPrintSheets() {
  printPagesEl.replaceChildren();
}

function printAvailable(): boolean {
  return !!doc && !doc.info.needsPassword && doc.info.printable;
}

function openPrintDialog() {
  if (printing || printDialog.open || !doc) return;
  if (!printAvailable()) {
    setStatus("This document does not permit printing");
    return;
  }
  updatePrintRange();
  printDialog.showModal();
  printRangeInput.focus();
  printRangeInput.select();
}

/** Status line under the range input: page count, or why the text is not usable. */
function updatePrintRange() {
  const pages = doc ? parsePageRange(printRangeInput.value, doc.pageCount) : null;
  printStatusEl.textContent = pages === null
    ? "Invalid range"
    : `${pages.length} page${pages.length === 1 ? "" : "s"}`;
  printStartButton.disabled = pages === null;
}

/** Escape during the build cancels it; the sheets are dropped by the job's own cleanup. */
function cancelPrint() {
  printToken += 1;
  setStatus("Print cancelled");
}

async function startPrint(rangeText: string) {
  if (!doc || printing || !printAvailable()) return;
  const pages = parsePageRange(rangeText, doc.pageCount);
  if (pages === null) {
    showError("That page range is not valid.");
    return;
  }
  if (!pages.length) {
    showError("That page range selects no pages.");
    return;
  }
  if (!api || typeof api.print !== "function") {
    showError("Printing needs the desktop app (window.pdf4all is unavailable).");
    return;
  }
  const startedGeneration = generation;
  const job = ++printToken;
  const started = performance.now();
  let bytes = 0;
  printing = true;
  clearError();
  syncControls();
  try {
    clearPrintSheets();
    for (const [index, page] of pages.entries()) {
      if (job !== printToken || startedGeneration !== generation) return;
      setStatus(`Preparing page ${index + 1} of ${pages.length} for printing…`);
      const sheet = await renderPrintSheet(page, job);
      if (sheet === null) return;
      bytes += sheet.length;
      if (bytes > PRINT_MAX_BYTES) {
        clearPrintSheets();
        showError("Too many pages to print at once; choose a smaller range");
        return;
      }
      appendPrintSheet(page, sheet, doc.pages[page]);
    }
    const prepMs = Math.round(performance.now() - started);
    console.log(`P1_PRINT ${JSON.stringify({ pages: pages.length, prepMs, bytes })}`);
    setStatus(`Printing ${pages.length} page${pages.length === 1 ? "" : "s"}…`);
    const result = await api.print();
    lastPrint = { ...result, pages: pages.length, bytes };
    if (result.ok) setStatus(`Sent ${pages.length} page${pages.length === 1 ? "" : "s"} to the printer`);
    else if (result.reason === "cancelled") setStatus("Print cancelled");
    else showError(PRINT_REASON_TEXT[result.reason] ?? "The document could not be printed.");
  } catch (err) {
    showError(messageOf(err));
  } finally {
    clearPrintSheets();
    printing = false;
    syncControls();
  }
}

const PRINT_REASON_TEXT: Record<string, string> = {
  busy: "Another print job is already running.",
  failed: "The document could not be printed.",
  "no-printer": "No printer is available.",
};

/**
 * One sheet: render → JPEG data URL → `<img>`. The pixels are dropped with the
 * canvas as soon as the data URL exists, and page navigation wins between sheets.
 */
async function renderPrintSheet(page: number, job: number): Promise<string | null> {
  const size = doc?.pages[page];
  if (!size) throw new Error(`Page ${page + 1} has no size to print.`);
  // 150 DPI, never past the engine's own scale and 4096 px edge limits.
  const scale = Math.max(MIN_SCALE, Math.min(
    PRINT_DPI / 72, MAX_SCALE, MAX_PIXEL_EDGE / Math.max(size.width, size.height),
  ));
  inFlight = `print:${page}`;
  let reply: RenderReply;
  try {
    reply = (await request({ op: "render", page, scale })) as RenderReply;
  } finally {
    inFlight = null;
  }
  if (job !== printToken || !doc) return null;
  const canvas2 = document.createElement("canvas");
  canvas2.width = reply.pixelWidth;
  canvas2.height = reply.pixelHeight;
  const context = canvas2.getContext("2d");
  if (!context) throw new Error("canvas 2d unavailable");
  context.putImageData(new ImageData(new Uint8ClampedArray(reply.pixels), reply.pixelWidth, reply.pixelHeight), 0, 0);
  const url = canvas2.toDataURL("image/jpeg", 0.85);
  canvas2.width = 0;
  canvas2.height = 0;
  await drain(); // a page render queued behind this sheet gets its turn before the next one
  return job === printToken ? url : null;
}

function appendPrintSheet(page: number, url: string, size: PageSize) {
  const sheet = document.createElement("div");
  sheet.className = size.width > size.height ? "print-sheet landscape" : "print-sheet";
  const image = document.createElement("img");
  image.src = url;
  image.alt = `Page ${page + 1}`;
  sheet.append(image);
  printPagesEl.append(sheet);
}

function messageOf(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}
