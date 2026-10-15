// SPDX-License-Identifier: AGPL-3.0-or-later
//! Text strings in the app's replies: annotation contents and subjects,
//! field names and values, page label prefixes and layer names all read a
//! string as MuPDF does (`object::decode_text_string`), so UTF-8 without a
//! byte order mark is UTF-8 everywhere, measured against MuPDF 1.28.1.
use serde_json::{Value, json};
use tandem_pdf_engine::app::{AppEngine, AppSession};

/// A classic-xref PDF from object bodies numbered from 1.
fn classic(objects: &[String]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn op(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    let (reply, _) = engine.dispatch(session, request, Vec::new());
    assert_eq!(reply["ok"], true, "{reply}");
    reply
}

#[test]
fn every_reply_reads_text_strings_as_mupdf_does() {
    // (string, MuPDF's reading): UTF-8 without a mark, a string that is
    // also UTF-8 but holds a byte PDFDocEncoding reads as an accent, and
    // Latin-1 bytes that are not UTF-8.
    for (hex, text) in [
        ("636166C3A9", "café"),
        ("E697A5E69CAC", "日本"),
        ("C3A918", "Ã©˘"),
        ("E974E9", "été"),
    ] {
        let objects = [
            format!(
                "<< /Type /Catalog /Pages 2 0 R /PageLabels << /Nums [0 << /S /D /P <{hex}> >>] >> \
                 /OCProperties << /OCGs [6 0 R] /D << /ON [6 0 R] >> >> /AcroForm << /Fields [5 0 R] >> >>"
            ),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Annots [4 0 R 5 0 R] >>".into(),
            format!(
                "<< /Type /Annot /Subtype /Text /Rect [10 10 30 30] /Contents <{hex}> /Subj <{hex}> >>"
            ),
            format!(
                "<< /Type /Annot /Subtype /Widget /FT /Tx /T <{hex}> /V <{hex}> /Rect [50 50 150 70] /P 3 0 R >>"
            ),
            format!("<< /Type /OCG /Name <{hex}> >>"),
        ];
        let mut engine = AppEngine::new();
        let mut session = AppSession::default();
        let open = json!({"op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}});
        let (reply, _) = engine.dispatch(&mut session, open, vec![classic(&objects)]);
        assert_eq!(reply["ok"], true, "{reply}");

        let annots = op(
            &mut engine,
            &mut session,
            json!({"op": "annots", "page": 0}),
        );
        let note = &annots["items"][0];
        assert_eq!(
            (&note["contents"], &note["subject"]),
            (&json!(text), &json!(text)),
            "{hex}"
        );
        let fields = op(
            &mut engine,
            &mut session,
            json!({"op": "fields", "page": 0}),
        );
        let field = &fields["items"][0];
        assert_eq!(
            (&field["name"], &field["value"]),
            (&json!(text), &json!(text)),
            "{hex}"
        );
        let labels = op(&mut engine, &mut session, json!({"op": "labels"}));
        assert_eq!(labels["items"], json!([format!("{text}1")]), "{hex}");
        let layers = op(&mut engine, &mut session, json!({"op": "layers"}));
        assert_eq!(layers["items"][0]["name"], text, "{hex}");
    }
}
