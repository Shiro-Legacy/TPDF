#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
set -eu
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
build_dir=${1:-"$script_dir/../reference/build"}
build_dir=$(CDPATH= cd -- "$build_dir" && pwd)
source_dir="$build_dir/mupdf-1.28.1-source"
check() {
    actual=$(shasum -a 256 "$source_dir/$1" | awk '{print $1}')
    if [ "$actual" != "$2" ]; then
        echo "glyph reference source mismatch: $1" >&2
        exit 1
    fi
}
check source/pdf/pdf-font.c c0c29c2826883e47191133f66b7d9fb769116f9923419606ac4f7136f54feca5
check source/fitz/font.c 7fe9aa150fd9ac515e769f2213570b9e696a5650bdb7cd2a62a2bef35a48b6e6
check source/fitz/encodings.c db046231a23e4470c89e31850cff8670a1ca95def64c71d0d4f4c416622fe4a6
check source/fitz/encodings.h 224122c5581a91254a22d3a009c50322d39fbbac5f1b2c1c5464cba8528839de
check source/fitz/glyphlist.h 16fe0427189e16ff852c02984a49dba184c0db51756b8ecef01524ad6efb6002
check thirdparty/freetype/src/sfnt/ttcmap.c f7288c09bf7b64381ebc794df9236d99f2d378043d89ab477b6e5be697f8a00a
check thirdparty/freetype/src/sfnt/ttpost.c 9d8b3d635e819e2278d0cab60641b95f6795e0b19877e42b3eb130c8f706180c
check thirdparty/freetype/src/base/ftobjs.c e5558a906bfef754843740c55a1110759f23c737303d092ba6fefdce2ba4bb9f
check thirdparty/freetype/include/freetype/freetype.h fd45e7ee01c8f0f2419186d3fc9dd58e25cd866ef0c49f0b347c2d141665a364
check COPYING 57c8ff33c9c0cfc3ef00e650a1cc910d7ee479a8bc509f6c9209a7c2a11399d6
check thirdparty/freetype/docs/FTL.TXT 5a5ee54c5001bbad1cdc1a57cc3dd4c42199b2da09d39c7ee41fab002d02967f
"${CC:-cc}" -std=c11 -Wall -Wextra -Werror -O2 \
    -I "$source_dir/thirdparty/freetype/include" "$script_dir/mupdf-truetype-cmap-oracle.c" \
    "$source_dir/build/release/libmupdf-third.a" \
    -lm -lpthread -o "$build_dir/mupdf-truetype-cmap-oracle"
"${CC:-cc}" -std=c11 -Wall -Wextra -Werror -O2 \
    -I "$source_dir/include" -I "$source_dir/thirdparty/freetype/include" \
    "$script_dir/mupdf-glyph-inventory-oracle.c" "$source_dir/build/release/libmupdf.a" \
    "$source_dir/build/release/libmupdf-third.a" \
    -lm -lpthread -o "$build_dir/mupdf-glyph-inventory-oracle"
printf '%s\n' "Native cmap/PDF-glyph oracles built in $build_dir"
