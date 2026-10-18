#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generates the standard-font metrics in engine/src/app/annots/fonts.rs
(spec 06 section 5.3, owner decisions A4 and A8) from Adobe's Core 14 AFM
files only: engine/generate/data/adobe-core14-afm/, with Adobe's
MustRead.html, whose notice goes into the generated data.

For each of the 12 non-symbolic faces it keeps, per glyph name, the
advance (WX) and the vertical extent of the glyph's bounding box (the lly
and ury of B). Nothing else of the AFM files is kept. The glyph names are
the union of the faces' names, sorted bytewise; the WinAnsi code of each
name comes from the engine's Annex D table at run time, not from here.

The data is written between the two marker lines of fonts.rs, which must
exist; the rest of that file is left as it is.

Run from anywhere: python3 -I engine/generate/annots_fonts.py
"""
import hashlib
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
AFM = os.path.join(HERE, "data", "adobe-core14-afm")
ENGINE = os.path.dirname(HERE)
OUT = os.path.join(ENGINE, "src", "app", "annots", "fonts.rs")
BEGIN = "// @generated begin: engine/generate/annots_fonts.py (do not edit)"
END = "// @generated end"

# Base14 order (fonts.rs).
FACES = [
    "Helvetica",
    "Helvetica-Bold",
    "Helvetica-Oblique",
    "Helvetica-BoldOblique",
    "Times-Roman",
    "Times-Bold",
    "Times-Italic",
    "Times-BoldItalic",
    "Courier",
    "Courier-Bold",
    "Courier-Oblique",
    "Courier-BoldOblique",
]

# Spec 06 section 5.3 sanity values (from the public AFMs).
SANITY = [
    ("Helvetica", "space", 278),
    ("Helvetica", "A", 667),
    ("Helvetica", "W", 944),
    ("Helvetica-Bold", "A", 722),
    ("Times-Roman", "A", 722),
    ("Times-Roman", "a", 444),
    ("Times-Bold", "A", 722),
    ("Times-Bold", "a", 500),
]


def fail(message):
    sys.exit("annots_fonts.py: " + message)


def sha256(path):
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def parse_afm(path):
    """FontName, Notice, and name -> (WX, lly, ury) of the character
    metrics section."""
    with open(path, encoding="latin-1") as f:
        lines = [line.strip() for line in f.read().splitlines()]
    font_name = notice = None
    glyphs = {}
    inside = False
    for line in lines:
        if line.startswith("FontName "):
            font_name = line.split(None, 1)[1]
        elif line.startswith("Notice "):
            notice = line.split(None, 1)[1]
        elif line.startswith("StartCharMetrics"):
            inside = True
        elif line.startswith("EndCharMetrics"):
            inside = False
        elif inside and line:
            fields = {}
            for part in line.split(";"):
                part = part.strip()
                if not part:
                    continue
                key, _, value = part.partition(" ")
                fields[key] = value.strip()
            if "N" not in fields or "WX" not in fields or "B" not in fields:
                fail(f"{path}: odd metrics line {line!r}")
            box = [int(v) for v in fields["B"].split()]
            if len(box) != 4:
                fail(f"{path}: odd bounding box in {line!r}")
            name = fields["N"]
            if name in glyphs:
                fail(f"{path}: {name} twice")
            glyphs[name] = (int(round(float(fields["WX"]))), box[1], box[3])
    if font_name is None or notice is None or not glyphs:
        fail(f"{path}: no FontName, Notice or metrics")
    return font_name, notice, glyphs


def adobe_notice():
    """The licence paragraph of MustRead.html, as Adobe wrote it."""
    with open(os.path.join(AFM, "MustRead.html"), encoding="latin-1") as f:
        html = f.read()
    match = re.search(r"(This file and the 14 PostScript.*?AFM files\.)", html, re.S)
    if not match:
        fail("MustRead.html: no licence paragraph")
    text = re.sub(r"<[^>]*>", "", match.group(1))
    return " ".join(text.split())


def wrap(text, width, prefix):
    out, line = [], ""
    for word in text.split():
        if line and len(prefix) + len(line) + 1 + len(word) > width:
            out.append(prefix + line)
            line = word
        else:
            line = f"{line} {word}" if line else word
    if line:
        out.append(prefix + line)
    return out


def main():
    faces = []
    for face in FACES:
        path = os.path.join(AFM, face + ".afm")
        font_name, notice, glyphs = parse_afm(path)
        if font_name != face:
            fail(f"{face}.afm names {font_name}")
        faces.append((face, sha256(path), notice, glyphs))
    by_face = {face: glyphs for face, _, _, glyphs in faces}
    for face, name, width in SANITY:
        if by_face[face][name][0] != width:
            fail(f"{face} {name}: {by_face[face][name][0]}, not {width}")
    for face in FACES[8:]:
        if any(w != 600 for w, _, _ in by_face[face].values()):
            fail(f"{face}: a glyph not 600 wide")
    names = sorted({name for _, _, _, glyphs in faces for name in glyphs},
                   key=lambda n: n.encode("ascii"))

    out = [BEGIN]
    out.append("//")
    out.append("// Derived from Adobe's Core 14 AFM files (decision A4): of each")
    out.append("// glyph, only the advance (WX) and the lly and ury of its bounding")
    out.append("// box (B) are kept; nothing else of the files is. Files and SHA-256:")
    for face, digest, _, _ in faces:
        out.append(f"//   {face}.afm  {digest}")
    out.append(f"//   MustRead.html  {sha256(os.path.join(AFM, 'MustRead.html'))}")
    out.append("//")
    out.append("// The files' copyright notices:")
    seen = []
    for _, _, notice, _ in faces:
        if notice not in seen:
            seen.append(notice)
            out.extend(wrap(notice, 78, "//   "))
    out.append("//")
    out.append("// Adobe's notice (MustRead.html, which accompanies the AFM files in")
    out.append("// engine/generate/data/adobe-core14-afm/):")
    out.append("//")
    out.extend(wrap(adobe_notice(), 78, "//   "))
    out.append("")
    out.append("/// Every glyph name of the 12 faces, sorted bytewise.")
    out.append("#[rustfmt::skip]")
    out.append(f"static GLYPHS: [&[u8]; {len(names)}] = [")
    line = "   "
    for name in names:
        item = f' b"{name}",'
        if len(line) + len(item) > 99:
            out.append(line)
            line = "   "
        line += item
    out.append(line)
    out.append("];")
    out.append("")
    out.append("/// Per face, in `Base14` order, and per name of `GLYPHS`: the advance")
    out.append("/// (1/1000 em), and the bottom and top of the glyph's bounding box; a")
    out.append("/// face without the glyph has [0, 0, 0].")
    out.append("#[rustfmt::skip]")
    out.append(f"static METRICS: [[[i16; 3]; {len(names)}]; {len(FACES)}] = [")
    for face, _, _, glyphs in faces:
        out.append(f"    // {face}")
        out.append("    [")
        line = "       "
        for name in names:
            w, lo, hi = glyphs.get(name, (0, 0, 0))
            item = f" [{w}, {lo}, {hi}],"
            if len(line) + len(item) > 99:
                out.append(line)
                line = "       "
            line += item
        out.append(line)
        out.append("    ],")
    out.append("];")
    out.append(END)

    with open(OUT, encoding="utf-8") as f:
        source = f.read()
    start = source.find(BEGIN)
    stop = source.find(END)
    if start < 0 or stop < start:
        fail(f"{OUT}: no generated region")
    source = source[:start] + "\n".join(out) + source[stop + len(END):]
    with open(OUT, "w", encoding="utf-8") as f:
        f.write(source)
    print(f"wrote {OUT}: {len(names)} glyphs x {len(FACES)} faces")


if __name__ == "__main__":
    main()
