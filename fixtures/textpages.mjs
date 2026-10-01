#!/usr/bin/env node
/**
 * The P2 unit 3 scratch document: three pages whose text is "one", "two",
 * "three". Shared by ds2's engine tests and grok's --edit=pages smoke, and
 * generated on demand (never committed) so it cannot drift from the code.
 *
 * As a module: `makeThreePagePdf(['one','two','three'])` returns bytes.
 * As a script: `node fixtures/textpages.mjs [out.pdf] [word...]` writes the
 * file (default `/tmp/pdf4all-three.pdf`) and prints its path.
 */
import { writeFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { makeTextPages } from './textpdf.mjs';

export function makeThreePagePdf(texts = ['one', 'two', 'three']) {
  return makeTextPages(texts.map(text => ({
    width: 612,
    height: 792,
    runs: [{ font: { name: 'Helvetica', resource: 'F1' }, text, size: 24, x: 72, y: 700 }],
  })));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const words = process.argv.slice(3);
  const out = process.argv[2] || '/tmp/pdf4all-three.pdf';
  writeFileSync(out, makeThreePagePdf(words.length ? words : undefined));
  console.log(out);
}
