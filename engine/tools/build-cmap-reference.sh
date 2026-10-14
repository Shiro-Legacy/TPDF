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
actual=$(shasum -a 256 "$source_dir/source/pdf/pdf-cmap.c" | awk '{print $1}')
parse_actual=$(shasum -a 256 "$source_dir/source/pdf/pdf-cmap-parse.c" | awk '{print $1}')
if [ "$actual" != dbae2bd8d9d34bc56ad7d53fcb3d2d8fd04364023d139d10984f207336b20d2e ] ||
   [ "$parse_actual" != f6023ee51112b849d25f64f1b4df4e7ed52a95eb6282cc25caac0f928078db87 ]; then
    echo "reference source mismatch; use build-reference.sh with the pinned archive" >&2
    exit 1
fi
"${CC:-cc}" -std=c11 -Wall -Wextra -Werror -O2 \
    -I "$source_dir/include" "$script_dir/mupdf-cmap-oracle.c" \
    "$source_dir/build/release/libmupdf.a" \
    "$source_dir/build/release/libmupdf-third.a" \
    -lm -lpthread -o "$build_dir/mupdf-cmap-oracle"
printf '%s\n' "CMap oracle: $build_dir/mupdf-cmap-oracle"
