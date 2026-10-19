/**
 * TEST-ONLY OpenSSL oracle for the signing tests (docs/rust-and-ux-plan.md §1.5): node:crypto checks what the
 * tandem-sign module (crates/tandem-sign) writes, independently of it. Nothing here ships: the app signs and
 * verifies through build/sign/*.js, which load the Rust module. The DER helpers are the former src/sign/der.ts.
 */
import { createHash, createVerify, verify, X509Certificate } from 'node:crypto';

export function derLen(n) {
  if (n < 0x80) return Buffer.from([n]);
  const bytes = [];
  for (let x = n; x; x >>= 8) bytes.unshift(x & 0xff);
  return Buffer.from([0x80 | bytes.length, ...bytes]);
}

export function tlv(tag, content) {
  const body = Buffer.isBuffer(content) ? content : Buffer.from(content);
  return Buffer.concat([Buffer.from([tag]), derLen(body.length), body]);
}

export function derSet(parts) {
  return tlv(0x31, Buffer.concat(parts.slice().sort(Buffer.compare)));
}

export function oid(dotted) {
  const arcs = dotted.split('.').map(Number);
  const body = [arcs[0] * 40 + arcs[1]];
  for (const arc of arcs.slice(2)) {
    const chunk = [arc & 0x7f];
    for (let rest = arc >> 7; rest; rest >>= 7) chunk.push((rest & 0x7f) | 0x80);
    body.push(...chunk.reverse());
  }
  return tlv(0x06, Buffer.from(body));
}

export function parseTlv(buf, offset = 0) {
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

export function kids(buf, node) {
  const out = [];
  for (let cursor = node.valueStart; cursor < node.valueEnd;) {
    const child = parseTlv(buf, cursor);
    out.push(child);
    cursor = child.end;
  }
  return out;
}

export function oidOf(buf, node) {
  const body = buf.subarray(node.valueStart, node.valueEnd);
  const arcs = [Math.floor(body[0] / 40), body[0] % 40];
  let value = 0;
  for (const byte of body.subarray(1)) {
    value = (value << 7) | (byte & 0x7f);
    if (!(byte & 0x80)) { arcs.push(value); value = 0; }
  }
  return arcs.join('.');
}

export function pemDer(pem) {
  return Buffer.from(String(pem).replace(/-----[^-]+-----/g, '').replace(/\s/g, ''), 'base64');
}

function issuerAndSerial(certDer) {
  const tbs = kids(certDer, parseTlv(certDer))[0];
  const parts = kids(certDer, tbs);
  const skip = parts[0].tag === 0xa0 ? 1 : 0;
  return { serial: certDer.subarray(parts[skip].start, parts[skip].end), issuer: certDer.subarray(parts[skip + 2].start, parts[skip + 2].end) };
}

const HASH = { '1.3.14.3.2.26': 'sha1', '2.16.840.1.101.3.4.2.1': 'sha256', '2.16.840.1.101.3.4.2.2': 'sha384', '2.16.840.1.101.3.4.2.3': 'sha512' };

/** The signed attributes of a CMS, by OID: { node, value } with `value` the first value's TLV. */
export function signedAttributes(cms) {
  const top = kids(cms, parseTlv(cms));
  const signedData = kids(cms, top[1])[0];
  const parts = kids(cms, signedData);
  const info = kids(cms, kids(cms, parts.filter(part => part.tag === 0x31).pop())[0]);
  const attrs = info.find(part => part.tag === 0xa0);
  const out = {};
  for (const attr of kids(cms, attrs)) {
    const [id, set] = kids(cms, attr);
    out[oidOf(cms, id)] = { node: attr, value: kids(cms, set)[0] };
  }
  return { info, attrs, map: out };
}

/**
 * node:crypto's view of a detached CMS: the message digest, the signature over the signed attributes (as a
 * SET), the signer's CN, and with `caPem` whether the leaf's certificate signature verifies under that CA.
 */
export function verifyDetached(cms, content, caPem) {
  const out = { digestMatches: false, signatureValid: false, signerCn: '' };
  if (caPem) out.chainToCa = false;
  try {
    const top = kids(cms, parseTlv(cms));
    if (oidOf(cms, top[0]) !== '1.2.840.113549.1.7.2') return out;
    const parts = kids(cms, kids(cms, top[1])[0]);
    const certSet = parts.find(part => part.tag === 0xa0);
    const signerSet = parts.filter(part => part.tag === 0x31).pop();
    const certs = kids(cms, certSet).map(node => cms.subarray(node.start, node.end));
    const info = kids(cms, kids(cms, signerSet)[0]);
    const sid = kids(cms, info[1]);
    const leaf = certs.find(der => {
      const ids = issuerAndSerial(der);
      return ids.issuer.equals(cms.subarray(sid[0].start, sid[0].end)) && ids.serial.equals(cms.subarray(sid[1].start, sid[1].end));
    }) ?? certs[0];
    const x509 = new X509Certificate(leaf);
    out.signerCn = /(?:^|\n)CN=([^\n]*)/.exec(x509.subject)?.[1] ?? '';
    const hash = HASH[oidOf(cms, kids(cms, info[2])[0])];
    const signedAttrs = info.find(part => part.tag === 0xa0);
    const signature = info.filter(part => part.tag === 0x04).pop();
    for (const attr of kids(cms, signedAttrs)) {
      const [id, set] = kids(cms, attr);
      if (oidOf(cms, id) !== '1.2.840.113549.1.9.4') continue;
      const digest = kids(cms, set)[0];
      out.digestMatches = cms.subarray(digest.valueStart, digest.valueEnd).equals(createHash(hash).update(content).digest());
    }
    const setTagged = Buffer.from(cms.subarray(signedAttrs.start, signedAttrs.end));
    setTagged[0] = 0x31;
    out.signatureValid = createVerify(hash).update(setTagged).verify(x509.publicKey, cms.subarray(signature.valueStart, signature.valueEnd));
    if (caPem) {
      const [tbs, , bit] = kids(leaf, parseTlv(leaf));
      out.chainToCa = verify('sha256', leaf.subarray(tbs.start, tbs.end), new X509Certificate(caPem).publicKey, leaf.subarray(bit.valueStart + 1, bit.valueEnd));
    }
  } catch {
    return out;
  }
  return out;
}
