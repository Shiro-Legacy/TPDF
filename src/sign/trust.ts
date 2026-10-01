import { X509Certificate } from 'node:crypto';
import { mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';

const BLOCK = /-----BEGIN CERTIFICATE-----[\s\S]*?-----END CERTIFICATE-----/g;

function parseBlocks(text: string): X509Certificate[] {
  const out: X509Certificate[] = [];
  for (const block of text.match(BLOCK) ?? []) {
    try { out.push(new X509Certificate(block)); } catch { /* garbage block is skipped */ }
  }
  return out;
}

function cnOf(cert: X509Certificate): string {
  return /(?:^|\n)CN=([^\n]*)/.exec(cert.subject)?.[1] ?? '';
}

function notAfter(cert: X509Certificate): string {
  return (cert.validToDate ?? new Date(cert.validTo)).toISOString();
}

/** path === null keeps the store in memory (smoke/drive). File writes are temp + rename. */
export function createTrustStore(path: string | null): {
  list: () => { cn: string; fingerprint256: string; notAfter: string }[];
  add: (pem: Buffer) => { added: number; total: number };
  clear: () => void;
  certs: () => X509Certificate[];
} {
  let memory: X509Certificate[] = [];
  const read = (): X509Certificate[] => {
    if (path === null) return memory.slice();
    try { return parseBlocks(readFileSync(path, 'utf8')); } catch { return []; }
  };
  const write = (certs: X509Certificate[]) => {
    if (path === null) { memory = certs; return; }
    const body = certs.map(cert => cert.toString().trim()).join('\n') + (certs.length ? '\n' : '');
    const tmp = `${path}.${process.pid}.tmp`;
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(tmp, body);
    renameSync(tmp, path);
  };
  return {
    list: () => read().map(cert => ({ cn: cnOf(cert), fingerprint256: cert.fingerprint256, notAfter: notAfter(cert) })),
    add(pem: Buffer) {
      const cur = read();
      const seen = new Set(cur.map(cert => cert.fingerprint256));
      let added = 0;
      for (const cert of parseBlocks(Buffer.isBuffer(pem) ? pem.toString('utf8') : String(pem))) {
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
