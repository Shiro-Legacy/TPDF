/**
 * Redaction redesign (docs/rust-and-ux-plan.md §2.3): "draw a box, everything inside is gone".
 *
 * This module owns the redaction tool's own surfaces and words: the chip and "Show what's
 * underneath" toggle in the tool row, the "Redactions (N)" side panel, the "Find text to redact…"
 * helper, the plain confirmation dialog, the pending-boxes save prompt, the flatten/unsigned
 * questions and the banner over a redacted copy. renderer.ts keeps the requests, the journal and
 * the drive; it reaches this module through `RedactHost` and a handful of hooks.
 *
 * Importing it touches no DOM (Node tests import the pure helpers); `createRedactUi` does.
 */
import { MSG } from "./messages.js";
import type { RedactInspectRegion, RedactPreview, RedactSanitize, RedactSanitizeKey } from "./protocol.js";

// ------------------------------------------------------------------ words (plan §3.2: plain, no internals)

export const SANITIZE_KEYS: readonly RedactSanitizeKey[] = ["info", "xmp", "attachments", "outlines", "annots", "links", "fields", "layers", "struct"];

/** The words live in the message catalogue (messages.ts, MSG.redact); these names are kept for the tool and its tests. */
const R = MSG.redact;
export const SANITIZE_LABELS = R.SANITIZE_LABELS;
export const REDACT_CHIP = R.REDACT_CHIP;
export const areasText = R.areasText;
export const boxesText = R.boxesText;
export const panelHeading = R.panelHeading;
export const redactButtonText = R.redactButtonText;
export const countText = R.countText;
export const confirmHeading = R.confirmHeading;
export const CONFIRM_TEXT = R.CONFIRM_TEXT;
export const bannerText = R.bannerText;
export const pagesName = R.pagesName;
export const flattenNote = R.flattenNote;
export const pendingText = R.pendingText;
export const pendingQuestion = R.pendingQuestion;
export const SAME_FILE_QUESTION = R.SAME_FILE_QUESTION;
export const OTHERS_QUESTION = R.OTHERS_QUESTION;
export const PENDING_DETAIL = R.PENDING_DETAIL;
export const sizeLimitText = R.sizeLimitText;
export const flattenQuestion = R.flattenQuestion;
export const SIGNED_QUESTION = R.SIGNED_QUESTION;
export const USAGE_RIGHTS_QUESTION = R.USAGE_RIGHTS_QUESTION;
export const OVER_REDACTED = R.OVER_REDACTED;
export const refusalText = R.refusalText;

// ------------------------------------------------------------------ pure helpers

/**
 * Whether main asks about unsaved boxes before a file replaces the window's document. D6's signed copy skips the
 * question for marks (the copy keeps them), but not for a signed file's boxes in memory: the copy does not have them.
 */
export function asksBoxesOnOpen(options: { skipDiscard?: boolean }, pending: { count: number; memory: boolean } | undefined): boolean {
  if (!pending || pending.count < 1) return false;
  return !options.skipDiscard || pending.memory;
}

export type RedactBox = { page: number; id: number; rect: number[]; search: boolean };
export type RedactPageGroup = { page: number; boxes: RedactBox[] };

/** The side panel's order: pages ascending, boxes top to bottom then left to right. */
export function groupByPage(boxes: RedactBox[]): RedactPageGroup[] {
  const pages = new Map<number, RedactBox[]>();
  for (const box of boxes) {
    const list = pages.get(box.page) ?? [];
    list.push(box);
    pages.set(box.page, list);
  }
  return [...pages.entries()].sort((a, b) => a[0] - b[0]).map(([page, list]) => ({
    page,
    boxes: [...list].sort((a, b) => (a.rect[1] ?? 0) - (b.rect[1] ?? 0) || (a.rect[0] ?? 0) - (b.rect[0] ?? 0)),
  }));
}

/** "Redact whole page": the page's full box in y-down page points. */
export function wholePageRect(size: { width: number; height: number }): number[] {
  return [0, 0, Math.round(size.width * 100) / 100, Math.round(size.height * 100) / 100];
}

/** All nine switches on or off (the "Also remove hidden information" checkbox). */
export function sanitizeAll(on: boolean): RedactSanitize {
  return Object.fromEntries(SANITIZE_KEYS.map(key => [key, on])) as RedactSanitize;
}

/** A drive's `{hidden?, details?}` into the nine switches: `hidden` sets all, `details` overrides single ones. */
export function sanitizeFrom(hidden: unknown, details: unknown): RedactSanitize {
  const out = sanitizeAll(hidden !== false);
  if (details && typeof details === "object" && !Array.isArray(details)) {
    for (const key of SANITIZE_KEYS) {
      const value = (details as Record<string, unknown>)[key];
      if (typeof value === "boolean") out[key] = value;
    }
  }
  return out;
}

export type PendingChoice = "redact" | "marks" | "cancel";
/** What sends the document out while boxes are pending. */
export type PendingAction = "save" | "export" | "print" | "sign";
/** The pending-boxes question's words for each way out ([Redact now…] stays first and default). */
export const pendingWords: (action: PendingAction, memory: boolean) => { detail: string; go: string } = R.pendingWords;

/** One tinted shape of the underneath view: a glyph quad (8 numbers) or a rect (4), y-down page pt. */
export type TintShape = { kind: "glyph" | "image" | "shape" | "annot"; points: number[] };

/** What `redact-inspect` says goes, flattened into tint shapes (glyphs, picture parts, drawings, comments and fields). */
export function tintShapes(regions: RedactInspectRegion[]): TintShape[] {
  const out: TintShape[] = [];
  for (const region of regions) {
    // Over the engine's bound the reply is coarse: tint the whole box rather than a partial picture.
    if ((region.truncated || region.needsFlatten) && region.rect?.length === 4) { out.push({ kind: "shape", points: region.rect }); if (!region.needsFlatten) continue; }
    for (const quad of region.glyphs ?? []) if (quad.length === 8) out.push({ kind: "glyph", points: quad });
    for (const image of region.images ?? []) if (image.rect?.length === 4) out.push({ kind: "image", points: image.rect });
    for (const shape of region.shapes ?? []) if (shape.rect?.length === 4) out.push({ kind: "shape", points: shape.rect });
    for (const annot of region.annots ?? []) if (annot.rect?.length === 4) out.push({ kind: "annot", points: annot.rect });
  }
  return out;
}

// ------------------------------------------------------------------ the DOM side

/** What the module needs from renderer.ts. */
export type RedactHost = {
  /** The open document's page sizes, or null. */
  pages(): { width: number; height: number }[] | null;
  /** The current preview (marks per page), or null. */
  preview(): RedactPreview | null;
  /** The boxes on one page (the renderer's `annots` request, Redact items only). */
  boxesOn(page: number): Promise<RedactBox[]>;
  goTo(page: number): void;
  deleteBox(page: number, annotId: number): Promise<void>;
  addSearch(text: string): Promise<boolean>;
  canAnnotate(): boolean;
  canEdit(): boolean;
  openOriginal(): Promise<void>;
  /** The overlay needs repainting (the underneath view changed). */
  repaint(): void;
  /** Main's close and open prompts count unsaved boxes (the host decides which are unsaved). */
  reportPending(): void;
  /** Review L6: the panel's "Box N" goes to the box and selects it. */
  showBox(page: number, id: number): void;
};

type Dialog = HTMLDialogElement;

function el<T extends HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`redact-ui: #${id} is missing`);
  return node as T;
}

/** Resolves a modal question with the value of the button pressed; Escape or Cancel resolve `cancel`. */
function ask<T extends string>(dialog: Dialog, buttons: [HTMLButtonElement, T][], cancel: T, focus: HTMLButtonElement): Promise<T> {
  return new Promise<T>(resolve => {
    let done = false;
    const finish = (value: T) => {
      if (done) return;
      done = true;
      for (const [button] of buttons) button.onclick = null;
      dialog.removeEventListener("close", onClose);
      if (dialog.open) dialog.close();
      resolve(value);
    };
    const onClose = () => finish(cancel);
    for (const [button, value] of buttons) button.onclick = () => finish(value);
    dialog.addEventListener("close", onClose);
    if (!dialog.open) dialog.showModal();
    focus.focus();
  });
}

export function createRedactUi(host: RedactHost) {
  const body = document.body;
  const pageRegion = el<HTMLElement>("page");
  const chip = el<HTMLElement>("redact-chip");
  const underneathInput = el<HTMLInputElement>("redact-underneath");
  const findOpen = el<HTMLButtonElement>("redact-find-open");
  const panel = el<HTMLElement>("redact-panel");
  const heading = el<HTMLElement>("redact-panel-heading");
  const applyOpen = el<HTMLButtonElement>("redact-apply-open");
  const countEl = el<HTMLOutputElement>("redact-count");
  const emptyEl = el<HTMLElement>("redact-panel-empty");
  const flattenNoteEl = el<HTMLElement>("redact-flatten-note");
  const listEl = el<HTMLOListElement>("redact-list");
  const findDialog = el<Dialog>("redact-find-dialog");
  const searchText = el<HTMLInputElement>("redact-search-text");
  const searchGo = el<HTMLButtonElement>("redact-search-go");
  const findCancel = el<HTMLButtonElement>("redact-find-cancel");
  const confirmDialog = el<Dialog>("redact-dialog");
  const confirmHeadingEl = el<HTMLElement>("redact-heading");
  const sharedEl = el<HTMLUListElement>("redact-shared");
  const hiddenInput = el<HTMLInputElement>("redact-hidden");
  const detailsEl = el<HTMLDetailsElement>("redact-details");
  const keptEl = el<HTMLElement>("redact-kept");
  const previewTable = el<HTMLTableElement>("redact-preview");
  const limitEl = el<HTMLElement>("redact-limit");
  const confirmButton = el<HTMLButtonElement>("redact-apply");
  const confirmCancel = el<HTMLButtonElement>("redact-cancel");
  const pendingDialog = el<Dialog>("redact-pending-dialog");
  const pendingHeading = el<HTMLElement>("redact-pending-heading");
  const pendingTextEl = el<HTMLElement>("redact-pending-text");
  const pendingNow = el<HTMLButtonElement>("redact-pending-now");
  const pendingMarks = el<HTMLButtonElement>("redact-pending-marks");
  const pendingCancel = el<HTMLButtonElement>("redact-pending-cancel");
  const askDialog = el<Dialog>("redact-ask-dialog");
  const askHeading = el<HTMLElement>("redact-ask-heading");
  const askText = el<HTMLElement>("redact-ask-text");
  const askYes = el<HTMLButtonElement>("redact-ask-yes");
  const askAlt = el<HTMLButtonElement>("redact-ask-alt");
  const askCancel = el<HTMLButtonElement>("redact-ask-cancel");
  const banner = el<HTMLElement>("redact-banner");
  const bannerTextEl = el<HTMLElement>("redact-banner-text");
  const bannerOriginal = el<HTMLButtonElement>("redact-banner-original");
  const bannerDismiss = el<HTMLButtonElement>("redact-banner-dismiss");
  const sanitizeBoxes = Object.fromEntries(SANITIZE_KEYS.map(key => [key, el<HTMLInputElement>(`redact-sanitize-${key}`)])) as Record<RedactSanitizeKey, HTMLInputElement>;

  chip.textContent = REDACT_CHIP;
  limitEl.textContent = sizeLimitText();
  for (const key of SANITIZE_KEYS) {
    const label = sanitizeBoxes[key].closest("label");
    const text = label?.querySelector(".redact-sanitize-name");
    if (text) text.textContent = SANITIZE_LABELS[key];
  }

  let active = false;
  let boxes: RedactBox[] = [];
  let listSeq = 0;
  let bannerState: { name: string; areas: number; original: boolean; overRedacted: boolean } | null = null;
  /** The last `redact-inspect` answer for the shown page's boxes (`key` names the boxes it was asked for). */
  let tint: { page: number; key: string; shapes: TintShape[] } | null = null;

  // ---------------------------------------------------------------- tool on/off

  function setActive(on: boolean): void {
    active = on;
    body.classList.toggle("redact-tool", on);
    panel.hidden = !on;
    if (!on) {
      body.classList.remove("redact-peek");
      if (findDialog.open) findDialog.close();
    }
    update();
  }

  function setUnderneath(on: boolean): void {
    underneathInput.checked = on;
    body.classList.toggle("redact-underneath", on);
    host.repaint();
  }

  // §2.3 step 2: while the pointer is down on the page the boxes turn translucent.
  pageRegion.addEventListener("pointerdown", event => {
    if (active && event.button === 0 && !event.ctrlKey) body.classList.add("redact-peek");
  }, true);
  const endPeek = () => body.classList.remove("redact-peek");
  // (A headless test imports renderer.js with a bare `window`; the listeners are then skipped.)
  const on = typeof window.addEventListener === "function" ? window.addEventListener.bind(window) : () => undefined;
  on("pointerup", endPeek);
  on("pointercancel", endPeek);

  // §2.3 step 1: Escape stops the tool. renderer.ts's layered Escape owns the key (X6): the tool is its last
  // layer, after an open dialog, menu, editor, gesture and a selected box (the first Escape deselects).

  underneathInput.addEventListener("change", () => setUnderneath(underneathInput.checked));

  // ---------------------------------------------------------------- panel

  function render(): void {
    const preview = host.preview();
    const marks = preview?.marks ?? 0, pages = preview?.pages.length ?? 0;
    heading.textContent = panelHeading(marks);
    applyOpen.textContent = redactButtonText(marks);
    applyOpen.disabled = !host.canEdit() || marks === 0;
    countEl.textContent = countText(marks, pages);
    findOpen.disabled = !host.canAnnotate() || !host.pages();
    searchGo.disabled = !host.canAnnotate() || !searchText.value;
    host.reportPending();
  }

  /** After the boxes changed: the counts now, the list once its pages are read. */
  function update(): void {
    render();
    if (active) void refreshList();
    else { listSeq += 1; boxes = []; }
  }

  async function refreshList(): Promise<void> {
    const seq = ++listSeq;
    const preview = host.preview();
    const pages = preview?.pages ?? [];
    const found: RedactBox[] = [];
    for (const page of pages) {
      const onPage = await host.boxesOn(page).catch(() => [] as RedactBox[]);
      if (seq !== listSeq) return;
      found.push(...onPage);
    }
    boxes = found;
    paintList();
  }

  function paintList(): void {
    const groups = groupByPage(boxes);
    emptyEl.hidden = groups.length > 0;
    listEl.replaceChildren(...groups.map(group => {
      const item = document.createElement("li");
      item.className = "redact-page";
      const go = document.createElement("button");
      go.type = "button";
      go.className = "redact-page-go";
      go.textContent = `Page ${group.page + 1}`;
      go.setAttribute("aria-label", `Go to page ${group.page + 1}, ${boxesText(group.boxes.length)}`);
      go.addEventListener("click", () => host.goTo(group.page));
      const count = document.createElement("span");
      count.className = "redact-page-count";
      count.textContent = boxesText(group.boxes.length);
      const rows = document.createElement("ul");
      group.boxes.forEach((box, index) => {
        const row = document.createElement("li");
        const name = `Box ${index + 1}${box.search ? " (found text)" : ""}`;
        const show = document.createElement("button");
        show.type = "button";
        show.className = "redact-box-go";
        show.textContent = name;
        show.title = `Show ${name.toLowerCase()} on page ${group.page + 1}`;
        show.addEventListener("click", () => host.showBox(box.page, box.id));
        const remove = document.createElement("button");
        remove.type = "button";
        remove.className = "redact-box-remove";
        remove.textContent = "Remove";
        remove.disabled = !host.canAnnotate();
        remove.setAttribute("aria-label", `Remove ${name.toLowerCase()} on page ${group.page + 1}`);
        remove.addEventListener("click", () => void host.deleteBox(box.page, box.id));
        row.append(show, remove);
        rows.append(row);
      });
      item.append(go, count, rows);
      return item;
    }));
  }

  // ---------------------------------------------------------------- find helper

  function openFind(): void {
    if (!findDialog.open) findDialog.showModal();
    searchText.focus();
    searchText.select();
    searchGo.disabled = !host.canAnnotate() || !searchText.value;
  }
  async function runFind(): Promise<boolean> {
    const text = searchText.value;
    if (!text) return false;
    const ok = await host.addSearch(text);
    if (ok && findDialog.open) findDialog.close();
    return ok;
  }
  findOpen.addEventListener("click", openFind);
  searchText.addEventListener("input", () => { searchGo.disabled = !host.canAnnotate() || !searchText.value; });
  searchText.addEventListener("keydown", event => {
    if (event.key !== "Enter" || !searchText.value) return;
    event.preventDefault();
    void runFind();
  });
  searchGo.addEventListener("click", () => void runFind());
  findCancel.addEventListener("click", () => findDialog.close());

  // ---------------------------------------------------------------- confirmation

  function sanitize(): RedactSanitize {
    return Object.fromEntries(SANITIZE_KEYS.map(key => [key, sanitizeBoxes[key].checked])) as RedactSanitize;
  }
  function syncHidden(): void {
    const values = SANITIZE_KEYS.map(key => sanitizeBoxes[key].checked);
    hiddenInput.checked = values.every(Boolean);
    hiddenInput.indeterminate = values.some(Boolean) && !values.every(Boolean);
    const preview = host.preview();
    const kept = SANITIZE_KEYS.filter(key => !sanitizeBoxes[key].checked && (!preview || (key === "xmp" ? preview.sanitize.xmp : preview.sanitize[key] > 0)));
    keptEl.textContent = kept.length ? `Kept in the copy: ${kept.map(key => SANITIZE_LABELS[key].toLowerCase()).join("; ")}.` : "";
  }
  function setSanitize(values: RedactSanitize): void {
    for (const key of SANITIZE_KEYS) sanitizeBoxes[key].checked = values[key];
    syncHidden();
  }
  hiddenInput.addEventListener("change", () => setSanitize(sanitizeAll(hiddenInput.checked)));
  for (const key of SANITIZE_KEYS) sanitizeBoxes[key].addEventListener("change", syncHidden);

  /** Fills and shows the dialog; renderer.ts wires its primary button. */
  function showConfirm(preview: RedactPreview): void {
    confirmHeadingEl.textContent = confirmHeading(preview.marks);
    const tbody = previewTable.tBodies[0];
    tbody.replaceChildren(...preview.pageCounts.map(item => {
      const row = document.createElement("tr");
      const page = document.createElement("td"), count = document.createElement("td");
      page.textContent = String(item.page + 1);
      count.textContent = String(item.marks);
      row.append(page, count);
      return row;
    }));
    sharedEl.replaceChildren(); // R3: shared pictures and drawings are copied, so they no longer block or warn
    detailsEl.open = false;
    setSanitize(sanitizeAll(true));
    confirmButton.disabled = preview.marks === 0 || !host.canEdit();
    if (!confirmDialog.open) confirmDialog.showModal();
    (confirmButton.disabled ? confirmCancel : confirmButton).focus();
  }
  confirmCancel.addEventListener("click", () => confirmDialog.close());

  // ---------------------------------------------------------------- questions

  /** `memory`: a signed file's boxes, which a saved copy does not carry. */
  function askPending(count: number, memory = false, action: PendingAction = "save"): Promise<PendingChoice> {
    const words = pendingWords(action, memory);
    pendingHeading.textContent = pendingText(count);
    pendingTextEl.textContent = words.detail;
    pendingMarks.textContent = words.go;
    return ask<PendingChoice>(pendingDialog, [[pendingNow, "redact"], [pendingMarks, "marks"], [pendingCancel, "cancel"]], "cancel", pendingNow);
  }
  function askYesNo(question: { heading: string; text: string; yes: string }): Promise<boolean> {
    askHeading.textContent = question.heading;
    askText.textContent = question.text;
    askYes.textContent = question.yes;
    askAlt.hidden = true;
    return ask<"yes" | "no">(askDialog, [[askYes, "yes"], [askCancel, "no"]], "no", askCancel).then(value => value === "yes");
  }
  /** Three answers: the primary `yes`, the `alt` button, or Cancel. */
  function askThree(question: { heading: string; text: string; yes: string; alt: string }): Promise<"yes" | "alt" | "no"> {
    askHeading.textContent = question.heading;
    askText.textContent = question.text;
    askYes.textContent = question.yes;
    askAlt.textContent = question.alt;
    askAlt.hidden = false;
    return ask<"yes" | "alt" | "no">(askDialog, [[askYes, "yes"], [askAlt, "alt"], [askCancel, "no"]], "no", askYes)
      .finally(() => { askAlt.hidden = true; });
  }

  // ---------------------------------------------------------------- banner (§2.3 step 6)

  function showBanner(name: string, areas: number, original: boolean, extra: { flattened?: number[]; unsigned?: boolean; usageRights?: boolean; overRedacted?: boolean } = {}): void {
    bannerState = { name, areas, original, overRedacted: extra.overRedacted === true };
    bannerTextEl.textContent = bannerText(name, areas, extra);
    bannerOriginal.hidden = !original;
    banner.hidden = false;
  }
  function hideBanner(): void {
    bannerState = null;
    banner.hidden = true;
  }
  bannerOriginal.addEventListener("click", () => void host.openOriginal());
  bannerDismiss.addEventListener("click", hideBanner);

  // ---------------------------------------------------------------- overlay

  /** The page's boxes (black, red outline) and, under them, the tint of what goes (shown while peeking or with the toggle). */
  function paintBoxes(fragment: DocumentFragment, ns: string, page: number, zoom: number, rects: number[][]): void {
    for (const box of rects) {
      const rect = document.createElementNS(ns, "rect");
      rect.setAttribute("x", String(box[0] * zoom));
      rect.setAttribute("y", String(box[1] * zoom));
      rect.setAttribute("width", String((box[2] - box[0]) * zoom));
      rect.setAttribute("height", String((box[3] - box[1]) * zoom));
      rect.setAttribute("class", "redact-mark");
      fragment.appendChild(rect);
    }
    if (!tint || tint.page !== page || tint.key !== inspectKey(page, rects)) return;
    for (const shape of tint.shapes) {
      const node = document.createElementNS(ns, "polygon");
      const p = shape.points.length === 8 ? shape.points : [shape.points[0], shape.points[1], shape.points[2], shape.points[1], shape.points[2], shape.points[3], shape.points[0], shape.points[3]];
      node.setAttribute("points", [0, 2, 4, 6].map(i => `${p[i] * zoom},${p[i + 1] * zoom}`).join(" "));
      node.setAttribute("class", `redact-tint redact-tint-${shape.kind}`);
      fragment.appendChild(node);
    }
  }
  function inspectKey(page: number, rects: number[][]): string {
    return `${page}:${JSON.stringify(rects)}`;
  }

  /** The underneath view: before the engine can say what a box takes, the box itself is tinted. */
  function underneath(): boolean {
    return body.classList.contains("redact-underneath") || body.classList.contains("redact-peek");
  }

  return {
    setActive,
    get active() { return active; },
    paintBoxes,
    inspectKey,
    get tintKey() { return tint?.key ?? ""; },
    setTint: (page: number, key: string, regions: RedactInspectRegion[] | null) => {
      tint = regions ? { page, key, shapes: tintShapes(regions) } : null;
      // R4: say before the question that this page will become an image.
      const flat = !!regions?.some(region => region.needsFlatten);
      flattenNoteEl.hidden = !flat;
      flattenNoteEl.textContent = flat ? flattenNote(page) : "";
    },
    get flattenNote() { return flattenNoteEl.hidden ? "" : flattenNoteEl.textContent ?? ""; },
    get tintCount() { return tint?.shapes.length ?? 0; },
    setUnderneath,
    get underneath() { return underneathInput.checked; },
    peeking: underneath,
    render,
    update,
    refreshList,
    get boxes() { return boxes; },
    openFind,
    runFind,
    findText: (text: string) => { searchText.value = text; },
    get findOpen() { return findDialog.open; },
    closeFind: () => { if (findDialog.open) findDialog.close(); },
    showConfirm,
    get confirmOpen() { return confirmDialog.open; },
    closeConfirm: () => { if (confirmDialog.open) confirmDialog.close(); },
    confirmButton,
    sanitize,
    setSanitize,
    askPending,
    askYesNo,
    askThree,
    get pendingOpen() { return pendingDialog.open; },
    showBanner,
    hideBanner,
    get banner() { return bannerState; },
    applyOpen,
  };
}

export type RedactUi = ReturnType<typeof createRedactUi>;
