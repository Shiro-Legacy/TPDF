/**
 * The worker protocol: the requests an app sends the engine worker, the replies
 * it gets and the shapes inside them. Types only, so importing this module loads
 * no engine code. The Rust engine's adapter (engine-rust.ts), the worker, the
 * mobile app (mobile/src), the MuPDF test oracle (engine/oracle/app-engine, which
 * re-exports them) and the tests import these. It was the Electron renderer's
 * protocol.ts; web/ is its home since that app was retired.
 */

/** Where `stamp` writes: the stamp kinds the app can write. */
export type StampKind = "watermark" | "header" | "footer";

/** The document parts `redact-apply` can strip (the oracle's REDACT_SANITIZE_KEYS), and the preview it replies with. */
export type RedactSanitizeKey = "info" | "xmp" | "attachments" | "outlines" | "annots" | "links" | "fields" | "layers" | "struct";
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

/** P6 U6 §1.1: one of the eight axis-aligned orientations of a placement `cm`. */
export type EditOrient = { swap: boolean; sx: 1 | -1; sy: 1 | -1 };

/** What `sign-prepare` signs: the signer's name and reason, where, and whether it certifies. */
export type SignPrepareOptions = {
  name: string; reason: string; page: number; rect: [number, number, number, number] | null;
  certify: boolean; when: number;
  /** P5 U3 §1.4: sign this existing empty field on `page` in place (`rect` is null); absent or null = a new widget. */
  field?: string | null;
};

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
  | "no-compare-document"
  /** `scan-compose`/`scan-measure` text needs the CJK fallback font and none is installed (Rust engine). */
  | "no-font"
  /** An op this engine does not implement (`scan-compose`/`scan-measure` on MuPDF, a test oracle only). */
  | "unsupported";

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
  /** M9: the local date (`YYYY-MM-DD`) a signature stamp shows; absent, the worker's date at add time. */
  today?: string;
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
/** Redaction redesign R6: a box held in memory; `rect` is y-down page pt (redact-add's space), clamped to the page, at least 1 by 1 pt. */
export type RedactRegion = { page: number; rect: number[] };
/**
 * M9: `entropy` (32 bytes) seeds an encrypted rewrite on the Rust engine; absent, the adapter draws fresh bytes.
 * R6: `regions` are applied together with the document's Redact marks (none need exist).
 * R4: `flatten` lists the pages (0-based) the user agreed may become a picture when a box covers
 * artwork that cannot be cut exactly (refusal reason `needs-flatten`, `pages`).
 * D5: `unsign: true` agrees to an unsigned copy of a signed file (refusal reason `signed`).
 */
export type RedactApplyRequest = {
  id: number; op: "redact-apply"; sanitize: Record<string, boolean>; entropy?: ArrayBuffer;
  regions?: RedactRegion[]; flatten?: number[]; unsign?: boolean;
};
/** Redaction redesign: what `redact-apply` would remove under each box, for the "show what's underneath" tint. Read only. */
/**
 * A box to inspect: a plain `{page, rect}` is an area box (R1). `annotId` names one of the page's Redact marks
 * instead (its own rect, quads and rule; `rect` not needed); `search: true` takes `rect` as a find hit grown by
 * two points and uses the search rule (the neighbours the margin grazes stay), as apply does for search marks.
 */
export type RedactInspectBox = { page: number; rect?: number[]; annotId?: number; search?: boolean };
export type RedactInspectRequest = { id: number; op: "redact-inspect"; regions: RedactInspectBox[] };
/** One box of `redact-inspect`'s reply; every rect and quad is y-down page pt. */
export type RedactInspectRegion = {
  page: number; rect: number[];
  /** What the removed glyphs spell, as the view reads them (lines joined by "\n"). */
  text: string;
  /** Quads [x0,y0,x1,y1,x2,y2,x3,y3] of the glyphs that go (adjacent upright glyphs of one show joined); always finite. */
  glyphs: number[][];
  /** Pictures the box touches: the part under the box; `whole` when apply removes the whole placement. */
  images: { rect: number[]; whole: boolean }[];
  /** Painted shapes and shadings the box touches: the part under the box; `whole` when apply removes the whole draw. */
  shapes: { rect: number[]; kind: "path" | "shading"; whole: boolean }[];
  /** Annotations and form fields the box touches (removed whole). `kind` is the PDF subtype in lower case
   *  ("widget" = a form field, "freetext", "link", "text" = a note, ...); `label` a field's name or a comment's start. */
  annots: { kind: string; rect: number[]; label: string }[];
  /** R4: the box covers artwork that cannot be cut exactly (a function or mesh shading, a pattern, soft mask or
   *  Type 3 font showing text or pictures): applying needs this page in `flatten` (refusal `needs-flatten`, `pages`). */
  needsFlatten: boolean;
  /** The reply's bound (10,000 quads, shapes and pictures in all) was reached: `glyphs` then holds one quad
   *  per removed run, or the box itself, and `text` may be cut short. */
  truncated: boolean;
};
export type RedactInspectReply = { id: number; ok: true; regions: RedactInspectRegion[] };
/** `redact-apply`'s reply: the copy, the marks and boxes applied, the pages changed and those flattened (R4). */
export type RedactApplyReply = {
  id: number; ok: true; bytes: ArrayBuffer; marks: number; pages: number[]; flattened: number[];
  /** D5: the source was signed and the copy is unsigned (sent with `unsign: true`). */
  unsigned: boolean;
  /** Something under a box is also used elsewhere (another page, a pattern, a mask, a comment's appearance,
   *  another box's placement of the same drawing...): it was removed there too. Nothing under a box is ever
   *  left; the UI may say more was removed than boxed. */
  overRedacted: boolean;
  sanitized: RedactInventory; kept: RedactInventory; verified: { text: number; pixels: number; raw: number };
};
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
 * The mobile scanner's PDF (engine/design/app-m9-mobile-release.md): one page per `pages` entry, of
 * `width` x `height` points, showing `images[image]` (a JPEG, stored as it is) over the whole page, then
 * stroked `boxes` and text `runs`. Positions are in points from the page's top-left, y down; a run's
 * (x, y) is its baseline origin and `width` fits its advance to that width. Text takes Helvetica
 * (WinAnsi), Helvetica's Latin glyph names, embedded Nimbus Sans outlines or the CJK fallback font, each
 * with ToUnicode. No session state is read or changed. Rust engine only: engine.ts refuses it.
 */
export type ScanComposeRequest = {
  id: number; op: "scan-compose"; images?: ArrayBuffer[]; producer?: string;
  pages: {
    width: number; height: number; image?: number;
    runs?: { text: string; x: number; y: number; size: number; bold?: boolean; invisible?: boolean; width?: number; color?: number[] }[];
    boxes?: { x: number; y: number; width: number; height: number; lineWidth?: number; color?: number[] }[];
  }[];
};
/** The advance (1/1000 em) of each distinct character of `text` as `scan-compose` draws it, and its font. */
export type ScanMeasureRequest = { id: number; op: "scan-measure"; text: string };
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
export type SavedRequest = { id: number; op: "saved"; ticket?: string };
export type SessionRequest =
  OpenRequest | RenderRequest | ThumbnailRequest | SearchRequest | AuthenticateRequest
  | CloseRequest | SaveRequest | ValidateRequest | SecuritySetRequest | AnnotsRequest | AnnotAddRequest
  | AnnotSetRequest | AnnotMoveRequest | AnnotDeleteRequest | RedactAddRequest | RedactSetRequest | RedactSearchRequest | RedactPreviewRequest | RedactApplyRequest | RedactInspectRequest | SignPrepareRequest | SignListRequest
  | PageRotateRequest | PageDeleteRequest | PageMoveRequest
  | FieldsRequest | FieldSetRequest | FieldAddRequest | FieldPropsRequest | FieldDeleteRequest
  | OutlineRequest | OutlineAddRequest | OutlineSetRequest
  | OutlineDeleteRequest | LinksRequest | LinkAddRequest | LinkDeleteRequest | LinkSetRequest
  | InsertPagesRequest | ExtractPagesRequest | PdfFromImagesRequest | TextAllRequest | ScanComposeRequest | ScanMeasureRequest
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
/** `ticket` names the state the bytes were written from; `saved` takes it back (both engines). */
export type SaveReply = { id: number; ok: true; bytes: ArrayBuffer; saveMs: number; dirty: boolean; revision: number; security?: SecurityState; ticket?: string };
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
export type ScanComposeReply = { id: number; ok: true; bytes: ArrayBuffer; pageCount: number };
/** `fonts[i]`: helvetica, latin (Helvetica's Latin glyph names), nimbus, cjk, or missing (drawn as a box). */
export type ScanMeasureReply = { id: number; ok: true; chars: string[]; regular: number[]; bold: number[]; fonts: string[] };
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
export type Reply = OpenReply | RenderReply | ThumbReply | SearchReply | SaveReply | ValidateReply | AnnotsReply | FieldsReply | OutlineReply | LinksReply | MutationReply | AuthenticateReply | SecurityReply | ExtractReply | PdfFromImagesReply | ScanComposeReply | ScanMeasureReply | TextAllReply | LabelsReply | LayersReply | PageIdsReply
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
