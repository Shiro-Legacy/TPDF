#!/bin/sh
# SPDX-License-Identifier: MIT
# RUSTC_WRAPPER for build-workflow.sh's wasm32 build: replaces cargo's `-C metadata=<hash>` with a hash that does not
# depend on where the tree is checked out, so the WASM is the same from any checkout (docs/building-rust-release.md).
#
# Why: cargo hashes a path dependency that lies outside the workspace root by its absolute path. The bridge is its own
# workspace (engine/bridge), so the engine (`..`) and the vendored hayro crates (`../vendor/...`) get a crate hash that
# changes with the checkout path. rustc derives symbol names and item order from it, so the code changes too, not only
# names. Registry crates are hashed by their registry URL and are not affected.
#
# The new value hashes the crate's name and version and its rustc arguments, with the build-machine roots replaced by
# fixed names and cargo's own hashes left out: `-C metadata`, `-C extra-filename` and the `-<16 hex>` in dependency
# file names. A dependency is then identified by its crate name, as rustc sees it, so the value does not depend on
# whether the dependency was built in this run or is an older cached build (a host proc macro is not rebuilt when only
# the wasm32 flags change). Output file names are unchanged, so cargo finds its outputs. Arguments that only shape
# diagnostics are left out. A call without `-C metadata` (cargo's probes) passes through.
# TPDF_RUSTC_WRAPPER, when set, is the caller's own wrapper and runs the compiler.
set -eu
rustc=$1
shift
# macOS strips DYLD_* variables when a SIP-protected program such as /bin/sh starts, so rustup's
# DYLD_FALLBACK_LIBRARY_PATH (the toolchain's lib, where rust-lld finds libLLVM.dylib) does not reach rustc through
# this script. Restore the toolchain part, as rustup sets it.
if [ "$(uname -s)" = Darwin ] && [ -d "$(dirname -- "$rustc")/../lib" ]; then
  DYLD_FALLBACK_LIBRARY_PATH="$(CDPATH= cd -- "$(dirname -- "$rustc")/../lib" && pwd -P)${DYLD_FALLBACK_LIBRARY_PATH:+:$DYLD_FALLBACK_LIBRARY_PATH}"
  export DYLD_FALLBACK_LIBRARY_PATH
fi
run() { if [ -n "${TPDF_RUSTC_WRAPPER:-}" ]; then exec "$TPDF_RUSTC_WRAPPER" "$rustc" "$@"; else exec "$rustc" "$@"; fi; }
case " $* " in
  *" metadata="* | *" -Cmetadata="*) ;;
  *) run "$@" ;;
esac
: "${CARGO_TARGET_DIR:?}" "${TPDF_ROOT:?}" "${TPDF_CARGO_HOME:?}" "${TPDF_RUSTUP_HOME:?}"
if command -v sha256sum >/dev/null 2>&1; then SHA256='sha256sum'; else SHA256='shasum -a 256'; fi
literal() { printf '%s' "$1" | sed 's/[][\.*^$|/]/\\&/g'; }

# Pass 1: the arguments that identify the crate, one per line, without cargo's hashes and diagnostics options.
norm=
previous=
for arg in "$@"; do
  case "$previous$arg" in
    -Cmetadata=* | -Cextra-filename=*) ;;
    *) case "$arg" in
         -Cmetadata=* | -Cextra-filename=* | --diagnostic-width=* | --error-format=* | --json=* | --color=* | -Cincremental=*) ;;
         -C) ;;
         *) norm="$norm$arg
" ;;
       esac ;;
  esac
  if [ "$arg" = -C ]; then previous=-C; else previous=; fi
done
# The roots, longest first in effect: the target folder may sit inside the checkout.
script="s|$(literal "$CARGO_TARGET_DIR")|/target|g
s|$(literal "$TPDF_ROOT")|/tpdf|g
s|$(literal "$TPDF_CARGO_HOME")|/cargo|g
s|$(literal "$TPDF_RUSTUP_HOME")|/rustup|g
s|-[0-9a-f]\{16\}\.rlib|.rlib|g
s|-[0-9a-f]\{16\}\.rmeta|.rmeta|g
s|-[0-9a-f]\{16\}\.dylib|.dylib|g
s|-[0-9a-f]\{16\}\.so|.so|g
s|-[0-9a-f]\{16\}\.dll|.dll|g"
stable=$(printf '%s\n%s\n%s' "${CARGO_PKG_NAME:-}" "${CARGO_PKG_VERSION:-}" "$norm" | sed "$script" | $SHA256 | cut -c1-16)

# Pass 2: the same arguments with the new metadata.
count=$#
index=0
previous=
while [ "$index" -lt "$count" ]; do
  arg=$1
  shift
  case "$previous$arg" in -Cmetadata=*) arg="metadata=$stable" ;; esac
  case "$arg" in -Cmetadata=*) arg="-Cmetadata=$stable" ;; esac
  if [ "$arg" = -C ]; then previous=-C; else previous=; fi
  set -- "$@" "$arg"
  index=$((index + 1))
done
run "$@"
