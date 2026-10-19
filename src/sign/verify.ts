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

/** The entry as JSON for the module: `contents` as hex, over-long strings cut just past the limit. */
function wire(entry: unknown): unknown {
  if (!entry || typeof entry !== 'object' || Array.isArray(entry)) return entry ?? null;
  const out: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(entry)) {
    if (key === 'contents') {
      out[key] = value instanceof Uint8Array && value.byteLength <= MAX_CONTENTS
        ? Buffer.from(value.buffer, value.byteOffset, value.byteLength).toString('hex')
        : (value === undefined ? undefined : false);
    } else {
      out[key] = typeof value === 'string' && value.length > MAX_STRING ? value.slice(0, MAX_STRING + 1) : value;
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
  };
}

/** Pure. No I/O. Every entry gets a verdict; a bad entry is unsupported/malformed, never a throw. */
export function verifySignatures(src: Buffer, entries: unknown[], trusted: TrustAnchor[] = [], now = Date.now()): Verdict[] {
  if (!Buffer.isBuffer(src)) return entries.map(shell);
  const ders = trusted.map(anchor => anchor instanceof Uint8Array ? anchor : anchor.raw);
  const json = JSON.stringify(entries.map(wire), (_key, value) => typeof value === 'bigint' ? String(value) : value);
  return JSON.parse(signModule().verifySignatures(readerOf(src), src.byteLength, json, packDers(ders), now)) as Verdict[];
}

/** One entry; see `verifySignatures`. */
export function verifySignature(src: Buffer, entry: SignatureEntry, trusted: TrustAnchor[] = []): Verdict {
  try {
    return verifySignatures(src, [entry], trusted)[0] ?? shell(entry);
  } catch {
    return shell(entry);
  }
}
