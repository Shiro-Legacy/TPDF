#!/usr/bin/env node
/**
 * P0-b probe 3: in-process mupdf wasm (no PKCS#7). QA only.
 *
 * Usage: node scripts/p0b/sign-probe.mjs <pdf>
 * Tools: mupdf 1.28.1 wasm (AGPL-3.0-or-later). No PKCS#7 symbols — cannot
 * verify CMS integrity/trust/revocation in-process.
 */
import { readFile } from 'node:fs/promises';
import mupdf from 'mupdf';

function collectSigs(pdf) {
  const root = pdf.getTrailer().get('Root');
  const acro = root.get('AcroForm');
  const out = [];
  if (acro.isNull()) return out;
  const fields = acro.get('Fields');
  if (fields.isNull() || !fields.isArray()) return out;
  for (let i = 0; i < fields.length; i++) {
    const f = fields.get(i).resolve();
    const ft = f.get('FT');
    if (!ft.isNull() && ft.isName() && ft.asName() === 'Sig') {
      const v = f.get('V');
      out.push({
        name: f.get('T').isString() ? f.get('T').asString() : '',
        hasValue: !v.isNull(),
        filter: (!v.isNull() && v.resolve().get('Filter').isName()) ? v.resolve().get('Filter').asName() : null,
        byteRange: !v.isNull(),
      });
    }
  }
  return out;
}

const pdf = mupdf.Document.openDocument(await readFile(process.argv[2]), 'application/pdf');
const root = pdf.getTrailer().get('Root');
const perms = root.get('Perms');
const report = {
  mupdf: '1.28.1',
  pdf: process.argv[2],
  countPages: pdf.countPages(),
  countVersions: pdf.countVersions(),
  validateChangeHistory: pdf.validateChangeHistory(),
  wasRepaired: pdf.wasRepaired(),
  canBeSavedIncrementally: pdf.canBeSavedIncrementally(),
  certified: !perms.isNull() && !perms.get('DocMDP').isNull(),
  signatures: collectSigs(pdf),
  pkcs7: 'none — mupdf-wasm.d.ts has no PKCS#7 symbols',
};
console.log(JSON.stringify(report, null, 2));
