#!/bin/sh
# SPDX-License-Identifier: MIT
set -eu
# --wasm-only skips the native tandem-rust-workflow binary at the end (CI, which ships only the package).
NATIVE_CLI=1
if [ "${1:-}" = "--wasm-only" ]; then NATIVE_CLI=0; elif [ $# -gt 0 ]; then echo "usage: $0 [--wasm-only]" >&2; exit 2; fi
ENGINE_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
CARGO_BIN=${CARGO_BIN:-cargo}
WASM_BINDGEN_BIN=${WASM_BINDGEN_BIN:-wasm-bindgen}
CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$ENGINE_DIR/bridge/target}
mkdir -p "$CARGO_TARGET_DIR"
CARGO_TARGET_DIR=$(CDPATH= cd -- "$CARGO_TARGET_DIR" && pwd -P)
export CARGO_TARGET_DIR
# Reproducible WASM (docs/building-rust-release.md): rustc writes the path of every source file that can panic into
# the module. Remap the checkout, the target folder, CARGO_HOME (the registry sources) and RUSTUP_HOME to fixed
# prefixes, so the module's bytes do not depend on where the tree is checked out or who builds it. rustc applies the
# last matching remap, so the target folder (inside the checkout by default) comes last. Only the wasm32 build gets
# them: a caller's CARGO_ENCODED_RUSTFLAGS or RUSTFLAGS are kept, with the remaps appended (either replaces any
# `build.rustflags` from a cargo config, as cargo always does), and the native build below is unchanged.
physical() { if [ -d "$1" ]; then (CDPATH= cd -- "$1" && pwd -P); else printf '%s' "$1"; fi; }
US=$(printf '\037')
TPDF_RUSTUP_HOME=$(physical "${RUSTUP_HOME:-$HOME/.rustup}")
TPDF_CARGO_HOME=$(physical "${CARGO_HOME:-$HOME/.cargo}")
TPDF_ROOT=$(physical "$ENGINE_DIR/..")
REMAP="--remap-path-prefix=$TPDF_RUSTUP_HOME=/rustup"
REMAP="$REMAP$US--remap-path-prefix=$TPDF_CARGO_HOME=/cargo"
REMAP="$REMAP$US--remap-path-prefix=$TPDF_ROOT=/tpdf"
REMAP="$REMAP$US--remap-path-prefix=$CARGO_TARGET_DIR=/target"
# A remap that matches no path, named after the wrapper's content: cargo's fingerprint includes the flags, so a changed
# tools/rustc-stable-metadata.sh rebuilds the wasm32 crates instead of linking ones hashed by the old rule.
if command -v sha256sum >/dev/null 2>&1; then SHA256='sha256sum'; else SHA256='shasum -a 256'; fi
WRAPPER_ID=$($SHA256 < "$ENGINE_DIR/tools/rustc-stable-metadata.sh" | cut -c1-16)
REMAP="$REMAP$US--remap-path-prefix=/tpdf-stable-metadata-$WRAPPER_ID=/tpdf-stable-metadata"
if [ -n "${CARGO_ENCODED_RUSTFLAGS:-}" ]; then
  WASM_RUSTFLAGS="$CARGO_ENCODED_RUSTFLAGS$US$REMAP"
else
  WASM_RUSTFLAGS=
  set -f
  for flag in ${RUSTFLAGS:-}; do WASM_RUSTFLAGS="$WASM_RUSTFLAGS$flag$US"; done
  set +f
  WASM_RUSTFLAGS="$WASM_RUSTFLAGS$REMAP"
fi
# Read before the build, stored only once the package is written (scripts/build.mjs checks it).
SOURCES=$(sh "$ENGINE_DIR/tools/workflow-sources.sh")
# cargo also hashes the engine and vendor crates (path dependencies outside the bridge's workspace) by their absolute
# path into each crate's `-C metadata`; tools/rustc-stable-metadata.sh replaces that with a path-independent value.
# A caller's RUSTC_WRAPPER keeps running the compiler, behind it.
export TPDF_ROOT TPDF_CARGO_HOME TPDF_RUSTUP_HOME
TPDF_RUSTC_WRAPPER=${RUSTC_WRAPPER:-} RUSTC_WRAPPER="$ENGINE_DIR/tools/rustc-stable-metadata.sh" \
CARGO_ENCODED_RUSTFLAGS="$WASM_RUSTFLAGS" "$CARGO_BIN" build --manifest-path "$ENGINE_DIR/bridge/Cargo.toml" --locked --release --target wasm32-unknown-unknown --lib
rm -f "$ENGINE_DIR/web/pkg/sources.sha256"
"$WASM_BINDGEN_BIN" "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/tandem_rust_workflow.wasm" --target web --out-dir "$ENGINE_DIR/web/pkg"
printf '%s\n' "$SOURCES" > "$ENGINE_DIR/web/pkg/sources.sha256"
# The native CLI is built only while bridge/src/main.rs exists (the MIT plan's Phase 0 removes it).
if [ "$NATIVE_CLI" = 1 ] && [ -f "$ENGINE_DIR/bridge/src/main.rs" ]; then
  "$CARGO_BIN" build --manifest-path "$ENGINE_DIR/bridge/Cargo.toml" --locked --release --bin tandem-rust-workflow
fi
