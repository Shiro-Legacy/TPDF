// The former TypeScript app engine on MuPDF, now the test-only MuPDF oracle
// (engine/oracle/app-engine, AGPL-3.0-or-later; see engine/oracle/README.md).
// The harness (rust-engine.mjs) reads the Rust engine's output with it, and a
// few tests render the same page with it for comparison. `npm run build:oracle`
// compiles it to build-oracle/; a missing build fails this import.
export * from '../build-oracle/engine/oracle/app-engine/engine.js';
