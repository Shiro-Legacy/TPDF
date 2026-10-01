#!/usr/bin/env python3
"""P5 U3 probe fixture: three EMPTY /Sig fields (no /V). Page 0 unrotated: Signature1 (no /AP) and
Signature2 (a grey /AP box); page 1 /Rotate 90: Signature3 (no /AP). Deterministic via minipdf."""
import pathlib, sys
FIX = pathlib.Path('fixtures').resolve()
sys.path.insert(0, str(FIX))
from minipdf import emit, stream
OUT = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else pathlib.Path(__file__).with_name('empty-sigfield.pdf')
DA = b"/DA (/Helv 12 Tf 0 g)"
objects = [
    b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [7 0 R 8 0 R 10 0 R] /SigFlags 1 /DR << /Font << /Helv 3 0 R >> >> " + DA + b" >> >>",
    b"<< /Type /Pages /Kids [4 0 R 6 0 R] /Count 2 >>",
    b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents 5 0 R /Annots [7 0 R 8 0 R] >>",
    stream(b"BT /F1 24 Tf 72 700 Td (PDF4All sign on the page) Tj ET\n"),
    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Rotate 90 /Resources << /Font << /F1 3 0 R >> >> /Contents 5 0 R /Annots [10 0 R] >>",
    # 7: Signature1, no /AP, y-up [72 112 340 172] -> page space y-down [72 620 340 680]
    b"<< /Type /Annot /Subtype /Widget /FT /Sig /T (Signature1) /F 4 /P 4 0 R /Rect [72 112 340 172] " + DA + b" >>",
    # 8: Signature2 with an existing grey /AP (an empty-field look)
    b"<< /Type /Annot /Subtype /Widget /FT /Sig /T (Signature2) /F 4 /P 4 0 R /Rect [360 112 540 172] /AP << /N 9 0 R >> " + DA + b" >>",
    stream(b"q 0.85 g 0 0 180 60 re f Q\n", b" /Type /XObject /Subtype /Form /BBox [0 0 180 60]"),
    # 10: Signature3 on the rotated page, raw rect [72 500 300 560]
    b"<< /Type /Annot /Subtype /Widget /FT /Sig /T (Signature3) /F 4 /P 6 0 R /Rect [72 500 300 560] " + DA + b" >>",
]
size, digest = emit(OUT, objects)
print(OUT, size, digest)
