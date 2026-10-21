# engine/oracle: the MuPDF test oracle (AGPL-3.0-or-later)

Test-only tooling that depends on MuPDF, which is licensed under the GNU Affero
General Public License, version 3 or later (AGPL-3.0-or-later). None of it is part
of any shipped build: the desktop app (`crates/`) links the Rust engine natively,
and the WASM package (`engine/web/pkg`) is built without this directory. The
directory is excluded from the MIT grant that covers the rest of the repository;
its contents are under the AGPL, whose text is in [`COPYING`](COPYING).

- `app-engine/`: the former TypeScript app engine on MuPDF (`engine.ts`,
  `content-edit.ts`, `redact.ts`, `sign-prepare.ts`, `mupdf-core.ts`), kept as the
  MuPDF reader the app test harness (`tests/rust-engine.mjs`,
  `tests/mupdf-oracle.mjs`) uses to inspect the Rust engine's output.
  `npm run build:oracle` compiles it to `build-oracle/`; it imports the npm `mupdf`
  package, a devDependency.
- The generators of the pinned standard-font tables: `generate-builtin-metrics.py`
  (`src/builtin_metrics_data.rs`), `generate-builtin-outlines.py` and
  `generate-substitute-outlines.py` (`src/substitute_metrics_data.rs`, and the
  Rust outline tables that `engine/generate/pack_outlines.py` packs into
  `src/builtin_outlines/*.bin`), with the native oracle they run,
  `mupdf-builtin-outlines-oracle.c`, built against the pinned MuPDF 1.28.1 source
  by `build-builtin-outlines-reference.sh`. Usage is in each header comment; paths
  are relative to the repository root unless a header says `engine/`.

The differential harness that compared the engine's lower layers with MuPDF (the
lexer, filter, document and rendering oracles, the Python drivers, the fuzzers,
the corpus runner and the bench scripts) was removed on October 21, 2026; nothing
in the tree ran it any more. It is in the git history before that date. The tools
for the engine modules removed in the MIT clean-room plan's Phase 0
(`docs/mit-clean-room-plan.md`) were deleted earlier and remain in the history up
to commit 9c6b66b, under `engine/tools/`. `engine/reference/PROVENANCE.md`
describes the evidence they produced.
