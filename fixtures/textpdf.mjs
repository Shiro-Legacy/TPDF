/**
 * Minimal one-page text-PDF builder for the fixtures, on top of MuPDF's
 * document-creation API.
 *
 * A font passed as `bytes` is embedded as a CID font (Identity-H, 2-byte
 * glyph codes) and then cut down by PDFDocument.subsetFonts(): that is what
 * keeps cjk.pdf / rtl.pdf at a few kB instead of megabytes. A font without
 * `bytes` is referenced as Base-14 and drawn with literal strings.
 */
import mupdf from 'mupdf';

const literal = (s) => s.replace(/([\\()])/g, '\\$1');

export function makeTextPdf({ width = 612, height = 792, runs }) {
  return makeTextPages([{ width, height, runs }]);
}

/** One page per entry, each with its own size and runs: the multi-page scratch builder. */
export function makeTextPages(pages) {
  const doc = new mupdf.PDFDocument();
  const fontDict = doc.newDictionary();
  const fonts = new Map();

  for (const { runs } of pages) for (const { font } of runs) {
    if (fonts.has(font.resource)) continue;
    if (font.bytes) {
      const handle = new mupdf.Font(font.name, font.bytes);
      fontDict.put(font.resource, doc.addFont(handle));
      fonts.set(font.resource, { handle, embedded: true });
    } else {
      const handle = new mupdf.Font(font.name);
      fontDict.put(font.resource, doc.addSimpleFont(handle, mupdf.Font.SIMPLE_ENCODING_LATIN));
      fonts.set(font.resource, { handle, embedded: false });
    }
  }

  const resources = doc.newDictionary();
  resources.put('Font', fontDict);

  for (const { width = 612, height = 792, runs } of pages) {
    const ops = runs
      .map(({ font, text, size, x, y, matrix = [1, 0, 0, 1] }) => {
        const { handle, embedded } = fonts.get(font.resource);
        let shown = `(${literal(text)})`;
        if (embedded) {
          const codes = [...text].map((ch) => {
            const code = handle.encodeCharacter(ch);
            if (!code) {
              throw new Error(
                `${font.name} has no glyph for U+${ch.codePointAt(0).toString(16)}`,
              );
            }
            return code.toString(16).padStart(4, '0');
          });
          shown = `<${codes.join('')}>`;
        }
        return `BT /${font.resource} ${size} Tf ${matrix.join(' ')} ${x} ${y} Tm ${shown} Tj ET`;
      })
      .join('\n');

    const page = doc.addPage(
      [0, 0, width, height],
      0,
      resources,
      Buffer.from(`${ops}\n`, 'latin1'),
    );
    doc.insertPage(-1, page);
  }
  doc.subsetFonts();
  return doc.saveToBuffer({ compress: true, garbage: true }).asUint8Array();
}
