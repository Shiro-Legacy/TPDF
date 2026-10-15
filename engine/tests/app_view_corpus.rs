// SPDX-License-Identifier: AGPL-3.0-or-later
//! Render parity with MuPDF 1.28.1 on structures the external corpus
//! differential found (`design/app-corpus-fidelity.md`), each rebuilt here
//! as a minimal synthetic document.
use serde_json::{Value, json};
use tandem_pdf_engine::app::{AppEngine, AppSession};

/// A classic-xref PDF from object bodies numbered from 1.
fn pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(body);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    let count = objects.len() + 1;
    bytes.extend_from_slice(format!("xref\n0 {count}\n0000000000 65535 f \n").as_bytes());
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    let trailer = format!("trailer\n<< /Size {count} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n");
    bytes.extend_from_slice(trailer.as_bytes());
    bytes
}

fn object(text: &str) -> Vec<u8> {
    text.as_bytes().to_vec()
}

/// A stream object: `dict` holds the entries besides /Length.
fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut body = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(b"\nendstream");
    body
}

/// A minimal ICC v2 display profile: sRGB's D50 colorants (or gray) with a
/// pure gamma curve. Gamma 1 draws 0.5 as about 188, gamma 3 about 99.
fn icc(gray: bool, gamma: f64) -> Vec<u8> {
    let s15 = |v: f64| ((v * 65536.0).round() as i32).to_be_bytes();
    let xyz = |[x, y, z]: [f64; 3]| {
        let mut tag = b"XYZ \0\0\0\0".to_vec();
        for v in [x, y, z] {
            tag.extend(s15(v));
        }
        tag
    };
    let mut curve = b"curv\0\0\0\0\0\0\0\x01".to_vec();
    curve.extend(((gamma * 256.0).round() as u16).to_be_bytes());
    curve.extend([0, 0]);
    let mut tags: Vec<(&[u8; 4], Vec<u8>)> = vec![(b"wtpt", xyz([0.9642, 1.0, 0.8249]))];
    if gray {
        tags.push((b"kTRC", curve));
    } else {
        tags.push((b"rXYZ", xyz([0.4361, 0.2225, 0.0139])));
        tags.push((b"gXYZ", xyz([0.3851, 0.7169, 0.0971])));
        tags.push((b"bXYZ", xyz([0.1431, 0.0606, 0.7141])));
        for sig in [b"rTRC", b"gTRC", b"bTRC"] {
            tags.push((sig, curve.clone()));
        }
    }
    let mut table = (tags.len() as u32).to_be_bytes().to_vec();
    let mut data = Vec::new();
    let start = 128 + 4 + 12 * tags.len();
    for (sig, tag) in &tags {
        table.extend(*sig);
        table.extend(((start + data.len()) as u32).to_be_bytes());
        table.extend((tag.len() as u32).to_be_bytes());
        data.extend(tag);
    }
    let mut header = vec![0u8; 128];
    header[..4].copy_from_slice(&((start + data.len()) as u32).to_be_bytes());
    header[8..12].copy_from_slice(&[2, 0x10, 0, 0]);
    header[12..16].copy_from_slice(b"mntr");
    header[16..20].copy_from_slice(if gray { b"GRAY" } else { b"RGB " });
    header[20..24].copy_from_slice(b"XYZ ");
    header[36..40].copy_from_slice(b"acsp");
    header[68..80].copy_from_slice(&[s15(0.9642), s15(1.0), s15(0.8249)].concat());
    [header, table, data].concat()
}

fn session_with(bytes: Vec<u8>) -> (AppEngine, AppSession) {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let request = json!({"id": 1, "op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}});
    let (reply, _) = engine.dispatch(&mut session, request, vec![bytes]);
    assert_eq!(reply["ok"], true, "{reply}");
    (engine, session)
}

/// The RGBA pixels of page 0 at scale 1, with the page's pixel width.
fn render(bytes: Vec<u8>) -> (Vec<u8>, usize) {
    let (mut engine, mut session) = session_with(bytes);
    let request = json!({"op": "render", "page": 0, "scale": 1});
    let (reply, mut buffers): (Value, _) = engine.dispatch(&mut session, request, Vec::new());
    assert_eq!(reply["ok"], true, "{reply}");
    let width = reply["pixelWidth"].as_u64().unwrap() as usize;
    (buffers.remove(0), width)
}

/// The RGB of pixel (x, y).
fn at(pixels: &[u8], width: usize, x: usize, y: usize) -> [u8; 3] {
    let i = (y * width + x) * 4;
    [pixels[i], pixels[i + 1], pixels[i + 2]]
}

fn near(rgb: [u8; 3], level: u8) -> bool {
    rgb.iter().all(|&c| c.abs_diff(level) <= 3)
}

/// A 200 × 20 page painting 50% gray in a 20 × 20 square per way device
/// colour reaches the page, left to right: `rg`, `cs`/`sc`, an image, an
/// inline image, a shading, a form without resources, an annotation
/// appearance, an Indexed image, then DeviceCMYK and DeviceGray (`k`, `g`).
/// Objects 1-2 are the catalog and the pages, `catalog` its extra entries;
/// `extra` are numbered from 13; `form` is added to the form's dictionary.
fn device_page(catalog: &str, page_resources: &str, form: &str, extra: &[Vec<u8>]) -> Vec<u8> {
    let content = [
        b"0.5 0.5 0.5 rg 0 0 20 20 re f\n".as_slice(),
        b"/DeviceRGB cs 0.5 0.5 0.5 sc 20 0 20 20 re f\n",
        b"q 20 0 0 20 40 0 cm /Im Do Q\n",
        b"q 20 0 0 20 60 0 cm BI /W 1 /H 1 /BPC 8 /CS /RGB ID \x80\x80\x80 EI Q\n",
        b"q 80 0 20 20 re W n /Sh sh Q\n",
        b"/Fm Do\n",
        b"q 20 0 0 20 140 0 cm /Ix Do Q\n",
        b"0 0 0 0.5 k 160 0 20 20 re f\n",
        b"0.5 g 180 0 20 20 re f\n",
    ]
    .concat();
    let mut objects = vec![
        object(&format!("<< /Type /Catalog /Pages 2 0 R {catalog} >>")),
        object("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        object(&format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 20] /Contents 4 0 R /Annots [8 0 R] \
             /Resources << /XObject << /Im 5 0 R /Fm 6 0 R /Ix 10 0 R >> /Shading << /Sh 11 0 R >> \
             {page_resources} >> >>"
        )),
        stream("", &content),
        stream(
            "/Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceRGB",
            b"\x80\x80\x80",
        ),
        stream(
            &format!("/Type /XObject /Subtype /Form /BBox [0 0 200 20] {form}"),
            b"0.5 0.5 0.5 rg 100 0 20 20 re f",
        ),
        object("null"),
        object("<< /Type /Annot /Subtype /Square /Rect [120 0 140 20] /F 4 /AP << /N 9 0 R >> >>"),
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 20 20]",
            b"0.5 0.5 0.5 rg 0 0 20 20 re f",
        ),
        stream(
            "/Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 \
             /ColorSpace [/Indexed /DeviceRGB 0 <808080>]",
            b"\x00",
        ),
        object(
            "<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [80 0 100 0] \
             /Function << /FunctionType 2 /Domain [0 1] /C0 [0.5 0.5 0.5] /C1 [0.5 0.5 0.5] /N 1 >> >>",
        ),
        object("null"),
    ];
    objects.extend(extra.iter().cloned());
    pdf(&objects)
}

/// The output intent dictionary naming object 13 as its profile.
const INTENT: &str =
    "/OutputIntents [<< /Type /OutputIntent /S /GTS_PDFA1 /DestOutputProfile 13 0 R >>]";

/// An ICCBased profile stream. hayro caches a stream by its dictionary's
/// bytes, so two profiles in one file must not have equal dictionaries.
fn profile(gray: bool, gamma: f64) -> Vec<u8> {
    let n = if gray { 1 } else { 3 };
    stream(&format!("/N {n} /Gamma {gamma:.1}"), &icc(gray, gamma))
}

/// The centre of each square of `device_page`.
fn squares(pixels: &[u8], width: usize) -> Vec<[u8; 3]> {
    (0..10)
        .map(|i| at(pixels, width, i * 20 + 10, 10))
        .collect()
}

#[test]
fn device_rgb_is_drawn_through_the_output_intent_profile() {
    let (plain, width) = render(device_page("", "", "", &[]));
    let plain = squares(&plain, width);
    for (i, rgb) in plain.iter().take(8).enumerate() {
        assert!(
            near(*rgb, 128),
            "square {i} without an output intent: {rgb:?}"
        );
    }
    // MuPDF draws DeviceRGB paint, images (Indexed bases too), inline
    // images, shadings, forms and appearances through an RGB output intent,
    // and leaves DeviceCMYK and DeviceGray as they were.
    let (mapped, width) = render(device_page(INTENT, "", "", &[profile(false, 1.0)]));
    let mapped = squares(&mapped, width);
    for (i, rgb) in mapped.iter().take(8).enumerate() {
        assert!(
            near(*rgb, 188),
            "square {i} through the output intent: {rgb:?}"
        );
    }
    assert_eq!(mapped[8..], plain[8..]);
}

#[test]
fn a_gray_output_intent_maps_device_gray_only() {
    let (plain, width) = render(device_page("", "", "", &[]));
    let plain = squares(&plain, width);
    let (mapped, width) = render(device_page(INTENT, "", "", &[profile(true, 1.0)]));
    let mapped = squares(&mapped, width);
    assert_eq!(mapped[..9], plain[..9]);
    assert!(near(mapped[9], 188), "{:?}", mapped[9]);
}

#[test]
fn page_and_form_defaults_win_over_the_output_intent() {
    // The page's /DefaultRGB (gamma 3) wins over the output intent (gamma
    // 1); a form with its own /DefaultRGB uses that inside it.
    let page = "/ColorSpace << /DefaultRGB [/ICCBased 14 0 R] >>";
    let profiles = [profile(false, 1.0), profile(false, 3.0)];
    let (pixels, width) = render(device_page(INTENT, page, "", &profiles));
    for (i, rgb) in squares(&pixels, width).iter().take(8).enumerate() {
        assert!(
            near(*rgb, 99),
            "square {i} through the page's default: {rgb:?}"
        );
    }
    let own = "/Resources << /ColorSpace << /DefaultRGB [/ICCBased 13 0 R] >> >>";
    let (pixels, width) = render(device_page(INTENT, page, own, &profiles));
    let squares = squares(&pixels, width);
    assert!(near(squares[5], 188), "{:?}", squares[5]);
    assert!(near(squares[0], 99), "{:?}", squares[0]);
}

#[test]
fn only_the_first_output_intent_counts_and_its_n_must_match() {
    let (plain, width) = render(device_page("", "", "", &[]));
    let plain = squares(&plain, width);
    let second = "/OutputIntents [<< /Type /OutputIntent /S /GTS_PDFA1 >> \
                  << /Type /OutputIntent /S /GTS_PDFA1 /DestOutputProfile 13 0 R >>]";
    let (pixels, width) = render(device_page(second, "", "", &[profile(false, 1.0)]));
    assert_eq!(squares(&pixels, width), plain);
    let mismatched = stream("/N 1", &icc(false, 1.0));
    let (pixels, width) = render(device_page(INTENT, "", "", &[mismatched]));
    assert_eq!(squares(&pixels, width), plain);
}

#[test]
fn the_output_intent_never_reaches_the_saved_file() {
    let source = device_page(INTENT, "", "", &[profile(false, 1.0)]);
    let (mut engine, mut session) = session_with(source.clone());
    let render = json!({"op": "render", "page": 0, "scale": 1});
    let (reply, _) = engine.dispatch(&mut session, render, Vec::new());
    assert_eq!(reply["ok"], true, "{reply}");
    let (reply, buffers) = engine.dispatch(&mut session, json!({"op": "save"}), Vec::new());
    assert_eq!(reply["ok"], true, "{reply}");
    let saved = &buffers[0];
    let name = b"TPDFDefault";
    assert!(!saved.windows(name.len()).any(|w| w == name));
    let content = b"0.5 0.5 0.5 rg 0 0 20 20 re f";
    assert!(saved.windows(content.len()).any(|w| w == content));
}

/// A 150 × 150 page filled with a tiling pattern of step 15 whose cell is
/// `bbox`, drawing a 6 × 6 blue square at (`at`, `at`) of the cell;
/// uncoloured (PaintType 2, painted blue by the page) when `uncoloured`.
fn tiling_page(bbox: &str, at: u32, uncoloured: bool) -> Vec<u8> {
    let (paint, fill, draw) = match uncoloured {
        false => (
            1,
            "/Pattern cs /P0 scn",
            format!("0 0 1 rg {at} {at} 6 6 re f"),
        ),
        true => (2, "/Cs cs 0 0 1 /P0 scn", format!("{at} {at} 6 6 re f")),
    };
    pdf(&[
        object("<< /Type /Catalog /Pages 2 0 R >>"),
        object("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        object(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 150 150] /Contents 4 0 R \
             /Resources << /Pattern << /P0 5 0 R >> /ColorSpace << /Cs [/Pattern /DeviceRGB] >> >> >>",
        ),
        stream("", format!("{fill} 0 0 150 150 re f").as_bytes()),
        stream(
            &format!(
                "/Type /Pattern /PatternType 1 /PaintType {paint} /TilingType 1 /BBox [{bbox}] \
                 /XStep 15 /YStep 15"
            ),
            draw.as_bytes(),
        ),
    ])
}

#[test]
fn a_tiling_cell_larger_than_its_step_overlaps_its_neighbours() {
    // MuPDF draws the whole cell at every step; hayro alone kept only the
    // first step of it, and the square at (64, 64) of an 80 × 80 cell
    // stepped by 15 never showed. 64 lands on 4 modulo 15.
    for uncoloured in [false, true] {
        for (bbox, corner) in [("0 0 80 80", 64), ("0 0 15 15", 4), ("80 80 0 0", 64)] {
            let (pixels, width) = render(tiling_page(bbox, corner, uncoloured));
            for (x, y) in [(7, 7), (22, 37), (142, 142)] {
                let rgb = at(&pixels, width, x, 150 - y);
                assert_eq!(rgb, [0, 0, 255], "{bbox} {uncoloured} at ({x}, {y})");
            }
            for (x, y) in [(12, 12), (2, 7), (7, 13)] {
                let rgb = at(&pixels, width, x, 150 - y);
                assert_eq!(rgb, [255, 255, 255], "{bbox} {uncoloured} at ({x}, {y})");
            }
        }
    }
}
