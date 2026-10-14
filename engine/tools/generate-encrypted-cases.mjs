#!/usr/bin/env node
// Writes MuPDF-encrypted variants of fixtures/hello.pdf to engine/tests/data/
// for cipher families the application fixtures lack. User "u", owner "o".
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const mupdf = await import(path.join(root, "node_modules/mupdf/dist/mupdf.js"));
const source = fs.readFileSync(path.join(root, "fixtures/hello.pdf"));
for (const [name, options] of [
  ["encrypted-rc4-40.pdf", "encrypt=rc4-40"],
  ["encrypted-aes128.pdf", "encrypt=aes-128"],
  ["encrypted-aes128-objstm.pdf", "encrypt=aes-128,compress,objstms"],
  ["encrypted-aes256-objstm.pdf", "encrypt=aes-256,compress,objstms"],
]) {
  const doc = mupdf.Document.openDocument(source, "application/pdf");
  const out = doc.saveToBuffer(`${options},user-password=u,owner-password=o`).asUint8Array();
  fs.writeFileSync(path.join(root, "engine/tests/data", name), out);
}
