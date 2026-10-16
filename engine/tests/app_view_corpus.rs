// SPDX-License-Identifier: AGPL-3.0-or-later
//! Render parity with MuPDF 1.28.1 on structures the external corpus
//! differential found (`design/app-corpus-fidelity.md`), each rebuilt here
//! as a minimal synthetic document.
use serde_json::{Value, json};
use std::time::Instant;
use tandem_pdf_engine::app::{AppEngine, AppSession};
use tandem_pdf_engine::object::ObjectId;
use tandem_pdf_engine::view::RenderOptions;

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

/// A CMap program with `/WMode program def` (none for None), mapping each
/// two-byte code to the same CID.
fn identity_cmap(program: Option<u8>) -> String {
    let wmode = program.map_or(String::new(), |w| format!("/WMode {w} def "));
    format!(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
         /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> def \
         /CMapName /Test def /CMapType 1 def {wmode}1 begincodespacerange <0000> <FFFF> \
         endcodespacerange 1 begincidrange <0000> <FFFF> 0 endcidrange endcmap \
         CMapName currentdict /CMap defineresource pop end end"
    )
}

/// "ABC" at (50, 150) in a non-embedded Type0 Helvetica whose embedded
/// CMap's dictionary sets /WMode `dict` and its program `program`.
fn cmap_page(dict: Option<u8>, program: Option<u8>) -> Vec<u8> {
    let to_unicode = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def \
         /CMapName /Adobe-Identity-UCS def /CMapType 2 def 1 begincodespacerange <0000> <FFFF> \
         endcodespacerange 3 beginbfchar <0001> <0041> <0002> <0042> <0003> <0043> endbfchar \
         endcmap CMapName currentdict /CMap defineresource pop end end";
    let wmode = dict.map_or(String::new(), |w| format!("/WMode {w}"));
    let system = "/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >>";
    pdf(&[
        object("<< /Type /Catalog /Pages 2 0 R >>"),
        object("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        object(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R \
             /Resources << /Font << /F1 5 0 R >> >> >>",
        ),
        stream("", b"BT /F1 20 Tf 50 150 Td <000100020003> Tj ET"),
        object(
            "<< /Type /Font /Subtype /Type0 /BaseFont /Helvetica /Encoding 6 0 R \
             /DescendantFonts [7 0 R] /ToUnicode 8 0 R >>",
        ),
        stream(
            &format!("/Type /CMap /CMapName /Test {system} {wmode}"),
            identity_cmap(program).as_bytes(),
        ),
        object(&format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Helvetica {system} \
             /FontDescriptor 9 0 R /DW 1000 >>"
        )),
        stream("", to_unicode.as_bytes()),
        object(
            "<< /Type /FontDescriptor /FontName /Helvetica /Flags 32 /FontBBox [0 -200 1000 900] \
             /ItalicAngle 0 /Ascent 900 /Descent -200 /CapHeight 700 /StemV 80 >>",
        ),
    ])
}

/// The width and height of the box holding every non-white pixel.
fn ink_extent(pixels: &[u8], width: usize) -> (usize, usize) {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for (i, px) in pixels.chunks_exact(4).enumerate() {
        if px[..3] != [255, 255, 255] {
            let (x, y) = (i % width, i / width);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
    }
    assert!(x0 <= x1, "nothing drawn");
    (x1 - x0, y1 - y0)
}

#[test]
fn an_embedded_cmap_writes_as_its_dictionary_says() {
    // MuPDF takes /WMode from the CMap's stream dictionary when it has one;
    // hayro alone read the program's.
    for (dict, program, vertical) in [
        (Some(1), Some(0), true),
        (Some(1), None, true),
        (Some(0), Some(1), false),
        (None, Some(1), true),
        (None, None, false),
    ] {
        let (pixels, width) = render(cmap_page(dict, program));
        let (w, h) = ink_extent(&pixels, width);
        assert_eq!(h > w, vertical, "{dict:?} {program:?}: {w} × {h}");
    }
}

/// A 100 × 100 page whose content draws form 5 (`form_resources`,
/// `form_content`); the page's resources hold /Sq, a form filling a blue
/// square at (20, 20), and /CsB, a Separation drawing its tint as blue.
/// Form 7 has resources without /Sq and draws it; object 9 is a resources
/// object without /Sq. `annot` draws form 5 as an annotation's appearance
/// instead.
fn inherit_page(form_resources: &str, form_content: &str, annot: bool) -> Vec<u8> {
    let (contents, annots) = match annot {
        true => ("", "/Annots [8 0 R]"),
        false => ("/Fm Do", ""),
    };
    pdf(&[
        object("<< /Type /Catalog /Pages 2 0 R >>"),
        object("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        object(&format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R {annots} \
             /Resources << /XObject << /Fm 5 0 R /Sq 6 0 R >> /ColorSpace << /CsB [/Separation /B \
             /DeviceRGB << /FunctionType 2 /Domain [0 1] /C0 [1 1 1] /C1 [0 0 1] /N 1 >>] >> >> >>"
        )),
        stream("", contents.as_bytes()),
        stream(
            &format!("/Type /XObject /Subtype /Form /BBox [0 0 100 100] {form_resources}"),
            form_content.as_bytes(),
        ),
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 100 100]",
            b"0 0 1 rg 20 20 40 40 re f",
        ),
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << /Font << >> >>",
            b"/Sq Do",
        ),
        object("<< /Type /Annot /Subtype /Square /Rect [0 0 100 100] /F 4 /AP << /N 5 0 R >> >>"),
        object("<< /ExtGState << >> >>"),
    ])
}

#[test]
fn a_form_finds_names_its_resources_lack_in_the_enclosing_ones() {
    // MuPDF looks a name up in the form's own resources, then its caller's,
    // out to the page's (appearances: the page's); hayro alone only in the
    // form's own.
    let lacking = "/Resources << /Font << >> >>";
    for (resources, content, annot) in [
        (lacking, "/Sq Do", false),
        (lacking, "/CsB cs 1 sc 20 20 40 40 re f", false),
        (
            "/Resources << /XObject << /Fm2 7 0 R >> >>",
            "/Fm2 Do",
            false,
        ),
        ("/Resources 9 0 R", "/Sq Do", false),
        ("", "/Sq Do", false),
        (lacking, "/Sq Do", true),
        ("", "/Sq Do", true),
    ] {
        let (pixels, width) = render(inherit_page(resources, content, annot));
        let rgb = at(&pixels, width, 40, 60);
        assert_eq!(rgb, [0, 0, 255], "{resources} {content} annot {annot}");
    }
}

/// A 200 × 200 page with one annotation of `kind` and flags `flags` whose
/// 7.2 × 7.2 /Rect has its top left at (100, 107.2), drawn by an 18 × 18
/// blue appearance with `matrix`.
fn icon_page(kind: &str, flags: u32, matrix: &str) -> Vec<u8> {
    pdf(&[
        object("<< /Type /Catalog /Pages 2 0 R >>"),
        object("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        object(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R \
             /Annots [5 0 R] >>",
        ),
        stream("", b""),
        object(&format!(
            "<< /Type /Annot /Subtype /{kind} /F {flags} /Rect [100 100 107.2 107.2] \
             /AP << /N 6 0 R >> >>"
        )),
        stream(
            &format!("/Type /XObject /Subtype /Form /BBox [0 0 18 18] {matrix}"),
            b"0 0 1 rg 0 0 18 18 re f",
        ),
    ])
}

#[test]
fn a_fixed_size_appearance_is_drawn_at_its_bbox_size_whatever_its_matrix() {
    // MuPDF draws a note, or an annotation flagged NoZoom, at its /BBox's
    // size from the top left of /Rect, whatever /Matrix scales; the view
    // drew it at the size /Matrix gave it.
    for (kind, flags, matrix, size) in [
        ("Text", 28, "/Matrix [.4 0 0 .4 306 396]", 18),
        ("Text", 4, "/Matrix [2 0 0 2 0 0]", 18),
        ("Square", 12, "/Matrix [.4 0 0 .4 306 396]", 18),
        ("Square", 12, "", 18),
        ("Square", 4, "/Matrix [.4 0 0 .4 306 396]", 7),
    ] {
        let (pixels, width) = render(icon_page(kind, flags, matrix));
        let (mut x0, mut x1, mut y0, mut y1) = (usize::MAX, 0, usize::MAX, 0);
        for (i, px) in pixels.chunks_exact(4).enumerate() {
            if px[0] < 128 && px[2] > 128 {
                let (x, y) = (i % width, 200 - 1 - i / width);
                (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
            }
        }
        let label = format!("{kind} {flags} {matrix}");
        assert_eq!((x0, y1), (100, 106), "{label}");
        assert_eq!((x1 + 1 - x0, y1 + 1 - y0), (size, size), "{label}");
    }
}

/// Page 0's pixels in the session's current view.
fn view_pixels(session: &mut AppSession) -> Vec<u8> {
    let view = session.view().unwrap();
    view.render(0, 1.0, RenderOptions::default())
        .unwrap()
        .pixels
}

#[test]
fn the_rewritten_view_follows_edits_undo_redo_and_an_open_operation() {
    // Rewrites are kept from one view to the next; an edit, its undo and
    // redo, and writes inside an open operation must each show.
    let source = device_page(INTENT, "", "", &[profile(false, 1.0)]);
    let find = |bytes: &[u8], what: &[u8], from: usize| {
        from + bytes[from..]
            .windows(what.len())
            .position(|w| w == what)
            .unwrap()
    };
    let (old, new) = (
        b"0.5 0.5 0.5 rg 0 0 20 20 re f\n",
        b"0.2 0.4 0.6 rg 0 0 20 20 re f\n",
    );
    let start = find(&source, old, 0);
    let end = find(&source, b"\nendstream", start);
    let edited_content = [new.as_slice(), &source[start + old.len()..end]].concat();
    // Same length, so the offsets hold.
    let mut edited = source.clone();
    edited.splice(start..start + old.len(), new.iter().copied());
    let before = view_pixels(&mut session_with(source.clone()).1);
    let after = view_pixels(&mut session_with(edited).1);
    assert_ne!(before, after);
    let (mut engine, mut session) = session_with(source);
    assert_eq!(view_pixels(&mut session), before);
    let content = ObjectId::new(4, 0).unwrap();
    let look = |session: &mut AppSession, inside: bool| {
        session
            .mutate("edit", |session| {
                session
                    .tx()?
                    .replace_decoded_stream(content, edited_content.clone())?;
                if inside {
                    assert_eq!(view_pixels(session), after, "inside the operation");
                }
                Ok(())
            })
            .unwrap();
    };
    look(&mut session, false);
    assert_eq!(view_pixels(&mut session), after);
    for (op, pixels) in [("undo", &before), ("redo", &after), ("undo", &before)] {
        let (reply, _) = engine.dispatch(&mut session, json!({"op": op}), Vec::new());
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(&view_pixels(&mut session), pixels, "after {op}");
    }
    look(&mut session, true);
    assert_eq!(view_pixels(&mut session), after);
}

/// A classic-xref PDF of `pages` pages, each filling `ops` small squares in
/// changing DeviceRGB colours, with the output intent of `INTENT` when
/// `intent`.
fn colourful_pages(pages: usize, ops: usize) -> Vec<u8> {
    let mut objects = vec![
        object(&format!("<< /Type /Catalog /Pages 2 0 R {INTENT} >>")),
        object(""),
        profile(false, 1.0).to_vec(),
    ];
    // Object 13 is the profile INTENT names; fill 4..13 with nothing.
    objects.resize(12, object("null"));
    objects.push(profile(false, 1.0));
    let mut kids = String::new();
    for page in 0..pages {
        let content: String = (0..ops)
            .map(|i| {
                format!(
                    "{} {} 0.5 rg {} {} 4 4 re f\n",
                    i % 7,
                    page % 5,
                    i % 150,
                    i % 190
                )
            })
            .collect();
        objects.push(stream("", content.as_bytes()));
        let content_id = objects.len();
        objects.push(object(&format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 150 190] /Contents {content_id} 0 R >>"
        )));
        kids.push_str(&format!("{} 0 R ", objects.len()));
    }
    objects[1] = object(&format!("<< /Type /Pages /Kids [{kids}] /Count {pages} >>"));
    pdf(&objects)
}

#[test]
fn a_view_after_an_edit_rewrites_only_what_the_edit_touched() {
    // 100 pages of 3,000 colour changes each (4 MB of content) drawn
    // through the output intent: the first view rewrites them all, a view
    // after an edit elsewhere none (it took as long as the first).
    let (mut engine, mut session) = session_with(colourful_pages(100, 3000));
    let started = Instant::now();
    session.view().unwrap();
    let first = started.elapsed();
    let note =
        json!({"op": "annot-add", "page": 0, "type": "Text", "point": [10, 10], "contents": "x"});
    let (reply, _) = engine.dispatch(&mut session, note, Vec::new());
    assert_eq!(reply["ok"], true, "{reply}");
    let started = Instant::now();
    session.view().unwrap();
    let again = started.elapsed();
    assert!(
        again * 4 < first,
        "first view {first:?}, after an edit {again:?}"
    );
}

#[test]
fn pages_sharing_one_annotation_array_walk_it_once() {
    // 4,000 pages share one /Annots array of 4,000 annotations with
    // appearances; the resources walk went through it for every page
    // (16 million items, about 2 s natively).
    let (pages, annots) = (4000, 4000);
    let mut objects = vec![
        object("<< /Type /Catalog /Pages 2 0 R >>"),
        object(""),
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 10 10]",
            b"0 0 1 rg 0 0 10 10 re f",
        ),
        stream("", b""),
    ];
    let items: String = (0..annots).map(|i| format!("{} 0 R ", 6 + i)).collect();
    objects.push(object(&format!("[{items}]")));
    for _ in 0..annots {
        objects.push(object(
            "<< /Type /Annot /Subtype /Square /Rect [10 10 20 20] /F 4 /AP << /N 3 0 R >> >>",
        ));
    }
    let first_page = objects.len() + 1;
    for _ in 0..pages {
        objects.push(object(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Annots 5 0 R >>",
        ));
    }
    let kids: String = (0..pages)
        .map(|i| format!("{} 0 R ", first_page + i))
        .collect();
    objects[1] = object(&format!("<< /Type /Pages /Kids [{kids}] /Count {pages} >>"));
    let (_, mut session) = session_with(pdf(&objects));
    let started = Instant::now();
    session.view().unwrap();
    let took = started.elapsed();
    assert!(took.as_millis() < 500, "first view {took:?}");
}

/// A 100 × 40 page showing `codes` at 20 pt from (10, 10) in a Type0 font
/// without a program: /BaseFont /Arial, Identity-H, a CIDFontType2 of
/// `ordering` with `cid_extra` in it; `to_unicode` maps codes when given.
fn cid_page(
    codes: &str,
    ordering: &str,
    cid_extra: &str,
    to_unicode: Option<&[(u16, u16)]>,
) -> Vec<u8> {
    let bfchar: String = (to_unicode.unwrap_or_default().iter())
        .map(|(code, unicode)| format!("<{code:04X}> <{unicode:04X}> "))
        .collect();
    let count = to_unicode.map_or(0, <[_]>::len);
    let cmap = format!(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /T def \
         /CMapType 2 def 1 begincodespacerange <0000> <FFFF> endcodespacerange \
         {count} beginbfchar {bfchar}endbfchar endcmap CMapName currentdict /CMap \
         defineresource pop end end"
    );
    let tounicode = if to_unicode.is_some() {
        "/ToUnicode 8 0 R"
    } else {
        ""
    };
    pdf(&[
        object("<< /Type /Catalog /Pages 2 0 R >>"),
        object("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        object(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 40] /Contents 4 0 R \
             /Resources << /Font << /F1 5 0 R /F2 9 0 R >> >> >>",
        ),
        stream(
            "",
            format!("BT /F1 20 Tf 10 10 Td {codes} Tj ET").as_bytes(),
        ),
        object(&format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /Arial /Encoding /Identity-H \
             /DescendantFonts [6 0 R] {tounicode} >>"
        )),
        object(&format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Arial /CIDSystemInfo \
             << /Registry (Adobe) /Ordering ({ordering}) /Supplement 0 >> /FontDescriptor 7 0 R \
             {cid_extra} >>"
        )),
        object(
            "<< /Type /FontDescriptor /FontName /Arial /Flags 32 /FontBBox [0 -200 1000 900] \
             /ItalicAngle 0 /Ascent 900 /Descent -200 /CapHeight 700 /StemV 80 >>",
        ),
        stream("", cmap.as_bytes()),
        object("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"),
        stream("", &[0, 0, 0, 36]),
    ])
}

fn ink_and_text(bytes: Vec<u8>) -> (usize, String) {
    let (mut engine, mut session) = session_with(bytes);
    let request = json!({"op": "render", "page": 0, "scale": 1});
    let (reply, buffers) = engine.dispatch(&mut session, request, Vec::new());
    assert_eq!(reply["ok"], true, "{reply}");
    let ink = buffers[0].chunks_exact(4).filter(|px| px[0] < 128).count();
    (
        ink,
        reply["text"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_string(),
    )
}

#[test]
fn a_cid_font_without_a_program_draws_only_the_glyphs_mupdf_places() {
    // MuPDF takes a CID as an index into the Macintosh glyph order when the
    // font has no /ToUnicode (and no /CIDToGIDMap stream): CID 36 is "A",
    // drawn with the substitute face as Helvetica draws it.
    let (cid, _) = render(cid_page("<0024>", "Identity", "", None));
    let mut reference = cid_page("<0024>", "Identity", "", None);
    let content = b"BT /F1 20 Tf 10 10 Td <0024> Tj ET";
    let at = reference
        .windows(content.len())
        .position(|w| w == content)
        .unwrap();
    // Same length, so the offsets hold.
    reference.splice(
        at..at + content.len(),
        b"BT /F2 20 Tf 10 10 Td (A)    Tj ET".iter().copied(),
    );
    let (expected, _) = render(reference);
    assert_eq!(cid, expected);
    assert_eq!(
        ink_and_text(cid_page("<00030004>", "Identity", "", None)).1,
        "!"
    );
    // What MuPDF cannot place it leaves out; hayro drew the substitute
    // face's glyph numbered like the CID.
    for (codes, extra, to_unicode) in [
        ("<012C>", "", None),
        ("<0001>", "/CIDToGIDMap 10 0 R", None),
        ("<0024>", "", Some([(0x24, 0x628)].as_slice())),
        (
            "<0025>",
            "/CIDToGIDMap /Identity",
            Some([(0x24, 0x41)].as_slice()),
        ),
    ] {
        let (ink, _) = ink_and_text(cid_page(codes, "Identity", extra, to_unicode));
        assert_eq!(ink, 0, "{codes} {extra} {to_unicode:?}");
    }
    let (ink, text) = ink_and_text(cid_page("<0024>", "Identity", "", Some(&[(0x24, 0x42)])));
    assert!(ink > 0);
    assert_eq!(text, "B");
}

#[test]
fn transfer_functions_are_left_out_as_mupdf_paints() {
    // MuPDF ignores /TR and /TR2 when it paints; hayro applied an inverting
    // function to the fill (0.2 0.5 0.8 drew as 204 127 51).
    for (states, gs) in [
        (
            "<< /G0 << /TR << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >> >> >>",
            "/G0 gs",
        ),
        ("<< /G0 5 0 R >>", "/G0 gs"),
        ("6 0 R", "/G1 gs"),
    ] {
        let (pixels, width) = render(pdf(&[
            object("<< /Type /Catalog /Pages 2 0 R >>"),
            object("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
            object(&format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 20 20] /Contents 4 0 R \
                 /Resources << /ExtGState {states} >> >>"
            )),
            stream("", format!("{gs} 0.2 0.5 0.8 rg 0 0 20 20 re f").as_bytes()),
            object("<< /TR2 << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >> >>"),
            object("<< /G1 5 0 R >>"),
        ]));
        let rgb = at(&pixels, width, 10, 10);
        assert!(
            rgb.iter()
                .zip([51, 128, 204])
                .all(|(&a, b)| a.abs_diff(b) <= 2),
            "{states}: {rgb:?}"
        );
    }
}

#[test]
fn an_annotation_flagged_invisible_is_left_out_whatever_its_type() {
    // MuPDF (and pdf.js) leave out an annotation whose /F sets Invisible,
    // a standard type included; the view drew a Square, a Text note and a
    // Widget so (veraPDF 6-3-2, 6-5-3 and 8.9.2.2 test files). Print alone
    // still draws.
    for (kind, flags, drawn) in [
        ("Square", 1, false),
        ("Square", 5, false),
        ("Text", 1, false),
        ("Widget /FT /Btn", 1, false),
        ("Square", 4, true),
    ] {
        let (pixels, _) = render(pdf(&[
            object("<< /Type /Catalog /Pages 2 0 R >>"),
            object("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
            object(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R \
                 /Annots [5 0 R] >>",
            ),
            stream("", b""),
            object(&format!(
                "<< /Type /Annot /Subtype /{kind} /F {flags} /Rect [10 10 50 50] \
                 /AP << /N 6 0 R >> >>"
            )),
            stream(
                "/Type /XObject /Subtype /Form /BBox [0 0 40 40]",
                b"0 0 1 rg 0 0 40 40 re f",
            ),
        ]));
        let blue = pixels
            .chunks_exact(4)
            .filter(|px| px[0] < 128 && px[2] > 128)
            .count();
        assert_eq!(blue > 0, drawn, "{kind} /F {flags}");
    }
}

#[test]
fn run_length_image_data_without_its_end_byte_is_drawn() {
    // MuPDF draws the runs it finds; hayro refused the stream and drew
    // nothing (govdocs1 file whose logo was missing). Two rows of a 2 × 2
    // gray image: a repeat run of 2 bytes, then a literal run of 2.
    for (data, drawn) in [
        (vec![255u8, 0x40, 1, 0x40, 0x40], true),
        (vec![255u8, 0x40, 1, 0x40, 0x40, 128], true),
        // A run cut short stays as it is.
        (vec![255u8, 0x40, 3, 0x40], false),
    ] {
        let (pixels, width) = render(pdf(&[
            object("<< /Type /Catalog /Pages 2 0 R >>"),
            object("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
            object(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 20 20] /Contents 4 0 R \
                 /Resources << /XObject << /I 5 0 R >> >> >>",
            ),
            stream("", b"q 20 0 0 20 0 0 cm /I Do Q"),
            stream(
                "/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceGray \
                 /BitsPerComponent 8 /Filter /RunLengthDecode",
                &data,
            ),
        ]));
        assert_eq!(near(at(&pixels, width, 10, 10), 0x40), drawn, "{data:?}");
    }
}
