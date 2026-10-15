/**
 * Pure PDF logic shared by the worker (browser, `mupdf` from ./vendor) and
 * tests/p1-worker.test.mjs (Node, `mupdf` from node_modules). It never touches
 * `self`, DOM or a global mupdf: `createEngine(mupdf)` closes over the module
 * the caller supplies, so both environments import this exact same source.
 */

import {
  MAX_IMAGE_PAGE_POINTS,
  OP_PERMISSION,
  allowed,
  PERMISSION_BITS,
  algorithmOf,
  permissionsOf,
  permissionNames,
  permissionBitsOf,
  permissionInt,
  at,
  round2,
  concatBytes,
  attempt,
  failEditText,
  failEditObject,
  failEditReplace,
  failField,
  failRedact,
  failLink,
  failReason,
  errorMessage,
  copyBytes,
  num,
  str,
  winAnsiByte,
  winAnsiBytes,
  checkStampText,
  BASE14_FONTS,
} from "./engine-core.js";

// Defined once in `./limits.js` (A1 step 5) so main and the worker share the numbers;
// re-exported because tests import them from `build/renderer/engine.js`.
import { MAX_PDF_BYTES, MAX_SAVE_BYTES, MAX_TEXT_ADD_CHARS, MAX_TEXT_ADD_LINES } from "./limits.js";
import { prepareSignature, type SignPrepareOptions } from "./sign-prepare.js";
export { MAX_PDF_BYTES, MAX_SAVE_BYTES, MAX_REDACT_SOURCE_BYTES, MAX_SIG_CONTENTS_BYTES } from "./limits.js";
// The bounds and renderer helpers moved to `./engine-shared.js`, which the renderer imports instead.
import {
  MAX_PIXEL_EDGE, MIN_SCALE, THUMB_MIN_EDGE, THUMB_MAX_EDGE, MAX_SCALE, NOTE_SIZE,
  MAX_INK_STROKES, MIN_INK_POINTS, MAX_INK_POINTS, MAX_STAMP_IMAGE_BYTES, SIGNATURE_SUBJECT, DRAWN_SIGNATURE_SUBJECT,
  MAX_NOTE_CHARS, MAX_BOOKMARK_CHARS, MAX_URI_CHARS, MAX_PASSWORD_CHARS, MAX_HITS_DEFAULT, MAX_HITS_CAP,
  MAX_FIELD_NAME_CHARS, MAX_FIELD_NODES, MAX_FIELD_DEPTH, MAX_FIELD_KIDS, MAX_FIELD_OPTIONS, MAX_FIELD_OPTION_CHARS,
  MAX_FIELD_KID_NAME, MAX_FIELD_TEXT_CHARS, MAX_FIELD_LABEL, MIN_FIELD_RECT_PT, parsePageRange, normaliseLine,
} from "./engine-shared.js";
export * from "./engine-shared.js";

// Re-exported for tests and any other importer of `engine.js` (A1 step 1): the move
// must not change what `build/renderer/engine.js` can be imported for.
export {
  PERMISSION_BITS,
  algorithmOf,
  permissionsOf,
  permissionNames,
  permissionBitsOf,
  permissionInt,
  at,
  round2,
  concatBytes,
  attempt,
  failEditText,
  failEditObject,
  failEditReplace,
  failField,
  failReason,
  errorMessage,
  copyBytes,
  num,
  str,
};

export type MupdfModule = typeof import("./vendor/mupdf.js");
export type MupdfDocument = ReturnType<MupdfModule["Document"]["openDocument"]>;
export type MupdfObject = InstanceType<MupdfModule["PDFObject"]>;
export type MupdfPixmap = InstanceType<MupdfModule["Pixmap"]>;


export const MAX_QUERY_CHARS = 256;
/** A FreeText box's fixed size; the point is its top-left, clamped so the box stays on the page. */
export const FREE_TEXT_SIZE: [number, number] = [160, 40];
/** Markup adds take one quad per search-hit line; more than this is a client bug, not a document. */
export const MAX_QUADS = 64;
/** Every annotation type this unit can add, edit and delete. */
export const EDITABLE_TYPES = [
  "Text", "Highlight", "Underline", "StrikeOut", "FreeText", "Square", "Circle", "Line", "Ink", "Stamp",
] as const;
/** The fixed look of each markup type; there is no colour picker this unit. */
export const MARKUP_COLORS: Record<string, [number, number, number]> = {
  Highlight: [1, 1, 0], Underline: [1, 0, 0], StrikeOut: [1, 0, 0],
};
/** Shapes and stamps (P2 unit 9): the fixed colour and width of each, still no picker. */
export const SHAPE_STYLES: Record<string, { color: [number, number, number]; width: number }> = {
  Square: { color: [1, 0, 0], width: 1 },
  Circle: { color: [1, 0, 0], width: 1 },
  Line: { color: [1, 0, 0], width: 1 },
  Ink: { color: [0, 0, 1], width: 1.5 },
  Stamp: { color: [1, 0, 0], width: 0 },
};
/** The six icon stamps mupdf synthesises its own appearance for; custom text is deferred. */
export const STAMP_ICONS = ["Approved", "Draft", "Confidential", "Final", "NotApproved", "Void"] as const;
/** draw-signature D1/D2: the ink width follows the bounds width (200 pt → 1.5). */
export const signatureLineWidth = (width: number): number => Math.min(4, Math.max(0.6, width * 0.0075));
export const MAX_SIGN_NAME_CHARS = 200;
const EDITABLE = new Set<string>(EDITABLE_TYPES);
/**
 * The only schemes this app may create, follow or hand to the OS. Everything
 * else (`file:`, `javascript:`, launch actions, and a URI with no scheme) is
 * classified `blocked` and is never followed or opened (contract-unit5.md).
 */
export const LINK_SCHEMES = ["http:", "https:", "mailto:"] as const;
/** One graft carries at most this many pages, the same bound a range input may name. */
export const MAX_GRAFT_PAGES = 2000;
/** `pdf-from-images` bounds, the same ones the picker enforces (50 files, 32 MiB each). */
export const MAX_IMAGES = 50;
export const MAX_IMAGE_BYTES = 32 * 1024 * 1024;
export const MAX_IMAGES_TOTAL_BYTES = 120 * 1024 * 1024;
/** A stamp's text and the label prefix (P2 unit 8). */
export const MAX_STAMP_CHARS = 200;
export const MAX_LABEL_PREFIX_CHARS = 32;
/** Bates numbers (contract-bates.md D1): 0 … 10^15−1, padded to 1 … 15 digits. */
const MAX_BATES = 1e15 - 1;
/** Compare §9.7: `compare-text` refuses `too-many` above this many lines on either side (O(n·m) table ≤ 16 MiB). */
export const MAX_COMPARE_LINES = 2000;
const MAX_BATES_PAD = 15;
/** The stamp kinds and the page-label numbering styles the app can write. */
export const STAMP_KINDS = ["watermark", "header", "footer"] as const;
export const LABEL_STYLES = ["D", "r", "R", "a", "A", ""] as const;
export type StampKind = (typeof STAMP_KINDS)[number];
/**
 * Full rewrite, never incremental: a save copy must not append onto hostile
 * bytes, and prior revisions are deliberately normalized away. Reachable
 * objects are written as they are, so tags/outlines/attachments survive.
 * `garbage=yes` collects objects nothing points at any more - without it,
 * orphaned original streams survive into the copy (P0-b probe 2: 7 raw copies
 * of a redacted token). `yes`, not `compact`: collect without renumbering, so
 * mupdf's journal still replays across the save (fable51 m89 gate). Measured on
 * large.pdf (500 pages): median 4 -> 4 ms, same as without it.
 */
export const SAVE_OPTIONS = "incremental=no,garbage=yes";
/** U5: the longest literal `find` (the `replace` side reuses MAX_EDIT_TEXT_CHARS). */
export const MAX_REPLACE_FIND_CHARS = 256;

/**
 * Why a save or validate was refused; exact tokens the renderer maps to UI
 * text. Transport failures and timeouts are the renderer's own `worker` token.
 */
export type SaveReason = "no-document" | "read-only" | "encrypted" | "too-large" | "invalid"
  | "no-match" | "no-marks" | "too-many" | "shared-content" | "scan-budget" | "verify-failed"
  | "unsupported-transform" | "unbalanced-content" | "certified" | "already-signed"
  /** P5 U3 §1.4: `sign-prepare {field}` names no empty signature field on that page. */
  | "no-such-field"
  /** Compare §9.7: `compare-render`/`compare-text` with an empty compare slot. */
  | "no-compare-document";

/** Which standard security handler a file uses; `none` means no `/Encrypt`. */
export type DocumentAlgorithm = "none" | "rc4-40" | "rc4-128" | "aes-128" | "aes-256" | "unknown";
/** The five `/P` flags this app reads. `annotate` is PDF bit 6, `form` bit 9. */
export type DocumentPermissions = { print: boolean; copy: boolean; edit: boolean; annotate: boolean; form: boolean };
/** How the user proved access; `owner` implies every permission (PDF /P does not bind the owner). */
export type AuthLevel = "none" | "user" | "owner";
/**
 * What the next save will write. A password lives here only for `aes-256` and only
 * in worker memory: it is cleared on open/close, never journaled, never in a reply.
 */
export type SecuritySetting =
  | { mode: "keep" }
  | { mode: "none" }
  | { mode: "aes-256"; userPassword: string; ownerPassword: string; permissions: number; flags: DocumentPermissions };
/** The renderer's view of security: never a password, never the in-memory key. */
export type SecurityState = {
  mode: "keep" | "none" | "aes-256";
  algorithm: DocumentAlgorithm;
  permissions: (keyof DocumentPermissions)[];
  auth: AuthLevel;
  pending: boolean;
};


/** The `mode` values the Security dialog offers; `keep` is the implicit default. */
const SECURITY_MODES = ["keep", "none", "aes-256"] as const;

export type Line = {
  text: string;
  x: number;
  y: number;
  w: number;
  h: number;
  size: number;
  family: string;
  weight: string;
  style: string;
};

export type PageSize = { width: number; height: number };

export type DocumentInfo = {
  encrypted: boolean;
  needsPassword: boolean;
  /** Standard security handler algorithm, read from `/Encrypt` without a password. */
  algorithm: DocumentAlgorithm;
  /** The five `/P` permissions; all true for an unencrypted document. */
  permissions: DocumentPermissions;
  repaired: boolean;
  signatures: number;
  certified: boolean;
  xfa: boolean;
  attachments: number;
  openAction: boolean;
  tagged: boolean;
  versions: number;
  /** The document's print permission; false only for an encrypted document that clears it. */
  printable: boolean;
};

export type RenderResult = {
  width: number;
  height: number;
  pixelWidth: number;
  pixelHeight: number;
  pixels: ArrayBuffer;
  text: string;
  lines: Line[];
  nonWhite: number;
  paintMs: number;
  renderMs: number;
};

export type OpenRequest = { id: number; op: "open"; pdf: ArrayBuffer | null; name: string };
export type RenderRequest = { id: number; op: "render"; page: number; scale: number };
export type ThumbnailRequest = { id: number; op: "thumbnail"; page: number; maxEdge: number };
export type SearchRequest = { id: number; op: "search"; query: string; maxHits?: number };
export type AuthenticateRequest = { id: number; op: "authenticate"; password: string };
export type CloseRequest = { id: number; op: "close" };
export type SaveRequest = { id: number; op: "save" };
export type ValidateRequest = { id: number; op: "validate"; pdf: ArrayBuffer | null };
/**
 * A pending security change: stored in the session, applied by the next save.
 * Only `aes-256` carries passwords, and only from the dialog inward.
 */
export type SecuritySetRequest = {
  id: number; op: "security-set";
  mode: string;
  userPassword?: unknown;
  ownerPassword?: unknown;
  permissions?: Partial<Record<keyof DocumentPermissions, unknown>>;
};
export type AnnotsRequest = { id: number; op: "annots"; page: number };
export type AnnotAddRequest = {
  id: number; op: "annot-add"; page: number; type: string;
  /** Text and FreeText anchor on a point; markups carry search-hit quads (page points, y down). */
  point?: number[]; quads?: number[][]; contents: string;
  /** Shapes and stamps (P2 unit 9): a rect, a Line's two points, Ink strokes, an icon, an image. */
  rect?: number[]; points?: number[][]; strokes?: number[][][];
  icon?: string; image?: { name: string; bytes: ArrayBuffer };
  /** A signature appearance: an image stamp, plus the name it was signed with. */
  sign?: { name: string };
  /** FreeText only: size the box to its text (`point` = left edge, first line's vertical centre). */
  fit?: boolean;
  /** draw-signature D1: Ink — a drawn signature (black, width from its bounds, `/Subj`, no contents); sign-merge D3: or a Stamp with `image` and no `sign`. */
  signature?: boolean;
};
// `id` is the request id on every op, so the annotation's own object number is
// `annotId` here; the `annots` reply has no request id collision and uses `id`.
export type AnnotSetRequest = { id: number; op: "annot-set"; page: number; annotId: number; contents: string; fit?: boolean };
/** fill-anywhere D2: move one FreeText; `point` is its new top-left (y-down page pt), size kept. */
export type AnnotMoveRequest = { id: number; op: "annot-move"; page: number; annotId: number; point: number[];
  /** draw-signature D2: a drawn signature's new bounds width (aspect kept, about the new top-left); sign-merge D3: a signature Stamp's too. */
  width?: number };
export type AnnotDeleteRequest = { id: number; op: "annot-delete"; page: number; annotId: number };
export type RedactAddRequest = { id: number; op: "redact-add"; page: number; rect: number[] };
/** P6 U3 §3.1: move or resize one Redact mark; `rect` is y-down page pt (the `AnnotItem.rect` space). */
export type RedactSetRequest = { id: number; op: "redact-set"; page: number; annotId: number; rect: number[] };
export type RedactSearchRequest = { id: number; op: "redact-search"; text: string };
export type RedactPreviewRequest = { id: number; op: "redact-preview" };
export type RedactApplyRequest = { id: number; op: "redact-apply"; sanitize: Record<string, boolean> };
export type SignPrepareRequest = { id: number; op: "sign-prepare" } & SignPrepareOptions;
export type SignListRequest = { id: number; op: "sign-list" };
/** Page organisation (P2 unit 3): one journal operation each, nothing else. */
/** AcroForm fill (P2 unit 4): one read and one write, each one journal operation. */
export type FieldsRequest = { id: number; op: "fields"; page: number };
export type FieldSetRequest = {
  id: number; op: "field-set"; page: number; widgetId: number; value: string | boolean;
};
/**
 * U6 (bounded form designer): create, change and delete fields on one page. Each
 * action is one journal operation and one `mutate`; a refusal never dirties.
 */
export type FieldAddKind = "text" | "checkbox" | "combo" | "radio" | "signature";
/** U7: the four properties `field-add` and `field-props` share (§2.1). `default` is a
 * string for text/combo/radio (a radio kid's export name) and a boolean for a checkbox. */
export type FieldPropertyRequest = {
  label?: string; readOnly?: boolean; required?: boolean; default?: string | boolean;
  /** P6 U2 §4.1: a combo's `/Ff` Edit bit (typed values accepted); combo only. */
  edit?: boolean;
};
export type FieldAddRequest = FieldPropertyRequest & {
  /** P5 U3 §1.2: `name` may be omitted or `""` for kind `signature` only (the engine picks `Signature<n>`). */
  id: number; op: "field-add"; page: number; kind: string; name?: string;
  /** Page points, y down (the space `annot-add` and `#content-box-*` use). */
  rect?: number[];
  options?: string[];
  maxLen?: number;
  multiline?: boolean;
  kids?: { on: string; rect: number[] }[];
};
export type FieldPropsRequest = FieldPropertyRequest & {
  id: number; op: "field-props"; page: number; widgetId: number;
  name?: string; rect?: number[]; maxLen?: number; multiline?: boolean; options?: string[];
};
export type FieldDeleteRequest = { id: number; op: "field-delete"; page: number; widgetId: number };
/** One fixed refusal token for the field-designer ops; never a guess about what to do next. */
export type FieldDetail =
  | "xfa" | "scripted" | "signature" | "duplicate-name" | "invalid-name"
  | "invalid" | "too-many" | "stale-target"
  /** U7 §8.1: `field-set` on a widget whose own field is read-only (the designer may still
   * change it); a document-level `read-only` refusal is a separate token on `reason`. */
  | "field-read-only";
/** Bookmarks and links (P2 unit 5). `outlineId`/`linkId` are the reply's own ids; the
 * request's `id` belongs to the worker seam on every op, so it cannot carry them. */
export type OutlineRequest = { id: number; op: "outline" };
export type OutlineAddRequest = { id: number; op: "outline-add"; title: string; page: number };
export type OutlineSetRequest = { id: number; op: "outline-set"; outlineId: number; title: string };
export type OutlineDeleteRequest = { id: number; op: "outline-delete"; outlineId: number };
export type LinksRequest = { id: number; op: "links"; page: number };
export type LinkAddRequest = {
  id: number; op: "link-add"; page: number; rect: number[];
  target: { page: number } | { uri: string };
};
export type LinkDeleteRequest = { id: number; op: "link-delete"; page: number; linkId: number };
/** P6 U4 §1.2: move/resize and/or retarget one link, named by its `/Annots` object number. */
export type LinkSetRequest = {
  id: number; op: "link-set"; page: number; annotId: number; rect?: number[];
  target?: { page: number } | { uri: string };
};
/** Assemble (P2 unit 6): bring pages in from another PDF, or write a range out. */
export type InsertPagesRequest = {
  id: number; op: "insert-pages"; pdf: ArrayBuffer; name: string; at: number;
  /** 0-based source indices; absent means `range` decides, and an absent range means all. */
  pages?: number[];
  /** The dialog's own text, parsed against the *source* count only the worker knows. */
  range?: string;
};
export type ExtractPagesRequest = { id: number; op: "extract-pages"; pages: number[] };
/** Exports (P2 unit 7): images become a new document, and the text comes out whole. */
export type PdfFromImagesRequest = {
  id: number; op: "pdf-from-images"; images: { name: string; bytes: ArrayBuffer }[];
};
export type TextAllRequest = { id: number; op: "text-all" };
/**
 * Compare (contract-compare.md D1/D2, §9.1-§9.7): a second, view-only document in the same session.
 * No OP_PERMISSION row, no journal, no revision: these ops never call requireMutable/mutate.
 */
export type CompareOpenRequest = { id: number; op: "compare-open"; pdf: ArrayBuffer | null; name: string };
export type CompareCloseRequest = { id: number; op: "compare-close" };
export type CompareRenderRequest = { id: number; op: "compare-render"; page: number; scale: number };
export type CompareTextRequest = { id: number; op: "compare-text"; pageA: number; pageB: number };
/** One stext line under preserve-spans: bbox [x0, y0, x1, y1] in y-down page pt. */
export type CompareLine = { text: string; bbox: number[] };
/**
 * Page decoration (P2 unit 8): one appended content stream per page, page labels and
 * layer visibility. The layer id is `layerId` because `id` is the request id on every op.
 */
export type StampRequest = {
  id: number; op: "stamp"; kind: string; text: string; pages?: number[];
  /** Bates (contract-bates.md D1/D3): the first number, its zero-pad width (default 6), header/footer alignment. */
  batesStart?: number; batesPad?: number; align?: string;
};
/**
 * Batch (contract-batch.md D2, §9.1-9.7): one file of a folder stamped on a TRANSIENT document -
 * never the open one, no journal, no revision. `pages` is a range string ("" or absent = all).
 */
export type BatchStampAction = {
  kind: string; text: string; pages?: string; align?: string; batesStart?: number; batesPad?: number;
};
export type BatchFileRequest = {
  id: number; op: "batch-file"; name: string; bytes: ArrayBuffer | null; action: BatchStampAction;
};
export type LabelsRequest = { id: number; op: "labels" };
export type LabelsSetRequest = {
  id: number; op: "labels-set"; from: number; style: string; prefix?: string; start?: number;
};
export type LabelsClearRequest = { id: number; op: "labels-clear"; from: number };
/** The current page objects' numbers, in page order: the drive's proof that a page moved. */
export type PageIdsRequest = { id: number; op: "page-ids" };
export type LayersRequest = { id: number; op: "layers" };
export type LayerSetRequest = { id: number; op: "layer-set"; layerId: number; visible: boolean };
/**
 * P3 U2: replace or delete one discovered text run, in place, in the stream that
 * holds its operand. `text: ""` deletes (the operator keeps `() Tj`).
 */
export type EditTextRequest = {
  id: number;
  op: "edit-text";
  page: number;
  targetId: string;
  generation: number;
  revision: number;
  text: string;
  /** P6 U2 §2.1/§9.1: page pt, y down (the `bounds` space). */
  move?: { dx: number; dy: number };
  /** P6 U2 §2.1/§9.2: the new font size in page pt (the reply's `fontSize` unit). */
  size?: number;
};
/** P6 U2 §3.3/§9.7: one paragraph rewrite; lengths in page pt, y down (the `bounds` space). */
export type EditParagraphRequestMessage = {
  id: number; op: "edit-paragraph"; page: number; paragraphId: string; generation: number; revision: number;
  text: string; width?: number; move?: { dx: number; dy: number }; size?: number; leading?: number;
};
/** One fixed refusal token for `edit-text`; never a guess about what to do next. */
export type EditTextDetail =
  | "stale-target" | "not-candidate" | "unsupported-encoding" | "overflow" | "tail-would-move"
  | "unmeasurable-width" | "ambiguous-text-association" | "too-large" | "outside-page" | "invalid" | "word-too-wide";
/** U4: one Image or Form placement action on one page. */
export type EditObjectAction = "move" | "resize" | "delete" | "replace" | "duplicate" | "transform" | "crop";
/** P6 U6 §1.2: a 90° step or a flip about the visible rect's centre. */
export type EditObjectTransform = "rotate-cw" | "rotate-ccw" | "flip-h" | "flip-v";
export type EditObjectRequest = {
  id: number;
  op: "edit-object";
  page: number;
  targetId: string;
  generation: number;
  revision: number;
  action: EditObjectAction;
  /** Page points, bottom-left origin, for `move` and `resize`. */
  box?: number[];
  /** A local PNG/JPEG for `replace`. */
  image?: { name: string; bytes: ArrayBuffer };
  /** P6 U6 §1.2: for `transform`. */
  transform?: EditObjectTransform;
};
/** One fixed refusal token for `edit-object`; never a guess about what to do next. */
export type EditObjectDetail =
  | "stale-target" | "not-candidate" | "unsupported-transform" | "unsupported-image"
  | "out-of-bounds" | "too-large" | "invalid-image" | "invalid-selection" | "invalid-transform"
  /** P6 U7 §9.16: `clip-paste` with no clipboard. */
  | "empty-clipboard";
/** P6 U5: 2..MAX_EDIT_TARGETS object edits on one page, all-or-nothing, one journal op. */
export type EditObjectsRequest = {
  id: number;
  op: "edit-objects";
  page: number;
  generation: number;
  revision: number;
  items: { targetId: string; action: "move" | "delete" | "duplicate" | "resize" | "transform"; box?: number[]; transform?: EditObjectTransform }[];
};
/** P6 U7 §10.25: the kind of one copied member (= the STATE selection member kind). */
export type ClipKind = "image" | "form" | "text" | "field" | "link" | "mark";
export type ClipKinds = Record<ClipKind, number>;
/**
 * P6 U7 §1.1/§10.1: copy the members into the worker clipboard. `id` is the targetId
 * (image, form, text), the widgetId (field) or the annotId (link, mark). Nothing is
 * written and no journal op is opened; `generation`/`revision` are required only when a
 * text or object member is present.
 */
export type ClipCopyRequest = {
  id: number; op: "clip-copy"; page: number; generation?: number; revision?: number;
  targets: { kind: ClipKind; id: string | number }[];
};
/** P6 U7 §1.2/§10.3: paste the clipboard onto `page` as one journal op; `at` = the
 * union's top-left in page points (bottom-left origin), from the page menu only. */
export type ClipPasteRequest = { id: number; op: "clip-paste"; page: number; generation?: number; revision?: number; at?: number[] };
/** P6 U7 §1.3/§9.13/§9.14: one member write of an `edit-batch`. */
export type EditBatchItem =
  | { op: "edit-text"; targetId: string; move: { dx: number; dy: number } }
  | { op: "field-props"; widgetId: number; rect: number[] }
  | { op: "field-delete"; widgetId: number }
  | { op: "link-set"; annotId: number; rect: number[] }
  | { op: "link-delete"; annotId: number }
  | { op: "redact-set"; annotId: number; rect: number[] }
  | { op: "annot-delete"; annotId: number }
  | { op: "edit-object"; targetId: string; action: "move" | "delete"; box?: number[] };
/** P6 U7 §1.3: 2..MAX_EDIT_TARGETS mixed-kind writes on one page, all-or-nothing, one journal op. */
export type EditBatchRequest = { id: number; op: "edit-batch"; page: number; generation?: number; revision?: number; items: EditBatchItem[] };
/** U4: an image XObject's own facts, as the item reports them. */
export type EditObjectImage = {
  width: number;
  height: number;
  smask: boolean;
  mask: boolean;
  decode: boolean;
  interpolate: boolean;
  oc: boolean;
};
/** U5: one scoped find/replace preview or apply, over one page's text runs. */
export type EditReplacePreviewRequest = {
  id: number;
  op: "edit-replace-preview";
  page: number;
  generation: number;
  revision: number;
  find: string;
  replace: string;
  /** Limit the preview to this one run. */
  targetId?: string;
};
export type EditReplaceApplyRequest = {
  id: number;
  op: "edit-replace-apply";
  page: number;
  generation: number;
  revision: number;
  find: string;
  replace: string;
  previewToken: string;
  targetIds: string[];
  /** The run scope the preview used; the token covers it (§7.7). */
  targetId?: string;
};
/** One fixed refusal token for `edit-replace-apply`; a rejected run carries its U2/U3 token. */
export type EditReplaceDetail = "stale-preview" | "stale-target" | "invalid" | "too-large" | "write-failed";

/** One optional-content layer as the panel sees it. */
export type LayerItem = { id: number; name: string; visible: boolean };
/**
 * P3 U1: read-only discovery of existing-content targets on one page. It never
 * writes, and a `candidate` item only means the operator/occurrence identity is
 * exact enough to validate a later edit against - it is not an edit capability.
 */
export type EditTargetsRequest = { id: number; op: "edit-targets"; page: number };
export type TextAddRequest = {
  id: number; op: "text-add"; page: number; point: number[]; text: string; font: string; size: number; color: number[];
};
export type EditTargetKind = "text" | "image" | "form";
export type EditTargetSupport = "candidate" | "unsupported";
/** Font facts as the stream declares them; `null` is "unknown", never inferred from a render. */
export type EditTargetFont = {
  name: string | null;
  subtype: string | null;
  embedded: boolean | null;
  subset: boolean | null;
  writingMode: number | null;
};
export type EditTarget = {
  targetId: string;
  kind: EditTargetKind;
  bounds: number[] | null;
  support: EditTargetSupport;
  /** Why an item is not a candidate; one of the fixed reason tokens. */
  reason?: string;
  /** Present only when decoding is proven, never guessed from appearance. */
  text?: string;
  font?: EditTargetFont;
  /** U2: text follows this run on the same line, so the width must not change. */
  tailFollows?: boolean;
  /** P3b: a `TJ` run; an edit rewrites it as one `Tj` and drops its kerning. */
  kerned?: boolean;
  /** U1b: shared, but an edit privatises this placement alone first (copy-on-write). */
  cowable?: boolean;
  /** U3: the characters proven drawable with this run's font on this page (CID runs). */
  repertoire?: string;
  /** U4: the placement box in page points (a simple object placement). P6 U6 §1.1/§9.8: `box` is the
   * visible rect, `orient` the `cm`'s axis orientation, `full` the uncropped box (only when clipped). */
  placement?: { box: number[]; orient: EditOrient; full?: number[] };
  /** U4: what `edit-object` may do to this placement, in the order the UI shows it. */
  objectOps?: EditObjectAction[];
  /** U4: the `/Resources/XObject` name this placement draws. */
  objectName?: string;
  /** U4: image facts (pixel size, mask, layer). */
  image?: EditObjectImage;
  /** P6 U2 §2.4: `Tf` × the text matrix scale, in page pt (the unit `edit-text {size}` takes). */
  fontSize?: number;
  /** P6 U2 §2.4/§9.3: `edit-text {move}` is proven for this run. */
  moveable?: boolean;
  /** P6 U2: why a candidate run is not moveable (`matrix`, `tail`, `tail-would-move`). */
  moveReason?: string;
  /** P6 U2 §9.17: the paragraph this line belongs to. */
  paragraphId?: string;
  /** print-edit D2: a print-to-PDF glyph line: how many per-glyph blocks it stands for. */
  glyphs?: number;
};
export type PageRotateRequest = { id: number; op: "page-rotate"; page: number; by: 90 | -90 };
export type PageDeleteRequest = { id: number; op: "page-delete"; page: number };
export type PageMoveRequest = { id: number; op: "page-move"; page: number; to: number };
export type UndoRequest = { id: number; op: "undo" };
export type RedoRequest = { id: number; op: "redo" };
/** Main reports that the write succeeded: the journal position becomes the baseline. */
export type SavedRequest = { id: number; op: "saved" };
export type SessionRequest =
  OpenRequest | RenderRequest | ThumbnailRequest | SearchRequest | AuthenticateRequest
  | CloseRequest | SaveRequest | ValidateRequest | SecuritySetRequest | AnnotsRequest | AnnotAddRequest
  | AnnotSetRequest | AnnotMoveRequest | AnnotDeleteRequest | RedactAddRequest | RedactSetRequest | RedactSearchRequest | RedactPreviewRequest | RedactApplyRequest | SignPrepareRequest | SignListRequest
  | PageRotateRequest | PageDeleteRequest | PageMoveRequest
  | FieldsRequest | FieldSetRequest | FieldAddRequest | FieldPropsRequest | FieldDeleteRequest
  | OutlineRequest | OutlineAddRequest | OutlineSetRequest
  | OutlineDeleteRequest | LinksRequest | LinkAddRequest | LinkDeleteRequest | LinkSetRequest
  | InsertPagesRequest | ExtractPagesRequest | PdfFromImagesRequest | TextAllRequest
  | CompareOpenRequest | CompareCloseRequest | CompareRenderRequest | CompareTextRequest
  | StampRequest | BatchFileRequest | LabelsRequest | LabelsSetRequest | LabelsClearRequest | LayersRequest | LayerSetRequest
  | EditTargetsRequest | EditTextRequest | EditParagraphRequestMessage | TextAddRequest | EditObjectRequest | EditObjectsRequest | EditReplacePreviewRequest | EditReplaceApplyRequest
  | ClipCopyRequest | ClipPasteRequest | EditBatchRequest
  | UndoRequest | RedoRequest | SavedRequest | PageIdsRequest;

/**
 * The AcroForm field kinds this unit shows. Push buttons are omitted entirely;
 * a signature field is listed but not fillable (contract-unit4.md, Decisions).
 */
export type FieldKind = "text" | "checkbox" | "radio" | "combo" | "list" | "signature";

/**
 * One widget on a page, as the fill panel sees it. `id` is the widget's own
 * object number and is what `field-set` names. Radio kids are separate items
 * sharing `name`, each with its `on` export value and a boolean `value`.
 */
export type FieldItem = {
  id: number;
  name: string;
  label: string;
  kind: FieldKind;
  value: string | boolean;
  options?: string[];
  /** P6 U2 §4.1: combo only — `/Ff` bit 19 (inherited): any typed value is accepted. */
  edit?: boolean;
  maxLen?: number;
  multiline?: boolean;
  readOnly: boolean;
  /** U7: `/Ff` bit 2 on the field dict (the parent's, for a radio kid). */
  required: boolean;
  /** U7 §8.4: the field dict's own raw `/TU` (the parent's, for a radio kid), present only
   * when the key exists; `label` keeps its `getLabel() || name` meaning. */
  tu?: string;
  /** U7: `/DV` — a string for text/combo, a boolean for a checkbox, a kid export name
   * for a radio; absent when the field has no `/DV`. */
  default?: string | boolean;
  /** The widget, its field, or a document-level script: never executed here. */
  scripted: boolean;
  /** The on-state export value of a checkbox or radio widget. */
  on?: string;
  /** `getRect()` in page points, y down from the page top (U6 §7.1). */
  rect: number[];
  /** P6 U4 §2.1/§9.11: on a `/Tabs /S` page, the widget's position in the struct-tree
   * `/OBJR` walk; absent when the walk does not reach it (or the tree is too big). */
  structIndex?: number;
  /** P5 U3 §1.1/§9.10: kind `signature` only — the widget's own dict has a `/V`. */
  signed?: boolean;
};

/** One annotation on a page, as the renderer sees it. `rect` is in the coordinate space `getRect()` reports (PDF points, y down from the page top). */
export type AnnotItem = {
  id: number;
  type: string;
  rect: number[];
  contents: string;
  /** `/Subj`, which is how a signature appearance stamp is told from an icon stamp. */
  subject: string;
  editable: boolean;
  /** P6 U3 §9.4: a Redact mark that still carries search quads or its search tag; absent otherwise. */
  search?: boolean;
};
/**
 * One document bookmark, in preorder. `id` is the preorder index in the reply it
 * came from, which is what `outline-set`/`outline-delete` name. `page` is the 0-based
 * page `resolveLink` reports, or null when there is no destination or it is not a
 * page (then `uri` carries the external destination).
 */
export type OutlineEntry = {
  id: number;
  title: string;
  depth: number;
  page: number | null;
  uri?: string;
};

/**
 * One link on a page, as the panel sees it. `id` is the index in `getLinks()`
 * order for that reply. `kind:'blocked'` means the destination is not a page and
 * the scheme is not on `LINK_SCHEMES`; the raw `uri` is still reported so the
 * panel can name the scheme, but it is never followed.
 */
export type LinkEntry = {
  id: number;
  /** P6 U4 §1.1/§9.1: the object number of the link's `/Annots` entry; 0 when it is a
   * direct dict or no entry could be paired (`link-set` refuses 0). */
  annotId: number;
  rect: number[];
  kind: "page" | "uri" | "blocked";
  page?: number;
  uri?: string;
};

/** The slice of every mutation/undo/redo/save reply that owns dirty state in the worker. */
export type JournalState = {
  revision: number;
  dirty: boolean;
  canUndo: boolean;
  canRedo: boolean;
};
/**
 * The page geometry a mutation reply carries besides the journal state: the
 * renderer takes its page count, page sizes and rotations from the reply so a
 * page op never has to reopen the document. `rotations` is what `P2_EDIT.pages.rotate` logs.
 */
export type PageState = {
  pageCount: number;
  pages: PageSize[];
  rotations: number[];
};

/** One match: a page index and the quads it covers, in PDF points at scale 1. */
export type SearchHit = { page: number; quads: number[][] };

/**
 * Holds the one open document. The worker owns this; the engine only drives it.
 *
 * `locked` is the engine's own record of whether the document still needs a
 * password. It cannot be re-derived from the document: mupdf's `needsPassword()`
 * re-tests the empty password, which drops an authenticated session back to
 * locked, so the document is never asked again once the answer is known.
 */
export type Session = {
  doc: MupdfDocument | null;
  name: string | null;
  sourceBytes: number;
  locked: boolean;
  /** How the user proved access; `owner` implies every permission. */
  auth: AuthLevel;
  /** The password that unlocked this session: worker memory only, cleared on close/open. */
  password: string | null;
  /**
   * The source bytes, kept only while the document is locked. MuPDF caches every
   * object parsed before `authenticatePassword` with its strings undecrypted (a
   * field name read after unlock came back as bytes), so a successful unlock
   * reopens from these bytes instead of trusting the pre-auth parse.
   */
  lockedBytes: ArrayBuffer | null;
  /** Original bytes retained only when needed for an incremental second signature. */
  signedSource: ArrayBuffer | null;
  /** A security change waiting for the next save. Not journaled, not undoable. */
  security: SecuritySetting | null;
  /** mupdf's journal cannot see a trailer-only change, so dirty tracks it here. */
  securityDirty: boolean;
  /**
   * Journal position that matches what is on disk. MuPDF has no "mark clean"
   * (hasUnsavedChanges() stays true after saveToBuffer, confirmed by test), so
   * dirty is this baseline compared with the live journal position. Undoing
   * back to the baseline is clean again, which is the correct behaviour.
   */
  savedPosition: number;
  /** Bumped by every mutation, undo and redo; renderer caches key on it. */
  revision: number;
  /**
   * P3 U1: which document this session holds, allocated by the engine on every
   * open and close. A target handle that carries the wrong generation is stale
   * even when the revision happens to match (a reopened file restarts at 0).
   */
  generation: number;
  /** P6 U7 D1: the one worker clipboard; `open`/`close` leave it alone. */
  clipboard: Clipboard | null;
  /** Compare D1/§9.3: the view-only second document; `open`/`close` leave it alone (unless the pair is over MAX_PDF_BYTES). */
  compare: { doc: MupdfDocument; name: string; pageCount: number; sourceBytes: number } | null;
  /** P6 U7 §9.2: while an `edit-batch` runs its annotation items, `mutate` only runs `work()`; "plan" skips it (§1.3 checks before the op). */
  batch: boolean | "plan";
};

/** P6 U7 §10.1/§10.13: one copied member; geometry in the source page's view space (y down) unless noted. */
export type ClipItem =
  | {
    kind: "image" | "form"; name: string; view: number[];
    /** PDF space (y up): the source placement's `cm`, orientation, full box, rect, visible box and clip rect. */
    matrix: number[]; orient: EditOrient; box: number[]; rect: number[]; visible: number[]; clip: number[] | null;
    /** The XObject grafted into the clipboard's scratch document. */
    object: number; objectOps: EditObjectAction[]; reason?: string;
  }
  | {
    kind: "text"; text: string; size: number; tfState: { tc: number; tw: number; tz: number; tl: number; tr: 0 | 1 | 2 };
    origin: number[]; linear: number[]; bytes: Uint8Array; font: number; bounds: number[];
  }
  | { kind: "field"; item: FieldItem; value: string | boolean; kids: { on: string; rect: number[] }[] | null; view: number[] }
  | { kind: "link"; rect: number[]; target: { page: number } | { uri: string } }
  | { kind: "mark"; rect: number[]; quads: number[][] | null };
export type Clipboard = { doc: InstanceType<MupdfModule["PDFDocument"]>; items: ClipItem[] };

export type MupdfPage = InstanceType<MupdfModule["PDFPage"]>;
type MupdfAnnotation = ReturnType<MupdfPage["getAnnotations"]>[number];
type MupdfWidget = ReturnType<MupdfPage["getWidgets"]>[number];
type MupdfLink = ReturnType<MupdfPage["getLinks"]>[number];
type AnnotationType = Parameters<MupdfPage["createAnnotation"]>[0];
/** The 8-number quad and 4-number rect wire shapes, as `annotation.ts` expects them. */
type Quad = [number, number, number, number, number, number, number, number];
type Rect = [number, number, number, number];
/** One node of `Document.loadOutline()`. It is the only place children are visible. */
type OutlineNode = { title?: string; uri?: string; open?: boolean; down?: OutlineNode[] };
type MupdfImage = InstanceType<MupdfModule["Image"]>;

// ---------------------------------------------------------------------------
// Worker replies. Declared once here (A1 step 3); the renderer imports these
// as types, so a reply field that drifts is a `tsc` error, not a drive failure.
// ---------------------------------------------------------------------------

export type OpenReply = {
  id: number;
  ok: true;
  pageCount: number;
  pages: PageSize[];
  rotations: number[];
  info: DocumentInfo;
  openMs: number;
  revision: number;
  auth: AuthLevel;
  /** Compare §9.3: the pair was over MAX_PDF_BYTES, so the compare slot was closed first. */
  compareClosed?: boolean;
};
export type RenderReply = { id: number; ok: true; page: number; revision: number } & RenderResult;
/** Compare §9.7 replies. */
export type CompareOpenReply = { id: number; ok: true; name: string; pageCount: number; pages: PageSize[] };
export type CompareRenderReply = { id: number; ok: true; page: number } & RenderResult;
export type CompareTextReply = {
  id: number; ok: true; pageA: number; pageB: number; revision: number;
  same: number; removed: CompareLine[]; added: CompareLine[]; moved: 0;
};
export type SearchReply = {
  id: number;
  ok: true;
  hits: SearchHit[];
  total: number;
  truncated: boolean;
  searchMs: number;
};
export type SaveReply = { id: number; ok: true; bytes: ArrayBuffer; saveMs: number; dirty: boolean; revision: number; security?: SecurityState };
/** Pixels only: the sidebar never gets structured text back (contract unit 4 §A.1). */
export type ThumbReply = {
  id: number;
  ok: true;
  page: number;
  revision: number;
  width: number;
  height: number;
  pixelWidth: number;
  pixelHeight: number;
  pixels: ArrayBuffer;
};
export type ValidateReply = { id: number; ok: true; pageCount: number; bytes: ArrayBuffer; dirty: boolean; revision: number };
export type AnnotsReply = { id: number; ok: true; page: number; items: AnnotItem[] };
export type MutationReply = { id: number; ok: true } & JournalState;
/** A page op or an undo/redo: the reply owns the new page count, sizes and rotations. */
export type PageReply = MutationReply & { pageCount: number; pages: PageSize[]; rotations: number[]; step?: string };
/** P6 U4 §2.1: the page's `/Tabs` name, null when absent or not one of the five. */
export type TabsName = "R" | "C" | "S" | "A" | "W";
export type FieldsReply = { id: number; ok: true; page: number; tabs: TabsName | null; items: FieldItem[] };
export type OutlineReply = { id: number; ok: true; items: OutlineEntry[] };
export type LinksReply = { id: number; ok: true; page: number; items: LinkEntry[] };
/** An insert reply is a page edit plus what it added, for the smoke's `P2_EDIT.assemble`. */
export type InsertReply = PageReply & { inserted: number; fieldNames: string[] };
export type ExtractReply = { id: number; ok: true; bytes: ArrayBuffer; pageCount: number };
export type PdfFromImagesReply = { id: number; ok: true; bytes: ArrayBuffer; pageCount: number };
export type TextAllReply = { id: number; ok: true; text: string };
export type LabelsReply = { id: number; ok: true; items: string[] };
export type LabelsSetReply = MutationReply & { items: string[] };
export type PageIdsReply = { id: number; ok: true; ids: number[] };
export type LayersReply = { id: number; ok: true; items: LayerItem[] };
export type LayerSetReply = { id: number; ok: true; persisted: boolean; items: LayerItem[] };
export type AuthenticateReply =
  | { id: number; ok: true; authenticated: false }
  | { id: number; ok: true; authenticated: true; pageCount: number; pages: PageSize[]; rotations: number[]; info: DocumentInfo; auth: AuthLevel };
export type SecurityReply = { id: number; ok: true; security: SecurityState } & JournalState;
export type Reply = OpenReply | RenderReply | ThumbReply | SearchReply | SaveReply | ValidateReply | AnnotsReply | FieldsReply | OutlineReply | LinksReply | MutationReply | AuthenticateReply | SecurityReply | ExtractReply | PdfFromImagesReply | TextAllReply | LabelsReply | LayersReply | PageIdsReply
  | CompareOpenReply | CompareRenderReply | CompareTextReply;
export type ErrorReply = { id: number; ok: false; error: string; reason?: string };

/**
 * A worker refusal: `dispatch`'s `{ ok: false }` reply. `reason` is the tagged
 * refusal/save reason, `detail` the U1-U6 refusal token; `extra` fields (for
 * example `missing` on a repertoire refusal) and the current journal state ride
 * along, hence the index signature. Declared once, imported by the renderer.
 */
export type RefusalReply = { id?: number; ok: false; error: string; reason?: string; detail?: string; [key: string]: unknown };


export type DispatchResult = { reply: Record<string, unknown>; transfer?: Transferable[] };

export function createEngine(mupdf: MupdfModule) {
  // ponytail: one Font per name for the process, never destroyed (12 max).
  const base14 = new Map<string, InstanceType<MupdfModule["Font"]>>();
  function base14Font(name: string) {
    let font = base14.get(name);
    if (!font) base14.set(name, font = new mupdf.Font(name));
    return font;
  }
  editSetBase14Font(base14Font);
  function openDocument(pdf: ArrayBuffer) {
    if (!(pdf instanceof ArrayBuffer)) throw new Error("open: expected an ArrayBuffer");
    if (pdf.byteLength < 5) throw new Error("open: not a PDF (too few bytes)");
    if (pdf.byteLength > MAX_PDF_BYTES) {
      throw new Error(`open: PDF exceeds the ${MAX_PDF_BYTES} byte ceiling`);
    }
    const started = performance.now();
    const doc = mupdf.Document.openDocument(pdf, "application/pdf");
    try {
      doc.asPDF()?.disableJS();
      // Undo/redo is MuPDF's own journal; without this every beginOperation is a
      // silent no-op (measured: canUndo() stays false and nothing is journaled).
      doc.asPDF()?.enableJournal();
    } catch (err) {
      doc.destroy();
      throw err;
    }
    return { doc, openMs: Math.round(performance.now() - started) };
  }

  function closeDocument(doc: MupdfDocument | null) {
    if (!doc) return;
    try {
      doc.destroy();
    } catch {
      // A document that fails to destroy must not take the session down.
    }
  }

  function pageSizes(doc: MupdfDocument): PageSize[] {
    // ponytail: every page's box is read here (2 ms for 500 pages); make it lazy if a
    // many-thousand-page file ever shows up.
    const count = doc.countPages();
    const sizes: PageSize[] = [];
    for (let index = 0; index < count; index++) {
      const page = doc.loadPage(index);
      try {
        sizes.push(sizeOf(page.getBounds()));
      } finally {
        page.destroy();
      }
    }
    return sizes;
  }

  /**
   * Document facts. `needsPassword` is supplied by the caller whenever the
   * session already knows it: asking the document again re-locks it (see Session).
   */
  function detect(doc: MupdfDocument, needsPassword?: boolean): DocumentInfo {
    const info: DocumentInfo = {
      encrypted: false, needsPassword: false, algorithm: "none", repaired: false, signatures: 0, certified: false,
      xfa: false, attachments: 0, openAction: false, tagged: false, versions: 0, printable: true,
      permissions: { print: true, copy: true, edit: true, annotate: true, form: true },
    };
    info.needsPassword = needsPassword ?? attempt(() => doc.needsPassword(), false);
    info.printable = attempt(() => doc.hasPermission("print"), true);
    const pdf = doc.asPDF();
    if (!pdf) return info;
    let trailer: MupdfObject | null = null;
    try {
      trailer = pdf.getTrailer() as MupdfObject;
    } catch {
      trailer = null;
    }
    // Reading `/Encrypt` needs no password: /V /R /Length /CF name the algorithm
    // even while the document is locked. Permissions come from mupdf, which reads
    // /P; an owner-authenticated session reports all five (the owner is not bound).
    info.algorithm = algorithmOf(trailer);
    info.permissions = permissionsOf(doc);
    const root = at(trailer, "Root");
    const acro = at(root, "AcroForm");
    const encryption = attempt(() => doc.getMetaData("encryption"), undefined);
    info.encrypted = (encryption !== undefined && encryption !== "None") || !!at(trailer, "Encrypt");
    info.repaired = attempt(() => pdf.wasRepaired(), false);
    info.versions = Math.max(0, attempt(() => pdf.countVersions(), 1) - 1); // saved prior revisions, 0 for a fresh file
    info.xfa = !!at(acro, "XFA");
    info.signatures = countSignatures(acro);
    info.certified = !!at(at(root, "Perms"), "DocMDP");
    info.openAction = !!at(root, "OpenAction") || !!at(root, "AA");
    const marked = at(at(root, "MarkInfo"), "Marked");
    info.tagged = !!marked && attempt(() => marked.asBoolean(), false);
    info.attachments = attempt(() => Object.keys(pdf.getEmbeddedFiles()).length, 0);
    return info;
  }

  /** Compare §9.4: the stext lines of one page with their own y-down bbox (parseLines keeps the baseline). */
  function pageLines(doc: MupdfDocument, pageIndex: number): CompareLine[] {
    const page = doc.loadPage(pageIndex);
    try {
      const structured = page.toStructuredText("preserve-spans");
      try {
        return stextLines(structured.asJSON());
      } finally {
        structured.destroy();
      }
    } finally {
      page.destroy();
    }
  }

  function renderPage(doc: MupdfDocument, pageIndex: number, scale: number): RenderResult {
    if (!Number.isInteger(pageIndex)) throw new Error("render: page must be an integer");
    if (typeof scale !== "number" || !Number.isFinite(scale) || scale < MIN_SCALE || scale > MAX_SCALE) {
      throw new Error(`render: scale must be between ${MIN_SCALE} and ${MAX_SCALE}`);
    }
    const count = doc.countPages();
    if (pageIndex < 0 || pageIndex >= count) {
      throw new Error(`render: page ${pageIndex} is out of range (0..${count - 1})`);
    }
    const started = performance.now();
    const page = doc.loadPage(pageIndex);
    try {
      const { width, height } = sizeOf(page.getBounds());
      const wantedWidth = Math.ceil(width * scale);
      const wantedHeight = Math.ceil(height * scale);
      if (wantedWidth > MAX_PIXEL_EDGE || wantedHeight > MAX_PIXEL_EDGE) {
        throw new Error(`render: ${wantedWidth}×${wantedHeight} px is over the ${MAX_PIXEL_EDGE} px edge limit`);
      }
      const pixmap = page.toPixmap(mupdf.Matrix.scale(scale, scale), mupdf.ColorSpace.DeviceRGB, false);
      try {
        const pixelWidth = pixmap.getWidth();
        const pixelHeight = pixmap.getHeight();
        if (pixelWidth > MAX_PIXEL_EDGE || pixelHeight > MAX_PIXEL_EDGE) {
          throw new Error(`render: ${pixelWidth}×${pixelHeight} px is over the ${MAX_PIXEL_EDGE} px edge limit`);
        }
        const rgba = toRgba(pixmap);
        const paintMs = Math.round(performance.now() - started);
        const structured = page.toStructuredText("preserve-spans");
        try {
          return {
            width, height, pixelWidth, pixelHeight,
            pixels: rgba.buffer as ArrayBuffer,
            text: structured.asText(),
            lines: parseLines(structured.asJSON()),
            nonWhite: countNonWhite(rgba),
            paintMs,
            renderMs: Math.round(performance.now() - started),
          };
        } finally {
          structured.destroy();
        }
      } finally {
        pixmap.destroy();
      }
    } finally {
      page.destroy();
    }
  }

  /**
   * Thumbnail: pixels only, no structured text. MIN_SCALE deliberately does not
   * apply — a 2000 pt page still has to yield a ≤ maxEdge px thumbnail — while
   * MAX_SCALE and MAX_PIXEL_EDGE still do. The caller owns the pixels buffer.
   */
  function renderThumbnail(doc: MupdfDocument, pageIndex: number, maxEdge: number) {
    if (!Number.isInteger(pageIndex)) throw new Error("thumbnail: page must be an integer");
    if (!Number.isInteger(maxEdge) || maxEdge < THUMB_MIN_EDGE || maxEdge > THUMB_MAX_EDGE) {
      throw new Error(`thumbnail: maxEdge must be an integer between ${THUMB_MIN_EDGE} and ${THUMB_MAX_EDGE}`);
    }
    const count = doc.countPages();
    if (pageIndex < 0 || pageIndex >= count) {
      throw new Error(`thumbnail: page ${pageIndex} is out of range (0..${count - 1})`);
    }
    const page = doc.loadPage(pageIndex);
    try {
      const { width, height } = sizeOf(page.getBounds());
      if (!(width > 0) || !(height > 0)) throw new Error("thumbnail: page has no area");
      const scale = Math.min(maxEdge / width, maxEdge / height, MAX_SCALE);
      const pixmap = page.toPixmap(mupdf.Matrix.scale(scale, scale), mupdf.ColorSpace.DeviceRGB, false);
      try {
        const pixelWidth = pixmap.getWidth();
        const pixelHeight = pixmap.getHeight();
        if (pixelWidth > MAX_PIXEL_EDGE || pixelHeight > MAX_PIXEL_EDGE) {
          throw new Error(`thumbnail: ${pixelWidth}×${pixelHeight} px is over the ${MAX_PIXEL_EDGE} px edge limit`);
        }
        const rgba = toRgba(pixmap);
        return { width, height, pixelWidth, pixelHeight, pixels: rgba.buffer as ArrayBuffer };
      } finally {
        pixmap.destroy();
      }
    } finally {
      page.destroy();
    }
  }

  /**
   * Whole-document search in page order, stopping at the hit budget.
   * ponytail: mupdf caps one page's `search` at 500 hits, so a single page with
   * more than 500 matches is silently under-counted; per-page paging if it matters.
   */
  function searchDocument(doc: MupdfDocument, query: string, maxHits?: number) {
    const budget = hitBudget(maxHits);
    const started = performance.now();
    const hits: SearchHit[] = [];
    let truncated = false;
    const count = doc.countPages();
    for (let index = 0; index < count; index++) {
      const page = doc.loadPage(index);
      try {
        for (const match of page.search(query, {})) {
          if (hits.length >= budget) {
            truncated = true;
            break;
          }
          hits.push({ page: index, quads: match.map(quad => [...quad]) });
        }
      } finally {
        page.destroy();
      }
      if (truncated) break;
    }
    return { hits, total: hits.length, truncated, searchMs: Math.round(performance.now() - started) };
  }

  /** The renderer's security view: never a password, never the in-memory key. */
  function securityState(session: Session): SecurityState {
    const setting = session.security;
    const info = session.doc ? detect(session.doc, session.locked) : null;
    const pending = session.securityDirty;
    if (setting?.mode === "aes-256") {
      return { mode: "aes-256", algorithm: "aes-256", permissions: permissionNames(setting.flags), auth: session.auth, pending };
    }
    if (setting?.mode === "none") {
      return {
        mode: "none", algorithm: "none", auth: session.auth, pending,
        permissions: permissionNames({ print: true, copy: true, edit: true, annotate: true, form: true }),
      };
    }
    return {
      mode: "keep", algorithm: info?.algorithm ?? "none", auth: session.auth, pending,
      permissions: info ? permissionNames(info.permissions) : [],
    };
  }

  /** One password the Security dialog submitted; the comma is mupdf's option separator. */
  function checkPassword(value: unknown, label: string): string {
    if (typeof value !== "string") throw new Error(`security-set: ${label} must be a string`);
    if (value.length > MAX_PASSWORD_CHARS) {
      throw new Error(`security-set: ${label} must be at most ${MAX_PASSWORD_CHARS} characters`);
    }
    if (value.includes(",")) throw new Error(`security-set: ${label} cannot contain a comma`);
    return value;
  }

  /** The five checkboxes; a missing one is false (the dialog always sends all five). */
  function permissionFlags(value: SecuritySetRequest["permissions"]): DocumentPermissions {
    const flag = (name: keyof DocumentPermissions) => {
      const raw = value?.[name];
      if (raw !== undefined && typeof raw !== "boolean") throw new Error(`security-set: ${name} must be true or false`);
      return raw === true;
    };
    return { print: flag("print"), copy: flag("copy"), edit: flag("edit"), annotate: flag("annotate"), form: flag("form") };
  }

  /** Validates a `security-set` request into the session's pending setting. */
  function securitySetting(request: SecuritySetRequest): SecuritySetting {
    if (typeof request.mode !== "string" || !(SECURITY_MODES as readonly string[]).includes(request.mode)) {
      throw new Error(`security-set: mode must be one of ${SECURITY_MODES.join(", ")}`);
    }
    if (request.mode === "keep") return { mode: "keep" };
    if (request.mode === "none") return { mode: "none" };
    const userPassword = checkPassword(request.userPassword, "user password");
    if (!userPassword) throw new Error("security-set: AES-256 needs a non-empty user password");
    // A blank owner password means "the same as the user password" (the dialog says so).
    const ownerPassword = request.ownerPassword === undefined || request.ownerPassword === ""
      ? userPassword
      : checkPassword(request.ownerPassword, "owner password");
    const flags = permissionFlags(request.permissions);
    return { mode: "aes-256", userPassword, ownerPassword, flags, permissions: permissionInt(flags) };
  }

  /**
   * The write options for the next save. `keep` re-encrypts an authenticated
   * document with its original algorithm and both passwords, and is a no-op on a
   * plain one (gate: `.stage/gate-u10.mjs`, findings in `p2-encrypt.md`).
   */
  function saveOptions(session: Session): string {
    const setting = session.security;
    if (setting?.mode === "aes-256") {
      return `${SAVE_OPTIONS},encrypt=aes-256,user-password=${setting.userPassword}`
        + `,owner-password=${setting.ownerPassword},permissions=${setting.permissions}`;
    }
    if (setting?.mode === "none") return `${SAVE_OPTIONS},encrypt=none`;
    return `${SAVE_OPTIONS},encrypt=keep`;
  }

  /**
   * Serializes the open document for a save. Fails closed: signed, certified and
   * still-locked documents are refused before anything is written. An unlocked
   * encrypted document is saved with its own encryption unless the session holds a
   * pending security change.
   */
  function serializeDocument(session: Session) {
    const doc = session.doc;
    if (!doc) failReason("no-document", "save: no document open");
    // Refuse an oversized source before any parsing or serialization work.
    if (session.sourceBytes > MAX_SAVE_BYTES) {
      failReason("too-large", `save: ${session.sourceBytes} bytes is over the ${MAX_SAVE_BYTES} byte save ceiling`);
    }
    if (session.locked) {
      failReason("encrypted", "save: this document is encrypted; enter its password before saving");
    }
    const info = detect(doc, session.locked);
    if (info.signatures > 0 || info.certified) {
      failReason("read-only", "save: signed or certified documents are read-only");
    }
    const pdf = doc.asPDF();
    if (!pdf) failReason("invalid", "save: the open document is not a PDF");
    const started = performance.now();
    let buffer: ReturnType<typeof pdf.saveToBuffer>;
    try {
      buffer = pdf.saveToBuffer(saveOptions(session));
    } catch (err) {
      return failReason("invalid", `save: ${errorMessage(err)}`);
    }
    let bytes: ArrayBuffer;
    try {
      const view = buffer.asUint8Array();
      // Check before copying: an oversized rewrite must not be copied out of the heap.
      if (view.byteLength > MAX_SAVE_BYTES) {
        failReason("too-large", `save: the copy would exceed the ${MAX_SAVE_BYTES} byte save ceiling`);
      }
      bytes = copyBytes(view);
    } finally {
      buffer.destroy();
    }
    return { bytes, saveMs: Math.round(performance.now() - started) };
  }

  /**
   * Reopens saved bytes in a throwaway document and checks it still looks like
   * the session document. An encrypted copy is opened with the session's own
   * password (worker memory only); a mismatch is `SaveReason "invalid"`.
   * Never replaces the session and never returns a retained copy: the caller's
   * own bytes are handed back for transfer.
   */
  function validateDocument(session: Session, pdf: ArrayBuffer, sanitized = false) {
    const open = session.doc;
    if (!open) failReason("no-document", "validate: no document open");
    if (!(pdf instanceof ArrayBuffer)) failReason("invalid", "validate: expected an ArrayBuffer");
    if (pdf.byteLength < 5) failReason("invalid", "validate: not a PDF (too few bytes)");
    if (pdf.byteLength > MAX_SAVE_BYTES) {
      failReason("too-large", `validate: PDF exceeds the ${MAX_SAVE_BYTES} byte save ceiling`);
    }
    let check: MupdfDocument;
    try {
      check = mupdf.Document.openDocument(pdf, "application/pdf");
      check.asPDF()?.disableJS();
    } catch (err) {
      failReason("invalid", `validate: ${errorMessage(err)}`);
    }
    try {
      // The copy is a fresh open, so it starts locked the way the file is: ask
      // `needsPassword()` once, here, before authenticating (it re-locks).
      const copied = detect(check);
      if (copied.needsPassword) {
        const setting = session.security;
        const password = setting?.mode === "aes-256" ? setting.userPassword : session.password;
        if (!password || !attempt(() => check.authenticatePassword(password), 0)) {
          failReason("invalid", "validate: the saved copy did not open with its own password");
        }
      }
      if (attempt(() => check.asPDF()?.wasRepaired() ?? false, false)) {
        failReason("invalid", "validate: the saved copy needed repair");
      }
      const pageCount = check.countPages();
      const expected = open.countPages();
      if (pageCount !== expected) {
        failReason("invalid", `validate: the saved copy has ${pageCount} pages, expected ${expected}`);
      }
      const before = detect(open, session.locked);
      const after = detect(check, false);
      const expectedEncrypted = session.security?.mode === "none" ? false : before.encrypted;
      if (sanitized ? after.encrypted !== expectedEncrypted : !sameSaveShape(before, after, expectedEncrypted)) {
        failReason("invalid", "validate: the saved copy changed the document's declared features");
      }
      const setting = session.security;
      if (setting?.mode === "aes-256") {
        // Compare the /P bits themselves: the copy's owner password may be the same
        // string as its user password, and then mupdf reports all five permissions.
        const bits = permissionBitsOf(check);
        for (const [name, bit] of PERMISSION_BITS) {
          if (((bits & bit) === bit) !== ((setting.permissions & bit) === bit)) {
            failReason("invalid", `validate: the saved copy's ${name} permission does not match what was requested`);
          }
        }
      } else if (setting?.mode === "none") {
        if (!samePermissions(after.permissions, { print: true, copy: true, edit: true, annotate: true, form: true })) {
          failReason("invalid", "validate: the saved copy is still restricted after encryption was removed");
        }
      } else if (!samePermissions(after.permissions, before.permissions)) {
        failReason("invalid", "validate: the saved copy has different permissions than the source");
      }
      return { pageCount };
    } finally {
      closeDocument(check);
    }
  }

  /**
   * Live journal position. Only bumps inside beginOperation/endOperation; an
   * operation that changed nothing leaves it alone, so it doubles as the dirty
   * baseline (see Session.savedPosition).
   */
  function journalPosition(doc: MupdfDocument | null): number {
    const pdf = doc?.asPDF();
    return pdf ? attempt(() => pdf.getJournal().position, 0) : 0;
  }

  /** The dirty/undo slice every mutation, undo, redo and saved reply carries. */
  function journalState(session: Session): JournalState {
    const pdf = session.doc?.asPDF() ?? null;
    return {
      revision: session.revision,
      dirty: !!pdf && (journalPosition(session.doc) !== session.savedPosition || session.securityDirty),
      canUndo: pdf ? attempt(() => pdf.canUndo(), false) : false,
      canRedo: pdf ? attempt(() => pdf.canRedo(), false) : false,
    };
  }

  /** `/Rotate` in 0/90/180/270 for one page; a missing or unusable value reads as 0. */
  function pageRotation(doc: MupdfDocument, page: number): number {
    const pdf = doc.asPDF();
    if (!pdf) return 0;
    const value = attempt(() => {
      const rotate = pdf.findPage(page).getInheritable("Rotate") as MupdfObject;
      return rotate && !rotate.isNull() ? rotate.asNumber() : 0;
    }, 0);
    return ((Math.trunc(value) % 360) + 360) % 360;
  }

  /**
   * The page count, sizes and rotations every page op, undo and redo reply carries,
   * so the renderer rebuilds its page list without reopening.
   * ponytail: the whole page tree is walked here (measured on large.pdf: 500 pages,
   * 2-6 ms for sizes+rotations together, the 50 ms budget in contract-unit3); cache
   * keyed on `revision` only if a much larger document shows up.
   */
  function pageState(session: Session): PageState {
    const doc = session.doc as MupdfDocument;
    return { pageCount: doc.countPages(), pages: pageSizes(doc), rotations: pageRotations(doc) };
  }

  /** `/Rotate` for every page, in page order. */
  function pageRotations(doc: MupdfDocument): number[] {
    const rotations: number[] = [];
    for (let page = 0; page < doc.countPages(); page++) rotations.push(pageRotation(doc, page));
    return rotations;
  }

  /** `journalState` plus the page geometry; every mutation, undo and redo reply spreads this. */
  /**
   * P3 U1: the document generation counter and the last discovery result. The
   * counter only ever grows, so a handle from a closed document can never name
   * the document that replaced it, and the cache is dropped on every open/close
   * and ignored as soon as the revision differs.
   */
  let generationCounter = 0;
  let editScanCache: { generation: number; revision: number; page: number; records: Map<string, EditTargetRecord>; paragraphs: Map<string, EditParagraph> } | null = null;

  /**
   * P3 U1: the only way a later edit names a discovered target. Every field is
   * checked, and the handle is resolved against a fresh bounded re-enumeration
   * of the page, so no renderer-supplied offset, object number or bounding box
   * is ever trusted. Not a worker command: U2 calls it inside its own dispatch.
   */
  /**
   * U5: the page's records, from the cached scan when the revision still matches and
   * from a fresh bounded scan otherwise. The scan is the same one `edit-targets` and
   * `resolveEditTarget` use, so a preview can never see a different page than an edit.
   */
  function editScanFor(session: Session, page: number): Map<string, EditTargetRecord> {
    const cached = editScanCache;
    if (cached && cached.generation === session.generation && cached.revision === session.revision && cached.page === page) {
      return cached.records;
    }
    const scan = scanEditTargets(session.doc as MupdfDocument, page);
    editScanCache = { generation: session.generation, revision: session.revision, page, records: scan.records, paragraphs: scan.paragraphs };
    return scan.records;
  }

  /** P6 U2 §9.16: the page's paragraphs from the same scan `editScanFor` caches. */
  function editParagraphsFor(session: Session, page: number): Map<string, EditParagraph> {
    editScanFor(session, page);
    return editScanCache!.paragraphs;
  }

  /**
   * U5: every run on the page (or one run) whose text contains `find`, judged by the
   * same `editPlanText` the single edit uses. Nothing is written and no operation is
   * opened. A shared placement is rejected because a batch never privatises.
   */
  function buildReplacePreview(
    session: Session, doc: MupdfDocument, page: number, find: string, replace: string, targetId?: string,
  ): { matches: EditReplaceMatch[]; rejected: EditReplaceRejected[]; plans: Map<string, EditTextPlan>; token: string } {
    const records = editScanFor(session, page);
    const considered = targetId === undefined
      ? [...records.values()].filter(record => record.kind === "text")
      : [records.get(targetId)].filter((record): record is EditTargetRecord => record !== undefined);
    if (targetId !== undefined && considered.length === 0) {
      failEditReplace("stale-target", `edit-replace: no target ${targetId} on page ${page}`);
    }
    const matches: EditReplaceMatch[] = [];
    const rejected: EditReplaceRejected[] = [];
    const plans = new Map<string, EditTextPlan>();
    const pageView = doc.loadPage(page) as MupdfPage;
    let structured: EditStructuredText | null = null;
    let runs: EditRun[] | null = null;
    let replyText = 0;
    try {
      // §7.3: one structured-text walk per preview, shared by every plan.
      try {
        structured = pageView.toStructuredText("preserve-spans");
        runs = editRunsOf(structured);
      } catch {
        structured = null;
        runs = null;
      }
      for (const record of considered) {
        const text = record.text;
        if (text === undefined || !text.includes(find)) continue;
        const count = text.split(find).length - 1;
        // §7.2: only a Form placement is refused here; a shared stream or inherited
        // resources are privatised once per element by the apply, never by the preview.
        if (record.support !== "candidate") {
          replyText += text.length;
          rejected.push({ targetId: record.targetId, text, count, detail: record.reason ?? "not-candidate", reason: record.reason });
          continue;
        }
        if (record.cow?.form) {
          replyText += text.length;
          rejected.push({ targetId: record.targetId, text, count, detail: "shared-placement" });
          continue;
        }
        // §7.1: bound the projected replacement before `next` is built.
        const projected = text.length + count * (replace.length - find.length);
        if (projected > MAX_EDIT_TEXT_CHARS) {
          replyText += text.length;
          rejected.push({ targetId: record.targetId, text, count, detail: "too-large" });
          continue;
        }
        const next = text.split(find).join(replace);
        replyText += text.length + next.length;
        if (replyText > MAX_EDIT_REPLY_TEXT_CHARS) {
          failEditReplace("too-large", `edit-replace: the reply text is over the ${MAX_EDIT_REPLY_TEXT_CHARS} character ceiling`);
        }
        try {
          const plan = editPlanText(doc, record, next, pageView, runs ?? undefined);
          plans.set(record.targetId, plan);
          matches.push({
            targetId: record.targetId, text, next, count,
            oldWidth: plan.oldWidth, newWidth: plan.newWidth, tailFollows: record.tailFollows === true,
          });
        } catch (err) {
          const tagged = err as { detail?: string; extra?: Record<string, unknown> } | null;
          rejected.push({ targetId: record.targetId, text, count, detail: tagged?.detail ?? "invalid", ...(tagged?.extra ?? {}) });
        }
      }
    } finally {
      structured?.destroy();
      pageView.destroy();
    }
    // Rejected rows after the last match are charged too.
    if (replyText > MAX_EDIT_REPLY_TEXT_CHARS) {
      failEditReplace("too-large", `edit-replace: the reply text is over the ${MAX_EDIT_REPLY_TEXT_CHARS} character ceiling`);
    }
    // §7.7: the token covers the scope as well as the page, find, replace and the runs.
    const scope = targetId ?? "page";
    const token = editHash(`${session.generation}|${session.revision}|${page}|${scope}|${find}|${replace}|${matches.map(match => match.targetId).join(",")}`);
    return { matches, rejected, plans, token };
  }

  function resolveEditTarget(
    session: Session,
    request: { generation: unknown; revision: unknown; page: unknown; targetId: unknown },
  ): EditTargetRecord {
    const doc = session.doc;
    if (!doc) failReason("no-document", "resolveEditTarget: no document open");
    if (session.locked) failReason("read-only", "resolveEditTarget: this document is read-only until it is unlocked");
    if (request.generation !== session.generation) failReason("invalid", "resolveEditTarget: stale document generation");
    if (request.revision !== session.revision) failReason("invalid", "resolveEditTarget: stale revision, the document changed");
    const page = checkPage(doc, request.page, "resolveEditTarget");
    if (typeof request.targetId !== "string" || !request.targetId) {
      failReason("invalid", "resolveEditTarget: targetId must be a non-empty string");
    }
    const record = editScanFor(session, page).get(request.targetId);
    if (!record) failReason("invalid", `resolveEditTarget: no target ${String(request.targetId)} on page ${page}`);
    return record as EditTargetRecord;
  }

  /** U5: the literal `find`/`replace` a preview or apply names, or `invalid`. */
  function checkReplaceArgs(find: unknown, replace: unknown): { find: string; replace: string } {
    if (typeof find !== "string" || find.length < 1 || find.length > MAX_REPLACE_FIND_CHARS) {
      failEditReplace("invalid", `edit-replace: find must be 1..${MAX_REPLACE_FIND_CHARS} characters`);
    }
    if (typeof replace !== "string" || replace.length > MAX_EDIT_TEXT_CHARS) {
      failEditReplace("invalid", `edit-replace: replace must be at most ${MAX_EDIT_TEXT_CHARS} characters`);
    }
    return { find, replace };
  }

  function editState(session: Session) {
    return { ...journalState(session), ...pageState(session) };
  }


  /**
   * The single gate every mutation routes through: an unlocked, signed-free document
   * whose pending security and permissions allow this operation.
   */
  function requireMutable(session: Session, op: string): MupdfDocument {
    const doc = session.doc;
    if (!doc) failReason("no-document", `${op}: no document open`);
    if (session.locked) failReason("read-only", `${op}: this document is read-only until it is unlocked`);
    const info = detect(doc as MupdfDocument, session.locked);
    if (info.signatures > 0 || info.certified) failReason("read-only", `${op}: this document is read-only`);
    const need = OP_PERMISSION[op] ?? "edit";
    if (!allowed(info, need)) {
      const what = need === "any" ? "changes" : need === "form" ? "filling form fields" : need === "annotate" ? "annotations" : need === "edit+annotate" ? "authoring form fields" : "changes to its content";
      failReason("read-only", `${op}: this document does not permit ${what}`);
    }
    return doc as MupdfDocument;
  }

  function checkPage(doc: MupdfDocument, page: unknown, op: string): number {
    const count = doc.countPages();
    if (!Number.isInteger(page) || (page as number) < 0 || (page as number) >= count) {
      failReason("invalid", `${op}: page must be an integer in 0..${Math.max(0, count - 1)}`);
    }
    return page as number;
  }

  function checkContents(contents: unknown, op: string): string {
    if (typeof contents !== "string" || contents.length > MAX_NOTE_CHARS) {
      throw new Error(`${op}: contents must be a string of at most ${MAX_NOTE_CHARS} characters`);
    }
    return contents;
  }

  /** A fixed-length list of finite numbers, copied out of the request (holes are missing). */
  function finiteNumbers(value: unknown, count: number, label: string, op: string): number[] {
    if (!Array.isArray(value) || value.length !== count) {
      throw new Error(`${op}: ${label} must be ${count} finite numbers`);
    }
    const numbers: number[] = [];
    for (let index = 0; index < count; index++) {
      const item = value[index];
      if (typeof item !== "number" || !Number.isFinite(item)) {
        throw new Error(`${op}: ${label} must be ${count} finite numbers`);
      }
      numbers.push(item);
    }
    return numbers;
  }

  /** The page box in page points: the space search quads, points and annotation rects share. */
  function pageBox(doc: MupdfDocument, pageIndex: number): number[] {
    const page = doc.loadPage(pageIndex) as MupdfPage;
    try {
      return [...page.getBounds()];
    } finally {
      page.destroy();
    }
  }

  function onPage(box: number[], x: number, y: number): boolean {
    return x >= box[0] && x <= box[2] && y >= box[1] && y <= box[3];
  }

  /** A point that must sit inside the page box; off-page geometry is refused, never clamped. */
  function checkPoint(box: number[], point: unknown, op: string): [number, number] {
    const [x, y] = finiteNumbers(point, 2, "point", op);
    if (!onPage(box, x, y)) throw new Error(`${op}: point [${x}, ${y}] is off the page`);
    return [x, y];
  }

  /** Search-hit quads (8 finite numbers each, every corner on the page) copied for setQuadPoints. */
  function checkQuads(box: number[], quads: unknown, op: string): Quad[] {
    if (!Array.isArray(quads) || quads.length < 1 || quads.length > MAX_QUADS) {
      throw new Error(`${op}: quads must be 1..${MAX_QUADS} quads`);
    }
    return quads.map((quad, index) => {
      const corners = finiteNumbers(quad, 8, `quad ${index}`, op) as Quad;
      for (let corner = 0; corner < 8; corner += 2) {
        if (!onPage(box, corners[corner], corners[corner + 1])) {
          throw new Error(`${op}: quad ${index} is off the page`);
        }
      }
      return corners;
    });
  }

  /** A rect with positive area and both corners on the page; refused, never clamped. */
  function checkRect(box: number[], rect: unknown, op: string): Rect {
    const [x0, y0, x1, y1] = finiteNumbers(rect, 4, "rect", op);
    if (!(x1 > x0) || !(y1 > y0)) throw new Error(`${op}: rect must have a positive area`);
    if (!onPage(box, x0, y0) || !onPage(box, x1, y1)) {
      throw new Error(`${op}: rect [${x0}, ${y0}, ${x1}, ${y1}] is off the page`);
    }
    return [x0, y0, x1, y1];
  }

  /** A Line's two on-page endpoints; an identical pair is a dot, not a line. */
  function checkLine(box: number[], points: unknown, op: string): [[number, number], [number, number]] {
    if (!Array.isArray(points) || points.length !== 2) {
      throw new Error(`${op}: a Line takes exactly two points`);
    }
    const [start, end] = [checkPoint(box, points[0], op), checkPoint(box, points[1], op)];
    if (start[0] === end[0] && start[1] === end[1]) {
      throw new Error(`${op}: a Line needs two distinct points`);
    }
    return [start, end];
  }

  /** Ink strokes: 1..64 strokes of 2..2048 on-page points each. */
  function checkStrokes(box: number[], strokes: unknown, op: string): [number, number][][] {
    if (!Array.isArray(strokes) || strokes.length < 1 || strokes.length > MAX_INK_STROKES) {
      throw new Error(`${op}: strokes must be 1..${MAX_INK_STROKES} strokes`);
    }
    return strokes.map((stroke, index) => {
      if (!Array.isArray(stroke) || stroke.length < MIN_INK_POINTS || stroke.length > MAX_INK_POINTS) {
        throw new Error(`${op}: stroke ${index} must be ${MIN_INK_POINTS}..${MAX_INK_POINTS} points`);
      }
      return stroke.map((point, position) => checkPoint(box, point, `${op}: stroke ${index} point ${position}`));
    });
  }

  /** One of the six icon stamps mupdf knows how to draw. */
  function checkIcon(icon: unknown, op: string): string {
    if (typeof icon !== "string" || !(STAMP_ICONS as readonly string[]).includes(icon)) {
      throw new Error(`${op}: icon must be one of ${STAMP_ICONS.join(", ")}`);
    }
    return icon;
  }

  function checkSignName(name: unknown, op: string): string {
    if (typeof name !== "string" || !name.trim() || name.length > MAX_SIGN_NAME_CHARS) {
      throw new Error(`${op}: a signature name must be 1..${MAX_SIGN_NAME_CHARS} characters`);
    }
    return name;
  }

  /** The image bytes decoded up front, so a bad file is a refusal and not a partial add. */
  function decodeStampImage(value: unknown, op: string): MupdfImage {
    const image = value as { name?: unknown; bytes?: unknown } | null;
    if (!image || typeof image.name !== "string" || !image.name || image.name.length > 255) {
      throw new Error(`${op}: a stamp image needs a name of at most 255 characters`);
    }
    if (!(image.bytes instanceof ArrayBuffer) || image.bytes.byteLength < 1) {
      throw new Error(`${op}: stamp image ${image.name} has no bytes`);
    }
    if (image.bytes.byteLength > MAX_STAMP_IMAGE_BYTES) {
      throw new Error(`${op}: ${image.name} is over the ${MAX_STAMP_IMAGE_BYTES} byte image ceiling`);
    }
    let decoded: MupdfImage;
    try {
      decoded = new mupdf.Image(new Uint8Array(image.bytes));
    } catch (err) {
      throw new Error(`${op}: ${image.name} is not a PNG or JPEG mupdf can read (${errorMessage(err)})`);
    }
    if (!(decoded.getWidth() > 0) || !(decoded.getHeight() > 0)) {
      decoded.destroy();
      throw new Error(`${op}: ${image.name} has no pixels`);
    }
    return decoded;
  }

  /** `Signed by <name> on <YYYY-MM-DD>`, the local date at add time. */
  function signatureContents(name: string, now = new Date()): string {
    const pad = (value: number) => String(value).padStart(2, "0");
    const date = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
    return `Signed by ${name} on ${date}`;
  }

  /** The FreeText box: fixed size with its top-left moved inside the page box (never shrunk). */
  function freeTextBox(box: number[], point: [number, number]): Rect {
    const [width, height] = FREE_TEXT_SIZE;
    const x = Math.max(box[0], Math.min(point[0], box[2] - width));
    const y = Math.max(box[1], Math.min(point[1], box[3] - height));
    return [x, y, x + width, y + height];
  }

  /**
   * fill-anywhere D1: a FreeText rect that draws every `\n` line on one line, measured with the
   * shared base-14 advance (`editAdvance`). Probed on mupdf 1.28.1 (Helv 12, border 0): no padding,
   * line height 1.2 x size, and a line wraps only when its advance exceeds the rect width; +0.5 pt
   * absorbs float rounding. `point` is the left edge and the first line's vertical centre; the rect
   * is moved inside the page box, and a line wider than the page gets the page width (and wraps).
   */
  function freeTextFit(box: number[], point: [number, number], contents: string): Rect {
    const size = 12, lineHeight = size * 1.2;
    const lines = contents.split("\n");
    const font = base14Font("Helvetica") as unknown as EditRunChar["font"];
    const advance = (line: string) => editAdvance([{ ch: "", font, size, quad: [] }], line)?.width ?? line.length * size;
    const width = Math.max(1, ...lines.map(advance)) + 0.5;
    const height = lines.length * lineHeight;
    // Review F4: a line wider than the page would wrap and be clipped by the fitted height, so it is refused.
    if (width > box[2] - box[0] || height > box[3] - box[1]) failEditText("overflow", "The text does not fit on the page; use shorter lines");
    const x = Math.max(box[0], Math.min(point[0], box[2] - width));
    const y = Math.max(box[1], Math.min(point[1] - lineHeight / 2, box[3] - height));
    return [x, y, x + width, y + height];
  }

  /** fill-anywhere D1: refuse a character the shared WinAnsi encoder cannot write (the `text-add` shape). */
  function checkFreeTextEncoding(contents: string, op: string): void {
    const missing = [...new Set([...contents].filter(ch => ch !== "\n" && winAnsiByte(ch.codePointAt(0) ?? 0) < 0))];
    if (missing.length) failEditText("unsupported-encoding", `${op}: unsupported WinAnsi characters`, { missing: missing.slice(0, 20) });
  }

  /** The annotation's own object number: stable across renders, validated per page. */
  function annotId(annot: MupdfAnnotation): number {
    return attempt(() => annot.getObject().asIndirect(), 0);
  }

  function findAnnotation(page: MupdfPage, annotId2: unknown, op: string): MupdfAnnotation {
    if (!Number.isInteger(annotId2)) throw new Error(`${op}: id must be an integer`);
    for (const annot of page.getAnnotations()) {
      if (annotId(annot) === annotId2) return annot;
    }
    throw new Error(`${op}: no annotation ${String(annotId2)} on this page`);
  }

  /**
   * One mutation = one journal operation, and one revision bump. Validation is
   * the caller's job and happens before `work` runs, so a throw here is MuPDF's.
   *
   * A failure after a native write must not leave a partial write: `endOperation`
   * runs only on success, and a throw abandons the operation (measured: abandon
   * restores streams and dictionaries and closes the op; ending a failed op leaves
   * it on the undo stack). `abandonOperation` must never follow `endOperation` and
   * never run twice, so the two paths are exclusive here.
   *
   * Ceiling (fable51 review-p3-unit1-gateA.md, accepted): an abandoned operation
   * that wrote anything drops the redo branch natively, exactly as a successful
   * mutation would. When that branch contained the saved state, the step count can
   * land on `savedPosition` again without the bytes matching, so `savedPosition` is
   * made unreachable and dirty stays true until the next save.
   */
  function mutate(session: Session, doc: MupdfDocument, op: string, work: () => void) {
    if (session.batch) { if (session.batch === true) work(); return journalState(session); }
    const pdf = doc.asPDF();
    if (!pdf) failReason("invalid", `${op}: the open document is not a PDF`);
    // Both reads happen before `beginOperation`: the journal throws while one is open.
    const branched = attempt(() => pdf.canRedo(), false);
    const before = journalPosition(doc);
    pdf.beginOperation(op);
    let failure: unknown = null;
    try {
      work();
      pdf.endOperation();
    } catch (err) {
      failure = err;
      pdf.abandonOperation();
    }
    if (branched && !attempt(() => pdf.canRedo(), false) && session.savedPosition > before) {
      session.savedPosition = -1;
    }
    if (failure) throw failure;
    session.revision += 1;
    return journalState(session);
  }

  /** Widgets and Popups are not user-editable annotations; the list never shows them. */
  /**
   * The union of an annotation's quad points in page points (y down).
   * ponytail: markup's getRect() has no /Rect accessor in this MuPDF build (an accessor gap,
   * not a page-space difference: setQuadPoints/getQuadPoints share the search-hit space).
   */
  function quadUnion(annot: MupdfAnnotation): number[] {
    const quads = attempt(() => annot.getQuadPoints(), [] as number[][]) ?? [];
    if (!quads.length) return [];
    const xs = quads.flatMap(quad => [quad[0], quad[2], quad[4], quad[6]]);
    const ys = quads.flatMap(quad => [quad[1], quad[3], quad[5], quad[7]]);
    return [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)];
  }

  /**
   * The annotation rect (page points, y down). Markup falls back to its quad union;
   * Line and Ink have no `/Rect` accessor in this build (measured), so their geometry is.
   */
  function annotRect(annot: MupdfAnnotation): number[] {
    const rect = attempt(() => [...annot.getRect()] as number[], [] as number[]);
    if (rect.length === 4) return rect;
    const quads = quadUnion(annot);
    if (quads.length) return quads;
    const points: number[][] = [];
    for (const point of attempt(() => annot.getLine() as number[][], [])) points.push(point);
    for (const stroke of attempt(() => annot.getInkList() as number[][][], [])) points.push(...stroke);
    if (!points.length) return [];
    const xs = points.map(point => point[0]);
    const ys = points.map(point => point[1]);
    return [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)];
  }

  /**
   * Document-wide facts the `scripted` flag needs: `/Names /JavaScript` (any
   * document-level script) and the `/AcroForm /CO` calculation-order list.
   */
  function fieldContext(doc: MupdfDocument): { docScripts: boolean; co: Set<number> } {
    const pdf = doc.asPDF();
    let root: MupdfObject | null = null;
    try {
      root = at(pdf?.getTrailer() as MupdfObject, "Root");
    } catch {
      root = null;
    }
    const co = new Set<number>();
    const coArray = at(at(root, "AcroForm"), "CO");
    if (coArray?.isArray()) {
      for (let index = 0; index < coArray.length; index++) {
        co.add(attempt(() => (coArray.get(index) as MupdfObject).asIndirect(), -1));
      }
    }
    return { docScripts: !!at(at(root, "Names"), "JavaScript"), co };
  }

  /** True when an `/AA` dictionary holds any `/S /JavaScript` action. */
  function hasJavaScript(aa: MupdfObject | null): boolean {
    if (!aa?.isDictionary()) return false;
    let found = false;
    aa.forEach(value => {
      const action = at(value as MupdfObject, "S");
      if (action?.isName() && action.asName() === "JavaScript") found = true;
    });
    return found;
  }

  /**
   * A widget is scripted when the document declares scripts at all, when the
   * widget or any field above it is in `/AcroForm /CO`, or when either carries
   * a `/AA` JavaScript action. Nothing is computed or formatted on its behalf.
   */
  function widgetScripted(dict: MupdfObject, ctx: { docScripts: boolean; co: Set<number> }): boolean {
    if (ctx.docScripts) return true;
    for (let node: MupdfObject | null = dict, depth = 0; node && depth < 8; depth++) {
      if (ctx.co.has(attempt(() => node!.asIndirect(), -1))) return true;
      if (hasJavaScript(at(node, "AA"))) return true;
      node = at(node, "Parent");
    }
    return false;
  }

  function fieldKind(widget: MupdfWidget): FieldKind | null {
    if (widget.isPushButton()) return null;
    if (widget.isText()) return "text";
    if (widget.isCheckbox()) return "checkbox";
    if (widget.isRadioButton()) return "radio";
    if (widget.isComboBox()) return "combo";
    if (widget.isListBox()) return "list";
    return widget.getFieldType() === "signature" ? "signature" : null;
  }

  /** The on-state export value of a checkbox or radio: its `/AP /N` keys but `Off`. */
  function fieldOn(dict: MupdfObject): string {
    const states = at(at(dict, "AP"), "N");
    if (!states?.isDictionary()) return "";
    let on = "";
    states.forEach((_value, key) => {
      if (typeof key === "string" && key !== "Off" && !on) on = key;
    });
    return on;
  }

  /** `/V` on the widget or its nearest field ancestor, as a name. */
  function fieldValueName(dict: MupdfObject): string {
    for (let node: MupdfObject | null = dict, depth = 0; node && depth < 8; depth++) {
      const value = at(node, "V");
      if (value?.isName()) return value.asName();
      node = at(node, "Parent");
    }
    return "";
  }

  /**
   * Whether a checkbox or radio widget is currently on. `/AS` is the widget's own
   * state and wins; a file that only keeps `/V` on the field falls back to it.
   */
  function buttonChecked(dict: MupdfObject): boolean {
    const as = at(dict, "AS");
    if (as?.isName()) return as.asName() !== "Off";
    const value = fieldValueName(dict);
    return value !== "" && value !== "Off";
  }

  /** The option values a combo or list accepts: export values, else display strings. */
  function fieldOptions(widget: MupdfWidget): string[] {
    const exported = attempt(() => widget.getOptions(true), [] as string[]);
    return exported.length ? exported : attempt(() => widget.getOptions(), [] as string[]);
  }

  function listFields(doc: MupdfDocument, pageIndex: number): FieldItem[] {
    const page = doc.loadPage(pageIndex) as MupdfPage;
    try {
      const context = fieldContext(doc);
      const items: FieldItem[] = [];
      for (const widget of page.getWidgets()) {
        const kind = fieldKind(widget);
        if (!kind) continue;
        const dict = widget.getObject() as MupdfObject;
        const name = attempt(() => widget.getName(), "");
        const item: FieldItem = {
          id: annotId(widget),
          name,
          label: attempt(() => widget.getLabel(), "") || name,
          kind,
          value: kind === "checkbox" || kind === "radio"
            ? buttonChecked(dict)
            : attempt(() => widget.getValue(), ""),
          readOnly: attempt(() => widget.isReadOnly(), false),
          required: (attempt(() => widget.getFieldFlags(), 0) & mupdf.PDFWidget.FIELD_IS_REQUIRED) !== 0,
          scripted: widgetScripted(dict, context),
          rect: attempt(() => [...widget.getRect()] as number[], [] as number[]),
        };
        // U7 §2.1/§8.4: `/TU` and `/DV` live on the field dict (the parent's, for a radio
        // kid); `tu` is the raw key, `label` keeps its fallback-to-name meaning.
        const ownDict = fieldOwnDict(kind, dict);
        const ownTu = at(ownDict, "TU");
        if (ownTu?.isString()) item.tu = ownTu.asString();
        const dv = at(ownDict, "DV");
        if (kind === "checkbox" && dv?.isName()) item.default = dv.asName() !== "Off";
        else if (kind === "radio" && dv?.isName()) item.default = dv.asName();
        else if (kind !== "checkbox" && kind !== "radio" && dv?.isString()) item.default = dv.asString();
        if (kind === "checkbox" || kind === "radio") item.on = fieldOn(dict);
        if (kind === "signature") item.signed = (at(dict, "V") ?? at(at(dict, "Parent"), "V")) !== null;
        if (kind === "combo" || kind === "list") item.options = fieldOptions(widget);
        if (kind === "combo") item.edit = comboEditable(widget);
        if (kind === "text") {
          const maxLen = attempt(() => widget.getMaxLen(), 0);
          if (maxLen > 0) item.maxLen = maxLen;
          if (attempt(() => widget.isMultiline(), false)) item.multiline = true;
        }
        items.push(item);
      }
      return items;
    } finally {
      page.destroy();
    }
  }

  /**
   * P5 U3 §1.4/§9.3/§9.6: the empty, merged, top-level signature field `name` on `pageIndex`,
   * matched by `widget.getName()`; each refusal is fixed and comes before any write.
   * ponytail: only merged top-level `/FT /Sig` fields are signed in place; a field hierarchy
   * (`/Parent`, or `/FT`/`/T` inherited) is refused until a unit adopts kids.
   */
  function checkSignatureField(doc: MupdfDocument, pageIndex: number, name: string): void {
    const page = doc.loadPage(pageIndex) as MupdfPage;
    try {
      const widget = page.getWidgets().find(w => attempt(() => w.getFieldType(), "") === "signature" && attempt(() => w.getName(), "") === name);
      if (!widget) failReason("no-such-field", `sign-prepare: page ${pageIndex} has no signature field ${name}`);
      const dict = widget.getObject() as MupdfObject;
      if (at(at(dict, "Parent"), "V")) failReason("already-signed", `sign-prepare: ${name} is already signed`);
      if (at(dict, "Parent") || attempt(() => at(dict, "FT")!.asName(), "") !== "Sig" || !at(dict, "T")?.isString()) {
        failReason("invalid", `sign-prepare: ${name} is part of a field hierarchy`);
      }
      // §9.3: a widget on /Annots but not in /AcroForm /Fields is no field a verifier sees.
      const fields = existingFields(doc.asPDF()!), own = objectId(dict);
      let listed = false;
      for (let i = 0, n = fields ? fields.length : 0; i < n && !listed; i++) listed = objectId(fields!.get(i) as MupdfObject) === own;
      if (own < 0 || !listed) failReason("invalid", `sign-prepare: ${name} is not in the document's /AcroForm /Fields`);
      if (at(dict, "V")) failReason("already-signed", `sign-prepare: ${name} is already signed`);
      if (attempt(() => widget.isReadOnly(), false)) failReason("invalid", `sign-prepare: ${name} is read-only`);
      const rect = attempt(() => [...widget.getRect()], [0, 0, 0, 0]);
      if (rect[2] - rect[0] < 40 || rect[3] - rect[1] < 20) failReason("invalid", `sign-prepare: ${name} is too small for a signature appearance`);
    } finally {
      page.destroy();
    }
  }

  /** The widget's own object number on this page, the id `field-set` names. */
  function findWidget(page: MupdfPage, widgetId: unknown, op = "field-set"): MupdfWidget {
    if (!Number.isInteger(widgetId)) failField("invalid", `${op}: id must be an integer`);
    for (const widget of page.getWidgets()) {
      if (annotId(widget) === widgetId) return widget;
    }
    failField("stale-target", `${op}: no form field ${String(widgetId)} on this page`);
  }

  // -------------------------------------------------------------- U6 form designer

  /** `/F 4`: print the widget. Every authored widget carries it (contract §2.1). */
  const FIELD_F_PRINT = 4;
  /** The one appearance font the designer draws with. */
  const FIELD_DA = "/Helv 12 Tf 0 g";
  /** The checkbox on-state is fixed `Yes` (contract §2.1). */
  const FIELD_CHECK_ON = "Yes";

  /**
   * The `/AcroForm`, created when absent with `/Fields`, `/DA` and `/DR /Font /Helv`;
   * when present only the missing pieces are added (probe `ensureAcroForm`).
   * `/NeedAppearances` is never written and never read (contract §2.1).
   */
  function ensureAcroForm(pdf: MupdfPdf): { fields: MupdfObject; helv: MupdfObject } {
    const root = attempt(() => pdf.getTrailer().get("Root") as MupdfObject, null);
    if (!root) failField("invalid", "field-add: the document has no catalog");
    let acro = at(root, "AcroForm");
    const helvFont = () => attempt(() => pdf.addObject({
      Type: "Font", Subtype: "Type1", BaseFont: "Helvetica", Encoding: "WinAnsiEncoding",
    }), null as unknown as MupdfObject);
    if (!acro) {
      const helv = helvFont();
      acro = pdf.addObject({
        Fields: pdf.newArray(), DA: pdf.newString(FIELD_DA), DR: { Font: { Helv: helv } },
      });
      root.put("AcroForm", acro);
      return { fields: at(acro, "Fields") as MupdfObject, helv };
    }
    let fields = at(acro, "Fields");
    if (!fields) { fields = pdf.newArray(); acro.put("Fields", fields); }
    const fonts = (() => {
      let dr = at(acro, "DR");
      if (!dr) { dr = pdf.newDictionary(); acro.put("DR", dr); }
      let font = at(dr, "Font");
      if (!font) { font = pdf.newDictionary(); dr.put("Font", font); }
      return font;
    })();
    let helv = at(fonts, "Helv");
    if (!helv) { helv = helvFont(); fonts.put("Helv", helv); }
    if (!at(acro, "DA")) acro.put("DA", pdf.newString(FIELD_DA));
    return { fields, helv };
  }

  /**
   * The page's `/Annots` as a **direct** array (contract §7.6): an indirect object is
   * copied, so appending or removing here never reaches another page that shares it.
   */
  function directAnnots(pdf: MupdfPdf, pageObject: MupdfObject): MupdfObject {
    const copy = pdf.newArray();
    const current = at(pageObject, "Annots");
    if (current?.isArray()) {
      for (let index = 0; index < current.length; index++) copy.push(current.get(index) as MupdfObject);
    }
    pageObject.put("Annots", copy);
    return copy;
  }

  /** An object number, or -1 when the entry is not an indirect object. */
  function objectId(object: MupdfObject | null): number {
    return attempt(() => object!.asIndirect(), -1);
  }

  /** The current top-level `/AcroForm /Fields`, or null when the document has no form. */
  function existingFields(pdf: MupdfPdf): MupdfObject | null {
    const root = attempt(() => pdf.getTrailer().get("Root") as MupdfObject, null);
    const fields = at(at(root, "AcroForm"), "Fields");
    return fields?.isArray() ? fields : null;
  }

  /** The full name a field's `/T` chain spells, `""` when the chain has no `/T`. */
  function fullFieldName(dict: MupdfObject): string {
    const parts: string[] = [];
    for (let node: MupdfObject | null = dict, depth = 0; node && depth < MAX_FIELD_DEPTH; depth++) {
      const own = at(node, "T");
      if (own?.isString()) parts.unshift(own.asString());
      node = at(node, "Parent");
    }
    return parts.filter(part => part !== "").join(".");
  }

  /**
   * Every fully-qualified name in `/AcroForm /Fields`, bounded by `MAX_FIELD_NODES`
   * (over it: `too-many`) and `MAX_FIELD_DEPTH` (deeper nodes are not visited).
   */
  function fieldTreeNames(fields: MupdfObject, op: string): Set<string> {
    const names = new Set<string>();
    let visited = 0;
    const walk = (node: MupdfObject, prefix: string, depth: number) => {
      if (depth > MAX_FIELD_DEPTH) return;
      visited += 1;
      if (visited > MAX_FIELD_NODES) failField("too-many", `${op}: this form is too large to design fields in`);
      const own = at(node, "T");
      const full = own?.isString() && own.asString() ? (prefix ? `${prefix}.${own.asString()}` : own.asString()) : prefix;
      if (at(node, "T")?.isString() && full) names.add(full);
      const kids = at(node, "Kids");
      if (kids?.isArray()) {
        for (let index = 0; index < kids.length; index++) walk(kids.get(index) as MupdfObject, full, depth + 1);
      }
    };
    for (let index = 0; index < fields.length; index++) walk(fields.get(index) as MupdfObject, "", 1);
    return names;
  }

  /** A field name: 1..128 chars, no `/` hierarchy dots, no control characters, never trimmed. */
  function checkFieldName(name: unknown, op: string): string {
    if (typeof name !== "string" || name.length < 1 || name.length > MAX_FIELD_NAME_CHARS) {
      failField("invalid-name", `${op}: a field name is 1..${MAX_FIELD_NAME_CHARS} characters`);
    }
    if (name !== name.trim() || name.includes(".") || /[\u0000-\u001f\u007f]/.test(name)) {
      failField("invalid-name", `${op}: a field name holds no dots, control characters or outer spaces`);
    }
    return name;
  }

  /** U7: a `/TU` — 0..256 chars, no control characters, never trimmed (contract §2.1). `""`
   * is legal and means "delete the label". */
  function checkFieldLabel(label: unknown, op: string): string {
    if (typeof label !== "string" || label.length > MAX_FIELD_LABEL) {
      failField("invalid", `${op}: a label holds at most ${MAX_FIELD_LABEL} characters`);
    }
    if (/[\u0000-\u001f\u007f]/.test(label)) {
      failField("invalid", `${op}: a label holds no control characters`);
    }
    return label;
  }

  /** U7: `readOnly` and `required` are booleans, whatever the field kind. */
  function checkFieldFlag(value: unknown, op: string, what: string): boolean {
    if (typeof value !== "boolean") failField("invalid", `${op}: ${what} is true or false`);
    return value;
  }

  /** Set or clear one `/Ff` bit on the field dict; every other bit is left alone (§2.1).
   * §8.7: a key that was absent stays absent when the resulting bits are 0; one that was
   * there is written back even as 0. */
  /** P6 U2 §4.1: `/Ff` bit 19 through MuPDF's inherited flags; a combo only. */
  function comboEditable(widget: MupdfWidget): boolean {
    return (attempt(() => widget.getFieldFlags(), 0) & mupdf.PDFWidget.CH_FIELD_IS_EDIT) !== 0;
  }

  function setFieldFlag(dict: MupdfObject, bit: number, on: boolean): void {
    const flags = at(dict, "Ff");
    const present = flags?.isNumber() === true;
    const bits = present ? (flags as MupdfObject).asNumber() : 0;
    const next = on ? bits | bit : bits & ~bit;
    if (!present && next === 0) return;
    dict.put("Ff", next);
  }

  /** The dict a `/TU`, `/Ff` or `/DV` write belongs on: the widget's own field dict, or the
   * structural radio parent for a kid (§2.1). */
  /** `/I` may sit on the widget or on the parent field holding `/V`; drop it along the chain. */
  function dropChoiceIndices(dict: MupdfObject): void {
    for (let node: MupdfObject | null = dict, depth = 0; node && depth < 32; node = at(node, "Parent"), depth++) node.delete("I");
  }

  function fieldOwnDict(kind: FieldKind, dict: MupdfObject): MupdfObject {
    return (kind === "radio" ? at(dict, "Parent") : null) ?? dict;
  }

  /** Every kid's on-state export name of a radio parent, in `/Kids` order. */
  function kidOnNames(parent: MupdfObject): string[] {
    const kids = at(parent, "Kids");
    if (!kids?.isArray()) return [];
    const names: string[] = [];
    for (let index = 0; index < kids.length; index++) names.push(fieldOn(kids.get(index) as MupdfObject));
    return names;
  }

  /** §8.2: a radio kid carrying its own `/Ff`, `/TU`, `/DV` or `/V` shadows the group's own
   * properties, so U7's four properties are refused there (the parent they would write is
   * not the dict MuPDF reads back). */
  function radioKidsCarryOwn(parent: MupdfObject): boolean {
    const kids = at(parent, "Kids");
    if (!kids?.isArray()) return false;
    for (let index = 0; index < kids.length; index++) {
      const kid = kids.get(index) as MupdfObject;
      if (at(kid, "Ff") || at(kid, "TU") || at(kid, "DV") || at(kid, "V")) return true;
    }
    return false;
  }

  /**
   * A `/DV` validated against the field as this same request leaves it (§2.1): `null` deletes
   * the key, otherwise the value to write — a boolean for a checkbox, a string (a kid's export
   * name for a radio) for the rest. `""` deletes for every string kind.
   */
  function checkFieldDefault(
    kind: FieldKind, value: unknown,
    resulting: { maxLen: number; options: string[]; kids: string[]; edit?: boolean }, op: string,
  ): string | boolean | null {
    if (kind === "checkbox") {
      if (typeof value !== "boolean") failField("invalid", `${op}: a checkbox default is true or false`);
      return value;
    }
    if (typeof value !== "string") failField("invalid", `${op}: a ${kind} default is a string`);
    if (value === "") return null;
    if (kind === "text") {
      if (value.length > MAX_FIELD_TEXT_CHARS) {
        failField("invalid", `${op}: a default holds at most ${MAX_FIELD_TEXT_CHARS} characters`);
      }
      if (resulting.maxLen > 0 && value.length > resulting.maxLen) {
        failField("invalid", `${op}: the default is longer than maxLen ${resulting.maxLen}`);
      }
      return value;
    }
    if (kind === "combo") {
      if (resulting.edit) {
        if (value.length > MAX_FIELD_TEXT_CHARS) {
          failField("invalid", `${op}: a default holds at most ${MAX_FIELD_TEXT_CHARS} characters`);
        }
        return value;
      }
      if (!resulting.options.includes(value)) {
        failField("invalid", `${op}: the default is not one of this field's options`);
      }
      return value;
    }
    if (kind !== "radio") failField("invalid", `${op}: a ${kind} takes no default value`);
    if (!resulting.kids.includes(value)) {
      failField("invalid", `${op}: the default is not one of this group's kid names`);
    }
    return value;
  }

  /** `/DV` on the field dict: a name for buttons and radio kids, a literal string for text
   * and combo; `null` removes the key (contract §2.1). */
  function writeFieldDefault(pdf: MupdfPdf, dict: MupdfObject, kind: FieldKind, value: string | boolean | null): void {
    if (value === null) { dict.delete("DV"); return; }
    if (kind === "checkbox") dict.put("DV", value ? (fieldOn(dict) || FIELD_CHECK_ON) : "Off");
    else if (kind === "radio") dict.put("DV", String(value));
    else dict.put("DV", pdf.newString(String(value)));
  }

  /** Names taken document-wide; `except` drops that field's own current name (§2.3). */
  function checkNameFree(fields: MupdfObject, name: string, except: MupdfObject | null, op: string): void {
    const taken = fieldTreeNames(fields, op);
    if (except) taken.delete(fullFieldName(except));
    if (taken.has(name)) failField("duplicate-name", `${op}: a field named ${name} already exists`);
  }

  /** A field rect: positive area, on the page, and at least 4x4 pt (contract §2.1). */
  function checkFieldRect(box: number[], rect: unknown, op: string): Rect {
    let corners: number[];
    try {
      corners = finiteNumbers(rect, 4, "rect", op);
    } catch (err) {
      failField("invalid", errorMessage(err));
    }
    const [x0, y0, x1, y1] = corners;
    if (x1 - x0 < MIN_FIELD_RECT_PT || y1 - y0 < MIN_FIELD_RECT_PT) {
      failField("invalid", `${op}: a field is at least ${MIN_FIELD_RECT_PT}x${MIN_FIELD_RECT_PT} pt`);
    }
    if (!onPage(box, x0, y0) || !onPage(box, x1, y1)) {
      failField("invalid", `${op}: rect [${x0}, ${y0}, ${x1}, ${y1}] is off the page`);
    }
    return [x0, y0, x1, y1];
  }

  /** A non-empty option string, 1..256 chars (contract §2.1, combo). */
  function checkOptionList(options: unknown, op: string): string[] {
    if (!Array.isArray(options) || options.length < 1 || options.length > MAX_FIELD_OPTIONS) {
      failField("invalid", `${op}: options must be 1..${MAX_FIELD_OPTIONS} entries`);
    }
    const list: string[] = [];
    for (const option of options) {
      if (typeof option !== "string" || option.length < 1 || option.length > MAX_FIELD_OPTION_CHARS) {
        failField("invalid", `${op}: every option is 1..${MAX_FIELD_OPTION_CHARS} characters`);
      }
      if (list.includes(option)) failField("invalid", `${op}: options must be unique`);
      list.push(option);
    }
    return list;
  }

  /** 2..32 radio kids with unique, non-`Off` export names (contract §2.1). */
  function checkKids(box: number[], kids: unknown, op: string): { on: string; rect: Rect }[] {
    if (!Array.isArray(kids) || kids.length < 2 || kids.length > MAX_FIELD_KIDS) {
      failField("invalid", `${op}: a radio group takes 2..${MAX_FIELD_KIDS} kids`);
    }
    const seen = new Set<string>();
    return kids.map((kid, index) => {
      const on = (kid as { on?: unknown })?.on;
      if (typeof on !== "string" || on.length < 1 || on.length > MAX_FIELD_KID_NAME || on === "Off") {
        failField("invalid", `${op}: kid ${index} needs an export name of 1..${MAX_FIELD_KID_NAME} characters, never Off`);
      }
      if (seen.has(on)) failField("invalid", `${op}: kid export names must be unique`);
      seen.add(on);
      return { on, rect: checkFieldRect(box, (kid as { rect?: unknown }).rect, op) };
    });
  }

  /** Whether `parent` is a radio group this unit may change: top-level, `/FT /Btn`, bit 16, not bit 17, kids are unnamed widgets (§7.3). */
  function isStructuralRadioParent(parent: MupdfObject | null): boolean {
    if (!parent || at(parent, "Parent")) return false;
    const ft = at(parent, "FT");
    if (!ft?.isName() || ft.asName() !== "Btn") return false;
    const flags = at(parent, "Ff");
    const bits = flags?.isNumber() ? flags.asNumber() : 0;
    if (!(bits & mupdf.PDFWidget.BTN_FIELD_IS_RADIO) || (bits & mupdf.PDFWidget.BTN_FIELD_IS_PUSHBUTTON)) return false;
    const kids = at(parent, "Kids");
    if (!kids?.isArray() || kids.length < 1) return false;
    for (let index = 0; index < kids.length; index++) {
      const kid = kids.get(index) as MupdfObject;
      if (at(kid, "T")) return false;
      const subtype = at(kid, "Subtype");
      if (!subtype?.isName() || subtype.asName() !== "Widget") return false;
    }
    return true;
  }

  /** True when `node` is one of `/AcroForm /Fields` (by object number). */
  function isTopLevelField(fields: MupdfObject, node: MupdfObject): boolean {
    const id = attempt(() => node.asIndirect(), -1);
    if (id < 0) return false;
    for (let index = 0; index < fields.length; index++) {
      if (attempt(() => (fields.get(index) as MupdfObject).asIndirect(), -1) === id) return true;
    }
    return false;
  }

  /** The form fill on `widget` is refused (`xfa`, `scripted`, `signature`) before any COS write. */
  function checkFieldEditable(doc: MupdfDocument, widget: MupdfWidget, op: string): FieldKind {
    const info = detect(doc, false);
    if (info.xfa) failField("xfa", `${op}: this is an XFA form; fields cannot be authored`);
    const kind = fieldKind(widget);
    if (!kind) failField("invalid", `${op}: that widget is not a form field`);
    // P5 U3 §1.3: an empty signature field is authored like a text field; a signed one never.
    if (kind === "signature" && at(widget.getObject() as MupdfObject, "V")) failField("signature", `${op}: a signed field cannot be changed`);
    if (widgetScripted(widget.getObject() as MupdfObject, fieldContext(doc))) {
      failField("scripted", `${op}: this field runs a script this app does not execute`);
    }
    return kind;
  }

  /** One `/AP /N` state stream: a white box, a 1 pt black border and, when on, a mark. */
  function buttonStream(pdf: MupdfPdf, width: number, height: number, on: boolean): MupdfObject {
    const box = `q 1 g 0 0 ${width} ${height} re f 0 G 1 w .5 .5 ${width - 1} ${height - 1} re S`;
    const mark = on
      ? ` 3 3 m ${width - 3} ${height - 3} l S 3 ${height - 3} m ${width - 3} 3 l S`
      : "";
    return pdf.addStream(new TextEncoder().encode(`${box}${mark} Q\n`), {
      Type: "XObject", Subtype: "Form", BBox: [0, 0, width, height], Resources: {},
    });
  }

  /** `/AP /N << /Off … /<state> … >>` for a button, in the widget's own size. */
  function buttonAppearanceStates(pdf: MupdfPdf, state: string, rect: Rect): Record<string, MupdfObject> {
    const width = rect[2] - rect[0];
    const height = rect[3] - rect[1];
    const states: Record<string, MupdfObject> = { Off: buttonStream(pdf, width, height, false) };
    states[state] = buttonStream(pdf, width, height, true);
    return states;
  }

  /** The states a button's `/AP /N` already holds, `Off` first (contract §7.5). */
  function buttonStateKeys(dict: MupdfObject): string[] {
    const states = at(at(dict, "AP"), "N");
    if (!states?.isDictionary()) return [];
    const keys: string[] = [];
    states.forEach((_value, key) => { if (typeof key === "string") keys.push(key); });
    return keys.length ? keys : [];
  }

  /** A minimal text appearance, the fallback when `update()` leaves no `/AP /N` (probe `addTextAppearance`). */
  function textAppearance(pdf: MupdfPdf, rect: Rect, helv: MupdfObject, text: string): MupdfObject {
    const width = rect[2] - rect[0];
    const height = rect[3] - rect[1];
    const escaped = text.replace(/([\\()])/g, "\\$1");
    const contents = `q 1 g 0 0 ${width} ${height} re f 0 G 1 w .5 .5 ${width - 1} ${height - 1} re S`
      + ` /Tx BMC BT /Helv 12 Tf 0 g 3 ${Math.max(3, (height - 12) / 2)} Td (${escaped}) Tj ET EMC Q\n`;
    return pdf.addStream(new TextEncoder().encode(contents), {
      Type: "XObject", Subtype: "Form", BBox: [0, 0, width, height], Resources: { Font: { Helv: helv } },
    });
  }

  /**
   * The appearance step of a create or a change (contract §2.1, §7.5): for text and
   * combo drop `/AP` first and let `update()` redraw, with the empty-value fallback
   * stream when it produced none; for buttons rewrite every existing state key in the
   * new size and drop `/AP /D`.
   */
  function refreshAppearance(pdf: MupdfPdf, widget: MupdfWidget, kind: FieldKind, helv: MupdfObject): void {
    const dict = widget.getObject() as MupdfObject;
    if (kind === "checkbox" || kind === "radio") {
      const keys = buttonStateKeys(dict).filter(key => key !== "Off");
      const state = keys[0] ?? FIELD_CHECK_ON;
      dict.delete("AP");
      dict.put("AP", { N: buttonAppearanceStates(pdf, state, [...widget.getRect()] as Rect) });
      const as = at(dict, "AS");
      if (!as?.isName()) dict.put("AS", "Off");
      widget.update();
      return;
    }
    dict.delete("AP");
    widget.update();
    const ap = at(dict, "AP");
    if (!at(ap, "N")) {
      // §8.6: the fallback stream draws the current `/V` itself, since update() did not.
      const current = attempt(() => widget.getValue(), "");
      dict.put("AP", { N: textAppearance(pdf, [...widget.getRect()] as Rect, helv, typeof current === "string" ? current : "") });
    }
  }

  /**
   * The widget wrappers for `ids` on a **freshly loaded** page wrapper (probe
   * Ceiling 2: a wrapper loaded before the COS create cannot see the new widget).
   */
  function withWidgets<T>(doc: MupdfDocument, page: number, ids: number[], work: (found: Map<number, MupdfWidget>) => T): T {
    const target = doc.loadPage(page) as MupdfPage;
    try {
      const found = new Map<number, MupdfWidget>();
      for (const widget of target.getWidgets()) {
        const id = annotId(widget);
        if (ids.includes(id)) found.set(id, widget);
      }
      return work(found);
    } finally {
      target.destroy();
    }
  }

  /** The merged (single-widget) field dict of a create, before the page is reloaded. */
  function mergedFieldDict(
    pdf: MupdfPdf, pageObject: MupdfObject, kind: FieldAddKind, name: string, rect: Rect,
    extra: { ff?: number; options?: string[]; value?: string | MupdfObject; as?: string; states?: string; mk?: object; tu?: string; dv?: string | MupdfObject },
  ): MupdfObject {
    const dict: Record<string, unknown> = {
      Type: "Annot", Subtype: "Widget",
      FT: kind === "checkbox" ? "Btn" : kind === "combo" ? "Ch" : kind === "signature" ? "Sig" : "Tx",
      T: pdf.newString(name), Rect: rect, F: FIELD_F_PRINT, P: pageObject, DA: pdf.newString(FIELD_DA),
    };
    if (extra.mk) dict.MK = extra.mk;
    if (extra.ff) dict.Ff = extra.ff;
    if (extra.options) dict.Opt = extra.options.map(option => pdf.newString(option));
    if (extra.tu !== undefined) dict.TU = pdf.newString(extra.tu);
    if (extra.dv !== undefined) dict.DV = extra.dv;
    if (extra.value !== undefined) dict.V = extra.value;
    if (extra.as !== undefined) dict.AS = extra.as;
    if (extra.states) dict.AP = { N: buttonAppearanceStates(pdf, extra.states, rect) };
    return pdf.addObject(dict);
  }

  /** The widget's own object number on this page, the id `field-set` names. */
  function listAnnotations(doc: MupdfDocument, pageIndex: number, editable: boolean): AnnotItem[] {
    const page = doc.loadPage(pageIndex) as MupdfPage;
    try {
      const items: AnnotItem[] = [];
      for (const annot of page.getAnnotations()) {
        const type = annot.getType();
        if (type === "Widget" || type === "Popup") continue;
        const search = type === "Redact" && (attempt(() => (annot.getQuadPoints() ?? []).length > 0, false)
          || !!at(annot.getObject() as MupdfObject, "PDF4AllSearch"));
        items.push({
          id: annotId(annot),
          type,
          rect: annotRect(annot),
          contents: attempt(() => annot.getContents(), ""),
          subject: attempt(() => annot.getSubject(), ""),
          editable: editable && EDITABLE.has(type),
          ...(search ? { search: true } : {}),
        });
      }
      return items;
    } finally {
      page.destroy();
    }
  }

  type OutlineIterator = ReturnType<MupdfDocument["outlineIterator"]>;
/** A document known to be a PDF, the type the graft/field helpers need. */
type MupdfPdf = NonNullable<ReturnType<MupdfDocument["asPDF"]>>;

  /**
   * The whole outline in preorder, as the panel sees it. `loadOutline()` is the
   * only accessor that reports children (`OutlineIterator.item()` does not), and it
   * is also the guide `outlineCursor` walks with.
   */
  function listOutline(doc: MupdfDocument): OutlineEntry[] {
    const count = doc.countPages();
    const entries: OutlineEntry[] = [];
    const walk = (nodes: OutlineNode[], depth: number) => {
      for (const node of nodes) {
        // ponytail: resolveLink on a stale destination can report an index past the
        // end; a page that is not there is not a destination.
        const resolved = node.uri ? attempt(() => doc.resolveLink(node.uri as string), -1) : -1;
        const page = resolved >= 0 && resolved < count ? resolved : null;
        const entry: OutlineEntry = { id: entries.length, title: node.title ?? "", depth, page };
        if (page === null && node.uri) entry.uri = node.uri;
        entries.push(entry);
        if (node.down?.length) walk(node.down, depth + 1);
      }
    };
    walk(loadOutlineTree(doc), 0);
    return entries;
  }

  /** `loadOutline()` with its null/throw collapsed to an empty forest. */
  function loadOutlineTree(doc: MupdfDocument): OutlineNode[] {
    return attempt(() => (doc.loadOutline() ?? []) as OutlineNode[], [] as OutlineNode[]);
  }

  /**
   * An iterator positioned on preorder entry `index`. The cursor cannot be probed
   * for children (`down()` on a leaf moves it to empty), so the walk follows the
   * loaded tree and descends only where it has children.
   */
  function outlineCursor(doc: MupdfDocument, index: unknown, op: string): OutlineIterator {
    if (!Number.isInteger(index) || (index as number) < 0) {
      throw new Error(`${op}: id must be a non-negative integer`);
    }
    const wanted = index as number;
    const it = doc.outlineIterator();
    let seen = 0;
    let found = false;
    const walk = (nodes: OutlineNode[]) => {
      for (let position = 0; position < nodes.length; position++) {
        if (seen === wanted) {
          found = true;
          return;
        }
        seen += 1;
        const kids = nodes[position].down ?? [];
        if (kids.length && it.down() === 0) {
          walk(kids);
          if (found) return;
          it.up();
        }
        if (position < nodes.length - 1 && it.next() !== 0) return;
      }
    };
    walk(loadOutlineTree(doc));
    if (!found) {
      it.destroy();
      throw new Error(`${op}: no bookmark ${wanted}`);
    }
    return it;
  }

  /** A bookmark title: user text, non-empty after trim, bounded like a note. */
  function checkTitle(title: unknown, op: string): string {
    if (typeof title !== "string" || title.length > MAX_BOOKMARK_CHARS) {
      throw new Error(`${op}: title must be a string of at most ${MAX_BOOKMARK_CHARS} characters`);
    }
    if (!title.trim()) throw new Error(`${op}: a bookmark needs a title`);
    return title;
  }

  /**
   * The destination shape `formatLinkURI` reads. The module's default export has no
   * `LinkDestination` class (only the namespace import does), and the function reads
   * the fields structurally, so the object is passed as a plain literal.
   */
  function fitDest(page: number) {
    return { type: "Fit" as const, chapter: 0, page, x: 0, y: 0, width: 0, height: 0, zoom: 0 };
  }

  /** Appends one top-level bookmark; `insert` places before the cursor, so walk to the end. */
  function appendBookmark(doc: MupdfDocument, title: string, page: number) {
    const it = doc.outlineIterator();
    try {
      const roots = loadOutlineTree(doc).length;
      for (let step = 0; step < roots; step++) {
        if (it.next() !== 0) break;
      }
      it.insert({ title, uri: doc.formatLinkURI(fitDest(page)), open: true });
    } finally {
      it.destroy();
    }
  }

  /** The URI's scheme with its colon, lowercased; "" when the URI has none. */
  function schemeOf(uri: string): string {
    const match = /^([A-Za-z][A-Za-z0-9+.-]*):/.exec(uri);
    return match ? `${match[1].toLowerCase()}:` : "";
  }

  /**
   * Every link on a page, in `getLinks()` order. A link resolving to a page is
   * `'page'`; a URI link is `'uri'` only when its scheme is on the allowlist and
   * `'blocked'` otherwise (the raw URI still travels, so the panel can name the scheme).
   */
  function listLinks(doc: MupdfDocument, pageIndex: number): LinkEntry[] {
    const page = doc.loadPage(pageIndex) as MupdfPage;
    try {
      const count = doc.countPages();
      return linkPairs(page).map(({ link, annotId: linkAnnotId }, index) => {
        const rect = attempt(() => [...link.getBounds()] as number[], [] as number[]);
        const resolved = attempt(() => doc.resolveLink(link), -1);
        if (resolved >= 0 && resolved < count) {
          return { id: index, annotId: linkAnnotId, rect, kind: "page" as const, page: resolved };
        }
        const uri = attempt(() => link.getURI(), "");
        const scheme = schemeOf(uri);
        return (LINK_SCHEMES as readonly string[]).includes(scheme)
          ? { id: index, annotId: linkAnnotId, rect, kind: "uri" as const, uri }
          : { id: index, annotId: linkAnnotId, rect, kind: "blocked" as const, uri };
      });
    } finally {
      page.destroy();
    }
  }

  /**
   * P6 U4 §9.1: every `getLinks()` item paired with its `/Annots` entry. The walk keeps
   * the entries mupdf lists (`/Subtype /Link`, a `/Rect`, one of `/Dest`/`/A`/`/AA`, not a
   * JavaScript `/A`) and pairs in order by rect (raw `/Rect` through the page transform,
   * ±0.01); an unpaired item gets `annotId` 0 and the cursor stays. Never throws: any
   * failure in the walk gives every item `annotId` 0. Ceiling: a `/Link` mupdf drops for
   * another reason that has exactly the next link's rect can take that link's id.
   */
  function linkPairs(page: MupdfPage): { link: MupdfLink; entry: MupdfObject | null; annotId: number }[] {
    const links = page.getLinks();
    try {
      const transform = page.getTransform();
      const annots = at(page.getObject() as MupdfObject, "Annots");
      const kept: { entry: MupdfObject; rect: number[] }[] = [];
      const length = annots?.isArray() ? annots.length : 0;
      for (let index = 0; index < length; index++) {
        const entry = (annots as MupdfObject).get(index) as MupdfObject;
        if (at(entry, "Subtype")?.asName() !== "Link") continue;
        if (!at(entry, "Dest") && !at(entry, "A") && !at(entry, "AA")) continue;
        if (at(at(entry, "A"), "S")?.asName() === "JavaScript") continue;
        const raw = at(entry, "Rect");
        if (!raw?.isArray() || raw.length < 4) continue;
        const [a, b, c, d] = [0, 1, 2, 3].map(k => (raw.get(k) as MupdfObject).asNumber());
        if (![a, b, c, d].every(Number.isFinite)) continue;
        const norm: Rect = [Math.min(a, c), Math.min(b, d), Math.max(a, c), Math.max(b, d)];
        kept.push({ entry, rect: [...mupdf.Rect.transform(norm, transform)] });
      }
      let cursor = 0;
      return links.map(link => {
        const bounds = [...link.getBounds()];
        for (let index = cursor; index < kept.length; index++) {
          if (kept[index].rect.every((value, k) => Math.abs(value - bounds[k]) <= 0.01)) {
            cursor = index + 1;
            const entry = kept[index].entry;
            return { link, entry, annotId: entry.isIndirect() ? entry.asIndirect() : 0 };
          }
        }
        return { link, entry: null, annotId: 0 };
      });
    } catch {
      return links.map(link => ({ link, entry: null, annotId: 0 }));
    }
  }

  /**
   * P6 U4 §9.11: object numbers of `/OBJR` entries in a depth-first walk of
   * `/StructTreeRoot /K` (descending only through `/K`, first appearance wins).
   * Null when there is no tree, it is past 50 000 nodes or 64 dict levels (a `/K` array
   * is not a level), or it is unreadable.
   */
  function structObjectOrder(doc: MupdfDocument): Map<number, number> | null {
    return attempt(() => {
      const pdf = doc.asPDF();
      const kids = at(at(at(pdf?.getTrailer() ?? null, "Root"), "StructTreeRoot"), "K");
      if (!kids) return null;
      const order = new Map<number, number>();
      const visited = new Set<number>();
      let nodes = 0;
      const visit = (node: MupdfObject, depth: number): boolean => {
        if (++nodes > 50_000 || depth > 64) return false;
        if (node.isIndirect()) {
          const number = node.asIndirect();
          if (visited.has(number)) return true;
          visited.add(number);
        }
        if (node.isArray()) {
          for (let index = 0; index < node.length; index++) {
            if (!visit(node.get(index) as MupdfObject, depth)) return false;
          }
          return true;
        }
        if (!node.isDictionary()) return true;
        const obj = at(node, "Obj");
        if (obj?.isIndirect()) {
          const number = obj.asIndirect();
          if (!order.has(number)) order.set(number, order.size);
          return true;
        }
        const next = at(node, "K");
        return next ? visit(next, depth + 1) : true;
      };
      return visit(kids, 0) ? order : null;
    }, null);
  }

  /** P6 U4 §2.1: the page's `/Tabs` when it is one of R/C/S/A/W. */
  function pageTabs(doc: MupdfDocument, pageIndex: number): TabsName | null {
    const name = attempt(() => at(doc.asPDF()?.findPage(pageIndex) ?? null, "Tabs")?.asName() ?? null, null);
    return name === "R" || name === "C" || name === "S" || name === "A" || name === "W" ? name : null;
  }

  /** A link rect inside the page box, normalized; a degenerate or off-page rect is refused. */
  function checkLinkRect(box: number[], rect: unknown, op: string): Rect {
    const [x0, y0, x1, y1] = finiteNumbers(rect, 4, "rect", op);
    const left = Math.min(x0, x1);
    const right = Math.max(x0, x1);
    const top = Math.min(y0, y1);
    const bottom = Math.max(y0, y1);
    if (right - left <= 0 || bottom - top <= 0) throw new Error(`${op}: rect must have a positive area`);
    if (!onPage(box, left, top) || !onPage(box, right, bottom)) {
      throw new Error(`${op}: rect [${left}, ${top}, ${right}, ${bottom}] is off the page`);
    }
    return [left, top, right, bottom];
  }

  /**
   * The links that name the page about to be deleted. Read before `deletePage`:
   * mupdf repoints a destination at whichever page takes the index over, so
   * afterwards the reference no longer names the removed page.
   */
  function linksToPage(doc: MupdfDocument, removed: number): { page: number; index: number }[] {
    const doomed: { page: number; index: number }[] = [];
    for (let pageIndex = 0; pageIndex < doc.countPages(); pageIndex++) {
      if (pageIndex === removed) continue;
      const page = doc.loadPage(pageIndex) as MupdfPage;
      try {
        page.getLinks().forEach((link, index) => {
          if (attempt(() => doc.resolveLink(link), -1) === removed) doomed.push({ page: pageIndex, index });
        });
      } finally {
        page.destroy();
      }
    }
    // Descending index within a page: each delete shifts the links after it.
    return doomed.sort((a, b) => a.page - b.page || b.index - a.index);
  }

  /** Drops the destination (title and subtree stay) of the named preorder entries. */
  function clearOutlineDests(doc: MupdfDocument, entries: number[]) {
    for (const index of [...entries].sort((a, b) => b - a)) {
      let it: OutlineIterator | null = null;
      try {
        it = outlineCursor(doc, index, "page-delete");
        const item = it.item();
        if (item) it.update({ title: item.title ?? "", uri: undefined, open: item.open });
      } catch {
        // A hostile or changed outline must not stop the delete; the walk is the guard.
      } finally {
        it?.destroy();
      }
    }
  }

  /**
   * Removes every top-level `/AcroForm /Fields` entry left without a widget anywhere
   * in the file: deleting a page whose widget was a field's only one leaves exactly
   * that phantom behind. A field whose widget survives on another page is kept.
   */
  function prunePhantomFields(doc: MupdfDocument) {
    const pdf = doc.asPDF();
    if (!pdf) return;
    let root: MupdfObject | null = null;
    try {
      root = at(pdf.getTrailer() as MupdfObject, "Root");
    } catch {
      return;
    }
    const fields = at(at(root, "AcroForm"), "Fields");
    if (!fields?.isArray() || fields.length === 0) return;
    const live = new Set<number>();
    for (let pageIndex = 0; pageIndex < doc.countPages(); pageIndex++) {
      const page = doc.loadPage(pageIndex) as MupdfPage;
      try {
        for (const widget of page.getWidgets()) {
          const id = attempt(() => widget.getObject().asIndirect(), -1);
          if (id > 0) live.add(id);
        }
      } finally {
        page.destroy();
      }
    }
    const hasWidget = (node: MupdfObject | null): boolean => {
      if (!node || node.isNull()) return false;
      const id = attempt(() => node.asIndirect(), -1);
      if (id > 0 && live.has(id)) return true;
      const kids = at(node, "Kids");
      if (!kids?.isArray()) return false;
      for (let index = 0; index < kids.length; index++) {
        if (hasWidget(kids.get(index) as MupdfObject)) return true;
      }
      return false;
    };
    for (let index = fields.length - 1; index >= 0; index--) {
      if (!hasWidget(fields.get(index) as MupdfObject)) fields.delete(index);
    }
  }

  /** The `at` index a page insert takes: anywhere in `0..pageCount` (pageCount appends). */
  function checkAt(doc: MupdfDocument, at: unknown, op: string): number {
    const count = doc.countPages();
    if (!Number.isInteger(at) || (at as number) < 0 || (at as number) > count) {
      throw new Error(`${op}: at must be an integer in 0..${count}`);
    }
    return at as number;
  }

  /** A bounded list of 0-based page indices in `0..count-1`; order and duplicates are kept. */
  function checkPages(pages: unknown, count: number, op: string): number[] {
    if (!Array.isArray(pages) || pages.length < 1 || pages.length > MAX_GRAFT_PAGES) {
      failReason("invalid", `${op}: pages must be 1..${MAX_GRAFT_PAGES} page numbers`);
    }
    return pages.map((page, index) => {
      if (!Number.isInteger(page) || (page as number) < 0 || (page as number) >= count) {
        failReason("invalid", `${op}: page ${index} must be an integer in 0..${Math.max(0, count - 1)}`);
      }
      return page as number;
    });
  }

  /**
   * One page with its annotations and form fields, all inside the caller's single
   * journal operation. `graftPage` brings the box, rotation, resources and contents
   * across but **drops `/Annots`** (with `/AA`, `/Dur` and `/StructParents`), so the
   * annotations are grafted separately through the same graft map: objects shared
   * between annots (a radio group's `/Parent`) are then copied once, which is what
   * keeps one field per group instead of one per widget.
   */
  function graftPages(pdf: MupdfPdf, to: number, source: MupdfPdf, pages: number[]): void {
    const map = pdf.newGraftMap();
    for (let index = 0; index < pages.length; index++) {
      map.graftPage(to + index, source, pages[index]);
    }
    const grafted: MupdfObject[] = [];
    for (let index = 0; index < pages.length; index++) {
      const sourcePage = source.findPage(pages[index]);
      const annots = at(sourcePage as unknown as MupdfObject, "Annots");
      if (!annots?.isArray() || annots.length === 0) continue;
      const copy = pdf.newArray();
      for (let position = 0; position < annots.length; position++) {
        const annot = map.graftObject(annots.get(position) as MupdfObject);
        copy.push(annot);
        grafted.push(annot);
      }
      pdf.findPage(to + index).put("Annots", copy);
    }
    registerGraftedFields(pdf, grafted);
  }

  /**
   * A grafted widget only lists on its page once its **top-level** field is in the
   * target's `/AcroForm /Fields` (mupdf loads widgets through the field tree, so a
   * widget whose field is missing is invisible to `getWidgets`, pypdf and Preview).
   * A `/T` that collides with a name already there is renamed `<name> (2)`, `(3)`, …
   * in the same operation, so no two top-level fields share a name.
   */
  function registerGraftedFields(pdf: MupdfPdf, grafted: MupdfObject[]): void {
    const tops = new Map<number, MupdfObject>();
    for (const annot of grafted) {
      const subtype = at(annot, "Subtype");
      if (!subtype?.isName() || subtype.asName() !== "Widget") continue;
      let node: MupdfObject = annot;
      // Eight levels is the same field-tree bound the fill path uses.
      for (let depth = 0; depth < 8; depth++) {
        const parent = at(node, "Parent");
        if (!parent) break;
        node = parent;
      }
      const id = attempt(() => node.asIndirect(), -1);
      if (id > 0) tops.set(id, node);
    }
    if (!tops.size) return;
    let root: MupdfObject | null = null;
    try {
      root = at(pdf.getTrailer() as MupdfObject, "Root");
    } catch {
      return;
    }
    if (!root) return;
    let acro = at(root, "AcroForm");
    if (!acro) {
      acro = pdf.newDictionary();
      root.put("AcroForm", acro);
    }
    let fields = at(acro, "Fields");
    if (!fields) {
      const created = pdf.newArray();
      acro.put("Fields", created);
      fields = created;
    }
    const taken = new Set<string>();
    for (let index = 0; index < fields.length; index++) {
      const name = attempt(() => (fields.get(index) as MupdfObject).get("T").asString(), "");
      if (name) taken.add(name);
    }
    for (const node of tops.values()) {
      const name = attempt(() => node.get("T").asString(), "");
      if (name) {
        let unique = name;
        if (taken.has(unique)) {
          // Bounded: the first free suffix is at most taken.size + 1.
          for (let suffix = 2; suffix <= taken.size + 2; suffix++) {
            if (!taken.has(`${name} (${suffix})`)) {
              unique = `${name} (${suffix})`;
              break;
            }
          }
        }
        if (unique !== name) node.put("T", pdf.newString(unique));
        taken.add(unique);
      }
      fields.push(node);
    }
  }

  /**
   * The top-level `/AcroForm /Fields` names, in order. The insert reply carries them
   * so the renderer can log what a merge did to a form (`P2_EDIT.assemble.fieldNames`)
   * without a second read op that would have to walk every page.
   */
  function fieldNames(doc: MupdfDocument): string[] {
    const pdf = doc.asPDF();
    if (!pdf) return [];
    let root: MupdfObject | null = null;
    try {
      root = at(pdf.getTrailer() as MupdfObject, "Root");
    } catch {
      return [];
    }
    const fields = at(at(root, "AcroForm"), "Fields");
    if (!fields?.isArray()) return [];
    const names: string[] = [];
    for (let index = 0; index < fields.length; index++) {
      const name = attempt(() => (fields.get(index) as MupdfObject).get("T").asString(), "");
      names.push(name);
    }
    return names;
  }

  /**
   * One page per image, in the order given. `addPage` only builds the page object
   * (measured: `countPages()` stays 0 until `insertPage` puts it in the page tree),
   * so each page is inserted at the end explicitly.
   */
  function pdfFromImages(images: { name: string; bytes: ArrayBuffer }[]) {
    const out = new mupdf.PDFDocument();
    try {
      const outPdf = out.asPDF();
      if (!outPdf) failReason("invalid", "pdf-from-images: the new document is not a PDF");
      for (const image of images) {
        let decoded: MupdfImage;
        try {
          decoded = new mupdf.Image(new Uint8Array(image.bytes));
        } catch (err) {
          failReason("invalid", `pdf-from-images: ${image.name} is not a PNG or JPEG mupdf can read (${errorMessage(err)})`);
        }
        try {
          const pixelsWide = decoded.getWidth();
          const pixelsHigh = decoded.getHeight();
          if (!(pixelsWide > 0) || !(pixelsHigh > 0)) {
            failReason("invalid", `pdf-from-images: ${image.name} has no pixels`);
          }
          // Points at the image's own resolution. mupdf reports 96 for an image with
          // no resolution metadata (measured on PNG and JPEG), which is indistinguishable
          // from a real 96, so it is read as "none" and the page keeps one point per pixel.
          const declared = decoded.getXResolution();
          const dpi = declared > 0 && declared !== 96 ? declared : 72;
          const naturalWidth = (pixelsWide * 72) / dpi;
          const naturalHeight = (pixelsHigh * 72) / dpi;
          // Past 14400 pt the page shrinks; the image keeps its pixels.
          const shrink = Math.min(1, MAX_IMAGE_PAGE_POINTS / naturalWidth, MAX_IMAGE_PAGE_POINTS / naturalHeight);
          const width = Math.round(naturalWidth * shrink * 100) / 100;
          const height = Math.round(naturalHeight * shrink * 100) / 100;
          const ref = outPdf.addImage(decoded);
          const resources = outPdf.newDictionary();
          const xobjects = outPdf.newDictionary();
          xobjects.put("Im0", ref);
          resources.put("XObject", xobjects);
          const page = outPdf.addPage([0, 0, width, height], 0, resources, `q ${width} 0 0 ${height} 0 0 cm /Im0 Do Q`);
          outPdf.insertPage(outPdf.countPages(), page);
        } finally {
          decoded.destroy();
        }
      }
      if (outPdf.countPages() !== images.length) {
        failReason("invalid", `pdf-from-images: built ${outPdf.countPages()} pages for ${images.length} images`);
      }
      const buffer = outPdf.saveToBuffer(SAVE_OPTIONS);
      let bytes: ArrayBuffer;
      try {
        const view = buffer.asUint8Array();
        if (view.byteLength > MAX_SAVE_BYTES) {
          failReason("too-large", `pdf-from-images: the document would exceed the ${MAX_SAVE_BYTES} byte save ceiling`);
        }
        bytes = copyBytes(view);
      } finally {
        buffer.destroy();
      }
      return { bytes, pageCount: images.length };
    } finally {
      closeDocument(out);
    }
  }

  /** The stamp template with the per-page placeholders filled in. */
  function stampTextFor(template: string, page: MupdfPage, index: number, count: number, bates = ""): string {
    const label = attempt(() => page.getLabel(), "") || String(index + 1);
    // {bates} first: its digits cannot form another token, a label could (§9.5).
    return template
      .replaceAll("{bates}", bates)
      .replaceAll("{page}", String(index + 1))
      .replaceAll("{pages}", String(count))
      .replaceAll("{label}", label);
  }

  /**
   * The page's own box, unrotated: `/CropBox` if it has one, else `/MediaBox`. mupdf's
   * `getBounds(box)` returns the *displayed* box (measured: a 400×200 page with
   * `/Rotate 270` reports `[0 0 200 400]` even for `"MediaBox"`), so the raw array is
   * read straight from the page dictionary.
   */
  function rawPageBox(pdf: MupdfPdf, pageIndex: number): Rect {
    const read = (value: MupdfObject | null): Rect | null => {
      if (!value?.isArray() || value.length !== 4) return null;
      const numbers = [0, 1, 2, 3].map(index => attempt(() => (value.get(index) as MupdfObject).asNumber(), NaN));
      if (numbers.some(number => !Number.isFinite(number))) return null;
      return [
        Math.min(numbers[0], numbers[2]), Math.min(numbers[1], numbers[3]),
        Math.max(numbers[0], numbers[2]), Math.max(numbers[1], numbers[3]),
      ];
    };
    const page = pdf.findPage(pageIndex);
    return read(at(page, "CropBox")) ?? read(at(page, "MediaBox")) ?? [0, 0, 612, 792];
  }

  /**
   * Bugfix 1 D1: a Base-14 font resource for `stampPage` and `text-add`. mupdf caches simple-font
   * resources per document by font digest; an undo drops the object the cache still names, so
   * the next `addSimpleFont` returns a reference to a freed object. Then the same dictionary
   * `addSimpleFont` writes in the good case is added as a fresh object.
   */
  function addBase14Font(pdf: MupdfPdf, name: string): MupdfObject {
    const font = new mupdf.Font(name);
    let object: MupdfObject;
    try { object = pdf.addSimpleFont(font, "Latin"); } finally { font.destroy(); }
    const type = at(object, "Type");
    if (type?.isName() && type.asName() === "Font") return object;
    const dict = pdf.newDictionary();
    dict.put("Type", pdf.newName("Font"));
    dict.put("Subtype", pdf.newName("Type1"));
    dict.put("BaseFont", pdf.newName(name));
    dict.put("Encoding", pdf.newName("WinAnsiEncoding"));
    return pdf.addObject(dict);
  }

  /**
   * One page's stamp, appended as its own content stream with a `q` stream first and
   * this last, so nothing the page draws can leak into it (and an unbalanced `q` in
   * the page cannot swallow it). The font is referenced from the page's own
   * `/Resources /Font` under `/PDF4AllF` (Base-14 Helvetica, never embedded).
   */
  function stampPage(pdf: MupdfPdf, pageIndex: number, kind: StampKind, template: string,
    options: { bates?: string; align?: string } = {}) {
    const page = pdf.loadPage(pageIndex) as MupdfPage;
    try {
      const pageObject = pdf.findPage(pageIndex);
      const box = rawPageBox(pdf, pageIndex);
      const [llx, lly] = [box[0], box[1]];
      const width = box[2] - box[0];
      const height = box[3] - box[1];
      const visual = [...page.getBounds()] as Rect;
      const visualHeight = visual[3] - visual[1];
      const visualWidth = visual[2] - visual[0];
      const rotate = pageRotation(pdf, pageIndex);
      const expanded = stampTextFor(template, page, pageIndex, pdf.countPages(), options.bates);
      const text = winAnsiBytes(expanded);

      // Where a visual point lands in the unrotated page space, measured for all four
      // rotations (see p2-decorate.md): R=0 (vx, H-vy), 90 (vy, vx), 180 (W-vx, vy),
      // 270 (W-vy, H-vx). The text matrix turns the glyphs upright on the display.
      const place = (vx: number, vy: number): { x: number; y: number; matrix: number[] } => {
        if (rotate === 90) return { x: llx + vy, y: lly + vx, matrix: [0, 1, -1, 0] };
        if (rotate === 180) return { x: llx + width - vx, y: lly + vy, matrix: [-1, 0, 0, -1] };
        if (rotate === 270) return { x: llx + width - vy, y: lly + height - vx, matrix: [0, -1, 1, 0] };
        return { x: llx + vx, y: lly + height - vy, matrix: [1, 0, 0, 1] };
      };

      let body: string;
      if (kind === "watermark") {
        const size = 48;
        // The stated approximation: half an em per character, no font measurement.
        const textWidth = expanded.length * size * 0.5;
        const c = Math.SQRT1_2;
        const centreX = llx + width / 2;
        const centreY = lly + height / 2;
        const tx = centreX - (c * (textWidth / 2) - c * (size / 2));
        const ty = centreY - (c * (textWidth / 2) + c * (size / 2));
        body = `0.5 g BT /PDF4AllF ${size} Tf ${c} ${c} ${-c} ${c} ${round2(tx)} ${round2(ty)} Tm `;
      } else {
        const size = 10;
        let vx = 36;
        if (options.align === "right") {
          // Measured Helvetica advances (§9.6), so the run ends at the right margin.
          const font = new mupdf.Font("Helvetica");
          try {
            let advance = 0;
            for (const character of expanded) advance += font.advanceGlyph(font.encodeCharacter(character.codePointAt(0) ?? 0), 0);
            vx = Math.max(36, visualWidth - 36 - advance * size);
          } finally {
            font.destroy();
          }
        }
        const vy = kind === "header" ? 36 : visualHeight - 36;
        const { x, y, matrix } = place(vx, vy);
        body = `0 g BT /PDF4AllF ${size} Tf ${matrix[0]} ${matrix[1]} ${matrix[2]} ${matrix[3]} ${round2(x)} ${round2(y)} Tm `;
      }

      const head = new TextEncoder().encode("q\n");
      const tail = concatBytes(
        new TextEncoder().encode(`Q\n/PDF4All <</Kind /${kind === "watermark" ? "Watermark" : kind === "header" ? "Header" : "Footer"}>> BDC `),
        new TextEncoder().encode(body),
        new Uint8Array([0x28]),
        text,
        new TextEncoder().encode(") Tj ET EMC\n"),
      );
      // The font reference, added once per page resources dictionary.
      let resources = at(pageObject, "Resources");
      if (!resources) {
        resources = pdf.newDictionary();
        pageObject.put("Resources", resources);
      }
      let fonts = at(resources, "Font");
      if (!fonts) {
        fonts = pdf.newDictionary();
        resources.put("Font", fonts);
      }
      if (!at(fonts, "PDF4AllF")) fonts.put("PDF4AllF", addBase14Font(pdf, "Helvetica"));

      const contents = at(pageObject, "Contents");
      const array = pdf.newArray();
      array.push(pdf.addStream(head, {}));
      if (contents?.isArray()) {
        for (let index = 0; index < contents.length; index++) array.push(contents.get(index));
      } else if (contents) {
        array.push(contents);
      }
      array.push(pdf.addStream(tail, {}));
      pageObject.put("Contents", array);
    } finally {
      page.destroy();
    }
  }

  /**
   * A stamp action's fields, checked before any document is touched (contract-batch.md §9.5):
   * kind, text, align, batesPad, batesStart and `bates-start-required`. The upper bound
   * batesStart + pages - 1 needs the page count: `checkBatesRange`.
   */
  function checkStampAction(action: { kind?: unknown; text?: unknown; align?: unknown; batesStart?: unknown; batesPad?: unknown }) {
    const kind = action.kind;
    if (typeof kind !== "string" || !(STAMP_KINDS as readonly string[]).includes(kind)) {
      throw new Error(`stamp: kind must be one of ${STAMP_KINDS.join(", ")}`);
    }
    const text = checkStampText(action.text, "stamp", MAX_STAMP_CHARS);
    // Bates and alignment (contract-bates.md §9.2-9.4): validated whenever present, before mutate.
    const { align, batesStart, batesPad } = action;
    if (align !== undefined && align !== "left" && align !== "right") {
      failReason("invalid", `stamp: invalid-align: align must be left or right, not ${JSON.stringify(align)}`);
    }
    if (batesPad !== undefined && (!Number.isInteger(batesPad) || (batesPad as number) < 1 || (batesPad as number) > MAX_BATES_PAD)) {
      failReason("invalid", `stamp: invalid-bates: batesPad must be an integer in 1..${MAX_BATES_PAD}`);
    }
    if (batesStart !== undefined && (!Number.isInteger(batesStart) || (batesStart as number) < 0 || (batesStart as number) > MAX_BATES)) {
      failReason("invalid", `stamp: invalid-bates: batesStart must be an integer in 0..${MAX_BATES}`);
    }
    if (text.includes("{bates}") && batesStart === undefined) {
      failReason("invalid", "stamp: bates-start-required: the text has {bates} but no batesStart");
    }
    return {
      kind: kind as StampKind, text,
      align: align as "left" | "right" | undefined, batesStart: batesStart as number | undefined, batesPad: batesPad as number | undefined,
    };
  }

  function checkBatesRange(batesStart: number | undefined, count: number) {
    if (batesStart !== undefined && batesStart + count - 1 > MAX_BATES) {
      failReason("invalid", `stamp: invalid-bates: batesStart must be an integer with batesStart..batesStart+${count - 1} in 0..${MAX_BATES}`);
    }
  }

  /**
   * The one stamp loop (contract-batch.md §9.5): the named pages in order, `{bates}` counting
   * from batesStart. Used inside stampPages's mutate and on batch-file's transient document.
   */
  function stampLoop(pdf: MupdfPdf, pages: number[], kind: StampKind, text: string,
    options: { batesStart?: number; batesPad?: number; align?: string }): { from: number; to: number } | undefined {
    const bates = text.includes("{bates}");
    const pad = options.batesPad ?? 6;
    for (const [index, pageIndex] of pages.entries()) {
      stampPage(pdf, pageIndex, kind, text, {
        bates: bates ? String((options.batesStart as number) + index).padStart(pad, "0") : undefined,
        align: kind === "watermark" ? undefined : options.align,
      });
    }
    return bates ? { from: options.batesStart as number, to: (options.batesStart as number) + pages.length - 1 } : undefined;
  }

  /** The whole stamp flow: validate everything, then one operation over the named pages. */
  function stampPages(session: Session, doc: MupdfDocument, request: StampRequest): DispatchResult {
    const { kind, text, align, batesStart, batesPad } = checkStampAction(request);
    const pdf = doc.asPDF();
    if (!pdf) throw new Error("stamp: the open document is not a PDF");
    const count = doc.countPages();
    // Duplicates would stamp the same page twice; sorting keeps save and render ordered.
    const pages = request.pages === undefined
      ? Array.from({ length: count }, (_, index) => index)
      : [...new Set(checkPages(request.pages, count, "stamp"))].sort((a, b) => a - b);
    checkBatesRange(batesStart, pages.length);
    const label = kind === "watermark" ? "add watermark" : kind === "header" ? "add header" : "add footer";
    let range: { from: number; to: number } | undefined;
    mutate(session, doc, label, () => {
      range = stampLoop(pdf, pages, kind, text, { batesStart, batesPad, align });
    });
    return { reply: { id: request.id, ok: true, ...(range ? { bates: range } : {}), ...editState(session) } };
  }

  /**
   * batch-file (contract-batch.md D2, §9.1-9.7): a transient open of one file's bytes, the stamp
   * loop, a constant-options save, and the document destroyed in `finally`. Never requireMutable,
   * never session.doc, never a journal. Every per-file refusal is a reply, not a throw.
   */
  function batchFile(request: BatchFileRequest): DispatchResult {
    const refuse = (reason: string, error: string, detail?: string): DispatchResult =>
      ({ reply: { id: request.id, ok: false, reason, ...(detail ? { detail } : {}), error } });
    const bytes = request.bytes;
    request.bytes = null;
    const action = (request.action ?? {}) as BatchStampAction;
    if (!(bytes instanceof ArrayBuffer)) return refuse("not-pdf", "batch-file: no bytes");
    if (bytes.byteLength > MAX_PDF_BYTES) return refuse("too-large", `batch-file: over the ${MAX_PDF_BYTES} byte ceiling`);
    let checked: ReturnType<typeof checkStampAction>;
    try {
      checked = checkStampAction(action);
    } catch (err) {
      const message = errorMessage(err);
      return refuse("stamp-error", message, /^stamp: ([a-z-]+):/.exec(message)?.[1] ?? "text");
    }
    if (bytes.byteLength < 5) return refuse("not-pdf", "batch-file: too few bytes for a PDF");
    let source: MupdfDocument;
    try {
      source = mupdf.Document.openDocument(bytes, "application/pdf");
    } catch (err) {
      return refuse("not-pdf", `batch-file: the file could not be read (${errorMessage(err)})`);
    }
    try {
      const pdf = source.asPDF();
      if (!pdf) return refuse("not-pdf", "batch-file: the file is not a PDF");
      pdf.disableJS();
      let info: ReturnType<typeof detect>;
      let count: number;
      try {
        info = detect(source);
        count = source.countPages();
      } catch (err) {
        return refuse("not-pdf", `batch-file: the file could not be read (${errorMessage(err)})`);
      }
      if (info.needsPassword || info.encrypted) return refuse("encrypted", "batch-file: encrypted files are skipped");
      if (info.signatures > 0 || info.certified) return refuse("read-only", "batch-file: signed or certified files are read-only");
      if (count < 1) return refuse("not-pdf", "batch-file: the file has no pages");
      let pages: number[];
      if (action.pages === undefined || action.pages === "") {
        pages = Array.from({ length: count }, (_, index) => index);
      } else {
        const parsed = typeof action.pages === "string" ? parsePageRange(action.pages, count) : null;
        if (!parsed || parsed.length < 1) return refuse("bad-pages", `batch-file: pages ${JSON.stringify(action.pages)} do not fit ${count} pages`);
        pages = [...new Set(parsed)].sort((a, b) => a - b);
      }
      let range: { from: number; to: number } | undefined;
      try {
        checkBatesRange(checked.batesStart, pages.length);
        range = stampLoop(pdf, pages, checked.kind, checked.text, checked);
      } catch (err) {
        const message = errorMessage(err);
        return refuse("stamp-error", message, /^stamp: ([a-z-]+):/.exec(message)?.[1] ?? "text");
      }
      let buffer: ReturnType<typeof pdf.saveToBuffer>;
      try {
        buffer = pdf.saveToBuffer(SAVE_OPTIONS);
      } catch (err) {
        const message = errorMessage(err);
        return refuse("stamp-error", message, "save");
      }
      let out: ArrayBuffer;
      try {
        const view = buffer.asUint8Array();
        if (view.byteLength > MAX_SAVE_BYTES) {
          return refuse("too-large", `batch-file: the result would exceed the ${MAX_SAVE_BYTES} byte save ceiling`);
        }
        out = copyBytes(view);
      } finally {
        buffer.destroy();
      }
      return {
        reply: { id: request.id, ok: true, pages: pages.length, pageCount: count, bytes: out, ...(range ? { bates: range } : {}) },
        transfer: [out],
      };
    } finally {
      closeDocument(source);
    }
  }

  /** Every page's label, `getLabel()`'s own default being the 1-based index. */
  function pageLabels(doc: MupdfDocument): string[] {
    const labels: string[] = [];
    for (let index = 0; index < doc.countPages(); index++) {
      const page = doc.loadPage(index) as MupdfPage;
      try {
        labels.push(attempt(() => page.getLabel(), "") || String(index + 1));
      } finally {
        page.destroy();
      }
    }
    return labels;
  }

  /** The optional-content layers, in mupdf's own order (the reverse of `/OCGs`). */
  function listLayers(pdf: MupdfPdf): LayerItem[] {
    const count = attempt(() => pdf.countLayers(), 0);
    const items: LayerItem[] = [];
    for (let index = 0; index < count; index++) {
      items.push({
        id: index,
        name: attempt(() => pdf.getLayerName(index), "") || `layer ${index + 1}`,
        visible: attempt(() => pdf.isLayerVisible(index), true),
      });
    }
    return items;
  }

  // -------------------------------------------------------------- P6 U7 clipboard, paste, batch

  const CLIP_KINDS: readonly ClipKind[] = ["image", "form", "text", "field", "link", "mark"];
  /** P6 U7 §10.4: the single op whose permission rule and token a pasted member of each kind takes. */
  const CLIP_PERMISSION: Record<ClipKind, string> = {
    image: "edit-object", form: "edit-object", text: "text-add", field: "field-add", link: "link-add", mark: "redact-add",
  };

  type Tagged = Error & { reason?: string; detail?: string; extra?: Record<string, unknown> };

  /** A refusal token outside the typed detail sets (`no-such-page`), reason `invalid`. */
  function failDetail(detail: string, message: string, extra?: Record<string, unknown>): never {
    const error = new Error(message) as Tagged;
    error.reason = "invalid";
    error.detail = detail;
    if (extra) error.extra = extra;
    throw error;
  }

  /**
   * P6 U7 §10.4/§9.12: run one member's checks and rethrow a refusal carrying the member's
   * position. With `plain`, an untagged Error gets that detail (a batch's `stale-target`).
   */
  function tagMember<T>(extra: Record<string, unknown>, run: () => T, plain?: string): T {
    try {
      return run();
    } catch (err) {
      const error = (err instanceof Error ? err : new Error(String(err))) as Tagged;
      if (plain && !error.detail) {
        error.detail = error.reason && error.reason !== "invalid" ? error.reason : plain;
        error.reason ??= "invalid";
      }
      error.extra = { ...(error.extra ?? {}), ...extra };
      throw error;
    }
  }

  /** The page's own `/Resources`, else the inherited one, else null. */
  function pageResources(pageObject: MupdfObject): MupdfObject | null {
    const own = at(pageObject, "Resources");
    if (own) return own;
    const inherited = attempt(() => pageObject.getInheritable("Resources") as MupdfObject, null);
    return inherited && !inherited.isNull() ? inherited : null;
  }

  /** An indirect object number for a grafted copy (a direct copy is added first). */
  function indirectOf(pdf: MupdfPdf, object: MupdfObject): number {
    return object.isIndirect() ? object.asIndirect() : (pdf.addObject(object) as MupdfObject).asIndirect();
  }

  /** P6 U7 §9.9: the raw bytes of every stream object in a scratch document. */
  function scratchStreamBytes(scratch: MupdfPdf): number {
    let total = 0;
    const count = attempt(() => scratch.countObjects(), 0);
    for (let number = 1; number < count; number++) {
      const object = attempt(() => scratch.newIndirect(number) as MupdfObject, null);
      if (!object || !attempt(() => object.isStream(), false)) continue;
      const raw = object.readRawStream();
      try {
        total += raw.getLength();
      } finally {
        raw.destroy();
      }
    }
    return total;
  }

  /** The 6-number inverse of a whole page transform. */
  function affineInverse(m: number[]): number[] {
    const lin = editInvert(m.slice(0, 4));
    if (!lin) failEditObject("unsupported-transform", "clip-paste: the page transform is not invertible");
    return [...lin, -(m[4] * lin[0] + m[5] * lin[2]), -(m[4] * lin[1] + m[5] * lin[3])];
  }

  function applyPoint(m: number[], x: number, y: number): number[] {
    return [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]];
  }

  function unionOf(boxes: number[][]): number[] {
    return [
      Math.min(...boxes.map(box => box[0])), Math.min(...boxes.map(box => box[1])),
      Math.max(...boxes.map(box => box[2])), Math.max(...boxes.map(box => box[3])),
    ];
  }

  function clipKinds(items: ClipItem[]): ClipKinds {
    const kinds = Object.fromEntries(CLIP_KINDS.map(kind => [kind, 0])) as ClipKinds;
    for (const item of items) kinds[item.kind] += 1;
    return kinds;
  }

  /**
   * P6 U7 §1.1/§10.1/§10.2: copy the members into a fresh scratch document; nothing is
   * written to the open document and no journal op is opened. A refusal keeps the old
   * clipboard; the old scratch is destroyed only after the new one is built and charged.
   */
  function clipCopy(session: Session, request: ClipCopyRequest): Record<string, unknown> {
    const doc = session.doc;
    if (!doc) failReason("no-document", "clip-copy: no document open");
    if (session.locked) failReason("read-only", "clip-copy: this document is read-only until it is unlocked");
    const info = detect(doc, session.locked);
    if (!allowed(info, "copy")) failReason("read-only", "clip-copy: this document does not permit copying");
    const page = checkPage(doc, request.page, "clip-copy");
    const pdf = doc.asPDF();
    if (!pdf) failReason("invalid", "clip-copy: the open document is not a PDF");
    const targets = request.targets as unknown;
    const seen = new Set<string>();
    const shapeless = (raw: unknown): boolean => {
      const kind = (raw as { kind?: unknown } | null)?.kind as ClipKind;
      const id = (raw as { id?: unknown } | null)?.id;
      if (!CLIP_KINDS.includes(kind)) return true;
      const content = kind === "image" || kind === "form" || kind === "text";
      if (content ? typeof id !== "string" || !id : !Number.isInteger(id)) return true;
      const key = `${content ? "content" : kind}:${String(id)}`;
      if (seen.has(key)) return true;
      seen.add(key);
      return false;
    };
    if (!Array.isArray(targets) || targets.length < 1 || targets.length > MAX_EDIT_TARGETS || targets.some(shapeless)) {
      failEditObject("invalid-selection", `clip-copy: targets must be 1..${MAX_EDIT_TARGETS} distinct { kind, id }`);
    }
    const list = targets as { kind: ClipKind; id: string | number }[];
    const scratch = new mupdf.PDFDocument();
    const items: ClipItem[] = [];
    let built = false;
    const pageView = doc.loadPage(page) as MupdfPage;
    try {
      // §9.18: one graft map per copy, so a resource shared by two members lands once.
      const graft = scratch.newGraftMap();
      const resources = pageResources(pageView.getObject() as MupdfObject);
      let fields: FieldItem[] | null = null;
      let links: LinkEntry[] | null = null;
      const radios = new Set<number>();
      for (const { kind, id } of list) {
        if (kind === "image" || kind === "form" || kind === "text") {
          const targetId = id as string;
          let record: EditTargetRecord;
          try {
            record = resolveEditTarget(session, { generation: request.generation, revision: request.revision, page, targetId });
          } catch (err) {
            const detail = (err as { reason?: string } | null)?.reason === "too-large" ? "too-large" : "stale-target";
            failEditObject(detail, `clip-copy: ${errorMessage(err)}`, { targetId });
          }
          const refuse = (why: string): never => failEditObject("not-candidate", `clip-copy: ${targetId} cannot be copied (${why})`, {
            targetId, ...(record.reason ? { reason: record.reason } : {}),
          });
          if (record.kind !== kind || record.support !== "candidate") refuse(record.reason ?? "not a candidate");
          if (record.depth !== 0 || (record.ocDepth ?? 0) !== 0) refuse("inside a Form or a layer");
          if (kind === "text") {
            if (record.glyphLine) refuse("a print-to-PDF line");
            const { text, origin, textLinear, payloadBytes, tfName, tfSize, bounds } = record;
            if (typeof text !== "string" || !origin || !textLinear || !payloadBytes || !tfName || typeof tfSize !== "number" || !bounds) {
              refuse("a run that continues a line");
            }
            const font = at(at(resources, "Font"), tfName as string);
            if (!font || !font.isDictionary()) refuse("its font does not resolve");
            if (record.font?.subtype === "Type3" || at(font, "Subtype")?.asName() === "Type3") refuse("type3");
            items.push({
              kind: "text", text: text as string, size: tfSize as number,
              tfState: { tc: record.characterSpacing ?? 0, tw: record.wordSpacing ?? 0, tz: (record.horizontalScale ?? 1) * 100, tl: record.leading ?? 0, tr: record.renderMode as 0 | 1 | 2 },
              origin: [...(origin as number[])], linear: [...(textLinear as number[])], bytes: new Uint8Array(payloadBytes as Uint8Array),
              font: indirectOf(scratch, graft.graftObject(font as MupdfObject) as MupdfObject), bounds: [...(bounds as number[])],
            });
            continue;
          }
          const placement = record.placement;
          if (!placement || !/^c\d+$/.test(record.path) || !record.objectName) refuse("not a top-level simple placement");
          // §9.17: its layer would be grafted without an `/OCProperties` entry.
          if (record.image?.oc) refuse("on a layer");
          const xobject = at(at(resources, "XObject"), record.objectName as string);
          if (!xobject) failEditObject("stale-target", `clip-copy: ${targetId} no longer resolves`, { targetId });
          const p = placement as EditPlacement;
          items.push({
            kind, name: record.objectName as string, view: [...p.view], matrix: [...p.matrix], orient: { ...p.orient },
            box: [...p.box], rect: [...p.rect], visible: [...p.visible], clip: p.clip ? [...p.clip.rect] : null,
            object: indirectOf(scratch, graft.graftObject(xobject) as MupdfObject),
            objectOps: [...(record.objectOps ?? [])], ...(record.reason ? { reason: record.reason } : {}),
          });
          continue;
        }
        const numeric = id as number;
        if (kind === "field") {
          const extra = { widgetId: numeric };
          fields ??= listFields(doc, page);
          const item = fields.find(field => field.id === numeric);
          if (!item) failEditObject("stale-target", `clip-copy: no form field ${numeric} on page ${page}`, extra);
          const refuse = (why: string): never => failEditObject("not-candidate", `clip-copy: field ${numeric} cannot be copied (${why})`, extra);
          if (info.xfa) refuse("an XFA form");
          if (item.scripted) refuse("scripted");
          if (item.kind === "signature" || item.kind === "list") refuse(item.kind);
          if (item.name.includes(".")) refuse("part of a field hierarchy");
          const dict = pdf.newIndirect(numeric) as MupdfObject;
          const parent = at(dict, "Parent");
          if (parent && !isStructuralRadioParent(parent)) refuse("part of a field hierarchy");
          if (item.kind !== "radio") {
            items.push({ kind: "field", item: { ...item }, value: item.value, kids: null, view: [...item.rect] });
            continue;
          }
          // §10.16: the whole group; a kid's `on` is its `/AS` when not Off, else its `/AP` state.
          if (!parent) refuse("a radio button without a group");
          const group = objectId(parent);
          if (radios.has(group)) continue;
          radios.add(group);
          const onPage = new Map(fields.map(field => [field.id, field]));
          const kidList = at(parent, "Kids") as MupdfObject;
          const found: { id: number; on: string; rect: number[]; checked: boolean }[] = [];
          for (let index = 0; index < kidList.length; index++) {
            const kid = kidList.get(index) as MupdfObject;
            const field = onPage.get(objectId(kid));
            if (!field) return refuse("the group has a kid on another page");
            const as = at(kid, "AS");
            const on = as?.isName() && as.asName() !== "Off" ? as.asName() : fieldOn(kid);
            found.push({ id: field.id, on, rect: [...field.rect], checked: field.value === true });
          }
          if (found.length < 2) refuse("the group has fewer than two kids");
          if (found.some(kid => !kid.on) || new Set(found.map(kid => kid.on)).size !== found.length) refuse("its kid names are not unique");
          const kids = [...found.filter(kid => kid.id === numeric), ...found.filter(kid => kid.id !== numeric)];
          items.push({
            kind: "field", item: { ...item }, value: kids.find(kid => kid.checked)?.on ?? "",
            kids: kids.map(kid => ({ on: kid.on, rect: kid.rect })), view: unionOf(kids.map(kid => kid.rect)),
          });
          continue;
        }
        const extra = { annotId: numeric };
        if (kind === "link") {
          if (numeric === 0) failEditObject("not-candidate", "clip-copy: a link without its own object cannot be copied", extra);
          links ??= listLinks(doc, page);
          const entry = links.find(link => link.annotId === numeric);
          if (!entry) failEditObject("stale-target", `clip-copy: no link ${numeric} on page ${page}`, extra);
          if (entry.kind === "blocked") failEditObject("not-candidate", "clip-copy: a blocked link cannot be copied", extra);
          items.push({ kind: "link", rect: [...entry.rect], target: entry.kind === "page" ? { page: entry.page as number } : { uri: entry.uri as string } });
          continue;
        }
        const annot = pageView.getAnnotations().find(candidate => annotId(candidate) === numeric);
        if (!annot) failEditObject("stale-target", `clip-copy: no annotation ${numeric} on page ${page}`, extra);
        if (annot.getType() !== "Redact") failEditObject("not-candidate", "clip-copy: only a redaction mark can be copied", extra);
        const quads = attempt(() => (annot.getQuadPoints() ?? []).map(quad => [...quad] as number[]), [] as number[][]);
        items.push({ kind: "mark", rect: annotRect(annot), quads: quads.length ? quads : null });
      }
      if (scratchStreamBytes(scratch) > MAX_IMAGE_BYTES) {
        failEditObject("too-large", `clip-copy: the copied objects are over the ${MAX_IMAGE_BYTES}-byte clipboard ceiling`);
      }
      built = true;
    } finally {
      pageView.destroy();
      if (!built) scratch.destroy();
    }
    const previous = session.clipboard;
    session.clipboard = { doc: scratch, items };
    previous?.doc.destroy();
    const { revision, dirty } = journalState(session);
    return { id: request.id, ok: true, count: items.length, kinds: clipKinds(items), revision, dirty };
  }

  /**
   * P6 U7 §1.2/§9.3–§9.6/§10.3–§10.6/§10.13–§10.22: the clipboard as ONE journal op. Every
   * check of every member runs before `beginOperation`; the in-op rescan proves the
   * objects and runs, else the op is abandoned. Geometry is view space (y down).
   */
  function clipPaste(session: Session, request: ClipPasteRequest): Record<string, unknown> {
    const doc = requireMutable(session, "clip-paste");
    const page = checkPage(doc, request.page, "clip-paste");
    const clipboard = session.clipboard;
    if (!clipboard || !clipboard.items.length) failEditObject("empty-clipboard", "clip-paste: the clipboard is empty");
    const point = request.at as unknown;
    if (point !== undefined && (!Array.isArray(point) || point.length !== 2 || point.some(value => typeof value !== "number" || !Number.isFinite(value)))) {
      failEditObject("out-of-bounds", "clip-paste: at must be two finite numbers");
    }
    const pdf = doc.asPDF();
    if (!pdf) failReason("invalid", "clip-paste: the open document is not a PDF");
    const items = clipboard.items;
    items.forEach((item, index) => tagMember({ index, kind: item.kind }, () => requireMutable(session, CLIP_PERMISSION[item.kind])));
    const pageObject = pdf.findPage(page) as MupdfObject;
    const pageView = doc.loadPage(page) as MupdfPage;
    let transform: number[];
    let clipPdf: number[];
    try {
      transform = [...pageView.getTransform()];
      clipPdf = editPageClipBox(pageView.getObject() as MupdfObject);
    } finally {
      pageView.destroy();
    }
    const inverse = affineInverse(transform);
    const isObject = (item: ClipItem) => item.kind === "image" || item.kind === "form";
    const viewOf = (item: ClipItem): number[] => item.kind === "text" ? item.bounds : item.kind === "link" || item.kind === "mark" ? item.rect : item.view;
    const boxesOf = (item: ClipItem): number[][] => item.kind === "field" && item.kids ? item.kids.map(kid => kid.rect) : [viewOf(item)];
    const content = items.some(item => isObject(item) || item.kind === "text");
    const scan = content ? scanEditTargets(doc, page) : null;
    const records = scan ? [...scan.records.values()] : [];
    const has = (kind: ClipKind) => items.some(item => item.kind === kind);
    const occupied: Record<"object" | "text" | "field" | "link" | "mark", number[][]> = {
      object: records.filter(record => (record.kind === "image" || record.kind === "form") && record.support === "candidate" && record.placement).map(record => record.placement!.view),
      text: records.filter(record => record.kind === "text" && record.bounds).map(record => record.bounds!),
      field: has("field") ? listFields(doc, page).map(field => field.rect) : [],
      link: has("link") ? listLinks(doc, page).map(link => link.rect) : [],
      mark: has("mark") ? listAnnotations(doc, page, false).filter(annot => annot.type === "Redact").map(annot => annot.rect) : [],
    };
    // §9.3/§10.3/§10.13: one delta for the union, in view space.
    const U = unionOf(items.map(viewOf).concat(items.flatMap(boxesOf)));
    const C = editViewBox(clipPdf, transform);
    const shift = (box: number[], dx: number, dy: number) => [box[0] + dx, box[1] + dy, box[2] + dx, box[3] + dy];
    let dx = 0;
    let dy = 0;
    if (point !== undefined) {
      const [vx, vy] = applyPoint(transform, (point as number[])[0], (point as number[])[1]);
      dx = vx - U[0];
      dy = vy - U[1];
    } else {
      const same = (left: number[], right: number[]) => left.every((value, index) => Math.abs(value - right[index]) <= 0.5);
      const taken = () => items.some(item => boxesOf(item).some(box => {
        const moved = shift(box, dx, dy);
        return occupied[isObject(item) ? "object" : item.kind as "text" | "field" | "link" | "mark"].some(other => same(moved, other));
      }));
      // D2: menu Duplicate's step (+10 right, +10 down on screen, back off at the far edges), at most 20.
      for (let step = 0; step < 20 && taken(); step++) {
        dx += U[2] + dx + 10 > C[2] ? -10 : 10;
        dy += U[3] + dy + 10 > C[3] ? -10 : 10;
      }
    }
    if (U[2] - U[0] > C[2] - C[0] + 1e-6 || U[3] - U[1] > C[3] - C[1] + 1e-6) {
      failEditObject("out-of-bounds", "clip-paste: the clipboard is larger than the page");
    }
    dx = Math.min(Math.max(dx, C[0] - U[0]), C[2] - U[2]);
    dy = Math.min(Math.max(dy, C[1] - U[1]), C[3] - U[3]);
    // §10.19: a rotated or scaled page takes no pasted object.
    const rotate = at(pageObject, "Rotate") ?? attempt(() => pageObject.getInheritable("Rotate") as MupdfObject, null);
    const userUnit = at(pageObject, "UserUnit");
    const geometryBlocked = (rotate !== null && !rotate.isNull() && Math.trunc(rotate.asNumber()) % 360 !== 0)
      || (userUnit !== null && userUnit.asNumber() !== 1);
    const resources = pageResources(pageObject);
    const xobjects = at(resources, "XObject");
    const fonts = at(resources, "Font");
    const snap = (value: number) => Math.round(value * 1e6) / 1e6;
    type ObjectWrite = { index: number; item: ClipItem & { kind: "image" | "form" }; name: string; cm: string; clipText: string | null; full: number[] };
    type TextWrite = { index: number; item: ClipItem & { kind: "text" }; fontName: string; block: string; origin: number[] };
    const objects: ObjectWrite[] = [];
    const texts: TextWrite[] = [];
    let nextObject = 1;
    let nextFont = 1;
    items.forEach((item, index) => {
      if (item.kind !== "image" && item.kind !== "form" && item.kind !== "text") return;
      tagMember({ index, kind: item.kind }, () => {
        if (item.kind === "text") {
          const linear = editMultiply(item.linear, inverse.slice(0, 4));
          const origin = [item.origin[0] + dx, item.origin[1] + dy];
          const [ox, oy] = applyPoint(inverse, origin[0], origin[1]);
          while (at(fonts, `PF${nextFont}`)) nextFont++;
          const fontName = `PF${nextFont++}`;
          const hex = Array.from(item.bytes, byte => byte.toString(16).padStart(2, "0")).join("");
          const { tc, tw, tz, tl, tr } = item.tfState;
          const n = editNumber;
          // §10.15: every text-state parameter pinned; fill black; kerning dropped. Debt sweep 2 §9.4: the
          // source's render mode, with a fixed black 1 pt stroke so an outline never takes the page's stroke state.
          const block = `\nq BT /${fontName} ${n(item.size)} Tf ${n(tc)} Tc ${n(tw)} Tw ${n(tz)} Tz ${n(tl)} TL 0 Ts ${tr} Tr 0 g 0 G 1 w 0 J 0 j [] 0 d ${linear.map(n).join(" ")} ${n(ox)} ${n(oy)} Tm <${hex}> Tj ET Q`;
          texts.push({ index, item, fontName, block, origin });
          return;
        }
        if (geometryBlocked) failEditObject("unsupported-transform", "clip-paste: rotated or scaled pages take no pasted objects");
        const mapped = editViewBox(shift(item.view, dx, dy), inverse).map(snap);
        const visible = [
          Math.max(mapped[0], clipPdf[0]), Math.max(mapped[1], clipPdf[1]),
          Math.min(mapped[2], clipPdf[2]), Math.min(mapped[3], clipPdf[3]),
        ];
        editObjectBox(visible, clipPdf);
        const source = {
          matrix: item.matrix, orient: item.orient, clip: item.clip ? { rect: item.clip } : null,
          box: item.box, visible: item.visible, rect: item.rect,
        } as unknown as EditPlacement;
        const after = editPlacementAfter(source, editRectAffine(item.visible, visible), visible);
        editCheckFull(after.full);
        while (at(xobjects, `PA${nextObject}`)) nextObject++;
        objects.push({ index, item, name: `PA${nextObject++}`, cm: after.cm, clipText: item.clip ? after.clipText : null, full: after.full });
      });
    });
    if (scan) {
      if (!scan.balancedContent) failDetail("unbalanced-content", "clip-paste: existing page content has unbalanced graphics state");
      if (scan.records.size + objects.length + texts.length > MAX_EDIT_TARGETS) {
        failEditObject("too-large", `clip-paste: the paste would put more than ${MAX_EDIT_TARGETS} targets on this page`);
      }
    }
    // §10.17/§10.18: fields through `addField`, names picked against the tree plus this paste.
    const tree = existingFields(pdf);
    const taken = tree ? fieldTreeNames(tree, "field-add") : new Set<string>();
    const fieldWork: (() => void)[] = [];
    const added: { name: string; ids: number[] }[] = [];
    const moveRect = (rect: number[]) => shift(rect, dx, dy);
    items.forEach((item, index) => {
      if (item.kind !== "field") return;
      tagMember({ index, kind: "field" }, () => {
        const source = item.item;
        let name = source.name;
        for (let n = 1; taken.has(name); n++) name = n === 1 ? `${source.name} copy` : `${source.name} copy ${n}`;
        taken.add(name);
        const spec: FieldAddRequest = {
          id: 0, op: "field-add", page, kind: source.kind, name,
          ...(item.kids ? { kids: item.kids.map(kid => ({ on: kid.on, rect: moveRect(kid.rect) })) } : { rect: moveRect(source.rect) }),
          ...(source.kind === "combo" ? { options: source.options ?? [], edit: source.edit === true } : {}),
          ...(source.kind === "text" && source.maxLen !== undefined ? { maxLen: source.maxLen } : {}),
          ...(source.kind === "text" && source.multiline ? { multiline: true } : {}),
          ...(source.tu !== undefined ? { label: source.tu } : {}),
          readOnly: source.readOnly, required: source.required,
          ...(source.default !== undefined ? { default: source.default } : {}),
        };
        added.push(addField(session, spec, work => { fieldWork.push(work); }, item.value));
      });
    });
    // §10.6/§10.20: a page link past the target's last page is skipped, never refused alone.
    const skipped: { index: number; kind: "link"; reason: "no-such-page" }[] = [];
    const linkWrites: { rect: Rect; uri: string }[] = [];
    items.forEach((item, index) => {
      if (item.kind !== "link") return;
      const target = item.target;
      if ("page" in target && target.page >= doc.countPages()) {
        skipped.push({ index, kind: "link", reason: "no-such-page" });
        return;
      }
      tagMember({ index, kind: "link" }, () => {
        const rect = checkLinkRect(pageBox(doc, page), moveRect(item.rect), "link-add");
        linkWrites.push({ rect, uri: "page" in target ? doc.formatLinkURI(fitDest(target.page)) : target.uri });
      });
    });
    if (skipped.length === items.length) failDetail("no-such-page", "clip-paste: every link names a page this document does not have", { index: 0, kind: "link" });
    // §10.22: marks through `redactRect` + `addRedactPlans`, without the search tag.
    const markPlans: { page: number; rect: number[]; quads?: number[][] }[] = [];
    const markIndices: number[] = [];
    items.forEach((item, index) => {
      if (item.kind !== "mark") return;
      tagMember({ index, kind: "mark" }, () => {
        const rect = redactRect(doc, page, moveRect(item.rect));
        const quads = item.quads?.map(quad => quad.map((value, k) => value + (k % 2 ? dy : dx)));
        markPlans.push({ page, rect, ...(quads ? { quads } : {}) });
        markIndices.push(index);
      });
    });
    if (markPlans.length) {
      const room = MAX_REDACT_MARKS - redactMarks(doc).length;
      if (markPlans.length > room) {
        tagMember({ index: markIndices[Math.max(0, room)], kind: "mark" }, () => failDetail("too-many", `clip-paste: at most ${MAX_REDACT_MARKS} marks`));
      }
    }
    const annotIds = (type: "link" | "mark"): Set<number> => {
      const view = doc.loadPage(page) as MupdfPage;
      try {
        return new Set(type === "link"
          ? linkPairs(view).map(pair => pair.annotId).filter(id => id > 0)
          : view.getAnnotations().filter(annot => annot.getType() === "Redact").map(annotId));
      } finally {
        view.destroy();
      }
    };
    const linksBefore = linkWrites.length ? annotIds("link") : new Set<number>();
    const marksBefore = markPlans.length ? annotIds("mark") : new Set<number>();
    let targetIds: string[] = [];
    try {
      mutate(session, doc, "paste", () => {
        // §9.18: one graft map per paste, so a shared SMask lands once.
        const graft = pdf.newGraftMap();
        if (objects.length || texts.length) {
          // §9.5: a fresh direct /Resources with fresh direct /XObject and /Font copies.
          const resourceCopy = pdf.newDictionary();
          resources?.forEach((value, key) => { if (typeof key === "string") resourceCopy.put(key, value); });
          const copyOf = (source: MupdfObject | null) => {
            const copy = pdf.newDictionary();
            source?.forEach((value, key) => { if (typeof key === "string") copy.put(key, value); });
            return copy;
          };
          if (objects.length) {
            const copy = copyOf(xobjects);
            for (const write of objects) copy.put(write.name, graft.graftObject(clipboard.doc.newIndirect(write.item.object)));
            resourceCopy.put("XObject", copy);
          }
          if (texts.length) {
            const copy = copyOf(fonts);
            for (const write of texts) copy.put(write.fontName, graft.graftObject(clipboard.doc.newIndirect(write.item.font)));
            resourceCopy.put("Font", copy);
          }
          pageObject.put("Resources", resourceCopy);
          // §9.6: ONE new /Contents element; no existing element is edited.
          const blocks = objects.map(write => `\nq\n${write.clipText ? `${write.clipText} re W n\n` : ""}${write.cm} cm\n/${write.name} Do\nQ`).join("")
            + texts.map(write => write.block).join("") + "\n";
          const contents = at(pageObject, "Contents");
          const contentArray = pdf.newArray();
          if (contents?.isArray()) for (let i = 0; i < contents.length; i++) contentArray.push(contents.get(i));
          else if (contents && !contents.isNull()) contentArray.push(contents);
          contentArray.push(pdf.addStream(new TextEncoder().encode(blocks), {}));
          pageObject.put("Contents", contentArray);
          const path = `c${contentArray.length - 1}`;
          // §9.6/§10.5/§10.28: the proof, before any widget is added.
          editScanCache = null;
          const fresh = [...scanEditTargets(doc, page).records.values()].filter(record => record.path === path);
          for (const write of objects) {
            const record = fresh.find(candidate => candidate.objectName === write.name);
            const box = record?.support === "candidate" ? record.placement?.box : undefined;
            if (!box || !box.every((value, k) => Math.abs(value - write.full[k]) <= 0.01)) {
              failEditObject("unsupported-transform", `clip-paste: ${write.name} did not land where it was placed`, { index: write.index, kind: write.item.kind });
            }
          }
          if (texts.length) {
            const view = doc.loadPage(page) as MupdfPage;
            let runs: EditRun[];
            try {
              runs = editStructuredRuns(view);
            } finally {
              view.destroy();
            }
            for (const write of texts) {
              let hits = 0;
              for (const run of runs) {
                for (let j = 0; j < run.chars.length; j++) {
                  const origin = run.chars[j].origin;
                  if (!origin || Math.abs(origin[0] - write.origin[0]) > 0.5 || Math.abs(origin[1] - write.origin[1]) > 0.5) continue;
                  let text = "";
                  for (let k = j; k < run.chars.length && text.length < write.item.text.length; k++) text += run.chars[k].ch;
                  if (text === write.item.text) hits++;
                }
              }
              if (hits !== 1) failEditObject("unsupported-transform", "clip-paste: a pasted run did not land where it was placed", { index: write.index, kind: "text" });
            }
            // §10.14: only the pasted runs that scan as candidates are named.
            targetIds = fresh.filter(record => record.kind === "text" && record.support === "candidate")
              .sort((left, right) => left.opStart - right.opStart).map(record => record.targetId);
          }
        }
        for (const work of fieldWork) work();
        if (linkWrites.length) {
          const owner = doc.loadPage(page) as MupdfPage;
          try {
            for (const write of linkWrites) owner.createLink(write.rect, write.uri);
          } finally {
            owner.destroy();
          }
        }
        if (markPlans.length) addRedactPlans(doc, markPlans);
      });
    } finally {
      editScanCache = null;
    }
    const linkIds = linkWrites.length ? [...annotIds("link")].filter(id => !linksBefore.has(id)) : [];
    const markIds = markPlans.length ? [...annotIds("mark")].filter(id => !marksBefore.has(id)) : [];
    return {
      id: request.id, ok: true, page,
      count: objects.length + texts.length + added.length + linkWrites.length + markPlans.length,
      names: objects.map(write => write.name), targetIds,
      fields: added.map(field => field.name), fieldIds: added.flatMap(field => field.ids),
      linkIds, markIds, skipped, ...editState(session),
    };
  }

  /**
   * P6 U7 §1.3/§9.1/§9.12–§9.14: 2..MAX_EDIT_TARGETS mixed-kind writes as ONE journal op.
   * Content items are planned against the one pre-batch scan and written once per
   * `/Contents` element; annotation items then run through their own case bodies by
   * recursive dispatch while `session.batch` holds `mutate` to `work()`.
   */
  function editBatch(session: Session, request: EditBatchRequest): Record<string, unknown> {
    const doc = requireMutable(session, "edit-batch");
    const page = checkPage(doc, request.page, "edit-batch");
    const raw = request.items as unknown;
    if (Array.isArray(raw) && raw.length > MAX_EDIT_TARGETS) failEditObject("too-large", `edit-batch: more than ${MAX_EDIT_TARGETS} items`);
    const shaped = (value: unknown): boolean => {
      if (!value || typeof value !== "object") return false;
      const item = value as Record<string, unknown>;
      const target = typeof item.targetId === "string" && item.targetId !== "";
      switch (item.op) {
        case "edit-text": return target && !("text" in item) && !("size" in item) && !!item.move && typeof item.move === "object";
        case "edit-object": return target && (item.action === "move" || item.action === "delete");
        case "field-props": return Number.isInteger(item.widgetId) && item.rect !== undefined;
        case "field-delete": return Number.isInteger(item.widgetId);
        case "link-set": case "redact-set": return Number.isInteger(item.annotId) && item.rect !== undefined;
        case "link-delete": case "annot-delete": return Number.isInteger(item.annotId);
        default: return false;
      }
    };
    if (!Array.isArray(raw) || raw.length < 2 || !raw.every(shaped)) {
      failEditObject("invalid-selection", "edit-batch: 2 or more items, each one edit-text {targetId, move}, edit-object {targetId, move|delete}, field-props, field-delete, link-set, link-delete, redact-set or annot-delete");
    }
    const list = raw as EditBatchItem[];
    const keyOf = (item: EditBatchItem): "targetId" | "widgetId" | "annotId" =>
      item.op === "edit-text" || item.op === "edit-object" ? "targetId" : item.op === "field-props" || item.op === "field-delete" ? "widgetId" : "annotId";
    const idOf = (item: EditBatchItem, index: number) => ({ index, [keyOf(item)]: (item as Record<string, unknown>)[keyOf(item)] });
    // Debt sweep 2 §9.1 (D1): an item carries only its op's keys, so no single-op option (a field's
    // name, a link's target, a page) can ride along into the dispatched request.
    const ALLOW: Record<EditBatchItem["op"], readonly string[]> = {
      "edit-text": ["targetId", "move"], "edit-object": ["targetId", "action", "box"],
      "field-props": ["widgetId", "rect"], "field-delete": ["widgetId"], "link-set": ["annotId", "rect"],
      "redact-set": ["annotId", "rect"], "link-delete": ["annotId"], "annot-delete": ["annotId"],
    };
    list.forEach((item, index) => {
      const key = Object.keys(item).find(name => name !== "op" && !ALLOW[item.op].includes(name));
      if (key !== undefined) failEditObject("invalid-selection", `edit-batch: unexpected key ${key} on ${item.op}`, idOf(item, index));
    });
    const seen = new Set<string>();
    list.forEach((item, index) => {
      const key = `${keyOf(item)}:${String((item as Record<string, unknown>)[keyOf(item)])}`;
      if (seen.has(key)) failEditObject("invalid-selection", `edit-batch: item ${index} repeats an id`, idOf(item, index));
      seen.add(key);
    });
    const member = <T>(item: EditBatchItem, index: number, run: () => T): T => tagMember(idOf(item, index), run, "stale-target");
    list.forEach((item, index) => member(item, index, () => requireMutable(session, item.op)));
    const clipView = doc.loadPage(page) as MupdfPage;
    let clip: number[];
    try {
      clip = editPageClipBox(clipView.getObject() as MupdfObject);
    } finally {
      clipView.destroy();
    }
    const groups = new Map<number, { plans: EditTextPlan[]; spans: EditSpanEdit[] }>();
    const group = (elementIndex: number) => {
      let entry = groups.get(elementIndex);
      if (!entry) groups.set(elementIndex, entry = { plans: [], spans: [] });
      return entry;
    };
    list.forEach((item, index) => {
      if (item.op !== "edit-text" && item.op !== "edit-object") return;
      member(item, index, () => {
        let record: EditTargetRecord;
        try {
          record = resolveEditTarget(session, { generation: request.generation, revision: request.revision, page, targetId: item.targetId });
        } catch (err) {
          const detail = (err as { reason?: string } | null)?.reason === "too-large" ? "too-large" : "stale-target";
          failEditObject(detail, `edit-batch: ${errorMessage(err)}`);
        }
        const reason = record.reason ? { reason: record.reason } : undefined;
        if (item.op === "edit-text") {
          if (record.kind !== "text" || record.support !== "candidate" || typeof record.text !== "string") {
            failEditText("not-candidate", `edit-batch: ${item.targetId} cannot be edited${record.reason ? ` (${record.reason})` : ""}`, reason);
          }
          // ponytail: a run inside a Form privatises its Form, which would split the element's one splice; it moves on its own.
          if (record.depth > 0) failEditText("not-candidate", `edit-batch: ${item.targetId} is inside a Form; move it on its own`);
          const plan = editPlanText(doc, record, record.text, null, undefined, { move: item.move });
          if (plan.prefix || plan.suffix || plan.rewrite) group(plan.elementIndex).plans.push(plan);
          return;
        }
        if (record.kind === "text" || record.support !== "candidate") {
          failEditObject("not-candidate", `edit-batch: ${item.targetId} cannot be edited${record.reason ? ` (${record.reason})` : ""}`, reason);
        }
        if (!(record.objectOps ?? []).includes(item.action)) {
          failEditObject((record.reason as EditObjectDetail | undefined) ?? "not-candidate", `edit-batch: ${item.action} is not available for ${item.targetId}`, reason);
        }
        const placement = record.placement;
        const elementIndex = Number(/^c(\d+)$/.exec(record.path)?.[1] ?? NaN);
        if (!placement || !Number.isInteger(elementIndex)) failEditObject("not-candidate", `edit-batch: only top-level simple placements are editable (${item.targetId})`);
        if (item.action === "delete") {
          group(elementIndex).spans.push({ record, start: placement.qStart, end: placement.qEnd, replacement: new Uint8Array(0) });
          return;
        }
        const write = editPlanPlacement(placement, "move", item.box, undefined, clip);
        if (!write.noop) group(elementIndex).spans.push(...editPlacementSpans(placement, write.cm, write.clipText).map(span => ({ record, ...span })));
      });
    });
    // §9.13: a batch link-delete names its annotId, resolved before any write.
    const pairsView = doc.loadPage(page) as MupdfPage;
    try {
      const pairs = linkPairs(pairsView);
      list.forEach((item, index) => {
        if (item.op !== "link-delete") return;
        member(item, index, () => {
          if (item.annotId <= 0 || !pairs.some(pair => pair.annotId === item.annotId)) {
            failEditObject("stale-target", `edit-batch: page ${page} has no link ${item.annotId}`);
          }
        });
      });
    } finally {
      pairsView.destroy();
    }
    // §1.3: an annotation item runs its single op; the "plan" pass (every refusal, no write) precedes the one op.
    const runAnnotation = (item: EditBatchItem, index: number) => member(item, index, () => {
      // §1.1: the single-op request is built from the op's allowed keys only, so nothing else leaks even if the ALLOW pass is bypassed.
      const picked = Object.fromEntries(ALLOW[item.op].map(key => [key, (item as Record<string, unknown>)[key]]));
      const reply = dispatch({ id: request.id, page, op: item.op, ...picked } as SessionRequest, session).reply;
      if (reply.ok !== true) {
        const error = new Error(String(reply.error)) as Tagged;
        if (typeof reply.reason === "string") error.reason = reply.reason;
        if (typeof reply.detail === "string") error.detail = reply.detail;
        throw error;
      }
    });
    try {
      session.batch = "plan";
      list.forEach((item, index) => {
        if (item.op !== "edit-text" && item.op !== "edit-object" && item.op !== "link-delete") runAnnotation(item, index);
      });
    } finally {
      session.batch = false;
    }
    const annotations = list.some(item => item.op !== "edit-text" && item.op !== "edit-object");
    if (!groups.size && !annotations) return { id: request.id, ok: true, page, count: 0, noop: true, ...editState(session) };
    const pdf = doc.asPDF();
    if (!pdf) failReason("invalid", "edit-batch: the open document is not a PDF");
    try {
      mutate(session, doc, "edit batch", () => {
        const pageView = doc.loadPage(page) as MupdfPage;
        try {
          const privatised = new Set<number>();
          for (const [elementIndex, entry] of [...groups.entries()].sort((left, right) => left[0] - right[0])) {
            const ordered = [...entry.plans].sort((left, right) => right.record.opStart - left.record.opStart);
            editWriteElement(pdf, pageView, elementIndex, ordered, privatised, entry.spans);
          }
          session.batch = true;
          list.forEach((item, index) => {
            if (item.op === "edit-text" || item.op === "edit-object") return;
            if (item.op !== "link-delete") { runAnnotation(item, index); return; }
            member(item, index, () => {
              const pair = linkPairs(pageView).find(candidate => candidate.annotId === item.annotId);
              if (!pair) failEditObject("stale-target", `edit-batch: page ${page} has no link ${item.annotId}`);
              pageView.deleteLink(pair.link);
            });
          });
        } finally {
          session.batch = false;
          pageView.destroy();
        }
      });
    } finally {
      session.batch = false;
      editScanCache = null;
    }
    return { id: request.id, ok: true, page, count: list.length, ...editState(session) };
  }

  /**
   * P6 U7 §10.17: the `field-add` body, every check before `write(work)`; `field-add` passes
   * `mutate`, `clip-paste` collects the work into its own one op. `valueOverride` sets the
   * created `/V`/`/AS` (a pasted value) independently of `default`. `firstId` is set by the work.
   */
  function addField(session: Session, request: FieldAddRequest, write: (work: () => void) => void, valueOverride?: string | boolean): { name: string; ids: number[]; firstId: number; kind: FieldAddKind } {
    const doc = requireMutable(session, "field-add");
    const info = detect(doc, session.locked);
    if (info.xfa) failField("xfa", "field-add: this is an XFA form; fields cannot be added");
    const page = checkPage(doc, request.page, "field-add");
    const pdf = doc.asPDF();
    if (!pdf) failReason("invalid", "field-add: the open document is not a PDF");
    const kindRaw = request.kind;
    if (kindRaw !== "text" && kindRaw !== "checkbox" && kindRaw !== "combo" && kindRaw !== "radio" && kindRaw !== "signature") {
      failField("invalid", "field-add: kind must be one of text, checkbox, combo, radio, signature");
    }
    const kind = kindRaw as FieldAddKind;
    // P5 U3 §1.2/§9.9: a signature field may come without a name; the engine picks one below.
    const autoName = kind === "signature" && (request.name === undefined || request.name === "");
    let name = autoName ? "" : checkFieldName(request.name, "field-add");
    if (kind === "signature") {
      // §9.5: properties a signature field never has, refused before any write.
      if (request.default !== undefined) failField("invalid", "field-add: default is not a signature property");
      if (request.options !== undefined) failField("invalid", "field-add: options are a dropdown property");
      if (request.kids !== undefined) failField("invalid", "field-add: kids are a radio property");
    }
    const box = pageBox(doc, page);
    // Every kind's payload is validated before the one mutation.
    let rect: Rect | null = null;
    let options: string[] | null = null;
    let kidSpecs: { on: string; rect: Rect }[] | null = null;
    let ff = 0;
    let value: string | MupdfObject | undefined;
    if (kind === "radio") {
      kidSpecs = checkKids(box, request.kids, "field-add");
      ff = mupdf.PDFWidget.BTN_FIELD_IS_RADIO;
      value = "Off";
    } else {
      // §1.2: a signature field's rect is required (no default).
      rect = checkFieldRect(box, kind === "signature" ? request.rect : request.rect ?? (kind === "checkbox"
        ? [72, 72, 88, 88] : [72, 72, 252, 96]), "field-add");
      if (kind === "signature") {
        // §9.9: the appearance floor, after the rect check; no `/V` on create.
        if (rect[2] - rect[0] < 40 || rect[3] - rect[1] < 20) failField("invalid", "field-add: a signature field is at least 40x20 pt");
      }
      else if (kind === "checkbox") { value = "Off"; }
      else if (kind === "text") { value = pdf.newString(""); }
      else { options = checkOptionList(request.options, "field-add"); value = pdf.newString(options[0]); }
    }
    if (kind === "combo") ff = mupdf.PDFWidget.CH_FIELD_IS_COMBO;
    if (request.edit !== undefined) {
      if (kind !== "combo") failField("invalid", "field-add: edit is a dropdown property");
      if (checkFieldFlag(request.edit, "field-add", "edit")) ff |= mupdf.PDFWidget.CH_FIELD_IS_EDIT;
    }
    if (kind === "text") {
      if (request.maxLen !== undefined) {
        if (!Number.isInteger(request.maxLen) || (request.maxLen as number) < 0 || (request.maxLen as number) > MAX_FIELD_TEXT_CHARS) {
          failField("invalid", `field-add: maxLen is an integer 0..${MAX_FIELD_TEXT_CHARS}`);
        }
      }
      if (request.multiline !== undefined && typeof request.multiline !== "boolean") {
        failField("invalid", "field-add: multiline is true or false");
      }
      if (request.multiline === true) ff |= mupdf.PDFWidget.TX_FIELD_IS_MULTILINE;
    } else if (request.maxLen !== undefined || request.multiline !== undefined) {
      failField("invalid", "field-add: maxLen and multiline are text properties");
    }
    // U7 §2.1: the label, the two flag bits and the default, validated before the one
    // mutation. The default is checked against the field this request creates.
    // U7 §2.1: an empty label is "no label" on add, exactly as it is on props.
    const label = request.label === undefined || request.label === "" ? null : checkFieldLabel(request.label, "field-add");
    const readOnly = request.readOnly === undefined ? null : checkFieldFlag(request.readOnly, "field-add", "readOnly");
    const required = request.required === undefined ? null : checkFieldFlag(request.required, "field-add", "required");
    const hasDefault = request.default !== undefined;
    const fieldDefault = hasDefault ? checkFieldDefault(kind, request.default, {
      maxLen: kind === "text" ? (request.maxLen ?? 0) : 0,
      options: options ?? [],
      kids: kidSpecs ? kidSpecs.map(spec => spec.on) : [],
      edit: request.edit === true,
    }, "field-add") : null;
    const flags = ff
      | (readOnly ? mupdf.PDFWidget.FIELD_IS_READ_ONLY : 0)
      | (required ? mupdf.PDFWidget.FIELD_IS_REQUIRED : 0);
    // §2.1/§8.5/§8.6: on create `/V` follows `default`, and the buttons' `/AS` is in the
    // create dict before the wrapper loads, so no `update()` round-trip can lose it.
    const radioOn = kind === "radio" && typeof fieldDefault === "string" ? fieldDefault : "";
    let as = kind === "checkbox" ? "Off" : undefined;
    if (hasDefault) {
      if (kind === "text" && typeof fieldDefault === "string") value = pdf.newString(fieldDefault);
      else if (kind === "combo" && typeof fieldDefault === "string") value = pdf.newString(fieldDefault);
      else if (kind === "checkbox") { as = fieldDefault === true ? FIELD_CHECK_ON : "Off"; value = as; }
    }
    // P6 U7 §10.17: a pasted field's own value, independent of its default.
    let radioValue = radioOn;
    if (valueOverride !== undefined) {
      if (kind === "text" || kind === "combo") value = pdf.newString(String(valueOverride));
      else if (kind === "checkbox") { as = valueOverride === true ? FIELD_CHECK_ON : "Off"; value = as; }
      else if (kind === "radio") radioValue = typeof valueOverride === "string" ? valueOverride : "";
    }
    const existing = existingFields(pdf);
    if (autoName) {
      // §9.9: the first `Signature<n>` no field in the document uses.
      const taken = existing ? fieldTreeNames(existing, "field-add") : new Set<string>();
      let n = 1;
      while (taken.has(`Signature${n}`)) n++;
      name = `Signature${n}`;
    }
    if (existing) checkNameFree(existing, name, null, "field-add");
    const ids: number[] = [];
    const result = { name, ids, firstId: 0, kind };
    // /MK: MuPDF's `update()` draws a border and background only when asked, and
    // counter-rotates only with /R; without them a filled field loses its box and an
    // off radio kid is invisible, and text on a /Rotate page runs sideways (review U6).
    const rotate = pageRotation(doc, page);
    const mk = { BC: [0], BG: [1], ...(rotate ? { R: rotate } : {}) };
    write(() => {
      const ctx = ensureAcroForm(pdf);
      const pageObject = pdf.findPage(page);
      const annots = directAnnots(pdf, pageObject);
      const rects = new Map<number, Rect>();
      if (kind === "radio") {
        const parent = pdf.createObject();
        const kids = kidSpecs!.map(spec => {
          const ref = pdf.addObject({
            Type: "Annot", Subtype: "Widget", Parent: parent, Rect: spec.rect, F: FIELD_F_PRINT,
            P: pageObject, AS: spec.on === radioValue ? spec.on : "Off", DA: pdf.newString(FIELD_DA), MK: mk,
            AP: { N: buttonAppearanceStates(pdf, spec.on, spec.rect) },
          });
          annots.push(ref);
          ids.push(objectId(ref));
          rects.set(objectId(ref), spec.rect);
          return ref;
        });
        parent.writeObject({
          FT: "Btn", T: pdf.newString(name), Ff: flags, Kids: kids, V: radioValue || "Off",
          ...(label !== null ? { TU: pdf.newString(label) } : {}),
          ...(radioOn ? { DV: radioOn } : {}),
        });
        ctx.fields.push(parent);
        result.firstId = ids[0];
      } else {
        const dict: { ff?: number; options?: string[]; value?: string | MupdfObject; as?: string; states?: string; mk?: object; tu?: string; dv?: string | MupdfObject } = { mk };
        if (flags) dict.ff = flags;
        if (options) dict.options = options;
        if (value !== undefined) dict.value = value;
        if (kind === "checkbox") { dict.as = as; dict.states = FIELD_CHECK_ON; }
        if (label !== null) dict.tu = label;
        if (hasDefault && fieldDefault !== null) {
          dict.dv = kind === "checkbox"
            ? (fieldDefault ? FIELD_CHECK_ON : "Off")
            : pdf.newString(String(fieldDefault));
        }
        const ref = mergedFieldDict(pdf, pageObject, kind, name, rect!, dict);
        if (kind === "text" && request.maxLen !== undefined && request.maxLen > 0) {
          (ref as MupdfObject).put("MaxLen", request.maxLen as number);
        }
        ctx.fields.push(ref);
        annots.push(ref);
        ids.push(objectId(ref));
        result.firstId = objectId(ref);
        rects.set(objectId(ref), rect!);
      }
      // The wrapper is loaded *after* the COS create (probe Ceiling 2), then the
      // rect goes through `setRect` so the page transform decides the PDF `/Rect` (§7.1).
      withWidgets(doc, page, ids, found => {
        for (const id of ids) {
          const widget = found.get(id);
          if (!widget) failField("invalid", "field-add: the new field did not land on the page");
          widget.setRect(rects.get(id)!);
          // §9.4: a signature field keeps no `/AP` (MuPDF's `update()` draws its own tag).
          if (kind !== "signature") refreshAppearance(pdf, widget, kind, ctx.helv);
        }
      });
    });
    return result;
  }

  /**
   * Turns one worker request into one reply. Never throws: every failure is an
   * `{ ok: false, error }` reply for the same id, which is what onmessage posts.
   */
  function dispatch(request: SessionRequest, session: Session): DispatchResult {
    try {
      switch (request?.op) {
        case "open": {
          if (typeof request.name !== "string" || !request.name) throw new Error("open: missing document name");
          // mupdf copies the bytes into the WASM heap, so the worker drops its JS copy
          // before anything else can retain it (grok measures the ceiling from this).
          const pdf = request.pdf;
          request.pdf = null;
          if (!pdf) throw new Error("open: no bytes");
          // Compare §9.3: the pair never exceeds MAX_PDF_BYTES; the compare slot goes first.
          let compareClosed = false;
          if (session.compare && pdf.byteLength + session.compare.sourceBytes > MAX_PDF_BYTES) {
            closeDocument(session.compare.doc);
            session.compare = null;
            compareClosed = true;
          }
          let opened: ReturnType<typeof openDocument>;
          let pageCount: number;
          let pages: PageSize[];
          let rotations: number[];
          let info: DocumentInfo;
          try {
            opened = openDocument(pdf);
            try {
              info = detect(opened.doc);
              pageCount = opened.doc.countPages();
              if (!Number.isInteger(pageCount) || pageCount < 0) throw new Error("open: unusable page count");
              pages = pageSizes(opened.doc);
              rotations = pageRotations(opened.doc);
            } catch (err) {
              closeDocument(opened.doc);
              throw err;
            }
            if (pageCount < 1 && !info.needsPassword) {
              closeDocument(opened.doc);
              throw new Error("open: PDF has no pages");
            }
          } catch (err) {
            // §9.19(d): the slot is already gone, so the failure reply must say so too.
            if (!compareClosed) throw err;
            const tagged = (err instanceof Error ? err : new Error(String(err))) as Tagged;
            tagged.extra = { ...(tagged.extra ?? {}), compareClosed: true };
            throw tagged;
          }
          const previous = session.doc;
          session.doc = opened.doc;
          session.name = request.name;
          session.sourceBytes = pdf.byteLength;
          session.locked = info.needsPassword;
          // ponytail: a locked file is held twice (bytes + WASM copy) until it is unlocked.
          session.lockedBytes = info.needsPassword ? pdf : null;
          session.signedSource = (info.signatures > 0 || info.certified) && pdf.byteLength <= MAX_SAVE_BYTES ? pdf.slice(0) : null;
          // A fresh open starts with no proven access and no pending security change.
          session.auth = "none";
          session.password = null;
          session.security = null;
          session.securityDirty = false;
          session.revision = 0;
          session.generation = ++generationCounter;
          editScanCache = null;
          // A freshly opened document has nothing unsaved relative to its bytes.
          session.savedPosition = journalPosition(opened.doc);
          closeDocument(previous);
          return {
            reply: {
              id: request.id, ok: true, pageCount, pages, rotations, info, openMs: opened.openMs, revision: 0, auth: session.auth,
              ...(compareClosed ? { compareClosed: true } : {}),
            },
          };
        }
        case "compare-open": {
          // Compare §9.7: a view-only second document; never journaled, no main doc needed.
          if (typeof request.name !== "string" || !request.name) throw new Error("compare-open: missing document name");
          const bytes = request.pdf;
          request.pdf = null;
          if (!(bytes instanceof ArrayBuffer) || bytes.byteLength < 5) failReason("invalid", "compare-open: not a PDF (too few bytes)");
          // §9.3: checked BEFORE opening, against the main doc's source bytes (0 without one).
          if (bytes.byteLength + session.sourceBytes > MAX_PDF_BYTES) {
            failReason("too-large", `compare-open: the two files together exceed the ${MAX_PDF_BYTES} byte ceiling`);
          }
          let doc: MupdfDocument;
          try {
            doc = mupdf.Document.openDocument(bytes, "application/pdf");
          } catch (err) {
            failReason("invalid", `compare-open: the file could not be read (${errorMessage(err)})`);
          }
          let pageCount: number;
          let pages: PageSize[];
          try {
            const pdf = doc.asPDF();
            if (!pdf) failReason("invalid", "compare-open: the file is not a PDF");
            pdf.disableJS();
            const info = detect(doc);
            if (info.needsPassword || info.encrypted) failReason("encrypted", "compare-open: an encrypted file cannot be compared");
            pageCount = doc.countPages();
            if (!Number.isInteger(pageCount) || pageCount < 1) failReason("invalid", "compare-open: the file has no pages");
            pages = pageSizes(doc);
          } catch (err) {
            closeDocument(doc);
            throw err;
          }
          // A refusal above kept the old slot; the old doc goes only once the new one is stored.
          const previous = session.compare;
          session.compare = { doc, name: request.name, pageCount, sourceBytes: bytes.byteLength };
          if (previous) closeDocument(previous.doc);
          return { reply: { id: request.id, ok: true, name: request.name, pageCount, pages } };
        }
        case "compare-close": {
          const previous = session.compare;
          session.compare = null;
          if (previous) closeDocument(previous.doc);
          return { reply: { id: request.id, ok: true } };
        }
        case "compare-render": {
          const compare = session.compare;
          if (!compare) failReason("no-compare-document", "compare-render: no compare document is open");
          const result = renderPage(compare.doc, request.page, request.scale);
          return {
            reply: { id: request.id, ok: true, page: request.page, ...result },
            transfer: [result.pixels],
          };
        }
        case "compare-text": {
          const doc = session.doc;
          if (!doc) failReason("no-document", "compare-text: no document open");
          if (session.locked) failReason("read-only", "compare-text: document is encrypted, a password is required");
          // §9.2: the MAIN doc's copy bit, as text-all honours it.
          if (!doc.hasPermission("copy")) failReason("read-only", "compare-text: this document does not permit copying");
          const compare = session.compare;
          if (!compare) failReason("no-compare-document", "compare-text: no compare document is open");
          const pageA = checkPage(doc, request.pageA, "compare-text");
          const pageB = checkPage(compare.doc, request.pageB, "compare-text");
          const a = pageLines(doc, pageA);
          const b = pageLines(compare.doc, pageB);
          if (a.length > MAX_COMPARE_LINES || b.length > MAX_COMPARE_LINES) {
            failReason("too-many", `compare-text: a page has more than ${MAX_COMPARE_LINES} lines`);
          }
          return { reply: { id: request.id, ok: true, pageA, pageB, revision: session.revision, ...diffLines(a, b) } };
        }
        case "render": {
          const doc = session.doc;
          if (!doc) throw new Error("render: no document open");
          if (session.locked) {
            throw new Error("render: document is encrypted, a password is required");
          }
          const result = renderPage(doc, request.page, request.scale);
          return {
            reply: { id: request.id, ok: true, page: request.page, revision: session.revision, ...result },
            transfer: [result.pixels],
          };
        }
        case "thumbnail": {
          const doc = session.doc;
          if (!doc) throw new Error("thumbnail: no document open");
          if (session.locked) {
            throw new Error("thumbnail: document is encrypted, a password is required");
          }
          const result = renderThumbnail(doc, request.page, request.maxEdge);
          return {
            reply: { id: request.id, ok: true, page: request.page, revision: session.revision, ...result },
            transfer: [result.pixels],
          };
        }
        case "search": {
          const doc = session.doc;
          if (!doc) throw new Error("search: no document open");
          const query = typeof request.query === "string" ? request.query.trim() : "";
          if (query.length < 1 || query.length > MAX_QUERY_CHARS) {
            throw new Error(`search: query must be 1..${MAX_QUERY_CHARS} characters`);
          }
          if (session.locked) {
            throw new Error("search: document is encrypted, a password is required");
          }
          return { reply: { id: request.id, ok: true, ...searchDocument(doc, query, request.maxHits) } };
        }
        case "authenticate": {
          const doc = session.doc;
          if (!doc) throw new Error("authenticate: no document open");
          if (!session.locked) throw new Error("authenticate: document is not locked");
          const password = request.password;
          if (typeof password !== "string" || password.length > MAX_PASSWORD_CHARS) {
            throw new Error(`authenticate: password must be a string of at most ${MAX_PASSWORD_CHARS} characters`);
          }
          // A failed password is not an error: the document stays open and locked,
          // and the password is not kept anywhere.
          const level = attempt(() => doc.authenticatePassword(password), 0);
          if (!level) {
            return { reply: { id: request.id, ok: true, authenticated: false } };
          }
          // Nothing parsed before the password (page boxes, /AcroForm, /Info) can be
          // trusted: reopen and authenticate before any object is touched.
          if (session.lockedBytes) {
            const fresh = openDocument(session.lockedBytes).doc;
            if (!attempt(() => fresh.authenticatePassword(password), 0)) {
              closeDocument(fresh);
              throw new Error("authenticate: the password did not unlock a fresh copy");
            }
            closeDocument(session.doc);
            session.doc = fresh;
            session.lockedBytes = null;
            session.savedPosition = journalPosition(fresh);
          }
          session.locked = false;
          // mupdf returns 2 for the user and 4 for the owner (bit 3); the owner is
          // not bound by /P, so the session records which of them opened it.
          session.auth = (level & 4) === 4 ? "owner" : "user";
          session.password = password;
          const unlocked = session.doc as MupdfDocument;
          const info = detect(unlocked, false);
          return {
            reply: {
              id: request.id, ok: true, authenticated: true, pageCount: unlocked.countPages(),
              pages: pageSizes(unlocked), rotations: pageRotations(unlocked), info, auth: session.auth,
            },
          };
        }
        case "close": {
          closeDocument(session.doc);
          session.doc = null;
          session.name = null;
          session.sourceBytes = 0;
          session.locked = false;
          session.lockedBytes = null;
          session.signedSource = null;
          session.auth = "none";
          session.password = null;
          session.security = null;
          session.securityDirty = false;
          session.revision = 0;
          session.savedPosition = 0;
          session.generation = ++generationCounter;
          editScanCache = null;
          return { reply: { id: request.id, ok: true } };
        }
        case "fields": {
          const doc = session.doc;
          if (!doc) throw new Error("fields: no document open");
          if (session.locked) throw new Error("fields: document is encrypted, a password is required");
          const page = checkPage(doc, request.page, "fields");
          const items = listFields(doc, page);
          const tabs = pageTabs(doc, page);
          const order = tabs === "S" ? structObjectOrder(doc) : null;
          if (order) {
            for (const item of items) {
              const index = order.get(item.id);
              if (index !== undefined) item.structIndex = index;
            }
          }
          return { reply: { id: request.id, ok: true, page, tabs, items } };
        }
        case "field-set": {
          const doc = requireMutable(session, "field-set");
          const page = checkPage(doc, request.page, "field-set");
          const target = doc.loadPage(page) as MupdfPage;
          let apply: (() => void) | null = null;
          try {
            const widget = findWidget(target, request.widgetId);
            const kind = fieldKind(widget);
            if (!kind) throw new Error("field-set: that widget is not a fillable form field");
            if (kind === "signature") failField("signature", "field-set: click the signature field to sign it");
            if (attempt(() => widget.isReadOnly(), false)) failField("field-read-only", "field-set: that field is read-only");
            const dict = widget.getObject() as MupdfObject;
            const value = request.value;
            if (kind === "checkbox" || kind === "radio") {
              if (typeof value !== "boolean") throw new Error(`field-set: a ${kind} takes true or false`);
              if (kind === "radio" && !value) throw new Error("field-set: a radio cannot be turned off, only switched");
              // toggle() flips, so it only runs when the state differs; otherwise this is a no-op reply.
              if (buttonChecked(dict) !== value) apply = () => { widget.toggle(); };
            } else {
              if (typeof value !== "string") throw new Error(`field-set: a ${kind} takes a string`);
              if (value.length > MAX_NOTE_CHARS) {
                throw new Error(`field-set: a value holds at most ${MAX_NOTE_CHARS} characters`);
              }
              if (kind === "text") {
                const maxLen = attempt(() => widget.getMaxLen(), 0);
                if (maxLen > 0 && value.length > maxLen) {
                  throw new Error(`field-set: this field holds at most ${maxLen} characters`);
                }
                apply = () => { widget.setTextValue(value); };
              } else {
                // §4.1: an Edit-flag combo takes any typed value; `/I` names option indices,
                // so it goes, since MuPDF leaves it behind.
                if (!(kind === "combo" && comboEditable(widget)) && !fieldOptions(widget).includes(value)) {
                  throw new Error(`field-set: ${JSON.stringify(value)} is not one of this field's options`);
                }
                apply = () => { widget.setChoiceValue(value); dropChoiceIndices(dict); };
              }
            }
            if (apply) {
              // One journal operation per set, and the appearance is regenerated with it.
              mutate(session, doc, "fill field", () => {
                apply!();
                widget.update();
              });
            }
          } finally {
            target.destroy();
          }
          return { reply: { id: request.id, ok: true, ...editState(session) } };
        }
        case "field-add": {
          const added = addField(session, request, work => { mutate(session, session.doc as MupdfDocument, "add field", work); });
          const { name, firstId, kind, ids } = added;
          return {
            reply: {
              id: request.id, ok: true, widgetId: firstId, name,
              ...(kind === "radio" ? { widgetIds: [...ids] } : {}),
              ...editState(session),
            },
          };
        }
        case "field-props": {
          const doc = requireMutable(session, "field-props");
          const page = checkPage(doc, request.page, "field-props");
          const pdf = doc.asPDF();
          if (!pdf) failReason("invalid", "field-props: the open document is not a PDF");
          const target = doc.loadPage(page) as MupdfPage;
          let name2: string | null = null;
          let rect2: Rect | null = null;
          let maxLen2: number | null = null;
          let multiline2: boolean | null = null;
          let options2: string[] | null = null;
          let label2: string | null = null;
          let readOnly2: boolean | null = null;
          let required2: boolean | null = null;
          let fieldDefault2: string | boolean | null = null;
          let edit2: boolean | null = null;
          let renameTarget: MupdfObject | null = null;
          const has = (value: unknown) => value !== undefined;
          try {
            const widget = findWidget(target, request.widgetId, "field-props");
            checkFieldEditable(doc, widget, "field-props");
            const dict = widget.getObject() as MupdfObject;
            const fields = existingFields(pdf);
            const parent = at(dict, "Parent");
            const structuralParent = parent && isStructuralRadioParent(parent) && !!fields && isTopLevelField(fields, parent)
              ? parent : null;
            if (parent && !structuralParent) {
              failField("invalid", "field-props: part of a field hierarchy; cannot change it here");
            }
            const wantsProperties = has(request.label) || has(request.readOnly)
              || has(request.required) || has(request.default);
            // §8.2: a group whose kids shadow the field properties would take the write
            // without MuPDF ever reading it back, so the four U7 properties are refused.
            if (wantsProperties && structuralParent && radioKidsCarryOwn(structuralParent)) {
              failField("invalid", "field-props: part of a field hierarchy; cannot change it here");
            }
            if (!has(request.name) && !has(request.rect) && !has(request.maxLen)
              && !has(request.multiline) && !has(request.options) && !has(request.edit) && !wantsProperties) {
              failField("invalid", "field-props: at least one property is needed");
            }
            const signature = fieldKind(widget) === "signature";
            // P5 U3 §9.1: an empty signature field takes rect/label/readOnly/required, never a rename.
            if (signature && has(request.name)) failField("signature", "field-props: a signature field keeps its name");
            if (signature && has(request.default)) failField("invalid", "field-props: default is not a signature property");
            if (has(request.name)) {
              name2 = checkFieldName(request.name, "field-props");
              renameTarget = structuralParent ?? dict;
              if (fields) checkNameFree(fields, name2, renameTarget, "field-props");
            }
            if (has(request.rect)) rect2 = checkFieldRect(pageBox(doc, page), request.rect, "field-props");
            if (signature && rect2 && (rect2[2] - rect2[0] < 40 || rect2[3] - rect2[1] < 20)) {
              failField("invalid", "field-props: a signature field is at least 40x20 pt");
            }
            const kind = fieldKind(widget)!;
            if (has(request.maxLen)) {
              if (kind !== "text") failField("invalid", "field-props: maxLen is a text property");
              const maxLen = request.maxLen;
              if (!Number.isInteger(maxLen) || (maxLen as number) < 0 || (maxLen as number) > MAX_FIELD_TEXT_CHARS) {
                failField("invalid", `field-props: maxLen is an integer 0..${MAX_FIELD_TEXT_CHARS}`);
              }
              // A value the field already holds is never truncated (§7.8).
              if ((maxLen as number) > 0 && attempt(() => widget.getValue(), "").length > (maxLen as number)) {
                failField("invalid", "field-props: maxLen is below the current value's length");
              }
              maxLen2 = maxLen as number;
            }
            if (has(request.multiline)) {
              if (kind !== "text") failField("invalid", "field-props: multiline is a text property");
              if (typeof request.multiline !== "boolean") failField("invalid", "field-props: multiline is true or false");
              multiline2 = request.multiline as boolean;
            }
            if (has(request.options)) {
              if (kind !== "combo") failField("invalid", "field-props: options are a dropdown property");
              options2 = checkOptionList(request.options, "field-props");
            }
            if (has(request.edit)) {
              if (kind !== "combo") failField("invalid", "field-props: edit is a dropdown property");
              edit2 = checkFieldFlag(request.edit, "field-props", "edit");
            }
            if (has(request.label)) label2 = checkFieldLabel(request.label, "field-props");
            if (has(request.readOnly)) readOnly2 = checkFieldFlag(request.readOnly, "field-props", "readOnly");
            if (has(request.required)) required2 = checkFieldFlag(request.required, "field-props", "required");
            if (has(request.default)) {
              // Validated against the field as this same request leaves it (§2.1/§8.3).
              fieldDefault2 = checkFieldDefault(kind, request.default, {
                maxLen: maxLen2 ?? (kind === "text" ? attempt(() => widget.getMaxLen(), 0) : 0),
                options: options2 ?? (kind === "combo" ? fieldOptions(widget) : []),
                kids: structuralParent ? kidOnNames(structuralParent) : [],
                edit: edit2 ?? (kind === "combo" && comboEditable(widget)),
              }, "field-props");
            }
            // §8.3: a maxLen that would cut the field's existing `/DV` is refused, whether or
            // not this request also sets one.
            if (kind === "text" && maxLen2 !== null && maxLen2 > 0) {
              const stale = at(fieldOwnDict(kind, dict), "DV");
              if (stale?.isString() && stale.asString().length > maxLen2) {
                failField("invalid", "field-props: maxLen is below the default value's length");
              }
            }
          } finally {
            target.destroy();
          }
          mutate(session, doc, "change field", () => {
            const t = doc.loadPage(page) as MupdfPage;
            try {
              const widget = findWidget(t, request.widgetId, "field-props");
              const dict = widget.getObject() as MupdfObject;
              const kind = fieldKind(widget)!;
              const ctx = ensureAcroForm(pdf);
              // §2.1: `/TU`, `/Ff` and `/DV` belong on the field dict (the parent's, for a kid).
              const own = fieldOwnDict(kind, dict);
              if (name2 !== null && renameTarget) renameTarget.put("T", pdf.newString(name2));
              let appearance = false;
              if (rect2) { widget.setRect(rect2); appearance = true; }
              if (maxLen2 !== null) {
                if (maxLen2 === 0) dict.delete("MaxLen");
                else dict.put("MaxLen", maxLen2);
                appearance = true;
              }
              if (multiline2 !== null) {
                const flags = at(dict, "Ff");
                const bits = flags?.isNumber() ? flags.asNumber() : 0;
                const next = multiline2
                  ? bits | mupdf.PDFWidget.TX_FIELD_IS_MULTILINE
                  : bits & ~mupdf.PDFWidget.TX_FIELD_IS_MULTILINE;
                dict.put("Ff", next);
                appearance = true;
              }
              if (edit2 !== null) {
                setFieldFlag(own, mupdf.PDFWidget.CH_FIELD_IS_EDIT, edit2);
                appearance = true;
              }
              // §4.1: an Edit combo keeps a typed `/V`/`/DV`; without the flag they must be options.
              const optionsNow = options2 ?? (edit2 === false ? fieldOptions(widget) : null);
              if (optionsNow && !comboEditable(widget)) {
                const current = attempt(() => widget.getValue(), "");
                if (!optionsNow.includes(current)) { dict.put("V", pdf.newString(optionsNow[0])); dropChoiceIndices(dict); }
                const stale = at(own, "DV");
                if (stale?.isString() && !optionsNow.includes(stale.asString())) own.delete("DV");
              }
              if (options2) {
                dict.put("Opt", options2.map(option => pdf.newString(option)));
                // §7.8/§8.3: a `/V` or `/DV` that is no longer an option is fixed above
                // (a combo `/V` is a string, never a name: it is read through the widget).
                appearance = true;
              }
              if (label2 !== null) {
                if (label2 === "") own.delete("TU");
                else own.put("TU", pdf.newString(label2));
              }
              if (readOnly2 !== null) setFieldFlag(own, mupdf.PDFWidget.FIELD_IS_READ_ONLY, readOnly2);
              if (required2 !== null) setFieldFlag(own, mupdf.PDFWidget.FIELD_IS_REQUIRED, required2);
              if (has(request.default)) writeFieldDefault(pdf, own, kind, fieldDefault2);
              // §9.4: a signature field's stale `/AP` goes; MuPDF's `update()` is never run on it.
              if (appearance && kind === "signature") dict.delete("AP");
              else if (appearance) refreshAppearance(pdf, widget, kind, ctx.helv);
            } finally {
              t.destroy();
            }
          });
          return { reply: { id: request.id, ok: true, ...editState(session) } };
        }
        case "field-delete": {
          const doc = requireMutable(session, "field-delete");
          const page = checkPage(doc, request.page, "field-delete");
          const pdf = doc.asPDF();
          if (!pdf) failReason("invalid", "field-delete: the open document is not a PDF");
          const target = doc.loadPage(page) as MupdfPage;
          let widgetId = 0;
          let parentId = -1;
          let dropDefault = false;
          try {
            const widget = findWidget(target, request.widgetId, "field-delete");
            checkFieldEditable(doc, widget, "field-delete");
            const dict = widget.getObject() as MupdfObject;
            const fields = existingFields(pdf);
            const parent = at(dict, "Parent");
            if (parent) {
              if (!isStructuralRadioParent(parent) || !fields || !isTopLevelField(fields, parent)) {
                failField("invalid", "field-delete: part of a field hierarchy; cannot delete here");
              }
              parentId = objectId(parent);
              // §8.3: removing the kid a `/DV` names leaves a default pointing at nothing.
              const dv = at(parent, "DV");
              const on = fieldOn(dict);
              dropDefault = !!dv?.isName() && on !== "" && dv.asName() === on;
            }
            widgetId = annotId(widget);
          } finally {
            target.destroy();
          }
          mutate(session, doc, "delete field", () => {
            const pageObject = pdf.findPage(page);
            // §7.6: the copy replaces a shared indirect `/Annots` on this page alone.
            const current = at(pageObject, "Annots");
            const kept = pdf.newArray();
            if (current?.isArray()) {
              for (let index = 0; index < current.length; index++) {
                const entry = current.get(index) as MupdfObject;
                if (objectId(entry) !== widgetId) kept.push(entry);
              }
            }
            pageObject.put("Annots", kept);
            const root = pdf.getTrailer().get("Root") as MupdfObject;
            const acro = at(root, "AcroForm") as MupdfObject;
            const fields = at(acro, "Fields") as MupdfObject;
            if (parentId < 0) {
              const remaining = pdf.newArray();
              for (let index = 0; index < fields.length; index++) {
                const entry = fields.get(index) as MupdfObject;
                if (objectId(entry) !== widgetId) remaining.push(entry);
              }
              acro.put("Fields", remaining);
            } else {
              const parent = pdf.newIndirect(parentId) as MupdfObject;
              if (dropDefault) parent.delete("DV");
              const kids = at(parent, "Kids") as MupdfObject;
              const remaining = pdf.newArray();
              for (let index = 0; index < kids.length; index++) {
                const entry = kids.get(index) as MupdfObject;
                if (objectId(entry) !== widgetId) remaining.push(entry);
              }
              parent.put("Kids", remaining);
              if (remaining.length === 0) {
                // The last kid takes the group with it (§2.4); `/AcroForm` itself stays (§7.9).
                const remainingFields = pdf.newArray();
                for (let index = 0; index < fields.length; index++) {
                  const entry = fields.get(index) as MupdfObject;
                  if (objectId(entry) !== parentId) remainingFields.push(entry);
                }
                acro.put("Fields", remainingFields);
              }
            }
          });
          return { reply: { id: request.id, ok: true, ...editState(session) } };
        }
        case "annots": {
          const doc = session.doc;
          if (!doc) throw new Error("annots: no document open");
          if (session.locked) throw new Error("annots: document is encrypted, a password is required");
          const page = checkPage(doc, request.page, "annots");
          const info = detect(doc, session.locked);
          const annotated = !session.locked && info.signatures === 0 && !info.certified && allowed(info, "annotate");
          const items = listAnnotations(doc, page, annotated);
          return { reply: { id: request.id, ok: true, page, items } };
        }
        case "annot-add": {
          const doc = requireMutable(session, "annot-add");
          const page = checkPage(doc, request.page, "annot-add");
          const type = request.type;
          if (typeof type !== "string" || !EDITABLE.has(type)) {
            throw new Error(`annot-add: type must be one of ${EDITABLE_TYPES.join(", ")}`);
          }
          const contents = checkContents(request.contents, "annot-add");
          const box = pageBox(doc, page);
          // Validate every shape before the one mutation: a refused add never dirties the document.
          const point = type === "Text" || type === "FreeText" ? checkPoint(box, request.point, "annot-add") : null;
          const quads = MARKUP_COLORS[type] ? checkQuads(box, request.quads, "annot-add") : null;
          const rect = type === "Square" || type === "Circle" || type === "Stamp"
            ? checkRect(box, request.rect, "annot-add") : null;
          const line = type === "Line" ? checkLine(box, request.points, "annot-add") : null;
          const strokes = type === "Ink" ? checkStrokes(box, request.strokes, "annot-add") : null;
          // sign-merge D3: a Stamp signature is an image without `sign` (no icon, no name); refused before mutate.
          if (type === "Stamp" && request.signature !== undefined && (request.signature !== true || request.image === undefined || request.icon !== undefined || request.sign !== undefined)) {
            throw new Error("annot-add: a Stamp signature takes an image and no icon or sign name");
          }
          // A Stamp is one of its six icons or one image, never both and never neither.
          const icon = type === "Stamp" && request.icon !== undefined ? checkIcon(request.icon, "annot-add") : null;
          if (type === "Stamp" && (icon === null) === (request.image === undefined)) {
            throw new Error(`annot-add: a Stamp takes one of ${STAMP_ICONS.join(", ")} or an image`);
          }
          const signName = type === "Stamp" && request.sign !== undefined
            ? checkSignName((request.sign as { name?: unknown }).name, "annot-add") : null;
          if (signName !== null && request.image === undefined) {
            throw new Error("annot-add: a signature appearance needs an image");
          }
          const image = request.image === undefined ? null : decodeStampImage(request.image, "annot-add");
          const fit = type === "FreeText" && request.fit === true;
          if (fit) checkFreeTextEncoding(contents, "annot-add");
          const fitRect = fit ? freeTextFit(box, point as [number, number], contents) : null;
          // draw-signature D1: refused before mutate on any other type or a non-boolean.
          if (request.signature !== undefined && type !== "Stamp" && (type !== "Ink" || typeof request.signature !== "boolean")) {
            throw new Error("annot-add: signature applies to Ink only and must be true or false");
          }
          const drawn = strokes !== null && request.signature === true;
          const imageSignature = type === "Stamp" && request.signature === true;
          const label = drawn ? "add drawn signature" : signName !== null || imageSignature ? "add signature"
            : type === "Text" ? "add note"
              : type === "FreeText" ? "add text box" : `add ${type.toLowerCase()}`;
          try {
            mutate(session, doc, label, () => {
              const target = doc.loadPage(page) as MupdfPage;
              try {
                const annot = target.createAnnotation(type as AnnotationType);
                if (quads) {
                  // The quads are the search hit's own y-down page points; MuPDF's writer flips
                  // them to PDF y-up on save, so setQuadPoints and `search` share one space.
                  annot.setQuadPoints(quads);
                  annot.setColor(MARKUP_COLORS[type]);
                  annot.setContents(contents);
                } else if (type === "FreeText") {
                  annot.setRect(fitRect ?? freeTextBox(box, point as [number, number]));
                  annot.setDefaultAppearance("Helv", 12, [0, 0, 0]);
                  annot.setBorderWidth(0);
                  annot.setContents(contents);
                } else if (type === "Square" || type === "Circle") {
                  annot.setRect(rect!);
                  annot.setColor(SHAPE_STYLES[type].color);
                  annot.setBorderWidth(SHAPE_STYLES[type].width);
                  annot.setContents(contents);
                } else if (line) {
                  annot.setLine(line[0], line[1]);
                  annot.setColor(SHAPE_STYLES.Line.color);
                  annot.setBorderWidth(SHAPE_STYLES.Line.width);
                  annot.setContents(contents);
                } else if (strokes && drawn) {
                  const xs = strokes.flat().map(point => point[0]);
                  annot.setInkList(strokes);
                  annot.setColor([0, 0, 0]);
                  annot.setBorderWidth(signatureLineWidth(Math.max(...xs) - Math.min(...xs)));
                  annot.setSubject(DRAWN_SIGNATURE_SUBJECT);
                } else if (strokes) {
                  annot.setInkList(strokes);
                  annot.setColor(SHAPE_STYLES.Ink.color);
                  annot.setBorderWidth(SHAPE_STYLES.Ink.width);
                  annot.setContents(contents);
                } else if (type === "Stamp") {
                  // An icon stamp's rect is measured by mupdf's own icon (it keeps the
                  // centre); an image stamp keeps the rect the caller asked for.
                  annot.setRect(rect!);
                  if (image) {
                    annot.setStampImage(image);
                    if (signName !== null) {
                      annot.setSubject(SIGNATURE_SUBJECT);
                      annot.setContents(signatureContents(signName));
                    } else if (imageSignature) {
                      annot.setSubject(SIGNATURE_SUBJECT); // sign-merge D3: no /Contents, no /T
                    } else {
                      annot.setContents(contents);
                    }
                  } else {
                    annot.setIcon(icon!);
                    annot.setColor(SHAPE_STYLES.Stamp.color);
                    annot.setContents(contents);
                  }
                } else {
                  // getRect semantics on this MuPDF build: the rect is y-down from the
                  // page top, so the point is the note's top-left as the renderer sees it.
                  annot.setRect([point![0], point![1], point![0] + NOTE_SIZE, point![1] + NOTE_SIZE]);
                  annot.setContents(contents);
                  annot.setIsOpen(false);
                }
                annot.update();
              } finally {
                target.destroy();
              }
            });
          } finally {
            image?.destroy();
          }
          return { reply: { id: request.id, ok: true, ...editState(session) } };
        }
        case "annot-set": {
          const doc = requireMutable(session, "annot-set");
          const page = checkPage(doc, request.page, "annot-set");
          const contents = checkContents(request.contents, "annot-set");
          const target = doc.loadPage(page) as MupdfPage;
          try {
            const annot = findAnnotation(target, request.annotId, "annot-set");
            if (!EDITABLE.has(annot.getType())) {
              throw new Error(`annot-set: only ${EDITABLE_TYPES.join(", ")} annotations are editable`);
            }
            // D1: only a box drawn the way `fit` measures it (Helv 12) is re-fitted; any other font keeps today's path.
            const fit = request.fit === true && annot.getType() === "FreeText"
              && attempt(() => { const da = annot.getDefaultAppearance(); return da.font === "Helv" && da.size === 12; }, false);
            if (fit) checkFreeTextEncoding(contents, "annot-set");
            // Refit from the current top-left: left edge and first line stay where they are.
            const current = fit ? [...annot.getRect()] as number[] : null;
            const rect = current ? freeTextFit(pageBox(doc, page), [current[0], current[1] + 7.2], contents) : null;
            mutate(session, doc, "edit annotation", () => {
              if (rect) annot.setRect(rect);
              annot.setContents(contents);
              annot.update();
            });
          } finally {
            target.destroy();
          }
          return { reply: { id: request.id, ok: true, ...editState(session) } };
        }
        case "annot-move": {
          // fill-anywhere D2: FreeText only; size kept, clamped inside the page; landing in place is a no-op.
          const doc = requireMutable(session, "annot-move");
          const page = checkPage(doc, request.page, "annot-move");
          const [px, py] = finiteNumbers(request.point, 2, "point", "annot-move");
          const box = pageBox(doc, page);
          const target = doc.loadPage(page) as MupdfPage;
          try {
            const annot = findAnnotation(target, request.annotId, "annot-move");
            const subject = attempt(() => annot.getSubject(), "");
            // sign-merge D3: a signature-appearance Stamp (image signatures and the old dialog's) moves and resizes like a drawing.
            const stampSig = annot.getType() === "Stamp" && subject === SIGNATURE_SUBJECT;
            const drawn = annot.getType() === "Ink" && subject === DRAWN_SIGNATURE_SUBJECT || stampSig;
            if (annot.getType() !== "FreeText" && !drawn) throw new Error("annot-move: only FreeText annotations can be moved");
            if (request.width !== undefined && !drawn) throw new Error("annot-move: width applies to drawn signatures only");
            if (request.width !== undefined && (typeof request.width !== "number" || !Number.isFinite(request.width))) {
              throw new Error("annot-move: width must be a finite number");
            }
            if (drawn) {
              // draw-signature D2: translate, then scale about the new top-left; aspect kept, inside the page.
              const strokes = stampSig ? [] : annot.getInkList() as number[][][];
              const [x0, y0, x1, y1] = annotRect(annot), w0 = x1 - x0, h0 = y1 - y0;
              if (request.width !== undefined && w0 < 1) throw new Error("annot-move: this signature is too narrow to resize");
              const pageW = box[2] - box[0], pageH = box[3] - box[1];
              // Review: a signature Stamp larger than the page (a foreign one) is shrunk to fit, aspect kept, so its rect lands inside the page.
              const newW = request.width === undefined ? (stampSig ? Math.min(w0, pageW, h0 > 0 ? pageH * w0 / h0 : Infinity) : w0)
                : Math.min(Math.max(request.width, 24), pageW, h0 > 0 ? pageH * w0 / h0 : Infinity);
              const scale = w0 > 0 ? newW / w0 : 1, newH = h0 * scale;
              const x = Math.max(box[0], Math.min(px, box[2] - newW)), y = Math.max(box[1], Math.min(py, box[3] - newH));
              const rect = [x, y, x + newW, y + newH];
              if (rect.every((value, index) => Math.abs(value - [x0, y0, x1, y1][index]) <= 0.01)) {
                return { reply: { id: request.id, ok: true, noop: true, ...editState(session) } };
              }
              const clampX = (v: number) => Math.max(box[0], Math.min(box[2], v)), clampY = (v: number) => Math.max(box[1], Math.min(box[3], v));
              const mapped = strokes.map(stroke => stroke.map(([sx, sy]) => [clampX(x + (sx - x0) * scale), clampY(y + (sy - y0) * scale)] as [number, number]));
              mutate(session, doc, Math.abs(newW - w0) > 0.01 ? "resize signature" : "move signature", () => {
                if (stampSig) annot.setRect(rect as Rect);
                else { annot.setInkList(mapped); annot.setBorderWidth(signatureLineWidth(newW)); }
                annot.update();
              });
              return { reply: { id: request.id, ok: true, ...editState(session) } };
            }
            const current = [...annot.getRect()] as number[];
            const width = Math.min(current[2] - current[0], box[2] - box[0]);
            const height = Math.min(current[3] - current[1], box[3] - box[1]);
            const x = Math.max(box[0], Math.min(px, box[2] - width));
            const y = Math.max(box[1], Math.min(py, box[3] - height));
            const rect: Rect = [x, y, x + width, y + height];
            if (rect.every((value, index) => Math.abs(value - current[index]) <= 0.01)) {
              return { reply: { id: request.id, ok: true, noop: true, ...editState(session) } };
            }
            mutate(session, doc, "move text box", () => {
              annot.setRect(rect);
              annot.update();
            });
          } finally {
            target.destroy();
          }
          return { reply: { id: request.id, ok: true, ...editState(session) } };
        }
        case "annot-delete": {
          const doc = requireMutable(session, "annot-delete");
          const page = checkPage(doc, request.page, "annot-delete");
          const target = doc.loadPage(page) as MupdfPage;
          try {
            const annot = findAnnotation(target, request.annotId, "annot-delete");
            if (!EDITABLE.has(annot.getType()) && annot.getType() !== "Redact") {
              throw new Error(`annot-delete: only editable annotations and Redact marks can be deleted`);
            }
            mutate(session, doc, "delete annotation", () => {
              // deleting a Text note takes its Popup with it; no orphan survives in /Annots
              target.deleteAnnotation(annot);
            });
          } finally {
            target.destroy();
          }
          return { reply: { id: request.id, ok: true, ...editState(session) } };
        }
        case "redact-add": {
          const doc = requireMutable(session, "redact-add");
          const page = checkPage(doc, request.page, "redact-add");
          const rect = redactRect(doc, page, request.rect);
          if (redactMarks(doc).length >= MAX_REDACT_MARKS) failReason("too-many", `redact-add: at most ${MAX_REDACT_MARKS} marks`);
          mutate(session, doc, "add redaction mark", () => addRedactPlans(doc, [{ page, rect }]));
          return { reply: { id: request.id, ok: true, added: 1, pages: [page], ...journalState(session) } };
        }
        case "redact-set": {
          // P6 U3 §3.1/§9.4: permission, page, clamped rect, the mark, then no-op, then one op.
          const doc = requireMutable(session, "redact-set");
          const page = checkPage(doc, request.page, "redact-set");
          let rect: number[];
          try {
            rect = redactRect(doc, page, request.rect, "redact-set");
          } catch (err) {
            failRedact("rect", errorMessage(err));
          }
          const target = doc.loadPage(page) as MupdfPage;
          try {
            let annot: MupdfAnnotation;
            try {
              annot = findAnnotation(target, request.annotId, "redact-set");
            } catch (err) {
              failRedact("no-such-annot", errorMessage(err));
            }
            if (annot.getType() !== "Redact") failRedact("not-redact", "redact-set: only a redaction mark can be moved");
            const current = attempt(() => [...annot.getRect()] as number[], [] as number[]);
            if (current.length === 4 && rect.every((value, index) => Math.abs(value - current[index]) <= 0.01)) {
              return { reply: { id: request.id, ok: true, noop: true, ...journalState(session) } };
            }
            mutate(session, doc, "move redaction mark", () => setRedactRect(annot, rect));
          } finally {
            target.destroy();
          }
          return { reply: { id: request.id, ok: true, ...journalState(session) } };
        }
        case "redact-search": {
          const doc = requireMutable(session, "redact-search");
          const plans = redactSearchPlan(doc, request.text);
          if (redactMarks(doc).length + plans.length > MAX_REDACT_MARKS) failReason("too-many", `redact-search: at most ${MAX_REDACT_MARKS} marks`);
          mutate(session, doc, "mark redaction search hits", () => addRedactPlans(doc, plans));
          return { reply: { id: request.id, ok: true, added: plans.length, pages: [...new Set(plans.map(plan => plan.page))], ...journalState(session) } };
        }
        case "redact-preview": {
          const doc = session.doc;
          if (!doc) failReason("no-document", "redact-preview: no document open");
          if (session.locked) failReason("read-only", "redact-preview: this document is locked");
          return { reply: { id: request.id, ok: true, ...redactPreview(doc) } };
        }
        case "sign-list": {
          const doc = session.doc;
          if (!doc) failReason("no-document", "sign-list: no document open");
          if (session.locked) failReason("encrypted", "sign-list: encrypted document needs a password");
          const pdf = doc.asPDF() as MupdfPdf;
          const trailer = pdf.getTrailer() as MupdfObject;
          const root = at(trailer, "Root");
          const fields = at(at(root, "AcroForm"), "Fields");
          const hasSignatureField = (nodes: MupdfObject | null, depth = 0): boolean => {
            if (!nodes?.isArray() || depth > MAX_FIELD_DEPTH) return false;
            for (let i = 0; i < nodes.length; i++) {
              const node = nodes.get(i) as MupdfObject;
              const ft = at(node, "FT");
              if ((ft?.isName() && ft.asName() === "Sig") || hasSignatureField(at(node, "Kids"), depth + 1)) return true;
            }
            return false;
          };
          if (detect(doc, false).encrypted && hasSignatureField(fields)) failReason("encrypted", "sign-list: encrypted signed documents cannot be verified in this version");
          const widgetById = new Map<number, { page: number; rect: number[] }>();
          for (let pageIndex = 0; pageIndex < doc.countPages(); pageIndex++) {
            const page = doc.loadPage(pageIndex) as MupdfPage;
            try {
              for (const widget of page.getWidgets()) {
                const dict = widget.getObject() as MupdfObject;
                const info = { page: pageIndex + 1, rect: [...widget.getRect()] };
                for (let node: MupdfObject | null = dict, depth = 0; node && depth < MAX_FIELD_DEPTH; depth++, node = at(node, "Parent")) {
                  widgetById.set(attempt(() => node!.asIndirect(), -1), info);
                }
              }
            } finally { page.destroy(); }
          }
          const signatures: Record<string, unknown>[] = [];
          let visited = 0;
          const walk = (node: MupdfObject, prefix: string, inheritedType: string | null, depth: number) => {
            if (depth > MAX_FIELD_DEPTH) return;
            if (++visited > MAX_FIELD_NODES) failReason("too-many", "sign-list: this form is too large to verify");
            const ownName = at(node, "T");
            const name = ownName?.isString() ? ownName.asString() : "";
            const field = [prefix, name].filter(Boolean).join(".");
            const ft = at(node, "FT");
            const type = ft?.isName() ? ft.asName() : inheritedType;
            const value = at(node, "V");
            if (type === "Sig" && (ft?.isName() || value || name)) {
              const widget = widgetById.get(attempt(() => node.asIndirect(), -1));
              const entry: Record<string, unknown> = { field, page: widget?.page ?? 0, rect: widget?.rect ?? null, empty: !value };
              if (value) {
                const br = at(value, "ByteRange");
                const contents = at(value, "Contents");
                const range = br?.isArray() ? Array.from({ length: br.length }, (_, i) => attempt(() => Number((br.get(i) as MupdfObject).asNumber()), NaN)) : [];
                if (!value.isDictionary() || !br?.isArray() || !contents?.isString() || range.length !== 4 || range.some(n => !Number.isInteger(n) || n < 0)) {
                  entry.status = "unsupported"; entry.reason = "malformed";
                } else {
                  entry.subFilter = attempt(() => at(value, "SubFilter")!.asName(), "");
                  entry.filter = attempt(() => at(value, "Filter")!.asName(), "");
                  for (const key of ["Name", "Reason", "Location", "ContactInfo", "M"]) {
                    const item = at(value, key);
                    if (item?.isString()) entry[key === "ContactInfo" ? "contact" : key === "M" ? "m" : key.toLowerCase()] = item.asString();
                  }
                  entry.byteRange = range;
                  entry.contents = contents?.asByteString?.() ?? new Uint8Array();
                  const refs = at(value, "Reference");
                  if (refs?.isArray()) for (let i = 0; i < refs.length; i++) {
                    const ref = refs.get(i) as MupdfObject;
                    if (attempt(() => at(ref, "TransformMethod")!.asName(), "") === "DocMDP") {
                      entry.docmdp = Number(at(at(ref, "TransformParams"), "P")?.asNumber?.()) || null; break;
                    }
                  }
                }
              }
              signatures.push(entry);
            }
            const kids = at(node, "Kids");
            if (kids?.isArray()) for (let i = 0; i < kids.length; i++) walk(kids.get(i) as MupdfObject, field, type, depth + 1);
          };
          if (fields?.isArray()) for (let i = 0; i < fields.length; i++) walk(fields.get(i) as MupdfObject, "", null, 1);
          const hasSigned = signatures.some(entry => !entry.empty);
          if (hasSigned && !session.signedSource) failReason("too-large", "sign-list: source is too large to verify");
          const bytes = session.signedSource ? session.signedSource.slice(0) : new ArrayBuffer(0);
          const perm = at(at(root, "Perms"), "DocMDP");
          let certified: number | null = null;
          if (perm) { const refs = at(perm, "Reference"); if (refs?.isArray()) for (let i = 0; i < refs.length; i++) {
            const ref = refs.get(i) as MupdfObject;
            if (attempt(() => at(ref, "TransformMethod")!.asName(), "") === "DocMDP") certified = Number(at(at(ref, "TransformParams"), "P")?.asNumber?.()) || null;
          } }
          return { reply: { id: request.id, ok: true, signatures, certified, versions: pdf.countVersions(), bytes }, transfer: [bytes] };
        }
        case "sign-prepare": {
          const doc = session.doc;
          if (!doc) failReason("no-document", "sign-prepare: no document open");
          if (session.locked) failReason("encrypted", "sign-prepare: encrypted documents cannot be signed in this version");
          if (session.sourceBytes > MAX_SAVE_BYTES) failReason("too-large", "sign-prepare: source exceeds save limit");
          if (typeof request.name !== "string" || !request.name.length || request.name.length > 200
            || typeof request.reason !== "string" || request.reason.length > 200 || /[\x00-\x1f\x7f]/.test(request.reason)
            || typeof request.certify !== "boolean" || typeof request.when !== "number" || !Number.isFinite(request.when)) {
            failReason("invalid", "sign-prepare: invalid signer, reason, or timestamp");
          }
          // P5 U3 §1.4/§9.8: a field signature names the field, its page, and no rect.
          const field = request.field ?? null;
          if (field !== null && (typeof field !== "string" || !field.length || request.rect !== null)) {
            failReason("invalid", "sign-prepare: a field signature takes rect null and the field's page");
          }
          const info = detect(doc, false);
          if (info.encrypted) failReason("encrypted", "sign-prepare: encrypted documents cannot be signed in this version");
          const root = at((doc.asPDF() as MupdfPdf).getTrailer(), "Root");
          const docMdp = at(at(root, "Perms"), "DocMDP");
          if (docMdp) {
            const reference = at(docMdp, "Reference");
            if (reference?.isArray() && reference.length) {
              const params = at(reference.get(0) as MupdfObject, "TransformParams");
              if (Number(at(params, "P")?.asNumber?.()) === 1) failReason("certified", "sign-prepare: this certified document does not allow changes");
            }
          }
          if (request.certify && info.signatures > 0) failReason("already-signed", "sign-prepare: certification must be the first signature");
          // A new visible field after certification is an illegal DocMDP change (pyHanko, Acrobat); invisible is form filling.
          // §1.4: filling an existing field is a permitted P=2 change (probe G6); a P=1 document was refused above.
          if (field === null && info.certified && request.rect !== null) failReason("certified", "sign-prepare: a certified document accepts only an invisible signature");
          if (!Number.isInteger(request.page) || request.page < 0 || request.page >= doc.countPages()) failReason("invalid", "sign-prepare: invalid page");
          if (request.rect !== null && (!Array.isArray(request.rect) || request.rect.length !== 4 || request.rect.some(n => typeof n !== "number" || !Number.isFinite(n)))) failReason("invalid", "sign-prepare: invalid rectangle");
          if (field !== null) checkSignatureField(doc, request.page, field);
          const incremental = !!session.signedSource;
          const source = session.signedSource ?? serializeDocument(session).bytes;
          const prepared = prepareSignature(mupdf, source, request, incremental);
          if (session.signedSource && !new Uint8Array(prepared.bytes).subarray(0, source.byteLength).every((b, i) => b === new Uint8Array(source)[i])) failReason("invalid", "sign-prepare: incremental save changed source bytes");
          return { reply: { id: request.id, ok: true, ...prepared, ...(field !== null ? { field } : {}) }, transfer: [prepared.bytes] };
        }
        case "redact-apply": {
          const doc = requireMutable(session, "redact-apply");
          if (session.sourceBytes > MAX_SAVE_BYTES) failReason("too-large", `redact-apply: source exceeds ${MAX_SAVE_BYTES} bytes`);
          const preview = redactPreview(doc);
          if (!preview.marks) failReason("no-marks", "redact-apply: there are no marks");
          if (preview.shared.length) failReason("shared-content", "redact-apply: marked graphics also appear without a mark");
          const sanitize = checkSanitize(request.sanitize);
          const intermediate = serializeDocument(session).bytes;
          checkRedactSourceBytes(intermediate.byteLength);
          const password = session.security?.mode === "aes-256" ? session.security.userPassword : session.password;
          const applied = applyRedactions(mupdf, intermediate, password, sanitize, saveOptions(session));
          if (applied.bytes.byteLength > MAX_SAVE_BYTES) failReason("too-large", `redact-apply: output exceeds ${MAX_SAVE_BYTES} bytes`);
          validateDocument(session, applied.bytes, true);
          return { reply: { id: request.id, ok: true, ...applied }, transfer: [applied.bytes] };
        }
        case "outline": {
          const doc = session.doc;
          if (!doc) throw new Error("outline: no document open");
          if (session.locked) throw new Error("outline: document is encrypted, a password is required");
          return { reply: { id: request.id, ok: true, items: listOutline(doc) } };
        }
        case "outline-add": {
          const doc = requireMutable(session, "outline-add");
          const page = checkPage(doc, request.page, "outline-add");
          const title = checkTitle(request.title, "outline-add");
          mutate(session, doc, "add bookmark", () => appendBookmark(doc, title, page));
          return { reply: { id: request.id, ok: true, ...editState(session) } };
        }
        case "outline-set": {
          const doc = requireMutable(session, "outline-set");
          const title = checkTitle(request.title, "outline-set");
          // Validation and the seek both happen before the one mutation.
          const it = outlineCursor(doc, request.outlineId, "outline-set");
          try {
            const item = it.item();
            if (!item) throw new Error(`outline-set: no bookmark ${String(request.outlineId)}`);
            mutate(session, doc, "rename bookmark", () => {
              // The destination and the subtree are the user's; only the title changes.
              it.update({ title, uri: item.uri, open: item.open, down: undefined });
            });
          } finally {
            it.destroy();
          }
          return { reply: { id: request.id, ok: true, ...editState(session) } };
        }
        case "outline-delete": {
          const doc = requireMutable(session, "outline-delete");
          const it = outlineCursor(doc, request.outlineId, "outline-delete");
          try {
            mutate(session, doc, "delete bookmark", () => {
              // delete() takes the whole subtree with it (stated).
              it.delete();
            });
          } finally {
            it.destroy();
          }
          return { reply: { id: request.id, ok: true, ...editState(session) } };
        }
        case "links": {
          const doc = session.doc;
          if (!doc) throw new Error("links: no document open");
          if (session.locked) throw new Error("links: document is encrypted, a password is required");
          const page = checkPage(doc, request.page, "links");
          return { reply: { id: request.id, ok: true, page, items: listLinks(doc, page) } };
        }
        case "link-add": {
          const doc = requireMutable(session, "link-add");
          const page = checkPage(doc, request.page, "link-add");
          const rect = checkLinkRect(pageBox(doc, page), request.rect, "link-add");
          const target = request.target as { page?: unknown; uri?: unknown } | null;
          let uri: string;
          if (target && target.page !== undefined) {
            uri = doc.formatLinkURI(fitDest(checkPage(doc, target.page, "link-add")));
          } else if (target && typeof target.uri === "string") {
            const scheme = schemeOf(target.uri);
            // The allowlist is the guard, but refusing `file:` also keeps mupdf from
            // writing the remote GoToR/filespec action it makes of that URI.
            if (!(LINK_SCHEMES as readonly string[]).includes(scheme)) {
              throw new Error(`link-add: ${scheme || "a URI without a scheme"} is not a link this app opens`);
            }
            if (target.uri.length > MAX_URI_CHARS) {
              throw new Error(`link-add: a link URI holds at most ${MAX_URI_CHARS} characters`);
            }
            uri = target.uri;
          } else {
            throw new Error("link-add: target must be {page} or {uri}");
          }
          mutate(session, doc, "add link", () => {
            const owner = doc.loadPage(page) as MupdfPage;
            try {
              owner.createLink(rect, uri);
            } finally {
              owner.destroy();
            }
          });
          return { reply: { id: request.id, ok: true, ...editState(session) } };
        }
        case "link-delete": {
          const doc = requireMutable(session, "link-delete");
          const page = checkPage(doc, request.page, "link-delete");
          const target = doc.loadPage(page) as MupdfPage;
          try {
            const links = target.getLinks();
            const index = request.linkId;
            if (!Number.isInteger(index) || index < 0 || index >= links.length) {
              throw new Error(`link-delete: no link ${String(index)} on this page`);
            }
            const link = links[index];
            mutate(session, doc, "delete link", () => target.deleteLink(link));
          } finally {
            target.destroy();
          }
          return { reply: { id: request.id, ok: true, ...editState(session) } };
        }
        case "link-set": {
          // P6 U4 §1.2/§9.2: permission, page, presence, rect, the entry, then target and scheme;
          // a no-op writes nothing; otherwise ONE op on one page handle, destroyed before the reply.
          const doc = requireMutable(session, "link-set");
          const page = checkPage(doc, request.page, "link-set");
          const hasRect = request.rect !== undefined;
          const hasTarget = request.target !== undefined;
          if (!hasRect && !hasTarget) failLink("target", "link-set: rect or target is required");
          let rect: Rect | null = null;
          if (hasRect) {
            try {
              rect = checkLinkRect(pageBox(doc, page), request.rect, "link-set");
            } catch (err) {
              failLink("rect", errorMessage(err));
            }
          }
          const wanted = request.annotId;
          const missing = `link-set: page ${page} has no link ${String(wanted)}`;
          if (!Number.isInteger(wanted) || wanted <= 0) failLink("no-such-annot", missing);
          const owner = doc.loadPage(page) as MupdfPage;
          let noop = false;
          try {
            const annots = at(owner.getObject() as MupdfObject, "Annots");
            let entry: MupdfObject | null = null;
            const length = annots?.isArray() ? annots.length : 0;
            for (let index = 0; index < length && !entry; index++) {
              const item = (annots as MupdfObject).get(index) as MupdfObject;
              if (item.isIndirect() && item.asIndirect() === wanted) entry = item;
            }
            if (!entry) failLink("no-such-annot", missing);
            if (at(entry, "Subtype")?.asName() !== "Link") failLink("not-link", `link-set: annotation ${wanted} is not a link`);
            const pair = linkPairs(owner).find(candidate => candidate.annotId === wanted);
            if (!pair) failLink("no-such-annot", missing);
            let uri: string | null = null;
            let targetPage = -1;
            if (hasTarget) {
              const target = request.target as { page?: unknown; uri?: unknown } | null;
              if (target && typeof target === "object" && target.page !== undefined) {
                try {
                  targetPage = checkPage(doc, target.page, "link-set");
                } catch (err) {
                  failLink("target", errorMessage(err));
                }
                uri = doc.formatLinkURI(fitDest(targetPage));
              } else if (target && typeof target === "object" && typeof target.uri === "string") {
                const scheme = schemeOf(target.uri);
                // P5: `setURI` has no scheme policy (javascript: -> /URI, file: -> /GoToR).
                if (!(LINK_SCHEMES as readonly string[]).includes(scheme)) {
                  failLink("scheme", `link-set: ${scheme || "a URI without a scheme"} is not a link this app opens`);
                }
                if (target.uri.length > MAX_URI_CHARS) {
                  failLink("scheme", `link-set: a link URI holds at most ${MAX_URI_CHARS} characters`);
                }
                uri = target.uri;
              } else {
                failLink("target", "link-set: target must be {page} or {uri}");
              }
            }
            const bounds = attempt(() => [...pair.link.getBounds()] as number[], [] as number[]);
            const rectChanged = rect !== null
              && !(bounds.length === 4 && rect.every((value, index) => Math.abs(value - bounds[index]) <= 0.01));
            let targetChanged = false;
            if (uri !== null) {
              const resolved = attempt(() => doc.resolveLink(pair.link), -1);
              targetChanged = targetPage >= 0
                ? resolved !== targetPage
                : !(resolved < 0 && attempt(() => pair.link.getURI(), "") === uri);
            }
            if (!rectChanged && !targetChanged) {
              noop = true;
            } else {
              const link = pair.link;
              const dict = pair.entry as MupdfObject;
              mutate(session, doc, "edit link", () => {
                // P1: setBounds takes raw /Rect space; P2: /Dest shadows /A, so drop it first.
                if (rectChanged) link.setBounds(mupdf.Rect.transform(rect as Rect, mupdf.Matrix.invert(owner.getTransform())));
                if (targetChanged) {
                  dict.delete("Dest");
                  link.setURI(uri as string);
                }
              });
            }
          } finally {
            owner.destroy();
          }
          if (noop) return { reply: { id: request.id, ok: true, noop: true, ...journalState(session) } };
          return { reply: { id: request.id, ok: true, ...editState(session) } };
        }
        case "insert-pages": {
          const doc = requireMutable(session, "insert-pages");
          const pdf = doc.asPDF();
          if (!pdf) throw new Error("insert-pages: the open document is not a PDF");
          const at = checkAt(doc, request.at, "insert-pages");
          if (typeof request.name !== "string" || !request.name) {
            throw new Error("insert-pages: missing source name");
          }
          const bytes = request.pdf;
          if (!(bytes instanceof ArrayBuffer)) throw new Error("insert-pages: no source bytes");
          if (bytes.byteLength < 5) throw new Error("insert-pages: the source is not a PDF (too few bytes)");
          if (bytes.byteLength > MAX_PDF_BYTES) {
            throw new Error(`insert-pages: the source exceeds the ${MAX_PDF_BYTES} byte ceiling`);
          }
          let source: MupdfDocument;
          try {
            source = mupdf.Document.openDocument(bytes, "application/pdf");
          } catch (err) {
            throw new Error(`insert-pages: the source could not be read (${errorMessage(err)})`);
          }
          try {
            const sourcePdf = source.asPDF();
            if (!sourcePdf) throw new Error("insert-pages: the source is not a PDF");
            sourcePdf.disableJS();
            // The same read-only rule the app applies to its own files: a source that
            // needs a password, or is encrypted at all, is not merged (its permissions
            // and /Encrypt are not carried into an edited document).
            const sourceInfo = detect(source);
            if (sourceInfo.needsPassword) throw new Error("insert-pages: the source needs a password");
            if (sourceInfo.encrypted) throw new Error("insert-pages: an encrypted source cannot be inserted");
            const count = source.countPages();
            if (count < 1) throw new Error("insert-pages: the source has no pages");
            let pages: number[];
            if (Array.isArray(request.pages)) {
              pages = checkPages(request.pages, count, "insert-pages");
            } else if (typeof request.range === "string") {
              // The dialog's text is parsed here, where the source count is known;
              // `parsePageRange` is the same parser the print dialog uses.
              const parsed = parsePageRange(request.range, count);
              if (!parsed) throw new Error("insert-pages: pages must look like \"1-3, 5\"");
              pages = checkPages(parsed, count, "insert-pages");
            } else {
              pages = Array.from({ length: count }, (_, index) => index);
            }
            // Every refusal above happens before this: a bad source never dirties the target.
            mutate(session, doc, "insert pages", () => graftPages(pdf, at, sourcePdf, pages));
            return {
              reply: {
                id: request.id, ok: true, inserted: pages.length,
                fieldNames: fieldNames(doc), ...editState(session),
              },
            };
          } finally {
            closeDocument(source);
          }
        }
        case "extract-pages": {
          const doc = session.doc;
          if (!doc) throw new Error("extract-pages: no document open");
          if (session.locked) throw new Error("extract-pages: document is encrypted, a password is required");
          const docPdf = doc.asPDF();
          if (!docPdf) throw new Error("extract-pages: the open document is not a PDF");
          // A copy is not a mutation: the journal, dirty state and revision stay put,
          // which is also why extracting from a signed or certified file is allowed.
          const pages = checkPages(request.pages, doc.countPages(), "extract-pages");
          const out = new mupdf.PDFDocument();
          try {
            const outPdf = out.asPDF();
            if (!outPdf) failReason("invalid", "extract-pages: the new document is not a PDF");
            graftPages(outPdf, 0, docPdf, pages);
            const buffer = outPdf.saveToBuffer(SAVE_OPTIONS);
            let bytes: ArrayBuffer;
            try {
              const view = buffer.asUint8Array();
              if (view.byteLength > MAX_SAVE_BYTES) {
                failReason("too-large", `extract-pages: the extract would exceed the ${MAX_SAVE_BYTES} byte save ceiling`);
              }
              bytes = copyBytes(view);
            } finally {
              buffer.destroy();
            }
            return {
              reply: { id: request.id, ok: true, bytes, pageCount: pages.length },
              transfer: [bytes],
            };
          } finally {
            closeDocument(out);
          }
        }
        case "pdf-from-images": {
          const images = request.images;
          if (!Array.isArray(images) || images.length < 1 || images.length > MAX_IMAGES) {
            throw new Error(`pdf-from-images: images must be 1..${MAX_IMAGES} files`);
          }
          let total = 0;
          for (const [index, image] of images.entries()) {
            if (!image || typeof image.name !== "string" || !image.name || image.name.length > 255) {
              throw new Error(`pdf-from-images: image ${index} needs a name of at most 255 characters`);
            }
            if (!(image.bytes instanceof ArrayBuffer) || image.bytes.byteLength < 1) {
              throw new Error(`pdf-from-images: image ${index} (${image.name}) has no bytes`);
            }
            if (image.bytes.byteLength > MAX_IMAGE_BYTES) {
              throw new Error(`pdf-from-images: ${image.name} is over the ${MAX_IMAGE_BYTES} byte image ceiling`);
            }
            total += image.bytes.byteLength;
          }
          if (total > MAX_IMAGES_TOTAL_BYTES) {
            throw new Error(`pdf-from-images: the images total more than ${MAX_IMAGES_TOTAL_BYTES} bytes`);
          }
          // Nothing at all touches the session: this builds and saves a new document.
          const { bytes, pageCount } = pdfFromImages(images);
          return { reply: { id: request.id, ok: true, bytes, pageCount }, transfer: [bytes] };
        }
        case "text-all": {
          const doc = session.doc;
          if (!doc) throw new Error("text-all: no document open");
          if (session.locked) throw new Error("text-all: document is encrypted, a password is required");
          // The copy bit is honoured: a document that forbids copying exports no text.
          if (!doc.hasPermission("copy")) failReason("read-only", "text-all: this document does not permit copying");
          const encoder = new TextEncoder();
          const parts: string[] = [];
          let bytes = 0;
          for (let pageIndex = 0; pageIndex < doc.countPages(); pageIndex++) {
            const page = doc.loadPage(pageIndex) as MupdfPage;
            try {
              const structured = page.toStructuredText("preserve-whitespace");
              try {
                const text = structured.asText().trim();
                bytes += encoder.encode(text).byteLength;
                parts.push(text);
              } finally {
                structured.destroy();
              }
            } finally {
              page.destroy();
            }
            if (bytes > MAX_SAVE_BYTES) {
              failReason("too-large", `text-all: the text exceeds the ${MAX_SAVE_BYTES} byte ceiling`);
            }
          }
          return { reply: { id: request.id, ok: true, text: parts.join("\f") } };
        }
        case "edit-targets": {
          // Read-only: the copy bit decides, exactly as `text-all` does. A signed
          // or no-edit document may still be inspected here; that grants no
          // mutation right, and a later writer still calls requireMutable.
          const doc = session.doc;
          if (!doc) throw new Error("edit-targets: no document open");
          if (session.locked) throw new Error("edit-targets: document is encrypted, a password is required");
          const page = checkPage(doc, request.page, "edit-targets");
          if (!doc.hasPermission("copy")) failReason("read-only", "edit-targets: this document does not permit copying");
          const scan = scanEditTargets(doc, page);
          editScanCache = { generation: session.generation, revision: session.revision, page, records: scan.records, paragraphs: scan.paragraphs };
          return {
            reply: {
              id: request.id, ok: true, page, generation: session.generation, revision: session.revision, items: scan.items,
              paragraphs: [...scan.paragraphs.values()].map(paragraph => paragraph.item),
              // P6 U3 §1.1/§9.1: user space (y up) → the page view space (y down) of this scan.
              transform: scan.transform,
            },
          };
        }
        case "edit-text": {
          // Permission first (default `edit`), then the handle, then every check that
          // can refuse without opening an operation.
          const doc = requireMutable(session, "edit-text");
          const page = checkPage(doc, request.page, "edit-text");
          let record: EditTargetRecord;
          try {
            record = resolveEditTarget(session, {
              generation: request.generation, revision: request.revision, page, targetId: request.targetId,
            });
          } catch (err) {
            if ((err as { reason?: string } | null)?.reason === "too-large") {
              failEditText("too-large", `edit-text: ${errorMessage(err)}`);
            }
            failEditText("stale-target", `edit-text: ${errorMessage(err)}`);
          }
          if (record.kind !== "text" || record.support !== "candidate") {
            failEditText(
              "not-candidate",
              `edit-text: that target cannot be edited${record.reason ? ` (${record.reason})` : ""}`,
              record.reason ? { reason: record.reason } : undefined,
            );
          }
          // P6 U2 §2.1: text defaults to the run's own when a move or size is given.
          const geometry = request.move !== undefined || request.size !== undefined;
          const next = request.text === undefined && geometry ? record.text : request.text;
          if (typeof next !== "string") failEditText("unsupported-encoding", "edit-text: text must be a string");
          const plan = editPlanText(doc, record, next, null, undefined, { move: request.move, size: request.size });
          const pdf = doc.asPDF();
          if (!pdf) failReason("invalid", "edit-text: the open document is not a PDF");
          // §9.15: nothing to write is ok with no journal op, so a stray gesture never dirties.
          // print-edit D6: a glyph line compares its normalised text and names a moved rewrite.
          if ((record.glyphLine ? plan.next : next) === record.text && !plan.prefix && !plan.suffix && !plan.rewrite) {
            return { reply: { id: request.id, ok: true, page, shift: 0, ...editState(session) } };
          }
          mutate(session, doc, geometry ? "move text" : "edit text", () => {
            const pageView = doc.loadPage(page) as MupdfPage;
            try {
              editWriteText(pdf, pageView, plan);
            } finally {
              pageView.destroy();
            }
          });
          return { reply: { id: request.id, ok: true, page, shift: plan.shift, ...(record.kerned ? { notice: "kerning in this run was dropped" } : {}), ...editState(session) } };
        }
        case "edit-paragraph": {
          const doc = requireMutable(session, "edit-paragraph");
          const page = checkPage(doc, request.page, "edit-paragraph");
          // §9.16: the id names a paragraph of this exact revision, else the draft is stale.
          if (request.generation !== session.generation || request.revision !== session.revision) {
            failEditText("stale-target", "edit-paragraph: the document changed since the scan");
          }
          const paragraph = editParagraphsFor(session, page).get(request.paragraphId);
          if (!paragraph) failEditText("stale-target", `edit-paragraph: no paragraph ${String(request.paragraphId)} on page ${page}`);
          const others = [...editScanFor(session, page).values()];
          const view = doc.loadPage(page) as MupdfPage;
          let plan: ReturnType<typeof editPlanParagraph>;
          try {
            plan = editPlanParagraph(view, paragraph, others, request);
          } finally {
            view.destroy();
          }
          // §9.15: nothing to write is ok with no journal op.
          if (plan.noop) return { reply: { id: request.id, ok: true, page, lines: plan.lines.length, ...editState(session) } };
          const pdf = doc.asPDF();
          if (!pdf) failReason("invalid", "edit-paragraph: the open document is not a PDF");
          mutate(session, doc, "edit paragraph", () => {
            const pageView = doc.loadPage(page) as MupdfPage;
            try {
              editWriteParagraph(pdf, pageView, plan);
            } finally {
              pageView.destroy();
            }
          });
          return { reply: { id: request.id, ok: true, page, lines: plan.lines.length, ...(paragraph.item.kerned ? { notice: "kerning in this paragraph was dropped" } : {}), ...editState(session) } };
        }
        case "text-add": {
          const doc = requireMutable(session, "text-add");
          const pageIndex = checkPage(doc, request.page, "text-add");
          if (!BASE14_FONTS.includes(request.font) || !Number.isFinite(request.size) || request.size < 4 || request.size > 400
            || typeof request.text !== "string" || request.text.length < 1 || request.text.length > MAX_TEXT_ADD_CHARS
            || !Array.isArray(request.point) || request.point.length !== 2 || request.point.some(value => !Number.isFinite(value))
            || !Array.isArray(request.color) || request.color.length !== 3 || request.color.some(value => !Number.isFinite(value) || value < 0 || value > 1)) {
            failReason("invalid", "text-add: invalid font, size, point, colour, or text");
          }
          const lines = request.text.split("\n");
          if (lines.length > MAX_TEXT_ADD_LINES || request.text.includes("\r") || lines.some(line => [...line].some(ch => {
            const code = ch.codePointAt(0) ?? 0;
            return code < 0x20;
          }))) failReason("invalid", `text-add: text must have at most ${MAX_TEXT_ADD_LINES} lines and no control characters`);
          const missing = [...new Set([...request.text].filter(ch => ch !== "\n" && winAnsiByte(ch.codePointAt(0) ?? 0) < 0))];
          if (missing.length) failEditText("unsupported-encoding", "text-add: unsupported WinAnsi characters", { missing: missing.slice(0, 20) });
          const page = doc.loadPage(pageIndex) as MupdfPage;
          let pointPdf: [number, number];
          let fontName = "";
          let stream: Uint8Array<ArrayBufferLike> = new Uint8Array();
          try {
            const bounds = page.getBounds();
            const [x, y] = request.point;
            if (x < bounds[0] || x > bounds[2] || y < bounds[1] || y > bounds[3]) failReason("invalid", "text-add: point is outside the page box");
            const pageDict = page.getObject() as MupdfObject;
            const rotate = attempt(() => pageDict.getInheritable("Rotate") as MupdfObject, null);
            const unit = at(pageDict, "UserUnit");
            if ((rotate && Math.trunc(rotate.asNumber()) % 360 !== 0) || (unit && unit.asNumber() !== 1)) failReason("unsupported-transform", "text-add: rotated or scaled pages are not supported");
            const transform = [...page.getTransform()];
            const det = transform[0] * transform[3] - transform[1] * transform[2];
            const px = x - transform[4], py = y - transform[5];
            pointPdf = [(transform[3] * px - transform[2] * py) / det, (-transform[1] * px + transform[0] * py) / det];
            const scan = scanEditTargets(doc, pageIndex);
            if (!scan.balancedContent) failReason("unbalanced-content", "text-add: existing page content has unbalanced graphics state");
            const pdf = doc.asPDF();
            if (!pdf) failReason("invalid", "text-add: the open document is not a PDF");
            const resources = at(pageDict, "Resources") ?? attempt(() => pageDict.getInheritable("Resources") as MupdfObject, null);
            const fontSource = resources ? at(resources, "Font") : null;
            let index = 1;
            while (at(fontSource, `PDF4AllF${index}`)) index++;
            fontName = `PDF4AllF${index}`;
            const encoder = new TextEncoder();
            // Text state survives across elements (q does not reset it): pin every parameter.
            const chunks: Uint8Array[] = [encoder.encode(`q BT /${fontName} ${request.size} Tf 0 Tc 0 Tw 100 Tz 0 TL 0 Ts 0 Tr 1 0 0 1 ${round2(pointPdf[0])} ${round2(pointPdf[1])} Tm ${request.color.map(round2).join(" ")} rg `)];
            for (let i = 0; i < lines.length; i++) {
              if (i) chunks.push(encoder.encode(`0 -${round2(request.size * 1.2)} Td `));
              chunks.push(encoder.encode("("), winAnsiBytes(lines[i]), encoder.encode(") Tj "));
            }
            chunks.push(encoder.encode("ET Q"));
            stream = concatBytes(...chunks);
            mutate(session, doc, "add text", () => {
              const resourceCopy = pdf.newDictionary();
              resources?.forEach((value, key) => { if (typeof key === "string") resourceCopy.put(key, value); });
              const fontCopy = pdf.newDictionary();
              fontSource?.forEach((value, key) => { if (typeof key === "string") fontCopy.put(key, value); });
              fontCopy.put(fontName, addBase14Font(pdf, request.font));
              resourceCopy.put("Font", fontCopy);
              pageDict.put("Resources", resourceCopy);
              const contents = at(pageDict, "Contents");
              const contentArray = pdf.newArray();
              if (contents?.isArray()) for (let i = 0; i < contents.length; i++) contentArray.push(contents.get(i));
              else if (contents && !contents.isNull()) contentArray.push(contents);
              contentArray.push(pdf.addStream(stream, {}));
              pageDict.put("Contents", contentArray);
            });
          } finally { page.destroy(); }
          // P6 U3 §9.5: the add has landed; a failed lookup scan must not turn it into a refusal.
          const added = attempt(() => scanEditTargets(doc, pageIndex).items.filter(item => item.kind === "text" && item.text === lines[0]).at(-1), undefined);
          return { reply: { id: request.id, ok: true, page: pageIndex, targetId: added?.targetId ?? "", bounds: added?.bounds ?? null, ...editState(session) } };
        }
        case "edit-replace-preview": {
          // Read-only, like edit-targets: the copy bit decides, no operation is opened.
          const doc = session.doc;
          if (!doc) throw new Error("edit-replace-preview: no document open");
          if (session.locked) throw new Error("edit-replace-preview: document is encrypted, a password is required");
          const page = checkPage(doc, request.page, "edit-replace-preview");
          if (!doc.hasPermission("copy")) failReason("read-only", "edit-replace-preview: this document does not permit copying");
          const { find, replace } = checkReplaceArgs(request.find, request.replace);
          const targetId = request.targetId;
          if (targetId !== undefined && (typeof targetId !== "string" || !targetId)) {
            failEditReplace("invalid", "edit-replace-preview: targetId must be a non-empty string");
          }
          const preview = buildReplacePreview(session, doc, page, find, replace, targetId);
          return {
            reply: {
              id: request.id, ok: true, page, generation: session.generation, revision: session.revision,
              find, replace, matches: preview.matches, rejected: preview.rejected, previewToken: preview.token,
            },
          };
        }
        case "edit-replace-apply": {
          const doc = requireMutable(session, "edit-replace-apply");
          const page = checkPage(doc, request.page, "edit-replace-apply");
          const { find, replace } = checkReplaceArgs(request.find, request.replace);
          if (typeof request.previewToken !== "string" || !request.previewToken) {
            failEditReplace("invalid", "edit-replace-apply: previewToken must be a non-empty string");
          }
          const ids = request.targetIds;
          if (!Array.isArray(ids) || ids.length < 1 || ids.length > MAX_EDIT_TARGETS
            || ids.some(id => typeof id !== "string" || !id)) {
            failEditReplace("invalid", `edit-replace-apply: targetIds must be 1..${MAX_EDIT_TARGETS} non-empty strings`);
          }
          const requested = ids as string[];
          const targetId = request.targetId;
          if (targetId !== undefined && (typeof targetId !== "string" || !targetId)) {
            failEditReplace("invalid", "edit-replace-apply: targetId must be a non-empty string");
          }
          // Recompute the preview from this session's own scan, in the same scope: the
          // token, not the request, is what proves the runs are still the ones the user saw.
          const preview = buildReplacePreview(session, doc, page, find, replace, targetId);
          if (preview.token !== request.previewToken) {
            failEditReplace("stale-preview", "edit-replace-apply: the page changed since the preview");
          }
          for (const id of requested) {
            if (!preview.plans.has(id)) {
              failEditReplace("stale-target", `edit-replace-apply: ${id} is not in this preview`, { targetId: id });
            }
          }
          const plans = requested.map(id => preview.plans.get(id) as EditTextPlan);
          const pdf = doc.asPDF();
          if (!pdf) failReason("invalid", "edit-replace-apply: the open document is not a PDF");
          mutate(session, doc, "replace text", () => {
            const pageView = doc.loadPage(page) as MupdfPage;
            try {
              // One writer, one operation: any failure here abandons the whole set.
              editWritePlans(pdf, pageView, plans, new Set(), (message, targetId) => failEditReplace("write-failed", `edit-replace-apply: ${message}`, { targetId }));
            } finally {
              pageView.destroy();
            }
          });
          return { reply: { id: request.id, ok: true, page, replaced: plans.length, ...editState(session) } };
        }
        case "edit-object": {
          const doc = requireMutable(session, "edit-object");
          const page = checkPage(doc, request.page, "edit-object");
          let record: EditTargetRecord;
          try {
            record = resolveEditTarget(session, {
              generation: request.generation, revision: request.revision, page, targetId: request.targetId,
            });
          } catch (err) {
            if ((err as { reason?: string } | null)?.reason === "too-large") {
              failEditObject("too-large", `edit-object: ${errorMessage(err)}`);
            }
            failEditObject("stale-target", `edit-object: ${errorMessage(err)}`);
          }
          if (record.kind === "text" || record.support !== "candidate") {
            failEditObject(
              "not-candidate",
              `edit-object: that target cannot be edited${record.reason ? ` (${record.reason})` : ""}`,
              record.reason ? { reason: record.reason } : undefined,
            );
          }
          const action = request.action;
          const ops = record.objectOps ?? [];
          if (!ops.includes(action)) {
            // The placement's own reason is the honest token when the action is simply
            // unavailable here (rotated/clipped, a mask, a layer, a Form replace).
            failEditObject(
              (record.reason as EditObjectDetail | undefined) ?? "not-candidate",
              `edit-object: ${String(action)} is not available for that placement${record.reason ? ` (${record.reason})` : ""}`,
              record.reason ? { reason: record.reason } : undefined,
            );
          }
          let box: number[] | null = null;
          // P6 U6 §1.2: move/resize/transform/crop are one affine on the `cm` and the clip.
          let write: EditPlacementWrite | null = null;
          if (action === "move" || action === "resize" || action === "duplicate" || action === "transform" || action === "crop") {
            const pageView = doc.loadPage(page) as MupdfPage;
            let pageClip: number[];
            try {
              pageClip = editPageClipBox(pageView.getObject() as MupdfObject);
            } finally {
              pageView.destroy();
            }
            const current = record.placement;
            if (!current) failEditObject("not-candidate", `edit-object: that placement cannot be edited`);
            if (action === "duplicate") {
              box = editObjectBox(request.box, pageClip);
              // P6 U6 §9.9: the copy's uncropped box stays within the ceiling.
              editCheckFull(editPlacementAfter(current, editRectAffine(current.visible, box), box).full);
            } else {
              // P6 U3 §1.2/§9.2, P6 U6 §9.10: the same computed `cm` and clip is a no-op: no op,
              // no dirty. P6 U5 §1.1: a duplicate is never a no-op; nor is a transform.
              write = editPlanPlacement(current, action, request.box, request.transform, pageClip);
              if (write.noop) return { reply: { id: request.id, ok: true, page, noop: true, ...editState(session) } };
            }
          }
          // P6 U5 §9.4: the next scan must still prove the page, copy included.
          if (action === "duplicate" && editScanFor(session, page).size + 1 > MAX_EDIT_TARGETS) {
            failEditObject("too-large", `edit-object: a duplicate would put more than ${MAX_EDIT_TARGETS} targets on this page`);
          }
          let decoded: MupdfImage | null = null;
          if (action === "replace") {
            try {
              decoded = decodeStampImage(request.image, "edit-object");
            } catch (err) {
              const message = errorMessage(err);
              failEditObject(message.includes("ceiling") ? "too-large" : "invalid-image", message);
            }
          }
          const pdf = doc.asPDF();
          if (!pdf) failReason("invalid", "edit-object: the open document is not a PDF");
          const elementIndex = Number(/^c(\d+)$/.exec(record.path)?.[1] ?? NaN);
          if (!Number.isInteger(elementIndex)) failEditObject("not-candidate", "edit-object: only top-level page content is editable");
          const placement = record.placement;
          const resourceName = record.objectName ?? null;
          try {
            mutate(session, doc, "edit object", () => {
              const pageView = doc.loadPage(page) as MupdfPage;
              try {
                const pageObject = pageView.getObject() as MupdfObject;
                const contents = at(pageObject, "Contents");
                const element = contents?.isArray()
                  ? contents.get(elementIndex) as MupdfObject
                  : elementIndex === 0 ? contents : null;
                if (!element || !element.isStream()) {
                  failReason("invalid", "edit-object: the content stream is no longer where it was");
                }
                // U1b: privatise this placement first, in the same operation; the record's
                // spans hold because every copy is proven byte-identical to its source.
                const stream = record.cow ? editPrivatise(pdf, pageObject, contents as MupdfObject, element, elementIndex, record) : element;
                const bytes = editReadStream(stream, "edit-object");
                if (action === "delete") {
                  if (!placement) failEditObject("not-candidate", "edit-object: that placement cannot be deleted");
                  stream.writeStream(editSplice(bytes, [
                    { start: placement.qStart, end: placement.qEnd, replacement: new Uint8Array(0) },
                  ]));
                  return;
                }
                // P6 U5 §1.1: a copy of the block right after its own `Q`.
                if (action === "duplicate") {
                  if (!placement || !box) failEditObject("not-candidate", "edit-object: that placement cannot be duplicated");
                  stream.writeStream(editSplice(bytes, [editDuplicateEdit(bytes, placement, box)]));
                  return;
                }
                // Move, resize, transform and crop are the same write: the planned `cm`
                // (against the rect the placement's own `cm` maps) and clip spans.
                if (write) {
                  if (!placement) failEditObject("not-candidate", "edit-object: that placement cannot be moved");
                  stream.writeStream(editSplice(bytes, editPlacementSpans(placement, write.cm, write.clipText)));
                  return;
                }
                // Replace (§7.4): always a fresh resource name, only this `Do` repointed.
                if (!resourceName || !decoded) failEditObject("not-candidate", "edit-object: that placement cannot be replaced");
                const xobjects = at(at(pageObject, "Resources"), "XObject");
                if (!xobjects) failReason("invalid", "edit-object: the page has no XObject resources");
                let name = "";
                for (let suffix = 1; ; suffix++) {
                  name = `${resourceName}r${suffix}`;
                  if (!at(xobjects, name)) break;
                }
                // Not `pdf.addImage`: mupdf caches added images by digest, and after an undo
                // that cache still names the freed object, so replace → undo → replace would
                // save a dangling reference. A scratch document has no history to go stale.
                const scratch = new mupdf.PDFDocument();
                try {
                  xobjects.put(name, pdf.newGraftMap().graftObject(scratch.addImage(decoded)));
                } finally {
                  scratch.destroy();
                }
                const edits = [{
                  start: record.operandStart, end: record.operandEnd,
                  replacement: new TextEncoder().encode(`/${name}`),
                }];
                // A simple placement keeps the old box: fit the new aspect inside it,
                // anchored at the box's bottom-left, only when it differs by over 1%.
                // P6 U6 §9.7: only an unturned, unclipped placement is refitted; any other keeps
                // its `cm` and clip (the picture stretches, as the pre-U6 non-simple rule did).
                const upright = placement && !placement.clip && !placement.orient.swap && placement.orient.sx === 1 && placement.orient.sy === 1;
                if (placement && upright) {
                  const [x0, y0, x1, y1] = placement.box;
                  const oldWidth = x1 - x0;
                  const oldHeight = y1 - y0;
                  const pixelsWide = decoded.getWidth();
                  const pixelsHigh = decoded.getHeight();
                  if (pixelsWide > 0 && pixelsHigh > 0) {
                    const oldAspect = oldWidth / oldHeight;
                    const aspect = pixelsWide / pixelsHigh;
                    if (Math.abs(aspect - oldAspect) > oldAspect * 0.01) {
                      const width = aspect > oldAspect ? oldWidth : oldHeight * aspect;
                      const height = aspect > oldAspect ? oldWidth / aspect : oldHeight;
                      edits.push({
                        start: placement.cmStart, end: placement.cmEnd,
                        replacement: new TextEncoder().encode(editPlacementMatrix([x0, y0, x0 + width, y0 + height], placement.rect)),
                      });
                    }
                  }
                }
                stream.writeStream(editSplice(bytes, edits));
                // The old entry stays (§7.4/7.5): this XObject dict may be shared with other
                // pages (an indirect /Resources, or the U1b copy of inherited resources shares
                // its subdicts), so removing it could blank another page's image. Ceiling: the
                // old image bytes stay in the saved file, like delete (not redaction).
              } finally {
                pageView.destroy();
              }
            });
          } finally {
            if (decoded) decoded.destroy();
          }
          return { reply: { id: request.id, ok: true, page, ...editState(session) } };
        }
        case "edit-objects": {
          // P6 U5 §1.2/§9.2–§9.4: every check before any write; one op; all-or-nothing.
          const doc = requireMutable(session, "edit-objects");
          const page = checkPage(doc, request.page, "edit-objects");
          const items = request.items as unknown;
          if (Array.isArray(items) && items.length > MAX_EDIT_TARGETS) {
            failEditObject("too-large", `edit-objects: more than ${MAX_EDIT_TARGETS} items`);
          }
          if (!Array.isArray(items) || items.some(item => !item || typeof item !== "object"
            || typeof item.targetId !== "string" || !item.targetId
            || !["move", "delete", "duplicate", "resize", "transform"].includes(item.action))) {
            failEditObject("invalid-selection", "edit-objects: items must be { targetId, action: move|resize|transform|delete|duplicate, box?, transform? }");
          }
          const list = items as EditObjectsRequest["items"];
          if (list.length < 2) failEditObject("invalid-selection", "edit-objects: at least two items (one item: use edit-object)");
          const seen = new Set<string>();
          for (const { targetId } of list) {
            if (seen.has(targetId)) failEditObject("invalid-selection", `edit-objects: ${targetId} is listed twice`, { targetId });
            seen.add(targetId);
          }
          const clipView = doc.loadPage(page) as MupdfPage;
          let clip: number[];
          try {
            clip = editPageClipBox(clipView.getObject() as MupdfObject);
          } finally {
            clipView.destroy();
          }
          type Write = { action: EditObjectsRequest["items"][number]["action"]; record: EditTargetRecord; placement: EditPlacement; box: number[] | null; write: EditPlacementWrite | null };
          const byElement = new Map<number, Write[]>();
          let count = 0;
          let duplicates = 0;
          for (const item of list) {
            const targetId = item.targetId;
            let record: EditTargetRecord;
            try {
              record = resolveEditTarget(session, { generation: request.generation, revision: request.revision, page, targetId });
            } catch (err) {
              const detail = (err as { reason?: string } | null)?.reason === "too-large" ? "too-large" : "stale-target";
              failEditObject(detail, `edit-objects: ${errorMessage(err)}`, { targetId });
            }
            const reason = record.reason ? { reason: record.reason } : {};
            if (record.kind === "text" || record.support !== "candidate") {
              failEditObject("not-candidate", `edit-objects: ${targetId} cannot be edited${record.reason ? ` (${record.reason})` : ""}`, { targetId, ...reason });
            }
            if (!(record.objectOps ?? []).includes(item.action)) {
              failEditObject(
                (record.reason as EditObjectDetail | undefined) ?? "not-candidate",
                `edit-objects: ${item.action} is not available for ${targetId}${record.reason ? ` (${record.reason})` : ""}`,
                { targetId, ...reason },
              );
            }
            const placement = record.placement;
            const elementIndex = Number(/^c(\d+)$/.exec(record.path)?.[1] ?? NaN);
            if (!placement || !Number.isInteger(elementIndex)) {
              failEditObject("not-candidate", `edit-objects: only top-level simple placements are editable (${targetId})`, { targetId });
            }
            let box: number[] | null = null;
            let write: EditPlacementWrite | null = null;
            if (item.action !== "delete") {
              try {
                if (item.action === "duplicate") {
                  box = editObjectBox(item.box, clip);
                  editCheckFull(editPlacementAfter(placement, editRectAffine(placement.visible, box), box).full);
                } else {
                  // P6 U6 §1.3/§9.10: resize items are written (and dropped as no-ops) like move.
                  write = editPlanPlacement(placement, item.action, item.box, item.transform, clip);
                }
              } catch (err) {
                const detail = (err as { detail?: EditObjectDetail }).detail ?? "out-of-bounds";
                failEditObject(detail, `edit-objects: ${errorMessage(err)} (${targetId})`, { targetId });
              }
              if (write?.noop) continue;
              if (item.action === "duplicate") duplicates++;
            }
            const writes = byElement.get(elementIndex) ?? [];
            writes.push({ action: item.action, record, placement, box, write });
            byElement.set(elementIndex, writes);
            count++;
          }
          if (!count) return { reply: { id: request.id, ok: true, page, noop: true, ...editState(session) } };
          if (duplicates && editScanFor(session, page).size + duplicates > MAX_EDIT_TARGETS) {
            failEditObject("too-large", `edit-objects: the duplicates would put more than ${MAX_EDIT_TARGETS} targets on this page`);
          }
          const pdf = doc.asPDF();
          if (!pdf) failReason("invalid", "edit-objects: the open document is not a PDF");
          mutate(session, doc, "edit objects", () => {
            const pageView = doc.loadPage(page) as MupdfPage;
            try {
              const pageObject = pageView.getObject() as MupdfObject;
              for (const [elementIndex, writes] of byElement) {
                const cow = writes.find(write => write.record.cow)?.record;
                const contents = at(pageObject, "Contents");
                const element = contents?.isArray()
                  ? contents.get(elementIndex) as MupdfObject
                  : elementIndex === 0 ? contents : null;
                if (!element || !element.isStream()) {
                  failReason("invalid", "edit-objects: the content stream is no longer where it was");
                }
                // U1b once per element: the copy is byte-identical, so every record's spans hold.
                const stream = cow ? editPrivatise(pdf, pageObject, contents as MupdfObject, element, elementIndex, cow) : element;
                const bytes = editReadStream(stream, "edit-objects");
                stream.writeStream(editSplice(bytes, writes.flatMap(({ action, placement, box, write }) =>
                  action === "delete" ? [{ start: placement.qStart, end: placement.qEnd, replacement: new Uint8Array(0) }]
                  : write ? editPlacementSpans(placement, write.cm, write.clipText)
                  : [editDuplicateEdit(bytes, placement, box as number[])])));
              }
            } finally {
              pageView.destroy();
            }
          });
          return { reply: { id: request.id, ok: true, page, count, ...editState(session) } };
        }
        case "clip-copy":
          return { reply: clipCopy(session, request) };
        case "clip-paste":
          return { reply: clipPaste(session, request) };
        case "edit-batch":
          return { reply: editBatch(session, request) };
        case "stamp": {
          const doc = requireMutable(session, "stamp");
          return stampPages(session, doc, request as StampRequest);
        }
        case "batch-file":
          return batchFile(request as BatchFileRequest);
        case "page-ids": {
          // Read-only and PDF-only: `[]` for anything else, and no `fail` paths (A2 §8.7).
          const pdf = session.doc?.asPDF() ?? null;
          const ids: number[] = [];
          if (pdf) {
            const count = session.doc?.countPages() ?? 0;
            for (let index = 0; index < count; index++) {
              const page = attempt(() => pdf.findPage(index) as MupdfObject, null);
              ids.push(page ? attempt(() => page.isIndirect() ? page.asIndirect() : -1, -1) : -1);
            }
          }
          return { reply: { id: request.id, ok: true, ids } };
        }
        case "labels": {
          const doc = session.doc;
          if (!doc) throw new Error("labels: no document open");
          if (session.locked) throw new Error("labels: document is encrypted, a password is required");
          return { reply: { id: request.id, ok: true, items: pageLabels(doc) } };
        }
        case "labels-set": {
          const doc = requireMutable(session, "labels-set");
          const pdf = doc.asPDF();
          if (!pdf) throw new Error("labels-set: the open document is not a PDF");
          const from = checkPage(doc, request.from, "labels-set");
          const style = request.style;
          if (typeof style !== "string" || !(LABEL_STYLES as readonly string[]).includes(style)) {
            throw new Error(`labels-set: style must be one of ${LABEL_STYLES.map(item => item || "\"\"").join(", ")}`);
          }
          const prefix = request.prefix ?? "";
          if (typeof prefix !== "string" || prefix.length > MAX_LABEL_PREFIX_CHARS) {
            throw new Error(`labels-set: prefix must be a string of at most ${MAX_LABEL_PREFIX_CHARS} characters`);
          }
          const start = request.start ?? 1;
          if (!Number.isInteger(start) || start < 1) {
            throw new Error("labels-set: start must be an integer of at least 1");
          }
          mutate(session, doc, "set page labels", () => {
            pdf.setPageLabels(from, style, prefix, start);
          });
          return { reply: { id: request.id, ok: true, ...editState(session), items: pageLabels(doc) } };
        }
        case "labels-clear": {
          const doc = requireMutable(session, "labels-clear");
          const pdf = doc.asPDF();
          if (!pdf) throw new Error("labels-clear: the open document is not a PDF");
          const from = checkPage(doc, request.from, "labels-clear");
          mutate(session, doc, "clear page labels", () => pdf.deletePageLabels(from));
          return { reply: { id: request.id, ok: true, ...editState(session), items: pageLabels(doc) } };
        }
        case "layers": {
          const doc = session.doc;
          if (!doc) throw new Error("layers: no document open");
          if (session.locked) throw new Error("layers: document is encrypted, a password is required");
          const pdf = doc.asPDF();
          if (!pdf) throw new Error("layers: the open document is not a PDF");
          return { reply: { id: request.id, ok: true, items: listLayers(pdf) } };
        }
        case "layer-set": {
          const doc = session.doc;
          if (!doc) throw new Error("layer-set: no document open");
          if (session.locked) throw new Error("layer-set: document is encrypted, a password is required");
          const pdf = doc.asPDF();
          if (!pdf) throw new Error("layer-set: the open document is not a PDF");
          const count = attempt(() => pdf.countLayers(), 0);
          const layer = request.layerId;
          if (!Number.isInteger(layer) || layer < 0 || layer >= count) {
            throw new Error(`layer-set: layer must be an integer in 0..${Math.max(0, count - 1)}`);
          }
          if (typeof request.visible !== "boolean") throw new Error("layer-set: visible must be true or false");
          // Measured: mupdf's `setLayerVisible` is NOT journaled and `saveToBuffer` does
          // not write /OCProperties, so this is a view-only toggle: no operation, no
          // revision bump, no dirty bit. `persisted:false` is what the panel and the
          // smoke say out loud (p2-decorate.md §Gate).
          pdf.setLayerVisible(layer, request.visible);
          return { reply: { id: request.id, ok: true, persisted: false, items: listLayers(pdf) } };
        }
        case "page-rotate": {
          const doc = requireMutable(session, "page-rotate");
          const page = checkPage(doc, request.page, "page-rotate");
          const by = request.by;
          if (by !== 90 && by !== -90) throw new Error("page-rotate: by must be 90 or -90");
          // The page's own rotation is the starting point: a page that came in at 270
          // rotated right is 0, not 90.
          const next = (pageRotation(doc, page) + by + 360) % 360;
          const pdf = doc.asPDF();
          if (!pdf) throw new Error("page-rotate: the open document is not a PDF");
          mutate(session, doc, "rotate page", () => {
            pdf.findPage(page).put("Rotate", pdf.newInteger(next));
          });
          return { reply: { id: request.id, ok: true, ...editState(session) } };
        }
        case "page-delete": {
          const doc = requireMutable(session, "page-delete");
          const page = checkPage(doc, request.page, "page-delete");
          // A document must keep a page; the renderer disables the button, this is the guard.
          if (doc.countPages() < 2) throw new Error("page-delete: a one-page document has nothing to delete");
          const pdf = doc.asPDF();
          if (!pdf) throw new Error("page-delete: the open document is not a PDF");
          // Every reference to the doomed page is read first: mupdf repoints a
          // destination at whichever page takes the index over, so after deletePage
          // the reference no longer names the removed page and the repair would miss it.
          const doomedLinks = linksToPage(doc, page);
          const doomedOutline = listOutline(doc).filter(item => item.page === page).map(item => item.id);
          mutate(session, doc, "delete page", () => {
            for (const doomed of doomedLinks) {
              const owner = doc.loadPage(doomed.page) as MupdfPage;
              try {
                const link = owner.getLinks()[doomed.index];
                if (link) owner.deleteLink(link);
              } finally {
                owner.destroy();
              }
            }
            clearOutlineDests(doc, doomedOutline);
            pdf.deletePage(page);
            prunePhantomFields(doc);
          });
          return { reply: { id: request.id, ok: true, ...editState(session) } };
        }
        case "page-move": {
          const doc = requireMutable(session, "page-move");
          const page = checkPage(doc, request.page, "page-move");
          const to = checkPage(doc, request.to, "page-move");
          if (to === page) throw new Error(`page-move: page ${page} is already there`);
          const pdf = doc.asPDF();
          if (!pdf) throw new Error("page-move: the open document is not a PDF");
          // rearrangePages takes the old index of each new position: splice is the permutation.
          const order = [...Array(doc.countPages()).keys()];
          order.splice(page, 1);
          order.splice(to, 0, page);
          mutate(session, doc, "move page", () => pdf.rearrangePages(order));
          return { reply: { id: request.id, ok: true, ...editState(session) } };
        }
        case "undo":
        case "redo": {
          const doc = requireMutable(session, request.op);
          const pdf = doc.asPDF();
          const can = attempt(() => request.op === "undo" ? !!pdf?.canUndo() : !!pdf?.canRedo(), false);
          if (!pdf || !can) {
            throw new Error(`${request.op}: there is nothing to ${request.op}`);
          }
          if (request.op === "undo") pdf.undo();
          else pdf.redo();
          session.revision += 1;
          // The journal step this reply moved over, so the renderer can say "Undid delete page"
          // and take the page-refresh path only when a page op was reverted.
          const { position, steps } = attempt(() => pdf.getJournal(), { position: 0, steps: [] as string[] });
          const step = request.op === "undo" ? steps[position] : steps[position - 1];
          return { reply: { id: request.id, ok: true, ...editState(session), ...(step ? { step } : {}) } };
        }
        case "saved": {
          if (!session.doc) failReason("no-document", "saved: no document open");
          // The write landed: this journal position is what is on disk now, and the
          // pending security change is part of the file too.
          session.savedPosition = journalPosition(session.doc);
          session.securityDirty = false;
          // Removing encryption is the one change that also drops the held password;
          // a new AES-256 password stays, because the next save must write it again.
          if (session.security?.mode === "none") {
            session.security = { mode: "none" };
            session.password = null;
          }
          return { reply: { id: request.id, ok: true, ...journalState(session) } };
        }
        case "security-set": {
          const current = session.doc;
          if (!current) throw new Error("security-set: no document open");
          if (session.locked) throw new Error("security-set: the document is locked");
          const info = detect(current, session.locked);
          if (info.signatures > 0 || info.certified) {
            throw new Error("security-set: signed or certified documents keep their security");
          }
          // Changing security is an owner act; an unencrypted document has no owner.
          if (info.encrypted && session.auth !== "owner") {
            throw new Error("security-set: open the document with its owner password to change security");
          }
          session.security = securitySetting(request as SecuritySetRequest);
          session.securityDirty = true;
          session.revision += 1;
          return {
            reply: { id: request.id, ok: true, ...journalState(session), security: securityState(session) },
          };
        }
        case "save": {
          const { bytes, saveMs } = serializeDocument(session);
          const { dirty, revision } = journalState(session);
          return { reply: { id: request.id, ok: true, bytes, saveMs, dirty, revision, security: securityState(session) }, transfer: [bytes] };
        }
        case "validate": {
          const bytes = request.pdf;
          request.pdf = null; // the reply hands the same buffer back, transferred
          if (!bytes) failReason("no-document", "validate: no bytes");
          const { pageCount } = validateDocument(session, bytes);
          const { dirty, revision } = journalState(session);
          return { reply: { id: request.id, ok: true, pageCount, bytes, dirty, revision }, transfer: [bytes] };
        }
        default:
          throw new Error(`unknown op: ${String((request as { op?: unknown })?.op)}`);
      }
    } catch (err) {
      const tagged = err as { reason?: string; detail?: string; extra?: Record<string, unknown> } | null;
      return {
        reply: {
          id: request?.id,
          ok: false,
          error: err instanceof Error ? err.message : String(err),
          ...(tagged?.reason ? { reason: tagged.reason } : {}),
          // `edit-text` refusals carry one fixed detail token, plus its numbers.
          ...(tagged?.detail ? { detail: tagged.detail } : {}),
          ...(tagged?.extra ?? {}),
          // A failed mutation may have dropped the redo branch before it threw
          // (abandon-with-write), so the renderer needs the current journal state
          // to grey Redo out; `journalState` is attempt-guarded and never throws.
          ...journalState(session),
        },
      };
    }
  }

  return {
    openDocument,
    closeDocument,
    pageSizes,
    detect,
    renderPage,
    searchDocument,
    serializeDocument,
    validateDocument,
    listAnnotations,
    journalState,
    journalPosition,
    parseLines,
    dispatch,
    resolveEditTarget,
    createSession: (): Session => ({
      doc: null, name: null, sourceBytes: 0, locked: false, auth: "none", password: null, lockedBytes: null, signedSource: null,
      security: null, securityDirty: false, revision: 0, savedPosition: 0, generation: 0, clipboard: null, batch: false,
      compare: null,
    }),
    maxPdfBytes: MAX_PDF_BYTES,
    maxSaveBytes: MAX_SAVE_BYTES,
    maxNoteChars: MAX_NOTE_CHARS,
    noteSize: NOTE_SIZE,
  };
}

export type Engine = ReturnType<typeof createEngine>;

/** Compare §9.4: stext JSON blocks→lines as `{text, bbox:[x0,y0,x1,y1]}`, y down, page pt. */
export function stextLines(json: string): CompareLine[] {
  const lines: CompareLine[] = [];
  let data: { blocks?: unknown };
  try {
    data = JSON.parse(json) as { blocks?: unknown };
  } catch {
    return lines;
  }
  if (!Array.isArray(data.blocks)) return lines;
  for (const block of data.blocks as { type?: string; lines?: unknown }[]) {
    if (!block || block.type !== "text" || !Array.isArray(block.lines)) continue;
    for (const line of block.lines as { text?: unknown; bbox?: { x?: unknown; y?: unknown; w?: unknown; h?: unknown } }[]) {
      if (!line || typeof line.text !== "string") continue;
      const box = line.bbox ?? {};
      const x = num(box.x), y = num(box.y);
      lines.push({ text: line.text, bbox: [x, y, x + num(box.w), y + num(box.h)] });
    }
  }
  return lines;
}

/**
 * Compare D2: line-level LCS. `moved` is always 0 (ponytail: no move detection; a moved line is
 * removed + added). O(n·m) table, n, m ≤ MAX_COMPARE_LINES (ponytail: Hirschberg/Myers if that bites).
 */
export function diffLines(a: CompareLine[], b: CompareLine[]) {
  const ka = a.map(line => normaliseLine(line.text));
  const kb = b.map(line => normaliseLine(line.text));
  const n = ka.length, m = kb.length, w = m + 1;
  const table = new Uint16Array((n + 1) * w);
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      table[i * w + j] = ka[i] === kb[j] ? table[(i + 1) * w + j + 1] + 1 : Math.max(table[(i + 1) * w + j], table[i * w + j + 1]);
    }
  }
  const removed: CompareLine[] = [];
  const added: CompareLine[] = [];
  let same = 0, i = 0, j = 0;
  while (i < n && j < m) {
    if (ka[i] === kb[j]) { same++; i++; j++; }
    else if (table[(i + 1) * w + j] >= table[i * w + j + 1]) removed.push(a[i++]);
    else added.push(b[j++]);
  }
  while (i < n) removed.push(a[i++]);
  while (j < m) added.push(b[j++]);
  return { same, removed, added, moved: 0 };
}

export function parseLines(json: string): Line[] {
  const lines: Line[] = [];
  let data: { blocks?: unknown };
  try {
    data = JSON.parse(json) as { blocks?: unknown };
  } catch {
    return lines;
  }
  if (!Array.isArray(data.blocks)) return lines;
  for (const block of data.blocks) {
    if (!block || typeof block !== "object") continue;
    const rec = block as { type?: string; lines?: unknown };
    if (rec.type !== "text" || !Array.isArray(rec.lines)) continue;
    for (const line of rec.lines) {
      if (!line || typeof line !== "object") continue;
      const item = line as {
        text?: unknown;
        y?: unknown;
        bbox?: { x?: unknown; y?: unknown; w?: unknown; h?: unknown };
        font?: { size?: unknown; family?: unknown; weight?: unknown; style?: unknown };
      };
      if (typeof item.text !== "string") continue;
      const bbox = item.bbox ?? {};
      const font = item.font ?? {};
      lines.push({
        text: item.text,
        x: num(bbox.x),
        y: num(item.y),
        w: num(bbox.w),
        h: num(bbox.h),
        size: num(font.size, 12),
        family: str(font.family, "sans-serif"),
        weight: str(font.weight, "normal"),
        style: str(font.style, "normal"),
      });
    }
  }
  return lines;
}

function hitBudget(maxHits: unknown): number {
  const asked = typeof maxHits === "number" && Number.isFinite(maxHits) ? Math.trunc(maxHits) : MAX_HITS_DEFAULT;
  return Math.min(MAX_HITS_CAP, Math.max(1, asked));
}

function sizeOf(bounds: readonly number[]): PageSize {
  return { width: bounds[2] - bounds[0], height: bounds[3] - bounds[1] };
}

function countNonWhite(rgba: Uint8ClampedArray): number {
  let count = 0;
  for (let i = 0; i < rgba.length; i += 4) {
    if (rgba[i] !== 255 || rgba[i + 1] !== 255 || rgba[i + 2] !== 255) count++;
  }
  return count;
}

function countSignatures(acro: MupdfObject | null): number {
  const fields = at(acro, "Fields");
  if (!fields || !fields.isArray()) return 0;
  let total = 0;
  for (let index = 0; index < fields.length; index++) total += countSignatureField(fields.get(index) as MupdfObject);
  return total;
}

function countSignatureField(field: MupdfObject): number {
  let total = 0;
  const kind = at(field, "FT");
  try {
    // Only a signature that is actually there counts: a blank /FT /Sig widget is not read-only.
    if (kind && kind.isName() && kind.asName() === "Sig" && at(field, "V")) total += 1;
  } catch {
    // Unreadable field type counts as none.
  }
  const kids = at(field, "Kids");
  if (kids && kids.isArray()) {
    for (let index = 0; index < kids.length; index++) total += countSignatureField(kids.get(index) as MupdfObject);
  }
  return total;
}








/** Fields a saved copy must still declare the same way as the session document. `encrypted`
 * is checked against the pending security change, and `needsPassword` is not comparable:
 * the source session is authenticated, the freshly written copy is not. */
const SAVE_SHAPE_KEYS: (keyof DocumentInfo)[] = [
  "signatures", "certified", "xfa", "attachments", "openAction", "tagged",
];

function sameSaveShape(before: DocumentInfo, after: DocumentInfo, expectedEncrypted: boolean): boolean {
  return after.encrypted === expectedEncrypted && SAVE_SHAPE_KEYS.every(key => before[key] === after[key]);
}

/** Permission flags only: the caller compares what it asked mupdf to write with what it read back. */
function samePermissions(before: DocumentPermissions, after: DocumentPermissions): boolean {
  return PERMISSION_BITS.every(([name]) => before[name] === after[name]);
}


function toRgba(pixmap: MupdfPixmap): Uint8ClampedArray {
  const source = pixmap.getPixels();
  const components = pixmap.getNumberOfComponents();
  if (components === 4) return source.slice();
  if (components !== 3) throw new Error(`render: unexpected pixmap components: ${components}`);
  const rgba = new Uint8ClampedArray((source.length / 3) * 4);
  for (let i = 0, j = 0; i < source.length; i += 3, j += 4) {
    rgba[j] = source[i];
    rgba[j + 1] = source[i + 1];
    rgba[j + 2] = source[i + 2];
    rgba[j + 3] = 255;
  }
  return rgba;
}

import {
  MAX_EDIT_TARGETS,
  MAX_EDIT_TEXT_CHARS,
  MAX_EDIT_REPLY_TEXT_CHARS,
  editHash,
  editObjectBox,
  editPageClipBox,
  editPlacementMatrix,
  editDuplicateEdit,
  editPlanPlacement,
  editPlacementAfter,
  editPlacementSpans,
  editViewBox,
  editMultiply,
  editInvert,
  editStructuredRuns,
  editWriteElement,
  editRectAffine,
  editCheckFull,
  editPlanText,
  editPrivatise,
  editReadStream,
  editRunsOf,
  editSplice,
  editWritePlans,
  editWriteText,
  editPlanParagraph,
  editWriteParagraph,
  scanEditTargets,
  editNumber,
  editSetBase14Font,
  editAdvance,
} from "./content-edit.js";
import type { EditRunChar, EditSpanEdit, EditOrient, EditParagraph, EditPlacement, EditPlacementWrite, EditReplaceMatch, EditReplaceRejected, EditRun, EditStructuredText, EditTargetRecord, EditTextPlan } from "./content-edit.js";
import {
  MAX_REDACT_MARKS,
  addRedactPlans,
  applyRedactions,
  checkSanitize,
  checkRedactSourceBytes,
  redactMarks,
  redactPreview,
  redactRect,
  redactSearchPlan,
  setRedactRect,
} from "./redact.js";
export { MAX_REDACT_MARKS, MAX_REDACT_TEXT, MAX_REDACT_IMAGE_PIXELS, REDACT_SANITIZE_KEYS, checkRedactSourceBytes } from "./redact.js";
export type { RedactInventory, RedactPreview, RedactSanitize, RedactShared } from "./redact.js";
export { MAX_EDIT_SCAN_BYTES, MAX_EDIT_TRAVERSALS, MAX_EDIT_FORM_DEPTH, MAX_EDIT_NESTING, MAX_EDIT_TOKENS, MAX_EDIT_TARGETS, MAX_EDIT_TEXT_CHARS, MAX_EDIT_REPLY_TEXT_CHARS, MAX_EDIT_OPERANDS } from "./content-edit.js";
export { editNumber } from "./content-edit.js";
export type { EditOrient } from "./content-edit.js";
export { MAX_IMAGE_PAGE_POINTS };
export type { EditTargetRecord };
export { allowed, OP_PERMISSION } from "./engine-core.js";
