#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Regenerates the glyph-name and Unicode tables of spec 05 from public
sources:

    src/simple_encoding_data.rs   the four named simple encodings (Annex D)
                                  and the Adobe Glyph List value of each of
                                  their glyph names
    src/view/text_unicode_data.rs the Adobe Glyph List, the ITC Zapf Dingbats
                                  list, the Symbol and ZapfDingbats built-in
                                  encodings, the standard Macintosh glyph
                                  order, presentation-form decompositions,
                                  Bidi_Mirroring_Glyph, and the unassigned
                                  code points whose default Bidi_Class is BN
                                  (the unicode-bidi crate reads them as L)

Usage (from anywhere; paths default to this script's neighbours):

    python3 -I engine/generate/generate_data.py [--data DIR] [--engine DIR]

Inputs, each checked against the SHA-256 pinned below:

    data/glyphlist.txt, data/zapfdingbats.txt
        https://github.com/adobe-type-tools/agl-aglfn (BSD-3-Clause)
    data/symbol.txt, data/zdingbat.txt
        https://unicode.org/Public/MAPPINGS/VENDORS/ADOBE/ (notice in file)
    data/UnicodeData.txt, data/BidiMirroring.txt
        https://www.unicode.org/Public/16.0.0/ucd/ (Unicode License v3)
    data/DerivedBidiClass.txt
        https://www.unicode.org/Public/16.0.0/ucd/extracted/ (Unicode License v3)
    tables/annex-d-latin.txt, tables/annex-d-expert.txt
        typed from ISO 32000-2:2020 Annex D.2 and D.4
    tables/mac-glyph-order.txt
        typed from Apple's TrueType Reference Manual, 'post' table

Optional cross-checks, run when the files are present (they are not
committed; fetch them from unicode.org Public/MAPPINGS/VENDORS/):

    data/crosscheck/CP1252.TXT  MICSFT/WINDOWS/CP1252.TXT  (WinAnsi)
    data/crosscheck/ROMAN.TXT   APPLE/ROMAN.TXT            (MacRoman)
    data/crosscheck/stdenc.txt  ADOBE/stdenc.txt           (Standard)

The output is deterministic: a re-run gives byte-identical files.
"""

import argparse
import hashlib
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))

SOURCES = {
    "glyphlist.txt": (
        "a3b2f61ced9f3644cc0d4ecde5c59df34ca286c689d9484a43a710a81c466789",
        "Adobe Glyph List 2.0 (2002-09-20), github.com/adobe-type-tools/agl-aglfn"
        " commit 4036a9ca80a62f64f9de4f7321a9a045ad0ecfd6, BSD-3-Clause",
    ),
    "zapfdingbats.txt": (
        "f6394e3cb8a447e84a1dad75d4baaf2aa7f45dc104faf369f4720e1a774ef2dc",
        "ITC Zapf Dingbats Glyph List 2.0, same repository, BSD-3-Clause",
    ),
    "symbol.txt": (
        "deb78ca840a429311939b9d165890873f71fb23ef223ceeb144a6c6d641a7e52",
        "Adobe Symbol Encoding to Unicode 1.0 (2011-07-12),"
        " unicode.org/Public/MAPPINGS/VENDORS/ADOBE/symbol.txt, Unicode, Inc. notice",
    ),
    "zdingbat.txt": (
        "2d8128a7280cdd47d93272f13c580d7f31fb34586ff65eaf29c7457c224974c0",
        "Adobe Zapf Dingbats Encoding to Unicode 1.0 (2011-07-12),"
        " unicode.org/Public/MAPPINGS/VENDORS/ADOBE/zdingbat.txt, Unicode, Inc. notice",
    ),
    "UnicodeData.txt": (
        "ff58e5823bd095166564a006e47d111130813dcf8bf234ef79fa51a870edb48f",
        "Unicode 16.0.0 UnicodeData.txt, unicode.org/Public/16.0.0/ucd/,"
        " Unicode License v3",
    ),
    "BidiMirroring.txt": (
        "d7afdadd1bbd66f5a663ac0e8f7958f18fd9491fc0bc59ec5877cb82db71db7d",
        "Unicode 16.0.0 BidiMirroring.txt, unicode.org/Public/16.0.0/ucd/,"
        " Unicode License v3",
    ),
    "DerivedBidiClass.txt": (
        "71ed943a49c58568d8d92e80ecc2ba2f06e62aee9c8ebb0e6e8bd2c3ed8b180e",
        "Unicode 16.0.0 extracted/DerivedBidiClass.txt,"
        " unicode.org/Public/16.0.0/ucd/extracted/, Unicode License v3",
    ),
}

TYPED = {
    "annex-d-latin.txt": "ISO 32000-2:2020 Annex D.2 (typed)",
    "annex-d-expert.txt": "ISO 32000-2:2020 Annex D.4 (typed)",
    "mac-glyph-order.txt": "Apple TrueType Reference Manual, 'post' table (typed)",
}

CROSSCHECK = {
    "CP1252.TXT": "f607ae328b4dff5e9bfef725f5fff0ae23f38797f8a5b95998a0d2735c0e8fad",
    "ROMAN.TXT": "18e571645be895e9553ed5c842ea8f65f9c5d3c9ccb43e66e0c33a132ed0d721",
    "stdenc.txt": "4bcda13f60f43b79fa403240f3557dc3f8018e80495d61e2b089b2607a372a9d",
}

# Facts the generator checks (spec 05, 5.1 and 4.3).
UNDEFINED_COUNTS = {"Standard": 107, "MacRoman": 48, "WinAnsi": 32, "MacExpert": 91}
REGISTRY_NAMES = 378
PRESENTATION_FORMS = 854


def fail(message):
    sys.exit("generate_data.py: " + message)


def sha256(path):
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def read_pinned(directory, name, digest):
    path = os.path.join(directory, name)
    if not os.path.exists(path):
        fail(f"{path} is missing (see the docstring for where to fetch it)")
    got = sha256(path)
    if got != digest:
        fail(f"{path}: SHA-256 {got}, pinned {digest}")
    with open(path, "rb") as f:
        return f.read().decode("utf-8")


def data_lines(text):
    for line in text.splitlines():
        line = line.split("#", 1)[0].strip() if not line.startswith("#") else ""
        if line:
            yield line


# ------------------------------------------------------------------ sources


def parse_glyph_list(text):
    """name -> tuple of code points, from an AGL-format file."""
    out = {}
    for line in text.splitlines():
        if not line or line.startswith("#"):
            continue
        name, values = line.split(";")
        if name in out:
            fail(f"glyph list names {name} twice")
        out[name] = tuple(int(v, 16) for v in values.split())
    return out


def parse_annex_latin(text):
    tables = {"Standard": [None] * 256, "MacRoman": [None] * 256, "WinAnsi": [None] * 256}
    for line in data_lines(text):
        fields = line.split()
        if len(fields) != 4:
            fail(f"annex-d-latin.txt: bad line {line!r}")
        name = fields[0]
        for table, code in zip(("Standard", "MacRoman", "WinAnsi"), fields[1:]):
            if code == "-":
                continue
            slot = int(code, 8)
            if tables[table][slot] is not None:
                fail(f"{table} {slot:#o} named twice")
            tables[table][slot] = name
    # The table's notes (Annex D.2).
    win = tables["WinAnsi"]
    for slot in (0o177, 0o201, 0o215, 0o217, 0o220, 0o235):
        assert win[slot] is None
        win[slot] = "bullet"
    assert win[0o240] is None and win[0o255] is None
    win[0o240] = "space"
    win[0o255] = "hyphen"
    mac = tables["MacRoman"]
    assert mac[0o312] is None
    mac[0o312] = "space"
    return tables


def parse_annex_expert(text):
    table = [None] * 256
    for line in data_lines(text):
        fields = line.split()
        if len(fields) != 2:
            fail(f"annex-d-expert.txt: bad line {line!r}")
        slot = int(fields[1], 8)
        if table[slot] is not None:
            fail(f"MacExpert {slot:#o} named twice")
        table[slot] = fields[0]
    return table


def parse_mac_order(text):
    names = list(data_lines(text))
    if len(names) != 258 or len(set(names)) != 258:
        fail("mac-glyph-order.txt must hold 258 distinct names")
    return names


def parse_vendor_mapping(text):
    """code -> list of Unicode values, from Adobe's symbol/zdingbat files."""
    out = {}
    for line in text.splitlines():
        if not line or line.startswith("#"):
            continue
        fields = line.split("\t")
        out.setdefault(int(fields[1], 16), []).append(int(fields[0], 16))
    return out


def parse_unicode_data(text):
    """Canonical combining classes, decompositions, and the assigned code
    points."""
    ccc, decomposition, assigned = {}, {}, set()
    first = None
    for line in text.splitlines():
        fields = line.split(";")
        cp = int(fields[0], 16)
        if fields[1].endswith(", First>"):
            first = cp
        elif fields[1].endswith(", Last>"):
            assigned.update(range(first, cp + 1))
        assigned.add(cp)
        ccc[cp] = int(fields[3])
        if fields[5]:
            parts = fields[5].split()
            if parts[0].startswith("<"):
                parts = parts[1:]
            decomposition[cp] = [int(p, 16) for p in parts]
    return ccc, decomposition, assigned


def default_bn_ranges(text, assigned):
    """Ranges of unassigned code points DerivedBidiClass.txt gives BN."""
    points = []
    for line in data_lines(text):
        span, value = (f.strip() for f in line.split(";"))
        if value != "BN":
            continue
        first, _, last = span.partition("..")
        for cp in range(int(first, 16), int(last or first, 16) + 1):
            if cp not in assigned:
                points.append(cp)
    ranges = []
    for cp in sorted(points):
        if ranges and ranges[-1][1] + 1 == cp:
            ranges[-1][1] = cp
        else:
            ranges.append([cp, cp])
    return [tuple(r) for r in ranges]


def nfkd(cp, decomposition, ccc):
    """The full compatibility decomposition of one code point, in canonical
    order (Unicode 3.11)."""

    def expand(c):
        if c in decomposition:
            out = []
            for part in decomposition[c]:
                out.extend(expand(part))
            return out
        return [c]

    chars = expand(cp)
    # Canonical ordering: a stable sort of each run of non-starters.
    out, run = [], []
    for c in chars:
        if ccc.get(c, 0) == 0:
            out.extend(sorted(run, key=lambda x: ccc.get(x, 0)))
            run = []
            out.append(c)
        else:
            run.append(c)
    out.extend(sorted(run, key=lambda x: ccc.get(x, 0)))
    return out


def parse_mirroring(text):
    pairs = []
    for line in data_lines(text):
        source, target = (int(v.strip(), 16) for v in line.split(";"))
        pairs.append((source, target))
    return sorted(pairs)


# ------------------------------------------------------------------ cross-checks


def crosscheck(directory, tables, agl):
    present = [n for n in CROSSCHECK if os.path.exists(os.path.join(directory, n))]
    if not present:
        print("cross-check files absent: skipped")
        return
    for name in present:
        text = read_pinned(directory, name, CROSSCHECK[name]).replace("\r", "")

        def codec(text):
            out = {}
            for line in text.splitlines():
                if line.startswith("#") or not line.strip():
                    continue
                fields = line.split("\t")
                if len(fields) >= 2 and fields[1].strip().startswith("0x"):
                    out[int(fields[0], 16)] = int(fields[1], 16)
            return out

        if name == "stdenc.txt":
            names = {}
            for line in text.splitlines():
                if line.startswith("#") or not line.strip():
                    continue
                fields = line.split("\t")
                names[int(fields[1], 16)] = fields[3].lstrip("# ").strip()
            ours = {c: n for c, n in enumerate(tables["Standard"]) if n}
            if ours != names:
                fail(f"Standard differs from stdenc.txt: {set(ours.items()) ^ set(names.items())}")
            print("Standard = stdenc.txt (all 149 names)")
            continue
        table, known = {
            # Annex D's notes: bullets in CP1252's undefined slots; space
            # and hyphen for no-break space and soft hyphen. Codes below
            # 0x20 are undefined in PDF, controls in CP1252.
            "CP1252.TXT": ("WinAnsi", {0x7F, 0x81, 0x8D, 0x8F, 0x90, 0x9D, 0xA0, 0xAD}),
            # PDF's MacRoman: no math symbols and no Apple logo, currency
            # (not the Euro of Mac OS 8.5) at 0xDB, space for no-break
            # space at 0xCA. Controls are undefined.
            "ROMAN.TXT": (
                "MacRoman",
                {0x7F, 0xAD, 0xB0, 0xB2, 0xB3, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA,
                 0xBD, 0xC3, 0xC5, 0xC6, 0xD7, 0xF0, 0xDB, 0xCA},
            ),
        }[name]
        mapping = codec(text)
        wrong = []
        for code in range(0x20, 0x100):
            glyph = tables[table][code]
            ours = agl[glyph][0] if glyph else None
            if code in known:
                continue
            if ours != mapping.get(code):
                wrong.append((hex(code), glyph, mapping.get(code)))
        if wrong:
            fail(f"{table} differs from {name}: {wrong}")
        print(f"{table} = {name} through the AGL, except the {len(known)} listed codes")


# ------------------------------------------------------------------ output


def rust_char(cp):
    return "'\\u{%X}'" % cp


def rust_bytes(name):
    return 'b"' + name + '"'


def header(sources, typed, extra=()):
    lines = [
        "// @generated by engine/generate/generate_data.py; do not edit.",
        "// SPDX-License-Identifier: MIT",
        "//",
        "// Sources (file, SHA-256, what it is and its licence):",
    ]
    for name in sources:
        digest, about = SOURCES[name]
        lines.append(f"//   {name}")
        lines.append(f"//     {digest}")
        lines.append(f"//     {about}")
    for name in typed:
        lines.append(f"//   generate/tables/{name}")
        lines.append(f"//     {sha256(os.path.join(HERE, 'tables', name))}")
        lines.append(f"//     {TYPED[name]}")
    lines.extend(extra)
    lines.append("")
    return lines


def write(path, lines):
    with open(path, "w", newline="\n") as f:
        f.write("\n".join(lines) + "\n")
    print(f"wrote {path} ({len(lines)} lines)")


def simple_encoding_file(tables, agl):
    names = sorted({n for t in tables.values() for n in t if n})
    if len(names) != REGISTRY_NAMES:
        fail(f"{len(names)} registry names, expected {REGISTRY_NAMES}")
    for table, count in UNDEFINED_COUNTS.items():
        undefined = sum(1 for n in tables[table] if n is None)
        if undefined != count:
            fail(f"{table}: {undefined} undefined codes, expected {count}")
    for name in names:
        if name not in agl or len(agl[name]) != 1:
            fail(f"registry name {name} has no single AGL value")
    index = {n: i for i, n in enumerate(names)}
    out = header(
        ["glyphlist.txt"],
        ["annex-d-latin.txt", "annex-d-expert.txt"],
        [
            "//",
            "// The four named simple encodings of ISO 32000-2 Annex D, with the",
            "// table notes applied, and the Adobe Glyph List value of each glyph",
            "// name they use. The Adobe Glyph List is Copyright 2002-2019 Adobe,",
            "// BSD-3-Clause; its notice is in generate/data/AGL-LICENSE.md.",
        ],
    )
    out.append("/// A slot without a glyph name.")
    out.append("pub(super) const UNDEFINED: u16 = u16::MAX;")
    out.append("")
    out.append(f"/// The {len(names)} glyph names of the four tables, sorted, each with its")
    out.append("/// Adobe Glyph List value.")
    out.append("#[rustfmt::skip]")
    out.append(f"pub(super) static REGISTRY: [(&[u8], [char; 1]); {len(names)}] = [")
    for name in names:
        out.append(f"    ({rust_bytes(name)}, [{rust_char(agl[name][0])}]),")
    out.append("];")
    for table, rust in (
        ("Standard", "STANDARD"),
        ("MacRoman", "MAC_ROMAN"),
        ("WinAnsi", "WIN_ANSI"),
        ("MacExpert", "MAC_EXPERT"),
    ):
        out.append("")
        out.append(f"/// {table}Encoding: the `REGISTRY` index of each code's glyph name.")
        out.append("#[rustfmt::skip]")
        out.append(f"pub(super) static {rust}: [u16; 256] = [")
        for row in range(16):
            cells = []
            for code in range(row * 16, row * 16 + 16):
                name = tables[table][code]
                cells.append("UNDEFINED" if name is None else str(index[name]))
            out.append(f"    /* {row * 16:#04x} */ " + ", ".join(cells) + ",")
        out.append("];")
    return out


def text_unicode_file(
    agl, zapf, symbol, dingbats, mac_order, normalise, mirrors, default_bn, decomposition
):
    out = header(
        ["glyphlist.txt", "zapfdingbats.txt", "symbol.txt", "zdingbat.txt",
         "UnicodeData.txt", "BidiMirroring.txt", "DerivedBidiClass.txt"],
        ["mac-glyph-order.txt"],
        [
            "//",
            "// Notices: the Adobe Glyph List and the ITC Zapf Dingbats list are",
            "// Copyright 2002-2019 Adobe, BSD-3-Clause (generate/data/AGL-LICENSE.md).",
            "// symbol.txt and zdingbat.txt: Copyright (c) 1991-2011 Unicode, Inc.;",
            "// \"Unicode, Inc. hereby grants the right to freely use the information",
            "// supplied in this file in the creation of products supporting the",
            "// Unicode Standard, and to make copies of this file in any form for",
            "// internal or external distribution as long as this notice remains",
            "// attached.\" (the full notice is in each file in generate/data/).",
            "// The Unicode Character Database: Copyright (c) 1991-2026 Unicode, Inc.,",
            "// Unicode License v3 (generate/data/UNICODE-LICENSE.txt).",
        ],
    )

    def glyph_table(rust, about, table):
        out.append(f"/// {about}")
        out.append("#[rustfmt::skip]")
        out.append(f"pub(super) static {rust}: [(&[u8], &[char]); {len(table)}] = [")
        for name in sorted(table, key=lambda n: n.encode()):
            chars = ", ".join(rust_char(v) for v in table[name])
            out.append(f"    ({rust_bytes(name)}, &[{chars}]),")
        out.append("];")
        out.append("")

    out.append(f"/// Entries of the pinned glyphlist.txt.")
    out.append(f"pub(super) const AGL_ENTRIES: usize = {len(agl)};")
    out.append("")
    glyph_table("AGL", "The Adobe Glyph List, sorted by name.", agl)
    glyph_table("ZAPF_DINGBATS_LIST", "The ITC Zapf Dingbats Glyph List, sorted by name.", zapf)

    def builtin(rust, about, mapping):
        out.append(f"/// {about}")
        out.append("/// Of two values for one code, the one without a decomposition in")
        out.append("/// UnicodeData.txt (U+03BC, not MICRO SIGN), else the lower.")
        out.append("#[rustfmt::skip]")
        out.append(f"pub(super) static {rust}: [(u8, char); {len(mapping)}] = [")
        for code in sorted(mapping):
            value = min(mapping[code], key=lambda v: (v in decomposition, v))
            out.append(f"    ({code:#04x}, {rust_char(value)}),")
        out.append("];")
        out.append("")

    builtin("SYMBOL_BUILTIN", "The Symbol font's built-in encoding (symbol.txt).", symbol)
    builtin(
        "DINGBATS_BUILTIN",
        "The ZapfDingbats font's built-in encoding (zdingbat.txt).",
        dingbats,
    )
    out.append("/// The standard Macintosh glyph order (Apple, 'post' table).")
    out.append("#[rustfmt::skip]")
    out.append(f"pub(super) static MAC_GLYPH_ORDER: [&[u8]; {len(mac_order)}] = [")
    for name in mac_order:
        out.append(f"    {rust_bytes(name)},")
    out.append("];")
    out.append("")
    out.append("/// Every character of U+FB00-U+FEFF with a decomposition in")
    out.append("/// UnicodeData.txt, with its full compatibility decomposition (NFKD).")
    out.append("#[rustfmt::skip]")
    out.append(f"pub(super) static PRESENTATION_FORMS: [(char, &[char]); {len(normalise)}] = [")
    for cp, chars in normalise:
        values = ", ".join(rust_char(v) for v in chars)
        out.append(f"    ({rust_char(cp)}, &[{values}]),")
    out.append("];")
    out.append("")
    out.append("/// Bidi_Mirroring_Glyph (BidiMirroring.txt), sorted.")
    out.append("#[rustfmt::skip]")
    out.append(f"pub(super) static MIRRORS: [(char, char); {len(mirrors)}] = [")
    for source, target in mirrors:
        out.append(f"    ({rust_char(source)}, {rust_char(target)}),")
    out.append("];")
    out.append("")
    out.append("/// Unassigned code points whose default Bidi_Class is BN")
    out.append("/// (DerivedBidiClass.txt), as inclusive ranges, sorted.")
    out.append("#[rustfmt::skip]")
    out.append(f"pub(super) static BIDI_DEFAULT_BN: [(char, char); {len(default_bn)}] = [")
    for first, last in default_bn:
        out.append(f"    ({rust_char(first)}, {rust_char(last)}),")
    out.append("];")
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--data", default=os.path.join(HERE, "data"))
    parser.add_argument("--engine", default=os.path.dirname(HERE))
    args = parser.parse_args()

    text = {n: read_pinned(args.data, n, d) for n, (d, _) in SOURCES.items()}
    typed = {}
    for name in TYPED:
        with open(os.path.join(HERE, "tables", name), encoding="utf-8") as f:
            typed[name] = f.read()

    agl = parse_glyph_list(text["glyphlist.txt"])
    zapf = parse_glyph_list(text["zapfdingbats.txt"])
    tables = parse_annex_latin(typed["annex-d-latin.txt"])
    tables["MacExpert"] = parse_annex_expert(typed["annex-d-expert.txt"])
    mac_order = parse_mac_order(typed["mac-glyph-order.txt"])
    symbol = parse_vendor_mapping(text["symbol.txt"])
    dingbats = parse_vendor_mapping(text["zdingbat.txt"])
    ccc, decomposition, assigned = parse_unicode_data(text["UnicodeData.txt"])
    default_bn = default_bn_ranges(text["DerivedBidiClass.txt"], assigned)
    normalise = [
        (cp, nfkd(cp, decomposition, ccc))
        for cp in range(0xFB00, 0xFF00)
        if cp in decomposition
    ]
    if len(normalise) != PRESENTATION_FORMS:
        fail(f"{len(normalise)} presentation forms, expected {PRESENTATION_FORMS}")
    for cp, chars in normalise:
        if any(0xFB00 <= c <= 0xFEFF for c in chars):
            fail(f"U+{cp:04X} does not decompose out of the range")
    mirrors = parse_mirroring(text["BidiMirroring.txt"])

    crosscheck(os.path.join(args.data, "crosscheck"), tables, agl)

    write(
        os.path.join(args.engine, "src", "simple_encoding_data.rs"),
        simple_encoding_file(tables, agl),
    )
    write(
        os.path.join(args.engine, "src", "view", "text_unicode_data.rs"),
        text_unicode_file(
            agl, zapf, symbol, dingbats, mac_order, normalise, mirrors, default_bn,
            decomposition,
        ),
    )


if __name__ == "__main__":
    main()
