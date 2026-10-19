import { mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';
import { signModule } from './native.js';

/** A certificate in the user's trusted list; `raw` is its DER. */
export type TrustedCert = { raw: Buffer; cn: string; fingerprint256: string; notAfter: string };

const BLOCK = /-----BEGIN CERTIFICATE-----([\s\S]*?)-----END CERTIFICATE-----/g;

/** Certificates from PEM text; blocks the module cannot read as a certificate are skipped. */
function parseBlocks(text: string): TrustedCert[] {
  const out: TrustedCert[] = [];
  for (const block of text.matchAll(BLOCK)) {
    const raw = Buffer.from(block[1].replace(/\s/g, ''), 'base64');
    const info = raw.length ? signModule().certInfo(raw) : undefined;
    if (!info) continue;
    const { cn, fingerprint256, notAfter } = JSON.parse(info) as TrustedCert;
    out.push({ raw, cn, fingerprint256, notAfter });
  }
  return out;
}

function pem(cert: TrustedCert): string {
  const lines = cert.raw.toString('base64').match(/.{1,64}/g) ?? [];
  return ['-----BEGIN CERTIFICATE-----', ...lines, '-----END CERTIFICATE-----'].join('\n');
}

/** path === null keeps the store in memory (smoke/drive). File writes are temp + rename. */
export function createTrustStore(path: string | null): {
  list: () => { cn: string; fingerprint256: string; notAfter: string }[];
  add: (pem: Buffer) => { added: number; total: number };
  clear: () => void;
  certs: () => TrustedCert[];
} {
  let memory: TrustedCert[] = [];
  const read = (): TrustedCert[] => {
    if (path === null) return memory.slice();
    try { return parseBlocks(readFileSync(path, 'utf8')); } catch { return []; }
  };
  const write = (certs: TrustedCert[]) => {
    if (path === null) { memory = certs; return; }
    const body = certs.map(pem).join('\n') + (certs.length ? '\n' : '');
    const tmp = `${path}.${process.pid}.tmp`;
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(tmp, body);
    renameSync(tmp, path);
  };
  return {
    list: () => read().map(({ cn, fingerprint256, notAfter }) => ({ cn, fingerprint256, notAfter })),
    add(text: Buffer) {
      const cur = read();
      const seen = new Set(cur.map(cert => cert.fingerprint256));
      let added = 0;
      for (const cert of parseBlocks(Buffer.isBuffer(text) ? text.toString('utf8') : String(text))) {
        if (seen.has(cert.fingerprint256)) continue;
        seen.add(cert.fingerprint256);
        cur.push(cert);
        added++;
      }
      write(cur);
      return { added, total: cur.length };
    },
    clear() { write([]); },
    certs: () => read(),
  };
}
