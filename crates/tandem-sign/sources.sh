#!/bin/sh
# SPDX-License-Identifier: MIT
# Prints "<sha256>  <path>" for each source the tandem-sign WASM module is built from, sorted, paths relative
# to crates/tandem-sign. engine/tools/build-workflow.sh stores it as pkg/sources.sha256; the app build
# (scripts/build.mjs) recomputes it and refuses a module whose sources have changed since.
set -eu
SIGN_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
if command -v sha256sum >/dev/null 2>&1; then SHA256='sha256sum'; else SHA256='shasum -a 256'; fi
cd "$SIGN_DIR"
find Cargo.toml Cargo.lock src -type f | LC_ALL=C sort | tr '\n' '\0' | xargs -0 $SHA256
