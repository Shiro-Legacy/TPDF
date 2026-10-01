// contract-agent-session.md D4: the bytes of a one-page blank PDF, built by hand (no Electron,
// no mupdf) so main can write it and a test can read it back. The page has an empty
// /Resources and an empty content stream; every xref offset is computed from the bytes.

/** The sizes File → New… and the drive `new` step accept, in points. */
export const PAGE_SIZES = { letter: [612, 792], a4: [595.276, 841.89] } as const;
export const MIN_PAGE_PT = 72;
export const MAX_PAGE_PT = 14400;

/** `"letter"` (also the default), `"a4"` or `[w, h]` with each side in 72…14400 pt; else undefined. */
export function pageSizeOf(size: unknown): [number, number] | undefined {
  if (size === undefined || size === 'letter') return [...PAGE_SIZES.letter];
  if (size === 'a4') return [...PAGE_SIZES.a4];
  if (!Array.isArray(size) || size.length !== 2) return;
  const [w, h] = size;
  const ok = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v) && v >= MIN_PAGE_PT && v <= MAX_PAGE_PT;
  return ok(w) && ok(h) ? [w, h] : undefined;
}

const num = (v: number) => String(Math.round(v * 1000) / 1000);

export function blankPdf(width: number, height: number): Uint8Array {
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 ${num(width)} ${num(height)}] /Resources << >> /Contents 4 0 R >>`,
    '<< /Length 0 >>\nstream\n\nendstream',
  ];
  let text = '%PDF-1.7\n';
  const offsets: number[] = [];
  objects.forEach((body, i) => {
    offsets.push(text.length);
    text += `${i + 1} 0 obj\n${body}\nendobj\n`;
  });
  const xref = text.length;
  text += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const offset of offsets) text += `${String(offset).padStart(10, '0')} 00000 n \n`;
  text += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return new TextEncoder().encode(text);
}
