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
import { MAX_REDACT_SOURCE_BYTES } from "./limits.js";
import type { RedactPreview, RedactSanitize, RedactSanitizeKey } from "./protocol.js";

// ------------------------------------------------------------------ words (plan §3.2: plain, no internals)

export const SANITIZE_KEYS: readonly RedactSanitizeKey[] = ["info", "xmp", "attachments", "outlines", "annots", "links", "fields", "layers", "struct"];

/** The nine hidden-information switches, in plain words (they sit under "Details"). */
export const SANITIZE_LABELS: Record<RedactSanitizeKey, string> = {
  info: "Author, title and other document properties",
  xmp: "Hidden metadata (editing history, program names)",
  attachments: "Attached files",
  outlines: "Bookmark titles",
  annots: "All other comments and markup",
  links: "Link addresses",
  fields: "Values typed into form fields",
  layers: "Layer names",
  struct: "Descriptions for screen readers (alternate text)",
};

const plural = (count: number, one: string, many = `${one}s`) => `${count} ${count === 1 ? one : many}`;

export const REDACT_CHIP = "Drag over anything you want removed. Esc to stop.";
export function areasText(count: number): string { return plural(count, "area"); }
export function boxesText(count: number): string { return plural(count, "box", "boxes"); }
export function panelHeading(count: number): string { return `Redactions (${count})`; }
export function redactButtonText(count: number): string { return count > 0 ? `Redact ${areasText(count)}…` : "Redact areas…"; }
export function countText(boxes: number, pages: number): string {
  return boxes > 0 ? `${boxesText(boxes)} on ${plural(pages, "page")}` : "No boxes yet";
}
export function confirmHeading(count: number): string { return `Permanently remove everything inside ${boxesText(count)}?`; }
export const CONFIRM_TEXT = "Text, pictures, drawings, comments and form fields inside the boxes are deleted from the file, not just covered. The result is saved as a new file; your original is not changed.";
export function bannerText(name: string, count: number): string {
  return `You are viewing the redacted copy '${name}'. ${areasText(count)} removed and checked.`;
}
export function pendingText(count: number): string {
  return `${areasText(count)} ${count === 1 ? "is" : "are"} marked but not redacted yet.`;
}
export const PENDING_DETAIL = "Saving now keeps the boxes as marks. Everything under them stays in the file until you redact it.";
/** §2.4: the size ceiling in decimal MB, rounded down so the stated size always works. */
export function sizeLimitText(bytes = MAX_REDACT_SOURCE_BYTES): string {
  const mb = Math.floor(bytes / 1e5) / 10;
  return `Redaction works on files up to ${mb} MB in this version.`;
}
export function flattenQuestion(pages: number[]): { heading: string; text: string; yes: string } {
  const list = pages.map(page => page + 1);
  const name = list.length === 1 ? `page ${list[0]}` : `pages ${list.slice(0, -1).join(", ")} and ${list.at(-1)}`;
  const contains = list.length === 1 ? "contains" : "contain";
  return {
    heading: `${name[0].toUpperCase()}${name.slice(1)} ${contains} artwork that can't be cut precisely.`,
    text: `Redact ${name} by turning ${list.length === 1 ? "it" : "them"} into an image? ${list.length === 1 ? "Its" : "Their"} text will no longer be selectable.`,
    yes: `Flatten ${name}`,
  };
}
export const SIGNED_QUESTION = {
  heading: "Redacting removes the signatures.",
  text: "Make an unsigned redacted copy? The original keeps its signatures.",
  yes: "Make unsigned copy",
};
/** A refused apply in plain words; `undefined` keeps the engine's own sentence. */
export function refusalText(reason: string | undefined): string | undefined {
  if (reason === "too-large") return `${sizeLimitText()} This file is larger.`;
  if (reason === "read-only") return "This document can't be changed, so it can't be redacted.";
  if (reason === "shared-content") return "A picture or drawing in a box also appears elsewhere without a box. Add a box there too, then redact again.";
  if (reason === "no-match") return "That text was not found in this document.";
  return undefined;
}
export function sharedText(item: { page: number; kind: "form" | "image"; otherPages: number[] }): string {
  const what = item.kind === "image" ? "picture" : "drawing";
  const same = item.otherPages.every(page => page === item.page);
  return same
    ? `Page ${item.page + 1}: this ${what} appears again on the same page outside the box. Add a box there too.`
    : `Page ${item.page + 1}: this ${what} also appears on page${item.otherPages.length === 1 ? "" : "s"} ${item.otherPages.map(page => page + 1).join(", ")} without a box. Add a box there too.`;
}

// ------------------------------------------------------------------ pure helpers

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
  /** Escape may leave the tool: nothing else (a dialog, a menu, a selection, a gesture) owns it. */
  escapeFree(): boolean;
  exitTool(): void;
  openOriginal(): Promise<void>;
  /** The overlay needs repainting (the underneath view changed). */
  repaint(): void;
  /** Main's close prompt counts pending boxes. */
  reportPending(count: number): void;
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
  const pendingNow = el<HTMLButtonElement>("redact-pending-now");
  const pendingMarks = el<HTMLButtonElement>("redact-pending-marks");
  const pendingCancel = el<HTMLButtonElement>("redact-pending-cancel");
  const askDialog = el<Dialog>("redact-ask-dialog");
  const askHeading = el<HTMLElement>("redact-ask-heading");
  const askText = el<HTMLElement>("redact-ask-text");
  const askYes = el<HTMLButtonElement>("redact-ask-yes");
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
  let bannerState: { name: string; areas: number; original: boolean } | null = null;
  let lastPending = -1;

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
  window.addEventListener("pointerup", endPeek);
  window.addEventListener("pointercancel", endPeek);

  // §2.3 step 1: Escape stops the tool when nothing else owns the key.
  window.addEventListener("keydown", event => {
    if (event.key !== "Escape" || !active || event.defaultPrevented) return;
    if (!host.escapeFree()) return;
    event.preventDefault();
    host.exitTool();
  });

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
    if (marks !== lastPending) { lastPending = marks; host.reportPending(marks); }
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
        const name = `${box.search ? "Found text" : "Box"} ${index + 1}`;
        const show = document.createElement("button");
        show.type = "button";
        show.className = "redact-box-go";
        show.textContent = name;
        show.title = `Show ${name.toLowerCase()} on page ${group.page + 1}`;
        show.addEventListener("click", () => host.goTo(box.page));
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
    sharedEl.replaceChildren(...preview.shared.map(item => {
      const row = document.createElement("li");
      row.textContent = sharedText(item);
      return row;
    }));
    detailsEl.open = false;
    setSanitize(sanitizeAll(true));
    confirmButton.disabled = preview.shared.length > 0 || preview.marks === 0 || !host.canEdit();
    if (!confirmDialog.open) confirmDialog.showModal();
    (confirmButton.disabled ? confirmCancel : confirmButton).focus();
  }
  confirmCancel.addEventListener("click", () => confirmDialog.close());

  // ---------------------------------------------------------------- questions

  function askPending(count: number): Promise<PendingChoice> {
    pendingHeading.textContent = pendingText(count);
    return ask<PendingChoice>(pendingDialog, [[pendingNow, "redact"], [pendingMarks, "marks"], [pendingCancel, "cancel"]], "cancel", pendingNow);
  }
  function askYesNo(question: { heading: string; text: string; yes: string }): Promise<boolean> {
    askHeading.textContent = question.heading;
    askText.textContent = question.text;
    askYes.textContent = question.yes;
    return ask<"yes" | "no">(askDialog, [[askYes, "yes"], [askCancel, "no"]], "no", askCancel).then(value => value === "yes");
  }

  // ---------------------------------------------------------------- banner (§2.3 step 6)

  function showBanner(name: string, areas: number, original: boolean): void {
    bannerState = { name, areas, original };
    bannerTextEl.textContent = bannerText(name, areas);
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

  /** The underneath view: before the engine can say what a box takes, the box itself is tinted. */
  function underneath(): boolean {
    return body.classList.contains("redact-underneath") || body.classList.contains("redact-peek");
  }

  return {
    setActive,
    get active() { return active; },
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
    get pendingOpen() { return pendingDialog.open; },
    showBanner,
    hideBanner,
    get banner() { return bannerState; },
    applyOpen,
  };
}

export type RedactUi = ReturnType<typeof createRedactUi>;
