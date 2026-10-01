import { createHash, createVerify, X509Certificate, constants } from 'node:crypto';
import { issuerAndSerial, oidOf, parseTlv, type Tlv } from './der.js';

/** Binding for both seats. Worker may also set status/reason when /V is malformed. */
export type SignatureEntry = {
  field: string;
  page: number;
  rect: number[] | null;
  empty: boolean;
  subFilter?: string;
  filter?: string;
  name?: string;
  reason?: string;
  location?: string;
  contact?: string;
  m?: string;
  byteRange?: number[];
  contents?: Uint8Array;
  docmdp?: number | null;
  status?: string;
};

export type Verdict = {
  field: string;
  page: number;
  status: 'valid' | 'invalid' | 'unsupported' | 'empty';
  reason?: string;
  signerCn?: string;
  issuerCn?: string;
  signingTime?: string;
  digest?: string;
  trust: 'trusted' | 'untrusted' | 'expired' | 'none';
  coverage: 'entire-file' | 'partial' | 'none';
  covered?: number;
  total?: number;
  docmdp?: number | null;
};

const HASH: Record<string, string> = {
  '1.3.14.3.2.26': 'sha1',
  '2.16.840.1.101.3.4.2.1': 'sha256',
  '2.16.840.1.101.3.4.2.2': 'sha384',
  '2.16.840.1.101.3.4.2.3': 'sha512',
};
const MESSAGE_DIGEST = '1.2.840.113549.1.9.4';
const SIGNING_TIME = '1.2.840.113549.1.9.5';
const SIGNED_DATA = '1.2.840.113549.1.7.2';
const PSS = '1.2.840.113549.1.1.10';
const DETACHED = new Set(['adbe.pkcs7.detached', 'ETSI.CAdES.detached']);
const MAX_CONTENTS = 64 * 1024;
const MAX_STRING = 1000;
const STRINGS = ['field', 'subFilter', 'filter', 'name', 'reason', 'location', 'contact', 'm'] as const;

// ponytail: no OCSP/CRL, no TSA, no EKU; chain depth 5. A certvalidator is the upgrade if those checks are required.
const MAX_CHAIN = 5;

function cnOf(cert: X509Certificate): string {
  return /(?:^|\n)CN=([^\n]*)/.exec(cert.subject)?.[1] ?? '';
}

function safeKids(buf: Buffer, node: Tlv): Tlv[] | undefined {
  if (!Number.isFinite(node.valueEnd) || node.valueEnd > buf.length || node.valueStart < 0) return;
  const out: Tlv[] = [];
  for (let cursor = node.valueStart; cursor < node.valueEnd;) {
    if (cursor + 2 > buf.length) return;
    const child = parseTlv(buf, cursor);
    if (!Number.isFinite(child.end) || child.end <= cursor || child.end > node.valueEnd) return;
    out.push(child);
    cursor = child.end;
  }
  return out;
}

function shell(entry: Partial<SignatureEntry> | null | undefined, status: Verdict['status'], reason: string | undefined, extra: Partial<Verdict> = {}): Verdict {
  const rec = entry && typeof entry === 'object' ? entry : {};
  const docmdp = rec.docmdp === null || typeof rec.docmdp === 'number' ? rec.docmdp : undefined;
  return {
    field: typeof rec.field === 'string' ? rec.field.slice(0, MAX_STRING) : '',
    page: Number.isInteger(rec.page) ? rec.page as number : 0,
    status,
    ...(reason ? { reason } : {}),
    trust: 'none',
    coverage: 'none',
    ...(docmdp !== undefined ? { docmdp } : {}),
    ...extra,
  };
}

function stringsOk(entry: SignatureEntry): boolean {
  const rec = entry as unknown as Record<string, unknown>;
  if (typeof rec.field !== 'string') return false;
  for (const key of STRINGS) {
    const value = rec[key];
    if (value === undefined) continue;
    if (typeof value !== 'string' || value.length > MAX_STRING) return false;
  }
  return true;
}

function hexEq(src: Buffer, b: number, c: number, contents: Uint8Array): Buffer | undefined {
  if (src[b] !== 0x3c || src[c - 1] !== 0x3e || c < b + 2) return;
  const hex = src.subarray(b + 1, c - 1);
  if (hex.length % 2 !== 0) return;
  for (const byte of hex) {
    const ok = (byte >= 0x30 && byte <= 0x39) || (byte >= 0x41 && byte <= 0x46) || (byte >= 0x61 && byte <= 0x66);
    if (!ok) return;
  }
  const decoded = Buffer.from(hex.toString('ascii'), 'hex');
  if (decoded.length !== hex.length / 2 || !decoded.equals(Buffer.from(contents))) return;
  return decoded;
}

/** Covered prefix must end at a revision: %%EOF plus at most two CR/LF bytes. */
function atRevision(prefix: Buffer): boolean {
  let end = prefix.length;
  let eol = 0;
  while (eol < 2 && end > 0 && (prefix[end - 1] === 0x0a || prefix[end - 1] === 0x0d)) {
    eol++;
    end--;
  }
  return end >= 5 && prefix.subarray(end - 5, end).toString('ascii') === '%%EOF';
}

function cmsEnd(cms: Buffer): number | undefined {
  if (cms.length < 2) return;
  const top = parseTlv(cms);
  if (top.start !== 0 || !Number.isFinite(top.end) || top.end <= 0 || top.end > cms.length) return;
  for (let i = top.end; i < cms.length; i++) if (cms[i] !== 0) return;
  return top.end;
}

function utcTime(text: string): Date | undefined {
  const m = /^(\d{2})(\d{2})(\d{2})(\d{2})(\d{2})(\d{2})Z$/.exec(text);
  if (!m) return;
  const yy = Number(m[1]);
  return new Date(Date.UTC(yy >= 50 ? 1900 + yy : 2000 + yy, Number(m[2]) - 1, Number(m[3]), Number(m[4]), Number(m[5]), Number(m[6])));
}

function genTime(text: string): Date | undefined {
  const m = /^(\d{4})(\d{2})(\d{2})(\d{2})(\d{2})(\d{2})Z$/.exec(text);
  if (!m) return;
  return new Date(Date.UTC(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6]));
}

function pdfTime(text: string): Date | undefined {
  const m = /^D:(\d{4})(\d{2})(\d{2})(\d{2})(\d{2})(\d{2})(?:Z|([+-])(\d{2})'(\d{2})')?/.exec(text);
  if (!m) return;
  const utc = Date.UTC(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6]);
  if (!m[7]) return new Date(utc);
  const off = (Number(m[8]) * 60 + Number(m[9])) * (m[7] === '-' ? -1 : 1);
  return new Date(utc - off * 60_000);
}

function timeOf(cms: Buffer, node: Tlv): Date | undefined {
  const text = cms.subarray(node.valueStart, node.valueEnd).toString('ascii');
  if (node.tag === 0x17) return utcTime(text);
  if (node.tag === 0x18) return genTime(text);
}

function covers(cert: X509Certificate, when: Date): boolean {
  const from = cert.validFromDate ?? new Date(cert.validFrom);
  const to = cert.validToDate ?? new Date(cert.validTo);
  const t = when.getTime();
  return Number.isFinite(t) && t >= from.getTime() && t <= to.getTime();
}

function issuedBy(cert: X509Certificate, issuer: X509Certificate): boolean {
  try { return issuer.ca === true && cert.checkIssued(issuer) && cert.verify(issuer.publicKey); }
  catch { return false; }
}

/** Leaf fingerprint match trusts any store cert. An issuer match requires ca === true. */
function anchorOf(leaf: X509Certificate, embedded: X509Certificate[], store: X509Certificate[]): { cert: X509Certificate; chain: X509Certificate[] } | undefined {
  const byFp = new Map(store.map(cert => [cert.fingerprint256, cert]));
  const self = byFp.get(leaf.fingerprint256);
  if (self) return { cert: self, chain: [leaf] };
  const seen = new Set<string>([leaf.fingerprint256]);
  let current = leaf;
  const chain = [leaf];
  for (let depth = 0; depth < MAX_CHAIN; depth++) {
    const storeIssuer = store.find(cert => issuedBy(current, cert));
    if (storeIssuer) return { cert: storeIssuer, chain: [...chain, storeIssuer] };
    const next = embedded.find(cert => !seen.has(cert.fingerprint256) && issuedBy(current, cert));
    if (!next) return;
    chain.push(next);
    const stored = byFp.get(next.fingerprint256);
    if (stored) return { cert: stored, chain };
    seen.add(next.fingerprint256);
    current = next;
  }
}

function verifyOne(src: Buffer, entry: SignatureEntry, trusted: X509Certificate[]): Verdict {
  if (!entry || typeof entry !== 'object' || !stringsOk(entry)) return shell(entry, 'unsupported', 'malformed');
  if (entry.empty) return shell(entry, 'empty', undefined);
  const range = entry.byteRange;
  const contents = entry.contents;
  if (!range || range.length !== 4 || range.some(n => !Number.isInteger(n) || n < 0) || !(contents instanceof Uint8Array) || contents.byteLength > MAX_CONTENTS) {
    return shell(entry, 'unsupported', 'malformed');
  }
  const [a, b, c, d] = range;
  if (a !== 0 || b < 1 || c < b || c > src.length || d > src.length - c || c < 1) return shell(entry, 'invalid', 'byte-range');
  const cms = hexEq(src, b, c, contents);
  if (!cms) return shell(entry, 'invalid', 'byte-range');
  const end = cmsEnd(cms);
  if (end === undefined) return shell(entry, 'invalid', 'byte-range');
  const covered = c + d;
  const partial = covered !== src.length;
  if (partial && !atRevision(src.subarray(0, covered))) return shell(entry, 'invalid', 'byte-range');
  const coverage: Verdict['coverage'] = partial ? 'partial' : 'entire-file';
  const span = { coverage, covered, total: src.length, docmdp: entry.docmdp === null || typeof entry.docmdp === 'number' ? entry.docmdp : undefined };

  const sub = entry.subFilter ?? '';
  if (!DETACHED.has(sub)) return shell(entry, 'unsupported', sub || 'missing-subfilter', span);

  const topKids = safeKids(cms, parseTlv(cms));
  if (!topKids || oidOf(cms, topKids[0]) !== SIGNED_DATA) return shell(entry, 'unsupported', 'malformed', span);
  const signed = safeKids(cms, topKids[1])?.[0];
  if (!signed) return shell(entry, 'unsupported', 'malformed', span);
  const parts = safeKids(cms, signed);
  if (!parts) return shell(entry, 'unsupported', 'malformed', span);
  const certSet = parts.find(part => part.tag === 0xa0);
  const signerSet = parts.filter(part => part.tag === 0x31).pop();
  const infos = signerSet ? safeKids(cms, signerSet) : undefined;
  if (!certSet || !infos || infos.length !== 1) return shell(entry, 'unsupported', 'signer-id', span);
  const info = safeKids(cms, infos[0]);
  if (!info || info.length < 5) return shell(entry, 'unsupported', 'malformed', span);
  if (info[1].tag !== 0x30) return shell(entry, 'unsupported', 'signer-id', span);
  const certs = safeKids(cms, certSet)?.map(node => cms.subarray(node.start, node.end));
  if (!certs) return shell(entry, 'unsupported', 'malformed', span);
  const sid = safeKids(cms, info[1]);
  if (!sid || sid.length < 2) return shell(entry, 'unsupported', 'signer-id', span);
  const leafDer = certs.find(der => {
    try {
      const ids = issuerAndSerial(der);
      return cms.subarray(sid[0].start, sid[0].end).equals(ids.issuer) && cms.subarray(sid[1].start, sid[1].end).equals(ids.serial);
    } catch { return false; }
  });
  if (!leafDer) return shell(entry, 'invalid', 'signer-cert', span);
  const leaf = new X509Certificate(leafDer);
  const digestNode = info[2];
  const digestKids = digestNode.tag === 0x30 ? safeKids(cms, digestNode) : undefined;
  const hashName = digestKids ? HASH[oidOf(cms, digestKids[0])] : undefined;
  if (!hashName) return shell(entry, 'unsupported', digestKids ? oidOf(cms, digestKids[0]) : 'unknown-digest', { ...span, signerCn: cnOf(leaf) });
  const signedAttrs = info.find(part => part.tag === 0xa0);
  const signature = info.filter(part => part.tag === 0x04).pop();
  if (!signedAttrs || !signature) return shell(entry, 'unsupported', 'no-signed-attrs', { ...span, signerCn: cnOf(leaf), digest: hashName });
  const attrs = safeKids(cms, signedAttrs);
  if (!attrs) return shell(entry, 'unsupported', 'malformed', span);
  const digests = attrs.filter(attr => {
    const body = safeKids(cms, attr);
    return body && oidOf(cms, body[0]) === MESSAGE_DIGEST;
  });
  if (digests.length !== 1) return shell(entry, 'invalid', 'digest', { ...span, signerCn: cnOf(leaf), digest: hashName });
  const digestValue = safeKids(cms, safeKids(cms, digests[0])![1])?.[0];
  if (!digestValue || digestValue.tag !== 0x04) return shell(entry, 'invalid', 'digest', { ...span, signerCn: cnOf(leaf), digest: hashName });
  const content = Buffer.concat([src.subarray(0, b), src.subarray(c, covered)]);
  const digestOk = cms.subarray(digestValue.valueStart, digestValue.valueEnd).equals(createHash(hashName).update(content).digest());
  if (!digestOk) return shell(entry, 'invalid', 'digest', { ...span, signerCn: cnOf(leaf), digest: hashName });
  const setTagged = Buffer.from(cms.subarray(signedAttrs.start, signedAttrs.end));
  setTagged[0] = 0x31;
  const sigAlg = info.filter(part => part.tag === 0x30).pop();
  const sigOid = sigAlg && safeKids(cms, sigAlg)?.[0] ? oidOf(cms, safeKids(cms, sigAlg)![0]) : '';
  const keyType = leaf.publicKey.asymmetricKeyType;
  const pss = sigOid === PSS;
  if ((keyType !== 'rsa' && keyType !== 'ec') || (pss && keyType !== 'rsa')) {
    return shell(entry, 'unsupported', sigOid || hashName, { ...span, signerCn: cnOf(leaf), digest: hashName });
  }
  let signatureValid = false;
  try {
    const check = createVerify(hashName);
    check.update(setTagged);
    const sig = cms.subarray(signature.valueStart, signature.valueEnd);
    signatureValid = pss
      ? check.verify({ key: leaf.publicKey, padding: constants.RSA_PKCS1_PSS_PADDING, saltLength: constants.RSA_PSS_SALTLEN_AUTO }, sig)
      : check.verify(leaf.publicKey, sig);
  } catch { signatureValid = false; }
  if (!signatureValid) return shell(entry, 'invalid', 'signature', { ...span, signerCn: cnOf(leaf), digest: hashName });

  const timeAttr = attrs.find(attr => {
    const body = safeKids(cms, attr);
    return body && oidOf(cms, body[0]) === SIGNING_TIME;
  });
  const timeNode = timeAttr ? safeKids(cms, safeKids(cms, timeAttr)![1])?.[0] : undefined;
  let when = timeNode ? timeOf(cms, timeNode) : undefined;
  if (!when && entry.m) when = pdfTime(entry.m);
  const noTime = !when;
  if (!when) when = new Date();
  const embedded = certs.map(der => { try { return new X509Certificate(der); } catch { return; } }).filter((cert): cert is X509Certificate => !!cert);
  const anchor = anchorOf(leaf, embedded, trusted);
  const trust: Verdict['trust'] = !anchor ? 'untrusted' : anchor.chain.some(cert => !covers(cert, when)) ? 'expired' : 'trusted';
  const notes = [hashName === 'sha1' ? 'weak-digest' : '', noTime ? 'no-signing-time' : '', entry.docmdp === 1 && partial ? 'changes-not-permitted' : ''].filter(Boolean);
  return shell(entry, 'valid', notes.join(' ') || undefined, {
    ...span,
    signerCn: cnOf(leaf),
    ...(trust === 'trusted' ? { issuerCn: cnOf(anchor!.cert) } : {}),
    signingTime: when.toISOString(),
    digest: hashName,
    trust,
  });
}

/** Pure. No I/O. A bad entry is unsupported/malformed, never a throw. */
export function verifySignature(src: Buffer, entry: SignatureEntry, trusted: X509Certificate[] = []): Verdict {
  try {
    if (!Buffer.isBuffer(src)) return shell(entry, 'unsupported', 'malformed');
    return verifyOne(src, entry, trusted);
  } catch {
    return shell(entry, 'unsupported', 'malformed');
  }
}
