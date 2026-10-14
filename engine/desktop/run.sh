#!/bin/sh
set -eu
ENGINE_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
TPDF_APP_ROOT=${TPDF_APP_ROOT:-$PWD}
export TPDF_APP_ROOT
exec node "$TPDF_APP_ROOT/node_modules/electron/cli.js" "$ENGINE_DIR/desktop/main.cjs" "$@"
