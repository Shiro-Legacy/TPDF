#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
set -eu
if [ "$#" -gt 1 ]; then echo 'usage: build-builtin-outlines-reference.sh [reference-build-directory]' >&2; exit 1; fi
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
actual=$(shasum -a 256 "$source_dir/thirdparty/freetype/src/cff/cffgload.c" | awk '{print $1}')
if [ "$actual" != a96b677f84f527a7943a17021f8748b27746ef046b51258daff259b29c61b929 ]; then echo "pinned CFF source mismatch: thirdparty/freetype/src/cff/cffgload.c" >&2; exit 1; fi
actual=$(shasum -a 256 "$source_dir/thirdparty/freetype/src/cff/cffobjs.c" | awk '{print $1}')
if [ "$actual" != 14762bf3254f5b0381f23070ef433580f36528800d1749f1c4899978d3cd80ff ]; then echo "pinned CFF source mismatch: thirdparty/freetype/src/cff/cffobjs.c" >&2; exit 1; fi
actual=$(shasum -a 256 "$source_dir/thirdparty/freetype/src/cff/cffload.c" | awk '{print $1}')
if [ "$actual" != 7affaa2eef1e95e6853442f10d1c2103fe10eda043b0bba195d877b0a8a67ba5 ]; then echo "pinned CFF source mismatch: thirdparty/freetype/src/cff/cffload.c" >&2; exit 1; fi
actual=$(shasum -a 256 "$source_dir/thirdparty/freetype/src/base/ftoutln.c" | awk '{print $1}')
if [ "$actual" != e9a31e1a88398615df585a009632c5eebbc456da0e09b6f6ad6ac2965c1bc3b3 ]; then echo "pinned CFF source mismatch: thirdparty/freetype/src/base/ftoutln.c" >&2; exit 1; fi
actual=$(shasum -a 256 "$source_dir/thirdparty/freetype/src/base/ftobjs.c" | awk '{print $1}')
if [ "$actual" != e5558a906bfef754843740c55a1110759f23c737303d092ba6fefdce2ba4bb9f ]; then echo "pinned CFF source mismatch: thirdparty/freetype/src/base/ftobjs.c" >&2; exit 1; fi
"${CC:-cc}" -std=c11 -Wall -Wextra -Werror -O2 \
 -I "$source_dir/include" -I "$source_dir/thirdparty/freetype/include" \
 "$script_dir/mupdf-builtin-outlines-oracle.c" \
 "$source_dir/build/release/libmupdf.a" "$source_dir/build/release/libmupdf-third.a" \
 -lm -lpthread -o "$build_dir/mupdf-builtin-outlines-oracle"
printf '%s\n' "Builtin outline oracle: $build_dir/mupdf-builtin-outlines-oracle"
