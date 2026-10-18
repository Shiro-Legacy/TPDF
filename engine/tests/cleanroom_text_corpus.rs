// SPDX-License-Identifier: MIT
//! Acceptance for the spec 04 integration addenda C3 and C4, found by the
//! corpus run of the clean-room tree. Every input is a page built here; the
//! expected values come from the addenda (spec 04 §12) and, for C4, from the
//! recorded black-box experiment E9 (spec 04 §12).
use tandem_pdf_engine::view::{MAX_SEARCH_QUADS, RenderOptions, ViewDocument};

// ------------------------------------------------------------------ builders

fn pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).bytes());
        out.extend(body);
        out.extend(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
    for offset in offsets {
        out.extend(format!("{offset:010} 00000 n \n").bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .bytes(),
    );
    out
}

fn stream(data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< /Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend(data);
    out.extend(b"\nendstream");
    out
}

/// One 300 × 300 page with `content`, font /F1 = object 5 (`font`), and
/// `extra` objects numbered from 6.
fn page_with(content: &str, font: &str, extra: &[Vec<u8>]) -> ViewDocument {
    let objects = [
        vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
            stream(content.as_bytes()),
            font.as_bytes().to_vec(),
        ],
        extra.to_vec(),
    ]
    .concat();
    ViewDocument::open(pdf(&objects), "").unwrap()
}

const HELVETICA: &str =
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>";

fn helvetica(content: &str) -> ViewDocument {
    page_with(content, HELVETICA, &[])
}

/// Whitespace removed: the C3 checks are about character order, not about
/// where lines break.
fn squeezed(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

// ------------------------------------------------------------------ C3

/// C3: a run that starts well back over the line already built (here a
/// form XObject's word drawn at twice the size from the same start) is
/// never interleaved with that line, character by character.
#[test]
#[ignore = "clean-room: spec 04 C3"]
fn c3_a_run_drawn_back_over_a_line_is_not_interleaved_with_it() {
    let doc = helvetica("BT /F1 24 Tf 72 200 Td (big) Tj ET BT /F1 48 Tf 72 200 Td (small) Tj ET");
    let text = doc.structured_text(0).unwrap();
    for (layout, out) in [
        ("as_text", text.as_text()),
        ("spans_text", text.spans_text()),
    ] {
        let flat = squeezed(&out);
        assert!(
            flat.contains("big") && flat.contains("small"),
            "{layout} interleaves the two runs: {out:?}"
        );
    }
    for word in ["big", "small"] {
        assert_eq!(
            text.search(word, MAX_SEARCH_QUADS).hits.len(),
            1,
            "search for {word:?} in {:?}",
            text.as_text()
        );
    }
}

/// C3: a caution sign drawn as a large glyph with a mark on it, and a small
/// label drawn across both: the label reads whole.
#[test]
#[ignore = "clean-room: spec 04 C3"]
fn c3_a_small_label_drawn_across_a_large_mark_reads_whole() {
    let doc = helvetica(
        "BT /F1 23 Tf 90 202 Td (V) Tj ET \
         BT /F1 14.5 Tf 100 200 Td (!) Tj ET \
         BT /F1 4.5 Tf 92 194 Td (CAUTION) Tj ET",
    );
    let text = doc.structured_text(0).unwrap();
    for (layout, out) in [
        ("as_text", text.as_text()),
        ("spans_text", text.spans_text()),
    ] {
        let flat = squeezed(&out);
        assert!(
            flat.contains("CAUTION") && flat.contains('!') && flat.contains('V'),
            "{layout} splits the label: {out:?}"
        );
    }
    assert_eq!(text.search("CAUTION", MAX_SEARCH_QUADS).hits.len(), 1);
}

/// C3: characters whose positions along the line differ by far less than
/// 0.01 em keep their drawing order. A zero-advance Thai vowel mark (b)
/// drawn after its base (a) and before the next letter (c), which a
/// one-thousandth-em kern puts a hair to the left of the mark: the text
/// reads base, mark, letter (U+0E04 U+0E37 U+0E2D).
#[test]
#[ignore = "clean-room: spec 04 C3"]
fn c3_characters_at_one_position_keep_their_drawing_order() {
    let cmap = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /T def \
                1 begincodespacerange <00> <FF> endcodespacerange\n3 beginbfchar\n\
                <61> <0E04>\n<62> <0E37>\n<63> <0E2D>\nendbfchar endcmap \
                CMapName currentdict /CMap defineresource pop end end";
    let font = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /FirstChar 97 /LastChar 99 \
                /Widths [556 0 556] /ToUnicode 6 0 R >>";
    let expected = "\u{0E04}\u{0E37}\u{0E2D}";
    for content in [
        "BT /F1 12 Tf 50 200 Td (abc) Tj ET",
        "BT /F1 12 Tf 50 200 Td [(ab) 1 (c)] TJ ET",
        "BT /F1 12 Tf 50 200 Td [(ab) 0.01 (c)] TJ ET",
    ] {
        let doc = page_with(content, font, &[stream(cmap.as_bytes())]);
        let text = doc.structured_text(0).unwrap();
        assert_eq!(text.as_text().trim(), expected, "{content}");
        assert_eq!(text.spans_text().trim(), expected, "{content}");
        assert_eq!(
            text.search(expected, MAX_SEARCH_QUADS).hits.len(),
            1,
            "{content}"
        );
    }
}

/// C3 control (holds today): runs side by side on one baseline, each to the
/// right of the last, read in drawing order on one line.
#[test]
fn c3_runs_that_follow_each_other_read_in_order() {
    let doc = helvetica(
        "BT /F1 10 Tf 20 200 Td (Total) Tj ET BT /F1 10 Tf 48 200 Td (due) Tj ET \
         BT /F1 6 Tf 66 204 Td (2) Tj ET",
    );
    let text = doc.structured_text(0).unwrap();
    assert_eq!(squeezed(&text.as_text()), "Totaldue2");
}

// ------------------------------------------------------------------ C4

fn render_sizes(entries: &str, scales: &[f64]) -> Vec<(u32, u32)> {
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R {entries} >>").into_bytes(),
    ];
    let doc = ViewDocument::open(pdf(&objects), "").unwrap();
    scales
        .iter()
        .map(|&s| {
            let image = doc.render(0, s, RenderOptions::default()).unwrap();
            (image.width, image.height)
        })
        .collect()
}

/// C4 (experiment E9): a side written with an excess of 0.001 point over a
/// whole number is that whole number of pixels at scale 1; 0.00105 and
/// 0.0011 are over the tolerance. The box numbers are read as written,
/// not rounded to single precision first.
#[test]
#[ignore = "clean-room: spec 04 C4"]
fn c4_pixel_size_reads_the_box_numbers_as_written() {
    let rows: [(&str, [(u32, u32); 2]); 8] = [
        ("/MediaBox [0 0 100 500.001]", [(100, 500), (200, 1001)]),
        ("/MediaBox [0 0 100 315.001]", [(100, 315), (200, 631)]),
        ("/MediaBox [0 0 100 1000.001]", [(100, 1000), (200, 2001)]),
        ("/MediaBox [0 0 100 100.001]", [(100, 100), (200, 201)]),
        (
            "/MediaBox [0 0 100 600] /CropBox [0 100.999 100 600]",
            [(100, 499), (200, 999)],
        ),
        (
            "/MediaBox [0 0 100 600] /CropBox [0 200.999 100 516]",
            [(100, 315), (200, 631)],
        ),
        ("/MediaBox [0 0 100 500.00105]", [(100, 501), (200, 1001)]),
        ("/MediaBox [0 0 100 500.0011]", [(100, 501), (200, 1001)]),
    ];
    for (entries, want) in rows {
        assert_eq!(render_sizes(entries, &[1.0, 2.0]), want, "{entries}");
    }
}
