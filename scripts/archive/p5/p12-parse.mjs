#!/usr/bin/env node
/**
 * PKCS#12 → key + certs, node:crypto only. PBES2/AES-256 and legacy
 * pbeWithSHAAnd3-KeyTripleDES-CBC / pbeWithSHAAnd40BitRC2-CBC.
 * QA only. Usage: node scripts/p5/p12-parse.mjs <file.p12> [password]
 */
import { createDecipheriv, createHash, createHmac, pbkdf2Sync } from 'node:crypto';
import { readFileSync } from 'node:fs';

const OID = {
  data: '1.2.840.113549.1.7.1',
  encryptedData: '1.2.840.113549.1.7.6',
  pbes2: '1.2.840.113549.1.5.13',
  pbkdf2: '1.2.840.113549.1.5.12',
  hmacSha1: '1.2.840.113549.2.7',
  hmacSha256: '1.2.840.113549.2.9',
  sha1: '1.3.14.3.2.26',
  sha256: '2.16.840.1.101.3.4.2.1',
  aes256: '2.16.840.1.101.3.4.1.42',
  shrouded: '1.2.840.113549.1.12.10.1.2',
  certBag: '1.2.840.113549.1.12.10.1.3',
  x509: '1.2.840.113549.1.9.22.1',
  pbe3des: '1.2.840.113549.1.12.1.3',
  pbeRc2: '1.2.840.113549.1.12.1.6',
};

function parseTlv(buf, offset = 0) {
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

function kids(buf, node) {
  const out = [];
  for (let cursor = node.valueStart; cursor < node.valueEnd;) {
    const child = parseTlv(buf, cursor);
    out.push(child);
    cursor = child.end;
  }
  return out;
}

function oidOf(buf, node) {
  const body = buf.subarray(node.valueStart, node.valueEnd);
  const arcs = [Math.floor(body[0] / 40), body[0] % 40];
  let value = 0;
  for (const byte of body.subarray(1)) {
    value = (value << 7) | (byte & 0x7f);
    if (!(byte & 0x80)) { arcs.push(value); value = 0; }
  }
  return arcs.join('.');
}

function integer(buf, node) {
  let value = 0;
  for (const byte of buf.subarray(node.valueStart, node.valueEnd)) value = (value * 256) + byte;
  return value;
}

function bmp(password) {
  const out = Buffer.alloc((password.length + 1) * 2);
  for (let i = 0; i < password.length; i++) {
    const code = password.charCodeAt(i);
    out[i * 2] = code >> 8;
    out[i * 2 + 1] = code & 0xff;
  }
  return out;
}

/** RFC 7292 Appendix B. `hash` is sha1 or sha256; v stays 64. */
export function pkcs12Kdf(password, salt, iterations, id, length, hash) {
  const u = createHash(hash).digest().length;
  const v = 64;
  const D = Buffer.alloc(v, id);
  const fill = (src) => {
    if (!src.length) return Buffer.alloc(0);
    const out = Buffer.alloc(Math.ceil(src.length / v) * v);
    for (let i = 0; i < out.length; i++) out[i] = src[i % src.length];
    return out;
  };
  let I = Buffer.concat([fill(salt), fill(bmp(password))]);
  const chunks = [];
  for (let i = 0; i < Math.ceil(length / u); i++) {
    let A = createHash(hash).update(Buffer.concat([D, I])).digest();
    for (let j = 1; j < iterations; j++) A = createHash(hash).update(A).digest();
    chunks.push(A);
    const B = Buffer.alloc(v);
    for (let k = 0; k < v; k++) B[k] = A[k % u];
    const next = Buffer.alloc(I.length);
    for (let block = 0; block < I.length; block += v) {
      let carry = 1;
      for (let k = v - 1; k >= 0; k--) {
        const sum = I[block + k] + B[k] + carry;
        next[block + k] = sum & 0xff;
        carry = sum >> 8;
      }
    }
    I = next;
  }
  return Buffer.concat(chunks).subarray(0, length);
}

function decipher(name, key, iv, data) {
  const crypto = createDecipheriv(name, key, iv);
  return Buffer.concat([crypto.update(data), crypto.final()]);
}

function pbes2(buf, alg, ciphertext, password) {
  const params = kids(buf, alg)[1];
  const [kdf, scheme] = kids(buf, params).map((node) => kids(buf, node));
  if (oidOf(buf, kdf[0]) !== OID.pbkdf2) throw new Error(`kdf ${oidOf(buf, kdf[0])}`);
  const kdfParams = kids(buf, kdf[1]);
  const salt = buf.subarray(kdfParams[0].valueStart, kdfParams[0].valueEnd);
  const rounds = integer(buf, kdfParams[1]);
  let keyLength = 32;
  let prf = 'sha1';
  for (const param of kdfParams.slice(2)) {
    if (param.tag === 0x02) keyLength = integer(buf, param);
    else prf = oidOf(buf, kids(buf, param)[0]) === OID.hmacSha256 ? 'sha256' : 'sha1';
  }
  const schemeOid = oidOf(buf, scheme[0]);
  if (schemeOid !== OID.aes256) throw new Error(`scheme ${schemeOid}`);
  const iv = buf.subarray(scheme[1].valueStart, scheme[1].valueEnd);
  return decipher('aes-256-cbc', pbkdf2Sync(password, salt, rounds, keyLength, prf), iv, ciphertext);
}

function pbeLegacy(buf, alg, ciphertext, password) {
  const id = oidOf(buf, kids(buf, alg)[0]);
  const params = kids(buf, kids(buf, alg)[1]);
  const salt = buf.subarray(params[0].valueStart, params[0].valueEnd);
  const rounds = integer(buf, params[1]);
  if (id === OID.pbe3des) {
    return decipher('des-ede3-cbc',
      pkcs12Kdf(password, salt, rounds, 1, 24, 'sha1'),
      pkcs12Kdf(password, salt, rounds, 2, 8, 'sha1'),
      ciphertext);
  }
  if (id === OID.pbeRc2) {
    // Electron's BoringSSL accepts rc2-40-cbc; stock Node 22 has no RC2.
    return decipher('rc2-40-cbc',
      pkcs12Kdf(password, salt, rounds, 1, 5, 'sha1'),
      pkcs12Kdf(password, salt, rounds, 2, 8, 'sha1'),
      ciphertext);
  }
  throw new Error(`pbe ${id}`);
}

function decryptAlg(buf, alg, ciphertext, password) {
  const id = oidOf(buf, kids(buf, alg)[0]);
  if (id === OID.pbes2) return pbes2(buf, alg, ciphertext, password);
  return pbeLegacy(buf, alg, ciphertext, password);
}

function bags(buf, password, found) {
  const top = parseTlv(buf);
  if (top.tag !== 0x30) return;
  const parts = kids(buf, top);
  if (parts[0]?.tag === 0x06) {
    const id = oidOf(buf, parts[0]);
    const explicit = parts.find((node) => node.tag === 0xa0);
    if (id === OID.data && explicit) {
      const oct = kids(buf, explicit)[0];
      bags(buf.subarray(oct.valueStart, oct.valueEnd), password, found);
      return;
    }
    if (id === OID.encryptedData && explicit) {
      const info = kids(buf, kids(buf, explicit)[0])[1];
      const infoParts = kids(buf, info);
      const alg = infoParts[1];
      const enc = infoParts[2];
      const cipher = buf.subarray(enc.valueStart, enc.valueEnd);
      bags(decryptAlg(buf, alg, cipher, password), password, found);
      return;
    }
    if (id === OID.shrouded && explicit) {
      const enc = kids(buf, kids(buf, explicit)[0]);
      const cipher = buf.subarray(enc[1].valueStart, enc[1].valueEnd);
      found.key = decryptAlg(buf, enc[0], cipher, password);
      return;
    }
    if (id === OID.certBag && explicit) {
      const bag = kids(buf, kids(buf, explicit)[0]);
      if (oidOf(buf, bag[0]) === OID.x509) {
        const oct = kids(buf, bag[1])[0];
        found.certs.push(buf.subarray(oct.valueStart, oct.valueEnd));
      }
      return;
    }
  }
  for (const part of parts) {
    if (part.tag === 0x30) bags(buf.subarray(part.start, part.end), password, found);
  }
}

export function parseP12(file, password) {
  const buf = Buffer.isBuffer(file) ? file : readFileSync(file);
  const found = { key: null, certs: [], mac: null };
  const pfx = kids(buf, parseTlv(buf));
  const auth = kids(buf, pfx[1]);
  const oct = kids(buf, auth[1])[0];
  const safe = buf.subarray(oct.valueStart, oct.valueEnd);
  if (pfx[2]) {
    const mac = kids(buf, pfx[2]);
    const digestInfo = kids(buf, mac[0]);
    const macAlg = oidOf(buf, kids(buf, digestInfo[0])[0]);
    const expected = buf.subarray(digestInfo[1].valueStart, digestInfo[1].valueEnd);
    const salt = buf.subarray(mac[1].valueStart, mac[1].valueEnd);
    const rounds = integer(buf, mac[2]);
    const hash = macAlg === OID.sha256 ? 'sha256' : 'sha1';
    const key = pkcs12Kdf(password, salt, rounds, 3, hash === 'sha256' ? 32 : 20, hash);
    const got = createHmac(hash, key).update(safe).digest();
    found.mac = got.equals(expected) ? 'ok' : 'bad';
  }
  bags(safe, password, found);
  return found;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const found = parseP12(process.argv[2], process.argv[3] ?? 'pdf4all');
  console.log(JSON.stringify({
    node: process.version,
    openssl: process.versions.openssl,
    mac: found.mac,
    key: found.key?.length ?? 0,
    certs: found.certs.map((cert) => cert.length),
  }));
}
