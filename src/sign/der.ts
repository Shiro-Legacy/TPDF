/** Minimal DER. Promoted from scripts/archive/p5/cms.mjs. No dependency. */

export type Tlv = { tag: number; start: number; valueStart: number; valueEnd: number; end: number };

export const cat = (parts: Buffer[]): Buffer => Buffer.concat(parts);

export function derLen(n: number): Buffer {
  if (n < 0x80) return Buffer.from([n]);
  const bytes: number[] = [];
  let x = n;
  while (x) { bytes.unshift(x & 0xff); x >>= 8; }
  return Buffer.from([0x80 | bytes.length, ...bytes]);
}

export function tlv(tag: number, content: Buffer | string): Buffer {
  const body = Buffer.isBuffer(content) ? content : Buffer.from(content);
  return cat([Buffer.from([tag]), derLen(body.length), body]);
}

export function seq(parts: Buffer[]): Buffer {
  return tlv(0x30, cat(parts));
}

export function derSet(parts: Buffer[]): Buffer {
  return tlv(0x31, cat(parts.slice().sort(Buffer.compare)));
}

export function oid(dotted: string): Buffer {
  const arcs = dotted.split('.').map(Number);
  const body = [arcs[0] * 40 + arcs[1]];
  for (const arc of arcs.slice(2)) {
    const chunk = [arc & 0x7f];
    let rest = arc >> 7;
    while (rest) { chunk.push((rest & 0x7f) | 0x80); rest >>= 7; }
    body.push(...chunk.reverse());
  }
  return tlv(0x06, Buffer.from(body));
}

export function utcTime(date: Date): Buffer {
  const p = (n: number) => String(n).padStart(2, '0');
  const text = `${p(date.getUTCFullYear() % 100)}${p(date.getUTCMonth() + 1)}${p(date.getUTCDate())}`
    + `${p(date.getUTCHours())}${p(date.getUTCMinutes())}${p(date.getUTCSeconds())}Z`;
  return tlv(0x17, text);
}

export function parseTlv(buf: Buffer, offset = 0): Tlv {
  const tag = buf[offset];
  let length = buf[offset + 1];
  let cursor = offset + 2;
  if (length & 0x80) {
    const count = length & 0x7f;
    length = 0;
    for (let i = 0; i < count; i++) length = (length << 8) | buf[cursor++];
  }
  return { tag, start: offset, valueStart: cursor, valueEnd: cursor + length, end: cursor + length };
}

export function kids(buf: Buffer, node: Tlv): Tlv[] {
  const out: Tlv[] = [];
  for (let cursor = node.valueStart; cursor < node.valueEnd;) {
    const child = parseTlv(buf, cursor);
    out.push(child);
    cursor = child.end;
  }
  return out;
}

export function oidOf(buf: Buffer, node: Tlv): string {
  const body = buf.subarray(node.valueStart, node.valueEnd);
  const arcs = [Math.floor(body[0] / 40), body[0] % 40];
  let value = 0;
  for (const byte of body.subarray(1)) {
    value = (value << 7) | (byte & 0x7f);
    if (!(byte & 0x80)) { arcs.push(value); value = 0; }
  }
  return arcs.join('.');
}

export function pemDer(pem: string | Buffer): Buffer {
  return Buffer.from(String(pem).replace(/-----[^-]+-----/g, '').replace(/\s/g, ''), 'base64');
}

/** Issuer Name and serial INTEGER, copied so the DN bytes match the cert. */
export function issuerAndSerial(certDer: Buffer): { serial: Buffer; issuer: Buffer } {
  const tbs = kids(certDer, parseTlv(certDer))[0];
  const parts = kids(certDer, tbs);
  const skip = parts[0].tag === 0xa0 ? 1 : 0;
  return {
    serial: certDer.subarray(parts[skip].start, parts[skip].end),
    issuer: certDer.subarray(parts[skip + 2].start, parts[skip + 2].end),
  };
}
