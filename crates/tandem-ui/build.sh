#!/bin/sh
# Builds the UI into dist/: the crate as WASM, wasm-bindgen's web glue, the
# page and its styles. Usage: sh build.sh [--release]
set -e
cd "$(dirname "$0")"
PROFILE=${1:---release}
WBG=${WASM_BINDGEN_BIN:-$HOME/.local/tpdf-wasm-tools/bin/wasm-bindgen}
cargo build -p tandem-ui --lib --target wasm32-unknown-unknown $PROFILE
DIR=release; [ "$PROFILE" = "--release" ] || DIR=debug
rm -rf dist && mkdir -p dist
"$WBG" ../target/wasm32-unknown-unknown/$DIR/tandem_ui.wasm --target web --no-typescript --out-dir dist
cp index.html styles.css dist/
[ -d assets ] && cp -R assets dist/ || true
