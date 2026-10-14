#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Pin native PDF glyph tables and record differences from Python text codecs."""
import argparse
import collections
import hashlib
import json
import pathlib
import re
import subprocess

p = argparse.ArgumentParser()
p.add_argument('oracle', type=pathlib.Path)
p.add_argument('source', type=pathlib.Path)
p.add_argument('report', type=pathlib.Path)
a = p.parse_args()
sha = lambda b: hashlib.sha256(b).hexdigest()
run = subprocess.run([str(a.oracle.resolve())], capture_output=True, timeout=10, check=True)
assert len(run.stdout) < 1024 * 1024
header = (a.source / 'source/fitz/encodings.h').read_text()
native, differences = collections.defaultdict(list), []
for line in run.stdout.decode('ascii').splitlines():
    name, byte, glyph, strict, loose = line.split('\t')
    row = {'byte': int(byte), 'glyph': None if glyph == '-' else glyph, 'nativeStrict': int(strict), 'nativeLoose': int(loose)}
    native[name].append(row)
assert len(native) == 4 and all(len(rows) == 256 for rows in native.values())
# Parse fixed C tables independently to ensure ABI uses the intended PDF table.
for encoding, symbol in [('WinAnsiEncoding','win_ansi'),('MacRomanEncoding','mac_roman'),('StandardEncoding','adobe_standard'),('MacExpertEncoding','mac_expert')]:
    body = re.search(r'fz_glyph_name_from_' + symbol + r'\[256\]\s*=\s*\{(.*?)\};', header, re.S).group(1)
    names = re.findall(r'"([^"]+)"|(_notdef)', body)
    expected = [name if name else None for name, undefined in names]
    assert [row['glyph'] for row in native[encoding]] == expected
for encoding, codec in [('WinAnsiEncoding','cp1252'),('MacRomanEncoding','mac_roman')]:
    for row in native[encoding]:
        try:
            scalar = ord(bytes([row['byte']]).decode(codec))
        except UnicodeDecodeError:
            scalar = None
        pdf_scalar = row['nativeStrict'] if row['glyph'] is not None else None
        if pdf_scalar != scalar:
            differences.append({'encoding':encoding, **row, 'pythonCodec':codec,'pythonScalar':scalar})
paths = ['source/fitz/encodings.c','source/fitz/encodings.h','source/fitz/glyphlist.h','scripts/glyphlist.txt','scripts/texglyphlist.txt','scripts/glyphdump.py','source/pdf/pdf-font.c','source/pdf/pdf-unicode.c']
report = {'reference':'MuPDF1.28.1 fixed PDF encoding tables and native strict/loose glyph resolver; no font loading',
'oracleSha256':sha(a.oracle.read_bytes()),'sourceSha256':{path:sha((a.source/path).read_bytes()) for path in paths},
'rows':1024,'undefinedSlots':{name:sum(row['glyph'] is None for row in rows) for name,rows in native.items()},
'namedGlyphsMissingStrictMapping':[{ 'encoding':name, **row } for name,rows in native.items() for row in rows if row['glyph'] is not None and row['nativeStrict']==0],
'codecDifferenceCounts':dict(collections.Counter(row['encoding'] for row in differences)),
'codecDifferences':differences,'tables':dict(native),
'limitations':['Python codec values are comparison data, not the expected PDF byte mapping.','Native glyph strict zero is an ambiguous C API sentinel; table names here have no U+0000 mapping.','Native loose glyph heuristics do not establish an Adobe Glyph List algorithm or multi-scalar policy.']}
a.report.write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps({key:value for key,value in report.items() if key not in ['tables','codecDifferences','sourceSha256']},indent=2))
