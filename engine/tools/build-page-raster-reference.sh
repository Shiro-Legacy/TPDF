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
actual=$(shasum -a 256 "$source_dir/source/fitz/draw-device.c" | awk '{print $1}')
if [ "$actual" != fd39760a22f744ee86fb40f27413cc07a76e3e61341886ad40e5b62e6a03e0d7 ]; then
    echo "raster reference source mismatch: source/fitz/draw-device.c" >&2
    exit 1
fi
actual=$(shasum -a 256 "$source_dir/source/fitz/draw-edge.c" | awk '{print $1}')
if [ "$actual" != e31acb81c328af2f4274f39084cfff1b29b12adb8c1edc46deec3a9732f6182a ]; then
    echo "raster reference source mismatch: source/fitz/draw-edge.c" >&2
    exit 1
fi
actual=$(shasum -a 256 "$source_dir/source/fitz/draw-rasterize.c" | awk '{print $1}')
if [ "$actual" != ab5eb96e3c2ab9292a9342bacd078f3ad6154a61d2e8f583d81f76abd982dd1a ]; then
    echo "raster reference source mismatch: source/fitz/draw-rasterize.c" >&2
    exit 1
fi
actual=$(shasum -a 256 "$source_dir/source/fitz/draw-path.c" | awk '{print $1}')
if [ "$actual" != 4a6bd0e4ec8d278d9c75183b5398605afb9f85514aa98805f4355627a246f659 ]; then
    echo "raster reference source mismatch: source/fitz/draw-path.c" >&2
    exit 1
fi
actual=$(shasum -a 256 "$source_dir/source/fitz/draw-paint.c" | awk '{print $1}')
if [ "$actual" != 6978ee443c195fdec5492aaad2a783bb2da290b7b03223a4a25fd2a8de82972b ]; then
    echo "raster reference source mismatch: source/fitz/draw-paint.c" >&2
    exit 1
fi
actual=$(shasum -a 256 "$source_dir/source/fitz/draw-blend.c" | awk '{print $1}')
if [ "$actual" != d5007500b42c62b9009e5ba3cdae8b18da978863a9f58fdb396ec29fd0dc3c7a ]; then
    echo "raster reference source mismatch: source/fitz/draw-blend.c" >&2
    exit 1
fi
actual=$(shasum -a 256 "$source_dir/source/pdf/pdf-op-run.c" | awk '{print $1}')
if [ "$actual" != a269ed581d47fce07ed62f6b0a8f687669d63d7475a62b06516ddadc0bbfff14 ]; then
    echo "raster reference source mismatch: source/pdf/pdf-op-run.c" >&2
    exit 1
fi
actual=$(shasum -a 256 "$source_dir/source/pdf/pdf-interpret.c" | awk '{print $1}')
if [ "$actual" != dbff6c518bc9c1c9f17bac161538548a95d598d29ab7a4d0e8a130b274b18883 ]; then
    echo "raster reference source mismatch: source/pdf/pdf-interpret.c" >&2
    exit 1
fi
"${CC:-cc}" -std=c11 -Wall -Wextra -Werror -O2 \
    -I "$source_dir/include" "$script_dir/mupdf-page-raster-oracle.c" \
    "$source_dir/build/release/libmupdf.a" \
    "$source_dir/build/release/libmupdf-third.a" \
    -lm -lpthread -o "$build_dir/mupdf-page-raster-oracle"
printf '%s\n' "Page geometry raster oracle: $build_dir/mupdf-page-raster-oracle"
