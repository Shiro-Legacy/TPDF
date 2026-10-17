#!/usr/bin/env node
// Writes MuPDF-encrypted variants of fixtures/hello.pdf to engine/tests/data/
// for cipher families the application fixtures lack. User "u", owner "o",
// except the -pdfdoc files: user "pässwort€中", owner "öwner•", which MuPDF
// hashes in PDFDocEncoding for R2-R4 (dropping 中) and as UTF-8 for R6.
// File names given as arguments limit which files are written.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const mupdf = await import(path.join(root, "node_modules/mupdf/dist/mupdf.js"));
const source = fs.readFileSync(path.join(root, "fixtures/hello.pdf"));
const only = process.argv.slice(2);
const ascii = "user-password=u,owner-password=o";
const pdfdoc = "user-password=pässwort€中,owner-password=öwner•";
for (const [name, options] of [
  ["encrypted-rc4-40.pdf", `encrypt=rc4-40,${ascii}`],
  ["encrypted-aes128.pdf", `encrypt=aes-128,${ascii}`],
  ["encrypted-aes128-objstm.pdf", `encrypt=aes-128,compress,objstms,${ascii}`],
  ["encrypted-aes256-objstm.pdf", `encrypt=aes-256,compress,objstms,${ascii}`],
  ["encrypted-rc4-40-pdfdoc.pdf", `encrypt=rc4-40,${pdfdoc}`],
  ["encrypted-rc4-128-pdfdoc.pdf", `encrypt=rc4-128,${pdfdoc}`],
  ["encrypted-aes128-pdfdoc.pdf", `encrypt=aes-128,${pdfdoc}`],
  ["encrypted-aes256-pdfdoc.pdf", `encrypt=aes-256,${pdfdoc}`],
]) {
  if (only.length > 0 && !only.includes(name)) continue;
  const doc = mupdf.Document.openDocument(source, "application/pdf");
  const out = doc.saveToBuffer(options).asUint8Array();
  fs.writeFileSync(path.join(root, "engine/tests/data", name), out);
}
