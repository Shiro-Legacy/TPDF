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
    echo "program oracle source mismatch: source/pdf/pdf-op-run.c" >&2
    exit 1
fi
actual=$(shasum -a 256 "$source_dir/source/pdf/pdf-interpret.c" | awk '{print $1}')
if [ "$actual" != dbff6c518bc9c1c9f17bac161538548a95d598d29ab7a4d0e8a130b274b18883 ]; then
    echo "program oracle source mismatch: source/pdf/pdf-interpret.c" >&2
    exit 1
fi
actual=$(shasum -a 256 "$source_dir/source/pdf/pdf-font.c" | awk '{print $1}')
if [ "$actual" != c0c29c2826883e47191133f66b7d9fb769116f9923419606ac4f7136f54feca5 ]; then
    echo "program oracle source mismatch: source/pdf/pdf-font.c" >&2
    exit 1
fi
actual=$(shasum -a 256 "$source_dir/include/mupdf/fitz/text.h" | awk '{print $1}')
if [ "$actual" != 174b56b21d13fe18246427dd68d712b0a81c254fbc17cd48154d023a4c1195c9 ]; then
    echo "program oracle source mismatch: include/mupdf/fitz/text.h" >&2
    exit 1
fi
actual=$(shasum -a 256 "$source_dir/source/fitz/path.c" | awk '{print $1}')
if [ "$actual" != 598278070de5bf44237e75298fdfab7b3a94371107e6acbfc5bebf7013294347 ]; then
    echo "program oracle source mismatch: source/fitz/path.c" >&2
    exit 1
fi
actual=$(shasum -a 256 "$source_dir/include/mupdf/fitz/path.h" | awk '{print $1}')
if [ "$actual" != 1644bfa7df3549cf8028b3eba7f93d3473f18fa3116bd8190855c56c64a61bc5 ]; then
    echo "program oracle source mismatch: include/mupdf/fitz/path.h" >&2
    exit 1
fi
actual=$(shasum -a 256 "$source_dir/include/mupdf/fitz/device.h" | awk '{print $1}')
if [ "$actual" != 6b12f488c9b69eee072d64aed9c5fb956ff39e5924117c077a423ada4d3f7bae ]; then
    echo "program oracle source mismatch: include/mupdf/fitz/device.h" >&2
    exit 1
fi
actual=$(shasum -a 256 "$source_dir/source/pdf/pdf-page.c" | awk '{print $1}')
if [ "$actual" != 3d8b0b0852d35449f0d146ad1dd62727d39225c1d3a28681b5ef5d6939ae4258 ]; then
    echo "page oracle source mismatch: source/pdf/pdf-page.c" >&2
    exit 1
fi
actual=$(shasum -a 256 "$source_dir/source/pdf/pdf-stream.c" | awk '{print $1}')
if [ "$actual" != ff97f1366fffe1c69f291799f6d8793d84ab7e602a20f09462aa0beb9cee2ab9 ]; then
    echo "page oracle source mismatch: source/pdf/pdf-stream.c" >&2
    exit 1
fi
actual=$(shasum -a 256 "$source_dir/source/fitz/font.c" | awk '{print $1}')
if [ "$actual" != 7fe9aa150fd9ac515e769f2213570b9e696a5650bdb7cd2a62a2bef35a48b6e6 ]; then
    echo "glyph reference source mismatch: source/fitz/font.c" >&2
    exit 1
fi
actual=$(shasum -a 256 "$source_dir/source/fitz/device.c" | awk '{print $1}')
if [ "$actual" != 4a03f562cb9b862cec42e2887d4e12606e90d7168b7705c27d834522bcc56264 ]; then
    echo "glyph reference source mismatch: source/fitz/device.c" >&2
    exit 1
fi
actual=$(shasum -a 256 "$source_dir/thirdparty/freetype/src/truetype/ttgload.c" | awk '{print $1}')
if [ "$actual" != 627e20092871f554c7eaf719e56dac72497d2d9a1a91b5919ac78bcf74d84deb ]; then
    echo "glyph reference source mismatch: thirdparty/freetype/src/truetype/ttgload.c" >&2
    exit 1
fi
actual=$(shasum -a 256 "$source_dir/thirdparty/freetype/src/base/ftoutln.c" | awk '{print $1}')
if [ "$actual" != e9a31e1a88398615df585a009632c5eebbc456da0e09b6f6ad6ac2965c1bc3b3 ]; then
    echo "glyph reference source mismatch: thirdparty/freetype/src/base/ftoutln.c" >&2
    exit 1
fi
actual=$(shasum -a 256 "$source_dir/thirdparty/freetype/src/base/ftobjs.c" | awk '{print $1}')
if [ "$actual" != e5558a906bfef754843740c55a1110759f23c737303d092ba6fefdce2ba4bb9f ]; then
    echo "glyph reference source mismatch: thirdparty/freetype/src/base/ftobjs.c" >&2
    exit 1
fi
actual=$(shasum -a 256 "$source_dir/thirdparty/freetype/src/sfnt/ttcmap.c" | awk '{print $1}')
if [ "$actual" != f7288c09bf7b64381ebc794df9236d99f2d378043d89ab477b6e5be697f8a00a ]; then
    echo "glyph reference source mismatch: thirdparty/freetype/src/sfnt/ttcmap.c" >&2
    exit 1
fi
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
for entry in \
 'source/pdf/pdf-xobject.c 4975e6feb3ec58c6c2794079f4ed2fe741f2322eff30c5b82d7505fe75a6ac1b' \
 'source/fitz/geometry.c 1af3ea90e5e49a281a69fa4162f7c1c3365117eaa51e187be8f21c72d63e52ae'; do
 set -- $entry
 actual=$(shasum -a 256 "$source_dir/$1" | awk '{print $1}')
 if [ "$actual" != "$2" ]; then echo "pinned Form source mismatch: $1" >&2; exit 1; fi
done
"${CC:-cc}" -std=c11 -Wall -Wextra -Werror -O2 \
    -I "$source_dir/include" -I "$source_dir/thirdparty/freetype/include" "$script_dir/mupdf-form-page-oracle.c" \
    "$source_dir/build/release/libmupdf.a" \
    "$source_dir/build/release/libmupdf-third.a" \
    -lm -lpthread -o "$build_dir/mupdf-form-page-oracle"
printf '%s\n' "Form-page unhinted/hinted reference: $build_dir/mupdf-form-page-oracle"
