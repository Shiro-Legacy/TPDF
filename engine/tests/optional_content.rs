// SPDX-License-Identifier: AGPL-3.0-or-later
//! Optional content renders in the default configuration's state: hidden
//! groups paint nothing, whether marked content, a Form, an image or an annotation.
use tandem_pdf_engine::session::{Session, SessionLimits};

/// Objects 1 catalog, 2 pages, 3 page, 4 content, 5 group A, 6 group B, then extras.
fn pdf(
    ocproperties: &str,
    resources: &str,
    content: &str,
    page_extra: &str,
    extra: &[&str],
) -> Vec<u8> {
    let mut objects = vec![
        format!("<< /Type /Catalog /Pages 2 0 R /OCProperties {ocproperties} >>"),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources {resources} {page_extra} >>"
        ),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len() + 1
        ),
        "<< /Type /OCG /Name (A) >>".to_string(),
        "<< /Type /OCG /Name (B) >>".to_string(),
    ];
    objects.extend(extra.iter().map(|s| s.to_string()));
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).bytes());
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

fn ink(bytes: Vec<u8>) -> usize {
    let mut session = Session::open_view(bytes, SessionLimits::default()).unwrap();
    let caps = session.capabilities();
    assert!(
        caps.pages[0].render_error.is_none(),
        "{:?}",
        caps.pages[0].render_error
    );
    let image = session.render(0, 72.0).unwrap();
    image.pixels().iter().filter(|&&v| v < 128).count()
}

const PROPS: &str = "<< /Properties << /A 5 0 R /B 6 0 R /M 7 0 R >> /XObject << /X 7 0 R >> >>";
const SQUARE: &str = "0 0 0 rg 10 10 80 80 re f";

fn marked(name: &str) -> String {
    format!("/OC /{name} BDC {SQUARE} EMC")
}

#[test]
fn off_groups_hide_marked_content_but_keep_state() {
    let on = "<< /OCGs [5 0 R 6 0 R] /D << >> >>";
    let off = "<< /OCGs [5 0 R 6 0 R] /D << /OFF [5 0 R] >> >>";
    let base_off = "<< /OCGs [5 0 R 6 0 R] /D << /BaseState /OFF /ON [6 0 R] >> >>";
    let membership = "<< /Type /OCMD /OCGs [5 0 R 6 0 R] /P /AllOn >>";
    assert!(ink(pdf(on, PROPS, &marked("A"), "", &[membership])) > 0);
    assert_eq!(ink(pdf(off, PROPS, &marked("A"), "", &[membership])), 0);
    assert!(ink(pdf(off, PROPS, &marked("B"), "", &[membership])) > 0);
    assert_eq!(
        ink(pdf(base_off, PROPS, &marked("A"), "", &[membership])),
        0
    );
    assert!(ink(pdf(base_off, PROPS, &marked("B"), "", &[membership])) > 0);
    // AllOn with A off hides; AnyOn (the default) with B on shows.
    assert_eq!(ink(pdf(off, PROPS, &marked("M"), "", &[membership])), 0);
    let any = "<< /Type /OCMD /OCGs [5 0 R 6 0 R] >>";
    assert!(ink(pdf(off, PROPS, &marked("M"), "", &[any])) > 0);
    let not_a = "<< /Type /OCMD /VE [/Not 5 0 R] >>";
    assert!(ink(pdf(off, PROPS, &marked("M"), "", &[not_a])) > 0);
    assert_eq!(ink(pdf(on, PROPS, &marked("M"), "", &[not_a])), 0);
    // Color set inside hidden content still applies afterwards.
    let state = "/OC /A BDC 0 0 0 rg EMC 10 10 80 80 re f".to_string();
    let white = "1 1 1 rg /OC /A BDC 0 0 0 rg EMC 10 10 80 80 re f".to_string();
    assert!(ink(pdf(off, PROPS, &state, "", &[membership])) > 0);
    assert!(ink(pdf(off, PROPS, &white, "", &[membership])) > 0);
}

#[test]
fn off_groups_hide_forms_and_annotations() {
    let off = "<< /OCGs [5 0 R 6 0 R] /D << /OFF [5 0 R] >> >>";
    let form = |group: &str| {
        format!(
            "<< /Type /XObject /Subtype /Form /BBox [0 0 100 100] /OC {group} /Length {} >>\nstream\n{SQUARE}\nendstream",
            SQUARE.len() + 1
        )
    };
    assert_eq!(ink(pdf(off, PROPS, "/X Do", "", &[&form("5 0 R")])), 0);
    assert!(ink(pdf(off, PROPS, "/X Do", "", &[&form("6 0 R")])) > 0);
    let annot = |group: &str| {
        format!(
            "<< /Type /Annot /Subtype /Square /Rect [0 0 100 100] /OC {group} /AP << /N 8 0 R >> >>"
        )
    };
    let appearance = format!(
        "<< /Type /XObject /Subtype /Form /BBox [0 0 100 100] /Length {} >>\nstream\n{SQUARE}\nendstream",
        SQUARE.len() + 1
    );
    let page = "/Annots [7 0 R]";
    assert_eq!(
        ink(pdf(off, "<< >>", "", page, &[&annot("5 0 R"), &appearance])),
        0
    );
    assert!(ink(pdf(off, "<< >>", "", page, &[&annot("6 0 R"), &appearance])) > 0);
}

#[test]
fn inline_images_render_and_hide_with_their_layer() {
    let on = "<< /OCGs [5 0 R 6 0 R] /D << >> >>";
    let off = "<< /OCGs [5 0 R 6 0 R] /D << /OFF [5 0 R] >> >>";
    let image = "q 80 0 0 80 10 10 cm BI /W 2 /H 1 /CS /G /BPC 8 ID \0\0 EI Q";
    let hex = "q 80 0 0 80 10 10 cm BI /W 1 /H 1 /CS /RGB /BPC 8 /F /AHx ID 000000> EI Q";
    assert!(ink(pdf(on, PROPS, image, "", &[])) > 0);
    assert!(ink(pdf(on, PROPS, hex, "", &[])) > 0);
    // No ">" before EI: the end marker is implied, as in other viewers.
    let unterminated = "q 80 0 0 80 10 10 cm BI /W 1 /H 1 /CS /G /BPC 8 /F /AHx ID\n00\nEI Q";
    assert!(ink(pdf(on, PROPS, unterminated, "", &[])) > 0);
    assert_eq!(
        ink(pdf(off, PROPS, &format!("/OC /A BDC {image} EMC"), "", &[])),
        0
    );
}
