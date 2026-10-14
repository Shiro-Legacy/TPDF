#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
set -eu
if [ "$#" -gt 1 ]; then echo "usage: $0 [reference-build-directory]" >&2; exit 1; fi
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
build_dir=${1:-"$script_dir/../reference/build"}
build_dir=$(CDPATH= cd -- "$build_dir" && pwd)
source_dir="$build_dir/mupdf-1.28.1-source"
(cd "$source_dir" && shasum -a 256 -c "$script_dir/../reference/image-native-pins.sha256")
for name in decode page inventory; do
 "${CC:-cc}" -std=c11 -Wall -Wextra -Werror -O2 -I "$source_dir/include" "$script_dir/mupdf-image-$name-oracle.c" "$source_dir/build/release/libmupdf.a" "$source_dir/build/release/libmupdf-third.a" -lm -lpthread -o "$build_dir/mupdf-image-$name-oracle"
done
