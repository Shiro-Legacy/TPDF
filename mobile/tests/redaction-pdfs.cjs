// Synthetic PDFs for the redaction browser test, written here (classic xref, Latin-1 bodies) as the engine's own
// tests write theirs (engine/tests/app_redact_boxes.rs `classic`, `mesh_pages`).
function classic(objects) {
  let out = '%PDF-1.7\n'; const offsets = [];
  objects.forEach((body, i) => { offsets.push(out.length); out += `${i + 1} 0 obj\n${body}\nendobj\n`; });
  const xref = out.length;
  out += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.map(o => `${String(o).padStart(10, '0')} 00000 n \n`).join('')}`;
  out += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(out, 'latin1');
}
const stream = (dict, content) => `<< ${dict} /Length ${content.length} >>\nstream\n${content}\nendstream`;
const helvetica = '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>';
const courier = `<< /Type /Font /Subtype /Type1 /BaseFont /Courier /FirstChar 32 /LastChar 126 /Widths [${Array(95).fill(600).join(' ')}] /Encoding /WinAnsiEncoding >>`;

/** Page 1 plain; page 2 cropped (CropBox 50 60 562 732) and turned a quarter (Rotate 90), with AAAASECRETAAAA at 24 pt. */
function rotatedPages() {
  const page = (contents, extra) => `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] ${extra} /Resources << /Font << /F1 5 0 R >> >> /Contents ${contents} 0 R >>`;
  return classic([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>',
    page(6, ''),
    page(7, '/CropBox [50 60 562 732] /Rotate 90'),
    helvetica,
    stream('', 'BT /F1 18 Tf 72 700 Td (Page one) Tj ET'),
    stream('', 'BT /F1 24 Tf 100 600 Td (AAAASECRETAAAA) Tj ET'),
  ]);
}

/** Page 1: a mesh shading (Gouraud triangles) over its top half with MESHSECRET drawn on it, and KEEPTEXT lower down; page 2: PAGETWO. */
function meshPages() {
  const tri = [0, 0, 128, 0, 0, 255, 128, 128, 0, 0, 255, 255, 0, 255, 128, 64, 0, 255, 255, 200, 0, 0, 255, 32];
  const mesh = stream('/ShadingType 4 /ColorSpace /DeviceGray /BitsPerCoordinate 8 /BitsPerComponent 8 /BitsPerFlag 8 /Decode [0 612 0 792 0 1]', String.fromCharCode(...tri));
  const page = contents => `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> /Shading << /Sh 6 0 R >> >> /Contents ${contents} 0 R >>`;
  return classic([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>',
    page(7), page(8), courier, mesh,
    stream('', 'q 0 396 612 396 re W n /Sh sh Q BT /F1 20 Tf 100 600 Td (MESHSECRET) Tj ET BT /F1 20 Tf 100 200 Td (KEEPTEXT) Tj ET'),
    stream('', 'BT /F1 20 Tf 100 600 Td (PAGETWO) Tj ET'),
  ]);
}

/** One header Form drawn on both pages through inherited resources (engine test e3_partial_boxes_where): boxing the
 *  account number on both pages and the name on page 1 only removes the name from page 2 too (over-redaction). */
function sharedHeader() {
  const res = '<< /Font << /F1 5 0 R >> /XObject << /Hdr 7 0 R >> >>';
  return classic([
    '<< /Type /Catalog /Pages 2 0 R >>',
    `<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /Resources ${res} >>`,
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 8 0 R >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 8 0 R >>',
    courier, 'null',
    stream('/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >>', 'BT /F1 20 Tf 100 400 Td (NAMEVISIBLE) Tj ET BT /F1 20 Tf 100 300 Td (ACCT-4417-0092) Tj ET'),
    stream('', '/Hdr Do'),
  ]);
}

module.exports = { rotatedPages, meshPages, sharedHeader };
