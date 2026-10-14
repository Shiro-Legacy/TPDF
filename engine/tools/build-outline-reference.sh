#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Link only the verified bundled FreeType from the pinned native reference build.
set -eu
if [ "$#" -gt 1 ]; then
    echo "usage: $0 [reference-build-directory]" >&2
    exit 1
fi
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
build_dir=${1:-"$script_dir/../reference/build"}
build_dir=$(CDPATH= cd -- "$build_dir" && pwd)
source_dir="$build_dir/mupdf-1.28.1-source"
check() {
    actual=$(shasum -a 256 "$source_dir/$1" | awk '{print $1}')
    if [ "$actual" != "$2" ]; then
        echo "outline reference source mismatch: $1" >&2
        exit 1
    fi
}
check thirdparty/freetype/src/truetype/ttgload.c 627e20092871f554c7eaf719e56dac72497d2d9a1a91b5919ac78bcf74d84deb
check thirdparty/freetype/src/base/ftoutln.c e9a31e1a88398615df585a009632c5eebbc456da0e09b6f6ad6ac2965c1bc3b3
check thirdparty/freetype/src/base/ftobjs.c e5558a906bfef754843740c55a1110759f23c737303d092ba6fefdce2ba4bb9f
check thirdparty/freetype/include/freetype/freetype.h fd45e7ee01c8f0f2419186d3fc9dd58e25cd866ef0c49f0b347c2d141665a364
check thirdparty/freetype/include/freetype/ftoutln.h db0585ea338debdb9b721adc5f6d138ec7f88df30ff44cf6fb3cac79a70982f8
"${CC:-cc}" -std=c11 -Wall -Wextra -Werror -O2 \
    -I "$source_dir/thirdparty/freetype/include" "$script_dir/mupdf-outline-oracle.c" \
    "$source_dir/build/release/libmupdf-third.a" \
    -lm -lpthread -o "$build_dir/mupdf-outline-oracle"
printf '%s\n' "Outline oracle: $build_dir/mupdf-outline-oracle"
