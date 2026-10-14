#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
set -eu
if [ "$#" -gt 1 ]; then echo 'usage: build-builtin-metrics-reference.sh [reference-build-directory]' >&2; exit 1; fi
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
build_dir=${1:-"$script_dir/../reference/build"}
build_dir=$(CDPATH= cd -- "$build_dir" && pwd)
source_dir="$build_dir/mupdf-1.28.1-source"
for entry in \
 'source/fitz/noto.c 98e48a54e2a42e66fa7aaf34d415497fb10b65006cdb295e69fb258bf222588f' \
 'source/fitz/font-table.h c5981e45dff1a226427079f5401156f746aa0a4a5287fe79797edb393f81ba73' \
 'source/fitz/font.c 7fe9aa150fd9ac515e769f2213570b9e696a5650bdb7cd2a62a2bef35a48b6e6' \
 'resources/fonts/urw/NimbusSans-Regular.cff 366720e054067fa49d030f58005c739fc787dcc9727a9327a28767777791d706' \
 'resources/fonts/urw/NimbusRoman-Regular.cff eb609a306b81357853ba91c2a3c47f3bba04f3a8cfdeedc733b1aa4af5d03d4c'; do
 set -- $entry
 actual=$(shasum -a 256 "$source_dir/$1" | awk '{print $1}')
 if [ "$actual" != "$2" ]; then echo "pinned source mismatch: $1" >&2; exit 1; fi
done
"${CC:-cc}" -std=c11 -Wall -Wextra -Werror -O2 \
 -I "$source_dir/include" -I "$source_dir/thirdparty/freetype/include" \
 "$script_dir/mupdf-builtin-metrics-oracle.c" \
 "$source_dir/build/release/libmupdf.a" "$source_dir/build/release/libmupdf-third.a" \
 -lm -lpthread -o "$build_dir/mupdf-builtin-metrics-oracle"
printf '%s\n' "Builtin metrics oracle: $build_dir/mupdf-builtin-metrics-oracle"
