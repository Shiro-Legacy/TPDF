// SPDX-License-Identifier: MIT
/**
 * Minimal text-PDF builder for the scratch documents the tests make at run
 * time: one or more pages of Base-14 text runs, written byte by byte (a
 * classic xref, uncompressed streams), no PDF library involved.
 *
 * A run is `{font: {name, resource}, text, size, x, y, matrix?}`: `name` a
 * Base-14 font (Helvetica, Times-Roman, Courier, ...) drawn with
 * WinAnsiEncoding, `resource` its name in the page's /Font dictionary, `x y`
 * the text origin in PDF user space (y up), `matrix` the first four numbers
 * of the text matrix (default `[1, 0, 0, 1]`). Text is limited to characters
 * WinAnsi can hold (Latin-1); the committed cjk.pdf and rtl.pdf fixtures,
 * which needed embedded fonts, were generated with a tool that has since
 * been retired (their provenance records say so) and are pinned by sha256.
 */

const literal = (s) => s.replace(/([\\()])/g, '\\$1');
const num = (v) => String(Math.round(v * 10000) / 10000);

/** `text` as WinAnsi bytes in a PDF literal string; a character outside Latin-1 is refused. */
function winAnsiLiteral(text) {
  for (const ch of text) {
    const code = ch.codePointAt(0);
    if (code > 0xff || (code >= 0x80 && code < 0xa0)) {
      throw new Error(`textpdf: ${JSON.stringify(ch)} (U+${code.toString(16)}) is not a WinAnsi character`);
    }
  }
  return `(${literal(text)})`;
}

export function makeTextPdf({ width = 612, height = 792, runs }) {
  return makeTextPages([{ width, height, runs }]);
}

/** One page per entry, each with its own size and runs: the multi-page scratch builder. */
export function makeTextPages(pages) {
  // Object 1 catalog, 2 pages, then one font object per distinct resource, then page + content pairs.
  const fonts = new Map();
  for (const { runs } of pages) for (const { font } of runs) {
    if (fonts.has(font.resource)) continue;
    if (font.bytes) throw new Error('textpdf: embedded fonts are not supported; use a Base-14 font name');
    fonts.set(font.resource, { name: font.name, object: 3 + fonts.size });
  }
  const firstPage = 3 + fonts.size;
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    `<< /Type /Pages /Kids [${pages.map((_, i) => `${firstPage + 2 * i} 0 R`).join(' ')}] /Count ${pages.length} >>`,
  ];
  for (const { name } of fonts.values()) {
    objects.push(`<< /Type /Font /Subtype /Type1 /BaseFont /${name} /Encoding /WinAnsiEncoding >>`);
  }
  const fontDict = [...fonts.entries()].map(([resource, { object }]) => `/${resource} ${object} 0 R`).join(' ');
  pages.forEach(({ width = 612, height = 792, runs }, index) => {
    const content = runs
      .map(({ font, text, size, x, y, matrix = [1, 0, 0, 1] }) =>
        `BT /${font.resource} ${num(size)} Tf ${matrix.map(num).join(' ')} ${num(x)} ${num(y)} Tm ${winAnsiLiteral(text)} Tj ET`)
      .join('\n') + '\n';
    const contentObject = firstPage + 2 * index + 1;
    objects.push(
      `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 ${num(width)} ${num(height)}] `
        + `/Resources << /Font << ${fontDict} >> >> /Contents ${contentObject} 0 R >>`,
    );
    objects.push(`<< /Length ${Buffer.byteLength(content, 'latin1')} >>\nstream\n${content}endstream`);
  });
  const parts = ['%PDF-1.7\n'];
  const offsets = [];
  let length = parts[0].length;
  objects.forEach((body, index) => {
    offsets.push(length);
    const text = `${index + 1} 0 obj\n${body}\nendobj\n`;
    parts.push(text);
    length += Buffer.byteLength(text, 'latin1');
  });
  const xref = length;
  parts.push(`xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`);
  for (const offset of offsets) parts.push(`${String(offset).padStart(10, '0')} 00000 n \n`);
  parts.push(`trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`);
  return new Uint8Array(Buffer.from(parts.join(''), 'latin1'));
}
