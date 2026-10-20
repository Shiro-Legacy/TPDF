#!/usr/bin/env node
// SPDX-License-Identifier: MIT
// Lists the PDF files of the external differential corpus (README in
// engine/reference/corpus-20261015) and writes the manifest the runner reads.
// The corpus is untrusted data kept outside the repository: this script reads
// bytes only to hash them and to look for the "%PDF-" header, records only
// paths, sizes and hashes, never executes anything in the corpus (git commits
// are read from .git files, not with git), and never reads the one private user document (matched by the SHA-256 of its file name).
// usage: node engine/tools/corpus-manifest.mjs [--corpus DIR] [--out FILE]
//   DIR defaults to $TPDF_CORPUS, else the first tpdf-corpus directory beside
//   the repository or beside its parent (worktrees live one level deeper).
import { createHash } from "node:crypto";
import { closeSync, existsSync, lstatSync, openSync, readFileSync, readSync, readdirSync, writeFileSync, mkdirSync, createReadStream } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const args = process.argv.slice(2);
const flag = (name, fallback) => {
  const at = args.indexOf(name);
  return at >= 0 && args[at + 1] !== undefined ? args[at + 1] : fallback;
};
const corpus = path.resolve(flag("--corpus", process.env.TPDF_CORPUS ?? defaultCorpus()));
const out = path.resolve(flag("--out", path.join(root, "engine/reference/corpus-20261015/manifest.json")));

export function defaultCorpus() {
  const candidates = [path.join(root, "../tpdf-corpus"), path.join(root, "../../tpdf-corpus")];
  return candidates.find(dir => existsSync(dir)) ?? candidates[0];
}

/** Never opened, hashed or listed (project rule). */
const FORBIDDEN_NAME_SHA256 = new Set(["79c4929919f1d1f0eea98be823f0ccd3ecdcb5bd20707473cfaf177c3c5395c7"]);
/** How far into a file the "%PDF-" header is looked for (MuPDF and Acrobat accept leading junk). */
const SIGNATURE_WINDOW = 1024;
/** An indirect object header (`12 0 obj`), the syntax a PDF with a junk prefix still has and a text file quoting "%PDF-" lacks. */
const OBJECT_SYNTAX = /\d+[\0\t\n\f\r ]+\d+[\0\t\n\f\r ]+obj/;
const SIGNATURE_RULE = "\"%PDF-\" within the first 1,024 bytes that starts the file or a line, or follows only whitespace or non-printing bytes; "
  + "elsewhere in a text line it counts only when the file also has an indirect object header (a fuzzed PDF with a junk prefix, not a text file quoting the header)";

export const SOURCES = [
  {
    name: "qpdf", dir: "qpdf", kind: "git", origin: "https://github.com/qpdf/qpdf.git",
    licence: "Apache-2.0", note: "test inputs and outputs under qpdf/qtest, examples/qtest, libtests and elsewhere",
  },
  {
    name: "pdfjs", dir: "pdfjs", kind: "git", origin: "https://github.com/mozilla/pdf.js.git",
    licence: "Apache-2.0 (repository); per-file provenance of test/pdfs is mixed",
    note: "in-repo PDFs, mostly test/pdfs; *.link files (URLs of PDFs not in the repository) are ignored",
    skip: name => name.endsWith(".link"),
  },
  {
    name: "verapdf", dir: "verapdf", kind: "git", origin: "https://github.com/veraPDF/veraPDF-corpus.git",
    licence: "CC-BY-4.0 (repository); it also bundles separately published suites, see provenance",
    provenance: [
      { prefix: "Isartor test files/", note: "the Isartor PDF/A-1 test suite, published separately by the PDF Association and bundled in the repository; the repository README describes its own suite as complementing Isartor" },
      { prefix: "TWG test files/", note: "a separately contributed set bundled in the repository; the repository states no terms of its own for it, so treat its provenance as mixed" },
    ],
  },
  {
    name: "pdf20examples", dir: "pdf20examples", kind: "git", origin: "https://github.com/pdf-association/pdf20examples.git",
    licence: "CC-BY-SA-4.0",
  },
  {
    name: "pyhanko", dir: "pyhanko", kind: "git", origin: "https://github.com/MatthiasValvekens/pyHanko.git",
    licence: "MIT (repository); some test files come from elsewhere, see provenance",
    provenance: [
      {
        prefix: "internal/common-test-utils/src/pyhanko_testing_commons/test_data/data/pdf/safedocs/",
        note: "a SOURCE file in this directory points to the PDF Association safedocs repository (https://github.com/pdf-association/safedocs)",
      },
      {
        prefix: "internal/common-test-utils/src/pyhanko_testing_commons/test_data/data/pdf/",
        note: "a file named source in this directory states the origin and licence of some of its files",
      },
    ],
  },
  {
    name: "irs", dir: "irs", kind: "download", origin: "https://www.irs.gov/pub/irs-pdf/<name>.pdf",
    licence: "US public domain (17 U.S.C. 105)", downloaded: "2026-10-15",
  },
  {
    name: "govdocs1", dir: "govdocs1", kind: "download",
    origin: "https://downloads.digitalcorpora.org/corpora/files/govdocs1/zipfiles/000.zip",
    licence: "US government web documents (Digital Corpora Govdocs1)", downloaded: "2026-10-15",
    note: "the 200 PDFs of batch 000.zip; the archive itself is kept in govdocs1-zip/000.zip and is not walked",
    archive: "govdocs1-zip/000.zip",
  },
];

/** The checked-out commit, read from the .git files (no git process runs in the corpus). */
function gitCommit(dir) {
  const git = path.join(dir, ".git");
  const head = readFileSync(path.join(git, "HEAD"), "utf8").trim();
  if (/^[0-9a-f]{40}$/.test(head)) return head;
  const ref = head.match(/^ref: (\S+)$/)?.[1];
  if (!ref || ref.includes("..")) throw new Error(`${dir}: unreadable HEAD`);
  const loose = path.join(git, ref);
  if (existsSync(loose)) return readFileSync(loose, "utf8").trim();
  for (const line of readFileSync(path.join(git, "packed-refs"), "utf8").split("\n")) {
    const [hash, name] = line.split(" ");
    if (name === ref && /^[0-9a-f]{40}$/.test(hash)) return hash;
  }
  throw new Error(`${dir}: ref ${ref} not found`);
}

function head(file) {
  const fd = openSync(file, "r");
  try {
    const buffer = Buffer.alloc(SIGNATURE_WINDOW);
    const read = readSync(fd, buffer, 0, SIGNATURE_WINDOW, 0);
    return buffer.subarray(0, read);
  } finally {
    closeSync(fd);
  }
}

/**
 * The offset of the file's PDF header, or -1 (SIGNATURE_RULE). A header that
 * starts a line or follows only whitespace or non-printing bytes counts as
 * read; one in the middle of a text line (a fuzzer dictionary, a tokenizer
 * dump, a diff) counts only when the file also has PDF object syntax.
 */
export function headerOffset(file) {
  const bytes = head(file);
  const offset = bytes.indexOf("%PDF-");
  if (offset <= 0) return offset;
  const before = bytes[offset - 1];
  if (before === 0x0a || before === 0x0d) return offset;
  if (bytes.subarray(0, offset).every(byte => byte < 0x21 || byte > 0x7e)) return offset;
  return OBJECT_SYNTAX.test(readFileSync(file).toString("latin1")) ? offset : -1;
}

function sha256(file) {
  return new Promise((resolve, reject) => {
    const hash = createHash("sha256");
    createReadStream(file).on("data", chunk => hash.update(chunk)).on("end", () => resolve(hash.digest("hex"))).on("error", reject);
  });
}

/** Regular files under dir (relative paths, sorted), skipping .git, symlinks and the forbidden name. */
function walk(dir, skip, counters) {
  const files = [];
  const visit = relative => {
    const entries = readdirSync(path.join(dir, relative), { withFileTypes: true })
      .sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
    for (const entry of entries) {
      const rel = relative ? `${relative}/${entry.name}` : entry.name;
      if (entry.name === ".git" && !relative) continue;
      if (FORBIDDEN_NAME_SHA256.has(createHash("sha256").update(entry.name).digest("hex"))) { counters.forbidden++; continue; }
      if (entry.isSymbolicLink()) { counters.symlinks++; continue; }
      if (entry.isDirectory()) visit(rel);
      else if (entry.isFile() && !skip?.(entry.name)) files.push(rel);
    }
  };
  visit("");
  return files;
}

async function main() {
  const sources = [];
  const byHash = new Map();
  for (const source of SOURCES) {
    const dir = path.join(corpus, source.dir);
    if (!existsSync(dir)) throw new Error(`missing corpus source ${source.name}: ${dir}`);
    const counters = { forbidden: 0, symlinks: 0, scanned: 0 };
    const entry = {
      name: source.name, dir: source.dir, origin: source.origin, licence: source.licence,
      ...(source.kind === "git" ? { commit: gitCommit(dir) } : { downloaded: source.downloaded }),
      ...(source.note ? { note: source.note } : {}),
    };
    const provenance = (source.provenance ?? []).map(item => ({ ...item, files: 0 }));
    if (source.archive) {
      const archive = path.join(corpus, source.archive);
      entry.archive = { path: source.archive, bytes: lstatSync(archive).size, sha256: await sha256(archive) };
    }
    let files = 0, bytes = 0, pdf = 0, misnamed = 0, notPdf = 0;
    for (const rel of walk(dir, source.skip, counters)) {
      counters.scanned++;
      const file = path.join(dir, rel);
      const offset = headerOffset(file);
      const signature = offset >= 0;
      const named = /\.pdf$/i.test(rel);
      if (!signature && !named) continue;
      const size = lstatSync(file).size;
      const hash = await sha256(file);
      files++;
      bytes += size;
      if (signature) pdf++;
      if (signature && !named) misnamed++;
      if (!signature) notPdf++;
      let record = byHash.get(hash);
      if (!record) {
        record = { sha256: hash, bytes: size, signature, ...(offset > 0 ? { signatureOffset: offset } : {}), locations: [] };
        byHash.set(hash, record);
      }
      record.locations.push({ source: source.name, path: rel, ...(named ? {} : { extension: false }) });
      // The most specific provenance note covers a file (provenance lists the deeper prefix first).
      const covering = provenance.find(item => rel.startsWith(item.prefix));
      if (covering) covering.files++;
    }
    if (provenance.length) entry.provenance = provenance;
    Object.assign(entry, {
      files, bytes, withSignature: pdf, signatureWithoutPdfExtension: misnamed, pdfExtensionWithoutSignature: notPdf,
      scannedFiles: counters.scanned, skippedSymlinks: counters.symlinks, skippedForbidden: counters.forbidden,
    });
    sources.push(entry);
    console.error(`${source.name}: ${files} files (${pdf} with %PDF-, ${notPdf} .pdf without), ${bytes} bytes`);
  }
  const order = new Map(SOURCES.map((source, index) => [source.name, index]));
  const files = [...byHash.values()].sort((a, b) => {
    const [x, y] = [a.locations[0], b.locations[0]];
    return order.get(x.source) - order.get(y.source) || (x.path < y.path ? -1 : x.path > y.path ? 1 : 0);
  });
  const duplicates = files.filter(file => file.locations.length > 1);
  const manifest = {
    schema: "tpdf-corpus-manifest/1",
    created: new Date().toISOString().slice(0, 10),
    corpusRoot: "outside the repository (see README.md); paths are relative to each source directory",
    signatureWindow: SIGNATURE_WINDOW,
    signatureRule: SIGNATURE_RULE,
    excludedNameSha256: [...FORBIDDEN_NAME_SHA256],
    sources,
    totals: {
      locations: files.reduce((n, file) => n + file.locations.length, 0),
      uniqueFiles: files.length,
      uniqueBytes: files.reduce((n, file) => n + file.bytes, 0),
      uniqueWithSignature: files.filter(file => file.signature).length,
      duplicateGroups: duplicates.length,
      crossSourceDuplicates: duplicates.filter(file => new Set(file.locations.map(l => l.source)).size > 1).length,
    },
    files,
  };
  // One file per line keeps the manifest diffable.
  const { files: _, ...header } = manifest;
  const text = JSON.stringify(header, null, 1).replace(/\n\}$/, "");
  mkdirSync(path.dirname(out), { recursive: true });
  writeFileSync(out, `${text},\n "files": [\n${files.map(file => `  ${JSON.stringify(file)}`).join(",\n")}\n ]\n}\n`);
  JSON.parse(readFileSync(out, "utf8"));
  console.error(`wrote ${path.relative(root, out)}: ${files.length} unique files, ${duplicates.length} duplicate groups`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
