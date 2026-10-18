#!/usr/bin/env python3
"""Regenerate the partial stubs (workstreams.txt `partial`) from HEAD, and
stubs.base for every class A path.

Each partial stub is HEAD's file with its derived items cut out by code
anchors and replaced by the fragments in stubs/parts/. Run it after HEAD
changes a partial file (make-handoff.sh reports STALE), then review the diff:

    python3 engine/tools/cleanroom/regen-stubs.py
"""
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
STUBS = os.path.join(HERE, "stubs")
PARTS = os.path.join(STUBS, "parts")
REPO = subprocess.run(["git", "-C", HERE, "rev-parse", "--show-toplevel"],
                      capture_output=True, text=True, check=True).stdout.strip()


def head(path):
    return subprocess.run(["git", "-C", REPO, "show", f"HEAD:engine/{path}"],
                          capture_output=True, text=True, check=True).stdout


def part(name):
    return open(os.path.join(PARTS, name)).read()


def find(lines, pattern, start=0):
    rx = re.compile(pattern)
    for i in range(start, len(lines)):
        if rx.search(lines[i]):
            return i
    sys.exit(f"anchor not found: {pattern}")


def item_start(lines, i):
    """Back over the doc comments and attributes of the item at line i."""
    while i > 0 and re.match(r"\s*(///|#\[)", lines[i - 1]):
        i -= 1
    return i


STRING = re.compile(r'b?"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])\'')


def item_end(lines, i):
    """One past the last line of the item (or block) starting at line i."""
    depth, opened = 0, False
    for j in range(i, len(lines)):
        code = STRING.sub("", lines[j].split("//")[0])
        for c in code:
            if c in "{([":
                depth += 1
                opened = True
            elif c in "})]":
                depth -= 1
        if opened and depth <= 0:
            return j + 1
        if not opened and code.rstrip().endswith(";"):
            return j + 1
    sys.exit(f"no end for the item at line {i + 1}")


def cut_item(lines, pattern, replacement=None):
    i = find(lines, pattern)
    a, b = item_start(lines, i), item_end(lines, i)
    # A blank line after the item goes with it.
    if b < len(lines) and not lines[b].strip():
        b += 1
    lines[a:b] = replacement if replacement is not None else []
    return a


def drop_names(lines, start_pattern, names):
    """Remove `names` from the use declaration starting at start_pattern."""
    i = find(lines, start_pattern)
    j = item_end(lines, i)
    text = "".join(lines[i:j])
    for name in names:
        text, n = re.subn(rf"\b{re.escape(name)},\s*|,\s*\b{re.escape(name)}\b(?=\s*[}};])", "", text)
        if n == 0:
            text, n = re.subn(rf"^use [^;]*\b{re.escape(name)};\n", "", text, flags=re.M)
        if n == 0:
            sys.exit(f"import {name} not found")
    lines[i:j] = [text] if text else []


def object_rs(lines):
    a = item_start(lines, find(lines, r"^pub\(crate\) const SORT_AFTER"))
    lines[a:] = [part("object.rs.tail")]
    drop_names(lines, r"^use std::collections::", ["btree_map"])
    i = find(lines, r"^use std::sync::Arc;")
    lines[i] = "use std::marker::PhantomData;\n"
    return lines


def view_mod_rs(lines):
    a = item_start(lines, find(lines, r"^pub\(crate\) struct PageFrame"))
    lines[a:] = [part("view_mod.rs.tail")]
    drop_names(lines, r"^use hayro_syntax::object::dict::keys::",
               ["CROP_BOX", "MEDIA_BOX", "PARENT", "ROTATE", "USER_UNIT"])
    drop_names(lines, r"^use hayro_syntax::object::\{", ["MaybeRef"])
    for line in (r"^use hayro_syntax::object::Rect;", r"^use hayro_syntax::xref::XRef;"):
        del lines[find(lines, line)]
    return lines


def render_rs(lines):
    i = find(lines, r"^fn pixel_size\(")
    a, b = item_start(lines, i), item_end(lines, i)
    lines[a:b] = [part("render.rs.pixel_size")]
    return lines


def annots_view_rs(lines):
    for pattern in (r"^const NOTE_RECT", r"^const MAX_POINTS", r"^const MAX_QUADS",
                    r"^fn rect_box\(", r"^pub fn synthesized_box\(", r"^pub fn synthesize\("):
        cut_item(lines, pattern)
    # "Bounds on one synthesized appearance" belonged to the two cut constants.
    i = find(lines, r"^pub struct Synthesized")
    at = cut_item(lines, r"^pub struct Synthesized", [part("annots_view.rs.items")])
    tests = find(lines, r"^mod tests \{")
    lines[item_start(lines, tests):item_end(lines, tests)] = []
    while lines and not lines[-1].strip():
        lines.pop()
    drop_names(lines, r"^use super::appearance::", ["Mark", "Style"])
    drop_names(lines, r"^use super::\{Color", ["Color", "Point"])
    del i, at
    return lines


def images_rs(lines):
    i = find(lines, r"^use crate::object::")
    lines[i + 1:i + 1] = ["\n", '#[path = "resolution.rs"]\n', "mod resolution;\n"]
    # PNG: the pHYs resolution.
    i = find(lines, r"let dpi = match info\.pixel_dims")
    a = i
    while lines[a - 1].strip().startswith("//"):
        a -= 1
    lines[a:item_end(lines, i)] = ['        let dpi: u32 = todo!("clean-room: spec 08");\n']
    # JPEG: the resolution fields and their precedence.
    lines[:] = [l for l in lines if not re.match(r"\s+(exif|app13|jfif): Option<", l)]
    i = find(lines, r"let \(exif_dpi, orientation\) = ")
    j = find(lines, r"let dpi = match \(", i)
    lines[i:item_end(lines, j)] = ['    let (dpi, orientation): (u32, u8) = todo!("clean-room: spec 08");\n']
    # The APP0, APP1 and APP13 readers in the marker loop.
    for pattern in (r'^\s+0xE0 if segment\.starts_with\(b"JFIF', r'^\s+0xE1 if segment\.starts_with\(b"Exif'):
        i = find(lines, pattern)
        del lines[i:item_end(lines, i)]
    del lines[find(lines, r"^\s+0xED if info\.app13")]
    cut_item(lines, r"^fn exif\(")
    cut_item(lines, r"^fn photoshop\(")
    # Unit tests of the cut readers; the ICC half of one stays.
    cut_item(lines, r"fn exif_reads_resolution")
    i = find(lines, r"fn photoshop_resolution_and_icc_chunks")
    a = item_start(lines, i)
    b = item_end(lines, i)
    if b < len(lines) and not lines[b].strip():
        b += 1
    lines[a:b] = [part("images.rs.icc_test")]
    return lines


PARTIAL = {
    "src/object.rs": object_rs,
    "src/view/mod.rs": view_mod_rs,
    "src/view/render.rs": render_rs,
    "src/app/annots/view.rs": annots_view_rs,
    "src/app/ops/pages/images.rs": images_rs,
}


def rustfmt(text):
    done = subprocess.run(["rustfmt", "--edition", "2024", "--emit", "stdout"],
                          input=text, capture_output=True, text=True)
    if done.returncode:
        sys.exit(done.stderr)
    return done.stdout


def main():
    for path, transform in PARTIAL.items():
        lines = head(path).splitlines(keepends=True)
        text = rustfmt("".join(transform(lines)))
        with open(os.path.join(STUBS, path), "w") as f:
            f.write(text)
        print("regenerated", path)
    rev = subprocess.run(["git", "-C", REPO, "rev-parse", "--short", "HEAD"],
                         capture_output=True, text=True, check=True).stdout.strip()
    out = ["# HEAD blob of each class A file when its stub was made (make-handoff.sh checks it).",
           f"# Commit: {rev}. Partial stubs: regen-stubs.py."]
    for line in open(os.path.join(HERE, "workstreams.txt")):
        f = line.split()
        if len(f) == 3 and re.fullmatch(r"0[1-8]", f[0]):
            blob = subprocess.run(["git", "-C", REPO, "rev-parse", "-q", "--verify", f"HEAD:engine/{f[1]}"],
                                  capture_output=True, text=True).stdout.strip() or "none"
            out.append(f"{blob} {f[1]}")
    with open(os.path.join(HERE, "stubs.base"), "w") as fh:
        fh.write("\n".join(out) + "\n")
    print("wrote stubs.base")


if __name__ == "__main__":
    main()
