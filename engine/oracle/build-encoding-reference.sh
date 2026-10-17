#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
set -eu
if [ "$#" -gt 1 ]; then echo "usage: $0 [reference-build-directory]" >&2; exit 1; fi
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
build_dir=${1:-"$script_dir/../reference/build"}
build_dir=$(CDPATH= cd -- "$build_dir" && pwd)
source_dir="$build_dir/mupdf-1.28.1-source"
actual=$(shasum -a 256 "$source_dir/source/fitz/encodings.h" | awk '{print $1}')
if [ "$actual" != 224122c5581a91254a22d3a009c50322d39fbbac5f1b2c1c5464cba8528839de ]; then
 echo 'reference source mismatch; use build-reference.sh pinned archive' >&2; exit 1
fi
for entry in \
 'encodings.c db046231a23e4470c89e31850cff8670a1ca95def64c71d0d4f4c416622fe4a6' \
 'glyphlist.h 16fe0427189e16ff852c02984a49dba184c0db51756b8ecef01524ad6efb6002'; do
 set -- $entry
 actual=$(shasum -a 256 "$source_dir/source/fitz/$1" | awk '{print $1}')
 if [ "$actual" != "$2" ]; then echo 'reference glyph source mismatch' >&2; exit 1; fi
done
"${CC:-cc}" -std=c11 -Wall -Wextra -Werror -O2 \
 -I "$source_dir/include" "$script_dir/mupdf-encoding-oracle.c" \
 "$source_dir/build/release/libmupdf.a" "$source_dir/build/release/libmupdf-third.a" \
 -lm -lpthread -o "$build_dir/mupdf-encoding-oracle"
printf '%s\n' "Encoding oracle: $build_dir/mupdf-encoding-oracle"
