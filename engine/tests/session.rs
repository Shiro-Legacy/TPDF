// SPDX-License-Identifier: AGPL-3.0-or-later
use tandem_pdf_engine::{
    document::{Document, DocumentLimits},
    object::ObjectId,
    session::{Session, SessionLimits},
    stream::StreamLimits,
    stream_object::decode_document_stream,
};
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("../fixtures/{name}")).unwrap()
}
fn open(bytes: Vec<u8>) -> Session {
    Session::open(bytes, SessionLimits::default()).unwrap()
}
fn target(s: &mut Session, text: &str) -> tandem_pdf_engine::session::TextTarget {
    s.text_targets(0)
        .unwrap()
        .into_iter()
        .find(|t| t.text == text)
        .unwrap()
}
fn replace(s: &mut Session, old: &str, new: &str) {
    let t = target(s, old);
    s.replace_text(&t, new).unwrap();
}
fn pdf(content: &[u8], metadata: &str, two_pages: bool, shared: bool) -> Vec<u8> {
    let kids = if two_pages { "3 0 R 6 0 R" } else { "3 0 R" };
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!("<< /Type /Pages /Kids [{kids}] /Count {} >>", if two_pages {2} else {1}).into_bytes(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
        [format!("<< /Length {} {metadata} >>\nstream\n", content.len()).as_bytes(), content, b"\nendstream"].concat(),
    ];
    if two_pages {
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Resources << /Font << /F1 4 0 R >> >> /Contents {} 0 R >>", if shared {5} else {7}).into_bytes());
        if !shared {
            let contents = b"BT /F1 12 Tf 20 200 Td (NEIGHBOR) Tj ET";
            objects.push(
                [
                    format!("<< /Length {} >>\nstream\n", contents.len()).as_bytes(),
                    contents,
                    b"\nendstream",
                ]
                .concat(),
            );
        }
    }
    pdf_objects(objects)
}
fn pdf_objects(objects: Vec<Vec<u8>>) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![];
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend(object);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

fn feature_pdf(catalog: &str, page: &str, extra: &[&str]) -> Vec<u8> {
    let contents = b"BT /F1 12 Tf 20 250 Td (ALPHA) Tj ET";
    let mut objects = vec![
        format!("<< /Type /Catalog /Pages 2 0 R {catalog} >>").into_bytes(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R {page} >>").into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
        [format!("<< /Length {} >>\nstream\n", contents.len()).as_bytes(), contents, b"\nendstream"].concat(),
    ];
    objects.extend(extra.iter().map(|s| s.as_bytes().to_vec()));
    pdf_objects(objects)
}
fn refusal(bytes: Vec<u8>) -> tandem_pdf_engine::session::SessionError {
    match Session::open(bytes, SessionLimits::default()) {
        Ok(_) => panic!("unsupported feature was accepted"),
        Err(error) => error,
    }
}
#[test]
fn empty_indirect_form_metadata_survives_edit_save_reopen() {
    let bytes = feature_pdf(
        "/AcroForm 6 0 R",
        "/Annots 8 0 R",
        &[
            "<< /Fields 7 0 R /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 4 0 R >> >> >>",
            "[]",
            "[]",
        ],
    );
    let mut session = open(bytes.clone());
    replace(&mut session, "ALPHA", "BETA");
    let saved = session.save().unwrap().bytes;
    let before = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let after = Document::open(&saved, DocumentLimits::default()).unwrap();
    for object in before.objects().filter(|o| o.id.number() != 5) {
        assert_eq!(object.value, after.object(object.id).unwrap().value);
    }
    assert_eq!(target(&mut open(saved), "BETA").text, "BETA");
}
#[test]
fn borderless_links_survive_edit_save_reopen() {
    let bytes = fixture("p6/links.pdf");
    let mut session = open(bytes.clone());
    replace(&mut session, "One", "New");
    let saved = session.save().unwrap().bytes;
    let before = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let after = Document::open(&saved, DocumentLimits::default()).unwrap();
    for object in before.objects().filter(|o| o.id.number() != 7) {
        assert_eq!(object.value, after.object(object.id).unwrap().value);
        assert_eq!(
            before.stream_data(object.id).unwrap(),
            after.stream_data(object.id).unwrap()
        );
    }
    assert_eq!(target(&mut open(saved), "New").text, "New");
}
#[test]
fn annotation_appearance_and_border_precedence_are_enforced() {
    for extra in [
        "",
        "/BS << >>",
        "/BS << /W 2 >> /Border [0 0 0]",
        "/Border [0 0 1]",
    ] {
        let annot = format!("<< /Subtype /Link /Rect [0 0 10 10] {extra} >>");
        let error = refusal(feature_pdf("", "/Annots [6 0 R]", &[&annot]));
        assert_eq!(error.reason, "unsupported-annotation", "{extra}");
        assert!(error.detail.contains("Page 1"));
        assert!(error.detail.contains("Link"));
    }
    let mut s = open(feature_pdf(
        "",
        "/Annots 6 0 R",
        &[
            "[7 0 R]",
            "<< /Subtype /Link /Rect [0 0 10 10] /BS 8 0 R /Border [0 0 2] >>",
            "<< /W 0 >>",
        ],
    ));
    replace(&mut s, "ALPHA", "BETA");
    open(s.save().unwrap().bytes);
    // An appearance dictionary that selects no stream draws nothing.
    open(feature_pdf(
        "",
        "/Annots [6 0 R]",
        &["<< /Subtype /Link /Rect [0 0 10 10] /Border [0 0 1] /AP << >> >>"],
    ));
}
#[test]
fn visible_annotations_forms_and_layers_get_specific_refusals() {
    let error = refusal(feature_pdf(
        "",
        "/Annots [6 0 R]",
        &["<< /Subtype /Highlight /Rect [0 0 10 10] >>"],
    ));
    assert_eq!(error.reason, "unsupported-annotation");
    assert!(error.detail.contains("Highlight"));
    assert_eq!(
        refusal(feature_pdf(
            "/AcroForm << /Fields [6 0 R] >>",
            "",
            &["<< /FT /Tx >>"]
        ))
        .reason,
        "unsupported-forms"
    );
    assert_eq!(
        refusal(feature_pdf(
            "/AcroForm << /Fields [] /XFA (data) >>",
            "",
            &[]
        ))
        .reason,
        "unsupported-forms"
    );
    assert_eq!(
        refusal(feature_pdf(
            "/OCProperties << /OCGs [] /D << >> >>",
            "",
            &[]
        ))
        .reason,
        "unsupported-layers"
    );
}
#[test]
fn only_live_catalog_and_page_feature_keys_trigger_admission() {
    let mut s = open(feature_pdf(
        "/AcroForm null /OCProperties << >>",
        "/Annots null",
        &["<< /Annots 123 /AcroForm (custom metadata) /OCProperties true >>"],
    ));
    replace(&mut s, "ALPHA", "BETA");
    open(s.save().unwrap().bytes);
}
#[test]
fn cyclic_and_malformed_feature_values_are_rejected() {
    for bytes in [
        feature_pdf("/AcroForm 6 0 R", "", &["7 0 R", "6 0 R"]),
        feature_pdf("", "/Annots 6 0 R", &["6 0 R"]),
        feature_pdf("/AcroForm << /Fields 2 >>", "", &[]),
        feature_pdf("", "/Annots 2", &[]),
        feature_pdf(
            "",
            "/Annots [6 0 R]",
            &["<< /Subtype /Link /Rect [0 0 10] /Border [0 0 0] >>"],
        ),
    ] {
        assert!(matches!(
            refusal(bytes).reason,
            "invalid-document" | "unsupported-save"
        ));
    }
}
#[test]
fn tagged_pdf_preserves_actual_text_by_not_exposing_edits() {
    let mut session = open(feature_pdf(
        "/StructTreeRoot 6 0 R",
        "",
        &["<< /Type /StructTreeRoot /K [] /ActualText (ALPHA) >>"],
    ));
    assert!(session.render(0, 72.).is_ok());
    let error = session.text_targets(0).unwrap_err();
    assert_eq!(error.reason, "not-candidate");
    assert!(error.detail.contains("structure-tree"));
    let original = session.bytes().to_vec();
    let saved = session.save().unwrap().bytes;
    let before = Document::open(&original, DocumentLimits::default()).unwrap();
    let after = Document::open(&saved, DocumentLimits::default()).unwrap();
    for object in before.objects() {
        assert_eq!(object.value, after.object(object.id).unwrap().value);
    }
    assert_eq!(
        open(saved).text_targets(0).unwrap_err().reason,
        "not-candidate"
    );
}
#[test]
fn existing_pdf_edit_undo_redo_save_reopen_and_edit_again() {
    let input = fixture("hello.pdf");
    let mut s = open(input.clone());
    let original = s.render(0, 72.).unwrap().pixels().to_vec();
    replace(&mut s, "PDF4All", "All4PDF");
    assert!(s.state().dirty);
    let edited = s.render(0, 72.).unwrap().pixels().to_vec();
    assert_ne!(original, edited);
    s.undo().unwrap();
    assert_eq!(s.bytes(), input);
    assert!(!s.state().dirty);
    assert_eq!(s.render(0, 72.).unwrap().pixels(), original);
    s.redo().unwrap();
    assert_eq!(s.render(0, 72.).unwrap().pixels(), edited);
    let saved = s.save().unwrap();
    assert!(s.state().dirty);
    s.acknowledge_saved(saved.token).unwrap();
    assert!(!s.state().dirty);
    let mut reopened = open(saved.bytes);
    assert_eq!(target(&mut reopened, "All4PDF").text, "All4PDF");
    assert_eq!(reopened.render(0, 72.).unwrap().pixels(), edited);
    replace(&mut reopened, "All4PDF", "PDF4All");
    let mut twice = open(reopened.save().unwrap().bytes);
    assert_eq!(twice.render(0, 72.).unwrap().pixels(), original);
}
#[test]
fn transformed_text_edit_preserves_image_and_neighbor() {
    let input = fixture("p0b/edit-transformed.pdf");
    let mut s = open(input.clone());
    let before = Document::open(&input, DocumentLimits::default()).unwrap();
    replace(&mut s, "ALPHA", "HALO");
    assert_eq!(target(&mut s, "NEIGHBOR").text, "NEIGHBOR");
    let bytes = s.save().unwrap().bytes;
    let after = Document::open(&bytes, DocumentLimits::default()).unwrap();
    for obj in before
        .objects()
        .filter(|o| o.id != ObjectId::new(5, 0).unwrap())
    {
        assert_eq!(obj.value, after.object(obj.id).unwrap().value);
        assert_eq!(
            before.stream_data(obj.id).unwrap(),
            after.stream_data(obj.id).unwrap()
        );
    }
    let mut reopened = open(bytes);
    assert_eq!(target(&mut reopened, "HALO").text, "HALO");
}
#[test]
fn compressed_escaped_text_edit_removes_filter_and_preserves_other_page() {
    let raw = b"BT /F1 12 Tf 20 250 Td (ALP\\110A) Tj ET";
    let encoded = miniz_oxide::deflate::compress_to_vec_zlib(raw, 6);
    let input = pdf(&encoded, "/Filter /FlateDecode", true, false);
    let mut s = open(input.clone());
    let page2 = s.render(1, 72.).unwrap().pixels().to_vec();
    replace(&mut s, "ALPHA", "HAL");
    let saved = s.save().unwrap().bytes;
    let doc = Document::open(&saved, DocumentLimits::default()).unwrap();
    let id = ObjectId::new(5, 0).unwrap();
    let tandem_pdf_engine::object::Object::Dictionary(dict) = &doc.object(id).unwrap().value else {
        panic!()
    };
    assert!(!dict.contains_key(b"Filter".as_slice()));
    assert!(!dict.contains_key(b"DecodeParms".as_slice()));
    assert!(
        decode_document_stream(&doc, id, StreamLimits::default())
            .unwrap()
            .windows(b"<48414C> Tj".len())
            .any(|v| v == b"<48414C> Tj")
    );
    let mut reopened = open(saved);
    assert_eq!(reopened.render(1, 72.).unwrap().pixels(), page2);
}
#[test]
fn refusal_and_noop_keep_redo_and_saved_state() {
    let mut s = open(fixture("hello.pdf"));
    replace(&mut s, "PDF4All", "All4PDF");
    s.undo().unwrap();
    let state = s.state();
    let bytes = s.bytes().to_vec();
    let t = target(&mut s, "PDF4All");
    assert_eq!(
        s.replace_text(&t, "☃").unwrap_err().reason,
        "unsupported-encoding"
    );
    assert_eq!(s.state(), state);
    assert_eq!(s.bytes(), bytes);
    assert_eq!(s.replace_text(&t, "PDF4All").unwrap(), state);
    s.redo().unwrap();
}
#[test]
fn stale_targets_after_undo_and_across_sessions() {
    let mut s = open(fixture("hello.pdf"));
    let t = target(&mut s, "PDF4All");
    s.replace_text(&t, "All4PDF").unwrap();
    s.undo().unwrap();
    assert_eq!(
        s.replace_text(&t, "All4PDF").unwrap_err().reason,
        "stale-target"
    );
    let mut other = open(fixture("hello.pdf"));
    assert_eq!(
        other.replace_text(&t, "All4PDF").unwrap_err().reason,
        "stale-target"
    );
}
#[test]
fn late_save_receipt_marks_only_the_saved_history_node() {
    let mut s = open(fixture("hello.pdf"));
    replace(&mut s, "PDF4All", "All4PDF");
    let first = s.save().unwrap();
    replace(&mut s, "All4PDF", "PDF4All");
    s.acknowledge_saved(first.token).unwrap();
    assert!(s.state().dirty);
    s.undo().unwrap();
    assert!(!s.state().dirty);
    s.redo().unwrap();
    assert!(s.state().dirty);
    let second = s.save().unwrap();
    s.acknowledge_saved(second.token).unwrap();
    assert!(!s.state().dirty);
    assert_eq!(
        s.acknowledge_saved(first.token).unwrap_err().reason,
        "stale-save"
    );
    assert!(!s.state().dirty);
    let mut other = open(fixture("hello.pdf"));
    assert_eq!(
        other.acknowledge_saved(second.token).unwrap_err().reason,
        "stale-save"
    );
}
#[test]
fn branch_at_same_position_is_not_saved() {
    let mut s = open(fixture("hello.pdf"));
    replace(&mut s, "PDF4All", "All4PDF");
    let saved = s.save().unwrap();
    s.acknowledge_saved(saved.token).unwrap();
    s.undo().unwrap();
    replace(&mut s, "PDF4All", "All");
    assert!(s.state().dirty);
    assert!(!s.state().can_redo);
}
#[test]
fn history_budget_refusal_is_atomic() {
    let input = fixture("hello.pdf");
    let limits = SessionLimits {
        max_history_bytes: input.len(),
        ..SessionLimits::default()
    };
    let mut s = Session::open(input.clone(), limits).unwrap();
    let t = target(&mut s, "PDF4All");
    let state = s.state();
    assert_eq!(
        s.replace_text(&t, "All4PDF").unwrap_err().reason,
        "too-large"
    );
    assert_eq!(s.state(), state);
    assert_eq!(s.bytes(), input);
}
fn texts(s: &mut Session, page: usize) -> Vec<String> {
    s.text_targets(page)
        .unwrap()
        .into_iter()
        .map(|t| t.text)
        .collect()
}
#[test]
fn shared_stream_is_copied_for_the_edited_page_only() {
    let input = pdf(b"BT /F1 12 Tf 20 250 Td (ALPHA) Tj ET", "", true, true);
    let mut s = open(input.clone());
    let t = target(&mut s, "ALPHA");
    s.replace_text(&t, "BETA").unwrap();
    assert_eq!(texts(&mut s, 0), ["BETA"]);
    assert_eq!(texts(&mut s, 1), ["ALPHA"]);
    s.undo().unwrap();
    assert_eq!(s.bytes(), input);
    s.redo().unwrap();
    let saved = s.save().unwrap().bytes;
    let mut reopened = open(saved);
    assert_eq!(texts(&mut reopened, 0), ["BETA"]);
    assert_eq!(texts(&mut reopened, 1), ["ALPHA"]);
}
#[test]
fn form_placements_are_copied_on_write() {
    // A Form drawn twice on page 1: editing one placement leaves the other.
    let mut s = open(fixture("p0b/edit-shared.pdf"));
    assert_eq!(texts(&mut s, 0), ["ALPHA", "ALPHA", "NEIGHBOR"]);
    let t = target(&mut s, "ALPHA");
    s.replace_text(&t, "BRAV").unwrap();
    assert_eq!(texts(&mut s, 0), ["BRAV", "ALPHA", "NEIGHBOR"]);
    assert_eq!(texts(&mut s, 1), ["PAGE2"]);
    let mut reopened = open(s.save().unwrap().bytes);
    assert_eq!(texts(&mut reopened, 0), ["BRAV", "ALPHA", "NEIGHBOR"]);
    // The edited copy can be edited again after reopening.
    let t = target(&mut reopened, "BRAV");
    reopened.replace_text(&t, "CHARLIE").unwrap();
    assert_eq!(texts(&mut reopened, 0), ["CHARLIE", "ALPHA", "NEIGHBOR"]);
    // Inherited resources shared by both pages stay unchanged for page 2.
    let mut s = open(fixture("p3/edit-objects.pdf"));
    let before = texts(&mut s, 1);
    let t = s.text_targets(0).unwrap().into_iter().next().unwrap();
    s.replace_text(&t, "MROF").unwrap();
    assert_eq!(texts(&mut s, 0)[0], "MROF");
    assert_eq!(texts(&mut s, 1), before);
}
#[test]
fn following_text_must_not_shift() {
    let input = pdf(
        b"BT /F1 12 Tf 20 250 Td (ALPHA) Tj (HAL) Tj ET",
        "",
        false,
        false,
    );
    let mut s = open(input.clone());
    let t = target(&mut s, "ALPHA");
    assert_eq!(s.replace_text(&t, "HAL").unwrap_err().reason, "overflow");
    assert_eq!(s.bytes(), input);
}
#[test]
fn page_overflow_is_atomic() {
    let mut s = open(fixture("hello.pdf"));
    let t = target(&mut s, "PDF4All");
    let state = s.state();
    assert_eq!(
        s.replace_text(&t, &"P".repeat(100)).unwrap_err().reason,
        "overflow"
    );
    assert_eq!(s.state(), state);
}
#[test]
fn deletion_and_hex_string_reopen() {
    let input = pdf(
        b"BT /F1 12 Tf 20 250 Td <414C504841> Tj ET",
        "",
        false,
        false,
    );
    let mut s = open(input);
    replace(&mut s, "ALPHA", "");
    let saved = s.save().unwrap();
    let mut reopened = open(saved.bytes);
    assert!(reopened.text_targets(0).unwrap().is_empty());
    s.undo().unwrap();
    assert_eq!(target(&mut s, "ALPHA").text, "ALPHA");
}
#[test]
fn unsupported_document_cannot_enter_edit_session() {
    assert!(Session::open(fixture("encrypted.pdf"), SessionLimits::default()).is_err());
    let input = fixture("hello.pdf");
    let limits = SessionLimits {
        max_pdf_bytes: 10,
        ..SessionLimits::default()
    };
    assert!(Session::open(input, limits).is_err());
}

#[test]
fn canonical_font_can_write_characters_not_present_on_page() {
    let mut s = open(fixture("hello.pdf"));
    replace(&mut s, "PDF4All", "Hello world");
    let mut reopened = open(s.save().unwrap().bytes);
    assert_eq!(target(&mut reopened, "Hello world").text, "Hello world");
}

#[test]
fn exhausted_work_budget_still_allows_undo_and_saving_edits() {
    let limits = SessionLimits {
        max_operations: 2,
        ..SessionLimits::default()
    };
    let mut s = Session::open(fixture("hello.pdf"), limits).unwrap();
    replace(&mut s, "PDF4All", "Hello world");
    assert_eq!(s.render(0, 72.).err().unwrap().reason, "too-large");
    s.undo().unwrap();
    s.redo().unwrap();
    let saved = s.save().unwrap();
    s.acknowledge_saved(saved.token).unwrap();
    assert!(!s.state().dirty);
    assert_eq!(
        target(&mut open(saved.bytes), "Hello world").text,
        "Hello world"
    );
}

#[test]
fn view_only_signature_metadata_does_not_authorize_mutation_or_save() {
    let bytes = feature_pdf(
        "/Perms << /DocMDP 6 0 R >>",
        "",
        &["<< /Type /Sig /ByteRange [0 1 2 3] /Contents <00> >>"],
    );
    assert_eq!(refusal(bytes.clone()).reason, "unsupported-save");
    let mut session = Session::open_view(bytes.clone(), SessionLimits::default()).unwrap();
    let state = session.state();
    assert!(session.capabilities().pages[0].render_error.is_none());
    assert_eq!(
        session.capabilities().save_error.as_ref().unwrap().reason,
        "unsupported-save"
    );
    assert_eq!(
        session.text_targets(0).unwrap_err().reason,
        "unsupported-save"
    );
    assert!(session.render(0, 72.).is_ok());
    assert_eq!(session.save().err().unwrap().reason, "unsupported-save");
    assert_eq!(session.bytes(), bytes);
    assert_eq!(session.state(), state);
}

#[test]
fn unsupported_page_does_not_hide_supported_neighbor_or_return_partial_pixels() {
    let bytes = pdf(
        b"BT /F1 12 Tf 20 250 Td (ALPHA) Tj ET /Unknown cs",
        "",
        true,
        false,
    );
    let mut session = Session::open_view(bytes.clone(), SessionLimits::default()).unwrap();
    let capabilities = session.capabilities();
    assert!(capabilities.pages[0].render_error.is_some());
    assert!(capabilities.pages[1].render_error.is_none());
    assert!(capabilities.save_error.is_some());
    assert!(session.render(0, 72.).is_err());
    assert!(session.render(1, 72.).is_ok());
    assert_eq!(session.render(2, 72.).err().unwrap().reason, "invalid-page");
    assert_eq!(session.text_targets(2).unwrap_err().reason, "invalid-page");
    assert!(session.save().is_err());
    assert_eq!(session.bytes(), bytes);
}

#[test]
fn view_admission_does_not_claim_missing_annotation_appearances_are_rendered() {
    let bytes = feature_pdf(
        "",
        "/Annots [6 0 R]",
        &["<< /Subtype /Highlight /Rect [0 0 10 10] >>"],
    );
    let mut session = Session::open_view(bytes, SessionLimits::default()).unwrap();
    assert_eq!(
        session.capabilities().pages[0]
            .render_error
            .as_ref()
            .unwrap()
            .reason,
        "unsupported-annotation"
    );
    assert_eq!(
        session.render(0, 72.).err().unwrap().reason,
        "unsupported-annotation"
    );
    assert!(session.save().is_err());
}

#[test]
fn capability_state_follows_deleted_text_undo_redo_and_fresh_reopen() {
    let mut session = Session::open_view(
        pdf(b"BT /F1 12 Tf 20 250 Td (ALPHA) Tj ET", "", false, false),
        SessionLimits::default(),
    )
    .unwrap();
    assert_eq!(session.capabilities().pages[0].text_target_count, 1);
    replace(&mut session, "ALPHA", "");
    assert_eq!(session.capabilities().pages[0].text_target_count, 0);
    session.undo().unwrap();
    assert_eq!(session.capabilities().pages[0].text_target_count, 1);
    session.redo().unwrap();
    assert_eq!(session.capabilities().pages[0].text_target_count, 0);
    let saved = session.save().unwrap();
    let reopened = Session::open_view(saved.bytes, SessionLimits::default()).unwrap();
    assert_eq!(reopened.capabilities().pages[0].text_target_count, 0);
    assert!(reopened.capabilities().save_error.is_none());
}

#[test]
fn view_admission_keeps_invalid_document_and_limits_as_hard_failures() {
    assert!(Session::open_view(vec![1, 2, 3], SessionLimits::default()).is_err());
    let bytes = pdf(b"", "", true, false);
    assert!(
        Session::open_view(
            bytes.clone(),
            SessionLimits {
                max_pages: 1,
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(
        Session::open_view(
            bytes,
            SessionLimits {
                max_pdf_bytes: 1,
                ..Default::default()
            }
        )
        .is_err()
    );
}

#[test]
fn explicit_default_rendering_intent_preserves_device_color_pixels_and_edits() {
    let base = b"0.2 0.4 0.6 rg 20 30 80 40 re f BT /F1 12 Tf 20 250 Td (ALPHA) Tj ET";
    let content = [
        b"/RelativeColorimetric ri q /RelativeColorimetric ri Q ".as_slice(),
        base,
    ]
    .concat();
    let mut original = open(pdf(base, "", false, false));
    let mut explicit = open(pdf(&content, "", false, false));
    for dpi in [72., 144.] {
        assert_eq!(
            original.render(0, dpi).unwrap().pixels(),
            explicit.render(0, dpi).unwrap().pixels()
        );
    }
    replace(&mut explicit, "ALPHA", "BETA");
    let bytes = explicit.save().unwrap().bytes;
    assert!(
        bytes
            .windows(b"/RelativeColorimetric ri".len())
            .any(|w| w == b"/RelativeColorimetric ri")
    );
    assert_eq!(target(&mut open(bytes), "BETA").text, "BETA");
}

#[test]
fn nondefault_and_malformed_intents_remain_explicit_render_refusals() {
    for instruction in [
        "/Perceptual ri",
        "/AbsoluteColorimetric ri",
        "/Saturation ri",
        "/Unknown ri",
        "1 ri",
    ] {
        let bytes = pdf(
            format!("{instruction} 0 0 10 10 re f").as_bytes(),
            "",
            false,
            false,
        );
        let mut session = Session::open_view(bytes, SessionLimits::default()).unwrap();
        assert!(
            session.capabilities().pages[0].render_error.is_some(),
            "{instruction}"
        );
        assert!(session.render(0, 72.).is_err(), "{instruction}");
    }
}

fn arial_pdf(name: &str, encoding: &str, width: u32) -> Vec<u8> {
    let content = b"BT /F1 20 Tf 20 250 Td (AAA) Tj ET";
    pdf_objects(vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_vec(),
        format!("<< /Type /Font /Subtype /TrueType /BaseFont /{name} /FontDescriptor 6 0 R /FirstChar 65 /LastChar 65 /Widths [{width}] /Encoding {encoding} >>").into_bytes(),
        [format!("<< /Length {} >>\nstream\n",content.len()).as_bytes(),content,b"\nendstream"].concat(),
        format!("<< /Type /FontDescriptor /FontName /{name} /Flags 32 /FontBBox [-200 -300 1100 1100] /ItalicAngle 0 /Ascent 1000 /Descent -300 /CapHeight 700 /StemV 80 >>").into_bytes(),
    ])
}

#[test]
fn pinned_arial_styles_are_distinct_and_display_only() {
    let mut rendered = Vec::new();
    for name in ["ArialMT", "Arial-BoldMT", "Arial-BoldItalicMT"] {
        let mut session = Session::open_view(
            arial_pdf(name, "/WinAnsiEncoding", 800),
            SessionLimits::default(),
        )
        .unwrap();
        let caps = session.capabilities();
        assert!(
            caps.pages[0].render_error.is_none(),
            "{name}: {:?}",
            caps.pages[0].render_error
        );
        assert!(
            caps.pages[0]
                .edit_error
                .as_ref()
                .unwrap()
                .detail
                .contains("embedding")
        );
        assert!(session.text_targets(0).is_err());
        let pixels = session.render(0, 144.).unwrap().pixels().to_vec();
        assert!(
            !rendered.contains(&pixels),
            "style did not change glyph outlines: {name}"
        );
        let saved = session.save().unwrap();
        let mut reopened = Session::open_view(saved.bytes, SessionLimits::default()).unwrap();
        assert_eq!(pixels, reopened.render(0, 144.).unwrap().pixels());
        rendered.push(pixels);
    }
}

#[test]
fn substitution_uses_pdf_widths_and_declared_glyph_names_not_unicode_inversion() {
    use tandem_pdf_engine::page_glyph::{
        PageGlyphLimits, PageGlyphSession, PageGlyphSessionLimits, PageOutlinePolicy,
    };
    for width in [400, 900] {
        let bytes = arial_pdf(
            "ArialMT",
            "<< /BaseEncoding /WinAnsiEncoding /Differences [65 /B] >>",
            width,
        );
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        let limits = PageGlyphSessionLimits {
            outline_policy: PageOutlinePolicy::AllowPinnedSubstitutes,
            ..Default::default()
        };
        let program = PageGlyphSession::new(&doc, limits)
            .unwrap()
            .trace_page(0, PageGlyphLimits::default())
            .unwrap();
        let text = program.page_program().program().text();
        assert_eq!(text.text(), "BBB");
        let codes = text.codes();
        assert_eq!(codes[0].code(), 65);
        let a = codes[0].transform().coefficients();
        let b = codes[1].transform().coefficients();
        assert!((b[4] - a[4] - f64::from(width) * 20. / 1000.).abs() < 1e-9);
    }
}

#[test]
fn substitute_allowlist_rejects_implicit_or_guessed_faces() {
    for name in ["Arial-ItalicMT", "UnknownArial", "AAAAAA+ArialMT"] {
        let mut session = Session::open_view(
            arial_pdf(name, "/WinAnsiEncoding", 800),
            SessionLimits::default(),
        )
        .unwrap();
        assert!(session.render(0, 72.).is_err(), "{name}");
    }
}

#[test]
fn default_flatness_preserves_curves_and_rejects_unimplemented_precision() {
    let curve = b"10 10 m 30 180 160 30 180 180 c 180 10 l h f";
    let expected = open(pdf(curve, "", false, false))
        .render(0, 144.)
        .unwrap()
        .pixels()
        .to_vec();
    for setting in ["0", "1", "0.0", "1.0"] {
        let content = [format!("{setting} i ").as_bytes(), curve].concat();
        assert_eq!(
            expected,
            open(pdf(&content, "", false, false))
                .render(0, 144.)
                .unwrap()
                .pixels()
        );
    }
    for setting in ["-1", "0.001", "1000", "/Name"] {
        let content = [format!("{setting} i ").as_bytes(), curve].concat();
        assert!(Session::open(pdf(&content, "", false, false), SessionLimits::default()).is_err());
    }
}

#[test]
fn indexed_color_key_leaves_existing_page_paint_visible() {
    let content = b"0 0 1 rg 0 0 300 300 re f q 100 0 0 100 0 0 cm /Im Do Q";
    let bytes=pdf_objects(vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Resources << /XObject << /Im 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        [format!("<< /Length {} >>\nstream\n",content.len()).as_bytes(),content,b"\nendstream"].concat(),
        b"<< /Type /XObject /Subtype /Image /Width 2 /Height 1 /BitsPerComponent 8 /ColorSpace [/Indexed /DeviceRGB 1 <ff000000ff00>] /Decode [0 255] /Mask [1 1] /Length 2 >>\nstream\n\x00\x01\nendstream".to_vec(),
    ]);
    let mut session = open(bytes);
    let image = session.render(0, 72.).unwrap();
    let pixel = |x: usize, y: usize| &image.pixels()[(y * 300 + x) * 4..(y * 300 + x + 1) * 4];
    assert_eq!(pixel(25, 250), [255, 0, 0, 255]);
    assert_eq!(pixel(75, 250), [0, 0, 255, 255]);
    assert_eq!(pixel(150, 250), [0, 0, 255, 255]);
    let bytes = session.save().unwrap().bytes;
    assert_eq!(image.pixels(), open(bytes).render(0, 72.).unwrap().pixels());
}

fn array_pdf(parts: &[&[u8]], indirect: bool, compressed: bool) -> Vec<u8> {
    let refs = (0..parts.len())
        .map(|i| format!("{} 0 R", 5 + i))
        .collect::<Vec<_>>()
        .join(" ");
    let contents = if indirect {
        format!("{} 0 R", 5 + parts.len())
    } else {
        format!("[{refs}]")
    };
    let mut objects=vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Resources << /Font << /F1 4 0 R >> >> /Contents {contents} >>").into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
    ];
    for part in parts {
        let data = if compressed {
            miniz_oxide::deflate::compress_to_vec_zlib(part, 6)
        } else {
            part.to_vec()
        };
        let filter = if compressed {
            "/Filter /FlateDecode"
        } else {
            ""
        };
        objects.push(
            [
                format!("<< /Length {} {filter} >>\nstream\n", data.len()).as_bytes(),
                &data,
                b"\nendstream",
            ]
            .concat(),
        );
    }
    if indirect {
        objects.push(format!("[{refs}]").into_bytes());
    }
    pdf_objects(objects)
}

#[test]
fn content_array_edit_changes_only_member_bytes_and_preserves_boundary_padding() {
    for indirect in [false, true] {
        for compressed in [false, true] {
            let bytes = array_pdf(
                &[
                    b"BT /F1 12 Tf 20 250 Td",
                    b"(ALPHA) Tj ET",
                    b"BT /F1 12 Tf 20 200 Td (NEIGHBOR) Tj ET",
                ],
                indirect,
                compressed,
            );
            let mut session = open(bytes.clone());
            let pixels = session.render(0, 72.).unwrap().pixels().to_vec();
            replace(&mut session, "ALPHA", "BETA");
            let edited = session.render(0, 72.).unwrap().pixels().to_vec();
            assert_ne!(pixels, edited);
            session.undo().unwrap();
            assert_eq!(pixels, session.render(0, 72.).unwrap().pixels());
            session.redo().unwrap();
            assert_eq!(edited, session.render(0, 72.).unwrap().pixels());
            let saved = session.save().unwrap().bytes;
            let before = Document::open(&bytes, DocumentLimits::default()).unwrap();
            let after = Document::open(&saved, DocumentLimits::default()).unwrap();
            for object in before.objects().filter(|o| o.id.number() != 6) {
                assert_eq!(object.value, after.object(object.id).unwrap().value);
                assert_eq!(
                    before.stream_data(object.id).unwrap(),
                    after.stream_data(object.id).unwrap()
                );
            }
            assert_eq!(
                decode_document_stream(
                    &after,
                    ObjectId::new(6, 0).unwrap(),
                    StreamLimits::default()
                )
                .unwrap(),
                b"<42455441> Tj ET"
            );
            let mut reopened = open(saved);
            replace(&mut reopened, "BETA", "GAMMA");
            assert_eq!(target(&mut reopened, "NEIGHBOR").text, "NEIGHBOR");
        }
    }
}

#[test]
fn tj_array_run_can_be_replaced_and_saved_without_moving_unrelated_text() {
    let mut session = open(pdf(
        b"BT /F1 12 Tf 20 250 Td [(AL) 30 (PHA)] TJ ET BT /F1 12 Tf 20 200 Td (NEIGHBOR) Tj ET",
        "",
        false,
        false,
    ));
    replace(&mut session, "ALPHA", "HELLO");
    let mut reopened = open(session.save().unwrap().bytes);
    assert_eq!(target(&mut reopened, "HELLO").text, "HELLO");
    assert_eq!(target(&mut reopened, "NEIGHBOR").text, "NEIGHBOR");
    replace(&mut reopened, "HELLO", "WORLD");
}

#[test]
fn tj_following_text_guard_preserves_bytes_dirty_and_redo_on_failure() {
    let mut session=open(pdf(b"BT /F1 12 Tf 20 250 Td [(AL) 30 (PHA)] TJ (TAIL) Tj ET BT /F1 12 Tf 20 200 Td (NEIGHBOR) Tj ET","",false,false));
    replace(&mut session, "NEIGHBOR", "NEXT");
    session.undo().unwrap();
    let bytes = session.bytes().to_vec();
    let state = session.state();
    let t = target(&mut session, "ALPHA");
    assert_eq!(
        session.replace_text(&t, "BETA").unwrap_err().reason,
        "overflow"
    );
    assert_eq!(session.bytes(), bytes);
    assert_eq!(session.state(), state);
    session.redo().unwrap();
    assert_eq!(target(&mut session, "NEXT").text, "NEXT");
}

#[test]
fn cross_stream_show_is_not_a_local_target_but_later_complete_show_is() {
    let bytes = array_pdf(
        &[
            b"BT /F1 12 Tf 20 250 Td (AL",
            b"PHA) Tj ET BT /F1 12 Tf 20 200 Td (NEIGHBOR) Tj ET",
        ],
        false,
        false,
    );
    let mut session = open(bytes);
    let items = session.text_targets(0).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].text, "NEIGHBOR");
    replace(&mut session, "NEIGHBOR", "NEXT");
    assert_eq!(session.text_targets(0).unwrap().len(), 1);
}

#[test]
fn repeated_stream_in_content_array_still_requires_copy_on_write() {
    // Construct the repeated placement explicitly instead of rewriting offsets.
    let part = b"BT /F1 12 Tf 20 250 Td (ALPHA) Tj ET";
    let bytes=pdf_objects(vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Resources << /Font << /F1 4 0 R >> >> /Contents [5 0 R 5 0 R] >>".to_vec(),b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),[format!("<< /Length {} >>\nstream\n",part.len()).as_bytes(),part,b"\nendstream"].concat()]);
    let mut session = open(bytes.clone());
    assert_eq!(session.text_targets(0).unwrap_err().reason, "not-candidate");
    assert_eq!(session.bytes(), bytes);
}
fn view(content: &[u8]) -> Session {
    Session::open_view(pdf(content, "", false, false), SessionLimits::default()).unwrap()
}
/// Red channel of the 72-DPI pixel whose center is at PDF user point (x, y).
fn ink(s: &mut Session, points: &[(u32, u32)]) -> Vec<bool> {
    let image = s.render(0, 72.).unwrap();
    points
        .iter()
        .map(|&(x, y)| image.pixels()[(299 - y) as usize * image.stride() + x as usize * 4] < 128)
        .collect()
}
#[test]
fn tag_only_marked_content_renders_like_untagged_content() {
    let plain = b"0 0 1 rg 10 10 50 50 re f BT /F1 12 Tf 20 250 Td (ALPHA) Tj ET";
    let tagged = b"/Artifact BMC 0 0 1 rg 10 10 50 50 re f /Inner BMC EMC EMC /P MP BT /Span BMC /F1 12 Tf 20 250 Td (ALPHA) Tj EMC ET";
    let expected = view(plain).render(0, 72.).unwrap().pixels().to_vec();
    let mut s = view(tagged);
    assert_eq!(s.render(0, 72.).unwrap().pixels(), expected);
    assert!(s.text_targets(0).unwrap().iter().any(|t| t.text == "ALPHA"));
    for content in [
        &b"EMC 0 g 10 10 50 50 re f"[..],
        b"/Open BMC 0 g 10 10 50 50 re f",
        b"/P <</MCID 0>> BDC 0 g 10 10 50 50 re f",
    ] {
        let s = view(content);
        assert!(
            s.capabilities().pages[0].render_error.is_some(),
            "{content:?}"
        );
    }
}
#[test]
fn dashed_strokes_follow_array_phase_and_restart_per_subpath() {
    let row = |x0: u32| (0..6).map(|i| (x0 + i * 10, 150)).collect::<Vec<_>>();
    // [20 10] from x=0: on 0-20, off 20-30, on 30-50, off 50-60.
    let mut s = view(b"0 G 10 w [20 10] 0 d 0 150 m 300 150 l S");
    assert_eq!(ink(&mut s, &row(5)), [true, true, false, true, true, false]);
    // Phase 5 shifts every boundary five units left.
    let mut s = view(b"0 G 10 w [20 10] 5 d 0 150 m 300 150 l S");
    assert_eq!(ink(&mut s, &row(2)), [true, true, false, true, true, false]);
    // An odd array repeats: [10] is 10 on, 10 off.
    let mut s = view(b"0 G 10 w [10] 0 d 0 150 m 300 150 l S");
    assert_eq!(
        ink(&mut s, &row(5)),
        [true, false, true, false, true, false]
    );
    // Each subpath restarts the pattern at the phase.
    let mut s = view(b"0 G 10 w [20 10] 0 d 0 150 m 25 150 l 100 150 m 300 150 l S");
    assert_eq!(
        ink(
            &mut s,
            &[(5, 150), (22, 150), (105, 150), (125, 150), (135, 150)]
        ),
        // 0-20 on, 20-25 off; the second subpath starts on again at 100.
        [true, false, true, false, true]
    );
    // Closed rectangles dash their closing edge (drawn downward from y=250 at
    // path length 600); curves split into dash pieces.
    let mut s = view(b"0 G 4 w [30 20] 0 d 50 50 200 200 re S 150 20 m 200 20 250 70 250 120 c S");
    assert_eq!(
        ink(&mut s, &[(60, 50), (90, 50), (50, 240), (50, 215)]),
        [true, false, true, false]
    );
    let image = s.render(0, 72.).unwrap();
    let painted = image.pixels().chunks(4).filter(|p| p[0] < 128).count();
    let mut solid = view(b"0 G 4 w 50 50 200 200 re S 150 20 m 200 20 250 70 250 120 c S");
    let full = solid.render(0, 72.).unwrap();
    let solid_painted = full.pixels().chunks(4).filter(|p| p[0] < 128).count();
    assert!(painted * 10 < solid_painted * 8 && painted * 10 > solid_painted * 4);
    for content in [
        &b"0 G 10 w [5 -1] 0 d 0 150 m 300 150 l S"[..],
        b"0 G 10 w [0 0] 0 d 0 150 m 300 150 l S",
        b"0 G 10 w 1 J [0 10] 0 d 0 150 m 300 150 l S",
    ] {
        let s = view(content);
        assert!(
            s.capabilities().pages[0].render_error.is_some(),
            "{content:?}"
        );
    }
}
#[test]
fn many_small_clips_render_with_bounded_masks() {
    // 400 full-page masks at 72 DPI would need ~550 MiB; cropped masks are tiny.
    // Each clip nests inside a second small clip to exercise parent bounds.
    let mut content = Vec::new();
    for i in 0..400u32 {
        let (x, y) = (5 + (i % 20) * 14, 5 + (i / 20) * 14);
        content.extend(
            format!("q {x} {y} 8 8 re W n q {x} {y} 6 6 re W n 0 g {x} {y} 10 10 re f Q Q ")
                .bytes(),
        );
    }
    let mut s = view(&content);
    // Inside the clip, outside it (but inside the unclipped fill), and between cells.
    assert_eq!(
        ink(&mut s, &[(7, 7), (13, 13), (21, 21), (17, 17)]),
        [true, false, true, false]
    );
}

fn stream(dict: &str, content: &str) -> String {
    format!(
        "<< /Length {} {dict} >>\nstream\n{content}\nendstream",
        content.len()
    )
}
fn annotated(annot: &str, streams: &[String]) -> Session {
    let mut extra = vec![annot];
    extra.extend(streams.iter().map(String::as_str));
    Session::open_view(
        feature_pdf("", "/Annots [6 0 R]", &extra),
        SessionLimits::default(),
    )
    .unwrap()
}
#[test]
fn annotation_appearance_maps_bbox_and_matrix_onto_rect() {
    let square = stream("/Subtype /Form /BBox [0 0 10 10]", "0 0 10 10 re f");
    let mut s = annotated(
        "<< /Subtype /Square /Rect [150 100 100 150] /AP << /N 7 0 R >> >>",
        &[square],
    );
    assert!(s.capabilities().pages[0].render_error.is_none());
    assert_eq!(
        ink(
            &mut s,
            &[(125, 125), (102, 148), (95, 125), (155, 125), (125, 95)]
        ),
        [true, true, false, false, false]
    );
    // A quarter turn maps BBox [0 0 10 10] to [-10 0 0 10]; its left half
    // lands in the lower half of Rect.
    let half = stream(
        "/Subtype /Form /BBox [0 0 10 10] /Matrix [0 1 -1 0 0 0]",
        "0 0 5 10 re f",
    );
    let mut s = annotated(
        "<< /Subtype /Square /Rect [100 100 150 150] /AP << /N 7 0 R >> >>",
        &[half],
    );
    assert_eq!(ink(&mut s, &[(125, 110), (125, 140)]), [true, false]);
    // Content outside BBox is clipped.
    let spill = stream("/Subtype /Form /BBox [0 0 10 10]", "-10 -10 30 30 re f");
    let mut s = annotated(
        "<< /Subtype /Square /Rect [100 100 150 150] /AP << /N 7 0 R >> >>",
        &[spill],
    );
    assert_eq!(
        ink(&mut s, &[(125, 125), (95, 125), (155, 155)]),
        [true, false, false]
    );
}
#[test]
fn appearance_state_selects_the_normal_stream() {
    let on = stream("/Subtype /Form /BBox [0 0 10 10]", "0 0 10 10 re f");
    let off = stream("/Subtype /Form /BBox [0 0 10 10]", "");
    for (state, expected) in [("/On", true), ("/Off", false), ("/Missing", false)] {
        let annot = format!(
            "<< /Subtype /Square /Rect [100 100 150 150] /AS {state} /AP << /N << /On 7 0 R /Off 8 0 R >> >> >>"
        );
        let mut s = annotated(&annot, &[on.clone(), off.clone()]);
        assert_eq!(ink(&mut s, &[(125, 125)]), [expected], "{state}");
    }
}
#[test]
fn hidden_and_noview_annotations_draw_nothing_even_without_appearance() {
    let square = stream("/Subtype /Form /BBox [0 0 10 10]", "0 0 10 10 re f");
    for flags in [2, 32, 34] {
        let annot =
            format!("<< /Subtype /Square /F {flags} /Rect [100 100 150 150] /AP << /N 7 0 R >> >>");
        let mut s = annotated(&annot, std::slice::from_ref(&square));
        assert_eq!(ink(&mut s, &[(125, 125)]), [false], "{flags}");
        let annot = format!("<< /Subtype /Highlight /F {flags} /Rect [100 100 150 150] >>");
        assert!(
            annotated(&annot, &[]).capabilities().pages[0]
                .render_error
                .is_none()
        );
    }
}
#[test]
fn annotation_appearance_starts_from_default_graphics_state() {
    // Page content leaves a light fill and a large line width set; the
    // appearance must still paint in black with a 1-unit default stroke.
    let contents = "0.9 g 0.9 G 40 w 1 0 0 1 50 0 cm";
    let bytes = pdf_objects(vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R /Annots [5 0 R] >>"
            .to_vec(),
        stream("", contents).into_bytes(),
        b"<< /Subtype /Square /Rect [100 100 150 150] /AP << /N 6 0 R >> >>".to_vec(),
        stream(
            "/Subtype /Form /BBox [0 0 50 50]",
            "0 0 20 50 re f 35.5 0 m 35.5 50 l S",
        )
        .into_bytes(),
    ]);
    let mut s = Session::open_view(bytes, SessionLimits::default()).unwrap();
    assert_eq!(
        ink(&mut s, &[(110, 125), (135, 125), (130, 125), (140, 125)]),
        [true, true, false, false]
    );
}
#[test]
fn form_fields_are_view_only_and_need_appearances_renders() {
    let square = stream("/Subtype /Form /BBox [0 0 10 10]", "0 0 10 10 re f");
    let bytes = feature_pdf(
        "/AcroForm << /Fields [6 0 R] >>",
        "/Annots [6 0 R]",
        &[
            "<< /FT /Tx /T (name) /Subtype /Widget /Rect [100 100 150 150] /AP << /N 7 0 R >> >>",
            &square,
        ],
    );
    let mut s = Session::open_view(bytes, SessionLimits::default()).unwrap();
    assert_eq!(
        s.capabilities().save_error.as_ref().unwrap().reason,
        "unsupported-forms"
    );
    assert_eq!(
        s.capabilities().pages[0]
            .edit_error
            .as_ref()
            .unwrap()
            .reason,
        "unsupported-forms"
    );
    assert_eq!(s.capabilities().pages[0].text_target_count, 0);
    assert_eq!(ink(&mut s, &[(125, 125)]), [true]);
    assert_eq!(s.save().err().unwrap().reason, "unsupported-forms");
    // Widgets outside the field tree still draw, as in Acrobat and pdf.js
    // (MuPDF hides them).
    let orphan = feature_pdf(
        "/AcroForm << /Fields [8 0 R] >>",
        "/Annots [6 0 R]",
        &[
            "<< /FT /Tx /T (name) /Subtype /Widget /Rect [100 100 150 150] /AP << /N 7 0 R >> >>",
            &square,
            "<< /FT /Tx /T (other) >>",
        ],
    );
    let mut s = Session::open_view(orphan, SessionLimits::default()).unwrap();
    assert_eq!(ink(&mut s, &[(125, 125)]), [true]);
    let bytes = feature_pdf(
        "/AcroForm << /Fields [6 0 R] /NeedAppearances true >>",
        "",
        &["<< /FT /Tx >>"],
    );
    // NeedAppearances regenerates field appearances at display time; the
    // document still renders and stays view-only.
    let s = Session::open_view(bytes, SessionLimits::default()).unwrap();
    assert!(s.capabilities().pages[0].render_error.is_none());
    assert_eq!(
        s.capabilities().save_error.as_ref().unwrap().reason,
        "unsupported-forms"
    );
}
#[test]
fn marked_content_properties_render_but_withhold_text_edits() {
    let plain = b"0 0 1 rg 10 10 50 50 re f BT /F1 12 Tf 20 250 Td (ALPHA) Tj ET";
    let marked = b"/Artifact <</Type /Pagination>> BDC 0 0 1 rg 10 10 50 50 re f EMC /P <</MCID 0>> DP BT /Span <</ActualText (BETA)>> BDC /F1 12 Tf 20 250 Td (ALPHA) Tj EMC ET";
    let expected = view(plain).render(0, 72.).unwrap().pixels().to_vec();
    let mut s = view(marked);
    assert_eq!(s.render(0, 72.).unwrap().pixels(), expected);
    let page = &s.capabilities().pages[0];
    assert!(page.render_error.is_none());
    assert!(
        page.edit_error
            .as_ref()
            .unwrap()
            .detail
            .contains("ActualText")
    );
    assert!(s.capabilities().save_error.is_none());
    assert_eq!(s.text_targets(0).unwrap_err().reason, "not-candidate");
}
#[test]
fn identity_h_cid_fonts_render_with_composite_glyphs_but_withhold_edits() {
    // Hebrew and Arabic Identity-H CIDFontType2 subsets; the Arabic face uses
    // composite glyphs and its ToUnicode contains a malformed unused entry.
    for name in ["rtl.pdf", "cjk.pdf"] {
        let mut s = Session::open_view(fixture(name), SessionLimits::default()).unwrap();
        let page = &s.capabilities().pages[0];
        assert!(page.render_error.is_none(), "{name}");
        assert!(
            page.edit_error
                .as_ref()
                .unwrap()
                .detail
                .contains("composite (CID) font")
        );
        let image = s.render(0, 72.).unwrap();
        assert!(
            image.pixels().chunks(4).any(|p| p[0] < 128),
            "{name} draws ink"
        );
    }
}

/// Page 1 places Form A twice; A places Form B, which shows INNER. A's
/// /Resources (8) is shared with page 2, which places B directly. Page 1's
/// own /Resources is indirect (9), and object 10 is unreachable from the start.
fn nested_pdf(a_resources: &str) -> Vec<u8> {
    let page1 = "q /A Do Q q 1 0 0 1 0 -100 cm /A Do Q BT /F1 12 Tf 20 20 Td (NEIGHBOR) Tj ET";
    pdf_objects(vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 11 0 R] /Count 2 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Resources 9 0 R /Contents 5 0 R >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
        stream("", page1).into_bytes(),
        stream(&format!("/Type /XObject /Subtype /Form /BBox [0 0 300 300] {a_resources}"), "/B Do").into_bytes(),
        stream("/Type /XObject /Subtype /Form /BBox [0 0 300 300] /Resources << /Font << /F1 4 0 R >> >>", "BT /F1 12 Tf 20 250 Td (INNER) Tj ET").into_bytes(),
        b"<< /XObject << /B 7 0 R >> >>".to_vec(),
        b"<< /Font << /F1 4 0 R >> /XObject << /A 6 0 R /B 7 0 R >> >>".to_vec(),
        b"(unreachable)".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Resources 8 0 R /Contents 12 0 R >>".to_vec(),
        stream("", "/B Do").into_bytes(),
    ])
}
fn object(bytes: &[u8], number: u32) -> Option<tandem_pdf_engine::object::Object> {
    let doc = Document::open(bytes, DocumentLimits::default()).unwrap();
    doc.object(ObjectId::new(number, 0).unwrap())
        .ok()
        .map(|o| o.value.clone())
}
#[test]
fn nested_form_placements_are_copied_on_write() {
    let input = nested_pdf("/Resources 8 0 R");
    let mut s = open(input.clone());
    assert_eq!(texts(&mut s, 0), ["INNER", "INNER", "NEIGHBOR"]);
    assert_eq!(texts(&mut s, 1), ["INNER"]);
    let t = target(&mut s, "INNER");
    s.replace_text(&t, "OUTER").unwrap();
    assert_eq!(texts(&mut s, 0), ["OUTER", "INNER", "NEIGHBOR"]);
    assert_eq!(texts(&mut s, 1), ["INNER"]);
    // The originals and the shared resource dictionary are untouched.
    for number in [6, 7, 8] {
        assert_eq!(object(s.bytes(), number), object(&input, number));
    }
    s.undo().unwrap();
    assert_eq!(s.bytes(), input);
    s.redo().unwrap();
    // Save, reopen in a fresh session and edit the copied chain again.
    let mut reopened = open(s.save().unwrap().bytes);
    assert_eq!(texts(&mut reopened, 0), ["OUTER", "INNER", "NEIGHBOR"]);
    let t = target(&mut reopened, "OUTER");
    reopened.replace_text(&t, "AGAIN").unwrap();
    assert_eq!(texts(&mut reopened, 0), ["AGAIN", "INNER", "NEIGHBOR"]);
    assert_eq!(texts(&mut reopened, 1), ["INNER"]);
    // The second placement can be edited independently too.
    let t = reopened.text_targets(0).unwrap().remove(1);
    reopened.replace_text(&t, "THIRD").unwrap();
    assert_eq!(texts(&mut reopened, 0), ["AGAIN", "THIRD", "NEIGHBOR"]);
    assert_eq!(texts(&mut reopened, 1), ["INNER"]);
}
#[test]
fn nested_form_without_own_resources_is_not_a_target() {
    // A inherits page 1's resources, so its copy would have nowhere private
    // to name B's copy; its text stays view-only.
    let mut s = open(nested_pdf(""));
    assert_eq!(texts(&mut s, 0), ["NEIGHBOR"]);
    assert_eq!(texts(&mut s, 1), ["INNER"]);
}
#[test]
fn objects_orphaned_by_copy_on_write_are_not_retained() {
    let input = nested_pdf("/Resources 8 0 R");
    assert!(object(&input, 9).is_some() && object(&input, 10).is_some());
    let mut s = open(input.clone());
    // An unedited save collects the unreachable object, like the app's
    // MuPDF save (garbage=yes).
    let saved = s.save().unwrap().bytes;
    assert!(object(&saved, 10).is_none());
    assert_eq!(object(&saved, 9), object(&input, 9));
    let t = target(&mut s, "INNER");
    s.replace_text(&t, "OUTER").unwrap();
    // Page 1 now owns a direct copy of its resources, so 9 is orphaned.
    for bytes in [s.bytes().to_vec(), s.save().unwrap().bytes] {
        assert!(object(&bytes, 9).is_none());
        assert!(object(&bytes, 10).is_none());
        assert!(object(&bytes, 8).is_some());
    }
}
#[test]
fn history_retains_deltas_not_whole_snapshots() {
    // A 2 MiB metadata stream makes each whole-file snapshot expensive.
    let big = "x".repeat(2 << 20);
    let input = pdf_objects(vec![
        b"<< /Type /Catalog /Pages 2 0 R /Metadata 6 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
        stream("", "BT /F1 12 Tf 20 250 Td (ALPHA) Tj ET").into_bytes(),
        stream("/Type /Metadata /Subtype /XML", &big).into_bytes(),
    ]);
    // Room for the original, the current node and small deltas only: whole
    // snapshots would be refused at the second edit.
    let limits = SessionLimits {
        max_history_bytes: input.len() * 2 + (64 << 10),
        ..SessionLimits::default()
    };
    let mut s = Session::open(input.clone(), limits).unwrap();
    let words = ["BETA", "GAMMA", "DELTA", "ALPHA", "OMEGA", "SIGMA"];
    let mut published = vec![input.clone()];
    let mut old = "ALPHA";
    for (step, word) in words.iter().cycle().take(20).enumerate() {
        let t = target(&mut s, old);
        s.replace_text(&t, word).unwrap();
        old = word;
        published.push(s.bytes().to_vec());
        assert!(
            s.retained_history_bytes() <= limits.max_history_bytes,
            "step {step}"
        );
    }
    // Undo and redo reproduce every published node exactly.
    for bytes in published.iter().rev().skip(1) {
        s.undo().unwrap();
        assert_eq!(s.bytes(), bytes.as_slice());
    }
    assert!(!s.state().can_undo);
    for bytes in &published[1..] {
        s.redo().unwrap();
        assert_eq!(s.bytes(), bytes.as_slice());
    }
    // Branching from the middle discards the redo deltas.
    for _ in 0..10 {
        s.undo().unwrap();
    }
    let before = s.retained_history_bytes();
    let t = s.text_targets(0).unwrap().remove(0);
    s.replace_text(&t, "BRANCH").unwrap();
    assert!(!s.state().can_redo);
    assert!(s.retained_history_bytes() < before + (16 << 10));
    let mut reopened = open(s.save().unwrap().bytes);
    assert_eq!(texts(&mut reopened, 0), ["BRANCH"]);
}
#[test]
fn fixture_form_nested_in_form_is_edited_through_a_copied_chain() {
    // Fm1 is placed twice by page 1, once inside Fm2, and once by page 2.
    // The app's MuPDF path stops at depth-1 Forms; this engine copies the
    // chain (an intentional difference, recorded in WORKFLOW-STATUS.md).
    let mut s = open(fixture("p3/targets-streams.pdf"));
    assert_eq!(texts(&mut s, 0), ["SPLIT", "FORM", "FORM", "FORM", "NEST"]);
    let t = s.text_targets(0).unwrap().remove(3);
    s.replace_text(&t, "MROF").unwrap();
    assert_eq!(texts(&mut s, 0), ["SPLIT", "FORM", "FORM", "MROF", "NEST"]);
    assert_eq!(texts(&mut s, 1), ["FORM", "PAGE2"]);
    let mut reopened = open(s.save().unwrap().bytes);
    assert_eq!(
        texts(&mut reopened, 0),
        ["SPLIT", "FORM", "FORM", "MROF", "NEST"]
    );
    assert_eq!(texts(&mut reopened, 1), ["FORM", "PAGE2"]);
}
fn object_count(bytes: &[u8]) -> usize {
    Document::open(bytes, DocumentLimits::default())
        .unwrap()
        .object_count()
}
#[test]
fn repeated_edits_of_a_form_copy_edit_it_in_place() {
    let mut s = open(fixture("p0b/edit-shared.pdf"));
    let t = target(&mut s, "ALPHA");
    s.replace_text(&t, "BRAV").unwrap();
    let (count, size) = (object_count(s.bytes()), s.bytes().len());
    // The copy is now drawn and named once, so later edits reuse it.
    for word in ["CHAR", "DELT", "ECHO", "FOXT", "GOLF"] {
        let t = s.text_targets(0).unwrap().remove(0);
        s.replace_text(&t, word).unwrap();
        assert_eq!(texts(&mut s, 0), [word, "ALPHA", "NEIGHBOR"]);
        assert_eq!(object_count(s.bytes()), count);
        assert_eq!(s.bytes().len(), size);
    }
    assert_eq!(texts(&mut s, 1), ["PAGE2"]);
    let mut reopened = open(s.save().unwrap().bytes);
    assert_eq!(texts(&mut reopened, 0), ["GOLF", "ALPHA", "NEIGHBOR"]);
}
#[test]
fn forms_drawn_or_named_more_than_once_are_still_copied() {
    // targets-streams.pdf: Fm2 is named only by page 1 and drawn once, so its
    // NEST text is edited in place. Fm1 is named by both pages and by Fm2.
    let input = fixture("p3/targets-streams.pdf");
    let count = object_count(&input);
    let mut s = open(input.clone());
    let t = target(&mut s, "NEST");
    s.replace_text(&t, "TSEN").unwrap();
    assert_eq!(object_count(s.bytes()), count);
    assert_eq!(texts(&mut s, 0), ["SPLIT", "FORM", "FORM", "FORM", "TSEN"]);
    // The FORM nested in Fm2: Fm1 is copied, and Fm2 (still private) names
    // the copy in its own resources instead of being copied itself.
    let t = s.text_targets(0).unwrap().remove(3);
    s.replace_text(&t, "MROF").unwrap();
    assert_eq!(object_count(s.bytes()), count + 1);
    assert_eq!(texts(&mut s, 0), ["SPLIT", "FORM", "FORM", "MROF", "TSEN"]);
    assert_eq!(texts(&mut s, 1), ["FORM", "PAGE2"]);
    // Page 2's single placement of Fm1 is still copied: page 1 names Fm1 too.
    let t = target_on(&mut s, 1, "FORM");
    s.replace_text(&t, "MROF").unwrap();
    assert_eq!(object_count(s.bytes()), count + 2);
    assert_eq!(texts(&mut s, 0), ["SPLIT", "FORM", "FORM", "MROF", "TSEN"]);
    assert_eq!(texts(&mut s, 1), ["MROF", "PAGE2"]);
    for _ in 0..3 {
        s.undo().unwrap();
    }
    assert_eq!(s.bytes(), input);
}
fn target_on(s: &mut Session, page: usize, text: &str) -> tandem_pdf_engine::session::TextTarget {
    s.text_targets(page)
        .unwrap()
        .into_iter()
        .find(|t| t.text == text)
        .unwrap()
}
