import { packDers, readerOf, signModule } from './native.js';

/** Signature verification, in the tandem-sign module (crates/tandem-sign/src/verify.rs). */

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
  /** The verdict in plain English, for people ("Signed by … on …. The document has not changed …"). */
  plain?: string;
};

/** A trusted certificate: DER bytes, or anything with them as `raw` (a trust-store entry, node's X509Certificate). */
export type TrustAnchor = Uint8Array | { raw: Uint8Array };

const MAX_CONTENTS = 64 * 1024;
const MAX_STRING = 1000;

/** Lone UTF-16 surrogates become U+FFFD: JSON.stringify escapes them, and the module's JSON reader refuses them. */
const LONE_SURROGATE = /[\ud800-\udbff](?![\udc00-\udfff])|(?<![\ud800-\udbff])[\udc00-\udfff]/g;

/** A string for the module: well-formed, and one cut just past the limit (still over it, so still malformed). */
function wireString(value: string): string {
  const cut = value.length > MAX_STRING ? value.slice(0, MAX_STRING + 2) : value;
  return cut.replace(LONE_SURROGATE, '\ufffd');
}

/** The entry as JSON for the module: `contents` as hex, every string well-formed and bounded. */
function wire(entry: unknown): unknown {
  if (!entry || typeof entry !== 'object' || Array.isArray(entry)) return entry ?? null;
  const out: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(entry)) {
    if (key === 'contents') {
      out[key] = value instanceof Uint8Array && value.byteLength <= MAX_CONTENTS
        ? Buffer.from(value.buffer, value.byteOffset, value.byteLength).toString('hex')
        : (value === undefined ? undefined : false);
    } else {
      out[key] = typeof value === 'string' ? wireString(value) : value;
    }
  }
  return out;
}

function shell(entry: unknown): Verdict {
  const rec = (entry && typeof entry === 'object' ? entry : {}) as Record<string, unknown>;
  return {
    field: typeof rec.field === 'string' ? rec.field.slice(0, MAX_STRING) : '',
    page: Number.isInteger(rec.page) ? rec.page as number : 0,
    status: 'unsupported', reason: 'malformed', trust: 'none', coverage: 'none',
    plain: 'This signature could not be read, so it was not checked.',
  };
}

/**
 * Pure. No I/O. Every entry gets a verdict: each is verified on its own, so an entry the module cannot take
 * is unsupported/malformed and never costs the others theirs.
 */
export function verifySignatures(src: Buffer, entries: unknown[], trusted: TrustAnchor[] = [], now = Date.now()): Verdict[] {
  if (!Buffer.isBuffer(src)) return entries.map(shell);
  const ders = packDers(trusted.map(anchor => anchor instanceof Uint8Array ? anchor : anchor.raw));
  const reader = readerOf(src);
  return entries.map(entry => {
    try {
      const json = JSON.stringify([wire(entry)], (_key, value) => typeof value === 'bigint' ? String(value) : value);
      return (JSON.parse(signModule().verifySignatures(reader, src.byteLength, json, ders, now)) as Verdict[])[0] ?? shell(entry);
    } catch {
      return shell(entry);
    }
  });
}

/** One entry; see `verifySignatures`. */
export function verifySignature(src: Buffer, entry: SignatureEntry, trusted: TrustAnchor[] = []): Verdict {
  return verifySignatures(src, [entry], trusted)[0] ?? shell(entry);
}
