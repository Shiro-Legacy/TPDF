#!/bin/sh
# SPDX-License-Identifier: MIT
# Prints "<sha256>  <path>" for each source the WASM package is built from, sorted, paths relative
# to engine/. build-workflow.sh stores it as web/pkg/sources.sha256; the app build (scripts/build.mjs)
# recomputes it and refuses a package whose sources have changed since.
set -eu
ENGINE_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if command -v sha256sum >/dev/null 2>&1; then SHA256='sha256sum'; else SHA256='shasum -a 256'; fi
cd "$ENGINE_DIR"
# vendor/ holds the patched crates the package is built from (Cargo.toml's [patch.crates-io]).
VENDOR=; if [ -d vendor ]; then VENDOR=vendor; fi
find Cargo.toml bridge/Cargo.toml bridge/Cargo.lock src bridge/src $VENDOR -type f | LC_ALL=C sort | tr '\n' '\0' | xargs -0 $SHA256
