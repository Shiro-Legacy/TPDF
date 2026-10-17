/**
 * The TrueType fallback font the scanner draws non-WinAnsi text with (the Rust build's Droid Sans Fallback,
 * scripts/cjk-font.mjs): glyph lookup, advances, and a subset holding only the glyphs a PDF uses.
 * The font is the app's own pinned file, not user input.
 */

export type TrueType = {
  unitsPerEm: number;
  ascent: number;
  descent: number;
  capHeight: number;
  bbox: [number, number, number, number];
  /** The glyph for a Unicode code point, 0 when the font has none. */
  glyph(code: number): number;
  /** A glyph's advance in font units. */
  advance(gid: number): number;
  /**
   * A font file with exactly these glyphs, renumbered: glyph i of the subset is `gids[i]` of this font
   * (`gids[0]` must be 0). Composite glyphs' parts are appended after them.
   */
  subset(gids: number[]): Uint8Array;
};

/** Tables a CIDFontType2 program carries (ISO 32000-1, 9.9); the rest are left out of the subset. */
const KEEP = ['cvt ', 'fpgm', 'prep'];

export function parseTrueType(bytes: Uint8Array): TrueType {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  // A copy of a range (a Node Buffer's slice would be a view, and the subset edits its copies).
  const copy = (start: number, end: number) => new Uint8Array(bytes.subarray(start, end));
  const tables = new Map<string, { offset: number; length: number }>();
  const tag = (at: number) => String.fromCharCode(bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]);
  for (let i = 0, count = view.getUint16(4); i < count; i++) {
    const at = 12 + 16 * i;
    tables.set(tag(at), { offset: view.getUint32(at + 8), length: view.getUint32(at + 12) });
  }
  const table = (name: string) => {
    const found = tables.get(name);
    if (!found || found.offset + found.length > bytes.length) throw new Error(`The fallback font has no ${name} table.`);
    return found;
  };
  const head = table('head').offset, hhea = table('hhea').offset, maxp = table('maxp').offset;
  const hmtx = table('hmtx').offset, loca = table('loca').offset, glyf = table('glyf').offset;
  const unitsPerEm = view.getUint16(head + 18);
  const longLoca = view.getInt16(head + 50) === 1;
  const glyphCount = view.getUint16(maxp + 4);
  const hMetrics = view.getUint16(hhea + 34);
  const os2 = tables.get('OS/2');
  const ascent = view.getInt16(hhea + 4), descent = view.getInt16(hhea + 6);
  const capHeight = os2 && view.getUint16(os2.offset) >= 2 ? view.getInt16(os2.offset + 88) : Math.round(ascent * .7);
  const bbox: [number, number, number, number] = [view.getInt16(head + 36), view.getInt16(head + 38), view.getInt16(head + 40), view.getInt16(head + 42)];

  const lookup = cmap(view, table('cmap').offset);
  const advance = (gid: number) => view.getUint16(hmtx + 4 * Math.min(gid, hMetrics - 1));
  const lsb = (gid: number) => gid < hMetrics ? view.getInt16(hmtx + 4 * gid + 2) : view.getInt16(hmtx + 4 * hMetrics + 2 * (gid - hMetrics));
  const range = (gid: number): [number, number] => longLoca
    ? [view.getUint32(loca + 4 * gid), view.getUint32(loca + 4 * gid + 4)]
    : [view.getUint16(loca + 2 * gid) * 2, view.getUint16(loca + 2 * gid + 2) * 2];

  function subset(gids: number[]): Uint8Array {
    if (gids[0] !== 0) throw new Error('A font subset starts with glyph 0.');
    const order = gids.slice();
    const index = new Map<number, number>();
    order.forEach((gid, i) => { if (!index.has(gid)) index.set(gid, i); });
    // Composite glyphs name their parts by glyph id: those parts join the subset.
    for (let i = 0; i < order.length; i++) {
      for (const part of components(order[i])) {
        if (!index.has(part)) { index.set(part, order.length); order.push(part); }
      }
    }
    const glyphs: Uint8Array[] = [];
    const offsets = [0];
    let total = 0;
    for (const gid of order) {
      const [start, end] = gid < glyphCount ? range(gid) : [0, 0];
      const data = copy(glyf + start, glyf + Math.max(start, end));
      if (data.length >= 10 && new DataView(data.buffer).getInt16(0) < 0) renumber(data, index);
      const padded = new Uint8Array((data.length + 3) & ~3);
      padded.set(data);
      glyphs.push(padded);
      total += padded.length;
      offsets.push(total);
    }
    const out = new Map<string, Uint8Array>();
    const headTable = copy(head, head + 54);
    new DataView(headTable.buffer).setUint32(8, 0);
    new DataView(headTable.buffer).setInt16(50, 1);
    out.set('head', headTable);
    const hheaTable = copy(hhea, hhea + 36);
    new DataView(hheaTable.buffer).setUint16(34, order.length);
    out.set('hhea', hheaTable);
    const maxpTable = copy(maxp, maxp + table('maxp').length);
    new DataView(maxpTable.buffer).setUint16(4, order.length);
    out.set('maxp', maxpTable);
    const metrics = new DataView(new ArrayBuffer(4 * order.length));
    order.forEach((gid, i) => { metrics.setUint16(4 * i, advance(gid)); metrics.setInt16(4 * i + 2, lsb(gid)); });
    out.set('hmtx', new Uint8Array(metrics.buffer));
    const locaTable = new DataView(new ArrayBuffer(4 * offsets.length));
    offsets.forEach((offset, i) => locaTable.setUint32(4 * i, offset));
    out.set('loca', new Uint8Array(locaTable.buffer));
    const glyfTable = new Uint8Array(total);
    let at = 0;
    for (const glyph of glyphs) { glyfTable.set(glyph, at); at += glyph.length; }
    out.set('glyf', glyfTable);
    // post version 3: no glyph names.
    const post = new Uint8Array(32);
    new DataView(post.buffer).setUint32(0, 0x00030000);
    out.set('post', post);
    for (const name of KEEP) {
      const found = tables.get(name);
      if (found) out.set(name, copy(found.offset, found.offset + found.length));
    }
    return writeFont(out);
  }

  function components(gid: number): number[] {
    if (gid >= glyphCount) return [];
    const [start, end] = range(gid);
    if (end - start < 10 || view.getInt16(glyf + start) >= 0) return [];
    const parts: number[] = [];
    walkComponents(view, glyf + start + 10, glyf + end, at => parts.push(view.getUint16(at)));
    return parts;
  }

  return {
    unitsPerEm, ascent, descent, capHeight, bbox,
    glyph: code => { const gid = lookup(code); return gid > 0 && gid < glyphCount ? gid : 0; },
    advance: gid => advance(gid < glyphCount ? gid : 0),
    subset,
  };
}

/** Calls `visit` with the offset of each component's glyph index in a composite glyph's component list. */
function walkComponents(view: DataView, at: number, end: number, visit: (glyphIndexAt: number) => void) {
  for (;;) {
    if (at + 4 > end) return;
    const flags = view.getUint16(at);
    visit(at + 2);
    at += 4 + (flags & 0x0001 ? 4 : 2);
    if (flags & 0x0008) at += 2;
    else if (flags & 0x0040) at += 4;
    else if (flags & 0x0080) at += 8;
    if (!(flags & 0x0020)) return;
  }
}

/** Rewrites a composite glyph's component ids to their subset ids. */
function renumber(data: Uint8Array, index: Map<number, number>) {
  const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
  walkComponents(view, 10, data.length, at => view.setUint16(at, index.get(view.getUint16(at)) ?? 0));
}

/** Unicode to glyph through the (3,10) format 12 subtable, else the (3,1) format 4 one. */
function cmap(view: DataView, at: number): (code: number) => number {
  let format4 = -1, format12 = -1;
  for (let i = 0, count = view.getUint16(at + 2); i < count; i++) {
    const platform = view.getUint16(at + 4 + 8 * i), encoding = view.getUint16(at + 6 + 8 * i);
    const sub = at + view.getUint32(at + 8 + 8 * i);
    const format = view.getUint16(sub);
    if (platform === 3 && encoding === 10 && format === 12) format12 = sub;
    if (platform === 3 && encoding === 1 && format === 4) format4 = sub;
  }
  if (format12 >= 0) {
    const groups = view.getUint32(format12 + 12);
    return code => {
      let low = 0, high = groups - 1;
      while (low <= high) {
        const mid = (low + high) >> 1, group = format12 + 16 + 12 * mid;
        if (code < view.getUint32(group)) high = mid - 1;
        else if (code > view.getUint32(group + 4)) low = mid + 1;
        else return view.getUint32(group + 8) + code - view.getUint32(group);
      }
      return 0;
    };
  }
  if (format4 < 0) throw new Error('The fallback font has no Unicode cmap.');
  const segments = view.getUint16(format4 + 6) / 2;
  const ends = format4 + 14, starts = ends + 2 * segments + 2, deltas = starts + 2 * segments, ranges = deltas + 2 * segments;
  return code => {
    if (code > 0xffff) return 0;
    for (let i = 0; i < segments; i++) {
      if (code > view.getUint16(ends + 2 * i)) continue;
      const start = view.getUint16(starts + 2 * i);
      if (code < start) return 0;
      const delta = view.getUint16(deltas + 2 * i), offset = view.getUint16(ranges + 2 * i);
      if (!offset) return (code + delta) & 0xffff;
      const glyph = view.getUint16(ranges + 2 * i + offset + 2 * (code - start));
      return glyph ? (glyph + delta) & 0xffff : 0;
    }
    return 0;
  };
}

/** An sfnt file from its tables, with checksums and the head checksum adjustment. */
function writeFont(tables: Map<string, Uint8Array>): Uint8Array {
  const names = [...tables.keys()].sort();
  const count = names.length;
  let size = 12 + 16 * count;
  for (const name of names) size += (tables.get(name)!.length + 3) & ~3;
  const out = new Uint8Array(size), view = new DataView(out.buffer);
  const power = 2 ** Math.floor(Math.log2(count));
  view.setUint32(0, 0x00010000);
  view.setUint16(4, count);
  view.setUint16(6, power * 16);
  view.setUint16(8, Math.log2(power));
  view.setUint16(10, count * 16 - power * 16);
  let offset = 12 + 16 * count, headAt = 0;
  names.forEach((name, i) => {
    const data = tables.get(name)!, record = 12 + 16 * i;
    for (let k = 0; k < 4; k++) out[record + k] = name.charCodeAt(k);
    out.set(data, offset);
    view.setUint32(record + 4, checksum(out, offset, data.length));
    view.setUint32(record + 8, offset);
    view.setUint32(record + 12, data.length);
    if (name === 'head') headAt = offset;
    offset += (data.length + 3) & ~3;
  });
  view.setUint32(headAt + 8, (0xb1b0afba - checksum(out, 0, out.length)) >>> 0);
  return out;
}

function checksum(bytes: Uint8Array, at: number, length: number): number {
  let sum = 0;
  for (let i = 0; i < length; i += 4) {
    sum = (sum + ((bytes[at + i] << 24) | ((bytes[at + i + 1] ?? 0) << 16) | ((bytes[at + i + 2] ?? 0) << 8) | (bytes[at + i + 3] ?? 0))) >>> 0;
  }
  return sum;
}
