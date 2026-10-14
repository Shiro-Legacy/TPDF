// SPDX-License-Identifier: AGPL-3.0-or-later
//! cs/CS with device-equivalent spaces (device names, resources naming them,
//! CalRGB and ICCBased by /N) set colors through sc/scn; other spaces refuse.
use tandem_pdf_engine::session::{Session, SessionLimits};

fn pdf(spaces: &str, content: &str) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << /ColorSpace << {spaces} >> >> >>"
        ),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len() + 1
        ),
        "<< /N 3 /Length 0 >>\nstream\n\nendstream".to_string(),
    ];
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

/// Center pixel RGB, or None when the page refuses to render.
fn center(spaces: &str, content: &str) -> Option<[u8; 3]> {
    let mut session = Session::open_view(pdf(spaces, content), SessionLimits::default()).ok()?;
    if session.capabilities().pages[0].render_error.is_some() {
        return None;
    }
    let image = session.render(0, 72.0).unwrap();
    let i = 50 * image.stride() + 50 * 4;
    Some(image.pixels()[i..i + 3].try_into().unwrap())
}

const SPACES: &str = "/RGBName /DeviceRGB /ICC [/ICCBased 5 0 R] /Cal [/CalRGB << /WhitePoint [0.95 1 1.09] >>] /Idx [/Indexed /DeviceRGB 0 <ff0000>] /Pat /Pattern";

#[test]
fn device_equivalent_spaces_set_fill_and_stroke_colors() {
    let red = Some([255, 0, 0]);
    for space in ["/DeviceRGB", "/RGBName", "/ICC", "/Cal"] {
        assert_eq!(
            center(SPACES, &format!("{space} cs 1 0 0 sc 0 0 100 100 re f")),
            red,
            "{space}"
        );
        assert_eq!(
            center(SPACES, &format!("{space} cs 1 0 0 scn 0 0 100 100 re f")),
            red,
            "{space}"
        );
    }
    assert_eq!(
        center(SPACES, "/DeviceGray cs 0.5 sc 0 0 100 100 re f"),
        Some([128; 3]).map(|_| center(SPACES, "0.5 g 0 0 100 100 re f").unwrap())
    );
    assert_eq!(
        center(SPACES, "/DeviceRGB CS 1 0 0 SC 40 w 0 50 m 100 50 l S"),
        red
    );
    // The initial color of a new space is black; q/Q restores the space.
    assert_eq!(
        center(SPACES, "/DeviceRGB cs 0 0 100 100 re f"),
        Some([0, 0, 0])
    );
    assert_eq!(
        center(SPACES, "q /DeviceRGB cs Q 0.5 sc 0 0 100 100 re f"),
        center(SPACES, "0.5 g 0 0 100 100 re f")
    );
    // Text uses the fill color.
    assert!(center(SPACES, "/RGBName cs 1 0 0 sc 0 0 100 100 re f BT ET").is_some());
}

#[test]
fn other_spaces_and_mismatched_components_refuse() {
    for content in [
        "/Idx cs 0 sc 0 0 100 100 re f",
        "/Pat cs /P1 scn 0 0 100 100 re f",
        "/Missing cs 0 0 100 100 re f",
        "/DeviceRGB cs 1 sc 0 0 100 100 re f",
        "1 0 0 sc 0 0 100 100 re f",
    ] {
        assert_eq!(center(SPACES, content), None, "{content}");
    }
}
