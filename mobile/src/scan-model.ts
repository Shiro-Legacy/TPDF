export const MAX_SCAN_PAGES = 12;
export const MAX_IMAGE_BYTES = 20 * 1024 * 1024;
export const MAX_SCAN_BYTES = 30 * 1024 * 1024;
export const MAX_PAGE_TEXT = 24_000;
/** Normalized coordinates measured from the top-left of the upright image. */
export type OCRLine = { text: string; x: number; y: number; width: number; height: number };
export type ScanPage = { id: string; data: string; width: number; height: number; lines?: OCRLine[]; markdown?: string };
export type ScanOutput = 'image' | 'searchable' | 'digital';

export function validateLines(value: unknown): OCRLine[] {
  if (!Array.isArray(value) || value.length > 4000) throw new Error('The text recognition result is too large.');
  let length = 0;
  return value.map(line => {
    if (!line || typeof line.text !== 'string' || line.text.length > 4000) throw new Error('Invalid recognized text.');
    length += line.text.length;
    if (length > MAX_PAGE_TEXT) throw new Error('Too much text on one page. Scan smaller sections.');
    for (const key of ['x', 'y', 'width', 'height']) {
      if (!Number.isFinite(line[key]) || line[key] < 0 || line[key] > 1) throw new Error('Invalid text position.');
    }
    if (!line.width || !line.height || line.x + line.width > 1.01 || line.y + line.height > 1.01) throw new Error('Text position is outside the page.');
    return { text: line.text, x: line.x, y: line.y, width: Math.min(line.width, 1 - line.x), height: Math.min(line.height, 1 - line.y) };
  });
}

export function validatePages(pages: ScanPage[], output: ScanOutput) {
  if (!pages.length || pages.length > MAX_SCAN_PAGES) throw new Error(`Choose 1–${MAX_SCAN_PAGES} pages.`);
  let bytes = 0;
  for (const page of pages) {
    if (!Number.isInteger(page.width) || !Number.isInteger(page.height) || page.width < 1 || page.height < 1 || page.width > 2400 || page.height > 2400) throw new Error('Invalid scan dimensions.');
    if (!/^data:image\/jpeg;base64,[A-Za-z0-9+/]+=*$/.test(page.data)) throw new Error('Invalid scan image.');
    bytes += page.data.length * .75;
    if (bytes > MAX_SCAN_BYTES) throw new Error('These scans exceed the 30 MB limit. Remove some pages.');
    if (output === 'searchable') {
      if (!page.lines) throw new Error('Recognize the text on every page first.');
      validateLines(page.lines);
    }
    if (output === 'digital' && (!page.markdown?.trim() || page.markdown.length > MAX_PAGE_TEXT)) throw new Error('Review the text on every page before saving.');
  }
}

export type TextBlock = { kind: 'heading' | 'paragraph' | 'list'; text: string; level?: number } | { kind: 'table'; rows: string[][] };
/** Deliberately small, text-only grammar. Model output can never execute HTML or load URLs. */
export function parseDocument(markdown: string): TextBlock[] {
  if (markdown.length > MAX_PAGE_TEXT) throw new Error('The reconstructed page is too long.');
  const rows = markdown.replace(/^```(?:markdown)?\s*\n|\n```\s*$/g, '').split(/\r?\n/);
  const blocks: TextBlock[] = [];
  for (let i = 0; i < rows.length; i++) {
    const row = rows[i].trim();
    if (!row) continue;
    if (row.startsWith('|') && row.endsWith('|')) {
      const table: string[][] = [];
      while (i < rows.length && rows[i].trim().startsWith('|') && rows[i].trim().endsWith('|')) {
        const cells = rows[i].trim().slice(1, -1).split('|').map(cell => cell.trim());
        if (!cells.every(cell => /^:?-{3,}:?$/.test(cell))) table.push(cells);
        i++;
      }
      i--;
      if (table.length) {
        if (Math.max(...table.map(cells => cells.length)) > 8) throw new Error('Tables support up to 8 columns. Edit this table before saving.');
        blocks.push({ kind: 'table', rows: table });
      }
    } else {
      const heading = /^(#{1,3})\s+(.+)$/.exec(row);
      const list = /^(?:[-*+]\s+|\d+[.)]\s+)/.test(row);
      if (heading) blocks.push({ kind: 'heading', level: heading[1].length, text: heading[2] });
      else if (list) blocks.push({ kind: 'list', text: row.replace(/^[-*+]\s+/, '• ') });
      else if (blocks.at(-1)?.kind === 'paragraph' && i > 0 && rows[i - 1].trim()) (blocks.at(-1) as { text: string }).text += ` ${row}`;
      else blocks.push({ kind: 'paragraph', text: row });
    }
  }
  return blocks;
}
