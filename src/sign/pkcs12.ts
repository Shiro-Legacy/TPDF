import {
  createDecipheriv, createHash, createHmac, createPrivateKey, createPublicKey, pbkdf2Sync,
  X509Certificate, type KeyObject,
} from 'node:crypto';
import { kids, oidOf, parseTlv, pemDer, type Tlv } from './der.js';

const OID = {
  data: '1.2.840.113549.1.7.1',
  encryptedData: '1.2.840.113549.1.7.6',
  pbes2: '1.2.840.113549.1.5.13',
  pbkdf2: '1.2.840.113549.1.5.12',
  hmacSha256: '1.2.840.113549.2.9',
  sha256: '2.16.840.1.101.3.4.2.1',
  aes256: '2.16.840.1.101.3.4.1.42',
  shrouded: '1.2.840.113549.1.12.10.1.2',
  certBag: '1.2.840.113549.1.12.10.1.3',
  x509: '1.2.840.113549.1.9.22.1',
  pbe3des: '1.2.840.113549.1.12.1.3',
  pbeRc2: '1.2.840.113549.1.12.1.6',
};

export type SignReason = 'bad-password' | 'unsupported-key' | 'unreadable' | 'certificate-expired';

export class SignError extends Error {
  readonly reason: SignReason;
  readonly date?: string;
  readonly status?: string;
  constructor(reason: SignReason, extra?: { date?: string; status?: string }) {
    super(extra?.status ?? reason);
    this.reason = reason;
    this.date = extra?.date;
    this.status = extra?.status;
  }
}

type Found = { key: KeyObject | null; certs: Buffer[] };

function integer(buf: Buffer, node: Tlv): number {
  let value = 0;
  for (const byte of buf.subarray(node.valueStart, node.valueEnd)) value = value * 256 + byte;
  return value;
}

function bmp(password: string): Buffer {
  const out = Buffer.alloc((password.length + 1) * 2);
  for (let i = 0; i < password.length; i++) {
    const code = password.charCodeAt(i);
    out[i * 2] = code >> 8;
    out[i * 2 + 1] = code & 0xff;
  }
  return out;
}

function pkcs12Kdf(password: string, salt: Buffer, iterations: number, id: number, length: number, hash: 'sha1' | 'sha256'): Buffer {
  const u = createHash(hash).digest().length;
  const v = 64;
  const D = Buffer.alloc(v, id);
  const fill = (src: Buffer): Buffer => {
    if (!src.length) return Buffer.alloc(0);
    const out = Buffer.alloc(Math.ceil(src.length / v) * v);
    for (let i = 0; i < out.length; i++) out[i] = src[i % src.length];
    return out;
  };
  let I = Buffer.concat([fill(salt), fill(bmp(password))]);
  const chunks: Buffer[] = [];
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

function decipher(name: string, key: Buffer, iv: Buffer, data: Buffer): Buffer {
  try {
    const crypto = createDecipheriv(name, key, iv);
    return Buffer.concat([crypto.update(data), crypto.final()]);
  } catch {
    throw new SignError('bad-password');
  }
}

function pbes2(buf: Buffer, alg: Tlv, ciphertext: Buffer, password: string): Buffer {
  const params = kids(buf, alg)[1];
  const [kdf, scheme] = kids(buf, params).map(node => kids(buf, node));
  if (oidOf(buf, kdf[0]) !== OID.pbkdf2) throw new SignError('unreadable');
  const kdfParams = kids(buf, kdf[1]);
  const salt = buf.subarray(kdfParams[0].valueStart, kdfParams[0].valueEnd);
  const rounds = integer(buf, kdfParams[1]);
  let keyLength = 32;
  let prf: 'sha1' | 'sha256' = 'sha1';
  for (const param of kdfParams.slice(2)) {
    if (param.tag === 0x02) keyLength = integer(buf, param);
    else prf = oidOf(buf, kids(buf, param)[0]) === OID.hmacSha256 ? 'sha256' : 'sha1';
  }
  if (oidOf(buf, scheme[0]) !== OID.aes256) throw new SignError('unreadable');
  const iv = buf.subarray(scheme[1].valueStart, scheme[1].valueEnd);
  return decipher('aes-256-cbc', pbkdf2Sync(password, salt, rounds, keyLength, prf), iv, ciphertext);
}

function pbeLegacy(buf: Buffer, alg: Tlv, ciphertext: Buffer, password: string): Buffer {
  const id = oidOf(buf, kids(buf, alg)[0]);
  const params = kids(buf, kids(buf, alg)[1]);
  const salt = buf.subarray(params[0].valueStart, params[0].valueEnd);
  const rounds = integer(buf, params[1]);
  if (id === OID.pbe3des) {
    return decipher('des-ede3-cbc', pkcs12Kdf(password, salt, rounds, 1, 24, 'sha1'), pkcs12Kdf(password, salt, rounds, 2, 8, 'sha1'), ciphertext);
  }
  if (id === OID.pbeRc2) {
    // Electron's Node has rc2-40-cbc. Repo Node 22 does not; that case is skipped in tests.
    return decipher('rc2-40-cbc', pkcs12Kdf(password, salt, rounds, 1, 5, 'sha1'), pkcs12Kdf(password, salt, rounds, 2, 8, 'sha1'), ciphertext);
  }
  throw new SignError('unreadable');
}

function decryptAlg(buf: Buffer, alg: Tlv, ciphertext: Buffer, password: string): Buffer {
  return oidOf(buf, kids(buf, alg)[0]) === OID.pbes2
    ? pbes2(buf, alg, ciphertext, password)
    : pbeLegacy(buf, alg, ciphertext, password);
}

function importKey(der: Buffer): KeyObject {
  try {
    return createPrivateKey({ key: der, format: 'der', type: 'pkcs8' });
  } finally {
    der.fill(0);
  }
}

function bags(buf: Buffer, password: string, found: Found): void {
  const top = parseTlv(buf);
  if (top.tag !== 0x30) return;
  const parts = kids(buf, top);
  if (parts[0]?.tag === 0x06) {
    const id = oidOf(buf, parts[0]);
    const explicit = parts.find(node => node.tag === 0xa0);
    if (id === OID.data && explicit) {
      const oct = kids(buf, explicit)[0];
      bags(buf.subarray(oct.valueStart, oct.valueEnd), password, found);
      return;
    }
    if (id === OID.encryptedData && explicit) {
      const info = kids(buf, kids(buf, explicit)[0])[1];
      const infoParts = kids(buf, info);
      const plain = decryptAlg(buf, infoParts[1], buf.subarray(infoParts[2].valueStart, infoParts[2].valueEnd), password);
      try {
        bags(plain, password, found);
      } finally {
        plain.fill(0);
      }
      return;
    }
    if (id === OID.shrouded && explicit) {
      const enc = kids(buf, kids(buf, explicit)[0]);
      const plain = decryptAlg(buf, enc[0], buf.subarray(enc[1].valueStart, enc[1].valueEnd), password);
      found.key = importKey(plain);
      return;
    }
    if (id === OID.certBag && explicit) {
      const bag = kids(buf, kids(buf, explicit)[0]);
      if (oidOf(buf, bag[0]) === OID.x509) {
        const oct = kids(buf, bag[1])[0];
        found.certs.push(Buffer.from(buf.subarray(oct.valueStart, oct.valueEnd)));
      }
      return;
    }
  }
  for (const part of parts) {
    if (part.tag === 0x30) bags(buf.subarray(part.start, part.end), password, found);
  }
}

export function assertRsa(key: KeyObject): void {
  const bits = key.asymmetricKeyDetails?.modulusLength ?? 0;
  if (key.asymmetricKeyType !== 'rsa' || bits < 2048) throw new SignError('unsupported-key');
}

function spki(der: Buffer): Buffer {
  return Buffer.from(new X509Certificate(der).publicKey.export({ type: 'spki', format: 'der' }));
}

function orderLeaf(certs: Buffer[], key: KeyObject): Buffer[] {
  const pub = Buffer.from(createPublicKey(key).export({ type: 'spki', format: 'der' }));
  const leaf = certs.findIndex(cert => spki(cert).equals(pub));
  if (leaf < 0) throw new SignError('unreadable');
  return [certs[leaf], ...certs.filter((_, index) => index !== leaf)];
}

function finish(found: Found): { key: KeyObject; certs: Buffer[] } {
  if (!found.key) throw new SignError('unreadable');
  assertRsa(found.key);
  return { key: found.key, certs: orderLeaf(found.certs, found.key) };
}

export function parseP12(file: Buffer, password: string): { key: KeyObject; certs: Buffer[] } {
  try {
    const buf = file;
    const found: Found = { key: null, certs: [] };
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
      const macKey = pkcs12Kdf(password, salt, rounds, 3, hash === 'sha256' ? 32 : 20, hash);
      const got = createHmac(hash, macKey).update(safe).digest();
      macKey.fill(0);
      if (!got.equals(expected)) throw new SignError('bad-password');
    }
    bags(Buffer.from(safe), password, found);
    return finish(found);
  } catch (err) {
    if (err instanceof SignError) throw err;
    throw new SignError('unreadable');
  }
}

export function parsePem(file: Buffer, password: string): { key: KeyObject; certs: Buffer[] } {
  const text = file.toString('utf8');
  const blocks = [...text.matchAll(/-----BEGIN ([A-Z0-9 ]+)-----[\s\S]*?-----END \1-----/g)];
  if (!blocks.length) throw new SignError('unreadable');
  let key: KeyObject | undefined;
  const certs: Buffer[] = [];
  for (const block of blocks) {
    const pem = block[0];
    if (block[1].includes('PRIVATE KEY')) {
      const encrypted = /ENCRYPTED/.test(block[1]) || /Proc-Type: 4,ENCRYPTED/.test(pem);
      try {
        key = encrypted
          ? createPrivateKey({ key: pem, passphrase: password })
          : createPrivateKey(pem);
      } catch {
        throw new SignError('bad-password');
      }
    } else if (block[1] === 'CERTIFICATE') {
      certs.push(pemDer(pem));
    }
  }
  if (!key) throw new SignError('unreadable');
  return finish({ key, certs });
}

function ymd(ms: number): string {
  const date = new Date(ms);
  const p = (n: number) => String(n).padStart(2, '0');
  return `${date.getUTCFullYear()}-${p(date.getUTCMonth() + 1)}-${p(date.getUTCDate())}`;
}

export function certUsable(certs: Buffer[], key: KeyObject, when: number):
  { ok: true; cn: string; notAfter: string } | { ok: false; reason: 'certificate-expired' | 'unsupported-key'; date?: string; status?: string } {
  try {
    assertRsa(key);
  } catch (err) {
    if (err instanceof SignError) return { ok: false, reason: 'unsupported-key' };
    throw err;
  }
  const cert = new X509Certificate(certs[0]);
  const notBefore = Date.parse(cert.validFrom);
  const notAfter = Date.parse(cert.validTo);
  if (when < notBefore || when >= notAfter) {
    const expired = when >= notAfter;
    const date = ymd(expired ? notAfter : notBefore);
    return { ok: false, reason: 'certificate-expired', date, status: expired ? `this certificate expired on ${date}` : `this certificate is not valid until ${date}` };
  }
  const cn = /(?:^|\n)CN=([^\n]*)/.exec(cert.subject)?.[1] ?? '';
  return { ok: true, cn, notAfter: new Date(notAfter).toISOString() };
}
