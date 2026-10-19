// "Draw a box, everything inside is gone" (docs/rust-and-ux-plan.md §2.3): the boxes the user draws, held in
// memory and sent to the engine as `regions`, their undo history, and the plain words the flow shows.
// No DOM here, so tests/redaction.test.mjs can check it directly.
import type { RedactInspectRegion, RedactSanitizeKey } from '../../src/renderer/protocol.js';
import type { Point } from './geometry.js';

export type Rect = [number, number, number, number];
export type Box = { id: number; page: number; rect: Rect; inspect?: RedactInspectRegion | null };
type Step = { kind: 'add' | 'remove'; box: Box; index: number };

/** A box smaller than this (page pt) on either side is a tap, not a box. */
export const MIN_BOX = 4;
/** The engine's bound on boxes in one redaction (MAX_REDACT_MARKS). */
export const MAX_BOXES = 2000;

/** The box two finger positions span, clamped to the page; null for a tap. */
export function boxFrom(start: Point, end: Point, width: number, height: number): Rect | null {
  const clamp = (v: number, max: number) => Math.max(0, Math.min(max, v));
  const x0 = clamp(Math.min(start[0], end[0]), width), x1 = clamp(Math.max(start[0], end[0]), width);
  const y0 = clamp(Math.min(start[1], end[1]), height), y1 = clamp(Math.max(start[1], end[1]), height);
  if (x1 - x0 < MIN_BOX || y1 - y0 < MIN_BOX) return null;
  const round = (v: number) => Math.round(v * 100) / 100;
  return [round(x0), round(y0), round(x1), round(y1)];
}

/** Boxes in memory with a simple undo of adding and removing. */
export class Boxes {
  private list: Box[] = [];
  private undoSteps: Step[] = [];
  private redoSteps: Step[] = [];
  private next = 1;
  get all(): readonly Box[] { return this.list; }
  get count() { return this.list.length; }
  get canUndo() { return this.undoSteps.length > 0; }
  get canRedo() { return this.redoSteps.length > 0; }
  onPage(page: number) { return this.list.filter(box => box.page === page); }
  find(id: number) { return this.list.find(box => box.id === id); }
  add(page: number, rect: Rect): Box | null {
    if (this.list.length >= MAX_BOXES) return null;
    const box: Box = { id: this.next++, page, rect };
    this.list.push(box);
    this.undoSteps.push({ kind: 'add', box, index: this.list.length - 1 }); this.redoSteps = [];
    return box;
  }
  remove(id: number): boolean {
    const index = this.list.findIndex(box => box.id === id);
    if (index < 0) return false;
    const [box] = this.list.splice(index, 1);
    this.undoSteps.push({ kind: 'remove', box, index }); this.redoSteps = [];
    return true;
  }
  /** Undoes the last add or remove; returns the box it affected. */
  undo(): Box | null { return this.replay(this.undoSteps, this.redoSteps, true); }
  redo(): Box | null { return this.replay(this.redoSteps, this.undoSteps, false); }
  private replay(from: Step[], to: Step[], undoing: boolean): Box | null {
    const step = from.pop();
    if (!step) return null;
    const removing = (step.kind === 'add') === undoing;
    if (removing) this.list = this.list.filter(box => box.id !== step.box.id);
    else this.list.splice(Math.min(step.index, this.list.length), 0, step.box);
    to.push(step);
    return step.box;
  }
  clear() { this.list = []; this.undoSteps = []; this.redoSteps = []; }
  /** What the engine gets: `[{page, rect}]` in y-down page pt. */
  regions() { return this.list.map(box => ({ page: box.page, rect: [...box.rect] })); }
  /** Boxes grouped by page, pages in order, boxes in drawing order. */
  byPage(): { page: number; boxes: Box[] }[] {
    const pages = new Map<number, Box[]>();
    for (const box of this.list) pages.set(box.page, [...(pages.get(box.page) ?? []), box]);
    return [...pages.entries()].sort((a, b) => a[0] - b[0]).map(([page, boxes]) => ({ page, boxes }));
  }
}

/** The topmost box on `page` containing `p`, widened by `slop` page pt for a fingertip. */
export function boxAt(boxes: readonly Box[], page: number, p: Point, slop = 0): Box | null {
  for (let i = boxes.length - 1; i >= 0; i--) {
    const box = boxes[i], [x0, y0, x1, y1] = box.rect;
    if (box.page === page && p[0] >= x0 - slop && p[0] <= x1 + slop && p[1] >= y0 - slop && p[1] <= y1 + slop) return box;
  }
  return null;
}

/** "contract.pdf" → "contract redacted.pdf"; "contract redacted 2.pdf" when that name is taken. */
export function redactedName(name: string, taken: Iterable<string>): string {
  const stem = name.replace(/\.pdf$/i, '').trim() || 'Document';
  const used = new Set([...taken].map(n => n.toLowerCase()));
  for (let n = 1; ; n++) {
    const candidate = `${stem} redacted${n === 1 ? '' : ` ${n}`}.pdf`;
    if (!used.has(candidate.toLowerCase())) return candidate;
  }
}

export const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/** The nine kinds of hidden information, in plain words, in the order the Details list shows them. */
export const SANITIZE_LABELS: [RedactSanitizeKey, string][] = [
  ['info', 'Author, title and other document properties'],
  ['xmp', 'Extra document details saved by other apps'],
  ['attachments', 'Attached files'],
  ['outlines', 'Bookmarks'],
  ['annots', 'Other comments and notes, on every page'],
  ['links', 'Web and file addresses in links'],
  ['fields', 'Everything typed into form fields'],
  ['layers', 'Layer names'],
  ['struct', 'Descriptions written for screen readers'],
];

/** Page numbers as people read them: "page 3", "pages 2 and 5", "pages 1, 2 and 4". */
export function pagesNamed(pages: number[]): string {
  const shown = [...new Set(pages)].sort((a, b) => a - b).map(p => String(p + 1));
  if (shown.length <= 1) return `page ${shown[0] ?? ''}`.trim();
  return `pages ${shown.slice(0, -1).join(', ')} and ${shown.at(-1)}`;
}

/** The question when a box covers artwork that cannot be cut exactly (engine reason `needs-flatten`). */
export function flattenQuestion(pages: number[]) {
  const named = pagesNamed(pages), many = new Set(pages).size > 1;
  const Named = named[0].toUpperCase() + named.slice(1);
  return {
    title: `${Named} ${many ? 'contain' : 'contains'} artwork that can't be cut precisely.`,
    text: `Redact ${named} by turning ${many ? 'them into images' : 'it into an image'}? ${many ? 'Their' : 'Its'} text will no longer be selectable.`,
    action: `Turn ${named} into ${many ? 'images' : 'an image'}`,
  };
}

/** A refusal of redact-apply in plain words (plan §3.2). Never the engine's own message. */
export function refusalMessage(reason: string | undefined, signed: boolean): { title: string; text: string } {
  const unchanged = 'Nothing was saved, and your original is unchanged.';
  switch (reason) {
    case 'verify-failed': return { title: 'The redacted copy didn’t pass its final check', text: `After removing the content, the copy is checked to make sure nothing is left under the boxes. That check did not pass, so the copy was thrown away. ${unchanged} Try drawing the boxes a little larger.` };
    case 'too-large': return { title: 'This PDF is too large to redact here', text: `Redaction works on documents up to 12 MB in this version. ${unchanged}` };
    case 'too-many': return { title: 'Too many boxes', text: `Redact up to ${MAX_BOXES} areas at a time. ${unchanged}` };
    case 'scan-budget': return { title: 'A page is too complex to redact here', text: `A page under one of the boxes has more content than this device can check safely. ${unchanged}` };
    case 'invalid': return { title: 'Something under a box can’t be removed safely', text: `This version can’t remove part of the content under one of the boxes exactly. ${unchanged}` };
    case 'read-only': return signed
      ? { title: 'This signed PDF can’t be redacted yet', text: `Redacting would break its signatures, and this version can’t make an unsigned copy. ${unchanged}` }
      : { title: 'This PDF doesn’t allow changes', text: `Its security settings don’t allow editing, so it can’t be redacted. ${unchanged}` };
    case 'no-marks': return { title: 'Nothing to redact', text: 'Drag over anything you want removed first.' };
    default: return { title: 'Redaction couldn’t be finished', text: unchanged };
  }
}
