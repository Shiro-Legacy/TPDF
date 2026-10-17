# Robustness fuzzers (M10)

Copies of the fuzz and probe scripts that earlier sessions wrote in scratch directories, kept here so the
M10 campaign (`engine/design/m10-fuzz.md`) can be rerun from the repository. They make inputs only from
small synthetic PDFs built in the script and from the repository's own `fixtures/`; none reads the external
corpus, and none writes into the repository. `engine/tools/mutation-smoke.py` (already here) is the sixth tool.

## Origin

| File | Copied from (scratch, not in any repository) | Written for |
|---|---|---|
| `p6_fuzz.rs` | `tpdf-wt/scratch/p6-robust/examples/p6_fuzz.rs` | P6 robust rendering (`engine/design/app-corpus-robust.md`, "scratch harness") |
| `lib.mjs`, `lib2.mjs` | `tpdf-wt/scratch/p8/open/probes/` | round 3 open/repair lane and its review |
| `p7-fuzz.mjs`, `p22-lexfuzz.mjs`, `p25-bytefuzz.mjs`, `q3-cutfuzz.mjs` | `tpdf-wt/scratch/p8/open/probes/` | same |
| `r1-fakestream.mjs`, `r1b-xreffake.mjs`, `r2-incremental.mjs` … `r6-dups.mjs` | `tpdf-wt/scratch/p8/review-open/probes/` | the review of the open lane |

The generation and mutation code is unchanged. The only edits make hard-coded paths of the scratch machine
overridable, one line each:

- `lib.mjs`, `lib2.mjs`: `APP` (the app checkout whose `build/` and `engine/web/pkg` are loaded) is
  `$TPDF_APP`, default this repository. `lib2.mjs`: the two baseline packages (`rsF04`, `rsR2`) come from
  `$RS_BASE_F04` / `$RS_BASE_R2` (a directory holding `tandem_rust_workflow.js`); unset, they are absent.
- `p7-fuzz.mjs`, `p22-lexfuzz.mjs`, `p25-bytefuzz.mjs`: the dump directory is `$FUZZ_DUMP` (default
  `<os tmpdir>/tpdf-fuzz`). `p7-fuzz.mjs` does not create its `fz/` subdirectory; make it first.
- `r*.mjs`: import `./lib2.mjs` instead of the absolute scratch path.
- `p6_fuzz.rs`: the optional dump case goes to `$FUZZ_DUMP` (default `.`).

## Running

Every run is heavy: on the shared 8 GB machine, go through `tpdf-wt/tools/slot` (two at a time, 3 GB cap).
Build first: `npm run build` (app) and the WASM package (`engine/tools/build-workflow.sh`).

```sh
cd engine/tools/fuzz
export FUZZ_DUMP=/path/outside/the/repo NO_BASE=1; mkdir -p $FUZZ_DUMP/fz
node p7-fuzz.mjs SEED N        # open/authenticate/security-set/save/validate on mutated fixtures; MuPDF open as reference
node p22-lexfuzz.mjs SEED N    # damaged tokens in a 3-page file's dictionaries; view and save vs MuPDF
node p25-bytefuzz.mjs SEED N   # 1-2 byte edits in dictionaries; save read back by MuPDF
node q3-cutfuzz.mjs SEED N     # one object cut short and moved; save read back by MuPDF
node r1-fakestream.mjs         # (and r1b … r6) fixed probe sets, no seed
```

`p6_fuzz.rs` is an engine example: copy it to `engine/examples/` for the build only (it is not rustfmt or
clippy clean, so it must not stay there), then

```sh
cargo build --release --example p6_fuzz && rm examples/p6_fuzz.rs
target/release/examples/p6_fuzz KIND SEED COUNT   # KIND: color shading tiling image mesh
python3 tools/mutation-smoke.py --rust target/release/pdf-inspect --seed SEED --cases N \
  --report /outside/report.json --failures-dir /outside/failures
```

Each JS fuzzer prints one JSON line of counts, then examples (`i=` is the case index for that seed and
N). A Rust trap shows as a reply `reason: "worker"` ("the PDF engine stopped after an internal error");
`p7-fuzz.mjs` counts them as `traps`, the others as refusals or save failures with that message.
Dumped inputs are synthetic or derived from repository fixtures; still keep them out of the repository.
`p7-fuzz.mjs` writes three of its seed files with MuPDF's encryption, whose key and IVs are random,
so its counts move by a few percent between runs; the other tools are deterministic for a seed.
