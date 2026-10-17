import { winAnsiByte } from '../../src/renderer/engine-core.js';
import { parseDocument, validatePages, type ScanPage, type ScanOutput } from './scan-model.js';
import { HELVETICA, HELVETICA_BOLD } from './scan-metrics.js';
import { parseTrueType, type TrueType } from './scan-font.js';

/**
 * What composing a scan needs from its host: the fallback font for text outside WinAnsi (read only when such
 * text is there) and, where the platform has one, a zlib compressor for content and font streams.
 */
export type ScanHost = {
  fallbackFont(): Promise<Uint8Array>;
  deflate?: (bytes: Uint8Array) => Promise<Uint8Array>;
};

/** The engine a composed scan is checked with: the scan worker's Rust engine (engine-rust.ts), or a test's. */
export type ScanEngine = {
  createSession(): unknown;
  dispatch(request: never, session: never): { reply: { ok: boolean; pageCount?: number } };
};

const MAX_OUTPUT_PAGES = 100;
const MAX_OUTPUT_BYTES = 50 * 1024 * 1024;
const TEXT_COLOR = '0.12 0.16 0.15';
const RULE_COLOR = '0.75 0.78 0.76';

/** zlib through CompressionStream (Safari 16.4+, Chrome 80+, Node 18+); without it streams stay uncompressed. */
export function platformDeflate(): ScanHost['deflate'] {
  if (typeof CompressionStream === 'undefined') return undefined;
  return async bytes => new Uint8Array(await new Response(new Blob([bytes as BlobPart]).stream().pipeThrough(new CompressionStream('deflate'))).arrayBuffer());
}

type FontKey = 'F1' | 'F2';
type Run = { font: FontKey | 'F0'; codes: number[] };

/**
 * One scan PDF: each page's JPEG as it is (DCTDecode) on a page whose long side is 792 pt, with the OCR lines
 * as invisible text ('searchable'), or the reviewed Markdown typeset on A4 pages ('digital'). WinAnsi text uses
 * Helvetica and Helvetica-Bold (not embedded); other text a subset of the fallback font (Type0, Identity-H).
 * Every run carries its Unicode, so search and copy work. Shared by the scan worker and its tests.
 */
export async function createScanPDF(pages: ScanPage[], output: ScanOutput, host: ScanHost): Promise<{ bytes: ArrayBuffer; pageCount: number }> {
  validatePages(pages, output);
  const texts = output === 'searchable' ? pages.flatMap(page => page.lines!.map(line => line.text))
    : output === 'digital' ? pages.map(page => page.markdown!) : [];
  const needsFallback = texts.some(text => [...clean(text)].some(char => winAnsiByte(char.codePointAt(0)!) < 0));
  const pdf = new Writer(host.deflate);
  const catalog = pdf.reserve(), tree = pdf.reserve();
  const fonts: Record<FontKey, number> = { F1: pdf.reserve(), F2: pdf.reserve() };
  const type0 = needsFallback ? new FallbackFont(parseTrueType(await host.fallbackFont()), pdf.reserve()) : null;
  const widths: Record<FontKey, number[]> = { F1: HELVETICA, F2: HELVETICA_BOLD };
  const kids: number[] = [];
  const images = new Map<string, number>();

  function runs(value: string, font: FontKey): Run[] {
    const result: Run[] = [];
    for (const char of clean(value)) {
      const code = char.codePointAt(0)!, byte = winAnsiByte(code);
      const key = byte >= 0 ? font : 'F0';
      const unit = byte >= 0 ? byte : type0!.code(code);
      const last = result[result.length - 1];
      if (last?.font === key) last.codes.push(unit);
      else result.push({ font: key, codes: [unit] });
    }
    return result;
  }
  /** Advance at size 1, in points. */
  function measure(value: string, font: FontKey): number {
    let total = 0;
    for (const run of runs(value, font)) {
      for (const unit of run.codes) total += run.font === 'F0' ? type0!.width(unit) : widths[run.font][unit - 32] ?? 0;
    }
    return total / 1000;
  }

  let content: string[] = [], pageWidth = 0, pageHeight = 0, image = 0, used = new Set<string>();
  async function endPage() {
    if (!pageWidth) return;
    const stream = await pdf.stream('', new TextEncoder().encode(content.join('\n')));
    const fontEntries = [...used].sort().map(key => `/${key} ${key === 'F0' ? type0!.id : fonts[key as FontKey]} 0 R`).join(' ');
    const resources = [image ? `/XObject << /Im0 ${image} 0 R >>` : '', fontEntries ? `/Font << ${fontEntries} >>` : ''].filter(Boolean).join(' ');
    kids.push(pdf.add(`<< /Type /Page /Parent ${tree} 0 R /MediaBox [0 0 ${num(pageWidth)} ${num(pageHeight)}] /Resources << ${resources} >> /Contents ${stream} 0 R >>`));
    content = []; pageWidth = 0; image = 0; used = new Set();
  }
  async function newPage(width: number, height: number) {
    await endPage();
    if (kids.length + 1 > MAX_OUTPUT_PAGES) throw new Error('The rebuilt document exceeds 100 pages.');
    pageWidth = width; pageHeight = height;
  }
  /** Text whose baseline starts at (x, y), y measured down from the top of the page as the MuPDF scanner placed it. */
  function text(value: string, x: number, y: number, size: number, font: FontKey = 'F1', invisible = false, fitWidth?: number) {
    const parts = runs(value, font);
    if (!parts.length) return;
    const end = measure(value, font) * size;
    const scale = fitWidth !== undefined && end > 0 ? fitWidth / end : 1;
    // The rendering mode is graphics state and outlives ET, so each text object sets its own.
    const ops = [`BT ${invisible ? '3 Tr' : `0 Tr ${TEXT_COLOR} rg`} ${num(size * scale)} 0 0 ${num(size)} ${num(x)} ${num(pageHeight - y)} Tm`];
    for (const run of parts) {
      used.add(run.font);
      const hex = run.codes.map(unit => unit.toString(16).padStart(run.font === 'F0' ? 4 : 2, '0')).join('');
      ops.push(`/${run.font} 1 Tf <${hex}> Tj`);
    }
    ops.push('ET');
    content.push(ops.join(' '));
  }
  function wrap(value: string, maxWidth: number, size: number, font: FontKey) {
    const result: string[] = [];
    const width = (line: string) => measure(line, font) * size;
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
  }

  for (const scan of pages) {
    if (output !== 'digital') {
      const width = scan.width / Math.max(scan.width, scan.height) * 792;
      const height = scan.height / Math.max(scan.width, scan.height) * 792;
      await newPage(width, height);
      // The same image twice (a page added twice) is stored once, as MuPDF's writer did.
      image = images.get(scan.data) ?? 0;
      if (!image) {
        const jpeg = Uint8Array.from(atob(scan.data.split(',')[1]), c => c.charCodeAt(0));
        const { dictionary, icc, components } = jpegImage(jpeg);
        const space = icc ? `[/ICCBased ${await pdf.stream(`/N ${components} /Alternate /${['', 'DeviceGray', '', 'DeviceRGB', 'DeviceCMYK'][components]}`, icc)} 0 R]` : '';
        image = pdf.add(dictionary.replace('%ICC%', space), jpeg);
        images.set(scan.data, image);
      }
      content.push(`q ${num(width)} 0 0 ${num(height)} 0 0 cm /Im0 Do Q`);
      if (output === 'searchable') for (const line of scan.lines!) {
        if (line.text.trim()) text(line.text, line.x * width, (line.y + line.height * .82) * height, line.height * height, 'F1', true, line.width * width);
      }
      continue;
    }
    await newPage(595, 842);
    let y = 52;
    const ensure = async (height: number) => { if (y + height > 790) { await newPage(595, 842); y = 52; } };
    for (const block of parseDocument(scan.markdown!)) {
      if (block.kind === 'table') {
        const columns = Math.max(...block.rows.map(row => row.length)), cellWidth = 491 / columns;
        for (let rowIndex = 0; rowIndex < block.rows.length; rowIndex++) {
          const row = block.rows[rowIndex], font: FontKey = rowIndex === 0 ? 'F2' : 'F1';
          const cells = Array.from({ length: columns }, (_, col) => wrap(row[col] || '', cellWidth - 12, 10, font));
          let offset = 0;
          const total = Math.max(...cells.map(cell => cell.length));
          while (offset < total) {
            await ensure(30);
            const count = Math.min(total - offset, Math.floor((790 - y - 12) / 14));
            const height = count * 14 + 12;
            for (let col = 0; col < columns; col++) {
              content.push(`q 0.5 w 0 J 0 j 10 M ${RULE_COLOR} RG ${num(52 + col * cellWidth)} ${num(pageHeight - y - height)} ${num(cellWidth)} ${num(height)} re S Q`);
              cells[col].slice(offset, offset + count).forEach((line, i) => text(line, 58 + col * cellWidth, y + 16 + i * 14, 10, font));
            }
            offset += count; y += height;
          }
        }
        y += 16;
      } else {
        const size = block.kind === 'heading' ? [24, 18, 14][(block.level || 1) - 1] : 11;
        const font: FontKey = block.kind === 'heading' ? 'F2' : 'F1';
        const indent = block.kind === 'list' ? 12 : 0;
        const wrapped = wrap(block.text, 491 - indent, size, font);
        await ensure(Math.min(3, wrapped.length) * size * 1.45);
        for (const line of wrapped) { await ensure(size * 1.45); text(line, 52 + indent, y + size, size, font); y += size * 1.45; }
        y += block.kind === 'heading' ? 12 : 8;
      }
    }
  }
  await endPage();

  pdf.set(catalog, `<< /Type /Catalog /Pages ${tree} 0 R >>`);
  pdf.set(tree, `<< /Type /Pages /Kids [${kids.map(kid => `${kid} 0 R`).join(' ')}] /Count ${kids.length} >>`);
  for (const [key, base] of [['F1', 'Helvetica'], ['F2', 'Helvetica-Bold']] as const) {
    pdf.set(fonts[key], `<< /Type /Font /Subtype /Type1 /BaseFont /${base} /Encoding /WinAnsiEncoding /FirstChar 32 /LastChar 255 /Widths [${widths[key].join(' ')}] >>`);
  }
  if (type0) await type0.write(pdf);
  // The document information MuPDF's writer gave a scan was its Producer; this names the app instead.
  const bytes = pdf.finish(catalog, pdf.add('<< /Producer (Tandem PDF) >>'));
  if (bytes.byteLength > MAX_OUTPUT_BYTES) throw new Error('The PDF exceeds the 50 MB output limit.');
  return { bytes: bytes.buffer as ArrayBuffer, pageCount: kids.length };
}

/**
 * Reopens a composed scan in a fresh engine session: it must open with `pageCount` pages and pass the engine's
 * strict check of a saved copy (no repair). The caller keeps the scans when this throws.
 */
export function checkScanPDF(engine: ScanEngine, bytes: ArrayBuffer, pageCount: number) {
  type Send = (request: Record<string, unknown>) => { ok: boolean; pageCount?: number };
  const session = engine.createSession();
  let id = 0;
  const send: Send = request => (engine.dispatch as unknown as (r: unknown, s: unknown) => { reply: ReturnType<Send> })({ id: ++id, ...request }, session).reply;
  const failed = new Error('The new PDF did not pass validation. Your scans are still here.');
  try {
    const opened = send({ op: 'open', pdf: bytes.slice(0), name: 'scan.pdf' });
    if (!opened.ok || opened.pageCount !== pageCount) throw failed;
    if (!send({ op: 'validate', pdf: bytes.slice(0) }).ok) throw failed;
  } finally {
    send({ op: 'close' });
  }
}

/** Tabs become spaces; other control characters are dropped (they have no glyph in either font). */
function clean(value: string): string {
  return value.replace(/\t/g, ' ').replace(/[\u0000-\u001f\u007f-\u009f]/g, '');
}

function num(value: number): string {
  if (!Number.isFinite(value)) throw new Error('Invalid page geometry.');
  const fixed = value.toFixed(4).replace(/\.?0+$/, '');
  return fixed === '-0' ? '0' : fixed;
}

/**
 * The image XObject for a JPEG, from its headers as MuPDF read them: size and components from the frame header,
 * inverted Adobe CMYK, and an embedded ICC profile (APP2 ICC_PROFILE chunks) as an ICCBased colour space.
 */
function jpegImage(jpeg: Uint8Array): { dictionary: string; icc: Uint8Array | null; components: number } {
  if (jpeg[0] !== 0xff || jpeg[1] !== 0xd8) throw new Error('Invalid scan image.');
  let at = 2, adobe = false;
  const icc: [number, Uint8Array][] = [];
  while (at + 4 <= jpeg.length) {
    if (jpeg[at] !== 0xff) throw new Error('Invalid scan image.');
    const marker = jpeg[at + 1];
    if (marker === 0xff) { at++; continue; }
    const length = (jpeg[at + 2] << 8) | jpeg[at + 3];
    const tag = (text: string) => String.fromCharCode(...jpeg.subarray(at + 4, at + 4 + text.length)) === text;
    if (marker === 0xee && tag('Adobe')) adobe = true;
    if (marker === 0xe2 && length > 16 && tag('ICC_PROFILE\0')) icc.push([jpeg[at + 16], jpeg.subarray(at + 18, at + 2 + length)]);
    if (marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc) {
      const height = (jpeg[at + 5] << 8) | jpeg[at + 6], width = (jpeg[at + 7] << 8) | jpeg[at + 8], components = jpeg[at + 9];
      const space = ({ 1: '/DeviceGray', 3: '/DeviceRGB', 4: '/DeviceCMYK' } as Record<number, string>)[components];
      if (!width || !height || !space || jpeg[at + 4] !== 8) throw new Error('Invalid scan image.');
      const decode = components === 4 && adobe ? ' /Decode [1 0 1 0 1 0 1 0]' : '';
      const profile = icc.length ? concat(icc.sort((a, b) => a[0] - b[0]).map(([, chunk]) => chunk)) : null;
      return {
        dictionary: `/Type /XObject /Subtype /Image /Width ${width} /Height ${height} /ColorSpace ${profile ? '%ICC%' : space} /BitsPerComponent 8 /Filter /DCTDecode${decode}`,
        icc: profile, components,
      };
    }
    at += 2 + length;
  }
  throw new Error('Invalid scan image.');
}

function concat(parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((sum, part) => sum + part.length, 0));
  let at = 0;
  for (const part of parts) { out.set(part, at); at += part.length; }
  return out;
}

/** The fallback font as a Type0 font: one code (its subset glyph) per distinct character, each mapped to its Unicode. */
class FallbackFont {
  private codes = new Map<number, number>();
  private chars: number[] = [];
  constructor(private font: TrueType, readonly id: number) {}
  code(char: number): number {
    let code = this.codes.get(char);
    if (code === undefined) {
      code = this.chars.length + 1;
      if (code > 0xffff) throw new Error('Too many different characters for one PDF.');
      this.codes.set(char, code);
      this.chars.push(char);
    }
    return code;
  }
  /** Advance of a code in 1/1000 em. */
  width(code: number): number {
    return this.font.advance(this.font.glyph(this.chars[code - 1])) * 1000 / this.font.unitsPerEm;
  }
  async write(pdf: Writer) {
    const scale = (value: number) => Math.round(value * 1000 / this.font.unitsPerEm);
    const program = this.font.subset([0, ...this.chars.map(char => this.font.glyph(char))]);
    const file = await pdf.stream(`/Length1 ${program.length}`, program);
    const [x0, y0, x1, y1] = this.font.bbox.map(scale);
    const name = '/TPDFSB+DroidSansFallback';
    const descriptor = pdf.add(`<< /Type /FontDescriptor /FontName ${name} /Flags 4 /FontBBox [${x0} ${y0} ${x1} ${y1}] /ItalicAngle 0 /Ascent ${scale(this.font.ascent)} /Descent ${scale(this.font.descent)} /CapHeight ${scale(this.font.capHeight)} /StemV 80 /FontFile2 ${file} 0 R >>`);
    const widths = this.chars.map((_, i) => Math.round(this.width(i + 1)));
    const descendant = pdf.add(`<< /Type /Font /Subtype /CIDFontType2 /BaseFont ${name} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor ${descriptor} 0 R /W [1 [${widths.join(' ')}]] /DW 1000 /CIDToGIDMap /Identity >>`);
    const map = this.chars.map((char, i) => `<${(i + 1).toString(16).padStart(4, '0')}> <${utf16(char)}>`);
    const lines = ['/CIDInit /ProcSet findresource begin', '12 dict begin', 'begincmap', '/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def',
      '/CMapName /Adobe-Identity-UCS def', '/CMapType 2 def', '1 begincodespacerange', '<0000> <FFFF>', 'endcodespacerange'];
    for (let i = 0; i < map.length; i += 100) lines.push(`${Math.min(100, map.length - i)} beginbfchar`, ...map.slice(i, i + 100), 'endbfchar');
    lines.push('endcmap', 'CMapName currentdict /CMap defineresource pop', 'end', 'end');
    const unicode = await pdf.stream('', new TextEncoder().encode(lines.join('\n')));
    pdf.set(this.id, `<< /Type /Font /Subtype /Type0 /BaseFont ${name} /Encoding /Identity-H /DescendantFonts [${descendant} 0 R] /ToUnicode ${unicode} 0 R >>`);
  }
}

function utf16(code: number): string {
  const units = code > 0xffff ? [0xd800 + ((code - 0x10000) >> 10), 0xdc00 + ((code - 0x10000) & 0x3ff)] : [code];
  return units.map(unit => unit.toString(16).padStart(4, '0')).join('').toUpperCase();
}

/** A PDF 1.7 file with a classic cross-reference table. */
class Writer {
  private objects: (Uint8Array | null)[] = [];
  constructor(private deflate?: ScanHost['deflate']) {}
  reserve(): number { this.objects.push(null); return this.objects.length; }
  /** `body` is the whole object, or a stream's dictionary entries with `stream`. */
  set(id: number, body: string, stream?: Uint8Array) {
    const encoder = new TextEncoder();
    if (!stream) { this.objects[id - 1] = encoder.encode(`${id} 0 obj\n${body}\nendobj\n`); return; }
    const head = encoder.encode(`${id} 0 obj\n<< ${body.replace(/^<<|>>$/g, '').trim()} /Length ${stream.length} >>\nstream\n`);
    const tail = encoder.encode('\nendstream\nendobj\n');
    const all = new Uint8Array(head.length + stream.length + tail.length);
    all.set(head); all.set(stream, head.length); all.set(tail, head.length + stream.length);
    this.objects[id - 1] = all;
  }
  add(body: string, stream?: Uint8Array): number { const id = this.reserve(); this.set(id, body, stream); return id; }
  /** A stream object, Flate-compressed when the host can. */
  async stream(dictionary: string, data: Uint8Array): Promise<number> {
    if (!this.deflate) return this.add(dictionary, data);
    return this.add(`${dictionary} /Filter /FlateDecode`, await this.deflate(data));
  }
  finish(root: number, info: number): Uint8Array {
    const encoder = new TextEncoder();
    const header = new Uint8Array([...encoder.encode('%PDF-1.7\n%'), 0xe2, 0xe3, 0xcf, 0xd3, 0x0a]);
    const offsets: number[] = [];
    let size = header.length;
    for (const object of this.objects) {
      if (!object) throw new Error('A PDF object was reserved and never written.');
      offsets.push(size); size += object.length;
    }
    const xref = [`xref\n0 ${this.objects.length + 1}\n0000000000 65535 f \n`, ...offsets.map(offset => `${String(offset).padStart(10, '0')} 00000 n \n`),
      `trailer\n<< /Size ${this.objects.length + 1} /Root ${root} 0 R /Info ${info} 0 R >>\nstartxref\n${size}\n%%EOF\n`].join('');
    const tail = encoder.encode(xref);
    const out = new Uint8Array(size + tail.length);
    out.set(header);
    let at = header.length;
    for (const object of this.objects) { out.set(object!, at); at += object!.length; }
    out.set(tail, at);
    return out;
  }
}
