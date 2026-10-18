#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Offline build: provide the official, pinned source archive locally.
set -eu
if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
    echo "usage: $0 /absolute/path/mupdf-1.28.1-source.tar.gz [build-directory]" >&2
    exit 1
fi
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
archive=$1
build_dir=${2:-"$script_dir/../reference/build"}
expected=dc94c60b2537e2ac9a2d379dd3801545f84a3a302d15c9da358362a1270707c3
actual=$(shasum -a 256 "$archive" | awk '{print $1}')
if [ "$actual" != "$expected" ]; then
    echo "MuPDF source checksum mismatch; refusing build" >&2
    exit 1
fi
mkdir -p "$build_dir"
build_dir=$(CDPATH= cd -- "$build_dir" && pwd)
source_dir="$build_dir/mupdf-1.28.1-source"
# Re-extract verified sources even when incremental build objects exist.
tar -xzf "$archive" -C "$build_dir"
make -C "$source_dir" -j "${JOBS:-4}" build=release \
    HAVE_GLUT=no HAVE_X11=no HAVE_CURL=no HAVE_LIBCRYPTO=no \
    HAVE_TESSERACT=no HAVE_LEPTONICA=no HAVE_ZXINGCPP=no libs
"${CC:-cc}" -std=c11 -Wall -Wextra -Werror -O2 \
    -I "$source_dir/include" "$script_dir/mupdf-lex-oracle.c" \
    "$source_dir/build/release/libmupdf.a" \
    "$source_dir/build/release/libmupdf-third.a" \
    -lm -lpthread -o "$build_dir/mupdf-lex-oracle"
printf '%s\n' "Oracle: $build_dir/mupdf-lex-oracle"
