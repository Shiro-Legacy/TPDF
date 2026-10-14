import type * as MuPDF from '../../src/renderer/vendor/mupdf.js';
import { parseDocument, validatePages, type ScanPage, type ScanOutput } from './scan-model.js';

/** Shared by the isolated worker and PDF round-trip tests. */
export function createScanPDF(m: typeof MuPDF, pages: ScanPage[], output: ScanOutput): ArrayBuffer {
  validatePages(pages, output);
  const buffer = new m.Buffer(), writer = new m.DocumentWriter(buffer, 'pdf', 'compress=yes');
  const regular = new m.Font('Helvetica'), bold = new m.Font('Helvetica-Bold');
  let device: MuPDF.Device | null = null;
  let pageCount = 0;
  function endPage() { if (device) { device.close(); device.destroy(); device = null; writer.endPage(); } }
  function newPage(width: number, height: number) {
    endPage();
    if (++pageCount > 100) throw new Error('The rebuilt document exceeds 100 pages.');
    device = writer.beginPage([0, 0, width, height]);
  }
  function text(value: string, x: number, y: number, size: number, font = regular, invisible = false, fitWidth?: number) {
    const content = new m.Text();
    try {
      // showString uses MuPDF's fallback fonts and writes Unicode mappings into the PDF.
      const end = content.showString(font, [size, 0, 0, -size, 0, 0], value);
      const scale = fitWidth !== undefined && end[4] > 0 ? fitWidth / end[4] : 1;
      const matrix: MuPDF.Matrix = [scale, 0, 0, 1, x, y];
      if (invisible) device!.ignoreText(content, matrix);
      else device!.fillText(content, matrix, m.ColorSpace.DeviceRGB, [.12, .16, .15], 1);
    } finally { content.destroy(); }
  }
  function width(value: string, size: number, font: MuPDF.Font) {
    const t = new m.Text();
    try { return t.showString(font, [size, 0, 0, -size, 0, 0], value)[4]; }
    finally { t.destroy(); }
  }
  function wrap(value: string, maxWidth: number, size: number, font: MuPDF.Font) {
    const result: string[] = [];
    let line = '';
    // Wrap at words where possible; split long tokens and CJK without dropping text.
    const words = Array.from(new Intl.Segmenter(undefined, { granularity: 'word' }).segment(value), s => s.segment);
    for (const word of words) {
      if (width(line + word, size, font) <= maxWidth) { line += word; continue; }
      if (line.trim()) result.push(line.trimEnd());
      line = '';
      for (const char of word.trimStart()) {
        if (line && width(line + char, size, font) > maxWidth) { result.push(line); line = ''; }
        line += char;
      }
    }
    if (line.trim()) result.push(line.trimEnd());
    return result.length ? result : [''];
  }
  try {
    for (const scan of pages) {
      if (output !== 'digital') {
        const width = scan.width / Math.max(scan.width, scan.height) * 792;
        const height = scan.height / Math.max(scan.width, scan.height) * 792;
        newPage(width, height);
        const bytes = Uint8Array.from(atob(scan.data.split(',')[1]), c => c.charCodeAt(0));
        const image = new m.Image(bytes);
        try { device!.fillImage(image, [width, 0, 0, height, 0, 0], 1); }
        finally { image.destroy(); }
        if (output === 'searchable') for (const line of scan.lines!) {
          if (line.text.trim()) text(line.text, line.x * width, (line.y + line.height * .82) * height, line.height * height, regular, true, line.width * width);
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
            const row = block.rows[rowIndex], font = rowIndex === 0 ? bold : regular;
            const cells = Array.from({ length: columns }, (_, col) => wrap(row[col] || '', cellWidth - 12, 10, font));
            let offset = 0;
            const total = Math.max(...cells.map(cell => cell.length));
            while (offset < total) {
              ensure(30);
              const count = Math.min(total - offset, Math.floor((790 - y - 12) / 14));
              const height = count * 14 + 12;
              for (let col = 0; col < columns; col++) {
                const path = new m.Path(), stroke = new m.StrokeState({ lineWidth: .5, lineCap: 'Butt', lineJoin: 'Miter', miterLimit: 10 });
                try {
                  path.rect(52 + col * cellWidth, y, 52 + (col + 1) * cellWidth, y + height);
                  device!.strokePath(path, stroke, m.Matrix.identity, m.ColorSpace.DeviceRGB, [.75, .78, .76], 1);
                } finally { path.destroy(); stroke.destroy(); }
                cells[col].slice(offset, offset + count).forEach((line, i) => text(line, 58 + col * cellWidth, y + 16 + i * 14, 10, font));
              }
              offset += count; y += height;
            }
          }
          y += 16;
        } else {
          const size = block.kind === 'heading' ? [24, 18, 14][(block.level || 1) - 1] : 11;
          const font = block.kind === 'heading' ? bold : regular;
          const indent = block.kind === 'list' ? 12 : 0;
          const wrapped = wrap(block.text, 491 - indent, size, font);
          ensure(Math.min(3, wrapped.length) * size * 1.45);
          for (const line of wrapped) { ensure(size * 1.45); text(line, 52 + indent, y + size, size, font); y += size * 1.45; }
          y += block.kind === 'heading' ? 12 : 8;
        }
      }
    }
    endPage(); writer.close();
    const bytes = new Uint8Array(buffer.asUint8Array());
    if (bytes.byteLength > 50 * 1024 * 1024) throw new Error('The PDF exceeds the 50 MB output limit.');
    const check = m.Document.openDocument(bytes, 'application/pdf');
    try {
      const pdf = check.asPDF();
      if (!pdf || pdf.wasRepaired() || check.countPages() !== pageCount) throw new Error('The new PDF did not pass validation. Your scans are still here.');
      pdf.disableJS();
      for (let index = 0; index < pageCount; index++) { const page = check.loadPage(index); page.destroy(); }
    } finally { check.destroy(); }
    return bytes.buffer;
  } finally {
    if (device) { try { endPage(); } catch { /* Preserve the original error. */ } }
    writer.destroy(); buffer.destroy(); regular.destroy(); bold.destroy();
  }
}
