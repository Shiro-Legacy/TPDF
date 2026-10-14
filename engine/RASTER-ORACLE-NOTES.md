# Test-only aliased line-fill raster oracle

`tools/build-raster-reference.sh /private/tmp/tandem-mupdf-reference` builds the
separate pinned native tool without changing existing oracle formats.
`mupdf-raster-oracle WIDTH HEIGHT` reads standalone content on stdin and emits an
opaque RGB P6 PPM after successful completion. The canvas starts white, CTM is
identity, and `fz_set_aa_level(ctx, 0)` disables antialiasing. No font programs,
external resources, full PDF pages or production Rust rasterizer are involved.

The allowlist is q/Q/cm/m/l/h/re/f/F/f*/n/W/W*/g/rg. Strokes, curves, CMYK,
text/resources and all other operators refuse before native execution. Caps are
1 MiB input, 1..4096 dimensions, 4,194,304 pixels, 128 MiB native allocation,
100,000 lexical tokens, absolute numeric operand magnitude one million,
10 CPU seconds and 15 wall seconds. Native warnings/errors refuse with empty
stdout. PPM publication begins only after rendering and diagnostic checks finish.
Exit1 denotes argument/input/output setup failure; exit2 denotes native/limit
failure. As with any stdout writer, an output I/O failure can interrupt publication.

`tools/check-raster-reference.py` validates 200 generated nondegenerate integer
compound-rectangle programs against an independent mask model: 204,800 pixels
match exactly. Both fill rules, signed extents, clipping, q/Q restoration and
primary RGB colors are exercised. Twelve argument/input/operator/diagnostic
refusals all produce no stdout. The executable hash is unchanged. Full evidence
is in `reference/raster-oracle-validation.json`.

Observed native raster conventions must remain separate from future Rust behavior:

- Fractional rectangle `1.6 2.6 3 2 re f` paints x1..3/y2..3, unlike conventional
  pixel-center inclusion. Eight fractional-origin cases are recorded separately.
- Native AA0 zero-area rectangles can paint thin rows/columns. For example
  `17 9 -14 0 re f` paints x3..16 at y9. The initial 200-case experiment containing
  zero extents had 40 differences against mathematical empty-fill expectations;
  its unmodified report is preserved as
  `reference/raster-native-degenerate-investigation.json`. These are observed
  native conventions, not failed production Rust tests or a false parity claim.
- Integer polygon vertices alone do not establish a shared sampling convention.
  Native triangle `0 0 m 8 0 l 0 4 l f` paints row widths8,6,4,2; center sampling
  would paint7,5,3,1. The simple slope-one triangle masks this distinction through
  boundary ties. Three exact native slanted-polygon rasters are recorded in the
  validation report. Integer-origin sampling is also not a complete description
  of native edge rounding: an exploratory rational-intersection model differed
  on75/100 integer triangles. No general native polygon formula is asserted here.

The proposed renderer must choose a documented supported subset and test its own
coverage rules; neither integer vertices nor one matching triangle prove native
pixel compatibility. The exact rectangle checks above do not authorize a Rust
raster implementation or imply rendering coverage for existing documents.

## Isolated candidate backend comparison

`tools/probe-raster-backends.py` compares the native oracle with the isolated
tiny-skia audit binary supplied by the parser agent; it does not add a production
engine dependency. With seed9301, all50 generated nondegenerate integer rectangles
match, but only16 of200 generated integer triangles match. Across255 total inputs,
68match and187differ. The fractional, zero-area and three targeted triangle cases
remain explicit observations. Binary hashes stayed unchanged.
`reference/raster-backend-coverage-probe.json` records every polygon and pixel hash.
These differences are design evidence, not a failed production Rust acceptance
gate. Exact native pixel comparison should initially restrict geometry further
than merely requiring integer vertices.
