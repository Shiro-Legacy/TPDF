# Clean-room rewrite: evidence for legal review

This page summarises how the MuPDF-derived parts of the Tandem PDF engine
were replaced, so that counsel can judge whether the engine may be released
under the MIT licence. The plan is in `docs/mit-clean-room-plan.md`.

**Caveat, stated up front.** Every spec writer, implementer and reviewer was
an AI model (Claude). Models of this kind have very likely seen MuPDF's
public source code during training. The process below keeps the implementers
away from the MuPDF-derived code and from MuPDF itself during the work, but it
cannot remove what a model learned earlier. How much protection this "clean
room" gives is a legal judgement.

## What was replaced

A read-only audit (2026-10-17) classified every engine file:

- **Class A:** derived from MuPDF source and shipped. About 11,000–12,000
  lines in eight workstreams, all rewritten:
  - 01 lexer, parser, dictionary, serializer;
  - 02 stream filters;
  - 03 repair;
  - 04 text extraction and page frame;
  - 05 glyph-to-Unicode and simple encodings;
  - 06 annotation appearances;
  - 07 widget appearances, variable text and fallback fonts;
  - 08 image resolution.
- **Class B:** derived but dead in the app. About 25,000 lines reachable only
  through a legacy WASM export, all deleted (Phase 0).
- **Class C:** written by the project and only compared with MuPDF output as
  a black box. Comments naming MuPDF internals were reworded. One inherited
  constant, the `startxref` search window, was re-derived from a corpus
  survey.
- **Class D:** data tables taken from MuPDF files. All were regenerated from
  their original public sources: the Adobe Glyph List, Adobe's CMaps and
  Core 14 AFMs, the Unicode Character Database, ISO 32000-2 Annex D, and
  Apple's Macintosh glyph order.

## Process

1. **Specs (dirty side).** For each workstream, a spec writer that could read
   the old code wrote a functional spec under `docs/clean-room/SPEC-RULES.md`:
   - no code;
   - no MuPDF names;
   - no constants copied from the old code;
   - no MuPDF artwork;
   - behaviour beyond the PDF standard recorded only as black-box experiments,
     with input, command and output.

   Two independent leak checkers then edited the specs to remove anything that
   slipped through. The owner settled the open questions
   (`docs/clean-room/OWNER-DECISIONS.md`).
2. **Acceptance tests (dirty side).** Tests that asserted MuPDF's exact output
   were replaced by tests of the spec's behaviour. The ledgers are in
   `docs/clean-room/acceptance/`.
3. **Hand-off trees.** `engine/tools/cleanroom/make-handoff.sh` built one tree
   per workstream with `git archive`, so no history came with it. Each tree
   held:
   - the engine with every class A file replaced by an interface stub
     (`todo!()` bodies);
   - the workstream's specs and acceptance tests;
   - no oracle tools, design notes or MuPDF-produced evidence.

   `check-handoff.sh` rejected any tree that named MuPDF or held a copy of an
   old class A file. Every tree passed with 0 leaks. The trees are kept in
   `tpdf-wt/clean-room-evidence/handoff-trees/`.
4. **Clean implementation.** Each implementer was told to:
   - read only its tree, public standards and the public data the spec named;
   - never open the repository, MuPDF or mupdf.js;
   - record every source in `SOURCES.md`.

   Their notes and logs are in `docs/clean-room/evidence/<workstream>/`.
5. **Review and integration.** An independent reviewer, who could see
   everything, checked each result against its spec and against the old code
   and MuPDF for resemblance. Every workstream was accepted with fixes, and
   the fixes went back to the same clean implementer. The orchestrator wrote
   only integration glue in surviving code: box handling in `view_fixes.rs`,
   test expectations and NOTICE text.

## Audit of the implementers' transcripts

Full transcripts of the seven implementers, with SHA-256 hashes, are in
`tpdf-wt/clean-room-evidence/transcripts/`, outside the repository.
`audit.py` lists every tool call that names a path outside the implementer's
own tree. Its result (`audit-2026-10-18.txt`):

- No implementer opened the repository, the old code, MuPDF, mupdf.js or
  `~/.codex`.
- The only hits are text in SOURCES/NOTES saying MuPDF was *not* consulted,
  and two reads of the implementer's own saved command output.
- Workstream 04 ran one stray command,
  `find / -path /System -prune -o -name "*.rs" -path "*hayro-cmap*" -print | head -0`.
  It searched for source files of hayro, a vendored Apache-2.0/MIT library,
  printed nothing and opened nothing. The implementer reported it, and its
  NOTES record it.
- Downloads came only from the public homes the specs named:
  - workstream 05 fetched the Adobe Glyph List from GitHub, Adobe's
    symbol and dingbat mappings and code-page files from unicode.org, and
    UCD 16.0;
  - workstream 07 fetched code-page mappings and the Adobe CMaps and AGL.
  - Workstream 06 used Adobe Core 14 AFMs that the orchestrator supplied, with
    Adobe's MustRead terms.

## Resemblance findings

Reviewers compared each result with the old code and with MuPDF as they know
it. All reported low resemblance.

- **Different designs:**
  - repair uses a candidate list and a read-through budget, not a lexer loop;
  - text extraction collects glyphs first, then lays them out, and uses a
    grid hash for overprints;
  - LZW is written back to front along its prefix chain;
  - dictionaries keep insertion order with an index.
- **New artwork and constants** in workstreams 06 and 07.
- **Lines identical to the old code** are interface signatures, the spec's
  error messages, and standard-dictated values (TIFF tags, LZW code widths,
  Annex D tables).
- One threshold, the 0.15 em space gap, equals the old value. The reviewer
  traced it to the black-box experiment E3 recorded in spec 04, where the
  reference viewer's switch point lies between 0.14 and 0.16 em.

## What stays AGPL

`engine/oracle/` keeps the native MuPDF oracle programs, the scripts that
import the `mupdf` npm package, and the old TypeScript MuPDF engine used as a
test oracle. It carries its own COPYING and README. It is test-only, built
into no shipped package, and outside the MIT grant. The npm `mupdf` package
is a devDependency only.
