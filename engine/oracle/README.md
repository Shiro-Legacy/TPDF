# engine/oracle: MuPDF test oracles (AGPL-3.0-or-later)

This directory is test-only reference tooling. Everything here links, imports
or otherwise depends on MuPDF, which is licensed under the GNU Affero General
Public License, version 3 or later (AGPL-3.0-or-later):

- the native oracles (`mupdf-*-oracle.c`) and the scripts that build them
  against the pinned MuPDF 1.28.1 source (`build-*-reference.sh`);
- the Python differential drivers and reference models (`differential*.py`,
  `compare-*.py`, `generate-*.py`, `encoding-table-inventory.py`) that run
  those oracles or read the pinned MuPDF source;
- the Node scripts that import the npm `mupdf` package (a devDependency), or
  the TypeScript MuPDF app engine in `app-engine/`, to compare the Rust engine
  with MuPDF (`compare-*.mjs`, `app-forms-*.mjs`, `corpus-run.mjs`, `fuzz/`,
  `bench/` and the inventories);
- `app-engine/`: the former TypeScript app engine on MuPDF (`engine.ts`,
  `content-edit.ts`, `redact.ts`, `sign-prepare.ts`, `mupdf-core.ts`), kept as
  the comparison side of those scripts and as the MuPDF reader that the app
  test harness (`tests/rust-engine.mjs`) uses to inspect the Rust engine's
  output. `npm run build:oracle` compiles it to `build-oracle/`.
- `freetype-cff-outlines.c`, a FreeType-based outline dump kept with the
  other reference tooling.

None of it is part of any shipped build: the app, the mobile app and the WASM
package are built without this directory, and the app packages are checked to
hold no MuPDF code (`scripts/check-package.mjs`). This directory is excluded
from the MIT grant that covers the rest of the repository; its contents are
under the AGPL, whose text is in [`COPYING`](COPYING).

The tools for the engine modules removed in the MIT clean-room plan's Phase 0
(`docs/mit-clean-room-plan.md`), such as the path, program, raster, glyph,
image, form and text-position oracles and their models, were deleted rather
than moved. They remain in the git history up to commit 9c6b66b, under
`engine/tools/`. `engine/reference/PROVENANCE.md` describes the evidence they
produced.

Usage of each tool is in its header comment. Paths are relative to the
repository root unless a header says `engine/`.
