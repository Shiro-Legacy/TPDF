#!/usr/bin/env python3
"""Generate form2.pdf with the sole addition of /AcroForm /XFA (stub)."""
import json
import pathlib
import runpy
import sys

HERE = pathlib.Path(__file__).resolve().parent
FIXTURES = HERE.parent
sys.path.insert(0, str(FIXTURES))
from minipdf import emit, sha256_file

base = runpy.run_path(str(FIXTURES / "make-form2.py"))
objects = list(base["objects"])
needle = b" /CO [17 0 R] >>"
assert objects[5].count(needle) == 1
objects[5] = objects[5].replace(needle, b" /CO [17 0 R] /XFA (stub) >>")
OUT = HERE / "form-xfa.pdf"
size, digest = emit(OUT, objects)
data = OUT.read_bytes()
assert data.count(b"/XFA (stub)") == 1 and data.count(b"/T (") == 9
assert b"/Names /JavaScript" not in data and b"/AA <<" in data and b"/CO [17 0 R]" in data
provenance = {
    "id": "form-xfa",
    "file": "fixtures/p3/form-xfa.pdf",
    "bytes": size,
    "sha256": digest,
    "pages": 1,
    "page_size_pt": [612, 792],
    "extractable_text": "PDF4All form2",
    "acroform": {"based_on": "fixtures/form2.pdf", "fields": 9, "xfa": "stub", "scripted_fields": ["amount"]},
    "generator": {
        "path": "fixtures/p3/make-form-xfa.py",
        "helper": "fixtures/minipdf.py",
        "sha256": sha256_file(__file__),
        "reproduce": "python3 fixtures/p3/make-form-xfa.py",
        "deterministic": True,
        "timestamps_in_output": False,
        "dependencies": ["python3 stdlib only"],
    },
    "provenance": {"authorship": "written by the project team for P3 U6", "source_documents": "fixtures/form2.pdf structure, generated in-tree", "third_party_content": "none", "network_access": "none"},
    "generated_on": "2026-09-17",
}
OUT.with_suffix(".provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
print(f"wrote {OUT} ({size} bytes) sha256={digest}")
