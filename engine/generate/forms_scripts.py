#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generates engine/src/app/forms/scripts_data.rs (spec 07 sections 4 and 6)
from public data files only:

- engine/generate/data/: the Unicode Consortium code-page mapping files
  (CP1252 for WinAnsiEncoding, ISO 8859-7 and CP1253 for Greek, CP1251 and
  KOI8-U for Cyrillic), the Adobe Glyph List and AGLFN, and Adobe's UTF-16
  CMaps (cmap-resources);
- engine/generate/data/ucd/: Unicode 16.0.0 Scripts.txt,
  DerivedCoreProperties.txt and UnicodeData.txt (required);
- engine/src/builtin_metrics_data.rs (the URW glyph names of this tree).

Run from anywhere: python3 -I engine/generate/forms_scripts.py
Nothing here is typed in: every table comes from those files and the rules
of spec 07.
"""
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(HERE, "data")
ENGINE = os.path.dirname(HERE)
OUT = os.path.join(ENGINE, "src", "app", "forms", "scripts_data.rs")


def read(path):
    with open(path, encoding="latin-1") as f:
        return f.read()


def mapping(name):
    """code -> Unicode scalar of a Unicode Consortium mapping file."""
    out = {}
    for line in read(os.path.join(DATA, name)).splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        parts = line.split()
        if len(parts) < 2:
            continue
        out[int(parts[0], 16)] = int(parts[1], 16)
    return out


def glyph_list():
    """AGL: name -> scalar (single-valued entries), and scalar -> names in
    file order."""
    by_name, by_value = {}, {}
    for line in read(os.path.join(DATA, "glyphlist.txt")).splitlines():
        if line.startswith("#") or ";" not in line:
            continue
        name, values = line.split(";", 1)
        values = values.split()
        if len(values) != 1:
            continue
        value = int(values[0], 16)
        by_name[name] = value
        by_value.setdefault(value, []).append(name)
    return by_name, by_value


def aglfn():
    out = {}
    for line in read(os.path.join(DATA, "aglfn.txt")).splitlines():
        if line.startswith("#") or not line.strip():
            continue
        value, name, _ = line.split(";", 2)
        out[int(value, 16)] = name
    return out


def urw_names(table):
    text = read(os.path.join(ENGINE, "src", "builtin_metrics_data.rs"))
    start = text.index("static %s:" % table)
    end = text.index("];", start)
    return set(re.findall(r'\(b"([^"]+)",', text[start:end]))


def ucd_file(name):
    path = os.path.join(DATA, "ucd", name)
    if not os.path.exists(path):
        sys.exit("missing %s: put the Unicode 16.0.0 UCD file there" % path)
    return path


def ranges(path, want):
    out = []
    for line in read(path).splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        span, value = [p.strip() for p in line.split(";", 1)]
        if value not in want:
            continue
        lo, _, hi = span.partition("..")
        out.append((int(lo, 16), int(hi or lo, 16), value))
    out.sort()
    return out


def merge(items):
    """Adjacent ranges of the same value joined."""
    out = []
    for lo, hi, value in items:
        if out and out[-1][2] == value and out[-1][1] + 1 >= lo:
            out[-1] = (out[-1][0], max(out[-1][1], hi), value)
        else:
            out.append((lo, hi, value))
    return out


def unicode_data():
    """UnicodeData.txt: general category (field 2) and decomposition
    (field 5) by code point."""
    category, decomposition = {}, {}
    for line in read(ucd_file("UnicodeData.txt")).splitlines():
        fields = line.split(";")
        if len(fields) < 6:
            continue
        cp = int(fields[0], 16)
        category[cp] = fields[2]
        if fields[5]:
            decomposition[cp] = fields[5]
    return category, decomposition


def cmap(name):
    """Supplement, and the CID of each ASCII code point, of a UTF-16 CMap."""
    text = read(os.path.join(DATA, name))
    supplement = int(re.search(r"/Supplement\s+(\d+)\s+def", text).group(1))
    cids = {}
    for block in re.findall(r"begincidrange(.*?)endcidrange", text, re.S):
        for lo, hi, cid in re.findall(r"<([0-9a-fA-F]+)>\s*<([0-9a-fA-F]+)>\s*(\d+)", block):
            lo, hi, cid = int(lo, 16), int(hi, 16), int(cid)
            for u in range(max(lo, 0x20), min(hi, 0x7E) + 1):
                cids[u] = cid + (u - lo)
    for block in re.findall(r"begincidchar(.*?)endcidchar", text, re.S):
        for code, cid in re.findall(r"<([0-9a-fA-F]+)>\s*(\d+)", block):
            u = int(code, 16)
            if 0x20 <= u <= 0x7E:
                cids[u] = int(cid)
    return supplement, cids


def main():
    agl_by_name, agl_by_value = glyph_list()
    category, decompositions = unicode_data()
    fn = aglfn()
    helvetica = urw_names("HELVETICA")

    # 6.4 WinAnsiEncoding: CP1252's defined codes 0x20..0x7E and 0x80..0xFF;
    # Annex D names 0xA0 `space` and 0xAD `hyphen`.
    cp1252 = mapping("CP1252.TXT")
    win = {}
    for code, value in cp1252.items():
        if 0x20 <= code <= 0x7E or code >= 0x80:
            if code == 0xA0:
                name = "space"
            elif code == 0xAD:
                name = "hyphen"
            else:
                name = fn.get(value) or (agl_by_value.get(value) or [None])[0]
            if name is None:
                sys.exit("no glyph name for WinAnsi 0x%02X" % code)
            win[code] = (value, name)
    win_chars = {value for value, _ in win.values()}

    # 6.3 drawability and names.
    def drawable(value):
        candidates = list(agl_by_value.get(value, [])) + ["uni%04X" % value]
        found = [n for n in candidates if n in helvetica]
        if not found:
            return None
        if fn.get(value) in found:
            return fn[value]
        return found[0]

    def layout(sources):
        seen, out = set(), []
        for value in sources:
            if value < 0xA0 or value in win_chars or value in seen:
                continue
            name = drawable(value)
            if name is None:
                continue
            seen.add(value)
            out.append((value, name))
        if len(out) > 128:
            sys.exit("a layout holds more than 128 codes")
        return [(0x80 + i, value, name) for i, (value, name) in enumerate(out)]

    def upper(table):
        return [table[code] for code in sorted(table) if code >= 0x80]

    greek = layout(upper(mapping("8859-7.TXT")) + upper(mapping("CP1253.TXT")))
    cyrillic = layout(upper(mapping("CP1251.TXT")) + upper(mapping("KOI8-U.TXT")))
    # 6.2: every letter of Latin Extended-A WinAnsi lacks, then U+0218..021B.
    latin_sources = [
        v for v in range(0x100, 0x180) if category.get(v, "").startswith("L")
    ] + list(range(0x218, 0x21C))
    latin = layout(latin_sources)
    missing = [v for v in latin_sources if v not in win_chars and v not in {x[1] for x in latin}]
    if missing:
        sys.exit("Latin letters the URW face cannot draw: %s" % missing)

    # 4.1 scripts and 4.3 ignorables.
    scripts_path = ucd_file("Scripts.txt")
    classes = {
        "Latin": "LATIN",
        "Greek": "GREEK",
        "Cyrillic": "CYRILLIC",
        "Han": "HAN",
        "Bopomofo": "HAN",
        "Hiragana": "KANA",
        "Katakana": "KANA",
        "Hangul": "HANGUL",
    }
    script_ranges = merge(
        sorted((lo, hi, classes[v]) for lo, hi, v in ranges(scripts_path, set(classes)))
    )
    di_path = ucd_file("DerivedCoreProperties.txt")
    ignorable = merge(ranges(di_path, {"Default_Ignorable_Code_Point"}))

    # 4.3 composition: pairs (base, mark U+0300..036F) whose canonical
    # composition is a character some 8-bit font of section 4.2 encodes.
    decomposition = decompositions.get
    repertoire = set(win_chars)
    for table in (greek, cyrillic, latin):
        repertoire.update(value for _, value, _ in table)
    compose = []
    for value in sorted(repertoire):
        d = decomposition(value)
        if not d or d.startswith("<"):
            continue
        parts = [int(p, 16) for p in d.split()]
        if len(parts) == 2 and 0x300 <= parts[1] <= 0x36F:
            compose.append((parts[0], parts[1], value))
    compose.sort()

    # 5.3 and 5.4: CMap supplements and the CIDs ASCII maps to.
    cmaps = []
    for name in ["UniGB-UTF16-H", "UniCNS-UTF16-H", "UniJIS-UTF16-H", "UniKS-UTF16-H"]:
        supplement, cids = cmap(name)
        if sorted(cids) != list(range(0x20, 0x7F)):
            sys.exit("%s does not map all of ASCII" % name)
        spans = []
        for cid in sorted(set(cids.values())):
            if spans and spans[-1][1] + 1 == cid:
                spans[-1][1] = cid
            else:
                spans.append([cid, cid])
        cmaps.append((name, supplement, spans))

    out = []
    w = out.append
    w("// SPDX-License-Identifier: MIT")
    w("// Generated by engine/generate/forms_scripts.py; do not edit.")
    w("// Sources (engine/generate/data, SOURCES.md): Unicode Consortium CP1252,")
    w("// 8859-7, CP1253, CP1251 and KOI8-U mapping files; Adobe Glyph List 2.0 and")
    w("// AGLFN 1.7 (BSD-3-Clause, Copyright 2002-2019 Adobe); Adobe cmap-resources")
    w("// UTF-16 CMaps (BSD-3-Clause, Copyright 1990-2023 Adobe); the URW glyph")
    w("// names of builtin_metrics_data.rs.")
    w("// Unicode 16.0.0 Scripts.txt, DerivedCoreProperties.txt and UnicodeData.txt")
    w("// (Unicode License v3).")
    w("")

    def name_bytes(name):
        return 'b"%s"' % name

    w("/// WinAnsiEncoding (ISO 32000-2 Annex D): code, Unicode scalar, glyph name.")
    w("pub(crate) static WIN_ANSI: [(u8, u32, &[u8]); %d] = [" % len(win))
    for code in sorted(win):
        value, name = win[code]
        w("    (0x%02X, 0x%04X, %s)," % (code, value, name_bytes(name)))
    w("];")
    w("")
    by_char = sorted((value, code) for code, (value, _) in win.items())
    w("/// WinAnsiEncoding by Unicode scalar.")
    w("pub(crate) static WIN_ANSI_BY_CHAR: [(u32, u8); %d] = [" % len(by_char))
    for value, code in by_char:
        w("    (0x%04X, 0x%02X)," % (value, code))
    w("];")
    w("")
    for label, table, doc in [
        ("GREEK", greek, "Greek: ISO 8859-7, then CP1253"),
        ("CYRILLIC", cyrillic, "Cyrillic: CP1251, then KOI8-U"),
        ("LATIN", latin, "extended Latin: Latin Extended-A letters, then U+0218..U+021B"),
    ]:
        w("/// The %s layout (%s; characters WinAnsi lacks that the URW" % (label.lower(), doc))
        w("/// Helvetica face draws, packed from 0x80): code, scalar, glyph name.")
        w("pub(crate) static %s: [(u8, u32, &[u8]); %d] = [" % (label, len(table)))
        for code, value, name in table:
            w("    (0x%02X, 0x%04X, %s)," % (code, value, name_bytes(name)))
        w("];")
        w("")
        rev = sorted((value, code) for code, value, _ in table)
        w("pub(crate) static %s_BY_CHAR: [(u32, u8); %d] = [" % (label, len(rev)))
        for value, code in rev:
            w("    (0x%04X, 0x%02X)," % (value, code))
        w("];")
        w("")
    w("/// Script classes (spec 07 section 4.1), sorted ranges.")
    w("pub(crate) static SCRIPTS: [(u32, u32, super::Class); %d] = [" % len(script_ranges))
    for lo, hi, cls in script_ranges:
        w("    (0x%04X, 0x%04X, super::Class::%s)," % (lo, hi, cls.capitalize()))
    w("];")
    w("")
    w("/// Default_Ignorable_Code_Point, sorted ranges.")
    w("pub(crate) static IGNORABLE: [(u32, u32); %d] = [" % len(ignorable))
    for lo, hi, _ in ignorable:
        w("    (0x%04X, 0x%04X)," % (lo, hi))
    w("];")
    w("")
    w("/// Canonical compositions into the 8-bit repertoire: base, mark, result,")
    w("/// sorted by base and mark.")
    w("pub(crate) static COMPOSE: [(u32, u32, u32); %d] = [" % len(compose))
    for base, mark, value in compose:
        w("    (0x%04X, 0x%04X, 0x%04X)," % (base, mark, value))
    w("];")
    w("")
    w("/// Per collection (GB1, CNS1, Japan1, Korea1): the UTF-16 CMap's")
    w("/// Supplement and the CID ranges it maps U+0020..U+007E to.")
    w("pub(crate) static CMAPS: [(u32, &[(u32, u32)]); 4] = [")
    for name, supplement, spans in cmaps:
        inner = ", ".join("(%d, %d)" % (a, b) for a, b in spans)
        w("    // %s" % name)
        w("    (%d, &[%s])," % (supplement, inner))
    w("];")
    with open(OUT, "w", encoding="utf-8") as f:
        f.write("\n".join(out) + "\n")
    print(
        "wrote %s: WinAnsi %d, Greek %d, Cyrillic %d, Latin %d, scripts %d, ignorable %d, compose %d"
        % (OUT, len(win), len(greek), len(cyrillic), len(latin), len(script_ranges), len(ignorable), len(compose))
    )


if __name__ == "__main__":
    main()
