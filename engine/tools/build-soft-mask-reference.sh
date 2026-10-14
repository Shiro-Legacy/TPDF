#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
set -eu
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
build_dir=${1:-/private/tmp/tandem-mupdf-reference}
sh "$script_dir/build-soft-mask-trace-reference.sh" "$build_dir"
source_dir="$build_dir/mupdf-1.28.1-source"
"${CC:-cc}" -std=c11 -Wall -Wextra -Werror -O2 -I "$source_dir/include" \
 "$script_dir/mupdf-soft-mask-oracle.c" "$source_dir/build/release/libmupdf.a" \
 "$source_dir/build/release/libmupdf-third.a" -lm -lpthread -o "$build_dir/mupdf-soft-mask-oracle"
