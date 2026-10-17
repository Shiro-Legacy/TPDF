# Clean-room spec rules

These rules apply to every spec in `docs/clean-room/specs/`. A spec is the only
description of behaviour a clean implementer receives, besides ISO 32000-2,
other public standards, and the acceptance tests. The implementer never sees the
code being replaced, its git history, its design notes, or MuPDF source.

The overall plan is in `docs/mit-clean-room-plan.md`.

## What a spec contains

1. **Scope.** What the component does and which callers depend on it.
2. **Interface.** The public Rust types and function signatures that surviving
   code calls, each with its meaning: inputs, outputs, errors, limits.
   Interfaces are functional, so they may be described exactly. Private
   helpers, internal structure and control flow are not part of the interface.
3. **Required behaviour.** Prose, tables and grammars, citing ISO 32000-2
   section numbers or other public standards (TIFF 6.0, PNG, EXIF, UAX #9,
   Adobe Glyph List, OpenType).
4. **Tolerant behaviour.** Behaviour beyond the spec, such as recovering from
   malformed input, written as observable input→output facts. Each fact must be
   re-observed by running a reference program (MuPDF via the oracle tools or
   mupdf.js, pdf.js, qpdf, Acrobat behaviour from public docs) on an input you
   construct. Record the experiment: input, command, observed output. Where
   viewers disagree, choose the behaviour that is best for users and say why.
5. **Limits and safety.** Resource bounds, error policy, no panics, no unsafe.
6. **Acceptance.** A test inventory:
   - existing tests that are black-box (input → expected output) and stay;
   - existing tests that assert equality with MuPDF's internal output (byte-equal
     streams, MuPDF artwork, MuPDF constants), which must be replaced, with the
     replacement check described;
   - new cases needed.

## What a spec must not contain

- Code or pseudocode from the current implementation, or prose that walks
  through its control flow step by step.
- MuPDF function, file, struct, macro or constant names (`pdf_*`, `fz_*`,
  `*.c`), or statements about how MuPDF works internally.
- Numeric constants copied from the current code or its comments. A number may
  appear only if a standard defines it, or if you measured it in a recorded
  black-box experiment. For heuristic thresholds (spacing, paragraph breaks,
  repair scan windows), prefer stating the acceptance criterion and leaving the
  value to the implementer.
- Expressive content taken from MuPDF: icon path data, stamp artwork, label
  tables, default colours chosen by MuPDF. Describe the need ("a recognisable
  comment icon that fits the /Rect") and leave the design to the implementer.
- Data tables. Name the public source to regenerate them from (Annex D, the
  Adobe Glyph List, the Unicode mapping files, UnicodeData.txt, URW AFMs).

If in doubt, leave it out and write the acceptance criterion instead.
