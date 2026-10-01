/**
 * Fetches a generation-time font into fixtures/.fonts/ (gitignored), pinned by
 * sha256. Only the fixture generators touch the network; the tests read the
 * committed PDFs and never download anything.
 */
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const DIR = path.join(import.meta.dirname, '.fonts');

export async function fontBytes({ name, url, sha256 }) {
  const file = path.join(DIR, name);
  const verify = (bytes) => {
    const got = createHash('sha256').update(bytes).digest('hex');
    if (got !== sha256) throw new Error(`${name}: sha256 ${got} != pinned ${sha256}`);
    return bytes;
  };

  if (existsSync(file)) return verify(readFileSync(file));

  mkdirSync(DIR, { recursive: true });
  const response = await fetch(url);
  if (!response.ok) throw new Error(`GET ${url} -> ${response.status} ${response.statusText}`);
  const bytes = verify(Buffer.from(await response.arrayBuffer()));
  writeFileSync(file, bytes);
  console.log(`fetched ${name} (${bytes.length} bytes, sha256 verified)`);
  return bytes;
}
