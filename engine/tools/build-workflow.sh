#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
set -eu
ENGINE_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
CARGO_BIN=${CARGO_BIN:-cargo}
WASM_BINDGEN_BIN=${WASM_BINDGEN_BIN:-wasm-bindgen}
CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$ENGINE_DIR/bridge/target}
export CARGO_TARGET_DIR
"$CARGO_BIN" build --manifest-path "$ENGINE_DIR/bridge/Cargo.toml" --locked --release --target wasm32-unknown-unknown --lib
"$WASM_BINDGEN_BIN" "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/tandem_rust_workflow.wasm" --target web --out-dir "$ENGINE_DIR/web/pkg"
"$CARGO_BIN" build --manifest-path "$ENGINE_DIR/bridge/Cargo.toml" --locked --release --bin tandem-rust-workflow
