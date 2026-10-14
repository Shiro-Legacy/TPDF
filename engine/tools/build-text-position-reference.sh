#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Reuse the verified static build produced by build-reference.sh.
set -eu
if [ "$#" -gt 1 ]; then
    echo "usage: $0 [reference-build-directory]" >&2
    exit 1
fi
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
build_dir=${1:-"$script_dir/../reference/build"}
build_dir=$(CDPATH= cd -- "$build_dir" && pwd)
source_dir="$build_dir/mupdf-1.28.1-source"
actual=$(shasum -a 256 "$source_dir/source/pdf/pdf-lex.c" | awk '{print $1}')
if [ "$actual" != 401002fee1c6de41e6c40c783f023351a2e34c6e94c0af9de230e283a8f7ecf1 ]; then
    echo "reference source mismatch; use build-reference.sh with the pinned archive" >&2
    exit 1
fi
actual=$(shasum -a 256 "$source_dir/source/pdf/pdf-op-run.c" | awk '{print $1}')
if [ "$actual" != a269ed581d47fce07ed62f6b0a8f687669d63d7475a62b06516ddadc0bbfff14 ]; then
    echo "text oracle source mismatch: source/pdf/pdf-op-run.c" >&2
    exit 1
fi
actual=$(shasum -a 256 "$source_dir/source/pdf/pdf-interpret.c" | awk '{print $1}')
if [ "$actual" != dbff6c518bc9c1c9f17bac161538548a95d598d29ab7a4d0e8a130b274b18883 ]; then
    echo "text oracle source mismatch: source/pdf/pdf-interpret.c" >&2
    exit 1
fi
actual=$(shasum -a 256 "$source_dir/source/pdf/pdf-font.c" | awk '{print $1}')
if [ "$actual" != c0c29c2826883e47191133f66b7d9fb769116f9923419606ac4f7136f54feca5 ]; then
    echo "text oracle source mismatch: source/pdf/pdf-font.c" >&2
    exit 1
fi
actual=$(shasum -a 256 "$source_dir/include/mupdf/fitz/text.h" | awk '{print $1}')
if [ "$actual" != 174b56b21d13fe18246427dd68d712b0a81c254fbc17cd48154d023a4c1195c9 ]; then
    echo "text oracle source mismatch: include/mupdf/fitz/text.h" >&2
    exit 1
fi
"${CC:-cc}" -std=c11 -Wall -Wextra -Werror -O2 \
    -I "$source_dir/include" "$script_dir/mupdf-text-position-oracle.c" \
    "$source_dir/build/release/libmupdf.a" \
    "$source_dir/build/release/libmupdf-third.a" \
    -lm -lpthread -o "$build_dir/mupdf-text-position-oracle"
printf '%s\n' "Text-position oracle: $build_dir/mupdf-text-position-oracle"
