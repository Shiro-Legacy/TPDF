import { parseDocument, validatePages, type ScanPage, type ScanOutput } from './scan-model.js';

/**
 * The engine a scan is composed and checked with: the Rust engine the app ships (the scan worker loads the
 * desktop build's worker-engine.js), or a test's. The engine writes the PDF (`scan-compose`), measures the
 * text rebuilt pages are laid out with (`scan-measure`) and picks each character's font; this module only
 * lays the pages out (engine/design/app-m9-mobile-release.md, "The scanner").
 */
export type ScanEngine = {
  createSession(): unknown;
  dispatch(request: never, session: never): { reply: Record<string, unknown> & { ok: boolean } };
};

type Run = { text: string; x: number; y: number; size: number; bold?: boolean; invisible?: boolean; width?: number; color?: number[] };
type Box = { x: number; y: number; width: number; height: number; lineWidth: number; color: number[] };
type Page = { width: number; height: number; image?: number; runs: Run[]; boxes: Box[] };

const MAX_OUTPUT_PAGES = 100;
const TEXT_COLOR = [0.12, 0.16, 0.15];
const RULE_COLOR = [0.75, 0.78, 0.76];

/**
 * NFC first, so a letter with a separate combining accent becomes the precomposed letter the fonts have
 * (e + U+0301 is é, in WinAnsi). Tabs become spaces; other control characters are dropped (no font has them),
 * and so are soft hyphens (shown only where a line breaks at one; lines are already broken). A lone surrogate
 * becomes U+FFFD.
 */
export function clean(value: string): string {
  // A lone surrogate iterates as one code unit (no regex lookbehind: Safari before 16.4 has none).
  const wellFormed = Array.from(value, char => (char.length === 1 && /[\ud800-\udfff]/.test(char) ? '\ufffd' : char)).join('');
  return wellFormed.normalize('NFC').replace(/\t/g, ' ').replace(/[\u0000-\u001f\u007f-\u009f\u00ad]/g, '');
}

/** A session on `engine` for one exchange of requests, closed afterwards. */
function withSession<T>(engine: ScanEngine, use: (send: (request: Record<string, unknown>) => Record<string, unknown> & { ok: boolean }) => T): T {
  const session = engine.createSession();
  let id = 0;
  const send = (request: Record<string, unknown>) =>
    (engine.dispatch as unknown as (r: unknown, s: unknown) => ReturnType<ScanEngine['dispatch']>)({ id: ++id, ...request }, session).reply;
  try {
    return use(send);
  } finally {
    send({ op: 'close' });
  }
}

/** What the scanner says for each limit `scan-compose` and `scan-measure` refuse past (their `detail`). */
const TOO_LARGE: Record<string, string> = {
  pages: 'The rebuilt document exceeds 100 pages.',
  images: 'These scans exceed the 50 MB limit. Remove some pages.',
  'image-size': 'A scan image is larger than 32 MB. Retake or crop that page.',
  runs: 'Too much text to place on these pages. Scan smaller sections.',
  text: 'Too much text on these pages. Scan smaller sections.',
  characters: 'Too many different characters for one PDF. Split the document.',
  output: 'The PDF exceeds the 50 MB output limit.',
};

/** The message the scanner shows for a refused scan op. */
export function failure(reply: Record<string, unknown>): Error {
  const error = typeof reply.error === 'string' ? reply.error : '';
  if (reply.reason === 'no-font') return new Error('The font for non-Latin text could not be loaded.');
  if (reply.reason === 'too-large') return new Error(TOO_LARGE[reply.detail as string] ?? TOO_LARGE.output);
  if (reply.reason === 'invalid' && /\bimage\b/.test(error)) return new Error('Invalid scan image.');
  return new Error('Could not create the PDF.');
}

function jpegBytes(data: string): ArrayBuffer {
  const binary = atob(data.split(',')[1]);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes.buffer;
}

/**
 * One scan PDF: each page's JPEG as it is on a page whose long side is 792 pt, with the OCR lines as invisible
 * text ('searchable'), or the reviewed Markdown typeset on A4 pages ('digital'). Composed by the engine
 * (`scan-compose`); throws the message the scanner shows.
 */
export function composeScan(engine: ScanEngine, pages: ScanPage[], output: ScanOutput): { bytes: ArrayBuffer; pageCount: number } {
  validatePages(pages, output);
  return withSession(engine, send => {
    const advances = output === 'digital' ? measure(send, pages) : null;
    const request = { op: 'scan-compose', producer: 'Tandem PDF', ...layout(pages, output, advances) };
    const reply = send(request);
    if (!reply.ok) throw failure(reply);
    return { bytes: reply.bytes as ArrayBuffer, pageCount: reply.pageCount as number };
  });
}

type Advances = Map<string, [number, number]>;

/** Every character of the rebuilt pages' blocks, measured by the engine: [regular, bold] in 1/1000 em. */
function measure(send: (request: Record<string, unknown>) => Record<string, unknown> & { ok: boolean }, pages: ScanPage[]): Advances {
  const parts: string[] = ['•'];
  for (const page of pages) {
    for (const block of parseDocument(page.markdown!)) {
      if (block.kind === 'table') parts.push(...block.rows.flat());
      else parts.push(block.text);
    }
  }
  const reply = send({ op: 'scan-measure', text: clean(parts.join('\n')) });
  if (!reply.ok) throw failure(reply);
  const chars = reply.chars as string[], regular = reply.regular as number[], bold = reply.bold as number[];
  return new Map(chars.map((char, i) => [char, [regular[i], bold[i]]]));
}

/** The pages `scan-compose` draws: images (each distinct one once), page sizes, text runs and table rules. */
function layout(pages: ScanPage[], output: ScanOutput, advances: Advances | null): { images: ArrayBuffer[]; pages: Page[] } {
  const images: ArrayBuffer[] = [];
  const imageIndex = new Map<string, number>();
  const out: Page[] = [];
  let page: Page | null = null;
  const newPage = (width: number, height: number) => {
    if (out.length + 1 > MAX_OUTPUT_PAGES) throw new Error('The rebuilt document exceeds 100 pages.');
    page = { width, height, runs: [], boxes: [] };
    out.push(page);
    return page;
  };
  /** Advance at size 1, in points. */
  const measure = (value: string, bold: boolean) => {
    let total = 0;
    for (const char of clean(value)) total += advances?.get(char)?.[bold ? 1 : 0] ?? 0;
    return total / 1000;
  };
  /** Text whose baseline starts at (x, y), y measured down from the top of the page. */
  const text = (value: string, x: number, y: number, size: number, bold = false) => {
    if (clean(value)) page!.runs.push({ text: clean(value), x, y, size, bold, color: TEXT_COLOR });
  };
  const wrap = (value: string, maxWidth: number, size: number, bold: boolean) => {
    value = value.normalize('NFC'); // so a line never splits a letter from its combining accent
    const result: string[] = [];
    const width = (line: string) => measure(line, bold) * size;
    let line = '';
    // Wrap at words where possible; split long tokens and CJK without dropping text.
    const words = Array.from(new Intl.Segmenter(undefined, { granularity: 'word' }).segment(value), s => s.segment);
    for (const word of words) {
      if (width(line + word) <= maxWidth) { line += word; continue; }
      if (line.trim()) result.push(line.trimEnd());
      line = '';
      for (const char of word.trimStart()) {
        if (line && width(line + char) > maxWidth) { result.push(line); line = ''; }
        line += char;
      }
    }
    if (line.trim()) result.push(line.trimEnd());
    return result.length ? result : [''];
  };

  for (const scan of pages) {
    if (output !== 'digital') {
      const width = scan.width / Math.max(scan.width, scan.height) * 792;
      const height = scan.height / Math.max(scan.width, scan.height) * 792;
      const current = newPage(width, height);
      // The same image twice (a page added twice) is stored once, as MuPDF's writer did.
      let image = imageIndex.get(scan.data);
      if (image === undefined) {
        image = images.push(jpegBytes(scan.data)) - 1;
        imageIndex.set(scan.data, image);
      }
      current.image = image;
      if (output === 'searchable') for (const line of scan.lines!) {
        if (!line.text.trim() || !clean(line.text)) continue;
        current.runs.push({ text: clean(line.text), x: line.x * width, y: (line.y + line.height * .82) * height,
          size: line.height * height, invisible: true, width: line.width * width });
      }
      continue;
    }
    newPage(595, 842);
    let y = 52;
    const ensure = (height: number) => { if (y + height > 790) { newPage(595, 842); y = 52; } };
    for (const block of parseDocument(scan.markdown!)) {
      if (block.kind === 'table') {
        const columns = Math.max(...block.rows.map(row => row.length)), cellWidth = 491 / columns;
        for (let rowIndex = 0; rowIndex < block.rows.length; rowIndex++) {
          const row = block.rows[rowIndex], bold = rowIndex === 0;
          const cells = Array.from({ length: columns }, (_, col) => wrap(row[col] || '', cellWidth - 12, 10, bold));
          let offset = 0;
          const total = Math.max(...cells.map(cell => cell.length));
          while (offset < total) {
            ensure(30);
            const count = Math.min(total - offset, Math.floor((790 - y - 12) / 14));
            const height = count * 14 + 12;
            for (let col = 0; col < columns; col++) {
              page!.boxes.push({ x: 52 + col * cellWidth, y, width: cellWidth, height, lineWidth: 0.5, color: RULE_COLOR });
              cells[col].slice(offset, offset + count).forEach((line, i) => text(line, 58 + col * cellWidth, y + 16 + i * 14, 10, bold));
            }
            offset += count; y += height;
          }
        }
        y += 16;
      } else {
        const size = block.kind === 'heading' ? [24, 18, 14][(block.level || 1) - 1] : 11;
        const bold = block.kind === 'heading';
        const indent = block.kind === 'list' ? 12 : 0;
        const wrapped = wrap(block.text, 491 - indent, size, bold);
        ensure(Math.min(3, wrapped.length) * size * 1.45);
        for (const line of wrapped) { ensure(size * 1.45); text(line, 52 + indent, y + size, size, bold); y += size * 1.45; }
        y += block.kind === 'heading' ? 12 : 8;
      }
    }
  }
  return { images, pages: out };
}

/**
 * Reopens a composed scan in a fresh engine session: it must open with `pageCount` pages and pass the engine's
 * strict check of a saved copy (no repair). The caller keeps the scans when this throws.
 */
export function checkScanPDF(engine: ScanEngine, bytes: ArrayBuffer, pageCount: number) {
  const failed = new Error('The new PDF did not pass validation. Your scans are still here.');
  withSession(engine, send => {
    const opened = send({ op: 'open', pdf: bytes.slice(0), name: 'scan.pdf' });
    if (!opened.ok || opened.pageCount !== pageCount) throw failed;
    if (!send({ op: 'validate', pdf: bytes.slice(0) }).ok) throw failed;
  });
}
