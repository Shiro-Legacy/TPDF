// SPDX-License-Identifier: AGPL-3.0-or-later
//! Display-only appearances for annotations without one, and for text/choice
//! fields under /NeedAppearances. Nothing generated is ever saved.
use tandem_pdf_engine::session::{Session, SessionLimits};

/// Objects: 1 catalog, 2 pages, 3 page, 4 empty content, 5 Helv font, 6 annotation, then extras.
fn pdf(acroform: &str, annot: &str, extra: &[&str]) -> Vec<u8> {
    let mut objects = vec![
        format!(
            "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [6 0 R] /DR << /Font << /Helv 5 0 R >> >> /DA (/Helv 0 Tf 0 g) {acroform} >> >>"
        ),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /Annots [6 0 R] >>"
            .to_string(),
        "<< /Length 0 >>\nstream\n\nendstream".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        annot.to_string(),
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

/// Dark pixels, or Err with the render refusal.
fn ink(bytes: Vec<u8>) -> Result<usize, String> {
    let mut session =
        Session::open_view(bytes, SessionLimits::default()).map_err(|e| e.to_string())?;
    if let Some(error) = &session.capabilities().pages[0].render_error {
        return Err(error.to_string());
    }
    let image = session.render(0, 72.0).unwrap();
    Ok(image.pixels().chunks(4).filter(|p| p[0] < 128).count())
}

const WIDGET: &str = "/Type /Annot /Subtype /Widget /Rect [20 100 180 124] /F 4 /T (f)";

#[test]
fn text_and_choice_fields_draw_their_values() {
    let text =
        |v: &str, extra: &str| pdf("", &format!("<< {WIDGET} /FT /Tx /V ({v}) {extra} >>"), &[]);
    assert_eq!(ink(text("", "")), Ok(0));
    let hello = ink(text("Hello", "")).unwrap();
    assert!(hello > 20);
    assert!(ink(text("Hello", "/Q 1")).unwrap() > 20);
    assert!(
        ink(text(
            "one two three four five six seven",
            "/Ff 4096 /DA (/Helv 10 Tf 0 g)"
        ))
        .unwrap()
            > hello
    );
    // Password fields show asterisks, not the value.
    assert!(ink(text("Hello", "/Ff 8192")).unwrap() > 0);
    let combo = pdf(
        "",
        &format!("<< {WIDGET} /FT /Ch /Ff 131072 /Opt [[(a) (Apple)] [(b) (Banana)]] /V (b) >>"),
        &[],
    );
    assert!(ink(combo).unwrap() > 20);
    let list = pdf(
        "",
        "<< /Type /Annot /Subtype /Widget /Rect [20 100 180 160] /FT /Ch /Opt [(one) (two)] /V (two) >>",
        &[],
    );
    assert!(ink(list).unwrap() > 20);
    // Border and background colors from /MK draw even when the value is empty.
    assert!(ink(text("", "/MK << /BC [0] >>")).unwrap() > 100);
}

#[test]
fn buttons_signatures_and_notes() {
    let check = |state: &str| pdf("", &format!("<< {WIDGET} /FT /Btn /V /{state} >>"), &[]);
    assert!(ink(check("Yes")).unwrap() > 10);
    assert_eq!(ink(check("Off")), Ok(0));
    let radio = pdf(
        "",
        &format!("<< {WIDGET} /FT /Btn /Ff 32768 /AS /On >>"),
        &[],
    );
    assert!(ink(radio).unwrap() > 10);
    let sig = pdf("", &format!("<< {WIDGET} /FT /Sig >>"), &[]);
    assert_eq!(ink(sig), Ok(0));
    let note = pdf(
        "",
        "<< /Type /Annot /Subtype /Text /Rect [20 100 180 180] >>",
        &[],
    );
    assert!(ink(note).unwrap() > 40);
    // Unsupported cases still refuse instead of drawing nothing.
    let push = pdf("", &format!("<< {WIDGET} /FT /Btn /Ff 65536 >>"), &[]);
    assert!(ink(push).is_err());
    let square = pdf(
        "",
        "<< /Type /Annot /Subtype /Square /Rect [20 100 180 180] >>",
        &[],
    );
    assert!(ink(square).is_err());
}

#[test]
fn need_appearances_regenerates_text_fields_over_stale_streams() {
    let annot = format!("<< {WIDGET} /FT /Tx /V (Fresh) /AP << /N 7 0 R >> >>");
    let stale =
        "<< /Type /XObject /Subtype /Form /BBox [0 0 160 24] /Length 0 >>\nstream\n\nendstream";
    assert_eq!(ink(pdf("", &annot, &[stale])), Ok(0));
    assert!(ink(pdf("/NeedAppearances true", &annot, &[stale])).unwrap() > 20);
    // Generated objects are display-only: the document stays view-only and
    // its bytes are unchanged.
    let bytes = pdf("/NeedAppearances true", &annot, &[stale]);
    let mut session = Session::open_view(bytes.clone(), SessionLimits::default()).unwrap();
    session.render(0, 36.0).unwrap();
    assert_eq!(session.bytes(), bytes.as_slice());
    assert!(session.capabilities().save_error.is_some());
}
