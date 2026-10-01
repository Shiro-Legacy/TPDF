#!/usr/bin/env python3
"""Generate an RC4-128 form: modify/fill allowed, annotation changes denied."""
import hashlib
import io
import json
import pathlib

from pypdf import PdfReader, PdfWriter
from pypdf.constants import UserAccessPermissions
from pypdf.generic import ArrayObject, DecodedStreamObject, DictionaryObject, NameObject, NumberObject, RectangleObject, TextStringObject

OUT = pathlib.Path(__file__).with_name("encrypted-edit-noannot.pdf")
PASSWORD = "pdf4all-user"
P = 4294967260  # signed 32-bit -36: bit 4 set, bit 6 clear, bit 9 set

writer = PdfWriter()
page = writer.add_blank_page(width=612, height=792)
font = writer._add_object(DictionaryObject({
    NameObject("/Type"): NameObject("/Font"), NameObject("/Subtype"): NameObject("/Type1"),
    NameObject("/BaseFont"): NameObject("/Helvetica"), NameObject("/Encoding"): NameObject("/WinAnsiEncoding"),
}))
page[NameObject("/Resources")] = DictionaryObject({NameObject("/Font"): DictionaryObject({NameObject("/F1"): font})})
content = DecodedStreamObject()
content.set_data(b"BT /F1 24 Tf 72 700 Td (PDF4All encrypted edit no annotate) Tj ET\nBT /F1 12 Tf 72 650 Td (fillable) Tj ET\n")
page[NameObject("/Contents")] = writer._add_object(content)
widget = writer._add_object(DictionaryObject({
    NameObject("/Type"): NameObject("/Annot"), NameObject("/Subtype"): NameObject("/Widget"),
    NameObject("/FT"): NameObject("/Tx"), NameObject("/T"): TextStringObject("fillable"),
    NameObject("/V"): TextStringObject(""), NameObject("/Rect"): RectangleObject([160, 630, 460, 654]),
    NameObject("/F"): NumberObject(4), NameObject("/P"): page.indirect_reference,
    NameObject("/DA"): TextStringObject("/Helv 12 Tf 0 g"),
}))
page[NameObject("/Annots")] = ArrayObject([widget])
writer._root_object[NameObject("/AcroForm")] = writer._add_object(DictionaryObject({
    NameObject("/Fields"): ArrayObject([widget]),
    NameObject("/DR"): DictionaryObject({NameObject("/Font"): DictionaryObject({NameObject("/Helv"): font})}),
    NameObject("/DA"): TextStringObject("/Helv 12 Tf 0 g"),
}))
permissions = UserAccessPermissions.all() & ~UserAccessPermissions.ADD_OR_MODIFY
writer.encrypt(PASSWORD, "pdf4all-owner", algorithm="RC4-128", permissions_flag=permissions)
buf = io.BytesIO()
writer.write(buf)
data = buf.getvalue()
OUT.write_bytes(data)

reader = PdfReader(io.BytesIO(data)); assert reader.is_encrypted and reader.decrypt(PASSWORD)
actual_p = int(reader.trailer["/Encrypt"].get_object()["/P"])
assert actual_p == P and actual_p & 8 and not actual_p & 32 and actual_p & 256
assert (reader.get_form_text_fields() or {}).get("fillable") == ""
provenance = {
    "id": "encrypted-edit-noannot",
    "file": "fixtures/p3/encrypted-edit-noannot.pdf",
    "bytes": len(data),
    "sha256": hashlib.sha256(data).hexdigest(),
    "pages": 1,
    "extractable_text": "PDF4All encrypted edit no annotate",
    "acroform": {"fields": [{"name": "fillable", "type": "Tx", "value": ""}]},
    "encryption": {
        "algorithm": "RC4-128", "user_password": PASSWORD, "owner_password": "pdf4all-owner",
        "P": P, "P_signed_32": -36, "permissions": {"modify_bit_4": True, "annotate_bit_6": False, "fill_bit_9": True},
    },
    "generator": {"path": "fixtures/p3/make-encrypted-edit-noannot.py", "reproduce": ".venv-qa/bin/python fixtures/p3/make-encrypted-edit-noannot.py", "dependencies": ["pypdf==6.18.1"]},
    "provenance": {"authorship": "written by the project team for P3 U6", "third_party_content": "none", "network_access": "none"},
    "generated_on": "2026-09-17",
}
OUT.with_suffix(".provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
print(f"wrote {OUT} ({len(data)} bytes) sha256={provenance['sha256']} /P={P}")
