#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
set -eu
ENGINE_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
CARGO_BIN=${CARGO_BIN:-cargo}
WASM_BINDGEN_BIN=${WASM_BINDGEN_BIN:-wasm-bindgen}
CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$ENGINE_DIR/bridge/target}
export CARGO_TARGET_DIR
# Read before the build, stored only once the package is written (scripts/build.mjs checks it).
SOURCES=$(sh "$ENGINE_DIR/tools/workflow-sources.sh")
"$CARGO_BIN" build --manifest-path "$ENGINE_DIR/bridge/Cargo.toml" --locked --release --target wasm32-unknown-unknown --lib
rm -f "$ENGINE_DIR/web/pkg/sources.sha256"
"$WASM_BINDGEN_BIN" "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/tandem_rust_workflow.wasm" --target web --out-dir "$ENGINE_DIR/web/pkg"
printf '%s\n' "$SOURCES" > "$ENGINE_DIR/web/pkg/sources.sha256"
"$CARGO_BIN" build --manifest-path "$ENGINE_DIR/bridge/Cargo.toml" --locked --release --bin tandem-rust-workflow
