#!/usr/bin/env python3
"""Compare two PDFs: pixels outside a page-space rect stay identical; inside change.

QA only, never shipped. stdlib + qlmanage + sips. No new packages.

Usage: .venv-qa/bin/python scripts/verify-edit-region.py before.pdf after.pdf --rect x0,y0,x1,y1 [--rect ...] [--margin 2] [--page 0]
"""
import argparse
import json
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

PAGE = (612.0, 792.0)


def ql_png(pdf: Path, outdir: Path) -> Path:
    subprocess.run(["qlmanage", "-t", "-s", "800", "-o", str(outdir), str(pdf)], check=True, capture_output=True)
    pngs = list(outdir.glob("*.png"))
    if not pngs:
        raise SystemExit("qlmanage produced no png")
    return pngs[0]


def bmp_pixels(bmp: Path):
    data = bmp.read_bytes()
    off = struct.unpack_from("<I", data, 10)[0]
    width = struct.unpack_from("<i", data, 18)[0]
    height = struct.unpack_from("<i", data, 22)[0]
    bpp = struct.unpack_from("<H", data, 28)[0]
    top_down = height < 0
    height = abs(height)
    row = ((width * bpp + 31) // 32) * 4

    def at(x, y):
        raw_y = y if top_down else height - 1 - y
        i = off + raw_y * row + x * (bpp // 8)
        return data[i + 2], data[i + 1], data[i]

    return width, height, at


def render(pdf: Path, tmp: Path):
    png = ql_png(pdf, tmp)
    bmp = tmp / "page.bmp"
    subprocess.run(["sips", "-s", "format", "bmp", str(png), "--out", str(bmp)], check=True, capture_output=True)
    return bmp_pixels(bmp)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("before", type=Path)
    p.add_argument("after", type=Path)
    p.add_argument("--rect", required=True, action="append", help="x0,y0,x1,y1 page points, PDF y-up; repeat for multiple regions")
    p.add_argument("--margin", type=float, default=2)
    p.add_argument("--page", type=int, default=0)
    args = p.parse_args()
    if args.page != 0:
        raise SystemExit("only page 0 is rendered (qlmanage thumbnail)")
    rects = [[float(x) for x in value.split(",")] for value in args.rect]
    if any(len(rect) != 4 for rect in rects):
        raise SystemExit("each --rect needs x0,y0,x1,y1")
    m = args.margin
    expanded = [[x0 - m, y0 - m, x1 + m, y1 + m] for x0, y0, x1, y1 in rects]

    with tempfile.TemporaryDirectory() as tmp_a, tempfile.TemporaryDirectory() as tmp_b:
        w0, h0, at0 = render(args.before, Path(tmp_a))
        w1, h1, at1 = render(args.after, Path(tmp_b))
        if (w0, h0) != (w1, h1):
            print(json.dumps({"ok": False, "error": "size", "before": [w0, h0], "after": [w1, h1]}))
            return 2
        sx, sy = w0 / PAGE[0], h0 / PAGE[1]

        def img(x, y):
            return min(w0 - 1, max(0, int(x * sx))), min(h0 - 1, max(0, int((PAGE[1] - y) * sy)))

        boxes = []
        for rx0, ry0, rx1, ry1 in expanded:
            ix0, iy_top = img(rx0, ry1)
            ix1, iy_bot = img(rx1, ry0)
            boxes.append((min(ix0, ix1), min(iy_top, iy_bot), max(ix0, ix1), max(iy_top, iy_bot)))

        inside_n = [0] * len(boxes)
        inside_diff = [0] * len(boxes)
        outside_diff = outside_n = 0
        for y in range(h0):
            for x in range(w0):
                same = at0(x, y) == at1(x, y)
                matches = [i for i, (xa, ya, xb, yb) in enumerate(boxes) if xa <= x <= xb and ya <= y <= yb]
                if matches:
                    for i in matches:
                        inside_n[i] += 1
                        if not same:
                            inside_diff[i] += 1
                else:
                    outside_n += 1
                    if not same:
                        outside_diff += 1
        regions = [{
            "rect": rect,
            "expanded": growth,
            "inside_n": total,
            "inside_diff": changed,
            "inside_frac": round(changed / total if total else 0, 6),
        } for rect, growth, total, changed in zip(rects, expanded, inside_n, inside_diff)]
        outside_identical = outside_diff == 0
        ok = outside_identical and all(region["inside_frac"] >= 0.01 for region in regions)
        verdict = {
            "ok": ok,
            "margin": m,
            "pixels": [w0, h0],
            "inside_n": sum(inside_n),
            "inside_diff": sum(inside_diff),
            "outside_n": outside_n,
            "outside_diff": outside_diff,
            "outside_identical": outside_identical,
        }
        if len(regions) == 1:
            verdict.update(regions[0])
        else:
            verdict.update({"rects": rects, "expanded": expanded, "regions": regions})
        print(json.dumps(verdict))
        return 0 if ok else 2


if __name__ == "__main__":
    raise SystemExit(main())
